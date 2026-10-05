//! Battery: native power_supply data and UPower details. Values the hardware
//! does not report are shown as such.

use crate::ui::chart::Chart;
use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, kv, label, set_usage, usage_bar, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::util::fmt_duration;

const IDS: &[&str] = &["upower-battery"];

fn opt(v: Option<f64>, unit: &str, prec: usize) -> String {
    v.map_or("Not reported".into(), |v| format!("{v:.prec$}{unit}"))
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Battery", "Charge, capacity and battery health", IDS);
    let grid = widgets::card_grid(1, 3);
    let now = Card::new("Charge", "battery-good-symbolic");
    let pct = label("…", &["big-value"]);
    let bar = usage_bar();
    let now_kv = KvGrid::new();
    now.body.append(&pct);
    now.body.append(&bar);
    now.body.append(&now_kv.grid);
    let health = Card::new("Health", "security-high-symbolic");
    let health_v = label("…", &["big-value"]);
    let health_kv = KvGrid::new();
    health.body.append(&health_v);
    health.body.append(&health_kv.grid);
    health.body.append(&label("Health = energy-full ÷ energy-full-design, computed only when both are reported.", &["caption", "dim-label"]));
    let hist = Card::new("Charge history", "utilities-system-monitor-symbolic");
    let chart = Chart::new(&["charge", "power W"], None, "", 150);
    hist.body.append(&chart.area);
    grid.append(&now.root);
    grid.append(&health.root);
    grid.append(&hist.root);
    page.overview.append(&grid);

    page.overview.append(&widgets::section("UPower details"));
    let up_kv = KvGrid::new();
    let up = Card::new("upower -i", "utilities-terminal-symbolic");
    up.body.append(&up_kv.grid);
    page.overview.append(&up.root);

    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        let b = s.batteries.first();
        chart.push(&[b.and_then(|b| b.capacity_pct), b.and_then(|b| b.power_w)]);
        if !pct.is_mapped() {
            return;
        }
        let Some(b) = b else {
            pct.set_text("No battery");
            bar.set_visible(false);
            now_kv.set(&kv([("Status", "No system battery found in /sys/class/power_supply")]));
            health_v.set_text("—");
            health_kv.set(&[]);
            return;
        };
        pct.set_text(&opt(b.capacity_pct, "%", 0));
        bar.set_visible(b.capacity_pct.is_some());
        set_usage(&bar, 100.0 - b.capacity_pct.unwrap_or(0.0));
        bar.set_fraction(b.capacity_pct.unwrap_or(0.0) / 100.0);
        let ac = s.ac_online.map_or("Unknown".to_string(), |a| if a { "Connected".into() } else { "Disconnected".into() });
        now_kv.set(&kv([
            ("State", b.status.clone()),
            ("AC adapter", ac),
            ("Energy", opt(b.energy_now_wh, " Wh", 2)),
            ("Power draw", opt(b.power_w, " W", 2)),
            ("Voltage", opt(b.voltage_v, " V", 2)),
            ("Time remaining (estimate)", b.time_remaining_h().map_or("Not available".into(), |h| fmt_duration(h * 3600.0))),
        ]));
        let h = b.health_pct();
        health_v.set_text(&opt(h, "%", 1));
        health_kv.set(&kv([
            ("Energy full", opt(b.energy_full_wh, " Wh", 2)),
            ("Energy full design", opt(b.energy_full_design_wh, " Wh", 2)),
            ("Cycle count", b.cycle_count.map_or("Not reported by hardware".into(), |c| c.to_string())),
            ("Technology", b.technology.clone().unwrap_or_else(|| "Not reported".into())),
            ("Manufacturer", b.manufacturer.clone().unwrap_or_else(|| "Not reported".into())),
            ("Model", b.model.clone().unwrap_or_else(|| "Not reported".into())),
        ]));
    });
    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        let p = st.parsed();
        match &p.battery {
            Some(b) => {
                let privacy = st.privacy();
                up_kv.set(&b.fields.iter().map(|(k, v)| (k.clone(), privacy.redact(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect::<Vec<_>>());
            }
            None => up_kv.set(&kv([("Status", st.result("upower-battery").map_or("Not run".into(), |o| format!("{} — {}", o.status.label(), o.detail)))])),
        }
    });
    page.root.upcast()
}
