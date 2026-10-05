//! End-to-end on the build host: run the unprivileged startup checks, parse,
//! score and export every report format. Requires Linux, not specific hardware.

use systemhealthcheck::collectors::{system, Monitor};
use systemhealthcheck::diagnostics::runner::RunControl;
use systemhealthcheck::diagnostics::scheduler::{run_plan, ScanKind, ScanPlan};
use systemhealthcheck::diagnostics::{RunStatus, Snapshot};
use systemhealthcheck::reports::{self, Format, ReportOptions};
use systemhealthcheck::settings::DEFAULT_AUTO_IDS;

#[test]
fn unprivileged_scan_scores_and_exports() {
    let results = run_plan(&ScanPlan::ids(DEFAULT_AUTO_IDS), true, &RunControl::default(), &|_| {});
    assert_eq!(results.len(), DEFAULT_AUTO_IDS.len());
    // Every command either succeeded or carries an explanation (never silent).
    for o in results.values() {
        assert!(o.status == RunStatus::Success || !o.detail.is_empty(), "{} failed without detail", o.id);
    }
    assert!(results["uname"].ok());
    assert!(results["os-release"].ok());

    let mut mon = Monitor::new();
    mon.sample(true);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let snap = Snapshot { taken_at: 1, kind: ScanKind::Quick, system: system::read_system(), sample: mon.sample(true), results };
    let parsed = snap.parsed();
    assert!(!parsed.os_release.is_empty());
    let health = systemhealthcheck::scoring::evaluate(&snap.sample, &parsed, &Default::default());
    assert!(health.overall.is_some(), "memory and filesystems always provide inputs");

    let rep = reports::build(&snap, &health, &[], &[], &ReportOptions { exclude_network: true, ..Default::default() });
    let dir = std::env::temp_dir().join(format!("systemhealthcheck-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    for f in [Format::Pdf, Format::Html, Format::Json, Format::Text] {
        let p = dir.join(format!("report.{}", f.ext()));
        reports::export(&rep, f, &p).expect("export");
        assert!(std::fs::metadata(&p).map(|m| m.len() > 500).unwrap_or(false), "{f:?} too small");
    }
    let _ = std::fs::remove_dir_all(dir);
}
