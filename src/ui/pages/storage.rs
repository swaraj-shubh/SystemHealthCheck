//! Storage: drives, filesystems with usage bars, NVMe/SMART health cards
//! (every reported field kept), hdparm timing and explicit large-directory scans.

use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, hbox, kv, label, set_usage, usage_bar, vbox, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::diagnostics::registry;
use systemhealthcheck::diagnostics::runner::{Params, RunControl};
use systemhealthcheck::parsers::storage::{parse_du, DriveHealth};
use systemhealthcheck::util::{fmt_bytes, fmt_duration};

const IDS: &[&str] = &["lsblk", "nvme-list", "df", "nvme-smart-log", "smartctl-a", "smartctl-x", "hdparm", "du-home", "du-root"];

/// Whole disks from /sys/block (no loop/ram/optical).
fn disks() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.retain(|n| !n.starts_with("loop") && !n.starts_with("ram") && !n.starts_with("zram") && !n.starts_with("sr"));
    v.sort();
    v
}

fn health_tiles(d: &DriveHealth) -> Vec<(&'static str, String, &'static str)> {
    let wear_css = d.percentage_used.map_or("status-unknown", |w| widgets::level_css(w, 50.0, 80.0, 95.0));
    let zero_css = |v: Option<f64>| v.map_or("status-unknown", |v| if v == 0.0 { "status-good" } else { "status-critical" });
    vec![
        ("Wear (percentage used)", widgets::or_na(d.percentage_used, "%"), wear_css),
        ("Temperature", widgets::or_na(d.temperature_c, " °C"), d.temperature_c.map_or("status-unknown", |t| widgets::level_css(t, 65.0, 72.0, 80.0))),
        ("Media errors", widgets::or_na(d.media_errors, ""), zero_css(d.media_errors)),
        ("Critical warning", widgets::or_na(d.critical_warning, ""), zero_css(d.critical_warning.map(|v| v as f64))),
        ("Available spare", widgets::or_na(d.available_spare_pct.map(|v| format!("{v}% (threshold {}%)", d.spare_threshold_pct.unwrap_or(0.0))), ""), "status-unknown"),
        ("SMART overall health", d.smart_passed.map_or("Not reported".into(), |p| if p { "PASSED".into() } else { "FAILED".into() }), d.smart_passed.map_or("status-unknown", |p| if p { "status-good" } else { "status-critical" })),
        ("Power-on hours", widgets::or_na(d.power_on_hours, ""), "status-unknown"),
        ("Power cycles", widgets::or_na(d.power_cycles, ""), "status-unknown"),
        ("Unsafe shutdowns", widgets::or_na(d.unsafe_shutdowns, ""), "status-unknown"),
        ("Data read", d.data_read_bytes.map_or("Not reported".into(), |b| fmt_bytes(b as u64)), "status-unknown"),
        ("Data written", d.data_written_bytes.map_or("Not reported".into(), |b| fmt_bytes(b as u64)), "status-unknown"),
        ("Error log entries", widgets::or_na(d.error_log_entries, ""), "status-unknown"),
    ]
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Storage", "Drives, filesystems, SSD/NVMe health and disk usage", IDS);

    // Drives
    page.overview.append(&widgets::section("Drives"));
    let drives = DataTable::new(&[col("Drive"), wide("Model"), num("Capacity"), col("Type"), num("Read/s"), num("Write/s"), num("Busy")], false, 0, 0);
    page.overview.append(&drives.widget);

    // Filesystems
    page.overview.append(&widgets::section("Filesystems"));
    let fs_box = vbox(8);
    page.overview.append(&fs_box);

    // SMART
    page.overview.append(&widgets::section("SSD / NVMe health"));
    let smart_bar = hbox(8);
    let names = disks();
    let dev_dd = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
    dev_dd.set_tooltip_text(Some("Drive to inspect"));
    dev_dd.update_property(&[gtk::accessible::Property::Label("Drive to inspect")]);
    let read_smart = gtk::Button::with_label("Read SMART / NVMe health");
    read_smart.add_css_class("suggested-action");
    read_smart.set_tooltip_text(Some("Runs nvme smart-log, smartctl -a and smartctl -x on the selected drive (administrator authentication)"));
    smart_bar.append(&label("Drive", &["dim-label"]));
    smart_bar.append(&dev_dd);
    smart_bar.append(&read_smart);
    page.overview.append(&smart_bar);
    let tiles = widgets::card_grid(2, 6);
    page.overview.append(&tiles);
    let smart_note = label("", &["dim-label"]);
    page.overview.append(&smart_note);
    let fields_exp = gtk::Expander::new(Some("All SMART / NVMe fields returned by the device"));
    let fields = DataTable::new(&[col("Field"), wide("Value")], true, 0, 280);
    fields_exp.set_child(Some(&fields.widget));
    page.overview.append(&fields_exp);
    let ata_exp = gtk::Expander::new(Some("ATA SMART attributes"));
    let ata = DataTable::new(&[num("ID"), wide("Attribute"), num("Value"), num("Worst"), num("Thresh"), col("Raw")], false, 0, 240);
    ata_exp.set_child(Some(&ata.widget));
    page.overview.append(&ata_exp);

    // Performance
    page.overview.append(&widgets::section("Read performance (hdparm -Tt)"));
    let perf = hbox(8);
    let hdparm_btn = gtk::Button::with_label("Run hdparm -Tt…");
    let perf_kv = KvGrid::new();
    perf.append(&hdparm_btn);
    page.overview.append(&perf);
    page.overview.append(&perf_kv.grid);

    // Large directories
    page.overview.append(&widgets::section("Large directories"));
    page.overview.append(&label("Recursive scans are never run automatically. They can take minutes on large disks.", &["dim-label"]));
    let du_bar = hbox(8);
    let du_home = gtk::Button::with_label("Scan home directory");
    let du_root = gtk::Button::with_label("Scan root filesystem (administrator)");
    let du_stop = gtk::Button::with_label("Stop");
    du_stop.set_sensitive(false);
    let du_spin = adw::Spinner::new();
    du_spin.set_visible(false);
    let du_status = label("", &["dim-label"]);
    for w in [du_home.upcast_ref::<gtk::Widget>(), du_root.upcast_ref(), du_stop.upcast_ref(), du_spin.upcast_ref(), du_status.upcast_ref()] {
        du_bar.append(w);
    }
    page.overview.append(&du_bar);
    let du_list = vbox(4);
    page.overview.append(&du_list);

    // Live drive / filesystem data
    let drives2 = drives.clone();
    let fsb = fs_box.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        if !fsb.is_mapped() {
            return;
        }
        drives2.set_rows(
            s.disks
                .iter()
                .map(|d| {
                    vec![
                        format!("/dev/{}", d.name),
                        d.model.clone(),
                        fmt_bytes(d.size_bytes),
                        if d.rotational { "HDD".into() } else if d.removable { "Removable".into() } else { "SSD / flash".into() },
                        format!("{}/s", fmt_bytes(d.read_rate as u64)),
                        format!("{}/s", fmt_bytes(d.write_rate as u64)),
                        format!("{:.0}%", d.busy_pct),
                    ]
                })
                .collect(),
        );
        widgets::clear(&fsb);
        for f in &s.filesystems {
            let row = vbox(4);
            row.add_css_class("card");
            row.add_css_class("sp-card");
            let top = hbox(8);
            let t = label(&format!("{}  ({})", f.mountpoint, f.source), &["heading"]);
            t.set_hexpand(true);
            top.append(&t);
            top.append(&label(&format!("{} · {} used of {} · {} free", f.fstype, fmt_bytes(f.used), fmt_bytes(f.used + f.avail), fmt_bytes(f.avail)), &["dim-label"]));
            let bar = usage_bar();
            set_usage(&bar, f.used_pct());
            row.append(&top);
            row.append(&bar);
            fsb.append(&row);
        }
    });

    let st = state.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        tiles.remove_all();
        let p = st.parsed();
        match &p.drive {
            Some(d) if d.has_data() => {
                for (name, value, css) in health_tiles(d) {
                    let c = vbox(2);
                    c.add_css_class("card");
                    c.add_css_class("sp-card");
                    c.append(&label(name, &["caption", "dim-label"]));
                    let v = label(&value, &["title-3", css]);
                    c.append(&v);
                    c.update_property(&[gtk::accessible::Property::Label(&format!("{name}: {value}"))]);
                    tiles.append(&c);
                }
                smart_note.set_text(&format!("Model: {} · Firmware: {} · Capacity: {}", d.model.clone().unwrap_or_default(), d.firmware.clone().unwrap_or_default(), d.capacity.clone().unwrap_or_default()));
                fields.set_rows(d.fields.iter().map(|(k, v)| vec![k.clone(), st.privacy().redact(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default()]).collect());
                ata.set_rows(d.ata_attributes.clone());
                ata_exp.set_visible(!d.ata_attributes.is_empty());
            }
            _ => {
                let why = ["nvme-smart-log", "smartctl-a"].iter().find_map(|i| st.result(i)).map_or(
                    "Not collected yet. SMART data needs administrator privileges: click “Read SMART / NVMe health”.".to_string(),
                    |o| format!("{}: {}", o.status.label(), o.detail),
                );
                smart_note.set_text(&why);
                fields.set_rows(Vec::new());
                ata_exp.set_visible(false);
            }
        }
        let (c, b) = p.hdparm;
        perf_kv.set(&if c.is_none() && b.is_none() {
            vec![]
        } else {
            kv([("Cached reads", widgets::or_na(c, " MB/s")), ("Buffered disk reads", widgets::or_na(b, " MB/s"))])
        });
    });

    let selected_dev = {
        let dd = dev_dd.clone();
        move || names.get(dd.selected() as usize).map(|n| format!("/dev/{n}"))
    };
    let selected_dev = Rc::new(selected_dev);
    let st = state.clone();
    let sd = selected_dev.clone();
    read_smart.connect_clicked(move |_| {
        let Some(dev) = sd() else { return };
        let mut reqs = Vec::new();
        for id in ["smartctl-a", "smartctl-x"] {
            if let Some(s) = registry::get(id) {
                reqs.push((s, Params::from([("dev".to_string(), dev.clone())])));
            }
        }
        if dev.starts_with("/dev/nvme") {
            let ctrl: String = dev.trim_start_matches("/dev/").split('n').take(2).collect::<Vec<_>>().join("n");
            if let Some(s) = registry::get("nvme-smart-log") {
                reqs.push((s, Params::from([("ctrl".to_string(), format!("/dev/{ctrl}"))])));
            }
            if let Some(s) = registry::get("nvme-list") {
                reqs.push((s, Params::new()));
            }
        }
        st.run_with_params(reqs, "Reading SMART data");
    });
    let st = state.clone();
    let sd = selected_dev.clone();
    hdparm_btn.connect_clicked(move |_| {
        let (Some(dev), Some(spec)) = (sd(), registry::get("hdparm")) else { return };
        st.run_spec(spec, Params::from([("dev".to_string(), dev)]), RunControl::default(), None, |_| {});
    });

    // du scans with progress and stop.
    let run_du = {
        let st = state.clone();
        let (du_stop, du_spin, du_status, du_list, du_home2, du_root2) = (du_stop.clone(), du_spin.clone(), du_status.clone(), du_list.clone(), du_home.clone(), du_root.clone());
        move |id: &'static str| {
            let Some(spec) = registry::get(id) else { return };
            let ctl = RunControl::default();
            let ctl_stop = ctl.clone();
            let h = du_stop.connect_clicked(move |_| ctl_stop.cancel());
            du_stop.set_sensitive(!spec.requires_sudo);
            du_spin.set_visible(true);
            du_home2.set_sensitive(false);
            du_root2.set_sensitive(false);
            let start = std::time::Instant::now();
            let status = du_status.clone();
            status.set_text("Scanning…");
            let tick_status = status.clone();
            let ticker = gtk::glib::timeout_add_seconds_local(1, move || {
                tick_status.set_text(&format!("Scanning… {}", fmt_duration(start.elapsed().as_secs_f64())));
                gtk::glib::ControlFlow::Continue
            });
            let ticker = std::cell::Cell::new(Some(ticker));
            let (stop, spin, list, h1, h2) = (du_stop.clone(), du_spin.clone(), du_list.clone(), du_home2.clone(), du_root2.clone());
            st.run_spec(spec, Params::new(), ctl, None, move |out| {
                if let Some(t) = ticker.take() {
                    t.remove();
                }
                stop.disconnect(h);
                stop.set_sensitive(false);
                spin.set_visible(false);
                h1.set_sensitive(true);
                h2.set_sensitive(true);
                let Some(o) = out else {
                    status.set_text("Cancelled before start");
                    return;
                };
                status.set_text(&format!("{} in {}", o.status.label(), fmt_duration(o.duration_ms as f64 / 1000.0)));
                widgets::clear(&list);
                let rows = parse_du(&o.stdout);
                let max = rows.iter().map(|r| r.1).fold(1.0, f64::max);
                for (size, bytes, path) in rows.iter().rev() {
                    let r = hbox(8);
                    let l = label(&format!("{size:>6}  {path}"), &["mono"]);
                    l.set_width_chars(48);
                    let bar = gtk::ProgressBar::new();
                    bar.add_css_class("usage");
                    bar.set_fraction(bytes / max);
                    bar.set_hexpand(true);
                    bar.set_valign(gtk::Align::Center);
                    r.append(&l);
                    r.append(&bar);
                    list.append(&r);
                }
            });
        }
    };
    let run_du = Rc::new(run_du);
    let r = run_du.clone();
    du_home.connect_clicked(move |_| r("du-home"));
    let r = run_du.clone();
    du_root.connect_clicked(move |_| r("du-root"));
    page.root.upcast()
}
