//! `sensors` (lm-sensors) and `upower -i` parsers.

use super::{kv_get, leading_num};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorReading {
    pub label: String,
    /// `None` for "N/A".
    pub value: Option<f64>,
    /// "°C", "RPM", "V", "W", "A", "MHz", ... as printed.
    pub unit: String,
    pub high: Option<f64>,
    pub crit: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorChip {
    pub name: String,
    pub adapter: String,
    pub readings: Vec<SensorReading>,
}

fn limit(extra: &str, key: &str) -> Option<f64> {
    let i = extra.find(key)?;
    let rest = extra[i + key.len()..].trim_start().trim_start_matches('=').trim_start();
    leading_num(rest)
}

/// Parse `sensors` output into chips and readings.
pub fn parse_sensors(text: &str) -> Vec<SensorChip> {
    let mut chips: Vec<SensorChip> = Vec::new();
    let mut new_chip = true;
    for line in text.lines() {
        if line.trim().is_empty() {
            new_chip = true;
            continue;
        }
        if new_chip && !line.contains(':') {
            chips.push(SensorChip { name: line.trim().to_string(), ..Default::default() });
            new_chip = false;
            continue;
        }
        new_chip = false;
        let Some(chip) = chips.last_mut() else { continue };
        if line.starts_with(char::is_whitespace) {
            // Continuation "(crit = +84.8 C)" for the previous reading.
            if let Some(r) = chip.readings.last_mut() {
                r.high = r.high.or(limit(line, "high"));
                r.crit = r.crit.or(limit(line, "crit"));
            }
            continue;
        }
        let Some((label, rest)) = line.split_once(':') else { continue };
        if label == "Adapter" {
            chip.adapter = rest.trim().to_string();
            continue;
        }
        let rest = rest.trim();
        let (main, extra) = rest.split_once('(').unwrap_or((rest, ""));
        let main = main.trim();
        let value = leading_num(main);
        let unit = if value.is_some() {
            main.trim_start_matches(|c: char| c.is_ascii_digit() || "+-.,".contains(c)).trim().to_string()
        } else {
            String::new()
        };
        let unit = match unit.as_str() {
            "C" | "°C" => "°C".to_string(),
            _ => unit,
        };
        chip.readings.push(SensorReading {
            label: label.trim().to_string(),
            value,
            unit,
            high: limit(extra, "high"),
            crit: limit(extra, "crit"),
        });
    }
    chips
}

/// Battery as reported by UPower. Absent values were not reported.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpowerBattery {
    pub native_path: Option<String>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub state: Option<String>,
    pub percentage: Option<f64>,
    pub energy_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
    pub energy_full_design_wh: Option<f64>,
    pub energy_rate_w: Option<f64>,
    pub voltage_v: Option<f64>,
    pub time_to_empty: Option<String>,
    pub time_to_full: Option<String>,
    pub charge_cycles: Option<u64>,
    pub capacity_pct: Option<f64>,
    pub technology: Option<String>,
    pub fields: Vec<(String, String)>,
}

impl UpowerBattery {
    /// Health = energy-full / energy-full-design, only when both are reported.
    pub fn health_pct(&self) -> Option<f64> {
        match (self.energy_full_wh, self.energy_full_design_wh) {
            (Some(f), Some(d)) if d > 0.0 => Some(f / d * 100.0),
            _ => self.capacity_pct,
        }
    }
}

/// Parse `upower -i <battery>`; History/statistics sections are kept only in `fields`.
pub fn parse_upower(text: &str) -> Option<UpowerBattery> {
    let body: String = text.lines().take_while(|l| !l.trim_start().starts_with("History")).collect::<Vec<_>>().join("\n");
    let fields = super::parse_kv(&body);
    if fields.is_empty() {
        return None;
    }
    let s = |k: &str| kv_get(&fields, k).map(str::to_string);
    let n = |k: &str| kv_get(&fields, k).and_then(leading_num);
    Some(UpowerBattery {
        native_path: s("native-path"),
        vendor: s("vendor"),
        model: s("model"),
        state: s("state"),
        percentage: n("percentage"),
        energy_wh: n("energy"),
        energy_full_wh: n("energy-full"),
        energy_full_design_wh: n("energy-full-design"),
        energy_rate_w: n("energy-rate"),
        voltage_v: n("voltage"),
        time_to_empty: s("time to empty"),
        time_to_full: s("time to full"),
        charge_cycles: n("charge-cycles").filter(|c| *c > 0.0).map(|c| c as u64),
        capacity_pct: n("capacity"),
        technology: s("technology"),
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENSORS: &str = "k10temp-pci-00c3
Adapter: PCI adapter
Tctl:         +52.0 C

amdgpu-pci-0400
Adapter: PCI adapter
vddgfx:           N/A
edge:         +52.0 C
sclk:         200 MHz

nvme-pci-0300
Adapter: PCI adapter
Composite:    +50.9 C  (low  =  -5.2 C, high = +79.8 C)
                       (crit = +84.8 C)

thinkpad-isa-0000
Adapter: ISA adapter
fan1:        2100 RPM
";

    #[test]
    fn sensors() {
        let chips = parse_sensors(SENSORS);
        assert_eq!(chips.len(), 4);
        assert_eq!(chips[0].name, "k10temp-pci-00c3");
        assert_eq!(chips[0].readings[0].value, Some(52.0));
        assert_eq!(chips[0].readings[0].unit, "°C");
        assert_eq!(chips[1].readings[0].value, None);
        assert_eq!(chips[1].readings[2].unit, "MHz");
        let comp = &chips[2].readings[0];
        assert_eq!(comp.high, Some(79.8));
        assert_eq!(comp.crit, Some(84.8));
        assert_eq!(chips[3].readings[0].unit, "RPM");
        assert_eq!(chips[3].readings[0].value, Some(2100.0));
    }

    const UPOWER: &str = "  native-path:          BAT1
  vendor:               Hewlett-Packard
  model:                PABAS0241231
  serial:               41167
  power supply:         yes
  battery
    present:             yes
    state:               discharging
    energy:              12.474 Wh
    energy-empty:        0 Wh
    energy-full:         31.7633 Wh
    energy-full-design:  41.0508 Wh
    energy-rate:         13.5924 W
    voltage:             10.889 V
    charge-cycles:       N/A
    time to empty:       55.0 minutes
    percentage:          39%
    capacity:            77.3757%
    technology:          lithium-ion
  History (charge):
    1791227137	39.000	discharging
";

    #[test]
    fn upower() {
        let b = parse_upower(UPOWER).expect("battery");
        assert_eq!(b.state.as_deref(), Some("discharging"));
        assert_eq!(b.percentage, Some(39.0));
        assert_eq!(b.energy_full_design_wh, Some(41.0508));
        assert_eq!(b.charge_cycles, None);
        assert_eq!(b.time_to_empty.as_deref(), Some("55.0 minutes"));
        let h = b.health_pct().expect("health");
        assert!((h - 77.375).abs() < 0.01);
        assert!(parse_upower("").is_none());
    }
}
