//! CPU usage (/proc/stat), frequencies (cpufreq sysfs) and static CPU info.

use crate::util::{read_num, read_trim};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuSample {
    pub model: String,
    pub threads: usize,
    /// Physical cores (unique core ids), when exposed.
    pub cores: Option<usize>,
    /// Percent; `None` on the first sample (no delta yet).
    pub total_usage: Option<f64>,
    pub per_core_usage: Vec<f64>,
    /// MHz per logical CPU.
    pub per_core_mhz: Vec<f64>,
    pub min_mhz: Option<f64>,
    pub max_mhz: Option<f64>,
    pub governor: Option<String>,
    pub available_governors: Option<String>,
    pub driver: Option<String>,
}

impl CpuSample {
    pub fn avg_mhz(&self) -> Option<f64> {
        (!self.per_core_mhz.is_empty()).then(|| self.per_core_mhz.iter().sum::<f64>() / self.per_core_mhz.len() as f64)
    }
}

#[derive(Default)]
pub struct CpuState {
    prev: Vec<(u64, u64)>,
    model: Option<(String, Option<usize>)>,
}

/// (busy, total) jiffies per line of /proc/stat "cpu*" rows; index 0 = aggregate.
pub fn parse_proc_stat(text: &str) -> Vec<(u64, u64)> {
    text.lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|l| {
            let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            // user nice system idle iowait irq softirq steal (guest counted in user)
            let total: u64 = v.iter().take(8).sum();
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            (total.saturating_sub(idle), total)
        })
        .collect()
}

fn usage(prev: (u64, u64), cur: (u64, u64)) -> f64 {
    let dt = cur.1.saturating_sub(prev.1);
    if dt == 0 {
        return 0.0;
    }
    (cur.0.saturating_sub(prev.0) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0)
}

fn static_info() -> (String, Option<usize>) {
    let info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = info
        .lines()
        .find(|l| l.starts_with("model name") || l.starts_with("Model") || l.starts_with("Hardware"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_else(|| "Unknown CPU".into());
    let mut cores: Vec<(String, String)> = Vec::new();
    let mut phys = String::new();
    for l in info.lines() {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim() {
                "physical id" => phys = v.trim().into(),
                "core id" => cores.push((phys.clone(), v.trim().into())),
                _ => {}
            }
        }
    }
    cores.sort();
    cores.dedup();
    (model, (!cores.is_empty()).then_some(cores.len()))
}

impl CpuState {
    pub fn sample(&mut self) -> CpuSample {
        let (model, cores) = self.model.get_or_insert_with(static_info).clone();
        let cur = parse_proc_stat(&std::fs::read_to_string("/proc/stat").unwrap_or_default());
        let have_prev = self.prev.len() == cur.len() && !cur.is_empty();
        let total_usage = have_prev.then(|| usage(self.prev[0], cur[0]));
        let per_core_usage = if have_prev {
            self.prev.iter().zip(&cur).skip(1).map(|(p, c)| usage(*p, *c)).collect()
        } else {
            vec![0.0; cur.len().saturating_sub(1)]
        };
        let threads = cur.len().saturating_sub(1).max(1);
        self.prev = cur;
        let base = "/sys/devices/system/cpu";
        let mut per_core_mhz: Vec<f64> = (0..threads)
            .filter_map(|i| read_num::<f64>(format!("{base}/cpu{i}/cpufreq/scaling_cur_freq")).map(|k| k / 1000.0))
            .collect();
        if per_core_mhz.is_empty() {
            per_core_mhz = std::fs::read_to_string("/proc/cpuinfo")
                .unwrap_or_default()
                .lines()
                .filter(|l| l.starts_with("cpu MHz"))
                .filter_map(|l| l.split_once(':').and_then(|(_, v)| v.trim().parse().ok()))
                .collect();
        }
        CpuSample {
            model,
            threads,
            cores,
            total_usage,
            per_core_usage,
            per_core_mhz,
            min_mhz: read_num::<f64>(format!("{base}/cpu0/cpufreq/cpuinfo_min_freq")).map(|k| k / 1000.0),
            max_mhz: read_num::<f64>(format!("{base}/cpu0/cpufreq/cpuinfo_max_freq")).map(|k| k / 1000.0),
            governor: read_trim(format!("{base}/cpu0/cpufreq/scaling_governor")),
            available_governors: read_trim(format!("{base}/cpu0/cpufreq/scaling_available_governors")),
            driver: read_trim(format!("{base}/cpu0/cpufreq/scaling_driver")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_deltas() {
        let a = parse_proc_stat("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n");
        let b = parse_proc_stat("cpu  200 0 200 1000 0 0 0 0 0 0\ncpu0 100 0 100 500 0 0 0 0 0 0\n");
        assert_eq!(a.len(), 2);
        assert!((usage(a[0], b[0]) - 50.0).abs() < 1e-9);
        assert!((usage(a[1], b[1]) - 50.0).abs() < 1e-9);
        assert_eq!(usage(a[0], a[0]), 0.0);
    }
}
