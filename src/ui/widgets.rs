//! Reusable widgets: cards, badges, key/value grids, raw output viewer,
//! issue rows (Unavailable / Permission required / Tool not installed) and
//! the standard page scaffold with [Overview] [Raw Output] tabs.

use super::state::{AppState, Event};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{CmdOutput, RunStatus};
use systemhealthcheck::scoring::State;
use systemhealthcheck::util::fmt_time;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    // Caps natural width so card grids can fit several columns; wrapped
    // labels still use all the width they are given.
    l.set_max_width_chars(32);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    b.add_css_class("flat");
    b
}

pub fn pill_button(text: &str, suggested: bool) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    if suggested {
        b.add_css_class("suggested-action");
    }
    b.add_css_class("pill");
    b
}

/// Status badge: colored pill with text (never color alone).
pub fn badge(text: &str, css: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("badge");
    l.add_css_class(css);
    l.set_valign(gtk::Align::Center);
    l
}

pub fn set_badge(l: &gtk::Label, text: &str, css: &str) {
    for c in ["status-good", "status-attention", "status-warning", "status-critical", "status-unknown"] {
        l.remove_css_class(c);
    }
    l.add_css_class(css);
    l.set_text(text);
}

pub fn state_badge(s: State) -> gtk::Label {
    badge(s.label(), s.css())
}

/// CSS class for a percentage where higher is worse.
pub fn level_css(pct: f64, attention: f64, warning: f64, critical: f64) -> &'static str {
    if pct >= critical {
        "status-critical"
    } else if pct >= warning {
        "status-warning"
    } else if pct >= attention {
        "status-attention"
    } else {
        "status-good"
    }
}

/// Usage bar with a status color class.
pub fn usage_bar() -> gtk::ProgressBar {
    let p = gtk::ProgressBar::new();
    p.add_css_class("usage");
    p
}

pub fn set_usage(p: &gtk::ProgressBar, pct: f64) {
    p.set_fraction((pct / 100.0).clamp(0.0, 1.0));
    for c in ["status-good", "status-attention", "status-warning", "status-critical"] {
        p.remove_css_class(c);
    }
    p.add_css_class(level_css(pct, 75.0, 90.0, 97.0));
    p.update_property(&[gtk::accessible::Property::Label(&format!("{pct:.0} percent"))]);
}

/// A card: header (icon, title, optional extra widgets) + body box.
pub struct Card {
    pub root: gtk::Box,
    pub header: gtk::Box,
    pub body: gtk::Box,
}

impl Card {
    pub fn new(title: &str, icon: &str) -> Card {
        let root = vbox(8);
        root.add_css_class("card");
        root.add_css_class("sp-card");
        let header = hbox(8);
        let img = gtk::Image::from_icon_name(icon);
        img.add_css_class("dim-label");
        let t = label(title, &["heading"]);
        t.set_hexpand(true);
        header.append(&img);
        header.append(&t);
        let body = vbox(4);
        root.append(&header);
        root.append(&body);
        Card { root, header, body }
    }
}

/// Two-column key/value grid that can be refreshed in place.
#[derive(Clone)]
pub struct KvGrid {
    pub grid: gtk::Grid,
}

impl KvGrid {
    pub fn new() -> KvGrid {
        let grid = gtk::Grid::new();
        grid.set_column_spacing(16);
        grid.set_row_spacing(4);
        KvGrid { grid }
    }

    pub fn set(&self, rows: &[(String, String)]) {
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        for (i, (k, v)) in rows.iter().enumerate() {
            let kl = label(k, &["dim-label"]);
            kl.set_valign(gtk::Align::Start);
            let vl = label(if v.is_empty() { "—" } else { v }, &[]);
            vl.set_selectable(true);
            vl.set_hexpand(true);
            self.grid.attach(&kl, 0, i as i32, 1, 1);
            self.grid.attach(&vl, 1, i as i32, 1, 1);
        }
        if rows.is_empty() {
            self.grid.attach(&label("No data yet", &["dim-label"]), 0, 0, 2, 1);
        }
    }
}

pub fn kv<S: Into<String>, T: Into<String>>(pairs: impl IntoIterator<Item = (S, T)>) -> Vec<(String, String)> {
    pairs.into_iter().map(|(a, b)| (a.into(), b.into())).collect()
}

pub fn or_na<T: std::fmt::Display>(v: Option<T>, unit: &str) -> String {
    v.map_or_else(|| "Unavailable".into(), |v| format!("{v}{unit}"))
}

// ---------------------------------------------------------------------------
// Raw output
// ---------------------------------------------------------------------------

/// Monospace output viewer with search, copy and save.
pub struct RawView {
    pub widget: gtk::Box,
    pub buffer: gtk::TextBuffer,
    view: gtk::TextView,
    name: RefCell<String>,
}

impl RawView {
    pub fn new(name: &str) -> Rc<RawView> {
        let buffer = gtk::TextBuffer::new(None);
        buffer.create_tag(Some("match"), &[("background", &"#f6d32d"), ("foreground", &"#000000")]);
        let view = gtk::TextView::with_buffer(&buffer);
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_left_margin(10);
        view.set_right_margin(10);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.update_property(&[gtk::accessible::Property::Label("Raw command output")]);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&view));
        scroll.set_vexpand(true);
        scroll.set_min_content_height(240);
        scroll.add_css_class("card");

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search output…"));
        search.set_hexpand(true);
        let copy = icon_button("edit-copy-symbolic", "Copy output");
        let save = icon_button("document-save-symbolic", "Save output to file");
        let bar = hbox(6);
        bar.append(&search);
        bar.append(&copy);
        bar.append(&save);
        let widget = vbox(6);
        widget.append(&bar);
        widget.append(&scroll);
        let rv = Rc::new(RawView { widget, buffer: buffer.clone(), view: view.clone(), name: RefCell::new(name.to_string()) });

        let b = buffer.clone();
        let v = view.clone();
        search.connect_search_changed(move |e| highlight(&b, &v, &e.text(), false));
        let b = buffer.clone();
        let v = view.clone();
        search.connect_activate(move |e| highlight(&b, &v, &e.text(), true));
        let b = buffer.clone();
        copy.connect_clicked(move |btn| {
            btn.clipboard().set_text(&b.text(&b.start_iter(), &b.end_iter(), false));
        });
        let rv2 = rv.clone();
        save.connect_clicked(move |btn| rv2.save(btn));
        rv
    }

    pub fn set_text(&self, text: &str) {
        self.buffer.set_text(text);
    }

    pub fn append(&self, text: &str) {
        let mut end = self.buffer.end_iter();
        self.buffer.insert(&mut end, text);
        let mark = self.buffer.create_mark(None, &self.buffer.end_iter(), false);
        self.view.scroll_mark_onscreen(&mark);
        self.buffer.delete_mark(&mark);
    }

    pub fn text(&self) -> String {
        self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false).to_string()
    }

    fn save(&self, w: &gtk::Button) {
        let text = self.text();
        let name = format!("{}.txt", self.name.borrow());
        save_text_dialog(w, &name, text);
    }
}

fn highlight(b: &gtk::TextBuffer, v: &gtk::TextView, q: &str, next: bool) {
    b.remove_tag_by_name("match", &b.start_iter(), &b.end_iter());
    if q.is_empty() {
        return;
    }
    let mut it = b.start_iter();
    let mut first = None;
    while let Some((s, e)) = it.forward_search(q, gtk::TextSearchFlags::CASE_INSENSITIVE, None) {
        b.apply_tag_by_name("match", &s, &e);
        first.get_or_insert(s);
        it = e;
    }
    // Enter jumps to the next match after the cursor.
    let target = if next {
        let cur = b.iter_at_mark(&b.get_insert());
        let mut after = cur;
        after.forward_char();
        after.forward_search(q, gtk::TextSearchFlags::CASE_INSENSITIVE, None).map(|(s, _)| s).or(first)
    } else {
        first
    };
    if let Some(mut s) = target {
        b.place_cursor(&s);
        v.scroll_to_iter(&mut s, 0.1, false, 0.0, 0.0);
    }
}

/// Save text through the portal-friendly GtkFileDialog.
pub fn save_text_dialog(w: &impl IsA<gtk::Widget>, name: &str, text: String) {
    let dialog = gtk::FileDialog::builder().title("Save output").initial_name(name).modal(true).build();
    let win = w.root().and_downcast::<gtk::Window>();
    let w = w.as_ref().clone();
    glib::spawn_future_local(async move {
        if let Ok(file) = dialog.save_future(win.as_ref()).await {
            if let Some(path) = file.path() {
                let msg = match std::fs::write(&path, text) {
                    Ok(()) => format!("Saved to {}", path.display()),
                    Err(e) => format!("Could not save: {e}"),
                };
                let _ = w.activate_action("app.toast", Some(&msg.to_variant()));
            }
        }
    });
}

/// Render results for display: command line, status, exit code, time, output.
pub fn format_outputs(state: &AppState, ids: &[&str]) -> String {
    let privacy = state.privacy();
    let mut s = String::new();
    for id in ids {
        let spec = registry::get(id);
        match state.result(id) {
            Some(o) => {
                s += &format!(
                    "$ {}\n# {} · {} · exit {} · {} ms · {}\n",
                    o.command,
                    spec.map_or(*id, |x| x.name),
                    o.status.label(),
                    o.exit_code.map_or("-".into(), |c| c.to_string()),
                    o.duration_ms,
                    fmt_time(o.finished_at)
                );
                if let Some(sp) = spec {
                    if sp.original != o.command {
                        s += &format!("# checklist: {}\n", sp.original);
                    }
                }
                if !o.detail.is_empty() {
                    s += &format!("# {}\n", o.detail);
                }
                s += &privacy.redact(&o.stdout);
                if !o.stderr.trim().is_empty() {
                    s += &format!("\n[stderr]\n{}", privacy.redact(&o.stderr));
                }
                s += "\n\n";
            }
            None => s += &format!("$ {}\n# not run yet\n\n", spec.map_or(*id, |x| x.original)),
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Issues (unavailable / permission / not installed)
// ---------------------------------------------------------------------------

/// Rows explaining why commands failed, with Install / Retry actions.
pub fn issues_box(state: &Rc<AppState>, ids: &'static [&'static str]) -> gtk::Box {
    let root = vbox(6);
    let st = state.clone();
    let r = root.clone();
    let refresh = move || {
        while let Some(c) = r.first_child() {
            r.remove(&c);
        }
        let mut seen_tools = std::collections::HashSet::new();
        for id in ids {
            let Some(o) = st.result(id) else { continue };
            if o.ok() {
                continue;
            }
            if let Some(t) = &o.missing_tool {
                if !seen_tools.insert(t.clone()) {
                    continue;
                }
            }
            r.append(&issue_row(&st, &o));
        }
        r.set_visible(r.first_child().is_some());
    };
    refresh();
    state.subscribe(move |e| {
        if let Event::Results(changed) = e {
            if changed.iter().any(|c| ids.contains(&c.as_str())) {
                refresh();
            }
        }
    });
    root
}

fn issue_row(state: &Rc<AppState>, o: &CmdOutput) -> gtk::Widget {
    let spec = registry::get(&o.id);
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(&format!("{} — {}", spec.map_or(o.id.as_str(), |s| s.name), o.status.label())));
    let detail = if o.detail.is_empty() { o.stderr.lines().next().unwrap_or("").to_string() } else { o.detail.clone() };
    row.set_subtitle(&glib::markup_escape_text(&detail));
    let icon = match o.status {
        RunStatus::NotInstalled => "package-x-generic-symbolic",
        RunStatus::PermissionDenied | RunStatus::AuthCancelled => "dialog-password-symbolic",
        RunStatus::Unavailable => "action-unavailable-symbolic",
        _ => "dialog-warning-symbolic",
    };
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    match (o.status, &o.missing_tool) {
        (RunStatus::NotInstalled, Some(tool)) => {
            if systemhealthcheck::packages::for_binary(tool).is_some() {
                let b = gtk::Button::with_label("Install…");
                b.set_valign(gtk::Align::Center);
                let st = state.clone();
                let t = tool.clone();
                b.connect_clicked(move |_| st.install_for(&t));
                row.add_suffix(&b);
            }
        }
        (RunStatus::Unavailable | RunStatus::NotRunnable, _) => {}
        _ => {
            if let Some(spec) = spec {
                let b = gtk::Button::with_label("Retry");
                b.set_valign(gtk::Align::Center);
                let st = state.clone();
                let id = spec.id;
                b.connect_clicked(move |_| st.run_ids(&[id], "Retrying"));
                row.add_suffix(&b);
            }
        }
    }
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.append(&row);
    list.upcast()
}

// ---------------------------------------------------------------------------
// Page scaffold
// ---------------------------------------------------------------------------

/// Standard page: title, actions, issues, [Overview] [Raw Output] tabs.
pub struct Page {
    pub root: gtk::Box,
    pub overview: gtk::Box,
    pub actions: gtk::Box,
    pub updated: gtk::Label,
    pub raw: Rc<RawView>,
}

impl Page {
    /// `ids`: registry commands shown in the raw tab and run by "Run diagnostics".
    pub fn new(state: &Rc<AppState>, title: &str, subtitle: &str, ids: &'static [&'static str]) -> Page {
        let root = vbox(12);
        root.set_margin_top(18);
        root.set_margin_bottom(18);
        root.set_margin_start(18);
        root.set_margin_end(18);

        let head = hbox(12);
        let titles = vbox(2);
        titles.append(&label(title, &["title-1"]));
        titles.append(&label(subtitle, &["dim-label"]));
        titles.set_hexpand(true);
        head.append(&titles);
        let actions = hbox(6);
        actions.set_valign(gtk::Align::Center);
        head.append(&actions);
        root.append(&head);

        let updated = label("", &["caption", "dim-label"]);
        root.append(&updated);

        if !ids.is_empty() {
            let run = pill_button("Run diagnostics", true);
            run.set_tooltip_text(Some(&format!(
                "Runs: {}",
                ids.iter().filter_map(|i| registry::get(i)).filter(|s| !s.is_manual_only()).map(|s| s.original).collect::<Vec<_>>().join("; ")
            )));
            let st = state.clone();
            let t = title.to_string();
            run.connect_clicked(move |_| {
                let runnable: Vec<&str> = ids.iter().copied().filter(|i| registry::get(i).is_some_and(|s| !s.is_manual_only())).collect();
                st.run_ids(&runnable, &format!("{t} diagnostics"));
            });
            actions.append(&run);
            root.append(&issues_box(state, ids));
        }

        let stack = adw::ViewStack::new();
        let overview = vbox(12);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(1400);
        clamp.set_child(Some(&overview));
        scroll.set_child(Some(&clamp));
        scroll.set_vexpand(true);
        stack.add_titled_with_icon(&scroll, Some("overview"), "Overview", "view-grid-symbolic");
        let raw = RawView::new(&title.to_lowercase().replace(' ', "-"));
        stack.add_titled_with_icon(&raw.widget, Some("raw"), "Raw Output", "utilities-terminal-symbolic");
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        switcher.set_halign(gtk::Align::Start);
        if !ids.is_empty() {
            root.append(&switcher);
        }
        root.append(&stack);

        let page = Page { root, overview, actions, updated, raw };
        if !ids.is_empty() {
            let raw = page.raw.clone();
            let upd = page.updated.clone();
            let st = state.clone();
            let refresh = move || {
                raw.set_text(&format_outputs(&st, ids));
                let last = ids.iter().filter_map(|i| st.result(i)).map(|o| o.finished_at).max();
                upd.set_text(&last.map_or("Diagnostics not run yet".into(), |t| format!("Last updated {}", fmt_time(t))));
            };
            refresh();
            state.subscribe(move |e| match e {
                Event::Results(changed) if changed.iter().any(|c| ids.contains(&c.as_str())) => refresh(),
                Event::Settings => refresh(),
                _ => {}
            });
        }
        page
    }
}

/// Wrap a widget in a FlowBox-friendly card grid.
pub fn card_grid(min_per_line: u32, max_per_line: u32) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_min_children_per_line(min_per_line);
    f.set_max_children_per_line(max_per_line);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f.set_valign(gtk::Align::Start);
    f
}

/// Section heading inside a page.
pub fn section(title: &str) -> gtk::Label {
    let l = label(title, &["title-4"]);
    l.set_margin_top(6);
    l
}

/// Empty state with an icon and explanation.
pub fn empty_state(icon: &str, title: &str, desc: &str) -> adw::StatusPage {
    let s = adw::StatusPage::new();
    s.set_icon_name(Some(icon));
    s.set_title(title);
    s.set_description(Some(desc));
    s.add_css_class("compact");
    s
}

pub fn open_path(path: &std::path::Path) {
    let uri = gio::File::for_path(path).uri();
    let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
}

/// Call `refresh` now, whenever `w` becomes visible, and on matching events
/// while it is visible (hidden pages do no work).
pub fn bind(state: &Rc<AppState>, w: &impl IsA<gtk::Widget>, filter: impl Fn(&Event) -> bool + 'static, refresh: impl Fn() + 'static) {
    let r = Rc::new(refresh);
    r();
    let r2 = r.clone();
    w.as_ref().connect_map(move |_| r2());
    let weak = w.as_ref().downgrade();
    state.subscribe(move |e| {
        if filter(e) && weak.upgrade().is_some_and(|w| w.is_mapped()) {
            r();
        }
    });
}

pub fn on_data(e: &Event) -> bool {
    matches!(e, Event::Results(_) | Event::Health | Event::Settings)
}

/// Remove every child of a box.
pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}
