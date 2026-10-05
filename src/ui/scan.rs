//! Quick Scan and Full Diagnostic progress dialog.

use super::pages;
use super::state::AppState;
use super::widgets::{hbox, label, vbox};
use adw::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::scheduler::{ScanEvent, ScanKind, ScanPlan, QUICK_SCAN};

struct Row {
    icon: gtk::Image,
    spinner: adw::Spinner,
    detail: gtk::Label,
}

fn row(list: &gtk::Box, title: &str) -> Row {
    let b = hbox(10);
    let spinner = adw::Spinner::new();
    let icon = gtk::Image::from_icon_name("content-loading-symbolic");
    icon.set_visible(false);
    let t = label(title, &[]);
    t.set_hexpand(true);
    let detail = label("waiting", &["dim-label", "caption"]);
    b.append(&spinner);
    b.append(&icon);
    b.append(&t);
    b.append(&detail);
    list.append(&b);
    Row { icon, spinner, detail }
}

fn finish_row(r: &Row, ok: bool, text: &str) {
    r.spinner.set_visible(false);
    r.icon.set_visible(true);
    r.icon.set_icon_name(Some(if ok { "emblem-ok-symbolic" } else { "dialog-warning-symbolic" }));
    r.icon.set_css_classes(if ok { &["success"] } else { &["warning"] });
    r.detail.set_text(text);
}

fn run(state: &Rc<AppState>, kind: ScanKind, plan: ScanPlan, rows: Vec<(String, String)>, groups: bool) {
    let dialog = adw::Dialog::new();
    dialog.set_title(kind.label());
    dialog.set_content_width(560);
    dialog.set_content_height(620);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let body = vbox(10);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.set_margin_bottom(18);
    let current = label("Starting…", &["heading"]);
    let progress = gtk::ProgressBar::new();
    let list = vbox(8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    let summary = label("", &[]);
    summary.set_visible(false);
    let buttons = hbox(8);
    buttons.set_halign(gtk::Align::End);
    let view = gtk::Button::with_label("View results");
    view.add_css_class("suggested-action");
    view.set_sensitive(false);
    let close = gtk::Button::with_label("Run in background");
    buttons.append(&close);
    buttons.append(&view);
    body.append(&current);
    body.append(&progress);
    body.append(&scroll);
    body.append(&summary);
    body.append(&buttons);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));

    let rows: HashMap<String, Row> = rows.into_iter().map(|(key, title)| (key, row(&list, &title))).collect();
    let total = rows.len().max(1);
    let d2 = dialog.clone();
    close.connect_clicked(move |_| {
        d2.close();
    });
    let d3 = dialog.clone();
    let st = state.clone();
    view.connect_clicked(move |_| {
        d3.close();
        st.go("overview");
    });
    let win = state.window.borrow().clone();
    dialog.present(win.as_ref());

    let rows = Rc::new(rows);
    let done_count = Rc::new(std::cell::Cell::new(0usize));
    let (r2, cur2, prog2, dc) = (rows.clone(), current.clone(), progress.clone(), done_count.clone());
    let on_event = move |ev: &ScanEvent| match ev {
        ScanEvent::Current(s) => cur2.set_text(s),
        ScanEvent::GroupDone(g, ok, n) if groups => {
            if let Some(r) = r2.get(g.label()) {
                finish_row(r, ok == n, &format!("{ok}/{n} succeeded"));
                dc.set(dc.get() + 1);
                prog2.set_fraction(dc.get() as f64 / total as f64);
            }
        }
        ScanEvent::Command(o) if !groups => {
            if let Some(r) = r2.get(&o.id) {
                finish_row(r, o.ok(), o.status.label());
                dc.set(dc.get() + 1);
                prog2.set_fraction(dc.get() as f64 / total as f64);
            }
        }
        _ => {}
    };
    let st = state.clone();
    state.run_scan(kind, plan, on_event, move |saved| {
        progress.set_fraction(1.0);
        current.set_text("Scan complete");
        close.set_label("Close");
        view.set_sensitive(true);
        let ok = st.results.borrow().values().filter(|o| o.ok()).count();
        let health = st.health.borrow().as_ref().and_then(|h| h.overall).map_or("insufficient data".into(), |o| format!("{o}/100"));
        let problems = st.health.borrow().as_ref().map(|h| h.problems().len()).unwrap_or(0);
        summary.set_text(&format!(
            "Health {health}. {problems} finding(s) need attention. {ok} commands have results.{}",
            if saved.is_some() { " Saved to history." } else { "" }
        ));
        summary.set_visible(true);
        st.toast(&format!("{} complete — health {health}", kind.label()));
        if kind == ScanKind::Full {
            let s = st.settings.borrow().clone();
            if s.include_benchmark_in_full {
                pages::benchmarks::run_default(&st);
            }
            if s.include_stress_in_full {
                pages::stress::start_default(&st);
            }
        }
    });
}

pub fn quick_scan(state: &Rc<AppState>) {
    let rows = QUICK_SCAN.iter().filter_map(|id| registry::get(id)).map(|s| (s.id.to_string(), s.original.to_string())).collect();
    run(state, ScanKind::Quick, ScanPlan::quick(), rows, false);
}

pub fn full_diagnostic(state: &Rc<AppState>) {
    let groups = state.settings.borrow().groups();
    let rows = groups.iter().map(|g| (g.label().to_string(), g.label().to_string())).collect();
    run(state, ScanKind::Full, ScanPlan::full(&groups), rows, true);
}
