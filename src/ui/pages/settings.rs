//! Settings: appearance, monitoring, startup, dashboard cards, diagnostics,
//! privacy, storage and alerts. Every change is saved immediately.

use crate::ui::pages::live::INTERVALS;
use crate::ui::state::AppState;
use crate::ui::widgets::{self, label};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::scheduler::Group;
use systemhealthcheck::settings::{Settings, Theme, DASHBOARD_CARDS};

fn switch(state: &Rc<AppState>, title: &str, subtitle: &str, get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) -> adw::SwitchRow {
    let r = adw::SwitchRow::new();
    r.set_title(title);
    if !subtitle.is_empty() {
        r.set_subtitle(subtitle);
    }
    r.set_active(get(&state.settings.borrow()));
    let st = state.clone();
    r.connect_active_notify(move |r| {
        set(&mut st.settings.borrow_mut(), r.is_active());
        st.save_settings();
    });
    r
}

fn spin(state: &Rc<AppState>, title: &str, min: f64, max: f64, step: f64, get: fn(&Settings) -> f64, set: fn(&mut Settings, f64)) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(min, max, step);
    r.set_title(title);
    r.set_value(get(&state.settings.borrow()));
    let st = state.clone();
    r.connect_value_notify(move |r| {
        set(&mut st.settings.borrow_mut(), r.value());
        st.save_settings();
    });
    r
}

fn group(title: &str, desc: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    if !desc.is_empty() {
        g.set_description(Some(desc));
    }
    g
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    // Appearance
    let g = group("Appearance", "");
    let theme = adw::ComboRow::new();
    theme.set_title("Theme");
    theme.set_model(Some(&gtk::StringList::new(&["System", "Light", "Dark"])));
    theme.set_selected(match state.settings.borrow().theme {
        Theme::System => 0,
        Theme::Light => 1,
        Theme::Dark => 2,
    });
    let st = state.clone();
    theme.connect_selected_notify(move |r| {
        let t = [Theme::System, Theme::Light, Theme::Dark][r.selected().min(2) as usize];
        st.settings.borrow_mut().theme = t;
        adw::StyleManager::default().set_color_scheme(match t {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        });
        st.save_settings();
    });
    g.add(&theme);
    g.add(&switch(state, "Compact mode", "Denser dashboard cards", |s| s.compact, |s, v| s.compact = v));
    page.add(&g);

    // Monitoring
    let g = group("Monitoring", "Live data is read natively from /proc and /sys.");
    let interval = adw::ComboRow::new();
    interval.set_title("Refresh interval");
    interval.set_model(Some(&gtk::StringList::new(&INTERVALS.map(|i| i.1))));
    interval.set_selected(INTERVALS.iter().position(|i| i.0 == state.settings.borrow().refresh_ms).unwrap_or(2) as u32);
    let st = state.clone();
    interval.connect_selected_notify(move |r| {
        if let Some(&(ms, _)) = INTERVALS.get(r.selected() as usize) {
            st.settings.borrow_mut().refresh_ms = ms;
            st.save_settings();
        }
    });
    g.add(&interval);
    g.add(&switch(state, "Start monitoring automatically", "", |s| s.start_monitoring, |s, v| s.start_monitoring = v));
    page.add(&g);

    // Startup
    let g = group("Startup", "The dashboard is always the first page.");
    g.add(&switch(state, "Run Quick Scan at startup", "Asks for administrator authentication for dmidecode and nvme", |s| s.quick_scan_on_start, |s, v| s.quick_scan_on_start = v));
    let auto = adw::ExpanderRow::new();
    auto.set_title("Automatic checks at startup");
    auto.set_subtitle("Unprivileged, inexpensive commands that populate the pages");
    for spec in registry::REGISTRY.iter().filter(|s| !s.requires_sudo && !s.is_manual_only() && !s.id.starts_with("apt")) {
        let r = adw::SwitchRow::new();
        r.set_title(spec.name);
        r.set_subtitle(&gtk::glib::markup_escape_text(spec.original));
        r.set_active(state.settings.borrow().auto_ids.iter().any(|i| i == spec.id));
        let st = state.clone();
        let id = spec.id;
        r.connect_active_notify(move |r| {
            let mut s = st.settings.borrow_mut();
            s.auto_ids.retain(|i| i != id);
            if r.is_active() {
                s.auto_ids.push(id.into());
            }
            drop(s);
            st.save_settings();
        });
        auto.add_row(&r);
    }
    g.add(&auto);
    page.add(&g);

    // Dashboard
    let g = group("Dashboard", "Choose which cards are visible and their order.");
    g.add(&switch(state, "Show raw output on cards", "Adds the first lines of each card's command output", |s| s.show_raw_on_dashboard, |s, v| s.show_raw_on_dashboard = v));
    let cards_box = gtk::ListBox::new();
    cards_box.add_css_class("boxed-list");
    cards_box.set_selection_mode(gtk::SelectionMode::None);
    let rebuild: Rc<dyn Fn()> = {
        let st = state.clone();
        let cb = cards_box.clone();
        Rc::new(move || {
            while let Some(c) = cb.first_child() {
                cb.remove(&c);
            }
            let order = {
                let s = st.settings.borrow();
                let mut o: Vec<String> = s.dashboard_cards.iter().filter(|c| DASHBOARD_CARDS.contains(&c.as_str())).cloned().collect();
                for c in DASHBOARD_CARDS {
                    if !o.iter().any(|x| x == c) {
                        o.push(c.to_string());
                    }
                }
                o
            };
            for (i, name) in order.iter().enumerate() {
                let row = adw::ActionRow::new();
                row.set_title(name);
                let sw = gtk::Switch::new();
                sw.set_valign(gtk::Align::Center);
                sw.set_active(!st.settings.borrow().hidden_cards.contains(name));
                sw.update_property(&[gtk::accessible::Property::Label(&format!("Show {name} card"))]);
                let up = widgets::icon_button("go-up-symbolic", &format!("Move {name} up"));
                let down = widgets::icon_button("go-down-symbolic", &format!("Move {name} down"));
                up.set_sensitive(i > 0);
                down.set_sensitive(i + 1 < order.len());
                for (b, delta) in [(&up, -1i32), (&down, 1)] {
                    let st2 = st.clone();
                    let ord = order.clone();
                    let cb2 = cb.clone();
                    b.connect_clicked(move |_| {
                        let j = (i as i32 + delta) as usize;
                        let mut ord = ord.clone();
                        ord.swap(i, j);
                        st2.settings.borrow_mut().dashboard_cards = ord.clone();
                        st2.save_settings();
                        // Rebuild on idle so this button is not destroyed inside its own handler.
                        let cb3 = cb2.clone();
                        gtk::glib::idle_add_local_once(move || cb3.activate_action("settings.rebuild-cards", None).unwrap_or(()));
                    });
                }
                let st2 = st.clone();
                let n = name.clone();
                sw.connect_active_notify(move |s| {
                    let mut set = st2.settings.borrow_mut();
                    set.hidden_cards.retain(|c| c != &n);
                    if !s.is_active() {
                        set.hidden_cards.push(n.clone());
                    }
                    drop(set);
                    st2.save_settings();
                });
                row.add_suffix(&up);
                row.add_suffix(&down);
                row.add_suffix(&sw);
                cb.append(&row);
            }
        })
    };
    rebuild();
    let actions = gtk::gio::SimpleActionGroup::new();
    let a = gtk::gio::SimpleAction::new("rebuild-cards", None);
    let rb = rebuild.clone();
    a.connect_activate(move |_, _| rb());
    actions.add_action(&a);
    page.insert_action_group("settings", Some(&actions));
    g.add(&cards_box);
    page.add(&g);

    // Diagnostics
    let g = group("Diagnostics", "");
    g.add(&switch(state, "Parallel execution", "Run independent diagnostic groups concurrently", |s| s.parallel, |s, v| s.parallel = v));
    g.add(&spin(state, "Command timeout (seconds)", 5.0, 600.0, 5.0, |s| s.command_timeout_s as f64, |s, v| s.command_timeout_s = v as u64));
    let groups = adw::ExpanderRow::new();
    groups.set_title("Full Diagnostic groups");
    groups.set_subtitle("Groups executed by Full Diagnostic");
    for gname in Group::ALL {
        let r = adw::SwitchRow::new();
        r.set_title(gname.label());
        r.set_subtitle(&gtk::glib::markup_escape_text(&gname.ids().iter().filter_map(|i| registry::get(i)).map(|s| s.original).collect::<Vec<_>>().join(" · ")));
        r.set_active(state.settings.borrow().full_groups.iter().any(|x| x == gname.label()));
        let st = state.clone();
        r.connect_active_notify(move |r| {
            let mut s = st.settings.borrow_mut();
            s.full_groups.retain(|x| x != gname.label());
            if r.is_active() {
                s.full_groups.push(gname.label().into());
            }
            drop(s);
            st.save_settings();
        });
        groups.add_row(&r);
    }
    g.add(&groups);
    g.add(&switch(state, "Include CPU benchmark in Full Diagnostic", "Still asks for confirmation before starting", |s| s.include_benchmark_in_full, |s, v| s.include_benchmark_in_full = v));
    g.add(&switch(state, "Include stress test in Full Diagnostic", "Still shows the stress warning and asks for confirmation", |s| s.include_stress_in_full, |s, v| s.include_stress_in_full = v));
    page.add(&g);

    // Privacy
    let g = group("Privacy", "SystemHealthCheck is local-first: no telemetry, no network access, nothing is uploaded.");
    g.add(&switch(state, "Mask serial number", "Serials and UUIDs are masked in the UI and raw output", |s| s.mask_serial, |s, v| s.mask_serial = v));
    g.add(&switch(state, "Mask hostname", "", |s| s.mask_hostname, |s, v| s.mask_hostname = v));
    g.add(&switch(state, "Exclude network details from reports", "MAC/IP addresses, SSIDs and connection names", |s| s.exclude_network_from_reports, |s, v| s.exclude_network_from_reports = v));
    page.add(&g);

    // Storage
    let g = group("Storage", "");
    g.add(&spin(state, "History retention (days)", 1.0, 3650.0, 1.0, |s| s.history_retention_days as f64, |s, v| s.history_retention_days = v as u32));
    let db_row = adw::ActionRow::new();
    db_row.set_title("Database location");
    let path = state.db.as_ref().map(|d| d.path.clone()).unwrap_or_else(systemhealthcheck::database::default_path);
    db_row.set_subtitle(&gtk::glib::markup_escape_text(&format!("{}\nSet SYSTEMHEALTHCHECK_DB to use another location.", path.display())));
    db_row.set_subtitle_selectable(true);
    let open = gtk::Button::with_label("Open folder");
    open.set_valign(gtk::Align::Center);
    open.connect_clicked(move |_| {
        if let Some(dir) = path.parent() {
            widgets::open_path(dir);
        }
    });
    db_row.add_suffix(&open);
    g.add(&db_row);
    page.add(&g);

    // Alerts
    let g = group("Alerts", "Shown as in-app toasts and desktop notifications, at most once every 5 minutes per type.");
    g.add(&switch(state, "Temperature alerts", "", |s| s.alert_temperature, |s, v| s.alert_temperature = v));
    g.add(&spin(state, "Temperature alert threshold (°C)", 50.0, 110.0, 1.0, |s| s.alert_temperature_c, |s, v| s.alert_temperature_c = v));
    g.add(&spin(state, "Stress test emergency stop (°C)", 60.0, 110.0, 1.0, |s| s.emergency_stop_c, |s, v| s.emergency_stop_c = v));
    g.add(&switch(state, "Battery alerts", "", |s| s.alert_battery, |s, v| s.alert_battery = v));
    g.add(&spin(state, "Low battery threshold (%)", 1.0, 50.0, 1.0, |s| s.alert_battery_pct, |s, v| s.alert_battery_pct = v));
    g.add(&switch(state, "Storage alerts", "", |s| s.alert_storage, |s, v| s.alert_storage = v));
    g.add(&spin(state, "Filesystem full threshold (%)", 50.0, 100.0, 1.0, |s| s.alert_storage_pct, |s, v| s.alert_storage_pct = v));
    g.add(&switch(state, "Kernel error alerts", "When the kernel log has at least the Warning threshold of errors", |s| s.alert_kernel, |s, v| s.alert_kernel = v));
    page.add(&g);

    // Thresholds (read-only for now)
    let g = group("Health score thresholds", "The rules used for scoring. Values are [Attention, Warning, Critical]; battery health uses 'below'.");
    let t = state.settings.borrow().thresholds.clone();
    let rows = [
        ("CPU temperature (°C)", t.cpu_temp),
        ("GPU temperature (°C)", t.gpu_temp),
        ("SSD temperature (°C)", t.ssd_temp),
        ("Load per thread", t.load_per_thread),
        ("RAM used (%)", t.mem_used_pct),
        ("Swap used (%)", t.swap_used_pct),
        ("SSD wear (%)", t.ssd_wear_pct),
        ("Filesystem used (%)", t.fs_used_pct),
        ("Battery health below (%)", t.battery_health_pct),
        ("Boot time (s)", t.boot_total_s),
        ("Kernel errors", t.kernel_errors),
    ];
    for (name, v) in rows {
        let r = adw::ActionRow::new();
        r.set_title(name);
        r.add_suffix(&label(&format!("{} / {} / {}", v[0], v[1], v[2]), &["dim-label", "numeric"]));
        g.add(&r);
    }
    page.add(&g);

    let g = group("About", "");
    let about = adw::ActionRow::new();
    about.set_title(&format!("SystemHealthCheck {}", env!("CARGO_PKG_VERSION")));
    about.set_subtitle("Linux Laptop Diagnostic & Performance Center · Rust, GTK4, libadwaita");
    g.add(&about);
    page.add(&g);
    page.upcast()
}
