//! Report generation: choose a source (saved scan or current session),
//! privacy options, preview, and export to PDF / HTML / JSON / text.

use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, hbox, label, Card, Page, RawView};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use systemhealthcheck::diagnostics::scheduler::ScanKind;
use systemhealthcheck::diagnostics::Snapshot;
use systemhealthcheck::reports::{self, Format, ReportOptions};
use systemhealthcheck::scoring::HealthReport;
use systemhealthcheck::util::{fmt_time, now};

fn current_snapshot(state: &AppState) -> Option<(Snapshot, HealthReport)> {
    let sample = state.sample()?;
    let snap = Snapshot { taken_at: now(), kind: ScanKind::Full, system: state.system.clone(), sample: (*sample).clone(), results: state.results.borrow().clone() };
    let th = state.settings.borrow().thresholds.clone();
    let h = systemhealthcheck::scoring::evaluate(&snap.sample, &snap.parsed(), &th);
    Some((snap, h))
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Reports", "Complete diagnostic report — generated locally, never uploaded", &[]);

    let cfg = Card::new("Report source & privacy", "preferences-system-privacy-symbolic");
    let src_row = hbox(8);
    let src = gtk::DropDown::from_strings(&["Current session data"]);
    src.update_property(&[gtk::accessible::Property::Label("Report source")]);
    src_row.append(&label("Source", &["dim-label"]));
    src_row.append(&src);
    cfg.body.append(&src_row);
    let serial = gtk::CheckButton::with_label("Include serial number (not recommended for shared reports)");
    let host = gtk::CheckButton::with_label("Mask hostname");
    let net = gtk::CheckButton::with_label("Exclude network details (MAC/IP addresses, SSIDs, connection names)");
    host.set_active(state.settings.borrow().mask_hostname);
    net.set_active(state.settings.borrow().exclude_network_from_reports);
    cfg.body.append(&serial);
    cfg.body.append(&host);
    cfg.body.append(&net);
    let btns = hbox(8);
    let preview_btn = gtk::Button::with_label("Preview");
    btns.append(&preview_btn);
    let mut export_btns = Vec::new();
    for (f, name) in [(Format::Pdf, "Export PDF"), (Format::Html, "Export HTML"), (Format::Json, "Export JSON"), (Format::Text, "Export text")] {
        let b = gtk::Button::with_label(name);
        if f == Format::Pdf {
            b.add_css_class("suggested-action");
        }
        btns.append(&b);
        export_btns.push((f, b));
    }
    cfg.body.append(&btns);
    cfg.body.append(&label(
        "Sections: system summary, overall health, CPU, RAM, swap, GPU, storage, NVMe/SMART, temperature, battery, processes, boot, kernel, PCI, USB, network, motherboard, system model, benchmark, stress test, package status, recommendations and raw diagnostic results.",
        &["caption", "dim-label"],
    ));
    page.overview.append(&cfg.root);
    let preview = RawView::new("report-preview");
    page.overview.append(&preview.widget);

    // Scan list for the source dropdown.
    let scans: Rc<RefCell<Vec<i64>>> = Rc::default();
    let st = state.clone();
    let (src2, scans2) = (src.clone(), scans.clone());
    widgets::bind(state, &page.overview, |e| matches!(e, Event::History), move || {
        let rows = st.db.as_ref().and_then(|d| d.list_scans().ok()).unwrap_or_default();
        let mut names = vec!["Current session data".to_string()];
        names.extend(rows.iter().map(|r| format!("{} — {} — {}", fmt_time(r.taken_at), r.kind, r.score.map_or("n/a".into(), |s| format!("{s}/100")))));
        *scans2.borrow_mut() = rows.iter().map(|r| r.id).collect();
        let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
        src2.set_model(Some(&model));
        src2.set_selected(if rows.is_empty() { 0 } else { 1 });
    });

    // Serial inclusion needs explicit confirmation.
    let st = state.clone();
    let confirmed = Rc::new(std::cell::Cell::new(false));
    serial.connect_toggled(move |c| {
        if !c.is_active() {
            confirmed.set(false);
            return;
        }
        if confirmed.get() {
            return;
        }
        c.set_active(false);
        let d = adw::AlertDialog::new(Some("Include serial number?"), Some("The serial number uniquely identifies this machine. Only include it in reports you will not share publicly."));
        d.add_response("cancel", "Cancel");
        d.add_response("include", "Include");
        d.set_response_appearance("include", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let (c2, ok) = (c.clone(), confirmed.clone());
        d.connect_response(None, move |_, r| {
            if r == "include" {
                ok.set(true);
                c2.set_active(true);
            }
        });
        let win = st.window.borrow().clone();
        d.present(win.as_ref());
    });

    let build_report = {
        let st = state.clone();
        let (src, scans, serial, host, net) = (src.clone(), scans.clone(), serial.clone(), host.clone(), net.clone());
        Rc::new(move || -> Option<reports::Report> {
            let idx = src.selected() as usize;
            let data = if idx == 0 {
                current_snapshot(&st)
            } else {
                let id = *scans.borrow().get(idx - 1)?;
                st.db.as_ref()?.load_scan(id).ok()
            };
            let Some((snap, health)) = data else {
                st.toast("No data available yet for a report");
                return None;
            };
            let benches = st.db.as_ref().and_then(|d| d.list_bench("sysbench").ok()).unwrap_or_default();
            let stresses = st.db.as_ref().and_then(|d| d.list_bench("stress").ok()).unwrap_or_default();
            let opts = ReportOptions { include_serial: serial.is_active(), mask_hostname: host.is_active(), exclude_network: net.is_active() };
            Some(reports::build(&snap, &health, &benches, &stresses, &opts))
        })
    };

    let br = build_report.clone();
    let pv = preview.clone();
    preview_btn.connect_clicked(move |_| {
        if let Some(r) = br() {
            pv.set_text(&reports::to_text(&r));
        }
    });
    for (fmt, b) in export_btns {
        let br = build_report.clone();
        let st = state.clone();
        b.connect_clicked(move |btn| {
            let Some(rep) = br() else { return };
            let name = format!("systemhealthcheck-report-{}.{}", chrono::Local::now().format("%Y%m%d-%H%M%S"), fmt.ext());
            let dialog = gtk::FileDialog::builder().title("Export report").initial_name(name.as_str()).modal(true).build();
            let win = btn.root().and_downcast::<gtk::Window>();
            let st2 = st.clone();
            gtk::glib::spawn_future_local(async move {
                let Ok(file) = dialog.save_future(win.as_ref()).await else { return };
                let Some(path) = file.path() else { return };
                // PDF rendering can take a moment on large reports: do it off the main thread.
                let p2 = path.clone();
                let res = gtk::gio::spawn_blocking(move || reports::export(&rep, fmt, &p2)).await;
                match res {
                    Ok(Ok(())) => st2.toast(&format!("Report saved to {}", path.display())),
                    Ok(Err(e)) => st2.toast(&format!("Export failed: {e}")),
                    Err(_) => st2.toast("Export failed"),
                }
            });
        });
    }
    page.root.upcast()
}
