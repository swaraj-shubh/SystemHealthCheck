//! systemd-analyze, blame, critical-chain and unit-file parsers.

use super::{parse_header_table, parse_systemd_time};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BootTimes {
    pub firmware_s: Option<f64>,
    pub loader_s: Option<f64>,
    pub kernel_s: Option<f64>,
    pub initrd_s: Option<f64>,
    pub userspace_s: Option<f64>,
    pub total_s: Option<f64>,
    /// "graphical.target reached after 12.126s in userspace."
    pub target_line: Option<String>,
}

/// `systemd-analyze`
pub fn parse_systemd_analyze(text: &str) -> Option<BootTimes> {
    let line = text.lines().find(|l| l.starts_with("Startup finished in "))?;
    let body = &line["Startup finished in ".len()..];
    let (parts, total) = body.rsplit_once(" = ")?;
    let mut t = BootTimes { total_s: parse_systemd_time(total.trim()), ..Default::default() };
    for part in parts.split(" + ") {
        let (time, label) = part.rsplit_once(" (").unwrap_or((part, "total)"));
        let v = parse_systemd_time(time.trim());
        match label.trim_end_matches(')') {
            "firmware" => t.firmware_s = v,
            "loader" => t.loader_s = v,
            "kernel" => t.kernel_s = v,
            "initrd" => t.initrd_s = v,
            "userspace" => t.userspace_s = v,
            _ => {}
        }
    }
    t.target_line = text.lines().find(|l| l.contains("reached after")).map(|l| l.trim().to_string());
    Some(t)
}

/// `systemd-analyze blame` rows: (seconds, unit).
pub fn parse_blame(text: &str) -> Vec<(f64, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (time, unit) = l.rsplit_once(' ')?;
            Some((parse_systemd_time(time)?, unit.to_string()))
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChainNode {
    pub depth: usize,
    pub unit: String,
    /// "@" time: when the unit became active.
    pub at_s: Option<f64>,
    /// "+" time: how long it took to start.
    pub took_s: Option<f64>,
}

/// `systemd-analyze critical-chain` (ASCII `` `- `` or Unicode `└─` trees).
pub fn parse_critical_chain(text: &str) -> Vec<ChainNode> {
    text.lines()
        .filter(|l| !l.starts_with("The time") && !l.trim().is_empty())
        .filter_map(|l| {
            let mut start = l.find(|c: char| !" `|-└─├│".contains(c))?;
            // The root mount unit is literally "-.mount".
            if l[start..].starts_with('.') && l[..start].ends_with("--") {
                start -= 1;
            }
            let depth = l[..start].chars().count() / 2;
            let mut it = l[start..].split_whitespace();
            let unit = it.next()?.to_string();
            let mut node = ChainNode { depth, unit, ..Default::default() };
            for tok in it {
                if let Some(t) = tok.strip_prefix('@') {
                    node.at_s = parse_systemd_time(t);
                } else if let Some(t) = tok.strip_prefix('+') {
                    node.took_s = parse_systemd_time(t);
                }
            }
            Some(node)
        })
        .collect()
}

/// `systemctl list-unit-files --state=enabled` rows: (unit, state, preset).
pub fn parse_unit_files(text: &str) -> Vec<(String, String, String)> {
    parse_header_table(text)
        .into_iter()
        .filter_map(|r| {
            let unit = r.get("UNIT FILE")?.clone();
            Some((unit, r.get("STATE").cloned().unwrap_or_default(), r.get("PRESET").cloned().unwrap_or_default()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze() {
        let t = "Startup finished in 7.353s (firmware) + 14.581s (loader) + 7.695s (kernel) + 12.126s (userspace) = 41.757s \ngraphical.target reached after 12.126s in userspace.\n";
        let b = parse_systemd_analyze(t).expect("parses");
        assert_eq!(b.firmware_s, Some(7.353));
        assert_eq!(b.loader_s, Some(14.581));
        assert_eq!(b.kernel_s, Some(7.695));
        assert_eq!(b.userspace_s, Some(12.126));
        assert_eq!(b.total_s, Some(41.757));
        let vm = "Startup finished in 1.2s (kernel) + 1min 3.5s (userspace) = 1min 4.7s\n";
        let b = parse_systemd_analyze(vm).expect("parses");
        assert_eq!(b.firmware_s, None);
        assert_eq!(b.userspace_s, Some(63.5));
        assert!(parse_systemd_analyze("Bootup is not yet finished").is_none());
    }

    #[test]
    fn blame() {
        let r = parse_blame("42.761s plocate-updatedb.service\n 5.505s NetworkManager-wait-online.service\n1min 2.1s slow.service\n  94ms dbus.service\n");
        assert_eq!(r.len(), 4);
        assert_eq!(r[0], (42.761, "plocate-updatedb.service".into()));
        assert_eq!(r[2].0, 62.1);
        assert_eq!(r[3].0, 0.094);
    }

    #[test]
    fn chain() {
        let t = "The time when unit became active or started is printed after the \"@\" character.\nThe time the unit took to start is printed after the \"+\" character.\n\ngraphical.target @12.126s\n`-multi-user.target @12.126s\n  `-docker.service @10.707s +1.418s\n    `-network-online.target @10.703s\n";
        let c = parse_critical_chain(t);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].depth, 0);
        assert_eq!(c[2].unit, "docker.service");
        assert_eq!(c[2].took_s, Some(1.418));
        assert!(c[3].depth > c[2].depth);
        let u = "graphical.target @5s\n└─multi-user.target @5s\n  └─ssh.service @4s +100ms\n";
        assert_eq!(parse_critical_chain(u)[2].unit, "ssh.service");
    }

    #[test]
    fn unit_files() {
        let t = "UNIT FILE                               STATE   PRESET\nssh.service                             enabled enabled\ndocker.socket                           enabled enabled\n\n2 unit files listed.\n";
        let r = parse_unit_files(t);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].0, "docker.socket");
    }
}
