//! Network interfaces: /proc/net/dev counters, /sys/class/net attributes and
//! addresses via getifaddrs(3).

use crate::util::{read_num, read_trim};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetIface {
    pub name: String,
    /// "wifi", "ethernet", "loopback", "bridge", "virtual"
    pub kind: String,
    pub state: String,
    pub mac: String,
    pub driver: String,
    pub speed_mbps: Option<i64>,
    pub mtu: Option<u32>,
    pub addrs: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_rate: f64,
    pub tx_rate: f64,
}

#[derive(Default)]
pub struct NetState {
    prev: HashMap<String, (u64, u64)>,
    at: Option<Instant>,
}

/// /proc/net/dev -> (iface, rx_bytes, tx_bytes)
pub fn parse_net_dev(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let v: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            Some((name.trim().to_string(), *v.first()?, *v.get(8)?))
        })
        .collect()
}

fn addresses() -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    // SAFETY: getifaddrs allocates a linked list we walk read-only and free once.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() && !ifa.ifa_name.is_null() {
                let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
                let fam = (*ifa.ifa_addr).sa_family as i32;
                let addr = if fam == libc::AF_INET {
                    let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    Some(std::net::Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)).to_string())
                } else if fam == libc::AF_INET6 {
                    let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    Some(std::net::Ipv6Addr::from(sa.sin6_addr.s6_addr).to_string())
                } else {
                    None
                };
                if let Some(a) = addr {
                    out.entry(name).or_default().push(a);
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

impl NetState {
    pub fn sample(&mut self) -> Vec<NetIface> {
        let now = Instant::now();
        let dt = self.at.map(|t| now.duration_since(t).as_secs_f64()).filter(|d| *d > 0.0);
        self.at = Some(now);
        let mut addrs = addresses();
        let counters = parse_net_dev(&std::fs::read_to_string("/proc/net/dev").unwrap_or_default());
        let mut out = Vec::with_capacity(counters.len());
        for (name, rx, tx) in counters {
            let base = std::path::Path::new("/sys/class/net").join(&name);
            let (rx_rate, tx_rate) = match (dt, self.prev.get(&name)) {
                (Some(dt), Some(&(prx, ptx))) => (rx.saturating_sub(prx) as f64 / dt, tx.saturating_sub(ptx) as f64 / dt),
                _ => (0.0, 0.0),
            };
            self.prev.insert(name.clone(), (rx, tx));
            let kind = if name == "lo" {
                "loopback"
            } else if base.join("wireless").exists() || base.join("phy80211").exists() {
                "wifi"
            } else if base.join("bridge").exists() {
                "bridge"
            } else if !base.join("device").exists() {
                "virtual"
            } else {
                "ethernet"
            };
            out.push(NetIface {
                kind: kind.into(),
                state: read_trim(base.join("operstate")).unwrap_or_default(),
                mac: read_trim(base.join("address")).unwrap_or_default(),
                driver: std::fs::read_link(base.join("device/driver"))
                    .ok()
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
                    .unwrap_or_default(),
                speed_mbps: read_num::<i64>(base.join("speed")).filter(|s| *s > 0),
                mtu: read_num(base.join("mtu")),
                addrs: addrs.remove(&name).unwrap_or_default(),
                name,
                rx_bytes: rx,
                tx_bytes: tx,
                rx_rate,
                tx_rate,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_dev() {
        let t = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 1000      10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0\n wlan0: 123456 100 0 0 0 0 0 0 654321 90 0 0 0 0 0 0\n";
        let r = parse_net_dev(t);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1], ("wlan0".into(), 123456, 654321));
    }
}
