//! Temperatures/fans (hwmon) and batteries/AC (power_supply class).

use crate::util::{read_num, read_trim};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensorKind {
    Cpu,
    Gpu,
    Storage,
    Board,
    Battery,
    Other,
}

impl SensorKind {
    pub fn label(self) -> &'static str {
        match self {
            SensorKind::Cpu => "CPU",
            SensorKind::Gpu => "GPU",
            SensorKind::Storage => "SSD/Disk",
            SensorKind::Board => "Motherboard/ACPI",
            SensorKind::Battery => "Battery",
            SensorKind::Other => "Other",
        }
    }
}

/// Classify a hwmon chip name.
pub fn classify(chip: &str) -> SensorKind {
    let c = chip.to_lowercase();
    if ["k10temp", "coretemp", "zenpower", "cpu_thermal", "cpu-thermal", "x86_pkg_temp", "fam15h_power"].iter().any(|p| c.starts_with(p)) {
        SensorKind::Cpu
    } else if ["amdgpu", "radeon", "nouveau", "i915", "xe", "nvidia"].iter().any(|p| c.starts_with(p)) {
        SensorKind::Gpu
    } else if c.starts_with("nvme") || c.starts_with("drivetemp") {
        SensorKind::Storage
    } else if c.starts_with("bat") {
        SensorKind::Battery
    } else if ["acpitz", "pch_", "thinkpad", "dell_smm", "hp", "asus", "nct", "it87", "f71", "w83"].iter().any(|p| c.starts_with(p)) {
        SensorKind::Board
    } else {
        SensorKind::Other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwmonSensor {
    pub chip: String,
    pub label: String,
    pub kind: SensorKind,
    /// °C for temperatures, RPM for fans. `None` when the read failed.
    pub value: Option<f64>,
    pub high: Option<f64>,
    pub crit: Option<f64>,
}

/// Read every hwmon temperature and fan. Returns (temperatures, fans).
pub fn read_hwmon() -> (Vec<HwmonSensor>, Vec<HwmonSensor>) {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hwmon") else { return (temps, fans) };
    let mut entries: Vec<_> = dir.flatten().map(|e| e.path()).collect();
    entries.sort();
    for hw in entries {
        let chip = read_trim(hw.join("name")).unwrap_or_else(|| "hwmon".into());
        let kind = classify(&chip);
        let Ok(files) = std::fs::read_dir(&hw) else { continue };
        let mut names: Vec<String> = files.flatten().map(|f| f.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for f in names {
            let Some(prefix) = f.strip_suffix("_input") else { continue };
            let is_temp = prefix.starts_with("temp");
            if !is_temp && !prefix.starts_with("fan") {
                continue;
            }
            let label = read_trim(hw.join(format!("{prefix}_label"))).unwrap_or_else(|| prefix.to_string());
            let scale = if is_temp { 1000.0 } else { 1.0 };
            let rd = |suffix: &str| read_num::<f64>(hw.join(format!("{prefix}_{suffix}"))).map(|v| v / scale);
            let s = HwmonSensor { chip: chip.clone(), label, kind, value: rd("input"), high: rd("max"), crit: rd("crit") };
            if is_temp {
                temps.push(s);
            } else {
                fans.push(s);
            }
        }
    }
    (temps, fans)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Battery {
    pub name: String,
    pub status: String,
    pub capacity_pct: Option<f64>,
    pub energy_now_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
    pub energy_full_design_wh: Option<f64>,
    pub voltage_v: Option<f64>,
    pub power_w: Option<f64>,
    pub cycle_count: Option<u64>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub technology: Option<String>,
}

impl Battery {
    /// energy_full / energy_full_design in percent, when both are exposed.
    pub fn health_pct(&self) -> Option<f64> {
        match (self.energy_full_wh, self.energy_full_design_wh) {
            (Some(f), Some(d)) if d > 0.0 => Some(f / d * 100.0),
            _ => None,
        }
    }

    /// Hours to empty/full estimated from the current power draw.
    pub fn time_remaining_h(&self) -> Option<f64> {
        let p = self.power_w.filter(|p| *p > 0.1)?;
        let now = self.energy_now_wh?;
        match self.status.as_str() {
            "Discharging" => Some(now / p),
            "Charging" => Some((self.energy_full_wh? - now).max(0.0) / p),
            _ => None,
        }
    }
}

fn read_battery(p: &Path, name: String) -> Battery {
    let micro = |f: &str| read_num::<f64>(p.join(f)).map(|v| v / 1e6);
    let voltage_design = micro("voltage_min_design").or(micro("voltage_now"));
    // Energy in Wh, or charge (Ah) converted with the design voltage.
    let energy = |e: &str, c: &str| micro(e).or_else(|| Some(micro(c)? * voltage_design?));
    let voltage = micro("voltage_now");
    let power = micro("power_now").or_else(|| Some(micro("current_now")? * voltage?));
    Battery {
        name,
        status: read_trim(p.join("status")).unwrap_or_else(|| "Unknown".into()),
        capacity_pct: read_num(p.join("capacity")),
        energy_now_wh: energy("energy_now", "charge_now"),
        energy_full_wh: energy("energy_full", "charge_full"),
        energy_full_design_wh: energy("energy_full_design", "charge_full_design"),
        voltage_v: voltage,
        power_w: power,
        cycle_count: read_num::<u64>(p.join("cycle_count")).filter(|c| *c > 0),
        manufacturer: read_trim(p.join("manufacturer")).filter(|s| !s.is_empty()),
        model: read_trim(p.join("model_name")).filter(|s| !s.is_empty()),
        technology: read_trim(p.join("technology")).filter(|s| !s.is_empty()),
    }
}

/// System batteries and AC adapter state.
pub fn read_power_supplies() -> (Vec<Battery>, Option<bool>) {
    let mut bats = Vec::new();
    let mut ac = None;
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return (bats, ac) };
    for e in dir.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        match read_trim(p.join("type")).as_deref() {
            Some("Battery") if read_trim(p.join("scope")).as_deref() != Some("Device") => bats.push(read_battery(&p, name)),
            Some("Mains") => ac = read_num::<u8>(p.join("online")).map(|v| v == 1).or(ac),
            _ => {}
        }
    }
    bats.sort_by(|a, b| a.name.cmp(&b.name));
    (bats, ac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(classify("k10temp"), SensorKind::Cpu);
        assert_eq!(classify("coretemp"), SensorKind::Cpu);
        assert_eq!(classify("amdgpu"), SensorKind::Gpu);
        assert_eq!(classify("nvme"), SensorKind::Storage);
        assert_eq!(classify("acpitz"), SensorKind::Board);
        assert_eq!(classify("iwlwifi_1"), SensorKind::Other);
    }

    #[test]
    fn battery_math() {
        let b = Battery {
            status: "Discharging".into(),
            energy_now_wh: Some(12.0),
            energy_full_wh: Some(30.0),
            energy_full_design_wh: Some(40.0),
            power_w: Some(12.0),
            ..Default::default()
        };
        assert_eq!(b.health_pct(), Some(75.0));
        assert_eq!(b.time_remaining_h(), Some(1.0));
    }
}
