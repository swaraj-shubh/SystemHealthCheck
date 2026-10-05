//! APT output parsers.

use serde::{Deserialize, Serialize};

/// `apt list --upgradable` rows: (package, new version, old version).
pub fn parse_apt_upgradable(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter(|l| l.contains("[upgradable from:"))
        .filter_map(|l| {
            let (pkg, rest) = l.split_once('/')?;
            let mut it = rest.split_whitespace();
            let _suite = it.next();
            let new = it.next()?.to_string();
            let old = l.rsplit_once("upgradable from: ")?.1.trim_end_matches(']').to_string();
            Some((pkg.to_string(), new, old))
        })
        .collect()
}

/// `apt-get --simulate autoremove`: packages that would be removed (name, version).
pub fn parse_apt_simulate(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Remv ")?;
            let (name, ver) = rest.split_once(' ').unwrap_or((rest, ""));
            Some((name.to_string(), ver.trim_matches(|c| c == '[' || c == ']').to_string()))
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyEntry {
    pub package: String,
    pub installed: Option<String>,
    pub candidate: Option<String>,
}

/// `apt-cache policy pkg...`
pub fn parse_apt_policy(text: &str) -> Vec<PolicyEntry> {
    let mut out: Vec<PolicyEntry> = Vec::new();
    for l in text.lines() {
        if !l.starts_with(' ') && l.ends_with(':') {
            out.push(PolicyEntry { package: l.trim_end_matches(':').to_string(), ..Default::default() });
        } else if let Some(e) = out.last_mut() {
            let t = l.trim();
            let clean = |v: &str| Some(v.trim().to_string()).filter(|v| v != "(none)" && !v.is_empty());
            if let Some(v) = t.strip_prefix("Installed:") {
                e.installed = clean(v);
            } else if let Some(v) = t.strip_prefix("Candidate:") {
                e.candidate = clean(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt() {
        let u = parse_apt_upgradable("Listing...\nlibavcodec-extra/kali-rolling 7:8.1.2-2+b3 amd64 [upgradable from: 7:7.1.3-1]\n");
        assert_eq!(u, vec![("libavcodec-extra".into(), "7:8.1.2-2+b3".into(), "7:7.1.3-1".into())]);
        let s = parse_apt_simulate("Reading package lists...\nRemv python3-fs [2.4.16-9]\nRemv rpcsvc-proto [1.4.4-1]\n");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], ("python3-fs".into(), "2.4.16-9".into()));
        let p = parse_apt_policy("sysbench:\n  Installed: 1.0.20+ds-9\n  Candidate: 1.0.20+ds-9\nhtop:\n  Installed: (none)\n  Candidate: 3.5.3-1\n  Version table:\n     3.5.3-1 500\n");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].installed, None);
        assert_eq!(p[1].candidate.as_deref(), Some("3.5.3-1"));
    }
}
