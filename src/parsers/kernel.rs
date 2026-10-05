//! dmesg, os-release, hostnamectl and lscpu parsers.

use super::parse_kv;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KSeverity {
    Error,
    Warning,
    /// From plain `dmesg --level=err,warn` where levels are not printed.
    ErrOrWarn,
}

impl KSeverity {
    pub fn label(self) -> &'static str {
        match self {
            KSeverity::Error => "Error",
            KSeverity::Warning => "Warning",
            KSeverity::ErrOrWarn => "Error/Warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KernelMsg {
    /// Seconds since boot, when printed.
    pub timestamp: Option<f64>,
    pub severity: KSeverity,
    pub message: String,
}

/// Parse `dmesg` output, decoded (`-x`: "kern  :err   : [ 0.3] msg") or plain.
pub fn parse_dmesg(text: &str) -> Vec<KernelMsg> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut severity = KSeverity::ErrOrWarn;
            let mut rest = l;
            // Decoded prefix: "facility:level : "
            if let Some((fac_lvl, after)) = l.split_once(" : ").filter(|(p, _)| !p.contains('[')) {
                if let Some((_, lvl)) = fac_lvl.split_once(':') {
                    severity = match lvl.trim() {
                        "err" | "crit" | "alert" | "emerg" => KSeverity::Error,
                        "warn" => KSeverity::Warning,
                        _ => KSeverity::ErrOrWarn,
                    };
                    rest = after;
                }
            }
            let rest = rest.trim_start();
            let (timestamp, message) = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
                Some((ts, msg)) => (ts.trim().parse().ok(), msg.trim()),
                None => (None, rest),
            };
            KernelMsg { timestamp, severity, message: message.to_string() }
        })
        .collect()
}

/// `/etc/os-release` as key/value pairs with quotes removed.
pub fn parse_os_release(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').trim_matches('\'').to_string()))
        })
        .collect()
}

/// `hostnamectl` and `lscpu` are plain "Key: Value" listings.
pub fn parse_colon_listing(text: &str) -> Vec<(String, String)> {
    parse_kv(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmesg_decoded_and_plain() {
        let d = "kern  :warn  : [    0.255444] VMSCAPE: SMT on\nkern  :err   : [    0.300163] ACPI BIOS Error (bug): Could not resolve symbol [\\_SB.PCI0.GPP2.BCM5], AE_NOT_FOUND\n";
        let m = parse_dmesg(d);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].severity, KSeverity::Warning);
        assert_eq!(m[1].severity, KSeverity::Error);
        assert_eq!(m[1].timestamp, Some(0.300163));
        assert!(m[1].message.starts_with("ACPI BIOS Error"));
        let p = parse_dmesg("[   12.5] usb 1-1: device descriptor read/64, error -71\n");
        assert_eq!(p[0].severity, KSeverity::ErrOrWarn);
        assert_eq!(p[0].timestamp, Some(12.5));
        assert_eq!(p[0].message, "usb 1-1: device descriptor read/64, error -71");
    }

    #[test]
    fn os_release_and_lscpu() {
        let o = parse_os_release("PRETTY_NAME=\"Kali GNU/Linux Rolling\"\nID=kali\n");
        assert_eq!(o[0], ("PRETTY_NAME".into(), "Kali GNU/Linux Rolling".into()));
        let l = parse_colon_listing("Architecture:      x86_64\nModel name:        AMD Ryzen 5 3500U with Radeon Vega Mobile Gfx\nThread(s) per core: 2\nCPU max MHz:       3700.0000\n");
        assert_eq!(super::super::kv_get(&l, "Model name"), Some("AMD Ryzen 5 3500U with Radeon Vega Mobile Gfx"));
        assert_eq!(super::super::kv_get(&l, "Thread(s) per core"), Some("2"));
    }
}
