//! Main window: sidebar navigation, header bar, page stack, status bar,
//! keyboard shortcuts and the scan progress dialog.

use super::state::{AppState, Event};
use super::widgets::{hbox, label, vbox};
use super::{pages, scan};
use adw::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;
use systemhealthcheck::settings::Theme;
use systemhealthcheck::util;

type Builder = fn(&Rc<AppState>) -> gtk::Widget;

/// (id, title, icon, builder) in sidebar order.
const PAGES: &[(&str, &str, &str, Builder)] = &[
    ("dashboard", "Dashboard", "view-grid-symbolic", pages::dashboard::build),
    ("overview", "Overview", "security-high-symbolic", pages::overview::build),
    ("live", "Live Monitor", "preferences-system-details-symbolic", pages::live::build),
    ("cpu", "CPU", "applications-engineering-symbolic", pages::cpu::build),
    ("memory", "Memory", "media-flash-symbolic", pages::memory::build),
    ("gpu", "GPU", "video-display-symbolic", pages::gpu::build),
    ("storage", "Storage", "drive-harddisk-solidstate-symbolic", pages::storage::build),
    ("thermals", "Thermals", "display-brightness-symbolic", pages::thermals::build),
    ("battery", "Battery", "battery-good-symbolic", pages::battery::build),
    ("processes", "Processes", "view-list-symbolic", pages::processes::build),
    ("network", "Network", "network-wireless-symbolic", pages::network::build),
    ("ports", "Ports", "preferences-system-network-symbolic", pages::ports::build),
    ("boot", "Boot", "system-reboot-symbolic", pages::boot::build),
    ("kernel", "Kernel", "dialog-warning-symbolic", pages::kernel::build),
    ("hardware", "Hardware", "preferences-system-devices-symbolic", pages::hardware::build),
    ("commands", "Commands", "utilities-terminal-symbolic", pages::commands::build),
    ("benchmarks", "Benchmarks", "preferences-system-time-symbolic", pages::benchmarks::build),
    ("stress", "Stress Tests", "power-profile-performance-symbolic", pages::stress::build),
    ("packages", "Packages", "package-x-generic-symbolic", pages::packages::build),
    ("reports", "Reports", "x-office-document-symbolic", pages::reports::build),
    ("history", "History", "document-open-recent-symbolic", pages::history::build),
    ("settings", "Settings", "preferences-system-symbolic", pages::settings::build),
];

pub fn build(app: &adw::Application, state: &Rc<AppState>) -> adw::ApplicationWindow {
    let win = adw::ApplicationWindow::builder().application(app).title("SystemHealthCheck").default_width(1360).default_height(880).build();
    win.set_size_request(380, 480);
    *state.window.borrow_mut() = Some(win.clone());
    apply_theme(state.settings.borrow().theme);

    // -- content area -------------------------------------------------------
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);
    let content_title = adw::WindowTitle::new("Dashboard", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&content_title));

    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh (Ctrl+R)"));
    refresh.set_action_name(Some("win.refresh"));
    refresh.update_property(&[gtk::accessible::Property::Label("Refresh")]);
    let quick = gtk::Button::with_label("Quick Scan");
    quick.set_tooltip_text(Some("Run the checklist's 10 most valuable commands (Ctrl+Shift+S)"));
    quick.set_action_name(Some("win.quick-scan"));
    let full = gtk::Button::with_label("Full Diagnostic");
    full.add_css_class("suggested-action");
    full.set_tooltip_text(Some("Run every appropriate diagnostic from the checklist (Ctrl+Shift+F)"));
    full.set_action_name(Some("win.full-scan"));
    header.pack_start(&refresh);
    header.pack_start(&quick);
    header.pack_start(&full);

    let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_btn.set_tooltip_text(Some("Settings (Ctrl+,)"));
    settings_btn.set_action_name(Some("win.settings"));
    settings_btn.update_property(&[gtk::accessible::Property::Label("Settings")]);
    header.pack_end(&settings_btn);
    header.pack_end(&theme_menu(state));
    let clock = label("", &["dim-label", "numeric"]);
    clock.set_tooltip_text(Some("Current time"));
    header.pack_end(&clock);
    let c2 = clock.clone();
    let tick = move || {
        c2.set_text(&chrono::Local::now().format("%H:%M:%S").to_string());
        glib::ControlFlow::Continue
    };
    tick();
    glib::timeout_add_seconds_local(1, tick);

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&header);
    content_view.set_content(Some(&stack));
    content_view.add_bottom_bar(&status_bar(state));
    let content_page = adw::NavigationPage::new(&content_view, "Dashboard");

    // -- sidebar -----------------------------------------------------------
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    for (_, title, icon, _) in PAGES {
        let row = hbox(12);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.append(&gtk::Image::from_icon_name(icon));
        row.append(&label(title, &[]));
        let lr = gtk::ListBoxRow::new();
        lr.set_child(Some(&row));
        lr.update_property(&[gtk::accessible::Property::Label(title)]);
        list.append(&lr);
    }
    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    side_scroll.set_child(Some(&list));
    side_scroll.set_vexpand(true);
    let side_header = adw::HeaderBar::new();
    side_header.set_title_widget(Some(&adw::WindowTitle::new("SystemHealthCheck", "Diagnostic & Performance Center")));
    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&side_scroll));
    let side_page = adw::NavigationPage::new(&side_view, "SystemHealthCheck");

    let split = adw::NavigationSplitView::new();
    split.set_sidebar(Some(&side_page));
    split.set_content(Some(&content_page));
    split.set_min_sidebar_width(200.0);
    split.set_max_sidebar_width(260.0);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    *state.toasts.borrow_mut() = Some(toasts.clone());
    win.set_content(Some(&toasts));

    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, 820.0, adw::LengthUnit::Sp));
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    win.add_breakpoint(bp);

    // Pages are built lazily on first visit to keep startup fast.
    let built: Rc<std::cell::RefCell<std::collections::HashSet<&'static str>>> = Rc::default();
    // Live Monitor charts should have history from launch, so build it now.
    if let Some(&(id, _, _, builder)) = PAGES.iter().find(|p| p.0 == "live") {
        stack.add_named(&builder(state), Some(id));
        built.borrow_mut().insert(id);
    }
    let show = {
        let stack = stack.clone();
        let st = state.clone();
        let content_title = content_title.clone();
        let content_page = content_page.clone();
        let split = split.clone();
        let list = list.clone();
        move |idx: usize| {
            let Some(&(id, title, _, builder)) = PAGES.get(idx) else { return };
            if built.borrow_mut().insert(id) {
                stack.add_named(&builder(&st), Some(id));
            }
            stack.set_visible_child_name(id);
            content_title.set_title(title);
            content_page.set_title(title);
            split.set_show_content(true);
            if list.selected_row().map(|r| r.index()) != Some(idx as i32) {
                list.select_row(list.row_at_index(idx as i32).as_ref());
            }
        }
    };
    let show = Rc::new(show);
    let s2 = show.clone();
    list.connect_row_activated(move |_, row| s2(row.index() as usize));
    let s3 = show.clone();
    *state.navigate.borrow_mut() = Some(Rc::new(move |id: &str| {
        if let Some(i) = PAGES.iter().position(|p| p.0 == id) {
            s3(i);
        }
    }));

    // Header subtitle: health status.
    let ct = content_title.clone();
    let st = state.clone();
    let update_status = move || {
        let sub = match st.health.borrow().as_ref() {
            Some(h) => match h.overall {
                Some(o) => format!("System health {o}/100 · {}", h.state.label()),
                None => "System health: insufficient data".into(),
            },
            None => "Collecting data…".into(),
        };
        ct.set_subtitle(&sub);
    };
    update_status();
    state.subscribe(move |e| {
        if matches!(e, Event::Health) {
            update_status();
        }
    });

    install_actions(app, &win, state, &show);
    show(0);
    win
}

fn apply_theme(t: Theme) {
    adw::StyleManager::default().set_color_scheme(match t {
        Theme::System => adw::ColorScheme::Default,
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
    });
}

fn theme_menu(state: &Rc<AppState>) -> gtk::MenuButton {
    let pop_box = vbox(4);
    pop_box.set_margin_top(6);
    pop_box.set_margin_bottom(6);
    pop_box.set_margin_start(6);
    pop_box.set_margin_end(6);
    let mut group: Option<gtk::CheckButton> = None;
    for (t, name) in [(Theme::System, "Follow system"), (Theme::Light, "Light"), (Theme::Dark, "Dark")] {
        let b = gtk::CheckButton::with_label(name);
        if let Some(g) = &group {
            b.set_group(Some(g));
        } else {
            group = Some(b.clone());
        }
        b.set_active(state.settings.borrow().theme == t);
        let st = state.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                st.settings.borrow_mut().theme = t;
                apply_theme(t);
                st.save_settings();
            }
        });
        pop_box.append(&b);
    }
    let pop = gtk::Popover::new();
    pop.set_child(Some(&pop_box));
    let mb = gtk::MenuButton::new();
    mb.set_icon_name("weather-clear-night-symbolic");
    mb.set_popover(Some(&pop));
    mb.set_tooltip_text(Some("Theme"));
    mb.update_property(&[gtk::accessible::Property::Label("Theme")]);
    mb
}

fn status_bar(state: &Rc<AppState>) -> gtk::Box {
    let bar = hbox(14);
    bar.add_css_class("statusbar");
    let last = label("", &["caption"]);
    let mon_switch = gtk::Switch::new();
    mon_switch.set_valign(gtk::Align::Center);
    mon_switch.set_tooltip_text(Some("Start/stop live monitoring (Ctrl+M)"));
    mon_switch.update_property(&[gtk::accessible::Property::Label("Live monitoring")]);
    let mon_label = label("", &["caption"]);
    let privilege = label(
        if util::is_root() { "Privileges: running as root" } else { "Privileges: user (pkexec asks when needed)" },
        &["caption", "dim-label"],
    );
    let spinner = adw::Spinner::new();
    let running = label("", &["caption"]);
    running.set_ellipsize(gtk::pango::EllipsizeMode::End);
    running.set_hexpand(true);
    running.set_wrap(false);
    let errors = gtk::Button::new();
    errors.add_css_class("flat");
    errors.add_css_class("caption");
    errors.set_tooltip_text(Some("Commands that failed or were unavailable — open the Commands page"));
    let st = state.clone();
    errors.connect_clicked(move |_| st.go("commands"));

    for l in [&last, &mon_label, &privilege] {
        l.set_wrap(false);
    }
    let mon_box = hbox(6);
    mon_box.append(&mon_switch);
    mon_box.append(&mon_label);
    bar.append(&last);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&mon_box);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&privilege);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    bar.append(&spinner);
    bar.append(&running);
    bar.append(&errors);

    let st = state.clone();
    mon_switch.connect_state_set(move |_, on| {
        if on {
            st.start_monitor();
        } else {
            st.stop_monitor();
        }
        glib::Propagation::Proceed
    });
    let st = state.clone();
    let refresh = move || {
        last.set_text(&st.last_scan.get().map_or("Last scan: never".into(), |t| format!("Last scan: {}", util::fmt_time(t))));
        let on = st.monitoring.load(std::sync::atomic::Ordering::SeqCst);
        if mon_switch.is_active() != on {
            mon_switch.set_active(on);
        }
        mon_label.set_text(&if on { format!("Monitoring every {}", fmt_interval(st.settings.borrow().refresh_ms)) } else { "Monitoring stopped".into() });
        let busy = st.busy_labels();
        spinner.set_visible(!busy.is_empty());
        running.set_text(&if busy.is_empty() { "Idle".into() } else { format!("Running: {}", busy.join(", ")) });
        let res = st.results.borrow();
        let failed = res.values().filter(|o| !o.ok()).count();
        errors.set_label(&format!("{failed} unavailable / failed"));
        errors.set_visible(failed > 0);
        if let Some(e) = &st.db_error {
            errors.set_label(e);
            errors.set_visible(true);
        }
    };
    refresh();
    state.subscribe(move |e| {
        if matches!(e, Event::Busy | Event::Results(_) | Event::Settings | Event::History) {
            refresh();
        }
    });
    bar
}

pub fn fmt_interval(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{} s", ms / 1000)
    }
}

fn install_actions(app: &adw::Application, win: &adw::ApplicationWindow, state: &Rc<AppState>, show: &Rc<impl Fn(usize) + 'static>) {
    let add = |name: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        win.add_action(&a);
    };
    let st = state.clone();
    add(
        "refresh",
        Box::new(move || {
            if !st.monitoring.load(std::sync::atomic::Ordering::SeqCst) {
                st.sample_once();
            }
            let ids: Vec<String> = st.settings.borrow().auto_ids.clone();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            st.run_ids(&ids, "Refresh");
        }),
    );
    let st = state.clone();
    add("quick-scan", Box::new(move || scan::quick_scan(&st)));
    let st = state.clone();
    add("full-scan", Box::new(move || scan::full_diagnostic(&st)));
    let st = state.clone();
    add("settings", Box::new(move || st.go("settings")));
    let st = state.clone();
    add(
        "toggle-monitor",
        Box::new(move || {
            if st.monitoring.load(std::sync::atomic::Ordering::SeqCst) {
                st.stop_monitor();
            } else {
                st.start_monitor();
            }
        }),
    );
    let w = win.clone();
    add("shortcuts", Box::new(move || shortcuts_dialog(&w)));
    for i in 0..9usize {
        let s = show.clone();
        add(&format!("page-{}", i + 1), Box::new(move || s(i)));
    }
    let quit = gio::SimpleAction::new("quit", None);
    let a2 = app.clone();
    quit.connect_activate(move |_, _| a2.quit());
    app.add_action(&quit);
    let toast = gio::SimpleAction::new("toast", Some(glib::VariantTy::STRING));
    let st = state.clone();
    toast.connect_activate(move |_, v| {
        if let Some(s) = v.and_then(|v| v.get::<String>()) {
            st.toast(&s);
        }
    });
    app.add_action(&toast);

    app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
    app.set_accels_for_action("win.quick-scan", &["<Control><Shift>s"]);
    app.set_accels_for_action("win.full-scan", &["<Control><Shift>f"]);
    app.set_accels_for_action("win.settings", &["<Control>comma"]);
    app.set_accels_for_action("win.toggle-monitor", &["<Control>m"]);
    app.set_accels_for_action("win.shortcuts", &["F1", "<Control>question"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    for i in 1..=9 {
        app.set_accels_for_action(&format!("win.page-{i}"), &[&format!("<Alt>{i}")]);
    }
}

fn shortcuts_dialog(win: &adw::ApplicationWindow) {
    let body = "Ctrl+R / F5 — Refresh\nCtrl+Shift+S — Quick Scan\nCtrl+Shift+F — Full Diagnostic\nCtrl+M — Start/stop live monitoring\nCtrl+, — Settings\nAlt+1…9 — Jump to the first nine pages\nCtrl+Q — Quit\nF1 — This help\n\nTab / Shift+Tab move between controls; arrow keys move in the sidebar and tables; Enter activates.";
    let d = adw::AlertDialog::new(Some("Keyboard Shortcuts"), Some(body));
    d.add_response("close", "Close");
    d.present(Some(win));
}
