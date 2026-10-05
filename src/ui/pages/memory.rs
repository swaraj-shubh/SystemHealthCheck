//! Memory & swap: live /proc/meminfo, dmidecode RAM slots, swap devices and
//! an explicit, reversible swappiness change.

use crate::ui::chart::Chart;
use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, kv, label, set_usage, usage_bar, vbox, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};
use systemhealthcheck::util::fmt_bytes;

const IDS: &[&str] = &["free", "dmidecode-memory", "dmidecode-memory-long", "dmidecode-memory-filtered", "swapon", "swappiness"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Memory", "RAM usage, installed modules and swap", IDS);

    let top = widgets::card_grid(1, 3);
    let ram = Card::new("RAM", "media-flash-symbolic");
    let ram_val = label("…", &["big-value"]);
    let ram_bar = usage_bar();
    let ram_kv = KvGrid::new();
    ram.body.append(&ram_val);
    ram.body.append(&ram_bar);
    ram.body.append(&ram_kv.grid);
    let swap = Card::new("Swap", "drive-harddisk-symbolic");
    let swap_val = label("…", &["big-value"]);
    let swap_bar = usage_bar();
    let swap_kv = KvGrid::new();
    swap.body.append(&swap_val);
    swap.body.append(&swap_bar);
    swap.body.append(&swap_kv.grid);
    let change = gtk::Button::with_label("Change swappiness…");
    let restore = gtk::Button::with_label("Restore original");
    restore.set_visible(false);
    let btns = widgets::hbox(6);
    btns.append(&change);
    btns.append(&restore);
    swap.body.append(&btns);
    let chart_card = Card::new("History", "utilities-system-monitor-symbolic");
    let chart = Chart::new(&["RAM", "Swap"], Some(100.0), "%", 150);
    chart_card.body.append(&chart.area);
    top.append(&ram.root);
    top.append(&swap.root);
    top.append(&chart_card.root);
    page.overview.append(&top);

    page.overview.append(&widgets::section("RAM slots"));
    let array_kv = KvGrid::new();
    page.overview.append(&array_kv.grid);
    let slots = widgets::card_grid(1, 4);
    page.overview.append(&slots);

    let st = state.clone();
    let rest = restore.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        let m = &s.memory;
        chart.push(&[Some(m.used_pct()), m.swap_used_pct().or(Some(0.0))]);
        if !ram_val.is_mapped() {
            return;
        }
        ram_val.set_text(&format!("{} / {}", fmt_bytes((m.total_kb - m.available_kb.min(m.total_kb)) * 1024), fmt_bytes(m.total_kb * 1024)));
        set_usage(&ram_bar, m.used_pct());
        ram_kv.set(&kv([
            ("Total", fmt_bytes(m.total_kb * 1024)),
            ("Used (free's definition)", fmt_bytes(m.used_kb() * 1024)),
            ("Available", fmt_bytes(m.available_kb * 1024)),
            ("Cached + reclaimable", fmt_bytes(m.cached_kb * 1024)),
            ("Buffers", fmt_bytes(m.buffers_kb * 1024)),
            ("Free", fmt_bytes(m.free_kb * 1024)),
            ("Shared", fmt_bytes(m.shared_kb * 1024)),
        ]));
        swap_val.set_text(&if m.swap_total_kb == 0 { "No swap".into() } else { format!("{} / {}", fmt_bytes(m.swap_used_kb() * 1024), fmt_bytes(m.swap_total_kb * 1024)) });
        set_usage(&swap_bar, m.swap_used_pct().unwrap_or(0.0));
        let mut rows = kv([
            ("Total", fmt_bytes(m.swap_total_kb * 1024)),
            ("Used", fmt_bytes(m.swap_used_kb() * 1024)),
            ("Free", fmt_bytes(m.swap_free_kb * 1024)),
            ("Swappiness", widgets::or_na(m.swappiness, "")),
        ]);
        for d in &m.swaps {
            rows.push((format!("{} ({})", d.name, d.kind), format!("{} used of {} · priority {}", fmt_bytes(d.used_kb * 1024), fmt_bytes(d.size_kb * 1024), d.priority)));
        }
        swap_kv.set(&rows);
        let orig = st.db.as_ref().and_then(|d| d.get_value("swappiness_original"));
        rest.set_visible(orig.as_ref().is_some_and(|o| Some(o.as_str()) != m.swappiness.map(|v| v.to_string()).as_deref()));
        if let Some(o) = orig {
            rest.set_label(&format!("Restore original ({o})"));
        }
    });

    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        widgets_clear_flow(&slots);
        match &st.parsed().dmi_memory {
            Some(d) => {
                array_kv.set(&kv([
                    ("Maximum reported capacity", d.max_capacity.clone().unwrap_or_default()),
                    ("Number of slots", d.slots.map_or(String::new(), |v| v.to_string())),
                    ("Error correction", d.error_correction.clone().unwrap_or_default()),
                ]));
                for m in &d.modules {
                    let b = vbox(2);
                    b.add_css_class("slot");
                    b.append(&label(&m.locator, &["heading"]));
                    match &m.size {
                        Some(sz) => {
                            b.add_css_class("filled");
                            b.append(&label(sz, &["title-3"]));
                            for t in [m.mem_type.as_str(), m.speed.as_str(), &format!("Configured {}", m.configured_speed), &m.form_factor, &m.manufacturer, &m.part_number] {
                                if !t.trim().is_empty() && t != "Configured " {
                                    b.append(&label(t, &["dim-label"]));
                                }
                            }
                        }
                        None => b.append(&label("Empty", &["title-3", "dim-label"])),
                    }
                    b.update_property(&[gtk::accessible::Property::Label(&format!("{} {}", m.locator, m.size.clone().unwrap_or_else(|| "empty".into())))]);
                    slots.append(&b);
                }
            }
            None => {
                array_kv.set(&[]);
                slots.append(&label("Slot information comes from `sudo dmidecode -t memory`. Click “Run diagnostics” and authenticate to read it.", &["dim-label"]));
            }
        }
    });

    let st = state.clone();
    change.connect_clicked(move |_| swappiness_dialog(&st));
    let st = state.clone();
    restore.connect_clicked(move |_| {
        if let Some(v) = st.db.as_ref().and_then(|d| d.get_value("swappiness_original")) {
            apply_swappiness(&st, v, false);
        }
    });
    page.root.upcast()
}

fn widgets_clear_flow(f: &gtk::FlowBox) {
    f.remove_all();
}

fn apply_swappiness(state: &Rc<AppState>, value: String, remember_original: bool) {
    let Some(spec) = registry::get("set-swappiness") else { return };
    let current = systemhealthcheck::util::read_trim("/proc/sys/vm/swappiness");
    let mut params = Params::new();
    params.insert("value".into(), value.clone());
    let st = state.clone();
    state.run_spec(spec, params, RunControl::default(), None, move |out| {
        let Some(o) = out else { return };
        if o.ok() {
            if remember_original {
                if let (Some(db), Some(cur)) = (&st.db, current) {
                    if db.get_value("swappiness_original").is_none() {
                        let _ = db.set_value("swappiness_original", &cur);
                    }
                }
            }
            st.toast(&format!("Swappiness set to {value} (until reboot)"));
        } else {
            st.toast(&format!("Swappiness not changed: {} {}", o.status.label(), o.detail));
        }
    });
}

fn swappiness_dialog(state: &Rc<AppState>) {
    let current: u32 = systemhealthcheck::util::read_num("/proc/sys/vm/swappiness").unwrap_or(60);
    let spin = gtk::SpinButton::with_range(0.0, 200.0, 1.0);
    spin.set_value(current as f64);
    spin.update_property(&[gtk::accessible::Property::Label("New swappiness value")]);
    let d = adw::AlertDialog::new(
        Some("Change swappiness"),
        Some(&format!(
            "Current value: {current}\n\nLower values keep more data in RAM; higher values swap earlier. The change applies until reboot and can be restored from this page. Nothing is changed until you confirm the next step."
        )),
    );
    d.set_extra_child(Some(&spin));
    d.add_response("cancel", "Cancel");
    d.add_response("next", "Continue…");
    d.set_response_appearance("next", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let st = state.clone();
    d.connect_response(None, move |_, r| {
        if r == "next" {
            apply_swappiness(&st, (spin.value() as u32).to_string(), true);
        }
    });
    let win = state.window.borrow().clone();
    d.present(win.as_ref());
}
