//! Static system identity (os-release, kernel, DMI via sysfs) and GPU sysfs data.

use crate::util::{read_num, read_trim};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os_name: String,
    pub os_id: String,
    pub os_version: String,
    pub kernel: String,
    pub kernel_version: String,
    pub arch: String,
    pub hostname: String,
    pub desktop: String,
    pub session_type: String,
    pub vendor: String,
    pub product: String,
    pub product_version: String,
    pub board_vendor: String,
    pub board_name: String,
    pub board_version: String,
    pub bios_vendor: String,
    pub bios_version: String,
    pub bios_date: String,
    pub chassis: String,
}

fn uname_machine() -> String {
    // SAFETY: utsname is plain data; uname fills it and returns 0 on success.
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return String::new();
        }
        std::ffi::CStr::from_ptr(u.machine.as_ptr()).to_string_lossy().into_owned()
    }
}

fn chassis_name(code: Option<u32>) -> &'static str {
    match code {
        Some(3..=7) => "Desktop",
        Some(8..=10 | 14) => "Laptop",
        Some(30..=32) => "Tablet/Convertible",
        Some(17 | 23) => "Server",
        Some(_) => "Other",
        None => "",
    }
}

pub fn read_system() -> SystemInfo {
    let osr = crate::parsers::kernel::parse_os_release(&std::fs::read_to_string("/etc/os-release").unwrap_or_default());
    let o = |k: &str| crate::parsers::kv_get(&osr, k).unwrap_or("").to_string();
    let dmi = |f: &str| read_trim(format!("/sys/class/dmi/id/{f}")).unwrap_or_default();
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    SystemInfo {
        os_name: if o("PRETTY_NAME").is_empty() { o("NAME") } else { o("PRETTY_NAME") },
        os_id: o("ID"),
        os_version: o("VERSION"),
        kernel: read_trim("/proc/sys/kernel/osrelease").unwrap_or_default(),
        kernel_version: read_trim("/proc/sys/kernel/version").unwrap_or_default(),
        arch: uname_machine(),
        hostname: read_trim("/proc/sys/kernel/hostname").unwrap_or_default(),
        desktop: env("XDG_CURRENT_DESKTOP"),
        session_type: env("XDG_SESSION_TYPE"),
        vendor: dmi("sys_vendor"),
        product: dmi("product_name"),
        product_version: dmi("product_version"),
        board_vendor: dmi("board_vendor"),
        board_name: dmi("board_name"),
        board_version: dmi("board_version"),
        bios_vendor: dmi("bios_vendor"),
        bios_version: dmi("bios_version"),
        bios_date: dmi("bios_date"),
        chassis: chassis_name(read_num("/sys/class/dmi/id/chassis_type")).into(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuSample {
    pub card: String,
    pub vendor_id: String,
    pub device_id: String,
    pub driver: String,
    /// Only exposed by some drivers (amdgpu).
    pub busy_pct: Option<f64>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
}

impl GpuSample {
    pub fn vendor_name(&self) -> &'static str {
        match self.vendor_id.as_str() {
            "0x1002" => "AMD",
            "0x10de" => "NVIDIA",
            "0x8086" => "Intel",
            _ => "Unknown vendor",
        }
    }
}

pub fn read_gpus() -> Vec<GpuSample> {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<GpuSample> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("card") || name.contains('-') {
                return None;
            }
            let dev = e.path().join("device");
            Some(GpuSample {
                card: name,
                vendor_id: read_trim(dev.join("vendor")).unwrap_or_default(),
                device_id: read_trim(dev.join("device")).unwrap_or_default(),
                driver: std::fs::read_link(dev.join("driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
                    .unwrap_or_default(),
                busy_pct: read_num(dev.join("gpu_busy_percent")),
                vram_used: read_num(dev.join("mem_info_vram_used")),
                vram_total: read_num(dev.join("mem_info_vram_total")),
            })
        })
        .collect();
    out.sort_by(|a, b| a.card.cmp(&b.card));
    out
}
