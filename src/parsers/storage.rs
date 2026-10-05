//! NVMe smart-log, smartctl, nvme list, hdparm and du parsers.

use super::{kv_get, leading_num, parse_dashed_table, parse_kv};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Unified drive health from `nvme smart-log` and/or `smartctl`.
/// Every field is optional: absent means "not reported by the device".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DriveHealth {
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub capacity: Option<String>,
    /// SMART overall-health self-assessment (smartctl).
    pub smart_passed: Option<bool>,
    pub critical_warning: Option<u64>,
    pub temperature_c: Option<f64>,
    pub available_spare_pct: Option<f64>,
    pub spare_threshold_pct: Option<f64>,
    pub percentage_used: Option<f64>,
    pub data_read_bytes: Option<f64>,
    pub data_written_bytes: Option<f64>,
    pub power_on_hours: Option<f64>,
    pub power_cycles: Option<f64>,
    pub unsafe_shutdowns: Option<f64>,
    pub media_errors: Option<f64>,
    pub error_log_entries: Option<f64>,
    /// ATA only.
    pub reallocated_sectors: Option<f64>,
    pub pending_sectors: Option<f64>,
    pub uncorrectable: Option<f64>,
    /// Every field the device returned, in order.
    pub fields: Vec<(String, String)>,
    /// ATA attribute table rows (ID, name, value, worst, thresh, raw).
    pub ata_attributes: Vec<Vec<String>>,
}

/// One NVMe "data unit" is 1000 * 512 bytes.
const NVME_UNIT: f64 = 512_000.0;

fn norm(k: &str) -> String {
    k.trim().to_lowercase().replace([' ', '-'], "_")
}

/// `nvme smart-log /dev/nvme0`
pub fn parse_nvme_smart_log(text: &str) -> DriveHealth {
    let fields: Vec<(String, String)> = text.lines().filter(|l| !l.starts_with("Smart Log")).filter_map(|l| {
        let (k, v) = l.split_once(':')?;
        Some((k.trim().to_string(), v.trim().to_string()))
    }).collect();
    let get = |key: &str| fields.iter().find(|(k, _)| norm(k) == key).and_then(|(_, v)| leading_num(v));
    DriveHealth {
        critical_warning: get("critical_warning").map(|v| v as u64),
        temperature_c: get("temperature"),
        available_spare_pct: get("available_spare"),
        spare_threshold_pct: get("available_spare_threshold"),
        percentage_used: get("percentage_used"),
        data_read_bytes: get("data_units_read").map(|v| v * NVME_UNIT),
        data_written_bytes: get("data_units_written").map(|v| v * NVME_UNIT),
        power_on_hours: get("power_on_hours"),
        power_cycles: get("power_cycles"),
        unsafe_shutdowns: get("unsafe_shutdowns"),
        media_errors: get("media_errors"),
        error_log_entries: get("num_err_log_entries"),
        fields,
        ..Default::default()
    }
}

fn hex_or_dec(v: &str) -> Option<u64> {
    let v = v.split_whitespace().next()?;
    match v.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => v.replace(',', "").parse().ok(),
    }
}

/// `smartctl -a|-x <dev>` (NVMe and ATA).
pub fn parse_smartctl(text: &str) -> DriveHealth {
    let fields: Vec<(String, String)> = parse_kv(text)
        .into_iter()
        .filter(|(k, _)| !k.starts_with("smartctl ") && !k.starts_with("Copyright"))
        .collect();
    let num = |k: &str| kv_get(&fields, k).and_then(leading_num);
    let mut h = DriveHealth {
        model: kv_get(&fields, "Model Number").or(kv_get(&fields, "Device Model")).map(str::to_string),
        firmware: kv_get(&fields, "Firmware Version").map(str::to_string),
        capacity: kv_get(&fields, "Total NVM Capacity")
            .or(kv_get(&fields, "Namespace 1 Size/Capacity"))
            .or(kv_get(&fields, "User Capacity"))
            .map(str::to_string),
        smart_passed: kv_get(&fields, "SMART overall-health self-assessment test result")
            .or(kv_get(&fields, "SMART Health Status"))
            .map(|v| v.contains("PASSED") || v == "OK"),
        critical_warning: kv_get(&fields, "Critical Warning").and_then(hex_or_dec),
        temperature_c: num("Temperature"),
        available_spare_pct: num("Available Spare"),
        spare_threshold_pct: num("Available Spare Threshold"),
        percentage_used: num("Percentage Used"),
        data_read_bytes: num("Data Units Read").map(|v| v * NVME_UNIT),
        data_written_bytes: num("Data Units Written").map(|v| v * NVME_UNIT),
        power_on_hours: num("Power On Hours"),
        power_cycles: num("Power Cycles"),
        unsafe_shutdowns: num("Unsafe Shutdowns"),
        media_errors: num("Media and Data Integrity Errors"),
        error_log_entries: num("Error Information Log Entries"),
        ..Default::default()
    };
    // ATA attribute table.
    let mut in_table = false;
    for l in text.lines() {
        if l.starts_with("ID# ATTRIBUTE_NAME") {
            in_table = true;
            continue;
        }
        if in_table {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if cols.len() < 10 || cols[0].parse::<u32>().is_err() {
                if l.trim().is_empty() || !l.starts_with(' ') {
                    in_table = false;
                }
                continue;
            }
            let raw = cols[9..].join(" ");
            let raw_n = leading_num(&raw);
            match cols[1] {
                "Reallocated_Sector_Ct" => h.reallocated_sectors = raw_n,
                "Current_Pending_Sector" => h.pending_sectors = raw_n,
                "Offline_Uncorrectable" => h.uncorrectable = raw_n,
                "Power_On_Hours" if h.power_on_hours.is_none() => h.power_on_hours = raw_n,
                "Power_Cycle_Count" if h.power_cycles.is_none() => h.power_cycles = raw_n,
                "Temperature_Celsius" | "Airflow_Temperature_Cel" if h.temperature_c.is_none() => h.temperature_c = raw_n,
                "Wear_Leveling_Count" | "Percent_Lifetime_Remain" | "SSD_Life_Left" if h.percentage_used.is_none() => {
                    // Normalized VALUE counts down from 100.
                    h.percentage_used = cols[3].parse::<f64>().ok().map(|v| (100.0 - v).max(0.0));
                }
                _ => {}
            }
            h.ata_attributes.push(vec![
                cols[0].into(),
                cols[1].into(),
                cols[3].into(),
                cols[4].into(),
                cols[5].into(),
                raw,
            ]);
        }
    }
    h.fields = fields;
    h
}

impl DriveHealth {
    /// Fill missing values of `self` from `other` (nvme smart-log wins over smartctl).
    pub fn merge(mut self, other: &DriveHealth) -> DriveHealth {
        macro_rules! fill {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = other.$f.clone(); } )* };
        }
        fill!(model, firmware, capacity, smart_passed, critical_warning, temperature_c, available_spare_pct, spare_threshold_pct,
              percentage_used, data_read_bytes, data_written_bytes, power_on_hours, power_cycles, unsafe_shutdowns, media_errors,
              error_log_entries, reallocated_sectors, pending_sectors, uncorrectable);
        if self.ata_attributes.is_empty() {
            self.ata_attributes = other.ata_attributes.clone();
        }
        self
    }

    /// `true` when at least one health value was parsed.
    pub fn has_data(&self) -> bool {
        self.percentage_used.is_some() || self.smart_passed.is_some() || self.media_errors.is_some() || self.temperature_c.is_some()
    }
}

/// One row of `nvme list`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NvmeDevice {
    pub node: String,
    pub model: String,
    pub serial: String,
    pub usage: String,
    pub firmware: String,
}

/// `nvme list` (classic table format).
pub fn parse_nvme_list(text: &str) -> Vec<NvmeDevice> {
    parse_dashed_table(text)
        .into_iter()
        .map(|r: BTreeMap<String, String>| {
            let g = |k: &str| r.get(k).cloned().unwrap_or_default();
            NvmeDevice { node: g("Node"), model: g("Model"), serial: g("SN"), usage: g("Usage"), firmware: g("FW Rev") }
        })
        .filter(|d| d.node.starts_with("/dev/"))
        .collect()
}

/// `hdparm -Tt` throughput in MB/s: (cached, buffered).
pub fn parse_hdparm(text: &str) -> (Option<f64>, Option<f64>) {
    let rate = |needle: &str| {
        text.lines()
            .find(|l| l.contains(needle))
            .and_then(|l| l.split('=').nth(1))
            .and_then(leading_num)
    };
    (rate("Timing cached reads"), rate("Timing buffered disk reads"))
}

/// `du -xh DIR --max-depth=1` rows: (size string, bytes, path).
pub fn parse_du(text: &str) -> Vec<(String, f64, String)> {
    text.lines()
        .filter_map(|l| {
            let (size, path) = l.split_once(char::is_whitespace)?;
            Some((size.to_string(), crate::diagnostics::runner::parse_human_size(size), path.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVME: &str = "Smart Log for NVME device:nvme0 namespace-id:ffffffff
critical_warning			: 0
temperature				: 51 °C (324 K)
available_spare				: 100%
available_spare_threshold		: 10%
percentage_used				: 4%
endurance group critical warning summary: 0
Data Units Read				: 12,345,678 (6.32 TB)
Data Units Written			: 9,876,543 (5.06 TB)
host_read_commands			: 123,456,789
host_write_commands			: 98,765,432
controller_busy_time			: 1,234
power_cycles				: 2,345
power_on_hours				: 3,456
unsafe_shutdowns			: 156
media_errors				: 0
num_err_log_entries			: 7
Warning Temperature Time		: 0
Critical Composite Temperature Time	: 0
Temperature Sensor 1           : 51 °C (324 K)
";

    #[test]
    fn nvme_smart_log() {
        let h = parse_nvme_smart_log(NVME);
        assert_eq!(h.critical_warning, Some(0));
        assert_eq!(h.temperature_c, Some(51.0));
        assert_eq!(h.percentage_used, Some(4.0));
        assert_eq!(h.available_spare_pct, Some(100.0));
        assert_eq!(h.spare_threshold_pct, Some(10.0));
        assert_eq!(h.power_on_hours, Some(3456.0));
        assert_eq!(h.unsafe_shutdowns, Some(156.0));
        assert_eq!(h.media_errors, Some(0.0));
        assert_eq!(h.error_log_entries, Some(7.0));
        assert_eq!(h.data_written_bytes, Some(9_876_543.0 * 512_000.0));
        assert_eq!(h.fields.len(), 19);
    }

    const SMART_NVME: &str = "smartctl 7.4 2023-08-01 r5530 [x86_64-linux-6.8.0] (local build)
Copyright (C) 2002-23, Bruce Allen, Christian Franke, www.smartmontools.org

=== START OF INFORMATION SECTION ===
Model Number:                       WDC PC SN530 SDBPNPZ-512G-1006
Serial Number:                      2002XX000000
Firmware Version:                   21106000
Total NVM Capacity:                 512,110,190,592 [512 GB]

=== START OF SMART DATA SECTION ===
SMART overall-health self-assessment test result: PASSED

SMART/Health Information (NVMe Log 0x02)
Critical Warning:                   0x00
Temperature:                        51 Celsius
Available Spare:                    100%
Available Spare Threshold:          10%
Percentage Used:                    4%
Data Units Read:                    12,345,678 [6.32 TB]
Data Units Written:                 9,876,543 [5.05 TB]
Power Cycles:                       2,345
Power On Hours:                     3,456
Unsafe Shutdowns:                   156
Media and Data Integrity Errors:    0
Error Information Log Entries:      0
";

    #[test]
    fn smartctl_nvme() {
        let h = parse_smartctl(SMART_NVME);
        assert_eq!(h.model.as_deref(), Some("WDC PC SN530 SDBPNPZ-512G-1006"));
        assert_eq!(h.smart_passed, Some(true));
        assert_eq!(h.critical_warning, Some(0));
        assert_eq!(h.temperature_c, Some(51.0));
        assert_eq!(h.percentage_used, Some(4.0));
        assert_eq!(h.power_on_hours, Some(3456.0));
        assert_eq!(h.media_errors, Some(0.0));
        assert!(h.fields.iter().all(|(k, _)| !k.starts_with("smartctl")));
    }

    const SMART_ATA: &str = "=== START OF INFORMATION SECTION ===
Device Model:     Samsung SSD 860 EVO 500GB
User Capacity:    500,107,862,016 bytes [500 GB]
SMART overall-health self-assessment test result: FAILED!

ID# ATTRIBUTE_NAME          FLAG     VALUE WORST THRESH TYPE      UPDATED  WHEN_FAILED RAW_VALUE
  5 Reallocated_Sector_Ct   0x0033   100   100   010    Pre-fail  Always       -       12
  9 Power_On_Hours          0x0032   095   095   000    Old_age   Always       -       21034
177 Wear_Leveling_Count     0x0013   093   093   000    Pre-fail  Always       -       71
190 Airflow_Temperature_Cel 0x0032   067   052   000    Old_age   Always       -       33
197 Current_Pending_Sector  0x0032   100   100   000    Old_age   Always       -       0

SMART Error Log Version: 1
";

    #[test]
    fn smartctl_ata() {
        let h = parse_smartctl(SMART_ATA);
        assert_eq!(h.smart_passed, Some(false));
        assert_eq!(h.reallocated_sectors, Some(12.0));
        assert_eq!(h.power_on_hours, Some(21034.0));
        assert_eq!(h.percentage_used, Some(7.0));
        assert_eq!(h.temperature_c, Some(33.0));
        assert_eq!(h.pending_sectors, Some(0.0));
        assert_eq!(h.ata_attributes.len(), 5);
    }

    #[test]
    fn merge_prefers_self() {
        let a = parse_nvme_smart_log(NVME);
        let b = parse_smartctl(SMART_NVME);
        let m = a.merge(&b);
        assert_eq!(m.model.as_deref(), Some("WDC PC SN530 SDBPNPZ-512G-1006"));
        assert_eq!(m.error_log_entries, Some(7.0));
        assert_eq!(m.smart_passed, Some(true));
    }

    #[test]
    fn hdparm_and_du() {
        let t = "\n/dev/nvme0n1:\n Timing cached reads:   10422 MB in  2.00 seconds = 5217.43 MB/sec\n Timing buffered disk reads: 3540 MB in  3.00 seconds = 1179.85 MB/sec\n";
        assert_eq!(parse_hdparm(t), (Some(5217.43), Some(1179.85)));
        let d = parse_du("4.0K\t/srv\n1.2G\t/usr\n");
        assert_eq!(d[1].2, "/usr");
        assert!(d[1].1 > 1e9);
    }
}
