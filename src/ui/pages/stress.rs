//! CPU stress test (stress-ng) with an explicit warning, live telemetry,
//! automatic emergency stop at a configured temperature and saved results.

use crate::ui::chart::Chart;
use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, DataTable};
use crate::ui::widgets::{self, hbox, kv, label, Card, KvGrid, Page};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};
use systemhealthcheck::parsers::bench::parse_stress_ng;
use systemhealthcheck::util::{cpu_threads, fmt_duration, fmt_time, now};

thread_local! {
    static START: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

pub fn start_default(state: &Rc<AppState>) {
    state.go("stress");
    START.with(|s| {
        if let Some(f) = s.borrow().clone() {
            f();
        }
    });
}

#[derive(Default)]
struct Telemetry {
    running: bool,
    started: Option<std::time::Instant>,
    peak: Option<f64>,
    temps: Vec<f64>,
    emergency: bool,
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Stress Tests", "stress-ng CPU load test with thermal safety stop", &["stress-ng-cpu"]);
    let banner = adw::Banner::new("This test intentionally places heavy load on the CPU. Temperature may increase significantly. Stop the test if abnormal temperatures or behavior occur.");
    banner.set_revealed(true);
    page.root.insert_child_after(&banner, Some(&page.updated));

    let cfg = Card::new("Configuration", "preferences-system-symbolic");
    let row = hbox(8);
    let workers = gtk::SpinButton::with_range(1.0, 1024.0, 1.0);
    workers.set_value(cpu_threads() as f64);
    workers.update_property(&[gtk::accessible::Property::Label("CPU workers")]);
    let secs = gtk::SpinButton::with_range(10.0, 3600.0, 10.0);
    secs.set_value(60.0);
    secs.update_property(&[gtk::accessible::Property::Label("Duration in seconds")]);
    let emerg = gtk::SpinButton::with_range(60.0, 110.0, 1.0);
    emerg.set_value(state.settings.borrow().emergency_stop_c);
    emerg.set_tooltip_text(Some("The test is stopped automatically when any CPU sensor reaches this temperature"));
    emerg.update_property(&[gtk::accessible::Property::Label("Emergency stop temperature")]);
    let start_btn = widgets::pill_button("Start stress test…", true);
    start_btn.add_css_class("destructive-action");
    let stop_btn = gtk::Button::with_label("Stop now");
    stop_btn.set_sensitive(false);
    for (t, w) in [("Workers", workers.upcast_ref::<gtk::Widget>()), ("Duration (s)", secs.upcast_ref()), ("Emergency stop (°C)", emerg.upcast_ref())] {
        row.append(&label(t, &["dim-label"]));
        row.append(w);
    }
    row.append(&start_btn);
    row.append(&stop_btn);
    cfg.body.append(&row);
    cfg.body.append(&label(&format!("{} logical CPUs detected. Checklist reference: stress-ng --cpu 4 --timeout 60s --metrics-brief", cpu_threads()), &["caption", "dim-label"]));
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    cfg.body.append(&progress);
    page.overview.append(&cfg.root);

    let grid = widgets::card_grid(1, 2);
    let live = Card::new("During the test", "utilities-system-monitor-symbolic");
    let live_kv = KvGrid::new();
    live.body.append(&live_kv.grid);
    let chart = Chart::new(&["CPU %", "Temp °C"], Some(110.0), "", 140);
    live.body.append(&chart.area);
    let result = Card::new("Result", "emblem-ok-symbolic");
    let res_kv = KvGrid::new();
    result.body.append(&res_kv.grid);
    grid.append(&live.root);
    grid.append(&result.root);
    page.overview.append(&grid);
    page.overview.append(&widgets::section("History"));
    let hist = DataTable::new(&[col("When"), num("Workers"), num("Duration"), num("Bogo ops/s"), num("Peak °C"), col("Status")], false, 0, 200);
    page.overview.append(&hist.widget);

    let st = state.clone();
    let refresh_hist = Rc::new(move || {
        let Some(db) = &st.db else { return };
        hist.set_rows(
            db.list_bench("stress")
                .unwrap_or_default()
                .iter()
                .map(|r| {
                    let s = &r.summary;
                    vec![
                        fmt_time(r.taken_at),
                        s["workers"].to_string(),
                        format!("{:.0} s", s["elapsed_s"].as_f64().unwrap_or(0.0)),
                        s["bogo_ops_per_s"].as_f64().map_or("—".into(), |v| format!("{v:.1}")),
                        s["peak_temp_c"].as_f64().map_or("—".into(), |v| format!("{v:.1}")),
                        s["status"].as_str().unwrap_or("").to_string(),
                    ]
                })
                .collect(),
        );
    });
    refresh_hist();

    let tel = Rc::new(RefCell::new(Telemetry::default()));
    let ctl_cell: Rc<RefCell<Option<RunControl>>> = Rc::default();
    let duration = Rc::new(Cell::new(60u64));

    // Live telemetry + emergency stop.
    let (t2, cc2, dur2, prog2, st2) = (tel.clone(), ctl_cell.clone(), duration.clone(), progress.clone(), state.clone());
    let emerg2 = emerg.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        let mut t = t2.borrow_mut();
        if !t.running {
            return;
        }
        let temp = s.max_temp(SensorKind::Cpu);
        if let Some(v) = temp {
            t.temps.push(v);
            t.peak = Some(t.peak.map_or(v, |p| p.max(v)));
            if v >= emerg2.value() && !t.emergency {
                t.emergency = true;
                if let Some(c) = cc2.borrow().as_ref() {
                    c.cancel();
                }
                st2.toast(&format!("Emergency stop: CPU reached {v:.0} °C"));
            }
        }
        chart.push(&[s.cpu.total_usage, temp]);
        let elapsed = t.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
        prog2.set_fraction((elapsed / dur2.get() as f64).min(1.0));
        prog2.set_text(Some(&format!("{} / {}", fmt_duration(elapsed), fmt_duration(dur2.get() as f64))));
        live_kv.set(&kv([
            ("CPU usage", widgets::or_na(s.cpu.total_usage.map(|u| format!("{u:.0}")), "%")),
            ("CPU frequency", widgets::or_na(s.cpu.avg_mhz().map(|m| format!("{m:.0}")), " MHz")),
            ("CPU temperature", widgets::or_na(temp.map(|v| format!("{v:.1}")), " °C")),
            ("Peak temperature", widgets::or_na(t.peak.map(|v| format!("{v:.1}")), " °C")),
            ("Elapsed", fmt_duration(elapsed)),
            ("Duration", fmt_duration(dur2.get() as f64)),
        ]));
    });

    let ctl_for_stop = ctl_cell.clone();
    stop_btn.connect_clicked(move |_| {
        if let Some(c) = ctl_for_stop.borrow().as_ref() {
            c.cancel();
        }
    });
    let st = state.clone();
    let (w2, s2, start2, stop2, rh) = (workers.clone(), secs.clone(), start_btn.clone(), stop_btn.clone(), refresh_hist.clone());
    let start: Rc<dyn Fn()> = Rc::new(move || {
        let Some(spec) = registry::get("stress-ng-cpu") else { return };
        let (nw, ns) = (w2.value() as u32, s2.value() as u64);
        duration.set(ns);
        if (emerg.value() - st.settings.borrow().emergency_stop_c).abs() > f64::EPSILON {
            st.settings.borrow_mut().emergency_stop_c = emerg.value();
            st.save_settings();
        }
        let params = Params::from([("workers".to_string(), nw.to_string()), ("secs".to_string(), ns.to_string())]);
        let ctl = RunControl::default();
        *ctl_cell.borrow_mut() = Some(ctl.clone());
        // Telemetry needs the live monitor; start it if it was stopped.
        let was_monitoring = st.monitoring.load(std::sync::atomic::Ordering::SeqCst);
        let (tel2, start3, stop3, rh2, st3, res_kv2) = (tel.clone(), start2.clone(), stop2.clone(), rh.clone(), st.clone(), res_kv.clone());
        let tel_on_start = tel.clone();
        let (st_on, start_on, stop_on) = (st.clone(), start2.clone(), stop2.clone());
        let on_line: Rc<dyn Fn(bool, &str)> = Rc::new(move |_, _| {
            let mut t = tel_on_start.borrow_mut();
            if !t.running && t.started.is_none() {
                *t = Telemetry { running: true, started: Some(std::time::Instant::now()), ..Default::default() };
                start_on.set_sensitive(false);
                stop_on.set_sensitive(true);
                if !st_on.monitoring.load(std::sync::atomic::Ordering::SeqCst) {
                    st_on.start_monitor();
                }
            }
        });
        *tel.borrow_mut() = Telemetry::default();
        st.run_spec(spec, params, ctl, Some(on_line), move |res| {
            let t = std::mem::take(&mut *tel2.borrow_mut());
            start3.set_sensitive(true);
            stop3.set_sensitive(false);
            if !was_monitoring && t.started.is_some() {
                st3.stop_monitor();
            }
            let Some(r) = res else { return };
            let parsed = parse_stress_ng(&format!("{}\n{}", r.stdout, r.stderr));
            let elapsed = t.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
            let avg = (!t.temps.is_empty()).then(|| t.temps.iter().sum::<f64>() / t.temps.len() as f64);
            let status = if t.emergency {
                "Stopped: emergency temperature reached".to_string()
            } else if parsed.completed {
                "Completed successfully".into()
            } else {
                format!("{} — {}", r.status.label(), r.detail)
            };
            let thermal = match t.peak {
                Some(p) if p >= 95.0 => "Peak temperature is very high — check cooling (fans, vents, thermal paste)",
                Some(p) if p >= 85.0 => "High peak temperature — the CPU may have throttled",
                Some(_) => "No thermal warnings",
                None => "CPU temperature not exposed — thermal behavior unknown",
            };
            let m = parsed.metrics.first();
            let mut rows = kv([
                ("Status", status.clone()),
                ("Duration", fmt_duration(elapsed)),
                ("Workers", nw.to_string()),
                ("Peak temperature", widgets::or_na(t.peak.map(|v| format!("{v:.1}")), " °C")),
                ("Average temperature", widgets::or_na(avg.map(|v| format!("{v:.1}")), " °C")),
                ("Thermal assessment", thermal.into()),
                ("Passed / failed", format!("{} / {}", parsed.passed.clone().unwrap_or_default(), parsed.failed.clone().unwrap_or_default())),
            ]);
            for mm in &parsed.metrics {
                rows.push((
                    format!("{} metrics", mm.stressor),
                    format!("{} bogo ops · {:.2} ops/s real · {:.2} ops/s usr+sys · real {:.2}s usr {:.2}s sys {:.2}s", mm.bogo_ops, mm.bogo_ops_per_s_real, mm.bogo_ops_per_s_cpu, mm.real_time_s, mm.usr_time_s, mm.sys_time_s),
                ));
            }
            res_kv2.set(&rows);
            if let Some(db) = &st3.db {
                let summary = serde_json::json!({
                    "workers": nw, "seconds": ns, "elapsed_s": elapsed, "bogo_ops_per_s": m.map(|m| m.bogo_ops_per_s_real),
                    "peak_temp_c": t.peak, "avg_temp_c": avg, "status": status, "emergency_stop": t.emergency,
                });
                let _ = db.save_bench(now(), "stress", &summary, &serde_json::json!({ "stdout": r.stdout, "stderr": r.stderr }));
            }
            rh2();
            st3.toast(&format!("Stress test: {status}"));
        });
    });
    START.with(|s| *s.borrow_mut() = Some(start.clone()));
    start_btn.connect_clicked(move |_| start());
    page.root.upcast()
}
