//! Transparent health scoring.
//!
//! Each category starts at 100. Every evaluated input produces a [`Finding`]
//! recording the input value, the rule applied, its severity and the reason.
//! Penalties: Attention -8, Warning -20, Critical -45. A category with no
//! inputs is "Insufficient data" rather than an invented score. Thresholds
//! are plain data ([`Thresholds`]) so they can be made user-configurable.

use crate::collectors::power::SensorKind;
use crate::collectors::Sample;
use crate::diagnostics::Parsed;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Ok,
    Attention,
    Warning,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Ok => "OK",
            Severity::Attention => "Attention",
            Severity::Warning => "Warning",
            Severity::Critical => "Critical",
        }
    }
    fn penalty(self) -> i32 {
        match self {
            Severity::Ok => 0,
            Severity::Attention => 8,
            Severity::Warning => 20,
            Severity::Critical => 45,
        }
    }
}

/// Displayed health states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum State {
    Excellent,
    Good,
    Normal,
    Attention,
    Warning,
    Critical,
    Unknown,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Excellent => "Excellent",
            State::Good => "Good",
            State::Normal => "Normal",
            State::Attention => "Attention",
            State::Warning => "Warning",
            State::Critical => "Critical",
            State::Unknown => "Insufficient data",
        }
    }

    /// CSS class for the status color (green/yellow/orange/red/gray).
    pub fn css(self) -> &'static str {
        match self {
            State::Excellent | State::Good | State::Normal => "status-good",
            State::Attention => "status-attention",
            State::Warning => "status-warning",
            State::Critical => "status-critical",
            State::Unknown => "status-unknown",
        }
    }

    fn from_score(s: u8) -> State {
        match s {
            95.. => State::Excellent,
            85.. => State::Good,
            70.. => State::Normal,
            55.. => State::Attention,
            30.. => State::Warning,
            _ => State::Critical,
        }
    }

    fn from_severity(s: Severity) -> State {
        match s {
            Severity::Ok => State::Excellent,
            Severity::Attention => State::Attention,
            Severity::Warning => State::Warning,
            Severity::Critical => State::Critical,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub input: String,
    pub rule: String,
    pub severity: Severity,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScore {
    pub name: String,
    pub score: Option<u8>,
    pub state: State,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub overall: Option<u8>,
    pub state: State,
    pub rule: String,
    pub categories: Vec<CategoryScore>,
}

impl HealthReport {
    pub fn category(&self, name: &str) -> Option<&CategoryScore> {
        self.categories.iter().find(|c| c.name == name)
    }

    /// Non-OK findings, worst first, for the Recommendations section.
    pub fn problems(&self) -> Vec<(&str, &Finding)> {
        let mut v: Vec<(&str, &Finding)> =
            self.categories.iter().flat_map(|c| c.findings.iter().map(move |f| (c.name.as_str(), f))).filter(|(_, f)| f.severity > Severity::Ok).collect();
        v.sort_by(|a, b| b.1.severity.cmp(&a.1.severity));
        v
    }
}

/// Every threshold used by the rules. Defaults are conservative, documented values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub cpu_temp: [f64; 3],
    pub gpu_temp: [f64; 3],
    pub ssd_temp: [f64; 3],
    pub load_per_thread: [f64; 3],
    pub mem_used_pct: [f64; 3],
    pub swap_used_pct: [f64; 3],
    pub ssd_wear_pct: [f64; 3],
    pub fs_used_pct: [f64; 3],
    /// Battery health (% of design capacity): attention/warning/critical *below*.
    pub battery_health_pct: [f64; 3],
    pub boot_total_s: [f64; 3],
    pub kernel_errors: [f64; 3],
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            cpu_temp: [80.0, 90.0, 98.0],
            gpu_temp: [80.0, 90.0, 100.0],
            ssd_temp: [65.0, 72.0, 80.0],
            load_per_thread: [0.8, 1.5, 3.0],
            mem_used_pct: [80.0, 90.0, 97.0],
            // Swap alone is never critical (RAM pressure is scored separately).
            swap_used_pct: [60.0, 85.0, 101.0],
            ssd_wear_pct: [50.0, 80.0, 95.0],
            fs_used_pct: [85.0, 92.0, 97.0],
            battery_health_pct: [80.0, 60.0, 40.0],
            boot_total_s: [45.0, 90.0, 180.0],
            // Many laptops log harmless ACPI firmware errors at every boot.
            kernel_errors: [10.0, 100.0, 500.0],
        }
    }
}

/// Higher-is-worse rule.
fn above(v: f64, t: [f64; 3], unit: &str) -> (Severity, String) {
    let sev = if v >= t[2] {
        Severity::Critical
    } else if v >= t[1] {
        Severity::Warning
    } else if v >= t[0] {
        Severity::Attention
    } else {
        Severity::Ok
    };
    (sev, format!("≥{}{unit} Attention, ≥{}{unit} Warning, ≥{}{unit} Critical", t[0], t[1], t[2]))
}

/// Lower-is-worse rule.
fn below(v: f64, t: [f64; 3], unit: &str) -> (Severity, String) {
    let sev = if v < t[2] {
        Severity::Critical
    } else if v < t[1] {
        Severity::Warning
    } else if v < t[0] {
        Severity::Attention
    } else {
        Severity::Ok
    };
    (sev, format!("<{}{unit} Attention, <{}{unit} Warning, <{}{unit} Critical", t[0], t[1], t[2]))
}

struct Cat {
    name: &'static str,
    findings: Vec<Finding>,
}

impl Cat {
    fn new(name: &'static str) -> Self {
        Cat { name, findings: Vec::new() }
    }
    fn push(&mut self, input: String, (severity, rule): (Severity, String), reason: &str) {
        self.findings.push(Finding { input, rule, severity, reason: reason.to_string() });
    }
    fn finish(self) -> CategoryScore {
        if self.findings.is_empty() {
            return CategoryScore { name: self.name.into(), score: None, state: State::Unknown, findings: self.findings };
        }
        let penalty: i32 = self.findings.iter().map(|f| f.severity.penalty()).sum();
        let score = (100 - penalty).clamp(0, 100) as u8;
        let worst = self.findings.iter().map(|f| f.severity).max().unwrap_or(Severity::Ok);
        let state = State::from_score(score).max(State::from_severity(worst));
        CategoryScore { name: self.name.into(), score: Some(score), state, findings: self.findings }
    }
}

/// Compute the health report from native readings and parsed command output.
pub fn evaluate(s: &Sample, p: &Parsed, t: &Thresholds) -> HealthReport {
    // CPU
    let mut cpu = Cat::new("CPU");
    if let Some(temp) = s.max_temp(SensorKind::Cpu) {
        cpu.push(format!("CPU temperature = {temp:.1} °C"), above(temp, t.cpu_temp, " °C"), "Sustained high CPU temperatures cause throttling and reduce component life.");
    }
    if s.uptime_s > 300.0 && s.cpu.threads > 0 {
        let ratio = s.load_avg[2] / s.cpu.threads as f64;
        cpu.push(
            format!("15-min load average = {:.2} on {} threads ({ratio:.2} per thread)", s.load_avg[2], s.cpu.threads),
            above(ratio, t.load_per_thread, "/thread"),
            "A run queue longer than the number of threads means work is waiting for CPU time.",
        );
    }

    // Memory
    let mut mem = Cat::new("Memory");
    if s.memory.total_kb > 0 {
        let u = s.memory.used_pct();
        mem.push(format!("RAM in use (total - available) = {u:.1}%"), above(u, t.mem_used_pct, "%"), "Little available memory forces the kernel to swap or kill processes.");
    }
    if let Some(sw) = s.memory.swap_used_pct() {
        mem.push(format!("Swap used = {sw:.1}%"), above(sw, t.swap_used_pct, "%"), "Heavy swap use indicates memory pressure and slows the system.");
    }

    // Storage
    let mut sto = Cat::new("Storage");
    if let Some(d) = &p.drive {
        if let Some(passed) = d.smart_passed {
            let sev = if passed { Severity::Ok } else { Severity::Critical };
            sto.push(
                format!("SMART overall-health = {}", if passed { "PASSED" } else { "FAILED" }),
                (sev, "FAILED → Critical".into()),
                "The drive's own self-assessment predicts failure.",
            );
        }
        if let Some(cw) = d.critical_warning {
            let sev = if cw == 0 { Severity::Ok } else { Severity::Critical };
            sto.push(format!("NVMe critical_warning = {cw:#x}"), (sev, "non-zero → Critical".into()), "The controller flagged spare, temperature, reliability or read-only conditions.");
        }
        if let Some(w) = d.percentage_used {
            sto.push(format!("SSD wear (percentage used) = {w:.0}%"), above(w, t.ssd_wear_pct, "%"), "Vendor estimate of rated endurance consumed.");
        }
        if let Some(m) = d.media_errors {
            let sev = if m == 0.0 { Severity::Ok } else if m < 10.0 { Severity::Warning } else { Severity::Critical };
            sto.push(format!("Media and data integrity errors = {m}"), (sev, "1-9 → Warning, ≥10 → Critical".into()), "Unrecovered data integrity errors on the medium.");
        }
        if let (Some(sp), Some(th)) = (d.available_spare_pct, d.spare_threshold_pct) {
            let sev = if sp <= th { Severity::Critical } else if sp <= th + 10.0 { Severity::Warning } else { Severity::Ok };
            sto.push(format!("Available spare = {sp}% (threshold {th}%)"), (sev, "≤ threshold+10 → Warning, ≤ threshold → Critical".into()), "Reserve blocks for replacing worn flash.");
        }
        for (name, v) in [("Reallocated sectors", d.reallocated_sectors), ("Pending sectors", d.pending_sectors), ("Offline uncorrectable", d.uncorrectable)] {
            if let Some(v) = v {
                let sev = if v == 0.0 { Severity::Ok } else if v < 10.0 { Severity::Warning } else { Severity::Critical };
                sto.push(format!("{name} = {v}"), (sev, "1-9 → Warning, ≥10 → Critical".into()), "Failing sectors are an early sign of disk failure.");
            }
        }
    }
    for fs in &s.filesystems {
        let u = fs.used_pct();
        sto.push(format!("{} ({}) used = {u:.1}%", fs.mountpoint, fs.source), above(u, t.fs_used_pct, "%"), "Full filesystems break updates, logging and applications.");
    }

    // Thermals
    let mut thr = Cat::new("Thermals");
    for (kind, th, label) in [(SensorKind::Gpu, t.gpu_temp, "GPU"), (SensorKind::Storage, t.ssd_temp, "SSD")] {
        if let Some(v) = s.max_temp(kind) {
            thr.push(format!("{label} temperature = {v:.1} °C"), above(v, th, " °C"), "High temperatures shorten component life and trigger throttling.");
        }
    }
    if let Some(v) = s.max_temp(SensorKind::Cpu) {
        thr.push(format!("CPU temperature = {v:.1} °C"), above(v, t.cpu_temp, " °C"), "CPU temperature (also counted in CPU).");
    }
    for sensor in s.sensors.iter().filter(|x| x.value.is_some() && x.crit.is_some()) {
        let (v, c) = (sensor.value.unwrap_or(0.0), sensor.crit.unwrap_or(f64::MAX));
        if v >= c - 5.0 {
            thr.push(
                format!("{} {} = {v:.1} °C (hardware critical {c:.1} °C)", sensor.chip, sensor.label),
                (if v >= c { Severity::Critical } else { Severity::Warning }, "within 5 °C of the hardware critical limit → Warning, at/above → Critical".into()),
                "The hardware reports this temperature is at its critical limit.",
            );
        }
    }

    // Battery
    let mut bat = Cat::new("Battery");
    let health = s.batteries.first().and_then(|b| b.health_pct()).or(p.battery.as_ref().and_then(|b| b.health_pct()));
    if let Some(h) = health {
        bat.push(format!("Full charge capacity = {h:.1}% of design"), below(h, t.battery_health_pct, "%"), "Capacity loss shortens runtime; below ~60% replacement is usually worthwhile.");
    }

    // Boot
    let mut bootc = Cat::new("Boot");
    if let Some(total) = p.boot.as_ref().and_then(|b| b.total_s) {
        bootc.push(format!("Total boot time = {total:.1} s (systemd-analyze)"), above(total, t.boot_total_s, " s"), "Includes firmware and loader time when reported.");
    }

    // Kernel
    let mut ker = Cat::new("Kernel");
    if let Some((errs, warns)) = p.kernel_counts {
        ker.push(
            format!("Kernel log: {errs} errors, {warns} warnings since boot"),
            above(errs as f64, t.kernel_errors, " errors"),
            "Many kernel errors can indicate driver or hardware problems; some firmware (ACPI) errors are harmless.",
        );
    }

    let categories: Vec<CategoryScore> = [cpu, mem, sto, thr, bat, bootc, ker].into_iter().map(Cat::finish).collect();
    let scored: Vec<u8> = categories.iter().filter_map(|c| c.score).collect();
    let overall = (!scored.is_empty()).then(|| (scored.iter().map(|&v| v as u32).sum::<u32>() / scored.len() as u32) as u8);
    let mut state = overall.map_or(State::Unknown, State::from_score);
    if categories.iter().any(|c| c.state == State::Critical) && state < State::Warning {
        state = State::Warning;
    }
    HealthReport {
        overall,
        state,
        rule: format!(
            "Overall = average of the {} categories with data; a Critical category caps the overall state at Warning or worse.",
            scored.len()
        ),
        categories,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::{disks::Filesystem, memory::MemSample, power::HwmonSensor};
    use crate::parsers::storage::DriveHealth;

    fn sensor(kind: SensorKind, v: f64) -> HwmonSensor {
        HwmonSensor { chip: "chip".into(), label: "t".into(), kind, value: Some(v), high: None, crit: None }
    }

    #[test]
    fn empty_inputs_mean_insufficient_data() {
        let r = evaluate(&Sample::default(), &Parsed::default(), &Thresholds::default());
        assert_eq!(r.overall, None);
        assert_eq!(r.state, State::Unknown);
        assert!(r.categories.iter().all(|c| c.score.is_none() && c.state == State::Unknown));
    }

    #[test]
    fn healthy_system_scores_high() {
        let s = Sample {
            memory: MemSample { total_kb: 8_000_000, available_kb: 6_000_000, ..Default::default() },
            sensors: vec![sensor(SensorKind::Cpu, 50.0), sensor(SensorKind::Storage, 40.0)],
            filesystems: vec![Filesystem { used: 50, avail: 50, mountpoint: "/".into(), ..Default::default() }],
            ..Default::default()
        };
        let p = Parsed {
            drive: Some(DriveHealth { smart_passed: Some(true), percentage_used: Some(4.0), media_errors: Some(0.0), ..Default::default() }),
            ..Default::default()
        };
        let r = evaluate(&s, &p, &Thresholds::default());
        assert_eq!(r.category("Storage").and_then(|c| c.score), Some(100));
        assert_eq!(r.category("Battery").map(|c| c.state), Some(State::Unknown));
        assert!(r.overall.is_some_and(|o| o >= 95));
        assert!(r.problems().is_empty());
    }

    #[test]
    fn failing_drive_is_critical_and_caps_overall() {
        let s = Sample { memory: MemSample { total_kb: 100, available_kb: 90, ..Default::default() }, ..Default::default() };
        let p = Parsed {
            drive: Some(DriveHealth { smart_passed: Some(false), media_errors: Some(12.0), percentage_used: Some(97.0), ..Default::default() }),
            ..Default::default()
        };
        let r = evaluate(&s, &p, &Thresholds::default());
        let st = r.category("Storage").expect("storage");
        assert_eq!(st.state, State::Critical);
        assert_eq!(st.score, Some(0));
        assert!(r.state >= State::Warning);
        assert_eq!(r.problems()[0].1.severity, Severity::Critical);
        assert!(st.findings.iter().all(|f| !f.input.is_empty() && !f.rule.is_empty() && !f.reason.is_empty()));
    }

    #[test]
    fn thresholds_apply() {
        assert_eq!(above(85.0, [80.0, 90.0, 98.0], "").0, Severity::Attention);
        assert_eq!(above(99.0, [80.0, 90.0, 98.0], "").0, Severity::Critical);
        assert_eq!(below(55.0, [80.0, 60.0, 40.0], "").0, Severity::Warning);
        assert_eq!(below(90.0, [80.0, 60.0, 40.0], "").0, Severity::Ok);
    }
}
