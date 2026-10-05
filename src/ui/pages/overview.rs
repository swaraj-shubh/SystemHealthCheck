//! Overview: health score with full transparency (input, rule, severity,
//! reason for every finding), system summary and the unified Quick Scan result.

use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, kv, label, section, vbox, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::diagnostics::scheduler::QUICK_SCAN;
use systemhealthcheck::scoring::Severity;
use systemhealthcheck::util::{fmt_bytes, fmt_duration};

fn sev_css(s: Severity) -> &'static str {
    match s {
        Severity::Ok => "status-good",
        Severity::Attention => "status-attention",
        Severity::Warning => "status-warning",
        Severity::Critical => "status-critical",
    }
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Overview", "Health score, system summary and the unified Quick Scan result", QUICK_SCAN);
    let quick = gtk::Button::with_label("Run Quick Scan");
    quick.set_action_name(Some("win.quick-scan"));
    page.actions.prepend(&quick);

    // Score
    let score_card = Card::new("System Health", "security-high-symbolic");
    let score = label("—", &["score"]);
    let badge = widgets::badge("", "status-unknown");
    score_card.header.append(&badge);
    let rule = label("", &["dim-label"]);
    score_card.body.append(&score);
    score_card.body.append(&rule);
    score_card.body.append(&label(
        "Scores are rule-based summaries of the values collected on this machine, not predictions. Each finding below lists the input value, the rule applied, the resulting severity and why it matters.",
        &["caption", "dim-label"],
    ));
    page.overview.append(&score_card.root);

    let cats = vbox(8);
    page.overview.append(&section("Category scores"));
    page.overview.append(&cats);

    page.overview.append(&section("System summary"));
    let sys = KvGrid::new();
    let sys_card = Card::new("System", "computer-symbolic");
    sys_card.body.append(&sys.grid);
    page.overview.append(&sys_card.root);

    page.overview.append(&section("Quick Scan result"));
    page.overview.append(&label("Unified view of the checklist's 10 most valuable commands. Raw output for each is on the Raw Output tab.", &["dim-label"]));
    let qs = KvGrid::new();
    let qs_card = Card::new("Snapshot", "emblem-ok-symbolic");
    qs_card.body.append(&qs.grid);
    page.overview.append(&qs_card.root);

    let st = state.clone();
    // Expanded categories survive the periodic rebuild.
    let expanded: Rc<std::cell::RefCell<std::collections::HashSet<String>>> = Rc::default();
    let refresh = move || {
        let health = st.health.borrow().clone();
        match &health {
            Some(h) => {
                score.set_text(&h.overall.map_or("Insufficient data".into(), |o| format!("{o} / 100")));
                widgets::set_badge(&badge, h.state.label(), h.state.css());
                rule.set_text(&h.rule);
                widgets::clear(&cats);
                for c in &h.categories {
                    let exp = adw::ExpanderRow::new();
                    exp.set_title(&c.name);
                    exp.set_expanded(expanded.borrow().contains(&c.name));
                    let (ex, name) = (expanded.clone(), c.name.clone());
                    exp.connect_expanded_notify(move |e| {
                        if e.is_expanded() {
                            ex.borrow_mut().insert(name.clone());
                        } else {
                            ex.borrow_mut().remove(&name);
                        }
                    });
                    exp.set_subtitle(&c.score.map_or("Insufficient data — no inputs available".into(), |s| format!("{s} / 100 · {} findings", c.findings.len())));
                    exp.add_suffix(&widgets::state_badge(c.state));
                    for f in &c.findings {
                        let row = adw::ActionRow::new();
                        row.set_title(&gtk::glib::markup_escape_text(&f.input));
                        row.set_subtitle(&gtk::glib::markup_escape_text(&format!("Rule: {}\nWhy: {}", f.rule, f.reason)));
                        row.set_subtitle_lines(4);
                        row.add_suffix(&widgets::badge(f.severity.label(), sev_css(f.severity)));
                        exp.add_row(&row);
                    }
                    let list = gtk::ListBox::new();
                    list.add_css_class("boxed-list");
                    list.set_selection_mode(gtk::SelectionMode::None);
                    list.append(&exp);
                    cats.append(&list);
                }
            }
            None => score.set_text("Collecting…"),
        }

        let sy = &st.system;
        let s = st.sample();
        let mask = st.settings.borrow().mask_hostname;
        sys.set(&kv([
            ("Operating system", sy.os_name.clone()),
            ("Kernel", sy.kernel.clone()),
            ("Architecture", sy.arch.clone()),
            ("Hostname", if mask { "<hidden>".into() } else { sy.hostname.clone() }),
            ("Desktop", format!("{} ({})", sy.desktop, sy.session_type)),
            ("Model", format!("{} {} {}", sy.vendor, sy.product, sy.product_version)),
            ("Chassis", sy.chassis.clone()),
            ("BIOS", format!("{} {} ({})", sy.bios_vendor, sy.bios_version, sy.bios_date)),
            ("Uptime", s.as_ref().map_or("…".into(), |s| fmt_duration(s.uptime_s))),
            ("Load average", s.as_ref().map_or("…".into(), |s| format!("{:.2} {:.2} {:.2}", s.load_avg[0], s.load_avg[1], s.load_avg[2]))),
        ]));

        let p = st.parsed();
        let lscpu = |k: &str| systemhealthcheck::parsers::kv_get(&p.lscpu, k).unwrap_or("Unavailable").to_string();
        let mut rows = vec![
            ("CPU (lscpu)".to_string(), format!("{} · {} CPUs · {} threads/core · max {} MHz", lscpu("Model name"), lscpu("CPU(s)"), lscpu("Thread(s) per core"), lscpu("CPU max MHz"))),
        ];
        if let Some(s) = &s {
            let m = &s.memory;
            rows.push(("Memory (free -h)".into(), format!("{} used of {} · {} available", fmt_bytes(m.used_kb() * 1024), fmt_bytes(m.total_kb * 1024), fmt_bytes(m.available_kb * 1024))));
            rows.push((
                "Swap (swapon --show)".into(),
                if m.swaps.is_empty() { "No swap configured".into() } else { m.swaps.iter().map(|w| format!("{} {} used of {}", w.name, fmt_bytes(w.used_kb * 1024), fmt_bytes(w.size_kb * 1024))).collect::<Vec<_>>().join("; ") },
            ));
            rows.push(("Sensors".into(), format!("CPU {} · GPU {} · SSD {}", widgets::or_na(s.max_temp(SensorKind::Cpu), " °C"), widgets::or_na(s.max_temp(SensorKind::Gpu), " °C"), widgets::or_na(s.max_temp(SensorKind::Storage), " °C"))));
        }
        rows.push((
            "RAM modules (dmidecode)".into(),
            match &p.dmi_memory {
                Some(d) => format!(
                    "{} slots, max {} · {}",
                    d.slots.map_or("?".into(), |v| v.to_string()),
                    d.max_capacity.clone().unwrap_or_else(|| "?".into()),
                    d.modules.iter().map(|m| format!("{}: {}", m.locator, m.size.clone().map_or("empty".into(), |sz| format!("{sz} {} {}", m.mem_type, m.speed)))).collect::<Vec<_>>().join(", ")
                ),
                None => "Not collected (requires administrator)".into(),
            },
        ));
        rows.push(("GPU (glxinfo -B)".into(), systemhealthcheck::parsers::kv_get(&p.glxinfo, "OpenGL renderer string").unwrap_or("Unavailable").to_string()));
        rows.push((
            "NVMe health".into(),
            match &p.drive {
                Some(d) => format!("wear {} · {} · media errors {} · critical warning {}", widgets::or_na(d.percentage_used, "%"), widgets::or_na(d.temperature_c, " °C"), widgets::or_na(d.media_errors, ""), widgets::or_na(d.critical_warning, "")),
                None => "Not collected (requires administrator and nvme-cli)".into(),
            },
        ));
        rows.push((
            "Battery (upower)".into(),
            match &p.battery {
                Some(b) => format!("{} · {} · health {}", widgets::or_na(b.percentage, "%"), b.state.clone().unwrap_or_default(), widgets::or_na(b.health_pct().map(|h| format!("{h:.1}")), "%")),
                None => "No battery reported".into(),
            },
        ));
        rows.push(("Boot (systemd-analyze)".into(), p.boot.as_ref().and_then(|b| b.total_s).map_or("Unavailable".into(), |t| format!("{t:.1} s total"))));
        rows.push((
            "Top memory (ps aux)".into(),
            p.top_mem.iter().take(5).map(|r| format!("{} {:.1}%", r.command.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or(""), r.mem)).collect::<Vec<_>>().join(", "),
        ));
        qs.set(&rows);
    };
    widgets::bind(state, &page.overview, |e| matches!(e, Event::Health | Event::Results(_) | Event::Settings), refresh);
    page.root.upcast()
}
