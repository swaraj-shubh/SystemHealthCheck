//! A diagnostic snapshot (raw results + native sample) and the structured
//! view derived from it. Only raw data is persisted; [`Parsed`] is recomputed
//! on load so parser improvements apply to old scans too.

use super::runner::CmdOutput;
use super::scheduler::ScanKind;
use crate::collectors::{system::SystemInfo, Sample};
use crate::parsers::{self, bench, boot, hardware, kernel, packages, power, storage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub taken_at: i64,
    pub kind: ScanKind,
    pub system: SystemInfo,
    pub sample: Sample,
    pub results: BTreeMap<String, CmdOutput>,
}

/// Structured data parsed from whatever results are available.
#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub lscpu: Vec<(String, String)>,
    pub cpupower: Vec<(String, String)>,
    pub dmi_memory: Option<hardware::DmiMemory>,
    pub glxinfo: Vec<(String, String)>,
    pub pci: Vec<hardware::PciDevice>,
    pub usb: Vec<hardware::UsbDevice>,
    pub usb_details: Vec<(String, String)>,
    pub drive: Option<storage::DriveHealth>,
    pub nvme_list: Vec<storage::NvmeDevice>,
    pub sensors: Vec<power::SensorChip>,
    pub battery: Option<power::UpowerBattery>,
    pub boot: Option<boot::BootTimes>,
    pub blame: Vec<(f64, String)>,
    pub chain: Vec<boot::ChainNode>,
    pub enabled_units: Vec<(String, String, String)>,
    pub kernel_msgs: Vec<kernel::KernelMsg>,
    /// (errors, warnings) when severities are known.
    pub kernel_counts: Option<(usize, usize)>,
    pub os_release: Vec<(String, String)>,
    pub hostnamectl: Vec<(String, String)>,
    pub lshw: Vec<hardware::LshwNode>,
    pub lshw_short: Vec<hardware::LshwShortRow>,
    pub network_hw: Vec<hardware::LshwNode>,
    pub wifi: Vec<hardware::WifiIface>,
    pub nmcli: Vec<[String; 4]>,
    pub baseboard: Option<parsers::DmiSection>,
    pub system_dmi: Option<parsers::DmiSection>,
    pub serial: Option<String>,
    pub top_mem: Vec<hardware::PsRow>,
    pub top_cpu: Vec<hardware::PsRow>,
    pub upgradable: Vec<(String, String, String)>,
    pub autoremove: Vec<(String, String)>,
    pub policy: Vec<packages::PolicyEntry>,
    pub sysbench: Option<bench::SysbenchResult>,
    pub stress: Option<bench::StressResult>,
    pub hdparm: (Option<f64>, Option<f64>),
}

/// Parse every available successful result.
pub fn parse_all(r: &BTreeMap<String, CmdOutput>) -> Parsed {
    let t = |id: &str| r.get(id).and_then(|o| o.text());
    let first = |ids: &[&str]| ids.iter().find_map(|id| t(id));
    let mut p = Parsed {
        lscpu: t("lscpu").map(kernel::parse_colon_listing).unwrap_or_default(),
        cpupower: t("cpupower").map(parsers::parse_kv).unwrap_or_default(),
        dmi_memory: first(&["dmidecode-memory", "dmidecode-memory-long"]).map(hardware::parse_dmidecode_memory),
        glxinfo: t("glxinfo").map(parsers::parse_kv).unwrap_or_default(),
        pci: t("lspci-vmm").map(hardware::parse_lspci_vmm).unwrap_or_default(),
        usb: t("lsusb").map(hardware::parse_lsusb).unwrap_or_default(),
        usb_details: t("lsusb-v").map(hardware::split_lsusb_verbose).unwrap_or_default(),
        nvme_list: t("nvme-list").map(storage::parse_nvme_list).unwrap_or_default(),
        sensors: first(&["sensors", "watch-sensors"]).map(power::parse_sensors).unwrap_or_default(),
        battery: t("upower-battery").and_then(power::parse_upower),
        boot: t("systemd-analyze").and_then(boot::parse_systemd_analyze),
        blame: first(&["systemd-blame", "systemd-blame-20"]).map(boot::parse_blame).unwrap_or_default(),
        chain: t("systemd-critical-chain").map(boot::parse_critical_chain).unwrap_or_default(),
        enabled_units: t("systemctl-enabled").map(boot::parse_unit_files).unwrap_or_default(),
        os_release: t("os-release").map(kernel::parse_os_release).unwrap_or_default(),
        hostnamectl: t("hostnamectl").map(kernel::parse_colon_listing).unwrap_or_default(),
        lshw: t("lshw").map(hardware::parse_lshw_tree).unwrap_or_default(),
        lshw_short: t("lshw-short").map(hardware::parse_lshw_short).unwrap_or_default(),
        network_hw: t("lshw-network").map(hardware::parse_lshw_tree).unwrap_or_default(),
        wifi: t("iw-dev").map(hardware::parse_iw_dev).unwrap_or_default(),
        nmcli: t("nmcli-status").map(hardware::parse_nmcli).unwrap_or_default(),
        baseboard: t("dmidecode-baseboard").and_then(|x| hardware::dmi_section(x, "Base Board Information")),
        system_dmi: t("dmidecode-system").and_then(|x| hardware::dmi_section(x, "System Information")),
        serial: t("dmidecode-serial").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        top_mem: first(&["ps-mem-20", "ps-mem-15"]).map(hardware::parse_ps_aux).unwrap_or_default(),
        top_cpu: t("ps-cpu-15").map(hardware::parse_ps_aux).unwrap_or_default(),
        upgradable: t("apt-upgradable").map(packages::parse_apt_upgradable).unwrap_or_default(),
        autoremove: t("apt-autoremove-preview").map(packages::parse_apt_simulate).unwrap_or_default(),
        policy: t("apt-policy").map(packages::parse_apt_policy).unwrap_or_default(),
        sysbench: t("sysbench-cpu").and_then(bench::parse_sysbench),
        stress: r.get("stress-ng-cpu").map(|o| bench::parse_stress_ng(&format!("{}\n{}", o.stdout, o.stderr))),
        hdparm: t("hdparm").map(storage::parse_hdparm).unwrap_or((None, None)),
        ..Default::default()
    };
    // Drive health: nvme smart-log first, then smartctl fills the gaps.
    let nvme = t("nvme-smart-log").map(storage::parse_nvme_smart_log);
    let smart = first(&["smartctl-x", "smartctl-a"]).map(storage::parse_smartctl);
    p.drive = match (nvme, smart) {
        (Some(n), Some(s)) => Some(n.merge(&s)),
        (n, s) => n.or(s),
    };
    // Kernel messages: the decoded form carries severities.
    if let Some(d) = t("dmesg-decoded") {
        p.kernel_msgs = kernel::parse_dmesg(d);
        let errs = p.kernel_msgs.iter().filter(|m| m.severity == kernel::KSeverity::Error).count();
        p.kernel_counts = Some((errs, p.kernel_msgs.len() - errs));
    } else if let Some(d) = first(&["dmesg-errwarn", "dmesg-errwarn-50"]) {
        p.kernel_msgs = kernel::parse_dmesg(d);
    }
    p
}

impl Snapshot {
    pub fn parsed(&self) -> Parsed {
        parse_all(&self.results)
    }
}

impl Parsed {
    /// Identifier values found in the output (serials, UUIDs) for masking.
    pub fn secrets(&self) -> Vec<String> {
        let keys = ["Serial Number", "UUID", "serial", "Asset Tag"];
        let from_kv = |kv: &[(String, String)]| -> Vec<String> {
            kv.iter().filter(|(k, _)| keys.iter().any(|x| k.eq_ignore_ascii_case(x))).map(|(_, v)| v.clone()).collect()
        };
        let mut v: Vec<String> = self.serial.iter().cloned().collect();
        v.extend(self.nvme_list.iter().map(|d| d.serial.clone()));
        if let Some(d) = &self.drive {
            v.extend(from_kv(&d.fields));
        }
        if let Some(b) = &self.battery {
            v.extend(from_kv(&b.fields));
        }
        for s in self.system_dmi.iter().chain(self.baseboard.iter()) {
            v.extend(from_kv(&s.fields));
        }
        if let Some(m) = &self.dmi_memory {
            for module in &m.modules {
                v.extend(from_kv(&module.fields));
            }
        }
        let ignore = ["Not Specified", "To Be Filled By O.E.M.", "Default string", "Unknown", "None", "N/A"];
        v.retain(|s| s.len() >= 4 && !ignore.iter().any(|i| s.eq_ignore_ascii_case(i)));
        v.sort();
        v.dedup();
        v
    }
}
