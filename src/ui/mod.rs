//! GTK4 + libadwaita front-end. Contains no shell logic: everything runs
//! through the `systemhealthcheck` library (registry, runner, collectors).

mod chart;
mod pages;
mod scan;
mod state;
mod table;
mod widgets;
mod window;

use adw::prelude::*;
use gtk::glib;

pub const APP_ID: &str = "io.github.systemhealthcheck.SystemHealthCheck";

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let state = state::AppState::new();
        let win = window::build(app, &state);
        win.present();
        startup(&state);
    });
    // Ignore GTK's own argument parsing of our flags.
    app.run_with_args(&std::env::args().take(1).collect::<Vec<_>>())
}

/// Startup behavior from settings: monitoring, automatic checks, first-run
/// dependency check, optional quick scan.
fn startup(state: &std::rc::Rc<state::AppState>) {
    let s = state.settings.borrow().clone();
    if s.start_monitoring {
        state.start_monitor();
    } else {
        state.sample_once();
    }
    let ids: Vec<&str> = s.auto_ids.iter().map(String::as_str).collect();
    state.run_ids(&ids, "Automatic checks");
    if !s.first_run_done {
        state.settings.borrow_mut().first_run_done = true;
        state.save_settings();
        let installed = state.installed.borrow();
        let missing = systemhealthcheck::packages::DEPENDENCIES.iter().filter(|d| !installed.contains(d.package)).count();
        if missing > 0 {
            state.toast(&format!("{missing} diagnostic tools are missing — see Packages"));
            drop(installed);
            state.go("packages");
        }
    }
    if s.quick_scan_on_start {
        scan::quick_scan(state);
    }
    if s.compact {
        if let Some(w) = state.window.borrow().as_ref() {
            w.add_css_class("compact-mode");
        }
    }
}
