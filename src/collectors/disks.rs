//! Block devices (/sys/block), disk I/O (/proc/diskstats) and filesystem
//! usage (/proc/self/mounts + statvfs).

use crate::util::{read_num, read_trim};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiskIo {
    pub name: String,
    pub model: String,
    pub size_bytes: u64,
    pub rotational: bool,
    pub removable: bool,
    pub read_rate: f64,
    pub write_rate: f64,
    /// Percent of time the device was busy.
    pub busy_pct: f64,
}

#[derive(Default)]
pub struct DiskState {
    prev: HashMap<String, (u64, u64, u64)>,
    at: Option<Instant>,
}

fn is_whole_disk(name: &str) -> bool {
    !(name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") || name.starts_with("sr"))
        && std::path::Path::new("/sys/block").join(name).exists()
}

/// /proc/diskstats -> (name, sectors_read, sectors_written, io_ticks_ms)
pub fn parse_diskstats(text: &str) -> Vec<(String, u64, u64, u64)> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() < 13 {
                return None;
            }
            let n = |i: usize| c[i].parse::<u64>().unwrap_or(0);
            Some((c[2].to_string(), n(5), n(9), n(12)))
        })
        .collect()
}

impl DiskState {
    pub fn sample(&mut self) -> Vec<DiskIo> {
        let now = Instant::now();
        let dt = self.at.map(|t| now.duration_since(t).as_secs_f64()).filter(|d| *d > 0.0);
        self.at = Some(now);
        parse_diskstats(&std::fs::read_to_string("/proc/diskstats").unwrap_or_default())
            .into_iter()
            .filter(|(n, ..)| is_whole_disk(n))
            .map(|(name, rd, wr, ticks)| {
                let base = std::path::Path::new("/sys/block").join(&name);
                let (read_rate, write_rate, busy_pct) = match (dt, self.prev.get(&name)) {
                    (Some(dt), Some(&(pr, pw, pt))) => (
                        rd.saturating_sub(pr) as f64 * 512.0 / dt,
                        wr.saturating_sub(pw) as f64 * 512.0 / dt,
                        (ticks.saturating_sub(pt) as f64 / (dt * 1000.0) * 100.0).min(100.0),
                    ),
                    _ => (0.0, 0.0, 0.0),
                };
                self.prev.insert(name.clone(), (rd, wr, ticks));
                DiskIo {
                    model: read_trim(base.join("device/model")).unwrap_or_default(),
                    size_bytes: read_num::<u64>(base.join("size")).unwrap_or(0) * 512,
                    rotational: read_num::<u8>(base.join("queue/rotational")) == Some(1),
                    removable: read_num::<u8>(base.join("removable")) == Some(1),
                    name,
                    read_rate,
                    write_rate,
                    busy_pct,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Filesystem {
    pub source: String,
    pub mountpoint: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

impl Filesystem {
    pub fn used_pct(&self) -> f64 {
        // Like df: used / (used + avail).
        let denom = self.used + self.avail;
        if denom == 0 {
            0.0
        } else {
            self.used as f64 / denom as f64 * 100.0
        }
    }
}

/// Decode the octal escapes (`\040` = space) used in /proc/mounts.
pub fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn statvfs(path: &str) -> Option<(u64, u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs writes into a zeroed plain-data struct; the path is NUL-terminated.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        let f = st.f_frsize as u64;
        Some((st.f_blocks as u64 * f, st.f_bfree as u64 * f, st.f_bavail as u64 * f))
    }
}

/// Real (block-device backed) filesystems, one entry per device.
pub fn filesystems() -> Vec<Filesystem> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    mounts
        .lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let (src, mp, fs) = (*c.first()?, unescape_mount(c.get(1)?), *c.get(2)?);
            if !src.starts_with("/dev/") || src.starts_with("/dev/loop") || fs == "squashfs" || !seen.insert(src.to_string()) {
                return None;
            }
            let (total, free, avail) = statvfs(&mp)?;
            Some(Filesystem { source: src.into(), mountpoint: mp, fstype: fs.into(), total, used: total.saturating_sub(free), avail })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats_and_mounts() {
        let d = parse_diskstats(" 259       0 nvme0n1 100 0 2000 50 300 0 4000 60 0 900 110 0 0 0 0\n");
        assert_eq!(d[0], ("nvme0n1".into(), 2000, 4000, 900));
        assert_eq!(unescape_mount("/media/my\\040disk"), "/media/my disk");
        let f = Filesystem { used: 75, avail: 25, ..Default::default() };
        assert_eq!(f.used_pct(), 75.0);
    }
}
