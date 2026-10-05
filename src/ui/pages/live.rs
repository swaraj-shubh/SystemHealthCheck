//! Live Monitor: native /proc + /sys sampling with charts. No shell commands
//! are spawned per tick. Monitoring can be stopped at any time.

use crate::ui::chart::Chart;
use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, hbox, label, Card, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::util::fmt_bytes;

pub const INTERVALS: [(u64, &str); 6] = [(250, "250 ms"), (500, "500 ms"), (1000, "1 s"), (2000, "2 s"), (5000, "5 s"), (10000, "10 s")];

/// Interval dropdown bound to settings.refresh_ms.
pub fn interval_dropdown(state: &Rc<AppState>) -> gtk::DropDown {
    let dd = gtk::DropDown::from_strings(&INTERVALS.map(|i| i.1));
    dd.set_tooltip_text(Some("Refresh interval"));
    dd.update_property(&[gtk::accessible::Property::Label("Refresh interval")]);
    let cur = state.settings.borrow().refresh_ms;
    dd.set_selected(INTERVALS.iter().position(|i| i.0 == cur).unwrap_or(2) as u32);
    let st = state.clone();
    dd.connect_selected_notify(move |d| {
        if let Some(&(ms, _)) = INTERVALS.get(d.selected() as usize) {
            if st.settings.borrow().refresh_ms != ms {
                st.settings.borrow_mut().refresh_ms = ms;
                st.save_settings();
            }
        }
    });
    dd
}

/// Start/stop toggle bound to the monitor.
pub fn monitor_toggle(state: &Rc<AppState>) -> gtk::ToggleButton {
    let t = gtk::ToggleButton::new();
    let st = state.clone();
    let sync = {
        let t = t.clone();
        let st = st.clone();
        move || {
            let on = st.monitoring.load(std::sync::atomic::Ordering::SeqCst);
            t.set_active(on);
            t.set_label(if on { "Stop monitoring" } else { "Start monitoring" });
        }
    };
    sync();
    t.connect_clicked(move |b| {
        if b.is_active() {
            st.start_monitor();
        } else {
            st.stop_monitor();
        }
    });
    state.subscribe(move |e| {
        if matches!(e, Event::Busy) {
            sync();
        }
    });
    t
}

fn chart_card(title: &str, icon: &str, chart: &Chart) -> Card {
    let c = Card::new(title, icon);
    chart.set_accessible_summary(title);
    c.body.append(&chart.area);
    c
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Live Monitor", "Native sampling of /proc and /sys — no commands are spawned per refresh", &[]);
    page.actions.append(&interval_dropdown(state));
    page.actions.append(&monitor_toggle(state));

    let grid = widgets::card_grid(1, 2);
    let cpu = Chart::new(&["CPU"], Some(100.0), "%", 120);
    let freq = Chart::new(&["avg MHz"], None, "", 120);
    let temp = Chart::new(&["CPU", "GPU", "SSD"], None, "°C", 120);
    let mem = Chart::new(&["RAM", "Swap"], Some(100.0), "%", 120);
    let gpu = Chart::new(&["GPU busy"], Some(100.0), "%", 120);
    let net = Chart::new(&["rx KiB/s", "tx KiB/s"], None, "", 120);
    let disk = Chart::new(&["read KiB/s", "write KiB/s"], None, "", 120);
    let bat = Chart::new(&["Battery"], Some(100.0), "%", 120);
    let gpu_note = label("", &["caption", "dim-label"]);
    let gpu_card = chart_card("GPU", "video-display-symbolic", &gpu);
    gpu_card.body.append(&gpu_note);
    for c in [
        chart_card("CPU utilization", "applications-engineering-symbolic", &cpu),
        chart_card("CPU frequency", "power-profile-performance-symbolic", &freq),
        chart_card("Temperatures", "display-brightness-symbolic", &temp),
        chart_card("Memory & swap", "media-flash-symbolic", &mem),
        gpu_card,
        chart_card("Network (all interfaces)", "network-wireless-symbolic", &net),
        chart_card("Disk I/O (all disks)", "drive-harddisk-symbolic", &disk),
        chart_card("Battery", "battery-good-symbolic", &bat),
    ] {
        grid.append(&c.root);
    }
    page.overview.append(&grid);

    let procs_card = Card::new("Top processes (CPU)", "view-list-symbolic");
    let procs = label("", &["mono"]);
    procs.set_selectable(true);
    procs_card.body.append(&procs);
    let more = gtk::Button::with_label("Open process monitor");
    let st = state.clone();
    more.connect_clicked(move |_| st.go("processes"));
    let hb = hbox(0);
    hb.append(&more);
    procs_card.body.append(&hb);
    page.overview.append(&procs_card.root);

    // Charts accumulate history even while hidden; drawing only happens when visible.
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        cpu.push(&[s.cpu.total_usage]);
        freq.push(&[s.cpu.avg_mhz()]);
        temp.push(&[s.max_temp(SensorKind::Cpu), s.max_temp(SensorKind::Gpu), s.max_temp(SensorKind::Storage)]);
        mem.push(&[Some(s.memory.used_pct()), Some(s.memory.swap_used_pct().unwrap_or(0.0))]);
        let busy = s.gpus.iter().find_map(|g| g.busy_pct);
        gpu.push(&[busy]);
        gpu_note.set_text(if busy.is_some() { "" } else { "GPU utilization is not exposed by this hardware/driver." });
        let (rx, tx) = s.net.iter().filter(|n| n.kind != "loopback").fold((0.0, 0.0), |a, n| (a.0 + n.rx_rate, a.1 + n.tx_rate));
        net.push(&[Some(rx / 1024.0), Some(tx / 1024.0)]);
        let (r, w) = s.disks.iter().fold((0.0, 0.0), |a, d| (a.0 + d.read_rate, a.1 + d.write_rate));
        disk.push(&[Some(r / 1024.0), Some(w / 1024.0)]);
        bat.push(&[s.batteries.first().and_then(|b| b.capacity_pct)]);
        if procs.is_mapped() && !s.processes.is_empty() {
            let mut p: Vec<_> = s.processes.iter().collect();
            p.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
            let lines: Vec<String> =
                p.iter().take(8).map(|x| format!("{:>7} {:>6.1}% {:>10}  {}", x.pid, x.cpu_pct, fmt_bytes(x.rss_bytes), x.name)).collect();
            procs.set_text(&format!("{:>7} {:>7} {:>10}  {}\n{}", "PID", "CPU", "RSS", "NAME", lines.join("\n")));
        }
    });
    page.root.upcast()
}
