//! Small shared helpers: path lookup, privilege checks, formatting.

use std::path::{Path, PathBuf};

/// Directories searched after `$PATH`; Debian users often lack sbin in PATH.
const EXTRA_DIRS: &[&str] = &["/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"];

/// Resolve a program name to an absolute path, like `which`.
pub fn which(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        return Path::new(prog).is_file().then(|| PathBuf::from(prog));
    }
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .filter(|d| !d.is_empty())
        .chain(EXTRA_DIRS.iter().copied())
        .map(|d| Path::new(d).join(prog))
        .find(|p| p.is_file())
}

/// `true` when running with effective uid 0.
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Unix timestamp in seconds.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Local, human-readable timestamp.
pub fn fmt_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".into())
}

/// Bytes as IEC units ("7.6 GiB").
pub fn fmt_bytes(b: u64) -> String {
    const U: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Seconds as "1h 02m", "3m 05s" or "4.2 s".
pub fn fmt_duration(secs: f64) -> String {
    if secs < 60.0 {
        return format!("{secs:.1} s");
    }
    let s = secs as u64;
    if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// Read a sysfs/procfs file, trimmed. `None` if missing or unreadable.
pub fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Read and parse a numeric sysfs file.
pub fn read_num<T: std::str::FromStr>(path: impl AsRef<Path>) -> Option<T> {
    read_trim(path)?.parse().ok()
}

/// Logical CPU count.
pub fn cpu_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Quote an argv element for display only (never passed to a shell).
pub fn shell_quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,%+@".contains(c)) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(8 * 1024 * 1024 * 1024), "8.0 GiB");
        assert_eq!(fmt_duration(4.21), "4.2 s");
        assert_eq!(fmt_duration(185.0), "3m 05s");
        assert_eq!(shell_quote("--sort=-%cpu"), "--sort=-%cpu");
        assert_eq!(shell_quote("a b"), "'a b'");
    }
}
