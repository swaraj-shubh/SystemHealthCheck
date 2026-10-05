//! Thermals: native hwmon temperatures/fans with live graphs (replaces
//! `watch -n 1 sensors`), lm-sensors output and an explicit sensors-detect.

use crate::ui::chart::Chart;
use crate::ui::pages::live::{interval_dropdown, monitor_toggle};
use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, label, Card, Page, RawView};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};

const IDS: &[&str] = &["sensors", "watch-sensors", "sensors-detect"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Thermals", "Temperatures, fans and cooling", IDS);
    page.actions.prepend(&monitor_toggle(state));
    page.actions.prepend(&interval_dropdown(state));

    let summary = widgets::card_grid(2, 5);
    let mut tiles = Vec::new();
    for kind in [SensorKind::Cpu, SensorKind::Gpu, SensorKind::Storage, SensorKind::Board] {
        let c = widgets::vbox(2);
        c.add_css_class("card");
        c.add_css_class("sp-card");
        c.append(&label(&format!("{} temperature", kind.label()), &["caption", "dim-label"]));
        let v = label("…", &["title-2"]);
        c.append(&v);
        summary.append(&c);
        tiles.push((kind, v, c));
    }
    let fan_tile = widgets::vbox(2);
    fan_tile.add_css_class("card");
    fan_tile.add_css_class("sp-card");
    fan_tile.append(&label("Fans", &["caption", "dim-label"]));
    let fan_v = label("…", &["title-3"]);
    fan_tile.append(&fan_v);
    summary.append(&fan_tile);
    page.overview.append(&summary);

    let chart_card = Card::new("Temperature history", "utilities-system-monitor-symbolic");
    let chart = Chart::new(&["CPU", "GPU", "SSD", "Board"], None, "°C", 180);
    chart.set_accessible_summary("Temperature history chart");
    chart_card.body.append(&chart.area);
    page.overview.append(&chart_card.root);

    page.overview.append(&widgets::section("All sensors (hwmon)"));
    let table = DataTable::new(&[col("Kind"), col("Chip"), wide("Sensor"), num("Value"), num("High"), num("Critical")], true, 2, 0);
    table.sort_by(0, false);
    page.overview.append(&table.widget);

    page.overview.append(&widgets::section("lm-sensors"));
    let chips = widgets::card_grid(1, 3);
    page.overview.append(&chips);

    page.overview.append(&widgets::section("Sensor detection"));
    page.overview.append(&label(
        "If few sensors appear, `sensors-detect` can find additional monitoring chips. It probes hardware buses and may write module configuration, so it only runs after you confirm.",
        &["dim-label"],
    ));
    let detect = gtk::Button::with_label("Run sensors-detect…");
    detect.set_halign(gtk::Align::Start);
    let detect_out = RawView::new("sensors-detect");
    detect_out.widget.set_visible(false);
    page.overview.append(&detect);
    page.overview.append(&detect_out.widget);

    let tbl = table.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        chart.push(&[s.max_temp(SensorKind::Cpu), s.max_temp(SensorKind::Gpu), s.max_temp(SensorKind::Storage), s.max_temp(SensorKind::Board)]);
        if !fan_v.is_mapped() {
            return;
        }
        for (kind, v, c) in &tiles {
            let t = s.max_temp(*kind);
            v.set_text(&t.map_or("Unavailable".into(), |t| format!("{t:.1} °C")));
            for cls in ["status-good", "status-attention", "status-warning", "status-critical", "status-unknown"] {
                v.remove_css_class(cls);
            }
            v.add_css_class(t.map_or("status-unknown", |t| widgets::level_css(t, 70.0, 85.0, 95.0)));
            c.update_property(&[gtk::accessible::Property::Label(&format!("{} temperature {}", kind.label(), v.text()))]);
        }
        fan_v.set_text(&if s.fans.is_empty() {
            "No fan RPM exposed".into()
        } else {
            s.fans.iter().map(|f| format!("{} {}", f.label, widgets::or_na(f.value, " RPM"))).collect::<Vec<_>>().join("\n")
        });
        let fmt = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.1} °C"));
        let mut rows: Vec<Vec<String>> = s.sensors.iter().map(|x| vec![x.kind.label().into(), x.chip.clone(), x.label.clone(), fmt(x.value), fmt(x.high), fmt(x.crit)]).collect();
        rows.extend(s.fans.iter().map(|f| vec!["Fan".into(), f.chip.clone(), f.label.clone(), widgets::or_na(f.value, " RPM"), "—".into(), "—".into()]));
        tbl.set_rows(rows);
    });

    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        chips.remove_all();
        let p = st.parsed();
        if p.sensors.is_empty() {
            let note = match st.result("sensors") {
                Some(o) if !o.ok() => format!("{}: {}", o.status.label(), o.detail),
                _ => "No lm-sensors output yet.".into(),
            };
            chips.append(&label(&note, &["dim-label"]));
        }
        for chip in &p.sensors {
            let c = Card::new(&chip.name, "display-brightness-symbolic");
            c.body.append(&label(&chip.adapter, &["caption", "dim-label"]));
            let k = widgets::KvGrid::new();
            k.set(
                &chip
                    .readings
                    .iter()
                    .map(|r| {
                        let mut v = r.value.map_or("N/A".into(), |v| format!("{v} {}", r.unit));
                        if let Some(h) = r.high {
                            v += &format!(" (high {h})");
                        }
                        if let Some(c) = r.crit {
                            v += &format!(" (crit {c})");
                        }
                        (r.label.clone(), v)
                    })
                    .collect::<Vec<_>>(),
            );
            c.body.append(&k.grid);
            chips.append(&c.root);
        }
    });

    let st = state.clone();
    let out = detect_out.clone();
    detect.connect_clicked(move |_| {
        let Some(spec) = registry::get("sensors-detect") else { return };
        out.widget.set_visible(true);
        out.set_text("");
        let o2 = out.clone();
        let st2 = st.clone();
        st.run_spec(
            spec,
            Params::new(),
            RunControl::default(),
            Some(Rc::new(move |_, l: &str| o2.append(&format!("{l}\n")))),
            move |res| {
                if let Some(r) = res {
                    st2.toast(&format!("sensors-detect: {}", r.status.label()));
                    st2.run_ids(&["sensors"], "Re-reading sensors");
                }
            },
        );
    });
    page.root.upcast()
}
