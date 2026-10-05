//! Dependencies / tools: installed vs missing diagnostic packages with
//! confirmed installation, APT updates, and a review-first package cleanup.

use crate::ui::state::AppState;
use crate::ui::table::{col, wide, DataTable};
use crate::ui::widgets::{self, hbox, label, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{self, Params, RunControl};
use systemhealthcheck::packages::DEPENDENCIES;

const IDS: &[&str] = &["apt-policy", "apt-upgradable", "apt-autoremove-preview", "apt-update", "apt-autoremove"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Packages", "Diagnostic tools, updates and package cleanup — nothing is installed or removed without your confirmation", IDS);

    page.overview.append(&widgets::section("Diagnostic tools"));
    let deps = gtk::ListBox::new();
    deps.add_css_class("boxed-list");
    deps.set_selection_mode(gtk::SelectionMode::None);
    page.overview.append(&deps);

    page.overview.append(&widgets::section("Updates"));
    let upd_bar = hbox(8);
    let apt_update = gtk::Button::with_label("Refresh package lists (apt update)…");
    let list_upg = gtk::Button::with_label("List upgradable packages");
    upd_bar.append(&apt_update);
    upd_bar.append(&list_upg);
    page.overview.append(&upd_bar);
    let upg_count = label("", &["dim-label"]);
    page.overview.append(&upg_count);
    let upg = DataTable::new(&[wide("Package"), col("New version"), col("Installed version")], true, 0, 220);
    page.overview.append(&upg.widget);

    page.overview.append(&widgets::section("Package cleanup"));
    page.overview.append(&label(
        "The checklist warns against running `sudo apt autoremove` blindly. Review the packages it would remove first; the removal button only becomes available after a review.",
        &["dim-label"],
    ));
    let clean_bar = hbox(8);
    let review = gtk::Button::with_label("Review removable packages");
    let execute = gtk::Button::with_label("Execute autoremove…");
    execute.add_css_class("destructive-action");
    execute.set_sensitive(false);
    clean_bar.append(&review);
    clean_bar.append(&execute);
    page.overview.append(&clean_bar);
    let rm_count = label("", &[]);
    page.overview.append(&rm_count);
    let rm = DataTable::new(&[wide("Package"), col("Version")], true, 0, 220);
    page.overview.append(&rm.widget);
    let output = widgets::RawView::new("apt-output");
    output.widget.set_visible(false);
    page.overview.append(&output.widget);

    let st = state.clone();
    let ex = execute.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        while let Some(c) = deps.first_child() {
            deps.remove(&c);
        }
        let installed = st.installed.borrow().clone();
        let p = st.parsed();
        for d in DEPENDENCIES {
            let row = adw::ActionRow::new();
            row.set_title(d.package);
            let has = installed.contains(d.package);
            let cand = p.policy.iter().find(|e| e.package == d.package).and_then(|e| e.candidate.clone());
            let inst_ver = p.policy.iter().find(|e| e.package == d.package).and_then(|e| e.installed.clone());
            row.set_subtitle(&gtk::glib::markup_escape_text(&format!(
                "{} · provides `{}`{}",
                d.purpose,
                d.binary,
                match (&inst_ver, &cand) {
                    (Some(i), _) => format!(" · {i}"),
                    (None, Some(c)) => format!(" · available: {c}"),
                    _ => String::new(),
                }
            )));
            row.add_suffix(&widgets::badge(if has { "✓ Installed" } else { "✗ Missing" }, if has { "status-good" } else { "status-warning" }));
            if !has {
                let b = gtk::Button::with_label("Install…");
                b.set_valign(gtk::Align::Center);
                let spec_cmd = registry::get(d.install_id).map(|s| s.original).unwrap_or("");
                b.set_tooltip_text(Some(spec_cmd));
                let st2 = st.clone();
                b.connect_clicked(move |_| st2.install_dep(d));
                row.add_suffix(&b);
            }
            deps.append(&row);
        }
        upg.set_rows(p.upgradable.iter().map(|(a, b, c)| vec![a.clone(), b.clone(), c.clone()]).collect());
        upg_count.set_text(&match st.result("apt-upgradable") {
            Some(o) if o.ok() => format!("{} packages can be upgraded (from the last `apt update`).", p.upgradable.len()),
            Some(o) => format!("{}: {}", o.status.label(), o.detail),
            None => "Not checked yet.".into(),
        });
        rm.set_rows(p.autoremove.iter().map(|(a, b)| vec![a.clone(), b.clone()]).collect());
        match st.result("apt-autoremove-preview") {
            Some(o) if o.ok() => {
                rm_count.set_text(&format!("{} packages would be removed by autoremove.", p.autoremove.len()));
                ex.set_sensitive(!p.autoremove.is_empty());
            }
            Some(o) => rm_count.set_text(&format!("{}: {}", o.status.label(), o.detail)),
            None => rm_count.set_text("Not reviewed yet."),
        }
    });

    let st = state.clone();
    list_upg.connect_clicked(move |_| st.run_ids(&["apt-upgradable"], "Checking upgradable packages"));
    let st = state.clone();
    review.connect_clicked(move |_| st.run_ids(&["apt-autoremove-preview", "apt-policy"], "Reviewing autoremove"));

    let stream_run = {
        let st = state.clone();
        let out = output.clone();
        move |id: &'static str, extra: String| {
            let Some(spec) = registry::get(id) else { return };
            out.widget.set_visible(true);
            out.set_text(&format!("$ {}\n{extra}", runner::display_command(spec, &Params::new())));
            let o = out.clone();
            let st2 = st.clone();
            st.run_spec(spec, Params::new(), RunControl::default(), Some(Rc::new(move |_, l: &str| o.append(&format!("{l}\n")))), move |r| {
                if let Some(r) = r {
                    st2.toast(&format!("{}: {}", spec.name, r.status.label()));
                    st2.run_ids(&["apt-policy", "apt-upgradable", "apt-autoremove-preview"], "Refreshing package status");
                }
            });
        }
    };
    let stream_run = Rc::new(stream_run);
    let sr = stream_run.clone();
    apt_update.connect_clicked(move |_| sr("apt-update", String::new()));
    let st = state.clone();
    execute.connect_clicked(move |_| {
        let pkgs = st.parsed().autoremove.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
        *st.confirm_note.borrow_mut() = Some(format!("Packages that will be removed ({}):\n{}", pkgs.len(), pkgs.join(", ")));
        stream_run("apt-autoremove", format!("# Reviewed list ({}): {}\n", pkgs.len(), pkgs.join(" ")));
    });
    page.root.upcast()
}
