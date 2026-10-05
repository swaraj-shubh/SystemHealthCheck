//! User settings, persisted as one JSON document in SQLite.

use crate::scoring::Thresholds;
use serde::{Deserialize, Serialize};

pub const DASHBOARD_CARDS: &[&str] = &["Health", "CPU", "Memory", "GPU", "Storage", "SSD Health", "Battery", "Thermals", "Boot", "Kernel", "Network"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub refresh_ms: u64,
    pub thermal_refresh_ms: u64,
    pub start_monitoring: bool,
    pub quick_scan_on_start: bool,
    pub dashboard_cards: Vec<String>,
    pub hidden_cards: Vec<String>,
    pub compact: bool,
    pub show_raw_on_dashboard: bool,
    /// Group labels run by Full Diagnostic.
    pub full_groups: Vec<String>,
    pub include_benchmark_in_full: bool,
    pub include_stress_in_full: bool,
    pub parallel: bool,
    pub command_timeout_s: u64,
    pub mask_serial: bool,
    pub mask_hostname: bool,
    pub exclude_network_from_reports: bool,
    pub history_retention_days: u32,
    pub alert_temperature: bool,
    pub alert_temperature_c: f64,
    pub emergency_stop_c: f64,
    pub alert_battery: bool,
    pub alert_battery_pct: f64,
    pub alert_storage: bool,
    pub alert_storage_pct: f64,
    pub alert_kernel: bool,
    /// Not persisted yet: always the documented defaults until thresholds
    /// become user-editable (so improved defaults reach existing installs).
    #[serde(skip)]
    pub thresholds: Thresholds,
    pub first_run_done: bool,
    /// Unprivileged checks run automatically at startup.
    pub auto_ids: Vec<String>,
}

/// Cheap, unprivileged checks that populate the pages at startup.
pub const DEFAULT_AUTO_IDS: &[&str] = &[
    "lscpu", "cpu-governor", "cpu-governors", "cpupower", "free", "swapon", "swappiness", "glxinfo", "lspci-vmm", "lspci-gpu-driver",
    "lsblk", "df", "sensors", "upower-battery", "systemd-analyze", "systemd-blame", "systemd-critical-chain", "systemctl-enabled",
    "uname", "os-release", "hostnamectl", "lsusb", "iw-dev", "nmcli-status", "dmesg-decoded", "dmesg-errwarn",
];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            refresh_ms: 1000,
            thermal_refresh_ms: 2000,
            start_monitoring: true,
            quick_scan_on_start: false,
            dashboard_cards: DASHBOARD_CARDS.iter().map(|s| s.to_string()).collect(),
            hidden_cards: Vec::new(),
            compact: false,
            show_raw_on_dashboard: false,
            full_groups: crate::diagnostics::scheduler::Group::ALL.iter().map(|g| g.label().to_string()).collect(),
            include_benchmark_in_full: false,
            include_stress_in_full: false,
            parallel: true,
            command_timeout_s: 60,
            mask_serial: true,
            mask_hostname: false,
            exclude_network_from_reports: true,
            history_retention_days: 365,
            alert_temperature: true,
            alert_temperature_c: 90.0,
            emergency_stop_c: 95.0,
            alert_battery: true,
            alert_battery_pct: 10.0,
            alert_storage: true,
            alert_storage_pct: 95.0,
            alert_kernel: true,
            thresholds: Thresholds::default(),
            first_run_done: false,
            auto_ids: DEFAULT_AUTO_IDS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Settings {
    /// Visible dashboard cards in the user's order (unknown names dropped,
    /// newly added cards appended).
    pub fn visible_cards(&self) -> Vec<String> {
        let mut order: Vec<String> = self.dashboard_cards.iter().filter(|c| DASHBOARD_CARDS.contains(&c.as_str())).cloned().collect();
        for c in DASHBOARD_CARDS {
            if !order.iter().any(|o| o == c) {
                order.push(c.to_string());
            }
        }
        order.retain(|c| !self.hidden_cards.contains(c));
        order
    }

    pub fn groups(&self) -> Vec<crate::diagnostics::scheduler::Group> {
        crate::diagnostics::scheduler::Group::ALL.into_iter().filter(|g| self.full_groups.iter().any(|l| l == g.label())).collect()
    }
}
