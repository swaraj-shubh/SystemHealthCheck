//! RAM and swap from /proc/meminfo, /proc/swaps and vm.swappiness.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwapDevice {
    pub name: String,
    pub kind: String,
    pub size_kb: u64,
    pub used_kb: u64,
    pub priority: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemSample {
    pub total_kb: u64,
    pub free_kb: u64,
    pub available_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub shared_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    pub swappiness: Option<u32>,
    pub swaps: Vec<SwapDevice>,
}

impl MemSample {
    /// Same definition as `free`: total - free - buffers - cache.
    pub fn used_kb(&self) -> u64 {
        self.total_kb.saturating_sub(self.free_kb + self.buffers_kb + self.cached_kb)
    }
    pub fn used_pct(&self) -> f64 {
        if self.total_kb == 0 {
            0.0
        } else {
            (self.total_kb - self.available_kb.min(self.total_kb)) as f64 / self.total_kb as f64 * 100.0
        }
    }
    pub fn swap_used_kb(&self) -> u64 {
        self.swap_total_kb.saturating_sub(self.swap_free_kb)
    }
    pub fn swap_used_pct(&self) -> Option<f64> {
        (self.swap_total_kb > 0).then(|| self.swap_used_kb() as f64 / self.swap_total_kb as f64 * 100.0)
    }
}

/// Parse /proc/meminfo text.
pub fn parse_meminfo(text: &str) -> MemSample {
    let mut m = MemSample::default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        let v: u64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
        match k {
            "MemTotal" => m.total_kb = v,
            "MemFree" => m.free_kb = v,
            "MemAvailable" => m.available_kb = v,
            "Buffers" => m.buffers_kb = v,
            "Cached" | "SReclaimable" => m.cached_kb += v,
            "Shmem" => m.shared_kb = v,
            "SwapTotal" => m.swap_total_kb = v,
            "SwapFree" => m.swap_free_kb = v,
            _ => {}
        }
    }
    m
}

/// Parse /proc/swaps.
pub fn parse_swaps(text: &str) -> Vec<SwapDevice> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            (c.len() >= 5).then(|| SwapDevice {
                name: c[0].into(),
                kind: c[1].into(),
                size_kb: c[2].parse().unwrap_or(0),
                used_kb: c[3].parse().unwrap_or(0),
                priority: c[4].parse().unwrap_or(0),
            })
        })
        .collect()
}

pub fn read() -> MemSample {
    let mut m = parse_meminfo(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
    m.swaps = parse_swaps(&std::fs::read_to_string("/proc/swaps").unwrap_or_default());
    m.swappiness = crate::util::read_num("/proc/sys/vm/swappiness");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo() {
        let m = parse_meminfo("MemTotal:  8000000 kB\nMemFree:  1000000 kB\nMemAvailable: 3000000 kB\nBuffers: 100000 kB\nCached: 1500000 kB\nSReclaimable: 100000 kB\nSwapTotal: 2000000 kB\nSwapFree: 1500000 kB\n");
        assert_eq!(m.used_kb(), 8000000 - 1000000 - 100000 - 1600000);
        assert!((m.used_pct() - 62.5).abs() < 1e-9);
        assert_eq!(m.swap_used_kb(), 500000);
        let s = parse_swaps("Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/dev/nvme0n1p3                          partition\t6143996\t\t3250000\t\t-2\n");
        assert_eq!(s[0].name, "/dev/nvme0n1p3");
        assert_eq!(s[0].priority, -2);
    }
}
