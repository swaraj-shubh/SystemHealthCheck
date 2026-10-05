//! Quick Scan / Full Diagnostic orchestration.
//!
//! Privileged commands are batched behind a single pkexec prompt; the rest
//! run per group, sequentially or in parallel. Progress is reported through a
//! callback so the UI can show the checklist-style progress page.

use super::registry::{self, CommandSpec};
use super::runner::{self, CmdOutput, Params, RunControl};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Full Diagnostic groups (spec §40), in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Group {
    System,
    Cpu,
    Ram,
    Swap,
    Processes,
    Gpu,
    Storage,
    Smart,
    Thermal,
    Battery,
    Boot,
    Linux,
    Pci,
    Usb,
    Network,
    Kernel,
    Motherboard,
    SystemModel,
}

impl Group {
    pub const ALL: [Group; 18] = [
        Group::System,
        Group::Cpu,
        Group::Ram,
        Group::Swap,
        Group::Processes,
        Group::Gpu,
        Group::Storage,
        Group::Smart,
        Group::Thermal,
        Group::Battery,
        Group::Boot,
        Group::Linux,
        Group::Pci,
        Group::Usb,
        Group::Network,
        Group::Kernel,
        Group::Motherboard,
        Group::SystemModel,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::System => "System",
            Group::Cpu => "CPU",
            Group::Ram => "RAM",
            Group::Swap => "Swap",
            Group::Processes => "Processes",
            Group::Gpu => "GPU",
            Group::Storage => "Storage",
            Group::Smart => "SMART",
            Group::Thermal => "Thermals",
            Group::Battery => "Battery",
            Group::Boot => "Boot",
            Group::Linux => "Linux",
            Group::Pci => "PCI",
            Group::Usb => "USB",
            Group::Network => "Network",
            Group::Kernel => "Kernel",
            Group::Motherboard => "Motherboard",
            Group::SystemModel => "System Model",
        }
    }

    /// Registry ids executed for this group.
    pub fn ids(self) -> &'static [&'static str] {
        match self {
            Group::System => &["lshw-short", "lshw"],
            Group::Cpu => &["lscpu", "lscpu-filtered", "cpu-governor", "cpu-governors", "cpupower", "watch-cpu-mhz"],
            Group::Ram => &["free", "dmidecode-memory", "dmidecode-memory-long", "dmidecode-memory-filtered"],
            Group::Swap => &["swapon", "swappiness"],
            Group::Processes => &["ps-cpu-15", "ps-mem-15", "ps-mem-20"],
            Group::Gpu => &["lspci-gpu", "lspci-gpu-driver", "glxinfo"],
            Group::Storage => &["lsblk", "nvme-list", "df"],
            Group::Smart => &["nvme-smart-log", "smartctl-a", "smartctl-x"],
            Group::Thermal => &["sensors", "watch-sensors"],
            Group::Battery => &["upower-battery"],
            Group::Boot => &["systemd-analyze", "systemd-blame", "systemd-blame-20", "systemd-critical-chain", "systemctl-enabled"],
            Group::Linux => &["uname", "os-release", "hostnamectl"],
            Group::Pci => &["lspci", "lspci-vmm"],
            Group::Usb => &["lsusb", "lsusb-v"],
            Group::Network => &["lshw-network", "iw-dev", "nmcli-status"],
            Group::Kernel => &["dmesg-errwarn", "dmesg-errwarn-50", "dmesg-decoded"],
            Group::Motherboard => &["dmidecode-baseboard", "dmidecode-board-mfr", "dmidecode-board-product"],
            Group::SystemModel => &["dmidecode-system", "dmidecode-product", "dmidecode-version", "dmidecode-serial"],
        }
    }
}

/// The checklist's "10 most valuable commands".
pub const QUICK_SCAN: &[&str] = &[
    "dmidecode-memory",
    "lscpu",
    "free",
    "swapon",
    "glxinfo",
    "nvme-smart-log",
    "upower-battery",
    "sensors",
    "systemd-analyze",
    "ps-mem-20",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanKind {
    Quick,
    Full,
}

impl ScanKind {
    pub fn label(self) -> &'static str {
        match self {
            ScanKind::Quick => "Quick Scan",
            ScanKind::Full => "Full Diagnostic",
        }
    }
}

/// Progress events for the UI.
#[derive(Debug, Clone)]
pub enum ScanEvent {
    /// Human description of the current step.
    Current(String),
    /// A group finished: (group, ok count, total count).
    GroupDone(Group, usize, usize),
    /// One command finished.
    Command(CmdOutput),
}

/// A unit of work: a label and its registry ids.
pub struct ScanPlan {
    pub steps: Vec<(Option<Group>, Vec<&'static CommandSpec>)>,
}

impl ScanPlan {
    pub fn quick() -> Self {
        ScanPlan { steps: vec![(None, QUICK_SCAN.iter().filter_map(|id| registry::get(id)).collect())] }
    }

    pub fn full(groups: &[Group]) -> Self {
        ScanPlan {
            steps: groups
                .iter()
                .map(|g| (Some(*g), g.ids().iter().filter_map(|id| registry::get(id)).filter(|s| !s.is_manual_only()).collect()))
                .collect(),
        }
    }

    pub fn ids(ids: &[&str]) -> Self {
        ScanPlan { steps: vec![(None, ids.iter().filter_map(|id| registry::get(id)).filter(|s| !s.is_manual_only()).collect())] }
    }
}

/// Execute a plan. Blocking: call from a worker thread.
pub fn run_plan(plan: &ScanPlan, parallel: bool, ctl: &RunControl, on_event: &(dyn Fn(ScanEvent) + Sync)) -> BTreeMap<String, CmdOutput> {
    let results = Mutex::new(BTreeMap::new());
    let record = |out: CmdOutput| {
        on_event(ScanEvent::Command(out.clone()));
        if let Ok(mut r) = results.lock() {
            r.insert(out.id.clone(), out);
        }
    };
    // 1. One authentication prompt for everything privileged.
    let mut seen = std::collections::HashSet::new();
    let privileged: Vec<(&'static CommandSpec, Params)> = plan
        .steps
        .iter()
        .flat_map(|(_, specs)| specs.iter())
        .filter(|s| s.requires_sudo && seen.insert(s.id))
        .map(|s| (*s, runner::default_params(s)))
        .collect();
    if !privileged.is_empty() {
        on_event(ScanEvent::Current(format!("Requesting administrator privileges for {} commands…", privileged.len())));
        for out in runner::run_privileged_batch(&privileged, ctl) {
            record(out);
        }
    }
    // 2. Unprivileged commands, per group.
    let run_step = |group: Option<Group>, specs: &Vec<&'static CommandSpec>| {
        if let Some(g) = group {
            on_event(ScanEvent::Current(format!("Scanning {}…", g.label().to_lowercase())));
        }
        for s in specs.iter().filter(|s| !s.requires_sudo) {
            if ctl.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            if group.is_none() {
                on_event(ScanEvent::Current(format!("Running {}…", s.original)));
            }
            record(runner::run(s, &runner::default_params(s), ctl));
        }
        if let Some(g) = group {
            let r = results.lock().map(|r| {
                let ok = specs.iter().filter(|s| r.get(s.id).is_some_and(|o| o.ok())).count();
                (ok, specs.len())
            });
            if let Ok((ok, total)) = r {
                on_event(ScanEvent::GroupDone(g, ok, total));
            }
        }
    };
    if parallel {
        std::thread::scope(|sc| {
            for (g, specs) in &plan.steps {
                sc.spawn(|| run_step(*g, specs));
            }
        });
    } else {
        for (g, specs) in &plan.steps {
            run_step(*g, specs);
        }
    }
    results.into_inner().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_reference_real_ids() {
        for g in Group::ALL {
            for id in g.ids() {
                assert!(registry::get(id).is_some(), "{id}");
            }
        }
        for id in QUICK_SCAN {
            assert!(registry::get(id).is_some(), "{id}");
        }
        assert_eq!(QUICK_SCAN.len(), 10);
    }

    #[test]
    fn full_scan_excludes_dangerous_commands() {
        let plan = ScanPlan::full(&Group::ALL);
        for (_, specs) in &plan.steps {
            for s in specs {
                assert!(!s.needs_confirmation(), "{}", s.id);
            }
        }
    }

    #[test]
    fn runs_unprivileged_plan() {
        let plan = ScanPlan::ids(&["uname", "os-release", "swappiness"]);
        let res = run_plan(&plan, true, &RunControl::default(), &|_| {});
        assert_eq!(res.len(), 3);
        assert!(res.values().all(|r| r.ok()));
    }
}
