//! Boot performance: systemd-analyze times, slowest units, the critical chain
//! as a tree, and enabled services (read-only — nothing can be disabled here).

use crate::ui::state::AppState;
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, label, Card, Page};
use adw::prelude::*;
use std::rc::Rc;

const IDS: &[&str] = &["systemd-analyze", "systemd-blame", "systemd-blame-20", "systemd-critical-chain", "systemctl-enabled"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Boot", "How long the system takes to start and which units are slowest", IDS);
    let tiles = widgets::card_grid(2, 5);
    page.overview.append(&tiles);
    let target = label("", &["dim-label"]);
    page.overview.append(&target);

    let cols = widgets::card_grid(1, 2);
    let blame_card = Card::new("Slowest units (systemd-analyze blame)", "preferences-system-time-symbolic");
    let blame = DataTable::new(&[num("Time"), wide("Unit")], true, 1, 360);
    blame.sort_by(0, true);
    blame_card.body.append(&blame.widget);
    let chain_card = Card::new("Critical chain", "system-reboot-symbolic");
    let chain = widgets::vbox(2);
    let chain_scroll = gtk::ScrolledWindow::new();
    chain_scroll.set_child(Some(&chain));
    chain_scroll.set_min_content_height(360);
    chain_card.body.append(&chain_scroll);
    cols.append(&blame_card.root);
    cols.append(&chain_card.root);
    page.overview.append(&cols);

    page.overview.append(&widgets::section("Enabled unit files"));
    page.overview.append(&label("Read-only view. SystemHealthCheck does not disable services.", &["dim-label"]));
    let enabled = DataTable::new(&[wide("Unit file"), col("State"), col("Preset")], true, 0, 300);
    page.overview.append(&enabled.widget);

    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        let p = st.parsed();
        tiles.remove_all();
        let b = p.boot.clone().unwrap_or_default();
        for (name, v) in [("Firmware", b.firmware_s), ("Loader", b.loader_s), ("Kernel", b.kernel_s), ("Initrd", b.initrd_s), ("Userspace", b.userspace_s), ("Total", b.total_s)] {
            if v.is_none() && matches!(name, "Initrd" | "Firmware" | "Loader") && p.boot.is_some() {
                continue;
            }
            let c = widgets::vbox(2);
            c.add_css_class("card");
            c.add_css_class("sp-card");
            c.append(&label(name, &["caption", "dim-label"]));
            c.append(&label(&v.map_or("Unavailable".into(), |v| format!("{v:.2} s")), &["title-2"]));
            tiles.append(&c);
        }
        target.set_text(&match (&p.boot, st.result("systemd-analyze")) {
            (Some(b), _) => b.target_line.clone().unwrap_or_default(),
            (None, Some(o)) if !o.ok() => format!("{}: {} {}", o.status.label(), o.detail, o.stderr.trim()),
            (None, Some(o)) => o.stdout.trim().to_string(),
            (None, None) => "systemd-analyze has not run yet".into(),
        });
        blame.set_rows(p.blame.iter().map(|(t, u)| vec![format!("{t:.3} s"), u.clone()]).collect());
        widgets::clear(&chain);
        for n in &p.chain {
            let row = hbox(6);
            row.set_margin_start(n.depth as i32 * 18);
            let marker = label(if n.depth == 0 { "●" } else { "└" }, &["dim-label"]);
            let name = label(&n.unit, if n.took_s.is_some_and(|t| t >= 1.0) { &["heading"][..] } else { &[][..] });
            let times = label(
                &format!("{}{}", n.at_s.map_or(String::new(), |a| format!("@{a:.3}s ")), n.took_s.map_or(String::new(), |t| format!("+{t:.3}s"))),
                &["dim-label", "caption", "mono"],
            );
            row.append(&marker);
            row.append(&name);
            row.append(&times);
            chain.append(&row);
        }
        if p.chain.is_empty() {
            chain.append(&label("No critical chain data yet.", &["dim-label"]));
        }
        enabled.set_rows(p.enabled_units.iter().map(|(u, s, pr)| vec![u.clone(), s.clone(), pr.clone()]).collect());
    });
    page.root.upcast()
}
