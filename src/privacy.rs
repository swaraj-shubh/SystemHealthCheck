//! Masking of sensitive identifiers (serial numbers, UUIDs, hostname, MAC and
//! IP addresses) in displayed raw output and exported reports.

#[derive(Debug, Clone, Default)]
pub struct Privacy {
    pub mask_serial: bool,
    /// Hostname to replace, when hostname masking is enabled.
    pub hostname: Option<String>,
    pub mask_network: bool,
    /// Known identifier values (e.g. the NVMe serial from `nvme list`) that
    /// are replaced wherever they appear when `mask_serial` is on.
    pub secrets: Vec<String>,
}

/// Keys whose values identify this specific machine/device.
const SENSITIVE_KEYS: &[&str] = &["serial number", "serial", "sn", "uuid", "machine id", "boot id", "asset tag"];

/// "5CD1234ABC" -> "XXXX-XXXX-4ABC" (keeps the last 4 characters).
pub fn mask_value(v: &str) -> String {
    let v = v.trim();
    let tail: String = v.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    if v.chars().count() <= 4 {
        "XXXX".into()
    } else {
        format!("XXXX-XXXX-{tail}")
    }
}

fn is_hex_pair(s: &[u8]) -> bool {
    s.len() == 2 && s.iter().all(u8::is_ascii_hexdigit)
}

/// Replace MAC addresses (aa:bb:cc:dd:ee:ff) with xx:xx:xx:xx:xx:xx.
pub fn mask_macs(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if i + 17 <= b.len() && (0..6).all(|k| is_hex_pair(&b[i + k * 3..i + k * 3 + 2])) && (0..5).all(|k| b[i + k * 3 + 2] == b':') {
            let boundary_ok = (i == 0 || !b[i - 1].is_ascii_hexdigit() && b[i - 1] != b':') && (i + 17 == b.len() || b[i + 17] != b':');
            if boundary_ok {
                out.push_str("xx:xx:xx:xx:xx:xx");
                i += 17;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Replace IPv4 addresses with x.x.x.x (whole dotted quads only).
pub fn mask_ipv4(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let parts: Vec<&str> = token.split('.').collect();
        let is_ip = parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u8>().is_ok());
        out.push_str(if is_ip { "x.x.x.x" } else { token });
        token.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            token.push(ch);
        } else {
            flush(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out
}

impl Privacy {
    /// Apply all enabled masks to free text (raw command output).
    pub fn redact(&self, text: &str) -> String {
        let mut s: String = if self.mask_serial {
            text.lines()
                .map(|l| match l.split_once(':') {
                    Some((k, v)) if SENSITIVE_KEYS.contains(&k.trim().to_lowercase().as_str()) && !v.trim().is_empty() => {
                        format!("{k}: {}", mask_value(v))
                    }
                    _ => l.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            text.to_string()
        };
        if self.mask_serial {
            for sec in self.secrets.iter().filter(|x| x.len() >= 4) {
                s = s.replace(sec.as_str(), &mask_value(sec));
            }
        }
        if text.ends_with('\n') && !s.ends_with('\n') {
            s.push('\n');
        }
        if let Some(h) = self.hostname.as_deref().filter(|h| h.len() > 1) {
            s = s.replace(h, "<hostname>");
        }
        if self.mask_network {
            s = mask_ipv4(&mask_macs(&s));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_serials() {
        let p = Privacy { mask_serial: true, ..Default::default() };
        let t = "System Information\n\tManufacturer: HP\n\tSerial Number: 5CD0123ABCD\n\tUUID: 1234-5678\nserial: 41167\n";
        let r = p.redact(t);
        assert!(r.contains("Serial Number: XXXX-XXXX-ABCD"));
        assert!(r.contains("UUID: XXXX-XXXX-5678"));
        assert!(r.contains("Manufacturer: HP"));
        assert!(!r.contains("41167"));
        assert!(r.ends_with('\n'));
        assert_eq!(mask_value("1234"), "XXXX");
        let p = Privacy { mask_serial: true, secrets: vec!["ABC123XYZ".into()], ..Default::default() };
        assert_eq!(p.redact("/dev/nvme0n1  ABC123XYZ  WDC"), "/dev/nvme0n1  XXXX-XXXX-3XYZ  WDC");
    }

    #[test]
    fn masks_network_and_host() {
        let p = Privacy { mask_serial: false, hostname: Some("kali".into()), mask_network: true, secrets: vec![] };
        let r = p.redact("kali addr 74:12:b3:8d:2d:89 inet 192.168.1.20/24 version 1.2.3 time 12:30:45");
        assert_eq!(r, "<hostname> addr xx:xx:xx:xx:xx:xx inet x.x.x.x/24 version 1.2.3 time 12:30:45");
    }
}
