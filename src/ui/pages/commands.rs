//! Command Center: browse the allowlisted registry by category, inspect the
//! exact command and its metadata, edit validated parameters, run, stop,
//! and search/copy/save the output. Arbitrary shell input is never accepted.

use crate::ui::state::AppState;
use crate::ui::widgets::{self, hbox, label, vbox, KvGrid, RawView};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry::{self, Category, CommandSpec, Danger, ParamKind, Step};
use systemhealthcheck::diagnostics::runner::{self, Params, RunControl};

fn danger_css(d: Danger) -> &'static str {
    match d {
        Danger::ReadOnly => "status-good",
        Danger::PrivilegedRead => "status-attention",
        Danger::ModifySystem => "status-warning",
        Danger::Destructive => "status-critical",
    }
}

struct Selected {
    spec: Option<&'static CommandSpec>,
    params: Rc<RefCell<Params>>,
    ctl: Option<RunControl>,
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let root = hbox(0);

    // ---- left: categories + commands --------------------------------------
    let left = vbox(6);
    left.set_margin_top(12);
    left.set_margin_start(12);
    left.set_margin_bottom(12);
    left.set_size_request(320, -1);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search commands…"));
    let cats: Vec<&str> = std::iter::once("All").chain(Category::ALL.iter().map(|c| c.label())).collect();
    let cat_dd = gtk::DropDown::from_strings(&cats);
    cat_dd.update_property(&[gtk::accessible::Property::Label("Category")]);
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    left.append(&label("Command Center", &["title-2"]));
    left.append(&label(&format!("{} allowlisted commands", registry::REGISTRY.len()), &["dim-label", "caption"]));
    left.append(&cat_dd);
    left.append(&search);
    left.append(&scroll);
    root.append(&left);
    root.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    // ---- right: details + output -------------------------------------------
    let right = vbox(10);
    right.set_hexpand(true);
    right.set_margin_top(12);
    right.set_margin_end(12);
    right.set_margin_start(12);
    right.set_margin_bottom(12);
    let title = label("Select a command", &["title-2"]);
    let badges = hbox(6);
    let desc = label("", &["dim-label"]);
    let meta = KvGrid::new();
    let params_box = hbox(8);
    let warning = label("", &["status-warning"]);
    warning.set_visible(false);
    let actions = hbox(6);
    let run = widgets::pill_button("Execute", true);
    let stop = gtk::Button::with_label("Stop");
    let clear = gtk::Button::with_label("Clear");
    stop.set_sensitive(false);
    run.set_sensitive(false);
    let status = label("", &["caption"]);
    status.set_hexpand(true);
    actions.append(&run);
    actions.append(&stop);
    actions.append(&clear);
    actions.append(&status);
    let out = RawView::new("command-output");
    for w in [title.upcast_ref::<gtk::Widget>(), badges.upcast_ref(), desc.upcast_ref(), meta.grid.upcast_ref(), params_box.upcast_ref(), warning.upcast_ref(), actions.upcast_ref(), out.widget.upcast_ref()] {
        right.append(w);
    }
    root.append(&right);

    let sel = Rc::new(RefCell::new(Selected { spec: None, params: Rc::default(), ctl: None }));
    let shown: Rc<RefCell<Vec<&'static CommandSpec>>> = Rc::default();

    // Populate list.
    let fill = {
        let list = list.clone();
        let shown = shown.clone();
        let search = search.clone();
        let cat_dd = cat_dd.clone();
        move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let q = search.text().to_lowercase();
            let cat = cat_dd.selected() as usize;
            let mut v = Vec::new();
            for spec in registry::REGISTRY {
                if cat > 0 && Category::ALL.get(cat - 1) != Some(&spec.category) {
                    continue;
                }
                if !q.is_empty() && !(spec.name.to_lowercase().contains(&q) || spec.original.to_lowercase().contains(&q) || spec.id.contains(&q)) {
                    continue;
                }
                let b = vbox(2);
                b.set_margin_top(4);
                b.set_margin_bottom(4);
                let top = hbox(6);
                let n = label(spec.name, &[]);
                n.set_hexpand(true);
                top.append(&n);
                if spec.requires_sudo {
                    let lock = gtk::Image::from_icon_name("dialog-password-symbolic");
                    lock.set_tooltip_text(Some("Requires administrator"));
                    top.append(&lock);
                }
                b.append(&top);
                let o = label(spec.original, &["caption", "dim-label", "mono"]);
                o.set_ellipsize(gtk::pango::EllipsizeMode::End);
                o.set_wrap(false);
                b.append(&o);
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&b));
                row.update_property(&[gtk::accessible::Property::Label(spec.name)]);
                list.append(&row);
                v.push(spec);
            }
            *shown.borrow_mut() = v;
        }
    };
    let fill = Rc::new(fill);
    fill();
    let f = fill.clone();
    search.connect_search_changed(move |_| f());
    let f = fill.clone();
    cat_dd.connect_selected_notify(move |_| f());

    // Show details on selection.
    let update_cmd = {
        let meta = meta.clone();
        let sel = sel.clone();
        move || {
            let s = sel.borrow();
            let Some(spec) = s.spec else { return };
            let params = s.params.borrow();
            let exact = runner::display_command(spec, &params);
            meta.set(&widgets::kv([
                ("Checklist command", spec.original.to_string()),
                ("Exact command executed", exact),
                ("Category", spec.category.label().to_string()),
                ("Requires administrator", if spec.requires_sudo { "Yes (pkexec)".into() } else { "No".to_string() }),
                ("Danger level", spec.danger.label().to_string()),
                ("Timeout", format!("{} s", spec.timeout_s)),
                ("Parser", spec.parser.unwrap_or("raw output only").to_string()),
                ("Output type", format!("{:?}", spec.output_type)),
                ("Registry id", spec.id.to_string()),
            ]));
        }
    };
    let update_cmd = Rc::new(update_cmd);

    let st = state.clone();
    let (sel2, shown2, uc) = (sel.clone(), shown.clone(), update_cmd.clone());
    let (title2, badges2, desc2, params2, warning2, run2, out2, status2) =
        (title.clone(), badges.clone(), desc.clone(), params_box.clone(), warning.clone(), run.clone(), out.clone(), status.clone());
    list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let Some(spec) = shown2.borrow().get(row.index() as usize).copied() else { return };
        title2.set_text(spec.name);
        widgets::clear(&badges2);
        badges2.append(&widgets::badge(spec.danger.label(), danger_css(spec.danger)));
        badges2.append(&widgets::badge(spec.category.label(), "status-unknown"));
        if spec.requires_sudo {
            badges2.append(&widgets::badge("Administrator", "status-attention"));
        }
        if !spec.from_checklist {
            badges2.append(&widgets::badge("Internal helper", "status-unknown"));
        }
        desc2.set_text(spec.description);
        warning2.set_visible(spec.warning.is_some());
        warning2.set_text(&spec.warning.map(|w| format!("⚠ {w}")).unwrap_or_default());
        let params = Rc::new(RefCell::new(runner::default_params(spec)));
        widgets::clear(&params2);
        for p in spec.params {
            params2.append(&label(p.label, &["dim-label"]));
            let cur = params.borrow().get(p.key).cloned().unwrap_or_default();
            let key = p.key.to_string();
            let pr = params.clone();
            let uc2 = uc.clone();
            match p.kind {
                ParamKind::Int { min, max } => {
                    let spin = gtk::SpinButton::with_range(min as f64, max as f64, 1.0);
                    spin.set_value(cur.parse().unwrap_or(min as f64));
                    spin.update_property(&[gtk::accessible::Property::Label(p.label)]);
                    spin.connect_value_changed(move |s| {
                        pr.borrow_mut().insert(key.clone(), (s.value() as u32).to_string());
                        uc2();
                    });
                    params2.append(&spin);
                }
                ParamKind::Package => {
                    let dd = gtk::DropDown::from_strings(systemhealthcheck::packages::INSTALLABLE);
                    pr.borrow_mut().insert(key.clone(), systemhealthcheck::packages::INSTALLABLE[0].to_string());
                    dd.connect_selected_notify(move |d| {
                        if let Some(v) = systemhealthcheck::packages::INSTALLABLE.get(d.selected() as usize) {
                            pr.borrow_mut().insert(key.clone(), v.to_string());
                            uc2();
                        }
                    });
                    params2.append(&dd);
                }
                ParamKind::NvmeController | ParamKind::BlockDevice | ParamKind::Unit => {
                    let e = gtk::Entry::new();
                    e.set_text(&cur);
                    e.set_width_chars(14);
                    e.set_tooltip_text(Some("Validated: must be an existing /dev/nvmeX, /dev/sdX, /dev/mmcblkX… device"));
                    e.update_property(&[gtk::accessible::Property::Label(p.label)]);
                    e.connect_changed(move |e| {
                        pr.borrow_mut().insert(key.clone(), e.text().to_string());
                        uc2();
                    });
                    params2.append(&e);
                }
            }
        }
        {
            let mut s = sel2.borrow_mut();
            s.spec = Some(spec);
            s.params = params;
        }
        uc();
        let runnable = !matches!(spec.step, Step::Interactive(_));
        run2.set_sensitive(runnable);
        run2.set_label(if spec.needs_confirmation() { "Execute…" } else { "Execute" });
        out2.set_text(&widgets::format_outputs(&st, &[spec.id]));
        status2.set_text(match spec.step {
            Step::Interactive(_) => "Interactive terminal program — use the native page instead.",
            _ => "",
        });
    });

    // Execute
    let st = state.clone();
    let (sel3, out3, status3, stop3, run3) = (sel.clone(), out.clone(), status.clone(), stop.clone(), run.clone());
    run.connect_clicked(move |_| {
        let (spec, params) = {
            let s = sel3.borrow();
            let Some(spec) = s.spec else { return };
            let params = s.params.borrow().clone();
            (spec, params)
        };
        if let Err(e) = runner::build_argv(spec, &params).or_else(|e| if matches!(spec.step, Step::Run(_)) { Err(e) } else { Ok(vec![]) }) {
            status3.set_text(&format!("Invalid parameter: {e}"));
            return;
        }
        let ctl = RunControl::default();
        sel3.borrow_mut().ctl = Some(ctl.clone());
        out3.set_text(&format!("$ {}\n", runner::display_command(spec, &params)));
        status3.set_text("Running…");
        stop3.set_sensitive(!spec.requires_sudo);
        stop3.set_tooltip_text(spec.requires_sudo.then_some("Privileged commands run as root and cannot be stopped from here; they end at their timeout."));
        run3.set_sensitive(false);
        let o = out3.clone();
        let privacy = st.privacy();
        let on_line: Rc<dyn Fn(bool, &str)> = Rc::new(move |is_err, l: &str| {
            let l = privacy.redact(l);
            o.append(&if is_err { format!("[stderr] {l}\n") } else { format!("{l}\n") });
        });
        let (status4, stop4, run4, out4, st4) = (status3.clone(), stop3.clone(), run3.clone(), out3.clone(), st.clone());
        st.run_spec(spec, params, ctl, Some(on_line), move |res| {
            stop4.set_sensitive(false);
            run4.set_sensitive(true);
            match res {
                None => status4.set_text("Not run (cancelled at confirmation)"),
                Some(r) => {
                    status4.set_text(&format!(
                        "{} · exit {} · {} ms{}",
                        r.status.label(),
                        r.exit_code.map_or("-".into(), |c| c.to_string()),
                        r.duration_ms,
                        if r.detail.is_empty() { String::new() } else { format!(" · {}", r.detail) }
                    ));
                    // Replace streamed text with the final (filtered) result.
                    out4.set_text(&widgets::format_outputs(&st4, &[r.id.as_str()]));
                }
            }
        });
    });
    let sel4 = sel.clone();
    stop.connect_clicked(move |_| {
        if let Some(c) = &sel4.borrow().ctl {
            c.cancel();
        }
    });
    let out5 = out.clone();
    clear.connect_clicked(move |_| out5.set_text(""));
    root.upcast()
}
