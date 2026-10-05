//! Native collectors reading `/proc` and `/sys` directly. These drive live
//! monitoring so no shell command is spawned on every refresh.

pub mod cpu;
pub mod disks;
pub mod memory;
pub mod net;
pub mod power;
pub mod procs;
pub mod system;

use serde::{Deserialize, Serialize};

/// One instant of native readings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sample {
    pub taken_at: i64,
    pub cpu: cpu::CpuSample,
    pub memory: memory::MemSample,
    pub sensors: Vec<power::HwmonSensor>,
    pub fans: Vec<power::HwmonSensor>,
    pub batteries: Vec<power::Battery>,
    pub ac_online: Option<bool>,
    pub gpus: Vec<system::GpuSample>,
    pub net: Vec<net::NetIface>,
    pub disks: Vec<disks::DiskIo>,
    pub filesystems: Vec<disks::Filesystem>,
    /// Empty when process sampling was skipped this tick.
    #[serde(skip)]
    pub processes: Vec<procs::Proc>,
    pub uptime_s: f64,
    pub load_avg: [f64; 3],
}

impl Sample {
    /// Highest temperature from sensors classified as `kind`.
    pub fn max_temp(&self, kind: power::SensorKind) -> Option<f64> {
        self.sensors.iter().filter(|s| s.kind == kind).filter_map(|s| s.value).reduce(f64::max)
    }
}

/// Stateful sampler: keeps previous counters to compute rates.
#[derive(Default)]
pub struct Monitor {
    cpu: cpu::CpuState,
    net: net::NetState,
    disks: disks::DiskState,
    procs: procs::ProcState,
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take a sample. Process scanning is the most expensive part and can be skipped.
    pub fn sample(&mut self, with_processes: bool) -> Sample {
        let memory = memory::read();
        let (sensors, fans) = power::read_hwmon();
        let (batteries, ac_online) = power::read_power_supplies();
        let total_mem = memory.total_kb;
        let uptime = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
        let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let mut l = load.split_whitespace().map(|v| v.parse().unwrap_or(0.0));
        Sample {
            taken_at: crate::util::now(),
            cpu: self.cpu.sample(),
            memory,
            sensors,
            fans,
            batteries,
            ac_online,
            gpus: system::read_gpus(),
            net: self.net.sample(),
            disks: self.disks.sample(),
            filesystems: disks::filesystems(),
            processes: if with_processes { self.procs.sample(total_mem) } else { Vec::new() },
            uptime_s: uptime.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0),
            load_avg: [l.next().unwrap_or(0.0), l.next().unwrap_or(0.0), l.next().unwrap_or(0.0)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test against the real /proc and /sys of the build host.
    #[test]
    fn samples_the_running_system() {
        let mut m = Monitor::new();
        let _ = m.sample(true);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let s = m.sample(true);
        assert!(s.cpu.threads >= 1);
        assert!(s.memory.total_kb > 0);
        assert!(s.cpu.total_usage.is_some_and(|u| (0.0..=100.0).contains(&u)));
        assert!(!s.processes.is_empty());
        assert!(s.uptime_s > 0.0);
    }
}

#[cfg(test)]
mod timing {
    /// `cargo test collector_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn collector_timing() {
        use std::time::Instant;
        let t = |name: &str, f: &mut dyn FnMut()| {
            let s = Instant::now();
            for _ in 0..20 {
                f();
            }
            eprintln!("{name:<12} {:>8.2} ms/sample", s.elapsed().as_secs_f64() * 1000.0 / 20.0);
        };
        let mut cpu = super::cpu::CpuState::default();
        let mut net = super::net::NetState::default();
        let mut disks = super::disks::DiskState::default();
        let mut procs = super::procs::ProcState::default();
        t("cpu", &mut || drop(cpu.sample()));
        t("memory", &mut || drop(super::memory::read()));
        t("hwmon", &mut || drop(super::power::read_hwmon()));
        t("power", &mut || drop(super::power::read_power_supplies()));
        t("gpus", &mut || drop(super::system::read_gpus()));
        t("net", &mut || drop(net.sample()));
        t("disks", &mut || drop(disks.sample()));
        t("filesystems", &mut || drop(super::disks::filesystems()));
        t("procs", &mut || drop(procs.sample(8_000_000)));
    }
}
