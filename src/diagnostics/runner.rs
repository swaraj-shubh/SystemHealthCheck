//! Executes registry commands with argv (never a shell), applies native
//! pipeline filters, enforces timeouts/cancellation and classifies failures.
//!
//! Privileged entries are executed through `pkexec <self> --helper ...`, so the
//! root side only ever runs allowlisted registry ids with re-validated params.

use super::registry::{self, CommandSpec, Filter, ParamKind, Step};
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Parameter values keyed by [`registry::Param::key`].
pub type Params = BTreeMap<String, String>;

/// Callback receiving output lines as they arrive: `(is_stderr, line)`.
pub type LineSink = Arc<dyn Fn(bool, &str) + Send + Sync>;

/// Max bytes captured per stream; protects against runaway output.
const MAX_CAPTURE: usize = 8 * 1024 * 1024;

/// Outcome classification shown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Success,
    Failed,
    NotInstalled,
    PermissionDenied,
    AuthCancelled,
    TimedOut,
    Cancelled,
    Unavailable,
    NotRunnable,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Success => "Success",
            RunStatus::Failed => "Failed",
            RunStatus::NotInstalled => "Tool not installed",
            RunStatus::PermissionDenied => "Permission required",
            RunStatus::AuthCancelled => "Authentication cancelled",
            RunStatus::TimedOut => "Timed out",
            RunStatus::Cancelled => "Cancelled",
            RunStatus::Unavailable => "Unavailable",
            RunStatus::NotRunnable => "Not runnable in GUI",
        }
    }
}

/// Everything captured from one command run. Nothing is discarded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CmdOutput {
    pub id: String,
    /// The command that was actually executed (argv joined for display).
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub status: RunStatus,
    /// Human explanation for non-success states.
    pub detail: String,
    /// Missing program, if any (drives "Install" buttons).
    pub missing_tool: Option<String>,
    pub finished_at: i64,
}

impl CmdOutput {
    pub fn ok(&self) -> bool {
        self.status == RunStatus::Success
    }

    /// stdout if the command succeeded.
    pub fn text(&self) -> Option<&str> {
        self.ok().then_some(self.stdout.as_str())
    }
}

/// Raw process outcome. Also the helper's JSON wire format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawExec {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub missing: Option<String>,
    pub spawn_error: Option<String>,
    pub unavailable: Option<String>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration_ms: u64,
}

/// Cancellation, streaming and timeout control for a run.
#[derive(Clone, Default)]
pub struct RunControl {
    pub cancel: Arc<AtomicBool>,
    pub on_line: Option<LineSink>,
    /// Overrides the registry timeout when set.
    pub timeout: Option<Duration>,
}

impl RunControl {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Pure syntactic check for a device path (no filesystem access).
pub fn valid_device_name(v: &str, controller: bool) -> bool {
    let Some(name) = v.strip_prefix("/dev/") else { return false };
    if !name.chars().all(|c| c.is_ascii_alphanumeric()) || name.len() > 32 {
        return false;
    }
    if controller {
        return name.len() > 4 && name.starts_with("nvme") && name[4..].chars().all(|c| c.is_ascii_digit());
    }
    ["nvme", "sd", "hd", "vd", "xvd", "mmcblk"].iter().any(|p| name.len() > p.len() && name.starts_with(p))
}

/// Validate one parameter value. This is the trust boundary for the helper.
pub fn validate_param(kind: ParamKind, v: &str) -> Result<(), String> {
    match kind {
        ParamKind::NvmeController | ParamKind::BlockDevice => {
            let ctrl = kind == ParamKind::NvmeController;
            if !valid_device_name(v, ctrl) {
                return Err(format!("'{v}' is not a valid {} path", if ctrl { "NVMe controller" } else { "block device" }));
            }
            if !Path::new(v).exists() {
                return Err(format!("{v} does not exist on this system"));
            }
            Ok(())
        }
        ParamKind::Int { min, max } => match v.parse::<u32>() {
            Ok(n) if (min..=max).contains(&n) && v.chars().all(|c| c.is_ascii_digit()) => Ok(()),
            _ => Err(format!("'{v}' must be a whole number between {min} and {max}")),
        },
        ParamKind::Unit => {
            let ok = v.len() <= 128
                && !v.starts_with(['-', '.'])
                && (v.ends_with(".service") || v.ends_with(".socket"))
                && v.chars().all(|c| c.is_ascii_alphanumeric() || "@._-:\\".contains(c));
            if ok {
                Ok(())
            } else {
                Err(format!("'{v}' is not a valid systemd service/socket name"))
            }
        }
        ParamKind::Package => {
            if crate::packages::INSTALLABLE.contains(&v) {
                Ok(())
            } else {
                Err(format!("'{v}' is not an allowlisted diagnostic package"))
            }
        }
    }
}

/// First NVMe controller / main disk on this machine, for sensible defaults.
fn detect_device(controller: bool) -> String {
    let mut names: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    let disk = names
        .iter()
        .find(|n| n.starts_with("nvme"))
        .or_else(|| names.iter().find(|n| n.starts_with("sd") || n.starts_with("mmcblk") || n.starts_with("vd")))
        .cloned();
    match (controller, disk) {
        (true, Some(d)) if d.starts_with("nvme") => format!("/dev/{}", d.split('n').take(2).collect::<Vec<_>>().join("n")),
        (true, _) => "/dev/nvme0".into(),
        (false, Some(d)) => format!("/dev/{d}"),
        (false, None) => "/dev/nvme0n1".into(),
    }
}

/// Default parameter values detected from the running system.
pub fn default_params(spec: &CommandSpec) -> Params {
    spec.params
        .iter()
        .map(|p| {
            let v = match (p.kind, p.key) {
                (ParamKind::NvmeController, _) => detect_device(true),
                (ParamKind::BlockDevice, _) => detect_device(false),
                (_, "threads" | "workers") => util::cpu_threads().to_string(),
                (_, "secs") if spec.id.starts_with("stress") => "60".into(),
                (_, "secs") => "30".into(),
                (_, "value") => util::read_trim("/proc/sys/vm/swappiness").unwrap_or_else(|| "60".into()),
                _ => String::new(),
            };
            (p.key.to_string(), v)
        })
        .collect()
}

/// Build the validated argv for a `Step::Run` entry.
pub fn build_argv(spec: &CommandSpec, params: &Params) -> Result<Vec<String>, String> {
    let Step::Run(template) = spec.step else {
        return Err("not an argv command".into());
    };
    for p in spec.params {
        let v = params.get(p.key).ok_or_else(|| format!("missing parameter '{}'", p.label))?;
        validate_param(p.kind, v)?;
    }
    let mut argv = Vec::with_capacity(template.len());
    for t in template {
        let mut a = (*t).to_string();
        if a.contains("{home}") {
            let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
            if !home.starts_with('/') {
                return Err("HOME is not an absolute path".into());
            }
            a = a.replace("{home}", &home);
        }
        for p in spec.params {
            a = a.replace(&format!("{{{}}}", p.key), &params[p.key]);
        }
        argv.push(a);
    }
    Ok(argv)
}

/// Human description of the native filters (`| grep ...`).
pub fn describe_filters(filters: &[Filter]) -> String {
    filters
        .iter()
        .map(|f| match f {
            Filter::Grep { any_of, ignore_case, after } => {
                let mut s = String::from(" | grep ");
                if *after > 0 {
                    s += &format!("-A {after} ");
                }
                s += if *ignore_case { "-Ei " } else { "-E " };
                s + &util::shell_quote(&any_of.join("|"))
            }
            Filter::Head(n) => format!(" | head -{n}"),
            Filter::Tail(n) => format!(" | tail -{n}"),
            Filter::SortHuman => " | sort -h".into(),
        })
        .collect()
}

/// The command as it will actually be executed, for display before running.
pub fn display_command(spec: &CommandSpec, params: &Params) -> String {
    let base = match spec.step {
        Step::Run(_) => match build_argv(spec, params) {
            Ok(a) => a.iter().map(|s| util::shell_quote(s)).collect::<Vec<_>>().join(" "),
            Err(e) => format!("<invalid: {e}>"),
        },
        Step::Read(p) => format!("cat {p}"),
        Step::UpowerBattery => "upower -e  →  upower -i <each BAT device>".into(),
        Step::Interactive(_) => spec.original.to_string(),
    };
    let elevate = if spec.requires_sudo && !util::is_root() { "pkexec " } else { "" };
    format!("{elevate}{base}{}", describe_filters(spec.filters))
}

// ---------------------------------------------------------------------------
// Process execution
// ---------------------------------------------------------------------------

fn kill_group(pid: u32, sig: libc::c_int) {
    // SAFETY: plain syscall; negative pid targets the process group we created.
    unsafe {
        libc::kill(-(pid as libc::pid_t), sig);
    }
}

fn spawn_reader<R: Read + Send + 'static>(r: R, is_err: bool, sink: Option<LineSink>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut out = String::new();
        let mut reader = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    if let Some(s) = &sink {
                        s(is_err, line.trim_end_matches('\n'));
                    }
                    if out.len() < MAX_CAPTURE {
                        out.push_str(&line);
                    }
                }
            }
        }
        out
    })
}

/// Run an argv with timeout/cancellation. `argv[0]` is resolved via [`util::which`].
pub fn exec_argv(argv: &[String], timeout: Duration, ctl: &RunControl) -> RawExec {
    let start = Instant::now();
    let mut raw = RawExec::default();
    let Some(prog) = argv.first() else {
        raw.spawn_error = Some("empty command".into());
        return raw;
    };
    let Some(path) = util::which(prog) else {
        raw.missing = Some(prog.clone());
        return raw;
    };
    let mut cmd = Command::new(path);
    cmd.args(&argv[1..])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            raw.missing = Some(prog.clone());
            return raw;
        }
        Err(e) => {
            raw.spawn_error = Some(e.to_string());
            return raw;
        }
    };
    let pid = child.id();
    let out_t = child.stdout.take().map(|s| spawn_reader(s, false, ctl.on_line.clone()));
    let err_t = child.stderr.take().map(|s| spawn_reader(s, true, ctl.on_line.clone()));
    let mut term_sent: Option<Instant> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(_) => break None,
        }
        let cancelled = ctl.cancel.load(Ordering::SeqCst);
        let timed_out = start.elapsed() > timeout;
        match term_sent {
            None if cancelled || timed_out => {
                raw.cancelled = cancelled;
                raw.timed_out = !cancelled && timed_out;
                kill_group(pid, libc::SIGTERM);
                term_sent = Some(Instant::now());
            }
            Some(t) if t.elapsed() > Duration::from_secs(3) => {
                kill_group(pid, libc::SIGKILL);
                let _ = child.kill();
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    raw.stdout = out_t.and_then(|t| t.join().ok()).unwrap_or_default();
    raw.stderr = err_t.and_then(|t| t.join().ok()).unwrap_or_default();
    raw.code = status.and_then(|s| s.code());
    raw.duration_ms = start.elapsed().as_millis() as u64;
    raw
}

/// Execute a spec's step locally with the current privileges (no filters).
pub fn exec_step(spec: &CommandSpec, params: &Params, ctl: &RunControl) -> RawExec {
    let timeout = ctl.timeout.unwrap_or(Duration::from_secs(spec.timeout_s));
    match spec.step {
        Step::Run(_) => match build_argv(spec, params) {
            Ok(argv) => exec_argv(&argv, timeout, ctl),
            Err(e) => RawExec { spawn_error: Some(e), ..Default::default() },
        },
        Step::Read(path) => {
            let start = Instant::now();
            let mut raw = RawExec::default();
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    raw.stdout = s;
                    raw.code = Some(0);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    raw.unavailable = Some(format!("{path} does not exist (not exposed by this hardware/kernel)"));
                }
                Err(e) => {
                    raw.stderr = format!("cat: {path}: {e}");
                    raw.code = Some(1);
                }
            }
            raw.duration_ms = start.elapsed().as_millis() as u64;
            raw
        }
        Step::UpowerBattery => {
            let start = Instant::now();
            let list = exec_argv(&["upower".into(), "-e".into()], timeout, ctl);
            if list.missing.is_some() || list.code != Some(0) {
                return list;
            }
            let bats: Vec<&str> = list.stdout.lines().filter(|l| l.contains("BAT")).collect();
            if bats.is_empty() {
                return RawExec {
                    unavailable: Some("UPower reports no battery (desktop system or battery not detected)".into()),
                    duration_ms: start.elapsed().as_millis() as u64,
                    ..list
                };
            }
            let mut acc = RawExec { code: Some(0), ..Default::default() };
            for b in bats {
                let r = exec_argv(&["upower".into(), "-i".into(), b.trim().to_string()], timeout, ctl);
                acc.stdout += &r.stdout;
                acc.stderr += &r.stderr;
                if r.code != Some(0) {
                    acc.code = r.code;
                }
                acc.cancelled |= r.cancelled;
                acc.timed_out |= r.timed_out;
            }
            acc.duration_ms = start.elapsed().as_millis() as u64;
            acc
        }
        Step::Interactive(_) => RawExec::default(),
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Parse a `du -h` / `sort -h` size like "1.5G" into bytes.
pub fn parse_human_size(s: &str) -> f64 {
    let s = s.trim();
    let (num, suf) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: f64 = num.replace(',', ".").parse().unwrap_or(0.0);
    let mult = match suf.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('K') => 1024f64,
        Some('M') => 1024f64.powi(2),
        Some('G') => 1024f64.powi(3),
        Some('T') => 1024f64.powi(4),
        Some('P') => 1024f64.powi(5),
        _ => 1.0,
    };
    n * mult
}

/// Apply native pipeline filters to command output.
pub fn apply_filters(text: &str, filters: &[Filter]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for f in filters {
        lines = match *f {
            Filter::Grep { any_of, ignore_case, after } => {
                let pats: Vec<String> = any_of.iter().map(|p| if ignore_case { p.to_lowercase() } else { (*p).to_string() }).collect();
                let mut out = Vec::new();
                let mut remaining = 0usize;
                let mut last_idx: Option<usize> = None;
                for (i, l) in lines.iter().enumerate() {
                    let hay = if ignore_case { l.to_lowercase() } else { l.clone() };
                    if pats.iter().any(|p| hay.contains(p.as_str())) {
                        if after > 0 && last_idx.is_some_and(|li| i > li + 1) {
                            out.push("--".to_string());
                        }
                        out.push(l.clone());
                        last_idx = Some(i);
                        remaining = after;
                    } else if remaining > 0 {
                        out.push(l.clone());
                        last_idx = Some(i);
                        remaining -= 1;
                    }
                }
                out
            }
            Filter::Head(n) => lines.into_iter().take(n).collect(),
            Filter::Tail(n) => {
                let skip = lines.len().saturating_sub(n);
                lines.into_iter().skip(skip).collect()
            }
            Filter::SortHuman => {
                let mut l = lines;
                l.sort_by(|a, b| {
                    let ka = parse_human_size(a.split_whitespace().next().unwrap_or(""));
                    let kb = parse_human_size(b.split_whitespace().next().unwrap_or(""));
                    ka.total_cmp(&kb)
                });
                l
            }
        };
    }
    let mut s = lines.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

const PERMISSION_MARKERS: &[&str] = &[
    "permission denied",
    "operation not permitted",
    "must be root",
    "must be run as root",
    "are you root",
    "requires root",
    "root privileges",
    "insufficient privileges",
    "access denied",
    "superuser",
];

/// Package that provides a program (for install hints).
pub fn package_for(prog: &str) -> &'static str {
    match prog {
        "smartctl" => "smartmontools",
        "nvme" => "nvme-cli",
        "sensors" | "sensors-detect" => "lm-sensors",
        "glxinfo" => "mesa-utils",
        "upower" => "upower",
        "cpupower" => "linux-cpupower",
        "sysbench" => "sysbench",
        "stress-ng" => "stress-ng",
        "htop" => "htop",
        "lshw" => "lshw",
        "dmidecode" => "dmidecode",
        "hdparm" => "hdparm",
        "lspci" => "pciutils",
        "lsusb" => "usbutils",
        "iw" => "iw",
        "nmcli" => "network-manager",
        "pkexec" => "pkexec",
        _ => "",
    }
}

fn finish(spec: &CommandSpec, raw: RawExec, command: String, elevated: bool) -> CmdOutput {
    let combined = format!("{}\n{}", raw.stderr, raw.stdout).to_lowercase();
    let (status, detail) = if let Some(u) = &raw.unavailable {
        (RunStatus::Unavailable, u.clone())
    } else if let Some(p) = &raw.missing {
        let pkg = package_for(p);
        let hint = if pkg.is_empty() { String::new() } else { format!(" (package: {pkg})") };
        (RunStatus::NotInstalled, format!("`{p}` is not installed{hint}"))
    } else if raw.cancelled {
        (RunStatus::Cancelled, "Stopped by user".into())
    } else if raw.timed_out {
        (RunStatus::TimedOut, "Command exceeded its timeout and was stopped".into())
    } else if let Some(e) = &raw.spawn_error {
        (RunStatus::Failed, e.clone())
    } else if elevated && raw.code == Some(126) {
        (RunStatus::AuthCancelled, "Authentication dialog was dismissed".into())
    } else if elevated && raw.code == Some(127) && !raw.stderr.contains("systemhealthcheck-helper") {
        (RunStatus::AuthCancelled, "Not authorized (authentication failed or no polkit agent is running)".into())
    } else if raw.code == Some(0) {
        (RunStatus::Success, String::new())
    } else if PERMISSION_MARKERS.iter().any(|m| combined.contains(m)) {
        (RunStatus::PermissionDenied, "Permission required: run with administrator privileges".into())
    } else {
        (RunStatus::Failed, format!("Exited with code {}", raw.code.map_or("signal".into(), |c| c.to_string())))
    };
    let missing_tool = raw.missing.clone().or_else(|| {
        raw.stderr.lines().find_map(|l| l.strip_prefix("systemhealthcheck-helper: not-installed: ").map(str::to_string))
    });
    let (status, detail) = match (&missing_tool, status) {
        (Some(p), RunStatus::Failed) => (RunStatus::NotInstalled, format!("`{p}` is not installed (package: {})", package_for(p))),
        _ => (status, detail),
    };
    let stdout = if status == RunStatus::Success { apply_filters(&raw.stdout, spec.filters) } else { raw.stdout };
    CmdOutput {
        id: spec.id.to_string(),
        command,
        stdout,
        stderr: raw.stderr,
        exit_code: raw.code,
        duration_ms: raw.duration_ms,
        status,
        detail,
        missing_tool,
        finished_at: util::now(),
    }
}

fn not_runnable(spec: &CommandSpec, why: String) -> CmdOutput {
    CmdOutput {
        id: spec.id.into(),
        command: spec.original.into(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        duration_ms: 0,
        status: RunStatus::NotRunnable,
        detail: why,
        missing_tool: None,
        finished_at: util::now(),
    }
}

/// Encode a request for the helper argv: `id` or `id?k=v&k=v`.
pub fn encode_request(id: &str, params: &Params) -> String {
    if params.is_empty() {
        return id.to_string();
    }
    let kv: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{id}?{}", kv.join("&"))
}

/// Decode a helper request. The values are validated later by `build_argv`.
pub fn decode_request(s: &str) -> Option<(&'static CommandSpec, Params)> {
    let (id, rest) = s.split_once('?').unwrap_or((s, ""));
    let spec = registry::get(id)?;
    let params = rest
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    Some((spec, params))
}

fn helper_argv(mode: &str, reqs: &[String]) -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate own executable: {e}"))?;
    let mut argv = vec!["pkexec".to_string(), exe.to_string_lossy().into_owned(), "--helper".into(), mode.into()];
    argv.extend(reqs.iter().cloned());
    Ok(argv)
}

/// Run one registry command (elevating through pkexec when required).
pub fn run(spec: &CommandSpec, params: &Params, ctl: &RunControl) -> CmdOutput {
    if let Step::Interactive(page) = spec.step {
        return not_runnable(spec, format!("`{}` is an interactive terminal program. Use the native {page} page instead.", spec.original));
    }
    if let Step::Run(_) = spec.step {
        if let Err(e) = build_argv(spec, params) {
            return not_runnable(spec, e);
        }
    }
    let command = display_command(spec, params);
    if spec.requires_sudo && !util::is_root() {
        let raw = match helper_argv("exec", &[encode_request(spec.id, params)]) {
            Ok(argv) => {
                let timeout = Duration::from_secs(spec.timeout_s + 120);
                exec_argv(&argv, ctl.timeout.map_or(timeout, |t| t + Duration::from_secs(120)), ctl)
            }
            Err(e) => RawExec { spawn_error: Some(e), ..Default::default() },
        };
        finish(spec, raw, command, true)
    } else {
        finish(spec, exec_step(spec, params, ctl), command, false)
    }
}

/// Run several privileged commands behind a single authentication prompt.
pub fn run_privileged_batch(reqs: &[(&'static CommandSpec, Params)], ctl: &RunControl) -> Vec<CmdOutput> {
    if reqs.is_empty() {
        return Vec::new();
    }
    if util::is_root() {
        return reqs.iter().map(|(s, p)| run(s, p, ctl)).collect();
    }
    let encoded: Vec<String> = reqs.iter().map(|(s, p)| encode_request(s.id, p)).collect();
    let total: u64 = reqs.iter().map(|(s, _)| ctl.timeout.map_or(s.timeout_s, |t| t.as_secs())).sum();
    let raw = match helper_argv("batch", &encoded) {
        Ok(argv) => exec_argv(&argv, Duration::from_secs(total + 120), &RunControl { on_line: None, ..ctl.clone() }),
        Err(e) => RawExec { spawn_error: Some(e), ..Default::default() },
    };
    let parsed: Option<Vec<RawExec>> = (raw.code == Some(0)).then(|| serde_json::from_str(&raw.stdout).ok()).flatten();
    match parsed {
        Some(list) if list.len() == reqs.len() => reqs
            .iter()
            .zip(list)
            .map(|((s, p), r)| finish(s, r, display_command(s, p), false))
            .collect(),
        _ => reqs
            .iter()
            .map(|(s, p)| {
                let mut r = raw.clone();
                r.stdout.clear();
                if r.code == Some(0) {
                    r.spawn_error = Some("Privileged helper returned malformed output".into());
                }
                finish(s, r, display_command(s, p), true)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_with_after_context_matches_gnu_grep() {
        let input = "00:02.0 VGA compatible controller: AMD\n\tSubsystem: HP\n\tKernel driver in use: amdgpu\n\tKernel modules: amdgpu\n\tExtra\n00:03.0 Audio device\n05:00.0 Display controller: X\n\tKernel driver in use: y\n";
        let out = apply_filters(input, &[Filter::Grep { any_of: &["vga", "3d", "display"], ignore_case: true, after: 3 }]);
        assert_eq!(
            out,
            "00:02.0 VGA compatible controller: AMD\n\tSubsystem: HP\n\tKernel driver in use: amdgpu\n\tKernel modules: amdgpu\n--\n05:00.0 Display controller: X\n\tKernel driver in use: y\n"
        );
    }

    #[test]
    fn head_tail_sort() {
        let input = "1.5G\t/usr\n12K\t/srv\n800M\t/var\n";
        assert_eq!(apply_filters(input, &[Filter::SortHuman]), "12K\t/srv\n800M\t/var\n1.5G\t/usr\n");
        assert_eq!(apply_filters("a\nb\nc\n", &[Filter::Head(2)]), "a\nb\n");
        assert_eq!(apply_filters("a\nb\nc\n", &[Filter::Tail(2)]), "b\nc\n");
        assert_eq!(apply_filters("", &[Filter::Head(2)]), "");
    }

    #[test]
    fn param_validation_rejects_injection() {
        assert!(valid_device_name("/dev/nvme0", true));
        assert!(valid_device_name("/dev/nvme0n1", false));
        assert!(valid_device_name("/dev/sda", false));
        assert!(!valid_device_name("/dev/nvme0n1", true));
        assert!(!valid_device_name("/dev/sda;rm -rf /", false));
        assert!(!valid_device_name("/etc/passwd", false));
        assert!(!valid_device_name("/dev/../etc", false));
        assert!(validate_param(ParamKind::Int { min: 1, max: 8 }, "4").is_ok());
        assert!(validate_param(ParamKind::Int { min: 1, max: 8 }, "9").is_err());
        assert!(validate_param(ParamKind::Int { min: 1, max: 8 }, "+4").is_err());
        assert!(validate_param(ParamKind::Package, "sysbench").is_ok());
        assert!(validate_param(ParamKind::Package, "sysbench; reboot").is_err());
        assert!(validate_param(ParamKind::Unit, "cups.service").is_ok());
        assert!(validate_param(ParamKind::Unit, "getty@tty1.service").is_ok());
        assert!(validate_param(ParamKind::Unit, "--all").is_err());
        assert!(validate_param(ParamKind::Unit, "cups.service; reboot").is_err());
        assert!(validate_param(ParamKind::Unit, "multi-user.target").is_err());
    }

    #[test]
    fn request_roundtrip() {
        let mut p = Params::new();
        p.insert("threads".into(), "4".into());
        p.insert("secs".into(), "10".into());
        let enc = encode_request("sysbench-cpu", &p);
        let (spec, dec) = decode_request(&enc).expect("decodes");
        assert_eq!(spec.id, "sysbench-cpu");
        assert_eq!(dec, p);
        assert!(decode_request("rm-rf").is_none());
    }

    #[test]
    fn argv_substitution() {
        let spec = registry::get("sysbench-cpu").expect("exists");
        let mut p = Params::new();
        p.insert("threads".into(), "2".into());
        p.insert("secs".into(), "5".into());
        assert_eq!(build_argv(spec, &p).expect("valid"), vec!["sysbench", "cpu", "--threads=2", "--time=5", "run"]);
        p.insert("threads".into(), "2 --evil".into());
        assert!(build_argv(spec, &p).is_err());
    }

    #[test]
    fn runs_real_commands_and_classifies() {
        let ctl = RunControl::default();
        let out = run(registry::get("swappiness").expect("exists"), &Params::new(), &ctl);
        assert!(out.ok(), "{out:?}");
        let spec = registry::get("uname").expect("exists");
        let out = run(spec, &Params::new(), &ctl);
        assert!(out.ok() && out.stdout.contains("Linux"));
        let missing = exec_argv(&["definitely-not-a-real-tool-xyz".into()], Duration::from_secs(5), &ctl);
        assert_eq!(missing.missing.as_deref(), Some("definitely-not-a-real-tool-xyz"));
    }

    #[test]
    fn timeout_kills_process_group() {
        let ctl = RunControl::default();
        let raw = exec_argv(&["sleep".into(), "10".into()], Duration::from_millis(200), &ctl);
        assert!(raw.timed_out);
        assert!(raw.duration_ms < 5000);
    }
}
