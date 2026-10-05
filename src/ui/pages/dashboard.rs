//! Dashboard: customizable, information-dense cards fed by live samples and
//! the latest diagnostic results. Missing data is shown as such, never invented.

use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, icon_button, label, set_badge, set_usage, usage_bar, vbox, Card};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::diagnostics::RunStatus;
use systemhealthcheck::util::{fmt_bytes, fmt_duration, fmt_time};

struct Dash {
    name: String,
    card: Card,
    badge: gtk::Label,
    value: gtk::Label,
    details: gtk::Label,
    bar: gtk::ProgressBar,
    raw: gtk::Label,
    updated: gtk::Label,
}

/// (card, icon, page to expand to, commands refreshed by the card's button)
fn meta(name: &str) -> (&'static str, &'static str, &'static [&'static str]) {
    match name {
        "Health" => ("security-high-symbolic", "overview", &[]),
        "CPU" => ("applications-engineering-symbolic", "cpu", &["lscpu", "cpu-governor"]),
        "Memory" => ("media-flash-symbolic", "memory", &["free", "swapon"]),
        "GPU" => ("video-display-symbolic", "gpu", &["glxinfo", "lspci-vmm"]),
        "Storage" => ("drive-harddisk-symbolic", "storage", &["df", "lsblk"]),
        "SSD Health" => ("drive-harddisk-solidstate-symbolic", "storage", &["nvme-smart-log", "smartctl-a"]),
        "Battery" => ("battery-good-symbolic", "battery", &["upower-battery"]),
        "Thermals" => ("display-brightness-symbolic", "thermals", &["sensors"]),
        "Boot" => ("system-reboot-symbolic", "boot", &["systemd-analyze", "systemd-blame"]),
        "Kernel" => ("dialog-warning-symbolic", "kernel", &["dmesg-decoded", "dmesg-errwarn"]),
        "Network" => ("network-wireless-symbolic", "network", &["nmcli-status", "iw-dev"]),
        _ => ("dialog-question-symbolic", "dashboard", &[]),
    }
}

fn first_raw(state: &AppState, ids: &[&str]) -> Option<String> {
    ids.iter().find_map(|i| state.result(i)).map(|o| {
        let text = if o.ok() { o.stdout } else { format!("{}: {}", o.status.label(), o.detail) };
        state.privacy().redact(&text.lines().take(12).collect::<Vec<_>>().join("\n"))
    })
}

/// Explain why privileged data is missing.
fn why_missing(state: &AppState, ids: &[&str]) -> String {
    match ids.iter().find_map(|i| state.result(i)) {
        None => "Not collected yet. Run Quick Scan or Full Diagnostic (administrator authentication required).".into(),
        Some(o) if o.status == RunStatus::NotInstalled => format!("Tool not installed: {}", o.detail),
        Some(o) => format!("{}: {}", o.status.label(), o.detail),
    }
}

fn update(state: &AppState, d: &Dash) {
    let s = state.sample();
    let p = state.parsed();
    let th = state.settings.borrow().thresholds.clone();
    let mut bar: Option<f64> = None;
    let (badge, css, value, details): (String, &str, String, String) = match d.name.as_str() {
        "Health" => match state.health.borrow().as_ref() {
            Some(h) => (
                h.state.label().into(),
                h.state.css(),
                h.overall.map_or("—".into(), |o| format!("{o}/100")),
                h.categories.iter().map(|c| format!("{} {}", c.name, c.score.map_or("n/a".into(), |v| v.to_string()))).collect::<Vec<_>>().join(" · "),
            ),
            None => ("Unknown".into(), "status-unknown", "—".into(), "Collecting data…".into()),
        },
        "CPU" => match &s {
            Some(s) => {
                let temp = s.max_temp(SensorKind::Cpu);
                let css = temp.map_or("status-unknown", |t| widgets::level_css(t, th.cpu_temp[0], th.cpu_temp[1], th.cpu_temp[2]));
                bar = s.cpu.total_usage;
                (
                    temp.map_or("Temp n/a".into(), |t| format!("{t:.0} °C")),
                    css,
                    s.cpu.total_usage.map_or("…".into(), |u| format!("{u:.0}%")),
                    format!(
                        "{}\n{} cores / {} threads\nFrequency: {}\nGovernor: {}",
                        s.cpu.model,
                        s.cpu.cores.map_or("?".into(), |c| c.to_string()),
                        s.cpu.threads,
                        s.cpu.avg_mhz().map_or("Unavailable".into(), |m| format!("{:.2} GHz", m / 1000.0)),
                        s.cpu.governor.clone().unwrap_or_else(|| "Unavailable".into())
                    ),
                )
            }
            None => ("…".into(), "status-unknown", "…".into(), "Waiting for first sample".into()),
        },
        "Memory" => match &s {
            Some(s) => {
                let m = &s.memory;
                let pct = m.used_pct();
                bar = Some(pct);
                (
                    format!("{pct:.0}%"),
                    widgets::level_css(pct, th.mem_used_pct[0], th.mem_used_pct[1], th.mem_used_pct[2]),
                    format!("{} / {}", fmt_bytes((m.total_kb - m.available_kb.min(m.total_kb)) * 1024), fmt_bytes(m.total_kb * 1024)),
                    format!(
                        "Available {}\nCached {}\nSwap {} / {}",
                        fmt_bytes(m.available_kb * 1024),
                        fmt_bytes(m.cached_kb * 1024),
                        fmt_bytes(m.swap_used_kb() * 1024),
                        fmt_bytes(m.swap_total_kb * 1024)
                    ),
                )
            }
            None => ("…".into(), "status-unknown", "…".into(), String::new()),
        },
        "GPU" => {
            let renderer = systemhealthcheck::parsers::kv_get(&p.glxinfo, "OpenGL renderer string").map(str::to_string);
            let pci = p.pci.iter().find(|d| d.is_display());
            let busy = s.as_ref().and_then(|s| s.gpus.iter().find_map(|g| g.busy_pct));
            bar = busy;
            let temp = s.as_ref().and_then(|s| s.max_temp(SensorKind::Gpu));
            (
                temp.map_or("Temp n/a".into(), |t| format!("{t:.0} °C")),
                temp.map_or("status-unknown", |t| widgets::level_css(t, th.gpu_temp[0], th.gpu_temp[1], th.gpu_temp[2])),
                renderer
                    .as_deref()
                    .map(|r| r.split(" (").next().unwrap_or(r).to_string())
                    .or(pci.map(|d| d.device.clone()))
                    .unwrap_or_else(|| "Unavailable".into()),
                format!(
                    "{}\nDriver: {}\nOpenGL: {}\n{}",
                    renderer.clone().unwrap_or_default(),
                    pci.map(|d| d.driver.clone()).filter(|x| !x.is_empty()).or(s.as_ref().and_then(|s| s.gpus.first().map(|g| g.driver.clone()))).unwrap_or_else(|| "Unavailable".into()),
                    systemhealthcheck::parsers::kv_get(&p.glxinfo, "OpenGL core profile version string").unwrap_or("Unavailable"),
                    busy.map_or("GPU utilization is not exposed by this hardware/driver.".into(), |b| format!("Utilization {b:.0}%"))
                ),
            )
        }
        "Storage" => match s.as_ref().and_then(|s| s.filesystems.iter().find(|f| f.mountpoint == "/").or(s.filesystems.first()).cloned()) {
            Some(root) => {
                let pct = root.used_pct();
                bar = Some(pct);
                let disks = s.as_ref().map(|s| s.disks.iter().map(|d| format!("{} {} ({})", d.name, d.model, fmt_bytes(d.size_bytes))).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                (
                    format!("{pct:.0}% used"),
                    widgets::level_css(pct, th.fs_used_pct[0], th.fs_used_pct[1], th.fs_used_pct[2]),
                    format!("{} / {}", fmt_bytes(root.used), fmt_bytes(root.used + root.avail)),
                    format!("{} on {} · {} free\n{disks}", root.mountpoint, root.source, fmt_bytes(root.avail)),
                )
            }
            None => ("n/a".into(), "status-unknown", "Unavailable".into(), "No block-device filesystems found".into()),
        },
        "SSD Health" => match p.drive.as_ref().filter(|d| d.has_data()) {
            Some(dr) => {
                let worst = state.health.borrow().as_ref().and_then(|h| h.category("Storage").map(|c| (c.state.label(), c.state.css())));
                let (b, c) = worst.unwrap_or(("Unknown", "status-unknown"));
                (
                    b.into(),
                    c,
                    dr.percentage_used.map_or("Wear n/a".into(), |w| format!("Wear {w:.0}%")),
                    format!(
                        "Temperature: {}\nMedia errors: {}\nCritical warning: {}\nUnsafe shutdowns: {}\nPower-on hours: {}\nSMART: {}",
                        widgets::or_na(dr.temperature_c, " °C"),
                        widgets::or_na(dr.media_errors, ""),
                        widgets::or_na(dr.critical_warning, ""),
                        widgets::or_na(dr.unsafe_shutdowns, ""),
                        widgets::or_na(dr.power_on_hours, ""),
                        dr.smart_passed.map_or("Not reported", |x| if x { "PASSED" } else { "FAILED" })
                    ),
                )
            }
            None => ("Unavailable".into(), "status-unknown", "No data".into(), why_missing(state, &["nvme-smart-log", "smartctl-a"])),
        },
        "Battery" => match s.as_ref().and_then(|s| s.batteries.first().cloned()) {
            Some(b) => {
                bar = b.capacity_pct;
                let health = b.health_pct();
                (
                    health.map_or("Health n/a".into(), |h| format!("Health {h:.0}%")),
                    health.map_or("status-unknown", |h| {
                        let t = th.battery_health_pct;
                        if h < t[2] { "status-critical" } else if h < t[1] { "status-warning" } else if h < t[0] { "status-attention" } else { "status-good" }
                    }),
                    b.capacity_pct.map_or("n/a".into(), |c| format!("{c:.0}%")),
                    format!(
                        "{}{}\nPower: {}\nCycles: {}",
                        b.status,
                        b.time_remaining_h().map_or(String::new(), |h| format!(" · {} remaining", fmt_duration(h * 3600.0))),
                        b.power_w.map_or("Unavailable".into(), |w| format!("{w:.1} W")),
                        b.cycle_count.map_or("Not reported".into(), |c| c.to_string())
                    ),
                )
            }
            None => ("None".into(), "status-unknown", "No battery".into(), "No system battery is exposed in /sys/class/power_supply.".into()),
        },
        "Thermals" => match &s {
            Some(s) => {
                let t = |k| s.max_temp(k).map_or("n/a".into(), |v| format!("{v:.0} °C"));
                let max = s.sensors.iter().filter_map(|x| x.value).fold(f64::NAN, f64::max);
                let fans = if s.fans.is_empty() { "No fan RPM exposed".into() } else { s.fans.iter().map(|f| format!("{} {}", f.label, widgets::or_na(f.value, " RPM"))).collect::<Vec<_>>().join(", ") };
                (
                    if max.is_nan() { "n/a".into() } else { format!("max {max:.0} °C") },
                    if max.is_nan() { "status-unknown" } else { widgets::level_css(max, th.cpu_temp[0], th.cpu_temp[1], th.cpu_temp[2]) },
                    format!("CPU {}", t(SensorKind::Cpu)),
                    format!("GPU {} · SSD {}\nBoard {}\n{fans}", t(SensorKind::Gpu), t(SensorKind::Storage), t(SensorKind::Board)),
                )
            }
            None => ("…".into(), "status-unknown", "…".into(), String::new()),
        },
        "Boot" => match &p.boot {
            Some(b) => (
                state.health.borrow().as_ref().and_then(|h| h.category("Boot").map(|c| c.state.label().to_string())).unwrap_or_default(),
                state.health.borrow().as_ref().and_then(|h| h.category("Boot").map(|c| c.state.css())).unwrap_or("status-unknown"),
                b.total_s.map_or("n/a".into(), |t| format!("{t:.1} s")),
                format!(
                    "Kernel {} · Userspace {}\nSlowest:\n{}",
                    widgets::or_na(b.kernel_s, " s"),
                    widgets::or_na(b.userspace_s, " s"),
                    p.blame.iter().take(3).map(|(t, u)| format!("  {t:.1} s  {u}")).collect::<Vec<_>>().join("\n")
                ),
            ),
            None => ("Unknown".into(), "status-unknown", "n/a".into(), why_missing(state, &["systemd-analyze"])),
        },
        "Kernel" => match p.kernel_counts {
            Some((e, w)) => (
                if e == 0 { "Clean".into() } else { format!("{e} errors") },
                widgets::level_css(e as f64, th.kernel_errors[0], th.kernel_errors[1], th.kernel_errors[2]),
                format!("{e} errors"),
                format!("{w} warnings since boot\nLatest: {}", p.kernel_msgs.last().map(|m| m.message.chars().take(80).collect::<String>()).unwrap_or_default()),
            ),
            None if !p.kernel_msgs.is_empty() => ("Unknown".into(), "status-unknown", format!("{} messages", p.kernel_msgs.len()), "Severity not decoded".into()),
            None => ("Unknown".into(), "status-unknown", "n/a".into(), why_missing(state, &["dmesg-decoded", "dmesg-errwarn"])),
        },
        "Network" => match &s {
            Some(s) => {
                let up: Vec<_> = s.net.iter().filter(|n| n.kind != "loopback" && n.state == "up").collect();
                (
                    format!("{} up", up.len()),
                    if up.is_empty() { "status-attention" } else { "status-good" },
                    up.iter().find(|n| n.kind == "wifi" || n.kind == "ethernet").map_or("Offline".into(), |n| n.name.clone()),
                    up.iter()
                        .take(4)
                        .map(|n| format!("{} ({}) ↓{}/s ↑{}/s", n.name, n.kind, fmt_bytes(n.rx_rate as u64), fmt_bytes(n.tx_rate as u64)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            None => ("…".into(), "status-unknown", "…".into(), String::new()),
        },
        _ => ("".into(), "status-unknown", "".into(), "".into()),
    };
    set_badge(&d.badge, &badge, css);
    d.badge.set_visible(!badge.is_empty());
    d.value.set_text(&value);
    d.details.set_text(&details);
    d.bar.set_visible(bar.is_some());
    if let Some(b) = bar {
        set_usage(&d.bar, b);
        if d.name == "Battery" {
            // For charge, low is bad.
            let css = if b <= 10.0 { "status-critical" } else if b <= 20.0 { "status-warning" } else { "status-good" };
            for c in ["status-good", "status-attention", "status-warning", "status-critical"] {
                d.bar.remove_css_class(c);
            }
            d.bar.add_css_class(css);
        }
    }
    d.card.root.update_property(&[gtk::accessible::Property::Description(&format!("{} {badge} {value}", d.name))]);
    let (_, _, ids) = meta(&d.name);
    let cmd_time = ids.iter().filter_map(|i| state.result(i)).map(|o| o.finished_at).max();
    d.updated.set_text(&match d.name.as_str() {
        "Health" => "Recomputed every few seconds from live data and the latest results".to_string(),
        "CPU" | "Memory" | "Storage" | "Battery" | "Thermals" | "Network" => {
            s.as_ref().map_or("Waiting for live data".into(), |s| format!("Live from /proc and /sys · {}", fmt_time(s.taken_at)))
        }
        _ => cmd_time.map_or("Not collected yet".into(), |t| format!("Updated {}", fmt_time(t))),
    });
    let show_raw = state.settings.borrow().show_raw_on_dashboard;
    let raw = if show_raw { first_raw(state, ids) } else { None };
    d.raw.set_visible(raw.is_some());
    d.raw.set_text(raw.as_deref().unwrap_or(""));
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let root = vbox(12);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);
    let head = crate::ui::widgets::hbox(12);
    let titles = vbox(2);
    titles.append(&label("Dashboard", &["title-1"]));
    let sub = label("", &["dim-label"]);
    titles.append(&sub);
    titles.set_hexpand(true);
    head.append(&titles);
    let customize = gtk::Button::with_label("Customize…");
    customize.set_valign(gtk::Align::Center);
    customize.set_tooltip_text(Some("Choose and reorder dashboard cards in Settings"));
    let st = state.clone();
    customize.connect_clicked(move |_| st.go("settings"));
    head.append(&customize);
    root.append(&head);

    let grid = crate::ui::widgets::card_grid(1, 4);
    root.append(&grid);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(&root));

    let cards: Rc<std::cell::RefCell<Vec<Rc<Dash>>>> = Rc::default();
    let rebuild = {
        let st = state.clone();
        let grid = grid.clone();
        let cards = cards.clone();
        move || {
            grid.remove_all();
            let mut v = Vec::new();
            for name in st.settings.borrow().visible_cards() {
                let (icon, page, ids) = meta(&name);
                let card = Card::new(&name, icon);
                let badge = widgets::badge("", "status-unknown");
                card.header.append(&badge);
                if !ids.is_empty() {
                    let r = icon_button("view-refresh-symbolic", &format!("Refresh {name}"));
                    let st2 = st.clone();
                    r.connect_clicked(move |_| st2.run_ids(ids, "Dashboard refresh"));
                    card.header.append(&r);
                }
                let e = icon_button("go-next-symbolic", &format!("Open {name} details"));
                let st2 = st.clone();
                e.connect_clicked(move |_| st2.go(page));
                card.header.append(&e);
                let value = label("…", &["big-value"]);
                // Large font: cap natural width so the grid fits several columns.
                value.set_max_width_chars(14);
                let bar = usage_bar();
                let details = label("", &["dim-label"]);
                details.set_selectable(true);
                let raw = label("", &["mono", "caption"]);
                raw.set_selectable(true);
                let updated = label("", &["caption", "dim-label"]);
                card.body.append(&value);
                card.body.append(&bar);
                card.body.append(&details);
                card.body.append(&raw);
                card.body.append(&updated);
                grid.append(&card.root);
                let d = Rc::new(Dash { name, card, badge, value, details, bar, raw, updated });
                update(&st, &d);
                v.push(d);
            }
            *cards.borrow_mut() = v;
        }
    };
    rebuild();
    let st = state.clone();
    let update_sub = move || {
        sub.set_text(&format!(
            "{} · {} · last scan {}",
            if st.system.product.is_empty() {
                "Unknown model".into()
            } else if st.system.product.starts_with(&st.system.vendor) {
                st.system.product.clone()
            } else {
                format!("{} {}", st.system.vendor, st.system.product)
            },
            st.system.os_name,
            st.last_scan.get().map_or("never".into(), fmt_time)
        ));
    };
    update_sub();
    let st = state.clone();
    let rebuild = Rc::new(rebuild);
    let root_w = root.clone();
    let cards_map = cards.clone();
    let st_map = state.clone();
    root.connect_map(move |_| {
        for d in cards_map.borrow().iter() {
            update(&st_map, d);
        }
    });
    state.subscribe(move |e| match e {
        Event::Settings => {
            rebuild();
            if let Some(w) = st.window.borrow().as_ref() {
                if st.settings.borrow().compact {
                    w.add_css_class("compact-mode");
                } else {
                    w.remove_css_class("compact-mode");
                }
            }
        }
        Event::History => update_sub(),
        Event::Sample(_) | Event::Results(_) | Event::Health => {
            if root_w.is_mapped() {
                for d in cards.borrow().iter() {
                    update(&st, d);
                }
            }
        }
        _ => {}
    });
    scroll.upcast()
}
