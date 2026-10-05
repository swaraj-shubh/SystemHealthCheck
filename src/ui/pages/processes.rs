//! Process monitor: native /proc table (the htop replacement) with sorting,
//! details, open location and confirmed termination.

use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, label, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::procs;
use systemhealthcheck::util::fmt_bytes;

const IDS: &[&str] = &["ps-cpu-15", "ps-mem-15", "ps-mem-20", "htop"];

fn details(pid: u32) -> String {
    let base = format!("/proc/{pid}");
    let status = std::fs::read_to_string(format!("{base}/status")).unwrap_or_else(|e| format!("status unavailable: {e}"));
    let cmd = std::fs::read(format!("{base}/cmdline")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
    let exe = std::fs::read_link(format!("{base}/exe")).map(|p| p.display().to_string()).unwrap_or_else(|_| "unavailable (permission)".into());
    let cwd = std::fs::read_link(format!("{base}/cwd")).map(|p| p.display().to_string()).unwrap_or_else(|_| "unavailable (permission)".into());
    format!("Command line: {cmd}\nExecutable: {exe}\nWorking directory: {cwd}\n\n{status}")
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Processes", "Running processes from /proc (native replacement for ps and htop)", IDS);
    let bar = hbox(6);
    let top_cpu = gtk::Button::with_label("Top CPU");
    let top_mem = gtk::Button::with_label("Top memory");
    let info = gtk::Button::with_label("Details");
    let open = gtk::Button::with_label("Open location");
    let term = gtk::Button::with_label("Terminate…");
    term.add_css_class("destructive-action");
    let pause = gtk::ToggleButton::with_label("Pause updates");
    for b in [&top_cpu, &top_mem, &info, &open] {
        bar.append(b);
    }
    bar.append(&pause);
    bar.append(&term);
    page.overview.append(&bar);
    let summary = label("", &["dim-label"]);
    page.overview.append(&summary);
    let table = DataTable::new(
        &[num("PID"), col("Process"), col("User"), num("CPU %"), num("Memory %"), num("Memory"), col("State"), num("Threads"), wide("Command")],
        true,
        0,
        520,
    );
    table.sort_by(3, true);
    page.overview.append(&table.widget);

    let t = table.clone();
    top_cpu.connect_clicked(move |_| t.sort_by(3, true));
    let t = table.clone();
    top_mem.connect_clicked(move |_| t.sort_by(5, true));

    let t = table.clone();
    let p2 = pause.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        if !t.widget.is_mapped() || p2.is_active() || s.processes.is_empty() {
            return;
        }
        let total_cpu: f64 = s.processes.iter().map(|p| p.cpu_pct).sum();
        summary.set_text(&format!("{} processes · {:.0}% CPU total (100% = one core) · refreshes with the live monitor", s.processes.len(), total_cpu));
        t.set_rows(
            s.processes
                .iter()
                .map(|p| {
                    vec![
                        p.pid.to_string(),
                        p.name.clone(),
                        p.user.clone(),
                        format!("{:.1}", p.cpu_pct),
                        format!("{:.1}", p.mem_pct),
                        fmt_bytes(p.rss_bytes),
                        p.state_label().into(),
                        p.threads.to_string(),
                        p.cmdline.clone(),
                    ]
                })
                .collect(),
        );
    });

    let selected_pid = {
        let t = table.clone();
        let st = state.clone();
        move || -> Option<(u32, String)> {
            let r = t.selected();
            if r.is_none() {
                st.toast("Select a process first");
            }
            r.and_then(|r| Some((r.first()?.parse().ok()?, r.get(1).cloned().unwrap_or_default())))
        }
    };
    let selected_pid = Rc::new(selected_pid);

    let sp = selected_pid.clone();
    let st = state.clone();
    let show_details = Rc::new(move || {
        let Some((pid, name)) = sp() else { return };
        let d = adw::AlertDialog::new(Some(&format!("{name} (PID {pid})")), None);
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_monospace(true);
        view.buffer().set_text(&details(pid));
        let sc = gtk::ScrolledWindow::new();
        sc.set_child(Some(&view));
        sc.set_min_content_height(360);
        sc.set_min_content_width(560);
        d.set_extra_child(Some(&sc));
        d.add_response("close", "Close");
        let win = st.window.borrow().clone();
        d.present(win.as_ref());
    });
    let sd = show_details.clone();
    info.connect_clicked(move |_| sd());
    table.connect_activate(move |_| show_details());

    let sp = selected_pid.clone();
    let st = state.clone();
    open.connect_clicked(move |_| {
        let Some((pid, _)) = sp() else { return };
        match std::fs::read_link(format!("/proc/{pid}/exe")) {
            Ok(p) => widgets::open_path(p.parent().unwrap_or(&p)),
            Err(e) => st.toast(&format!("Cannot read executable path: {e}")),
        }
    });

    let sp = selected_pid.clone();
    let st = state.clone();
    term.connect_clicked(move |_| {
        let Some((pid, name)) = sp() else { return };
        let d = adw::AlertDialog::new(
            Some(&format!("Terminate {name}?")),
            Some(&format!("PID {pid} will receive SIGTERM and may lose unsaved work. Choose “Force kill” (SIGKILL) only if it does not exit.")),
        );
        d.add_response("cancel", "Cancel");
        d.add_response("kill", "Force kill");
        d.add_response("term", "Terminate");
        d.set_response_appearance("term", adw::ResponseAppearance::Destructive);
        d.set_response_appearance("kill", adw::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        let st2 = st.clone();
        d.connect_response(None, move |_, r| {
            if r == "term" || r == "kill" {
                match procs::signal(pid, r == "kill") {
                    Ok(()) => st2.toast(&format!("Signal sent to {name} ({pid})")),
                    Err(e) => st2.toast(&format!("Could not signal {pid}: {e}")),
                }
            }
        });
        let win = st.window.borrow().clone();
        d.present(win.as_ref());
    });
    page.root.upcast()
}
