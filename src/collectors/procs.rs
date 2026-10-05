//! Process table from /proc/<pid>/stat (the native replacement for `ps`/`htop`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub cmdline: String,
    pub user: String,
    pub state: char,
    /// Percent of one CPU (like `ps`/`top`), so it can exceed 100.
    pub cpu_pct: f64,
    pub mem_pct: f64,
    pub rss_bytes: u64,
    pub threads: u32,
}

impl Proc {
    pub fn state_label(&self) -> &'static str {
        match self.state {
            'R' => "Running",
            'S' => "Sleeping",
            'D' => "Disk wait",
            'Z' => "Zombie",
            'T' | 't' => "Stopped",
            'I' => "Idle",
            _ => "Other",
        }
    }
}

/// Fields parsed from one /proc/<pid>/stat line.
#[derive(Debug, PartialEq)]
pub struct StatLine {
    pub name: String,
    pub state: char,
    pub ppid: u32,
    pub ticks: u64,
    pub threads: u32,
    pub start: u64,
    pub rss_pages: u64,
}

/// Parse /proc/<pid>/stat. The name may contain spaces and parentheses.
pub fn parse_stat(s: &str) -> Option<StatLine> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let name = s.get(open + 1..close)?.to_string();
    let f: Vec<&str> = s.get(close + 2..)?.split_whitespace().collect();
    let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Some(StatLine {
        name,
        state: f.first()?.chars().next()?,
        ppid: n(1) as u32,
        ticks: n(11) + n(12),
        threads: n(17) as u32,
        start: n(19),
        rss_pages: n(21),
    })
}

pub struct ProcState {
    prev: HashMap<u32, (u64, u64)>,
    at: Option<Instant>,
    users: HashMap<u32, String>,
    cmdlines: HashMap<(u32, u64), String>,
    tick_hz: f64,
    page: u64,
}

impl Default for ProcState {
    fn default() -> Self {
        // SAFETY: sysconf has no preconditions.
        let (hz, page) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK), libc::sysconf(libc::_SC_PAGESIZE)) };
        let users = std::fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let c: Vec<&str> = l.split(':').collect();
                Some((c.get(2)?.parse().ok()?, c.first()?.to_string()))
            })
            .collect();
        Self {
            prev: HashMap::new(),
            at: None,
            users,
            cmdlines: HashMap::new(),
            tick_hz: if hz > 0 { hz as f64 } else { 100.0 },
            page: if page > 0 { page as u64 } else { 4096 },
        }
    }
}

impl ProcState {
    pub fn sample(&mut self, total_mem_kb: u64) -> Vec<Proc> {
        let now = Instant::now();
        let dt = self.at.map(|t| now.duration_since(t).as_secs_f64()).filter(|d| *d > 0.0);
        self.at = Some(now);
        let mut next_prev = HashMap::with_capacity(self.prev.len());
        let mut next_cmd = HashMap::with_capacity(self.cmdlines.len());
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else { return out };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            let path = e.path();
            let Some(st) = std::fs::read_to_string(path.join("stat")).ok().and_then(|s| parse_stat(&s)) else { continue };
            let cpu_pct = match (dt, self.prev.get(&pid)) {
                (Some(dt), Some(&(start, t))) if start == st.start => st.ticks.saturating_sub(t) as f64 / self.tick_hz / dt * 100.0,
                _ => 0.0,
            };
            next_prev.insert(pid, (st.start, st.ticks));
            let key = (pid, st.start);
            let cmdline = self.cmdlines.remove(&key).unwrap_or_else(|| {
                std::fs::read(path.join("cmdline"))
                    .map(|b| String::from_utf8_lossy(&b).replace('\0', " ").trim().to_string())
                    .unwrap_or_default()
            });
            next_cmd.insert(key, cmdline.clone());
            let uid = e.metadata().map(|m| m.uid()).unwrap_or(u32::MAX);
            let rss_bytes = st.rss_pages * self.page;
            out.push(Proc {
                pid,
                ppid: st.ppid,
                user: self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string()),
                state: st.state,
                cpu_pct,
                mem_pct: if total_mem_kb > 0 { rss_bytes as f64 / (total_mem_kb as f64 * 1024.0) * 100.0 } else { 0.0 },
                rss_bytes,
                threads: st.threads,
                cmdline: if cmdline.is_empty() { format!("[{}]", st.name) } else { cmdline },
                name: st.name,
            });
        }
        self.prev = next_prev;
        self.cmdlines = next_cmd;
        out
    }
}

/// systemd service/socket unit a process belongs to (from /proc/<pid>/cgroup).
/// Only system units (`system.slice`) are returned; user-session scopes are not services.
pub fn systemd_unit(pid: u32) -> Option<String> {
    let cg = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cg.lines().find_map(|l| l.strip_prefix("0::"))?;
    unit_from_cgroup(path)
}

pub fn unit_from_cgroup(path: &str) -> Option<String> {
    if !path.starts_with("/system.slice/") {
        return None;
    }
    path.split('/').rev().find(|s| s.ends_with(".service") || s.ends_with(".socket")).map(str::to_string)
}

/// Owner uid of a process.
pub fn owner_uid(pid: u32) -> Option<u32> {
    std::fs::metadata(format!("/proc/{pid}")).ok().map(|m| m.uid())
}

/// Send SIGTERM (or SIGKILL) to a process. Returns the OS error on failure.
pub fn signal(pid: u32, kill: bool) -> Result<(), String> {
    if pid <= 1 {
        return Err("refusing to signal PID 0/1".into());
    }
    // SAFETY: plain syscall with a validated positive pid.
    let r = unsafe { libc::kill(pid as libc::pid_t, if kill { libc::SIGKILL } else { libc::SIGTERM }) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_units() {
        assert_eq!(unit_from_cgroup("/system.slice/cups.service"), Some("cups.service".into()));
        assert_eq!(unit_from_cgroup("/system.slice/system-getty.slice/getty@tty1.service"), Some("getty@tty1.service".into()));
        assert_eq!(unit_from_cgroup("/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope"), None);
    }

    #[test]
    fn stat_with_weird_name() {
        let s = "1234 (Web Content (x)) S 1000 1234 1234 0 -1 4194560 1000 0 0 0 150 50 0 0 20 0 30 0 5000 123456789 2500 18446744073709551615";
        let p = parse_stat(s).expect("parses");
        assert_eq!(p.name, "Web Content (x)");
        assert_eq!(p.state, 'S');
        assert_eq!(p.ppid, 1000);
        assert_eq!(p.ticks, 200);
        assert_eq!(p.threads, 30);
        assert_eq!(p.start, 5000);
        assert_eq!(p.rss_pages, 2500);
    }
}
