//! Privileged helper mode: `SystemHealthCheck --helper <exec|batch> <request>...`.
//!
//! Started by the GUI through `pkexec`. It accepts only registry ids marked
//! `requires_sudo`, re-validates every parameter (this is the trust boundary)
//! and never runs a shell. `exec` streams output and exits with the command's
//! code; `batch` prints a JSON array of [`RawExec`] results.

use crate::diagnostics::runner::{self, RawExec, RunControl};
use crate::util;
use std::io::Write;
use std::sync::Arc;

fn decode(req: &str) -> Result<(&'static crate::diagnostics::CommandSpec, runner::Params), String> {
    let (spec, params) = runner::decode_request(req).ok_or_else(|| format!("unknown command '{req}'"))?;
    if !spec.requires_sudo {
        return Err(format!("'{}' does not require elevation", spec.id));
    }
    runner::build_argv(spec, &params)?;
    Ok((spec, params))
}

/// Entry point; returns the process exit code.
pub fn main(args: &[String]) -> i32 {
    if !util::is_root() {
        eprintln!("systemhealthcheck-helper: must be started as root through pkexec");
        return 2;
    }
    // Single-threaded here; apt must never prompt (stdin is closed anyway).
    std::env::set_var("DEBIAN_FRONTEND", "noninteractive");
    match args {
        [mode, req] if mode == "exec" => {
            let (spec, params) = match decode(req) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("systemhealthcheck-helper: invalid: {e}");
                    return 2;
                }
            };
            let ctl = RunControl {
                on_line: Some(Arc::new(|is_err, line: &str| {
                    if is_err {
                        eprintln!("{line}");
                    } else {
                        println!("{line}");
                    }
                })),
                ..Default::default()
            };
            let raw = runner::exec_step(spec, &params, &ctl);
            if let Some(p) = raw.missing {
                eprintln!("systemhealthcheck-helper: not-installed: {p}");
                return 3;
            }
            if let Some(u) = raw.unavailable {
                eprintln!("{u}");
                return 4;
            }
            if let Some(e) = raw.spawn_error {
                eprintln!("systemhealthcheck-helper: {e}");
                return 5;
            }
            raw.code.unwrap_or(1)
        }
        [mode, reqs @ ..] if mode == "batch" => {
            let ctl = RunControl::default();
            let results: Vec<RawExec> = reqs
                .iter()
                .map(|r| match decode(r) {
                    Ok((spec, params)) => runner::exec_step(spec, &params, &ctl),
                    Err(e) => RawExec { spawn_error: Some(e), ..Default::default() },
                })
                .collect();
            match serde_json::to_string(&results) {
                Ok(json) => {
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(json.as_bytes());
                    let _ = out.flush();
                    0
                }
                Err(e) => {
                    eprintln!("systemhealthcheck-helper: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: SystemHealthCheck --helper exec <request> | batch <request>...");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn helper_accepts_only_privileged_allowlisted_requests() {
        assert!(decode("lshw-short").is_ok());
        assert!(decode("dmidecode-serial").is_ok());
        // Unknown ids, unprivileged ids and invalid parameters are refused.
        assert!(decode("rm -rf /").is_err());
        assert!(decode("uname").is_err());
        assert!(decode("smartctl-a?dev=/etc/shadow").is_err());
        assert!(decode("smartctl-a?dev=/dev/sda;reboot").is_err());
        assert!(decode("apt-install-pkg?pkg=openssh-server").is_err());
        assert!(decode("set-swappiness?value=999").is_err());
        assert!(decode("set-swappiness?value=10").is_ok());
    }
}
