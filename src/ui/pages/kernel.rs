//! Linux / kernel: OS identity (uname, os-release, hostnamectl) and kernel
//! errors/warnings from dmesg with severity filtering.

use crate::ui::state::AppState;
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, kv, label, Card, KvGrid, Page};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use systemhealthcheck::parsers::kernel::KSeverity;
use systemhealthcheck::parsers::kv_get;

const IDS: &[&str] = &["uname", "os-release", "hostnamectl", "dmesg-errwarn", "dmesg-errwarn-50", "dmesg-decoded"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Kernel", "Linux distribution, kernel and kernel errors/warnings", IDS);
    let grid = widgets::card_grid(1, 2);
    let os = Card::new("Linux", "computer-symbolic");
    let os_kv = KvGrid::new();
    os.body.append(&os_kv.grid);
    let host = Card::new("hostnamectl", "preferences-system-details-symbolic");
    let host_kv = KvGrid::new();
    host.body.append(&host_kv.grid);
    grid.append(&os.root);
    grid.append(&host.root);
    page.overview.append(&grid);

    page.overview.append(&widgets::section("Kernel errors & warnings"));
    let counts = label("", &["title-4"]);
    page.overview.append(&counts);
    let filter_bar = hbox(0);
    filter_bar.add_css_class("linked");
    let all = gtk::ToggleButton::with_label("All");
    let errs = gtk::ToggleButton::with_label("Errors");
    let warns = gtk::ToggleButton::with_label("Warnings");
    errs.set_group(Some(&all));
    warns.set_group(Some(&all));
    all.set_active(true);
    filter_bar.append(&all);
    filter_bar.append(&errs);
    filter_bar.append(&warns);
    page.overview.append(&filter_bar);
    let table = DataTable::new(&[num("Time (s)"), col("Severity"), wide("Message")], true, 0, 420);
    table.sort_by(0, true);
    page.overview.append(&table.widget);
    let mode = Rc::new(Cell::new(0u8));

    let st = state.clone();
    let m = mode.clone();
    let t = table.clone();
    let refresh = Rc::new(move || {
        let p = st.parsed();
        let s = &st.system;
        let o = |k: &str| kv_get(&p.os_release, k).unwrap_or("").to_string();
        let hostname = if st.settings.borrow().mask_hostname { "<hidden>".to_string() } else { s.hostname.clone() };
        os_kv.set(&kv([
            ("Distribution", if o("PRETTY_NAME").is_empty() { s.os_name.clone() } else { o("PRETTY_NAME") }),
            ("ID / like", format!("{} / {}", o("ID"), o("ID_LIKE"))),
            ("Version", o("VERSION")),
            ("Codename", o("VERSION_CODENAME")),
            ("Kernel", s.kernel.clone()),
            ("Kernel build", s.kernel_version.clone()),
            ("Architecture", s.arch.clone()),
            ("Hostname", hostname),
            ("Desktop environment", if s.desktop.is_empty() { "Unavailable".into() } else { format!("{} ({})", s.desktop, s.session_type) }),
            ("uname -a", st.result("uname").and_then(|r| r.text().map(|t| st.privacy().redact(t.trim()))).unwrap_or_default()),
        ]));
        let privacy = st.privacy();
        host_kv.set(&p.hostnamectl.iter().map(|(k, v)| (k.clone(), privacy.redact(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect::<Vec<_>>());
        counts.set_text(&match p.kernel_counts {
            Some((e, w)) => format!("{e} errors · {w} warnings since boot"),
            None if !p.kernel_msgs.is_empty() => format!("{} messages (severity not decoded)", p.kernel_msgs.len()),
            None => st
                .result("dmesg-decoded")
                .or(st.result("dmesg-errwarn"))
                .map_or("dmesg has not run yet".into(), |o| format!("{}: {}", o.status.label(), o.detail)),
        });
        let want = m.get();
        t.set_rows(
            p.kernel_msgs
                .iter()
                .filter(|k| match want {
                    1 => k.severity == KSeverity::Error,
                    2 => k.severity == KSeverity::Warning,
                    _ => true,
                })
                .map(|k| vec![k.timestamp.map_or("—".into(), |t| format!("{t:.6}")), k.severity.label().into(), k.message.clone()])
                .collect(),
        );
    });
    for (b, v) in [(&all, 0u8), (&errs, 1), (&warns, 2)] {
        let m = mode.clone();
        let r = refresh.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                m.set(v);
                r();
            }
        });
    }
    widgets::bind(state, &page.overview, widgets::on_data, move || refresh());
    page.root.upcast()
}
