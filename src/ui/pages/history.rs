//! History: saved scans with trend chart, view, compare, delete and export.

use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, label, Card, Page, RawView};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::diagnostics::Snapshot;
use systemhealthcheck::reports::{self, Format, ReportOptions};
use systemhealthcheck::scoring::HealthReport;
use systemhealthcheck::util::{fmt_time, now};

/// Headline metrics compared between scans.
fn metrics(s: &Snapshot, h: &HealthReport) -> Vec<(String, Option<f64>, &'static str)> {
    let p = s.parsed();
    let mut v = vec![("Overall health".to_string(), h.overall.map(f64::from), "/100")];
    for c in &h.categories {
        v.push((format!("{} score", c.name), c.score.map(f64::from), "/100"));
    }
    v.extend([
        ("SSD wear".to_string(), p.drive.as_ref().and_then(|d| d.percentage_used), "%"),
        ("SSD media errors".into(), p.drive.as_ref().and_then(|d| d.media_errors), ""),
        ("SSD unsafe shutdowns".into(), p.drive.as_ref().and_then(|d| d.unsafe_shutdowns), ""),
        ("SSD power-on hours".into(), p.drive.as_ref().and_then(|d| d.power_on_hours), " h"),
        ("Battery health".into(), s.sample.batteries.first().and_then(|b| b.health_pct()).or(p.battery.as_ref().and_then(|b| b.health_pct())), "%"),
        ("Boot time".into(), p.boot.as_ref().and_then(|b| b.total_s), " s"),
        ("Kernel errors".into(), p.kernel_counts.map(|c| c.0 as f64), ""),
        ("RAM used".into(), Some(s.sample.memory.used_pct()), "%"),
        ("CPU temperature".into(), s.sample.max_temp(SensorKind::Cpu), " °C"),
    ]);
    v
}

fn ago(ts: i64) -> String {
    let d = (now() - ts).max(0);
    match d {
        0..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86_400),
    }
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "History", "Saved scans, trends and comparisons", &[]);
    if let Some(e) = &state.db_error {
        page.overview.append(&widgets::empty_state("dialog-error-symbolic", "History unavailable", e));
        return page.root.upcast();
    }
    let trend_card = Card::new("Health trend", "utilities-system-monitor-symbolic");
    let trend = crate::ui::chart::Chart::new(&["overall"], Some(100.0), "", 140);
    trend_card.body.append(&trend.area);
    let trend_text = label("", &["dim-label"]);
    trend_card.body.append(&trend_text);
    page.overview.append(&trend_card.root);

    let bar = hbox(6);
    let view = gtk::Button::with_label("View");
    let compare = gtk::Button::with_label("Compare with previous");
    let export = gtk::Button::with_label("Export JSON…");
    let delete = gtk::Button::with_label("Delete…");
    delete.add_css_class("destructive-action");
    for b in [&view, &compare, &export, &delete] {
        bar.append(b);
    }
    page.overview.append(&bar);
    let table = DataTable::new(&[num("ID"), col("When"), col("Age"), wide("Kind"), num("Score"), col("State")], true, 0, 260);
    page.overview.append(&table.widget);
    let detail = RawView::new("scan");
    page.overview.append(&detail.widget);

    let st = state.clone();
    let t2 = table.clone();
    widgets::bind(state, &page.overview, |e| matches!(e, Event::History), move || {
        let Some(db) = &st.db else { return };
        let rows = db.list_scans().unwrap_or_default();
        t2.set_rows(rows.iter().map(|r| vec![r.id.to_string(), fmt_time(r.taken_at), ago(r.taken_at), r.kind.clone(), r.score.map_or("—".into(), |s| s.to_string()), r.state.clone()]).collect());
        let pts: Vec<_> = rows.iter().rev().filter_map(|r| r.score).collect();
        trend.reset();
        for p in &pts {
            trend.push(&[Some(*p as f64)]);
        }
        let pick = |days: i64| rows.iter().find(|r| now() - r.taken_at >= days * 86_400).and_then(|r| r.score);
        trend_text.set_text(&format!(
            "Latest {} · 7 days ago {} · 30 days ago {} · {} scans stored",
            rows.first().and_then(|r| r.score).map_or("—".into(), |s| format!("{s}/100")),
            pick(7).map_or("—".into(), |s| format!("{s}/100")),
            pick(30).map_or("—".into(), |s| format!("{s}/100")),
            rows.len()
        ));
    });

    let sel_id = {
        let t = table.clone();
        let st = state.clone();
        Rc::new(move || {
            let id = t.selected().and_then(|r| r.first().and_then(|v| v.parse::<i64>().ok()));
            if id.is_none() {
                st.toast("Select a scan first");
            }
            id
        })
    };

    let (st, sid, d) = (state.clone(), sel_id.clone(), detail.clone());
    view.connect_clicked(move |_| {
        let (Some(id), Some(db)) = (sid(), &st.db) else { return };
        match db.load_scan(id) {
            Ok((snap, health)) => {
                let rep = reports::build(&snap, &health, &[], &[], &ReportOptions { mask_hostname: st.settings.borrow().mask_hostname, ..Default::default() });
                d.set_text(&reports::to_text(&rep));
            }
            Err(e) => st.toast(&format!("Could not load scan: {e}")),
        }
    });
    let (st, sid, d) = (state.clone(), sel_id.clone(), detail.clone());
    compare.connect_clicked(move |_| {
        let (Some(id), Some(db)) = (sid(), &st.db) else { return };
        let rows = db.list_scans().unwrap_or_default();
        let Some(pos) = rows.iter().position(|r| r.id == id) else { return };
        let Some(prev) = rows.get(pos + 1) else {
            st.toast("No earlier scan to compare with");
            return;
        };
        let (Ok((a, ha)), Ok((b, hb))) = (db.load_scan(id), db.load_scan(prev.id)) else { return };
        let (ma, mb) = (metrics(&a, &ha), metrics(&b, &hb));
        let mut out = format!("Comparing scan #{} ({}) with #{} ({})\n\n{:<28} {:>14} {:>14} {:>10}\n", id, fmt_time(a.taken_at), prev.id, fmt_time(b.taken_at), "Metric", "Selected", "Previous", "Change");
        for ((name, va, unit), (_, vb, _)) in ma.iter().zip(mb.iter()) {
            let f = |v: &Option<f64>| v.map_or("—".to_string(), |x| format!("{x:.1}{unit}"));
            let delta = match (va, vb) {
                (Some(x), Some(y)) => format!("{:+.1}", x - y),
                _ => "—".into(),
            };
            out += &format!("{name:<28} {:>14} {:>14} {delta:>10}\n", f(va), f(vb));
        }
        // Commands whose status changed.
        let mut changed = Vec::new();
        for (cid, o) in &a.results {
            if let Some(po) = b.results.get(cid) {
                if po.status != o.status {
                    changed.push(format!("  {cid}: {} → {}", po.status.label(), o.status.label()));
                }
            }
        }
        if !changed.is_empty() {
            out += &format!("\nCommand status changes:\n{}\n", changed.join("\n"));
        }
        d.set_text(&out);
    });
    let (st, sid) = (state.clone(), sel_id.clone());
    export.connect_clicked(move |btn| {
        let (Some(id), Some(db)) = (sid(), &st.db) else { return };
        let Ok((snap, health)) = db.load_scan(id) else { return };
        let opts = ReportOptions { mask_hostname: st.settings.borrow().mask_hostname, exclude_network: st.settings.borrow().exclude_network_from_reports, ..Default::default() };
        let rep = reports::build(&snap, &health, &[], &[], &opts);
        match reports::to_json(&rep) {
            Ok(json) => widgets::save_text_dialog(btn, &format!("systemhealthcheck-scan-{id}.{}", Format::Json.ext()), json),
            Err(e) => st.toast(&e),
        }
    });
    let (st, sid) = (state.clone(), sel_id.clone());
    delete.connect_clicked(move |_| {
        let Some(id) = sid() else { return };
        let d = adw::AlertDialog::new(Some("Delete this scan?"), Some(&format!("Scan #{id} will be permanently removed from history.")));
        d.add_response("cancel", "Cancel");
        d.add_response("delete", "Delete");
        d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let st2 = st.clone();
        d.connect_response(None, move |_, r| {
            if r == "delete" {
                if let Some(db) = &st2.db {
                    match db.delete_scan(id) {
                        Ok(()) => st2.toast("Scan deleted"),
                        Err(e) => st2.toast(&format!("Delete failed: {e}")),
                    }
                }
                st2.emit(Event::History);
            }
        });
        let win = st.window.borrow().clone();
        d.present(win.as_ref());
    });
    page.root.upcast()
}
