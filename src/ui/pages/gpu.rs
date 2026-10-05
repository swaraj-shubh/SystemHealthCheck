//! GPU: PCI display controllers with drivers, OpenGL (glxinfo -B) and live
//! sysfs utilization where the driver exposes it.

use crate::ui::state::{AppState, Event};
use crate::ui::widgets::{self, kv, label, set_usage, usage_bar, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::collectors::power::SensorKind;
use systemhealthcheck::parsers::kv_get;
use systemhealthcheck::util::fmt_bytes;

const IDS: &[&str] = &["lspci-gpu", "lspci-gpu-driver", "glxinfo", "lspci-vmm"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "GPU", "Graphics hardware, kernel driver and OpenGL renderer", IDS);
    let grid = widgets::card_grid(1, 3);
    page.overview.append(&grid);

    let live = Card::new("Live", "utilities-system-monitor-symbolic");
    let busy = label("…", &["big-value"]);
    let bar = usage_bar();
    let live_kv = KvGrid::new();
    live.body.append(&busy);
    live.body.append(&bar);
    live.body.append(&live_kv.grid);
    let gl = Card::new("OpenGL (glxinfo -B)", "video-display-symbolic");
    let gl_kv = KvGrid::new();
    gl.body.append(&gl_kv.grid);
    let fixed = widgets::card_grid(1, 2);
    fixed.append(&gl.root);
    fixed.append(&live.root);
    page.overview.append(&fixed);

    let st = state.clone();
    let grid2 = grid.clone();
    widgets::bind(state, &page.overview, widgets::on_data, move || {
        grid2.remove_all();
        let p = st.parsed();
        let g = |k: &str| kv_get(&p.glxinfo, k).unwrap_or("").to_string();
        gl_kv.set(&if p.glxinfo.is_empty() {
            kv([("Status", st.result("glxinfo").map_or("Not run".into(), |o| format!("{} — {}", o.status.label(), o.detail)))])
        } else {
            kv([
                ("OpenGL renderer", g("OpenGL renderer string")),
                ("OpenGL vendor", g("OpenGL vendor string")),
                ("Core profile version", g("OpenGL core profile version string")),
                ("OpenGL version", g("OpenGL version string")),
                ("Direct rendering", g("direct rendering")),
                ("Accelerated", g("Accelerated")),
                ("Video memory", g("Video memory")),
                ("Unified memory", g("Unified memory")),
                ("Display", g("name of display")),
            ])
        });
        let gpus: Vec<_> = p.pci.iter().filter(|d| d.is_display()).collect();
        if gpus.is_empty() {
            let c = Card::new("Display controllers", "video-display-symbolic");
            c.body.append(&label("No PCI display controller parsed yet (lspci -vmmnnk). Integrated SoC GPUs may not appear on PCI.", &["dim-label"]));
            grid2.append(&c.root);
        }
        for d in gpus {
            let c = Card::new(&d.device, "video-display-symbolic");
            let k = KvGrid::new();
            k.set(&kv([
                ("Slot", d.slot.clone()),
                ("Class", d.class.clone()),
                ("Vendor", d.vendor.clone()),
                ("Device", d.device.clone()),
                ("Subsystem", d.subsystem.clone()),
                ("Kernel driver in use", d.driver.clone()),
                ("Kernel modules", d.modules.clone()),
                ("Revision", d.rev.clone()),
            ]));
            c.body.append(&k.grid);
            grid2.append(&c.root);
        }
    });
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        if !busy.is_mapped() {
            return;
        }
        let g = s.gpus.iter().find(|g| g.busy_pct.is_some()).or(s.gpus.first());
        match g.and_then(|g| g.busy_pct) {
            Some(b) => {
                busy.set_text(&format!("{b:.0}% busy"));
                set_usage(&bar, b);
                bar.set_visible(true);
            }
            None => {
                busy.set_text("Utilization n/a");
                bar.set_visible(false);
            }
        }
        live_kv.set(&kv([
            ("Card", g.map(|g| format!("{} ({} {})", g.card, g.vendor_name(), g.device_id)).unwrap_or_else(|| "No DRM device".into())),
            ("Driver", g.map(|g| g.driver.clone()).unwrap_or_default()),
            ("Temperature", widgets::or_na(s.max_temp(SensorKind::Gpu).map(|t| format!("{t:.1}")), " °C")),
            (
                "VRAM",
                match g.and_then(|g| Some((g.vram_used?, g.vram_total?))) {
                    Some((u, t)) => format!("{} / {}", fmt_bytes(u), fmt_bytes(t)),
                    None => "Not exposed".into(),
                },
            ),
            ("Utilization", if g.and_then(|g| g.busy_pct).is_some() { "From sysfs gpu_busy_percent".into() } else { "GPU utilization is not exposed by this hardware/driver.".into() }),
        ]));
    });
    page.root.upcast()
}
