//! CPU benchmark (sysbench) with configurable threads/duration, progress,
//! saved results and comparison with previous runs.

use crate::ui::state::AppState;
use crate::ui::table::{col, num, DataTable};
use crate::ui::widgets::{self, hbox, kv, label, Card, KvGrid, Page, RawView};
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};
use systemhealthcheck::parsers::bench::parse_sysbench;
use systemhealthcheck::util::{cpu_threads, fmt_time, now};

thread_local! {
    static START: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Used by Full Diagnostic when benchmarks are enabled: opens the page and
/// starts a run with the current selection (the confirmation still appears).
pub fn run_default(state: &Rc<AppState>) {
    state.go("benchmarks");
    START.with(|s| {
        if let Some(f) = s.borrow().clone() {
            f();
        }
    });
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Benchmarks", "sysbench CPU benchmark — runs in the background, never blocks the UI", &["sysbench-cpu"]);
    let threads_max = cpu_threads();

    let cfg = Card::new("Configuration", "preferences-system-symbolic");
    let row = hbox(8);
    let choices = ["1 thread", "2 threads", "4 threads", "All threads", "Custom"];
    let dd = gtk::DropDown::from_strings(&choices);
    dd.set_selected(3);
    dd.update_property(&[gtk::accessible::Property::Label("Threads")]);
    let custom = gtk::SpinButton::with_range(1.0, 1024.0, 1.0);
    custom.set_value(threads_max as f64);
    custom.set_sensitive(false);
    custom.update_property(&[gtk::accessible::Property::Label("Custom thread count")]);
    let secs = gtk::SpinButton::with_range(5.0, 600.0, 5.0);
    secs.set_value(30.0);
    secs.update_property(&[gtk::accessible::Property::Label("Duration in seconds")]);
    let run = widgets::pill_button("Run benchmark…", true);
    let stop = gtk::Button::with_label("Stop");
    stop.set_sensitive(false);
    row.append(&label("Threads", &["dim-label"]));
    row.append(&dd);
    row.append(&custom);
    row.append(&label("Duration (s)", &["dim-label"]));
    row.append(&secs);
    row.append(&run);
    row.append(&stop);
    cfg.body.append(&row);
    cfg.body.append(&label(&format!("This CPU has {threads_max} logical threads. Checklist reference: sysbench cpu --threads=8 --time=30 run"), &["caption", "dim-label"]));
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_visible(false);
    cfg.body.append(&progress);
    page.overview.append(&cfg.root);

    let grid = widgets::card_grid(1, 2);
    let result = Card::new("Latest result", "emblem-ok-symbolic");
    let eps = label("—", &["big-value"]);
    let res_kv = KvGrid::new();
    result.body.append(&eps);
    result.body.append(&res_kv.grid);
    let live_out = RawView::new("sysbench");
    let live = Card::new("Output", "utilities-terminal-symbolic");
    live_out.widget.set_size_request(-1, 220);
    live.body.append(&live_out.widget);
    grid.append(&result.root);
    grid.append(&live.root);
    page.overview.append(&grid);

    page.overview.append(&widgets::section("History"));
    let hist = DataTable::new(&[col("When"), num("Threads"), num("Duration"), num("Events/s"), num("Total events"), num("vs previous")], false, 0, 240);
    page.overview.append(&hist.widget);

    let c2 = custom.clone();
    dd.connect_selected_notify(move |d| c2.set_sensitive(d.selected() == 4));

    let st = state.clone();
    let refresh_hist = Rc::new(move || {
        let Some(db) = &st.db else { return };
        let rows = db.list_bench("sysbench").unwrap_or_default();
        let eps_of = |r: &systemhealthcheck::database::BenchRow| r.summary["events_per_sec"].as_f64();
        hist.set_rows(
            rows.iter()
                .enumerate()
                .map(|(i, r)| {
                    let delta = match (eps_of(r), rows.get(i + 1).and_then(eps_of)) {
                        (Some(a), Some(b)) if b > 0.0 => format!("{:+.1}%", (a - b) / b * 100.0),
                        _ => "—".into(),
                    };
                    vec![
                        fmt_time(r.taken_at),
                        r.summary["threads"].to_string(),
                        format!("{} s", r.summary["seconds"]),
                        eps_of(r).map_or("—".into(), |v| format!("{v:.2}")),
                        r.summary["total_events"].to_string(),
                        delta,
                    ]
                })
                .collect(),
        );
    });
    refresh_hist();

    let ctl_cell: Rc<RefCell<Option<RunControl>>> = Rc::default();
    let st = state.clone();
    let (dd2, custom2, secs2, stop2, run2, prog2, out2, eps2, rh, cc) =
        (dd.clone(), custom.clone(), secs.clone(), stop.clone(), run.clone(), progress.clone(), live_out.clone(), eps.clone(), refresh_hist.clone(), ctl_cell.clone());
    let start: Rc<dyn Fn()> = Rc::new(move || {
        let Some(spec) = registry::get("sysbench-cpu") else { return };
        let threads = match dd2.selected() {
            0 => 1,
            1 => 2,
            2 => 4,
            4 => custom2.value() as usize,
            _ => threads_max,
        };
        let seconds = secs2.value() as u64;
        let params = Params::from([("threads".to_string(), threads.to_string()), ("secs".to_string(), seconds.to_string())]);
        let ctl = RunControl::default();
        *cc.borrow_mut() = Some(ctl.clone());
        out2.set_text("");
        let o = out2.clone();
        let on_line: Rc<dyn Fn(bool, &str)> = Rc::new(move |_, l: &str| o.append(&format!("{l}\n")));
        let (stop3, run3, prog3, eps3, rh3, st3) = (stop2.clone(), run2.clone(), prog2.clone(), eps2.clone(), rh.clone(), st.clone());
        let res_kv = res_kv.clone();
        // Progress timer runs only once the command actually starts.
        let started = Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
        let s2 = started.clone();
        let p4 = prog2.clone();
        let timer = gtk::glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            if let Some(t) = s2.get() {
                let f = (t.elapsed().as_secs_f64() / seconds as f64).min(1.0);
                p4.set_fraction(f);
                p4.set_text(Some(&format!("{:.0} / {seconds} s", t.elapsed().as_secs_f64().min(seconds as f64))));
            }
            gtk::glib::ControlFlow::Continue
        });
        let timer = Rc::new(std::cell::Cell::new(Some(timer)));
        let on_line2 = {
            let started = started.clone();
            let (stop5, run5, prog5) = (stop2.clone(), run2.clone(), prog2.clone());
            let inner = on_line.clone();
            Rc::new(move |e: bool, l: &str| {
                if started.get().is_none() {
                    started.set(Some(std::time::Instant::now()));
                    stop5.set_sensitive(true);
                    run5.set_sensitive(false);
                    prog5.set_visible(true);
                }
                inner(e, l)
            }) as Rc<dyn Fn(bool, &str)>
        };
        let cpu_model = st.sample().map(|s| s.cpu.model.clone()).unwrap_or_default();
        st.run_spec(spec, params, ctl, Some(on_line2), move |res| {
            if let Some(t) = timer.take() {
                t.remove();
            }
            stop3.set_sensitive(false);
            run3.set_sensitive(true);
            prog3.set_visible(false);
            let Some(r) = res else { return };
            match parse_sysbench(&r.stdout).filter(|_| r.ok()) {
                Some(b) => {
                    eps3.set_text(&format!("{:.2} events/s", b.events_per_sec.unwrap_or(0.0)));
                    res_kv.set(&kv([
                        ("Events per second", widgets::or_na(b.events_per_sec, "")),
                        ("Total events", widgets::or_na(b.total_events, "")),
                        ("Execution time", widgets::or_na(b.total_time_s, " s")),
                        ("Threads", threads.to_string()),
                        ("Latency avg / p95 / max", format!("{} / {} / {} ms", widgets::or_na(b.latency_avg_ms, ""), widgets::or_na(b.latency_p95_ms, ""), widgets::or_na(b.latency_max_ms, ""))),
                        ("CPU", cpu_model.clone()),
                        ("Timestamp", fmt_time(now())),
                    ]));
                    if let Some(db) = &st3.db {
                        let summary = serde_json::json!({
                            "events_per_sec": b.events_per_sec, "total_events": b.total_events, "seconds": seconds,
                            "threads": threads, "cpu": cpu_model, "latency_avg_ms": b.latency_avg_ms,
                        });
                        let _ = db.save_bench(now(), "sysbench", &summary, &serde_json::json!({ "stdout": r.stdout, "command": r.command }));
                    }
                    rh3();
                    st3.toast("Benchmark complete and saved");
                }
                None => {
                    eps3.set_text(r.status.label());
                    res_kv.set(&kv([("Status", format!("{} — {}", r.status.label(), r.detail)), ("stderr", r.stderr.clone())]));
                }
            }
        });
    });
    START.with(|s| *s.borrow_mut() = Some(start.clone()));
    run.connect_clicked(move |_| start());
    stop.connect_clicked(move |_| {
        if let Some(c) = ctl_cell.borrow().as_ref() {
            c.cancel();
        }
    });
    page.root.upcast()
}
