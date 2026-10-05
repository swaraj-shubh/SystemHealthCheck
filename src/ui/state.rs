//! Shared UI state: latest results, live samples, settings, an event bus for
//! pages, and helpers that run work off the GTK main thread.

use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use systemhealthcheck::collectors::{system::SystemInfo, Monitor, Sample};
use systemhealthcheck::database::Db;
use systemhealthcheck::diagnostics::registry::{self, CommandSpec};
use systemhealthcheck::diagnostics::runner::{self, CmdOutput, Params, RunControl};
use systemhealthcheck::diagnostics::scheduler::{self, ScanEvent, ScanKind, ScanPlan};
use systemhealthcheck::diagnostics::{snapshot, Parsed, Snapshot};
use systemhealthcheck::privacy::Privacy;
use systemhealthcheck::scoring::{self, HealthReport};
use systemhealthcheck::settings::Settings;
use systemhealthcheck::util;

/// Events pages subscribe to.
#[derive(Clone)]
pub enum Event {
    Sample(Rc<Sample>),
    /// Registry ids whose results changed.
    Results(Rc<Vec<String>>),
    Settings,
    Busy,
    Health,
    /// A scan was saved to history.
    History,
}

type Listener = Rc<dyn Fn(&Event)>;

pub struct AppState {
    pub db: Option<Db>,
    pub db_error: Option<String>,
    pub settings: RefCell<Settings>,
    pub system: SystemInfo,
    pub results: RefCell<BTreeMap<String, CmdOutput>>,
    pub parsed: RefCell<Rc<Parsed>>,
    pub sample: RefCell<Option<Rc<Sample>>>,
    pub health: RefCell<Option<Rc<HealthReport>>>,
    pub reveal_serial: Cell<bool>,
    pub last_scan: Cell<Option<i64>>,
    pub installed: RefCell<HashSet<String>>,
    busy: RefCell<Vec<(u64, String)>>,
    next_op: Cell<u64>,
    listeners: RefCell<Vec<Listener>>,
    pub monitoring: Arc<AtomicBool>,
    monitor_gen: Arc<AtomicU64>,
    pub interval_ms: Arc<AtomicU64>,
    pub window: RefCell<Option<adw::ApplicationWindow>>,
    pub toasts: RefCell<Option<adw::ToastOverlay>>,
    pub navigate: RefCell<Option<Rc<dyn Fn(&str)>>>,
    alert_last: RefCell<BTreeMap<&'static str, i64>>,
    /// Extra text for the next confirmation dialog (e.g. reviewed package list).
    pub confirm_note: RefCell<Option<String>>,
    health_tick: Cell<u32>,
}

impl AppState {
    pub fn new() -> Rc<Self> {
        let path = systemhealthcheck::database::default_path();
        let (db, db_error) = match Db::open(&path) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(format!("History database unavailable ({}): {e}", path.display()))),
        };
        let settings = db.as_ref().map(|d| d.load_settings()).unwrap_or_default();
        let interval = settings.refresh_ms;
        let st = Rc::new(AppState {
            db,
            db_error,
            settings: RefCell::new(settings),
            system: systemhealthcheck::collectors::system::read_system(),
            results: RefCell::new(BTreeMap::new()),
            parsed: RefCell::new(Rc::new(Parsed::default())),
            sample: RefCell::new(None),
            health: RefCell::new(None),
            reveal_serial: Cell::new(false),
            last_scan: Cell::new(None),
            installed: RefCell::new(systemhealthcheck::packages::installed_packages()),
            busy: RefCell::new(Vec::new()),
            next_op: Cell::new(1),
            listeners: RefCell::new(Vec::new()),
            monitoring: Arc::new(AtomicBool::new(false)),
            monitor_gen: Arc::new(AtomicU64::new(0)),
            interval_ms: Arc::new(AtomicU64::new(interval)),
            window: RefCell::new(None),
            toasts: RefCell::new(None),
            navigate: RefCell::new(None),
            alert_last: RefCell::new(BTreeMap::new()),
            confirm_note: RefCell::new(None),
            health_tick: Cell::new(0),
        });
        // Restore the most recent scan so privileged data is visible with its timestamp.
        if let Some(db) = &st.db {
            let retention = st.settings.borrow().history_retention_days;
            let _ = db.prune(retention, util::now());
            if let Some(row) = db.list_scans().ok().and_then(|l| l.into_iter().next()) {
                if let Ok((snap, _)) = db.load_scan(row.id) {
                    st.last_scan.set(Some(snap.taken_at));
                    *st.results.borrow_mut() = snap.results;
                    st.reparse();
                }
            }
        }
        st
    }

    // -- event bus ----------------------------------------------------------

    pub fn subscribe(&self, f: impl Fn(&Event) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn emit(&self, e: Event) {
        let ls: Vec<Listener> = self.listeners.borrow().clone();
        for l in ls {
            l(&e);
        }
    }

    // -- accessors --------------------------------------------------------

    pub fn result(&self, id: &str) -> Option<CmdOutput> {
        self.results.borrow().get(id).cloned()
    }

    pub fn parsed(&self) -> Rc<Parsed> {
        self.parsed.borrow().clone()
    }

    pub fn sample(&self) -> Option<Rc<Sample>> {
        self.sample.borrow().clone()
    }

    /// Privacy filter for on-screen raw output.
    pub fn privacy(&self) -> Privacy {
        let s = self.settings.borrow();
        Privacy {
            mask_serial: s.mask_serial && !self.reveal_serial.get(),
            hostname: s.mask_hostname.then(|| self.system.hostname.clone()),
            mask_network: false,
            secrets: self.parsed().secrets(),
        }
    }

    pub fn save_settings(&self) {
        if let Some(db) = &self.db {
            if let Err(e) = db.save_settings(&self.settings.borrow()) {
                self.toast(&format!("Could not save settings: {e}"));
            }
        }
        self.interval_ms.store(self.settings.borrow().refresh_ms, Ordering::SeqCst);
        self.emit(Event::Settings);
    }

    pub fn toast(&self, msg: &str) {
        if let Some(t) = self.toasts.borrow().as_ref() {
            let toast = adw::Toast::new(msg);
            toast.set_timeout(4);
            t.add_toast(toast);
        }
    }

    pub fn go(&self, page: &str) {
        let nav = self.navigate.borrow().clone();
        if let Some(n) = nav {
            n(page);
        }
    }

    // -- busy tracking ----------------------------------------------------

    pub fn begin(&self, label: &str) -> u64 {
        let id = self.next_op.get();
        self.next_op.set(id + 1);
        self.busy.borrow_mut().push((id, label.to_string()));
        self.emit(Event::Busy);
        id
    }

    pub fn end(&self, id: u64) {
        self.busy.borrow_mut().retain(|(i, _)| *i != id);
        self.emit(Event::Busy);
    }

    pub fn busy_labels(&self) -> Vec<String> {
        self.busy.borrow().iter().map(|(_, l)| l.clone()).collect()
    }

    // -- results & health ---------------------------------------------------

    fn reparse(&self) {
        let p = snapshot::parse_all(&self.results.borrow());
        *self.parsed.borrow_mut() = Rc::new(p);
        self.recompute_health();
    }

    pub fn recompute_health(&self) {
        let Some(sample) = self.sample() else { return };
        let th = self.settings.borrow().thresholds.clone();
        let h = scoring::evaluate(&sample, &self.parsed(), &th);
        *self.health.borrow_mut() = Some(Rc::new(h));
        self.emit(Event::Health);
    }

    pub fn merge_results(&self, outs: impl IntoIterator<Item = CmdOutput>) {
        let mut ids = Vec::new();
        {
            let mut r = self.results.borrow_mut();
            for o in outs {
                ids.push(o.id.clone());
                r.insert(o.id.clone(), o);
            }
        }
        if ids.is_empty() {
            return;
        }
        if ids.iter().any(|i| i.starts_with("apt-install") || i == "apt-autoremove") {
            *self.installed.borrow_mut() = systemhealthcheck::packages::installed_packages();
        }
        self.reparse();
        self.emit(Event::Results(Rc::new(ids)));
        self.check_kernel_alert();
    }

    // -- running commands -----------------------------------------------------

    /// Run registry ids (default params) in the background; privileged ones
    /// share one authentication prompt.
    pub fn run_ids(self: &Rc<Self>, ids: &[&str], label: &str) {
        let plan = ScanPlan::ids(ids);
        if plan.steps.iter().all(|(_, s)| s.is_empty()) {
            return;
        }
        let parallel = self.settings.borrow().parallel;
        let timeout = std::time::Duration::from_secs(self.settings.borrow().command_timeout_s);
        let op = self.begin(label);
        let (tx, rx) = async_channel::unbounded::<CmdOutput>();
        std::thread::spawn(move || {
            let ctl = RunControl { timeout: Some(timeout), ..Default::default() };
            scheduler::run_plan(&plan, parallel, &ctl, &|ev| {
                if let ScanEvent::Command(o) = ev {
                    let _ = tx.send_blocking(o);
                }
            });
        });
        let st = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(o) = rx.recv().await {
                st.merge_results([o]);
            }
            st.end(op);
        });
    }

    /// Run specific commands with explicit parameters (privileged ones share
    /// one authentication prompt). No confirmation: callers only pass reads.
    pub fn run_with_params(self: &Rc<Self>, reqs: Vec<(&'static CommandSpec, Params)>, label: &str) {
        let op = self.begin(label);
        let timeout = std::time::Duration::from_secs(self.settings.borrow().command_timeout_s);
        let (tx, rx) = async_channel::unbounded::<CmdOutput>();
        std::thread::spawn(move || {
            let ctl = RunControl { timeout: Some(timeout), ..Default::default() };
            let (sudo, plain): (Vec<_>, Vec<_>) = reqs.into_iter().partition(|(s, _)| s.requires_sudo);
            for o in runner::run_privileged_batch(&sudo, &ctl) {
                let _ = tx.send_blocking(o);
            }
            for (s, p) in plain {
                let _ = tx.send_blocking(runner::run(s, &p, &ctl));
            }
        });
        let st = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(o) = rx.recv().await {
                st.merge_results([o]);
            }
            st.end(op);
        });
    }

    /// Run a scan plan with progress callbacks; saves a snapshot to history.
    pub fn run_scan(self: &Rc<Self>, kind: ScanKind, plan: ScanPlan, on_event: impl Fn(&ScanEvent) + 'static, on_done: impl FnOnce(Option<i64>) + 'static) {
        let parallel = self.settings.borrow().parallel;
        let timeout = std::time::Duration::from_secs(self.settings.borrow().command_timeout_s);
        let op = self.begin(kind.label());
        let (tx, rx) = async_channel::unbounded::<ScanEvent>();
        std::thread::spawn(move || {
            let ctl = RunControl { timeout: Some(timeout), ..Default::default() };
            scheduler::run_plan(&plan, parallel, &ctl, &|ev| {
                let _ = tx.send_blocking(ev);
            });
        });
        let st = self.clone();
        glib::spawn_future_local(async move {
            let mut collected = BTreeMap::new();
            while let Ok(ev) = rx.recv().await {
                if let ScanEvent::Command(o) = &ev {
                    collected.insert(o.id.clone(), o.clone());
                    st.merge_results([o.clone()]);
                }
                on_event(&ev);
            }
            let saved = st.save_snapshot(kind, collected);
            st.end(op);
            on_done(saved);
        });
    }

    fn save_snapshot(&self, kind: ScanKind, results: BTreeMap<String, CmdOutput>) -> Option<i64> {
        let sample = self.sample().map(|s| (*s).clone()).unwrap_or_else(|| Monitor::new().sample(true));
        let snap = Snapshot { taken_at: util::now(), kind, system: self.system.clone(), sample, results };
        let th = self.settings.borrow().thresholds.clone();
        let health = scoring::evaluate(&snap.sample, &snap.parsed(), &th);
        self.last_scan.set(Some(snap.taken_at));
        let id = match &self.db {
            Some(db) => match db.save_scan(&snap, &health) {
                Ok(id) => Some(id),
                Err(e) => {
                    self.toast(&format!("Could not save scan: {e}"));
                    None
                }
            },
            None => None,
        };
        self.emit(Event::History);
        id
    }

    /// Ask for confirmation when the registry requires it. Resolves to `true` to proceed.
    pub async fn confirm(self: &Rc<Self>, spec: &CommandSpec, params: &Params) -> bool {
        let note = self.confirm_note.borrow_mut().take();
        if !spec.needs_confirmation() {
            return true;
        }
        let body = format!(
            "{}{}\n\nCommand that will run:\n{}\n\nChecklist command: {}\nRisk: {}{}",
            spec.warning.unwrap_or(spec.description),
            note.map(|n| format!("\n\n{n}")).unwrap_or_default(),
            runner::display_command(spec, params),
            spec.original,
            spec.danger.label(),
            if spec.requires_sudo && !util::is_root() { "\nYou will be asked to authenticate as an administrator." } else { "" }
        );
        let dialog = adw::AlertDialog::new(Some(&format!("Run “{}”?", spec.name)), Some(&body));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("run", "Run");
        let appearance = if spec.danger >= registry::Danger::ModifySystem { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested };
        dialog.set_response_appearance("run", appearance);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let win = self.window.borrow().clone();
        dialog.choose_future(win.as_ref()).await == "run"
    }

    /// Confirm if needed, then run one command with streamed output.
    /// `on_line` receives (is_stderr, line); `on_done` the final result.
    pub fn run_spec(
        self: &Rc<Self>,
        spec: &'static CommandSpec,
        params: Params,
        ctl: RunControl,
        on_line: Option<Rc<dyn Fn(bool, &str)>>,
        on_done: impl FnOnce(Option<CmdOutput>) + 'static,
    ) {
        let st = self.clone();
        glib::spawn_future_local(async move {
            if !st.confirm(spec, &params).await {
                on_done(None);
                return;
            }
            let op = st.begin(spec.name);
            enum Msg {
                Line(bool, String),
                Done(Box<CmdOutput>),
            }
            let (tx, rx) = async_channel::unbounded::<Msg>();
            let txl = tx.clone();
            let mut ctl = ctl;
            ctl.on_line = Some(Arc::new(move |e, l: &str| {
                let _ = txl.send_blocking(Msg::Line(e, l.to_string()));
            }));
            std::thread::spawn(move || {
                let out = runner::run(spec, &params, &ctl);
                let _ = tx.send_blocking(Msg::Done(Box::new(out)));
            });
            let mut on_done = Some(on_done);
            while let Ok(m) = rx.recv().await {
                match m {
                    Msg::Line(e, l) => {
                        if let Some(f) = &on_line {
                            f(e, &l);
                        }
                    }
                    Msg::Done(out) => {
                        st.merge_results([(*out).clone()]);
                        if let Some(f) = on_done.take() {
                            f(Some(*out));
                        }
                        break;
                    }
                }
            }
            st.end(op);
        });
    }

    /// Install the package that provides `binary` (always confirmed).
    pub fn install_for(self: &Rc<Self>, binary: &str) {
        let Some(dep) = systemhealthcheck::packages::for_binary(binary) else {
            self.toast(&format!("No known package provides `{binary}`"));
            return;
        };
        self.install_dep(dep);
    }

    pub fn install_dep(self: &Rc<Self>, dep: &'static systemhealthcheck::packages::Dependency) {
        let Some(spec) = registry::get(dep.install_id) else { return };
        let mut params = Params::new();
        if spec.params.iter().any(|p| p.key == "pkg") {
            params.insert("pkg".into(), dep.package.into());
        }
        let st = self.clone();
        let pkg = dep.package;
        self.run_spec(spec, params, RunControl::default(), None, move |out| {
            if let Some(o) = out {
                st.toast(&if o.ok() { format!("{pkg} installed") } else { format!("Installing {pkg}: {} — {}", o.status.label(), o.detail) });
            }
        });
    }

    // -- monitoring ---------------------------------------------------------

    pub fn start_monitor(self: &Rc<Self>) {
        if self.monitoring.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = self.monitoring.clone();
        let interval = self.interval_ms.clone();
        // A restarted monitor supersedes any thread still finishing its sleep.
        let gen = self.monitor_gen.clone();
        let mine = gen.fetch_add(1, Ordering::SeqCst) + 1;
        let alive = move || running.load(Ordering::SeqCst) && gen.load(Ordering::SeqCst) == mine;
        let (tx, rx) = async_channel::bounded::<Sample>(2);
        std::thread::spawn(move || {
            let mut mon = Monitor::new();
            let mut last_procs = std::time::Instant::now() - std::time::Duration::from_secs(10);
            while alive() {
                let start = std::time::Instant::now();
                // Process scans are capped at one per second.
                let with_procs = last_procs.elapsed().as_millis() >= 950;
                if with_procs {
                    last_procs = std::time::Instant::now();
                }
                if tx.send_blocking(mon.sample(with_procs)).is_err() {
                    break;
                }
                let wait = interval.load(Ordering::SeqCst).max(100);
                while alive() && (start.elapsed().as_millis() as u64) < wait {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            }
        });
        let st = self.clone();
        glib::spawn_future_local(async move {
            let mut last_procs: Vec<systemhealthcheck::collectors::procs::Proc> = Vec::new();
            while let Ok(mut s) = rx.recv().await {
                if s.processes.is_empty() {
                    s.processes = std::mem::take(&mut last_procs);
                }
                last_procs = s.processes.clone();
                let s = Rc::new(s);
                *st.sample.borrow_mut() = Some(s.clone());
                let t = st.health_tick.get();
                st.health_tick.set(t + 1);
                if t % 5 == 0 {
                    st.recompute_health();
                }
                st.check_alerts(&s);
                st.emit(Event::Sample(s));
            }
        });
        self.emit(Event::Busy);
    }

    pub fn stop_monitor(&self) {
        self.monitoring.store(false, Ordering::SeqCst);
        self.emit(Event::Busy);
    }

    /// One-off sample when monitoring is stopped.
    pub fn sample_once(self: &Rc<Self>) {
        let st = self.clone();
        glib::spawn_future_local(async move {
            let s = gtk::gio::spawn_blocking(|| {
                let mut m = Monitor::new();
                let _ = m.sample(false);
                std::thread::sleep(std::time::Duration::from_millis(250));
                m.sample(true)
            })
            .await;
            if let Ok(s) = s {
                let s = Rc::new(s);
                *st.sample.borrow_mut() = Some(s.clone());
                st.recompute_health();
                st.emit(Event::Sample(s));
            }
        });
    }

    // -- alerts -------------------------------------------------------------

    fn alert(&self, key: &'static str, msg: &str) {
        let now = util::now();
        let mut last = self.alert_last.borrow_mut();
        if last.get(key).is_some_and(|t| now - t < 300) {
            return;
        }
        last.insert(key, now);
        drop(last);
        self.toast(msg);
        if let Some(app) = gtk::gio::Application::default() {
            let n = gtk::gio::Notification::new("SystemHealthCheck");
            n.set_body(Some(msg));
            app.send_notification(Some(key), &n);
        }
    }

    fn check_alerts(&self, s: &Sample) {
        let cfg = self.settings.borrow().clone();
        if cfg.alert_temperature {
            if let Some(hot) = s.sensors.iter().filter(|x| x.value.is_some_and(|v| v >= cfg.alert_temperature_c)).max_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal)) {
                self.alert("temp", &format!("High temperature: {} {} at {:.0} °C", hot.chip, hot.label, hot.value.unwrap_or(0.0)));
            }
        }
        if cfg.alert_battery {
            if let Some(b) = s.batteries.iter().find(|b| b.status == "Discharging" && b.capacity_pct.is_some_and(|c| c <= cfg.alert_battery_pct)) {
                self.alert("battery", &format!("Battery low: {:.0}%", b.capacity_pct.unwrap_or(0.0)));
            }
        }
        if cfg.alert_storage {
            if let Some(f) = s.filesystems.iter().find(|f| f.used_pct() >= cfg.alert_storage_pct) {
                self.alert("storage", &format!("{} is {:.0}% full", f.mountpoint, f.used_pct()));
            }
        }
    }

    fn check_kernel_alert(&self) {
        if !self.settings.borrow().alert_kernel {
            return;
        }
        let errs = self.parsed().kernel_counts.map(|c| c.0).unwrap_or(0);
        let th = self.settings.borrow().thresholds.kernel_errors[1];
        if errs as f64 >= th {
            self.alert("kernel", &format!("Kernel log contains {errs} errors"));
        }
    }
}
