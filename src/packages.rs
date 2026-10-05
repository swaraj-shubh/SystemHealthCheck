//! Diagnostic tool dependencies: what each package provides, how to install
//! it (always via an allowlisted registry entry) and whether it is installed
//! (read natively from the dpkg database).

use std::collections::HashSet;

/// Packages the generic `apt-install-pkg` entry may install.
pub const INSTALLABLE: &[&str] = &[
    "smartmontools",
    "nvme-cli",
    "lm-sensors",
    "mesa-utils",
    "upower",
    "linux-cpupower",
    "sysbench",
    "stress-ng",
    "htop",
    "lshw",
    "dmidecode",
    "hdparm",
    "pciutils",
    "usbutils",
    "iw",
    "network-manager",
];

#[derive(Debug, Clone, Copy)]
pub struct Dependency {
    pub package: &'static str,
    pub binary: &'static str,
    pub purpose: &'static str,
    /// Registry id that installs it (PDF command when one exists).
    pub install_id: &'static str,
}

pub const DEPENDENCIES: &[Dependency] = &[
    Dependency { package: "smartmontools", binary: "smartctl", purpose: "SMART health (smartctl)", install_id: "apt-install-smart" },
    Dependency { package: "nvme-cli", binary: "nvme", purpose: "NVMe list and health log", install_id: "apt-install-nvme" },
    Dependency { package: "lm-sensors", binary: "sensors", purpose: "Temperatures and fans", install_id: "apt-install-sensors" },
    Dependency { package: "mesa-utils", binary: "glxinfo", purpose: "OpenGL / GPU information", install_id: "apt-install-mesa" },
    Dependency { package: "upower", binary: "upower", purpose: "Battery information", install_id: "apt-install-upower" },
    Dependency { package: "linux-cpupower", binary: "cpupower", purpose: "CPU frequency information", install_id: "apt-install-cpupower" },
    Dependency { package: "sysbench", binary: "sysbench", purpose: "CPU benchmark", install_id: "apt-install-sysbench" },
    Dependency { package: "stress-ng", binary: "stress-ng", purpose: "CPU stress test", install_id: "apt-install-stress" },
    Dependency { package: "htop", binary: "htop", purpose: "Interactive process viewer (terminal)", install_id: "apt-install-htop" },
    Dependency { package: "lshw", binary: "lshw", purpose: "Hardware tree", install_id: "apt-install-pkg" },
    Dependency { package: "dmidecode", binary: "dmidecode", purpose: "RAM modules, motherboard, system model", install_id: "apt-install-pkg" },
    Dependency { package: "hdparm", binary: "hdparm", purpose: "Disk read benchmark", install_id: "apt-install-pkg" },
    Dependency { package: "pciutils", binary: "lspci", purpose: "PCI devices", install_id: "apt-install-pkg" },
    Dependency { package: "usbutils", binary: "lsusb", purpose: "USB devices", install_id: "apt-install-pkg" },
    Dependency { package: "iw", binary: "iw", purpose: "Wi-Fi interfaces", install_id: "apt-install-pkg" },
    Dependency { package: "network-manager", binary: "nmcli", purpose: "NetworkManager status", install_id: "apt-install-pkg" },
];

/// Find the dependency providing a binary.
pub fn for_binary(bin: &str) -> Option<&'static Dependency> {
    DEPENDENCIES.iter().find(|d| d.binary == bin)
}

/// Installed package names from a dpkg status database.
pub fn parse_dpkg_status(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for para in text.split("\n\n") {
        let mut name = None;
        let mut installed = false;
        for l in para.lines() {
            if let Some(n) = l.strip_prefix("Package: ") {
                name = Some(n.trim().to_string());
            } else if let Some(s) = l.strip_prefix("Status: ") {
                installed = s.trim().ends_with(" installed");
            }
        }
        if let (Some(n), true) = (name, installed) {
            out.insert(n);
        }
    }
    out
}

/// Installed packages on this system (empty if not a dpkg system).
pub fn installed_packages() -> HashSet<String> {
    parse_dpkg_status(&std::fs::read_to_string("/var/lib/dpkg/status").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_status() {
        let t = "Package: sysbench\nStatus: install ok installed\nVersion: 1\n\nPackage: htop\nStatus: deinstall ok config-files\n";
        let s = parse_dpkg_status(t);
        assert!(s.contains("sysbench"));
        assert!(!s.contains("htop"));
    }

    #[test]
    fn install_ids_exist() {
        for d in DEPENDENCIES {
            assert!(crate::diagnostics::registry::get(d.install_id).is_some(), "{}", d.install_id);
            assert!(INSTALLABLE.contains(&d.package));
        }
    }
}
