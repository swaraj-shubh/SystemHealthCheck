//! CPU page: lscpu/cpupower/cpufreq data plus native per-core live monitoring
//! (replaces `watch -n 1 "grep 'cpu MHz' /proc/cpuinfo"`).

use crate::ui::chart::Chart;
use crate::ui::pages::live::{interval_dropdown, monitor_toggle};
use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, kv, label, set_usage, usage_bar, Card, KvGrid, Page};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::parsers::kv_get;

const IDS: &[&str] = &["lscpu", "lscpu-filtered", "cpu-governor", "cpu-governors", "cpupower", "watch-cpu-mhz"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "CPU", "Model, topology, caches, frequency scaling and live per-core load", IDS);
    page.actions.prepend(&monitor_toggle(state));
    page.actions.prepend(&interval_dropdown(state));

    let top = widgets::card_grid(1, 3);
    let info = Card::new("Processor", "applications-engineering-symbolic");
    let info_kv = KvGrid::new();
    info.body.append(&info_kv.grid);
    let live = Card::new("Live", "power-profile-performance-symbolic");
    let usage = label("…", &["big-value"]);
    let bar = usage_bar();
    let live_kv = KvGrid::new();
    live.body.append(&usage);
    live.body.append(&bar);
    live.body.append(&live_kv.grid);
    let freq = Card::new("Frequency scaling", "preferences-system-time-symbolic");
    let freq_kv = KvGrid::new();
    freq.body.append(&freq_kv.grid);
    top.append(&info.root);
    top.append(&live.root);
    top.append(&freq.root);
    page.overview.append(&top);

    let charts = widgets::card_grid(1, 3);
    let c_use = Chart::new(&["usage"], Some(100.0), "%", 110);
    let c_freq = Chart::new(&["avg", "max core"], None, " MHz", 110);
    let c_temp = Chart::new(&["CPU"], None, "°C", 110);
    for (t, c) in [("CPU utilization", &c_use), ("CPU frequency", &c_freq), ("CPU temperature", &c_temp)] {
        let card = Card::new(t, "utilities-system-monitor-symbolic");
        c.set_accessible_summary(t);
        card.body.append(&c.area);
        charts.append(&card.root);
    }
    page.overview.append(&charts);

    page.overview.append(&widgets::section("Per-core load and frequency"));
    let cores = gtk::Grid::new();
    cores.set_column_spacing(12);
    cores.set_row_spacing(4);
    let cores_card = Card::new("Cores", "view-list-symbolic");
    cores_card.body.append(&cores);
    page.overview.append(&cores_card.root);
    let rows: Rc<RefCell<Vec<(gtk::ProgressBar, gtk::Label)>>> = Rc::default();

    let cp = Card::new("cpupower frequency-info", "utilities-terminal-symbolic");
    let cp_kv = KvGrid::new();
    cp.body.append(&cp_kv.grid);
    page.overview.append(&cp.root);

    // Static information (from command results).
    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        let p = st.parsed();
        let l = |k: &str| kv_get(&p.lscpu, k).unwrap_or("").to_string();
        info_kv.set(&kv([
            ("Model", l("Model name")),
            ("Vendor", l("Vendor ID")),
            ("Architecture", l("Architecture")),
            ("Sockets", l("Socket(s)")),
            ("Cores per socket", l("Core(s) per socket")),
            ("Threads per core", l("Thread(s) per core")),
            ("Logical CPUs", l("CPU(s)")),
            ("L1d / L1i", format!("{} / {}", l("L1d cache"), l("L1i cache"))),
            ("L2 / L3", format!("{} / {}", l("L2 cache"), l("L3 cache"))),
            ("Virtualization", l("Virtualization")),
            ("Boost", l("Frequency boost")),
        ]));
        cp_kv.set(&if p.cpupower.is_empty() {
            kv([("Status", st.result("cpupower").map_or("Not run".into(), |o| format!("{} — {}", o.status.label(), o.detail)))])
        } else {
            p.cpupower.clone()
        });
    });

    // Live values.
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        let c = &s.cpu;
        c_use.push(&[c.total_usage]);
        c_freq.push(&[c.avg_mhz(), c.per_core_mhz.iter().copied().reduce(f64::max)]);
        c_temp.push(&[s.max_temp(SensorKind::Cpu)]);
        if !usage.is_mapped() {
            return;
        }
        usage.set_text(&c.total_usage.map_or("…".into(), |u| format!("{u:.1}%")));
        if let Some(u) = c.total_usage {
            set_usage(&bar, u);
        }
        live_kv.set(&kv([
            ("Model", c.model.clone()),
            ("Threads / cores", format!("{} / {}", c.threads, c.cores.map_or("?".into(), |x| x.to_string()))),
            ("Average frequency", c.avg_mhz().map_or("Unavailable".into(), |m| format!("{m:.0} MHz"))),
            ("Temperature", widgets::or_na(s.max_temp(SensorKind::Cpu).map(|t| format!("{t:.1}")), " °C")),
            ("Load average", format!("{:.2} {:.2} {:.2}", s.load_avg[0], s.load_avg[1], s.load_avg[2])),
        ]));
        freq_kv.set(&kv([
            ("Driver", c.driver.clone().unwrap_or_else(|| "Unavailable".into())),
            ("Governor", c.governor.clone().unwrap_or_else(|| "Unavailable".into())),
            ("Available governors", c.available_governors.clone().unwrap_or_else(|| "Unavailable".into())),
            ("Min frequency", widgets::or_na(c.min_mhz, " MHz")),
            ("Max frequency", widgets::or_na(c.max_mhz, " MHz")),
        ]));
        let mut r = rows.borrow_mut();
        if r.len() != c.per_core_usage.len() {
            widgets_clear_grid(&cores);
            r.clear();
            for i in 0..c.per_core_usage.len() {
                let name = label(&format!("cpu{i}"), &["dim-label", "mono"]);
                let pb = usage_bar();
                pb.set_hexpand(true);
                pb.set_valign(gtk::Align::Center);
                let val = label("", &["mono"]);
                val.set_width_chars(22);
                cores.attach(&name, 0, i as i32, 1, 1);
                cores.attach(&pb, 1, i as i32, 1, 1);
                cores.attach(&val, 2, i as i32, 1, 1);
                r.push((pb, val));
            }
        }
        for (i, (pb, val)) in r.iter().enumerate() {
            let u = c.per_core_usage.get(i).copied().unwrap_or(0.0);
            set_usage(pb, u);
            val.set_text(&format!("{u:5.1}%  {}", c.per_core_mhz.get(i).map_or("—".into(), |m| format!("{m:6.0} MHz"))));
        }
    });
    page.root.upcast()
}

fn widgets_clear_grid(g: &gtk::Grid) {
    while let Some(c) = g.first_child() {
        g.remove(&c);
    }
}
