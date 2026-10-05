//! Open ports: every listening/bound TCP/UDP socket with its process and
//! systemd service, plus confirmed actions to stop the process or service.

use crate::ui::state::AppState;
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, label, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::procs;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};
use systemhealthcheck::parsers::hardware::{parse_ss, Port};

const IDS: &[&str] = &["ss-listening", "ss-listening-root"];

/// Latest successful ss result (the administrator run sees all owners).
fn ports(state: &AppState) -> Vec<Port> {
    IDS.iter()
        .filter_map(|i| state.result(i))
        .filter(|o| o.ok())
        .max_by_key(|o| o.finished_at)
        .map(|o| parse_ss(&o.stdout))
        .unwrap_or_default()
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Ports", "Listening network ports, the processes behind them, and controls to shut them down", IDS);
    let refresh = gtk::Button::with_label("Refresh");
    let all = gtk::Button::with_label("Show all owners (administrator)");
    all.set_tooltip_text(Some("Runs `sudo ss -tulpn` so ports owned by root services show their process"));
    page.actions.prepend(&all);
    page.actions.prepend(&refresh);

    let summary = label("", &["dim-label"]);
    page.overview.append(&summary);
    let bar = hbox(6);
    let stop_proc = gtk::Button::with_label("Stop process…");
    let stop_svc = gtk::Button::with_label("Stop service…");
    let disable_svc = gtk::Button::with_label("Stop & disable service…");
    for b in [&stop_proc, &stop_svc, &disable_svc] {
        b.add_css_class("destructive-action");
        bar.append(b);
    }
    let only_exposed = gtk::CheckButton::with_label("Only ports reachable from the network");
    bar.append(&only_exposed);
    page.overview.append(&bar);
    let table = DataTable::new(
        &[col("Proto"), col("State"), col("Address"), num("Port"), col("Exposure"), wide("Process"), num("PID"), col("User"), wide("Service")],
        true,
        0,
        440,
    );
    table.sort_by(3, false);
    page.overview.append(&table.widget);
    page.overview.append(&label(
        "Stopping a process that belongs to a systemd service usually gets it restarted — stop the service instead. \"Stop & disable\" also keeps it from starting at boot. Socket-activated services may also need their .socket unit stopped. Every action asks for confirmation.",
        &["caption", "dim-label"],
    ));

    let st = state.clone();
    let t = table.clone();
    let oe = only_exposed.clone();
    let fill = Rc::new(move || {
        let list = ports(&st);
        let users: std::collections::HashMap<u32, String> = std::fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let c: Vec<&str> = l.split(':').collect();
                Some((c.get(2)?.parse().ok()?, c.first()?.to_string()))
            })
            .collect();
        let exposed = list.iter().filter(|p| p.exposed()).count();
        let unknown = list.iter().filter(|p| p.owners.is_empty()).count();
        summary.set_text(&format!(
            "{} sockets · {exposed} reachable from the network{}",
            list.len(),
            if unknown > 0 { format!(" · {unknown} with unknown owner (click “Show all owners”)") } else { String::new() }
        ));
        let rows = list
            .iter()
            .filter(|p| !oe.is_active() || p.exposed())
            .map(|p| {
                let (name, pid) = p.owners.first().cloned().map_or((String::from("unknown (needs administrator)"), String::new()), |(n, pid)| (n, pid.to_string()));
                let pidn: Option<u32> = pid.parse().ok();
                let user = pidn.and_then(procs::owner_uid).map(|u| users.get(&u).cloned().unwrap_or_else(|| u.to_string())).unwrap_or_default();
                let unit = pidn.and_then(procs::systemd_unit).unwrap_or_default();
                vec![
                    p.proto.clone(),
                    p.state.clone(),
                    p.address.clone(),
                    p.port.to_string(),
                    if p.exposed() { "Network".into() } else { "Local only".into() },
                    name,
                    pid,
                    user,
                    unit,
                ]
            })
            .collect();
        t.set_rows(rows);
    });
    let f = fill.clone();
    only_exposed.connect_toggled(move |_| f());
    let f = fill.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || f());
    // Re-scan when the page is shown (one cheap `ss` call, never on a timer).
    let st = state.clone();
    page.overview.connect_map(move |_| st.run_ids(&["ss-listening"], "Listing ports"));
    let st = state.clone();
    refresh.connect_clicked(move |_| st.run_ids(&["ss-listening"], "Listing ports"));
    let st = state.clone();
    all.connect_clicked(move |_| st.run_ids(&["ss-listening-root"], "Listing ports (administrator)"));

    let selected = {
        let t = table.clone();
        let st = state.clone();
        Rc::new(move || {
            let r = t.selected();
            if r.is_none() {
                st.toast("Select a port first");
            }
            r
        })
    };

    // Stop process: own processes are signalled directly, others via pkexec.
    let (st, sel) = (state.clone(), selected.clone());
    stop_proc.connect_clicked(move |_| {
        let Some(r) = sel() else { return };
        let Ok(pid) = r[6].parse::<u32>() else {
            st.toast("Owner unknown — click “Show all owners (administrator)” first");
            return;
        };
        if !r[8].is_empty() {
            *st.confirm_note.borrow_mut() = Some(format!("Note: this process belongs to {} and may be restarted by systemd.", r[8]));
        }
        let own = procs::owner_uid(pid) == Some(unsafe { libc::getuid() });
        let what = format!("{} (PID {pid}) listening on {} port {}", r[5], r[0], r[3]);
        if own {
            let d = adw::AlertDialog::new(Some("Stop process?"), Some(&format!("{what} will receive SIGTERM and may lose unsaved work.")));
            d.add_response("cancel", "Cancel");
            d.add_response("stop", "Stop process");
            d.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
            d.set_close_response("cancel");
            let st2 = st.clone();
            d.connect_response(None, move |_, resp| {
                if resp == "stop" {
                    match procs::signal(pid, false) {
                        Ok(()) => st2.toast(&format!("Sent SIGTERM to PID {pid}")),
                        Err(e) => st2.toast(&format!("Could not stop PID {pid}: {e}")),
                    }
                    st2.confirm_note.borrow_mut().take();
                    let st3 = st2.clone();
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || st3.run_ids(&["ss-listening"], "Listing ports"));
                }
            });
            let win = st.window.borrow().clone();
            d.present(win.as_ref());
        } else if let Some(spec) = registry::get("kill-pid") {
            let st2 = st.clone();
            st.run_spec(spec, Params::from([("pid".to_string(), pid.to_string())]), RunControl::default(), None, move |o| {
                if let Some(o) = o {
                    st2.toast(&format!("Stop {what}: {}", o.status.label()));
                    st2.run_ids(&["ss-listening-root"], "Listing ports (administrator)");
                }
            });
        }
    });

    for (btn, id) in [(&stop_svc, "systemctl-stop"), (&disable_svc, "systemctl-disable")] {
        let (st, sel) = (state.clone(), selected.clone());
        btn.connect_clicked(move |_| {
            let Some(r) = sel() else { return };
            if r[8].is_empty() {
                st.toast("This port is not owned by a systemd system service — use “Stop process…”");
                return;
            }
            let Some(spec) = registry::get(id) else { return };
            *st.confirm_note.borrow_mut() = Some(format!("Port: {} {}:{} ({})", r[0], r[2], r[3], r[5]));
            let st2 = st.clone();
            let unit = r[8].clone();
            st.run_spec(spec, Params::from([("unit".to_string(), unit.clone())]), RunControl::default(), None, move |o| {
                if let Some(o) = o {
                    st2.toast(&format!("{} {unit}: {}", spec.name, if o.ok() { "done".to_string() } else { format!("{} — {}", o.status.label(), o.detail) }));
                    st2.run_ids(&["ss-listening"], "Listing ports");
                }
            });
        });
    }
    page.root.upcast()
}
