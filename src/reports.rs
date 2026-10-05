//! Diagnostic report: one document model rendered to plain text, HTML, JSON
//! and PDF (cairo + pango, no browser). Privacy options are applied while the
//! model is built, so every format gets the same redactions.

use crate::collectors::power::SensorKind;
use crate::database::BenchRow;
use crate::diagnostics::{registry, Parsed, Snapshot};
use crate::privacy::Privacy;
use crate::scoring::HealthReport;
use crate::util::{fmt_bytes, fmt_duration, fmt_time};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "content", rename_all = "lowercase")]
pub enum Block {
    Para(String),
    Kv(Vec<(String, String)>),
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Pre(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    pub generated_at: String,
    pub scan_taken_at: String,
    pub privacy_note: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Html,
    Json,
    Text,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Html => "html",
            Format::Json => "json",
            Format::Text => "txt",
        }
    }
}

/// Report options. Serial numbers are excluded unless explicitly requested.
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    pub include_serial: bool,
    pub mask_hostname: bool,
    pub exclude_network: bool,
}

fn kv<I: IntoIterator<Item = (S, T)>, S: Into<String>, T: Into<String>>(items: I) -> Block {
    Block::Kv(items.into_iter().map(|(a, b)| (a.into(), b.into())).filter(|(_, b): &(String, String)| !b.is_empty()).collect())
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Block {
    Block::Table { headers: headers.iter().map(|h| h.to_string()).collect(), rows }
}

fn opt<T: std::fmt::Display>(v: Option<T>, unit: &str) -> String {
    v.map(|v| format!("{v}{unit}")).unwrap_or_else(|| "Not reported".into())
}

const NETWORK_IDS: &[&str] = &["lshw-network", "iw-dev", "nmcli-status", "hostnamectl"];

/// Build the report document.
pub fn build(snap: &Snapshot, health: &HealthReport, benches: &[BenchRow], stresses: &[BenchRow], opts: &ReportOptions) -> Report {
    let p: Parsed = snap.parsed();
    let s = &snap.sample;
    let sys = &snap.system;
    let privacy = Privacy {
        mask_serial: !opts.include_serial,
        hostname: opts.mask_hostname.then(|| sys.hostname.clone()),
        mask_network: opts.exclude_network,
        secrets: p.secrets(),
    };
    let r = |t: &str| privacy.redact(t);
    let mut sec: Vec<Section> = Vec::new();
    let mut add = |title: &str, blocks: Vec<Block>| sec.push(Section { title: title.into(), blocks });

    // 1. System summary
    add(
        "1. System Summary",
        vec![kv([
            ("Operating system", sys.os_name.clone()),
            ("Kernel", sys.kernel.clone()),
            ("Architecture", sys.arch.clone()),
            ("Hostname", if opts.mask_hostname { "<hostname>".into() } else { sys.hostname.clone() }),
            ("Vendor", sys.vendor.clone()),
            ("Model", sys.product.clone()),
            ("Chassis", sys.chassis.clone()),
            ("BIOS", format!("{} {} ({})", sys.bios_vendor, sys.bios_version, sys.bios_date).trim().to_string()),
            ("Desktop", format!("{} {}", sys.desktop, sys.session_type).trim().to_string()),
            ("Uptime", fmt_duration(s.uptime_s)),
            ("Scan", format!("{} at {}", snap.kind.label(), fmt_time(snap.taken_at))),
        ])],
    );

    // 2. Overall health
    let mut blocks = vec![
        Block::Para(format!(
            "Overall: {} — {}",
            health.overall.map_or("Insufficient data".into(), |o| format!("{o}/100")),
            health.state.label()
        )),
        Block::Para(health.rule.clone()),
        table(
            &["Category", "Score", "State"],
            health
                .categories
                .iter()
                .map(|c| vec![c.name.clone(), c.score.map_or("—".into(), |v| format!("{v}/100")), c.state.label().into()])
                .collect(),
        ),
    ];
    let rows: Vec<Vec<String>> = health
        .categories
        .iter()
        .flat_map(|c| c.findings.iter().map(move |f| vec![c.name.clone(), f.input.clone(), f.rule.clone(), f.severity.label().into(), f.reason.clone()]))
        .collect();
    blocks.push(table(&["Category", "Input", "Rule", "Severity", "Reason"], rows));
    add("2. Overall Health", blocks);

    // 3. CPU
    let lscpu = |k: &str| crate::parsers::kv_get(&p.lscpu, k).unwrap_or("").to_string();
    add(
        "3. CPU",
        vec![kv([
            ("Model", s.cpu.model.clone()),
            ("Threads", s.cpu.threads.to_string()),
            ("Physical cores", opt(s.cpu.cores, "")),
            ("Sockets", lscpu("Socket(s)")),
            ("Usage at scan", opt(s.cpu.total_usage.map(|u| format!("{u:.1}")), "%")),
            ("Average frequency", opt(s.cpu.avg_mhz().map(|m| format!("{m:.0}")), " MHz")),
            ("Min / max frequency", format!("{} / {}", opt(s.cpu.min_mhz, " MHz"), opt(s.cpu.max_mhz, " MHz"))),
            ("Governor", s.cpu.governor.clone().unwrap_or_default()),
            ("Available governors", s.cpu.available_governors.clone().unwrap_or_default()),
            ("L1d / L1i / L2 / L3", format!("{} / {} / {} / {}", lscpu("L1d cache"), lscpu("L1i cache"), lscpu("L2 cache"), lscpu("L3 cache"))),
            ("Temperature", opt(s.max_temp(SensorKind::Cpu).map(|t| format!("{t:.1}")), " °C")),
        ])],
    );

    // 4. RAM
    let m = &s.memory;
    let mut blocks = vec![kv([
        ("Total", fmt_bytes(m.total_kb * 1024)),
        ("Used", fmt_bytes(m.used_kb() * 1024)),
        ("Available", fmt_bytes(m.available_kb * 1024)),
        ("Cached", fmt_bytes(m.cached_kb * 1024)),
        ("Free", fmt_bytes(m.free_kb * 1024)),
    ])];
    match &p.dmi_memory {
        Some(d) => {
            blocks.push(kv([("Maximum capacity (firmware)", opt(d.max_capacity.clone(), "")), ("Slots", opt(d.slots, ""))]));
            blocks.push(table(
                &["Slot", "Size", "Type", "Speed", "Configured", "Manufacturer", "Part"],
                d.modules
                    .iter()
                    .map(|x| {
                        vec![
                            x.locator.clone(),
                            x.size.clone().unwrap_or_else(|| "Empty".into()),
                            x.mem_type.clone(),
                            x.speed.clone(),
                            x.configured_speed.clone(),
                            x.manufacturer.clone(),
                            x.part_number.clone(),
                        ]
                    })
                    .collect(),
            ));
        }
        None => blocks.push(Block::Para("Module details unavailable (dmidecode requires administrator privileges).".into())),
    }
    add("4. RAM", blocks);

    // 5. Swap
    add(
        "5. Swap",
        vec![
            kv([
                ("Total", fmt_bytes(m.swap_total_kb * 1024)),
                ("Used", fmt_bytes(m.swap_used_kb() * 1024)),
                ("Swappiness", opt(m.swappiness, "")),
            ]),
            table(
                &["Device", "Type", "Size", "Used", "Priority"],
                m.swaps.iter().map(|w| vec![w.name.clone(), w.kind.clone(), fmt_bytes(w.size_kb * 1024), fmt_bytes(w.used_kb * 1024), w.priority.to_string()]).collect(),
            ),
        ],
    );

    // 6. GPU
    let glx = |k: &str| crate::parsers::kv_get(&p.glxinfo, k).unwrap_or("").to_string();
    add(
        "6. GPU",
        vec![
            table(
                &["Slot", "Class", "Vendor", "Device", "Driver"],
                p.pci.iter().filter(|d| d.is_display()).map(|d| vec![d.slot.clone(), d.class.clone(), d.vendor.clone(), d.device.clone(), d.driver.clone()]).collect(),
            ),
            kv([
                ("OpenGL renderer", glx("OpenGL renderer string")),
                ("OpenGL version", glx("OpenGL core profile version string")),
                ("Direct rendering", glx("direct rendering")),
                ("Video memory", glx("Video memory")),
                (
                    "Utilization",
                    s.gpus.iter().find_map(|g| g.busy_pct).map_or("GPU utilization is not exposed by this hardware/driver.".into(), |b| format!("{b:.0}%")),
                ),
            ]),
        ],
    );

    // 7. Storage
    add(
        "7. Storage",
        vec![
            table(
                &["Disk", "Model", "Size", "Type"],
                s.disks
                    .iter()
                    .map(|d| vec![d.name.clone(), d.model.clone(), fmt_bytes(d.size_bytes), if d.rotational { "HDD".into() } else { "SSD/flash".into() }])
                    .collect(),
            ),
            table(
                &["Filesystem", "Mountpoint", "Type", "Size", "Used", "Free", "Use%"],
                s.filesystems
                    .iter()
                    .map(|f| vec![f.source.clone(), f.mountpoint.clone(), f.fstype.clone(), fmt_bytes(f.total), fmt_bytes(f.used), fmt_bytes(f.avail), format!("{:.0}%", f.used_pct())])
                    .collect(),
            ),
        ],
    );

    // 8. NVMe / SMART
    let smart = match &p.drive {
        Some(d) => vec![kv([
            ("Model", d.model.clone().unwrap_or_default()),
            ("SMART overall health", d.smart_passed.map_or("Not reported".into(), |b| if b { "PASSED".into() } else { "FAILED".into() })),
            ("Critical warning", opt(d.critical_warning, "")),
            ("Temperature", opt(d.temperature_c, " °C")),
            ("Percentage used (wear)", opt(d.percentage_used, "%")),
            ("Available spare", opt(d.available_spare_pct, "%")),
            ("Power-on hours", opt(d.power_on_hours, "")),
            ("Power cycles", opt(d.power_cycles, "")),
            ("Unsafe shutdowns", opt(d.unsafe_shutdowns, "")),
            ("Media errors", opt(d.media_errors, "")),
            ("Data read", d.data_read_bytes.map_or("Not reported".into(), |b| fmt_bytes(b as u64))),
            ("Data written", d.data_written_bytes.map_or("Not reported".into(), |b| fmt_bytes(b as u64))),
        ])],
        None => vec![Block::Para("No SMART/NVMe data (tool missing, permission denied or no supported drive).".into())],
    };
    add("8. NVMe / SMART", smart);

    // 9. Temperature
    let mut rows: Vec<Vec<String>> = s
        .sensors
        .iter()
        .map(|t| vec![t.kind.label().into(), t.chip.clone(), t.label.clone(), opt(t.value.map(|v| format!("{v:.1}")), " °C"), opt(t.crit, " °C")])
        .collect();
    rows.extend(s.fans.iter().map(|f| vec!["Fan".into(), f.chip.clone(), f.label.clone(), opt(f.value, " RPM"), String::new()]));
    add("9. Temperature", vec![table(&["Kind", "Chip", "Sensor", "Value", "Critical"], rows)]);

    // 10. Battery
    let bat = match (s.batteries.first(), &p.battery) {
        (None, None) => vec![Block::Para("No battery detected.".into())],
        (b, u) => vec![kv([
            ("Charge", opt(b.and_then(|b| b.capacity_pct).or(u.as_ref().and_then(|u| u.percentage)), "%")),
            ("State", b.map(|b| b.status.clone()).or(u.as_ref().and_then(|u| u.state.clone())).unwrap_or_default()),
            ("Energy", opt(b.and_then(|b| b.energy_now_wh).map(|v| format!("{v:.2}")), " Wh")),
            ("Energy full", opt(b.and_then(|b| b.energy_full_wh).map(|v| format!("{v:.2}")), " Wh")),
            ("Energy full design", opt(b.and_then(|b| b.energy_full_design_wh).map(|v| format!("{v:.2}")), " Wh")),
            ("Health (full / design)", opt(b.and_then(|b| b.health_pct()).or(u.as_ref().and_then(|u| u.health_pct())).map(|v| format!("{v:.1}")), "%")),
            ("Voltage", opt(b.and_then(|b| b.voltage_v).map(|v| format!("{v:.2}")), " V")),
            ("Cycle count", opt(b.and_then(|b| b.cycle_count).or(u.as_ref().and_then(|u| u.charge_cycles)), "")),
            ("Technology", b.and_then(|b| b.technology.clone()).unwrap_or_default()),
        ])],
    };
    add("10. Battery", bat);

    // 11. Processes
    let mut procs = s.processes.clone();
    procs.sort_by(|a, b| b.rss_bytes.cmp(&a.rss_bytes));
    let rows = if procs.is_empty() {
        p.top_mem.iter().map(|x| vec![x.pid.to_string(), x.user.clone(), format!("{:.1}", x.cpu), format!("{:.1}", x.mem), x.command.chars().take(80).collect()]).collect()
    } else {
        procs.iter().take(20).map(|x| vec![x.pid.to_string(), x.user.clone(), format!("{:.1}", x.cpu_pct), format!("{:.1}", x.mem_pct), x.name.clone()]).collect()
    };
    add("11. Processes (top by memory)", vec![table(&["PID", "User", "CPU %", "Mem %", "Command"], rows)]);

    // 12. Boot
    let mut blocks = vec![];
    if let Some(b) = &p.boot {
        blocks.push(kv([
            ("Firmware", opt(b.firmware_s, " s")),
            ("Loader", opt(b.loader_s, " s")),
            ("Kernel", opt(b.kernel_s, " s")),
            ("Userspace", opt(b.userspace_s, " s")),
            ("Total", opt(b.total_s, " s")),
        ]));
    }
    blocks.push(table(&["Time", "Unit"], p.blame.iter().take(15).map(|(t, u)| vec![format!("{t:.3} s"), u.clone()]).collect()));
    add("12. Boot", blocks);

    // 13. Kernel
    let counts = p.kernel_counts.map_or(format!("{} error/warning messages (severity not decoded)", p.kernel_msgs.len()), |(e, w)| format!("{e} errors, {w} warnings"));
    add(
        "13. Kernel",
        vec![
            Block::Para(counts),
            table(
                &["Time (s)", "Severity", "Message"],
                p.kernel_msgs.iter().rev().take(25).map(|k| vec![opt(k.timestamp, ""), k.severity.label().into(), r(&k.message)]).collect(),
            ),
        ],
    );

    // 14/15. PCI / USB
    add("14. PCI", vec![table(&["Slot", "Class", "Vendor", "Device", "Driver"], p.pci.iter().map(|d| vec![d.slot.clone(), d.class.clone(), d.vendor.clone(), d.device.clone(), d.driver.clone()]).collect())]);
    add("15. USB", vec![table(&["Bus", "Device", "ID", "Description"], p.usb.iter().map(|d| vec![d.bus.clone(), d.device.clone(), d.id.clone(), d.description.clone()]).collect())]);

    // 16. Network
    if opts.exclude_network {
        add("16. Network", vec![Block::Para("Network details excluded by privacy settings (interface names and types only).".into()), table(&["Interface", "Type", "State"], s.net.iter().map(|n| vec![n.name.clone(), n.kind.clone(), n.state.clone()]).collect())]);
    } else {
        add(
            "16. Network",
            vec![table(
                &["Interface", "Type", "State", "Driver", "MAC", "Addresses"],
                s.net.iter().map(|n| vec![n.name.clone(), n.kind.clone(), n.state.clone(), n.driver.clone(), n.mac.clone(), n.addrs.join(", ")]).collect(),
            )],
        );
    }

    // 17. Motherboard
    let board = match &p.baseboard {
        Some(b) => vec![Block::Kv(b.fields.iter().map(|(k, v)| (k.clone(), r(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect())],
        None => vec![kv([("Manufacturer", sys.board_vendor.clone()), ("Product", sys.board_name.clone()), ("Version", sys.board_version.clone())])],
    };
    add("17. Motherboard", board);

    // 18. System model
    let mut model = vec![kv([("Manufacturer", sys.vendor.clone()), ("Product", sys.product.clone()), ("Version", sys.product_version.clone())])];
    model.push(Block::Para(match (&p.serial, opts.include_serial) {
        (Some(sn), true) => format!("Serial number: {sn}"),
        (Some(_), false) => "Serial number: omitted (sensitive; enable it explicitly when exporting).".into(),
        (None, _) => "Serial number: not collected.".into(),
    }));
    add("18. System Model", model);

    // 19/20. Benchmark / stress
    let bench_rows = |rows: &[BenchRow]| -> Vec<Vec<String>> {
        rows.iter()
            .take(10)
            .map(|b| {
                let mut v = vec![fmt_time(b.taken_at)];
                if let Some(o) = b.summary.as_object() {
                    v.push(o.iter().map(|(k, x)| format!("{k}={x}")).collect::<Vec<_>>().join(", "));
                }
                v
            })
            .collect()
    };
    add("19. Benchmark", if benches.is_empty() { vec![Block::Para("No benchmark has been run.".into())] } else { vec![table(&["When", "Result"], bench_rows(benches))] });
    add("20. Stress Test", if stresses.is_empty() { vec![Block::Para("No stress test has been run.".into())] } else { vec![table(&["When", "Result"], bench_rows(stresses))] });

    // 21. Package status
    let installed = crate::packages::installed_packages();
    add(
        "21. Package Status",
        vec![table(
            &["Package", "Provides", "Status"],
            crate::packages::DEPENDENCIES
                .iter()
                .map(|d| vec![d.package.into(), d.purpose.into(), if installed.contains(d.package) { "Installed".into() } else { "Missing".into() }])
                .collect(),
        )],
    );

    // 22. Recommendations
    let probs = health.problems();
    let recs = if probs.is_empty() {
        vec![Block::Para("No rule flagged a problem in the collected data.".into())]
    } else {
        vec![table(&["Severity", "Category", "Finding", "Why it matters"], probs.iter().map(|(c, f)| vec![f.severity.label().into(), c.to_string(), f.input.clone(), f.reason.clone()]).collect())]
    };
    add("22. Recommendations", recs);

    // 23. Raw results
    let mut raw = Vec::new();
    for (id, out) in &snap.results {
        if id == "dmidecode-serial" && !opts.include_serial {
            continue;
        }
        if opts.exclude_network && NETWORK_IDS.contains(&id.as_str()) {
            raw.push(Block::Para(format!("$ {} — omitted (network details excluded)", out.command)));
            continue;
        }
        let name = registry::get(id).map_or(id.as_str(), |s| s.name);
        let mut body = format!(
            "$ {}\n# {} | status: {} | exit: {} | {} ms\n",
            r(&out.command),
            name,
            out.status.label(),
            out.exit_code.map_or("-".into(), |c| c.to_string()),
            out.duration_ms
        );
        if !out.detail.is_empty() {
            body += &format!("# {}\n", out.detail);
        }
        body += &r(&out.stdout);
        if !out.stderr.trim().is_empty() {
            body += &format!("\n[stderr]\n{}", r(&out.stderr));
        }
        raw.push(Block::Pre(body));
    }
    add("23. Raw Diagnostic Results", raw);

    Report {
        title: "SystemHealthCheck Diagnostic Report".into(),
        generated_at: fmt_time(crate::util::now()),
        scan_taken_at: fmt_time(snap.taken_at),
        privacy_note: format!(
            "Serial numbers: {}. Hostname: {}. Network details: {}. Generated locally; nothing was uploaded.",
            if opts.include_serial { "included" } else { "masked/omitted" },
            if opts.mask_hostname { "masked" } else { "shown" },
            if opts.exclude_network { "excluded" } else { "included" }
        ),
        sections: sec,
    }
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

pub fn to_text(rep: &Report) -> String {
    let mut s = format!("{}\nGenerated: {}   Scan: {}\n{}\n", rep.title, rep.generated_at, rep.scan_taken_at, rep.privacy_note);
    for sec in &rep.sections {
        s += &format!("\n{}\n{}\n", sec.title, "=".repeat(sec.title.chars().count()));
        for b in &sec.blocks {
            match b {
                Block::Para(p) => s += &format!("{p}\n"),
                Block::Kv(items) => {
                    let w = items.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
                    for (k, v) in items {
                        s += &format!("  {k:<w$}  {v}\n");
                    }
                }
                Block::Table { headers, rows } => {
                    let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
                    for r in rows {
                        for (i, c) in r.iter().enumerate() {
                            if let Some(x) = w.get_mut(i) {
                                *x = (*x).max(c.chars().count()).min(60);
                            }
                        }
                    }
                    let line = |cells: &[String]| {
                        cells.iter().enumerate().map(|(i, c)| format!("{:<width$}", c.chars().take(60).collect::<String>(), width = w.get(i).copied().unwrap_or(0))).collect::<Vec<_>>().join("  ")
                    };
                    s += &format!("  {}\n", line(headers));
                    for r in rows {
                        s += &format!("  {}\n", line(r).trim_end());
                    }
                    if rows.is_empty() {
                        s += "  (no data)\n";
                    }
                }
                Block::Pre(t) => s += &format!("{t}\n"),
            }
        }
    }
    s
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn to_html(rep: &Report) -> String {
    let mut h = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
    );
    h += &format!("<title>{}</title>", html_escape(&rep.title));
    h += "<style>body{font-family:system-ui,sans-serif;max-width:1100px;margin:2rem auto;padding:0 1rem;color:#222;background:#fff}h1{margin-bottom:.2rem}h2{border-bottom:1px solid #ddd;padding-bottom:.3rem;margin-top:2rem}table{border-collapse:collapse;width:100%;margin:.6rem 0;font-size:.9rem}th,td{border:1px solid #ddd;padding:.3rem .5rem;text-align:left;vertical-align:top}th{background:#f4f4f4}pre{background:#f7f7f7;border:1px solid #e3e3e3;padding:.6rem;overflow:auto;font-size:.8rem;white-space:pre-wrap}.meta{color:#666}@media(prefers-color-scheme:dark){body{background:#1e1e1e;color:#ddd}th{background:#2a2a2a}th,td,h2{border-color:#444}pre{background:#252525;border-color:#444}.meta{color:#aaa}}</style></head><body>";
    h += &format!(
        "<h1>{}</h1><p class=\"meta\">Generated {} · Scan {}<br>{}</p>",
        html_escape(&rep.title),
        html_escape(&rep.generated_at),
        html_escape(&rep.scan_taken_at),
        html_escape(&rep.privacy_note)
    );
    for sec in &rep.sections {
        h += &format!("<h2>{}</h2>", html_escape(&sec.title));
        for b in &sec.blocks {
            match b {
                Block::Para(p) => h += &format!("<p>{}</p>", html_escape(p)),
                Block::Kv(items) => {
                    h += "<table>";
                    for (k, v) in items {
                        h += &format!("<tr><th>{}</th><td>{}</td></tr>", html_escape(k), html_escape(v));
                    }
                    h += "</table>";
                }
                Block::Table { headers, rows } => {
                    h += "<table><tr>";
                    for c in headers {
                        h += &format!("<th>{}</th>", html_escape(c));
                    }
                    h += "</tr>";
                    for r in rows {
                        h += "<tr>";
                        for c in r {
                            h += &format!("<td>{}</td>", html_escape(c));
                        }
                        h += "</tr>";
                    }
                    if rows.is_empty() {
                        h += &format!("<tr><td colspan=\"{}\">No data</td></tr>", headers.len());
                    }
                    h += "</table>";
                }
                Block::Pre(t) => h += &format!("<pre>{}</pre>", html_escape(t)),
            }
        }
    }
    h += "</body></html>\n";
    h
}

pub fn to_json(rep: &Report) -> Result<String, String> {
    serde_json::to_string_pretty(rep).map_err(|e| e.to_string())
}

/// Render to PDF with cairo + pango (A4, paginated).
pub fn to_pdf(rep: &Report, path: &std::path::Path) -> Result<(), String> {
    use pangocairo::functions::{create_layout, show_layout};
    use pangocairo::pango;
    const W: f64 = 595.0;
    const H: f64 = 842.0;
    const M: f64 = 40.0;
    let surface = cairo::PdfSurface::new(W, H, path).map_err(|e| e.to_string())?;
    let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    let y = std::cell::Cell::new(M);
    let emit = |markup: Option<&str>, text: &str, font: &str, gap: f64| -> Result<(), String> {
        // Split long text into chunks of lines so pagination stays possible.
        let lines: Vec<&str> = text.lines().collect();
        let chunks: Vec<String> = if lines.len() > 40 { lines.chunks(40).map(|c| c.join("\n")).collect() } else { vec![text.to_string()] };
        for chunk in chunks {
            let layout = create_layout(&cr);
            layout.set_font_description(Some(&pango::FontDescription::from_string(font)));
            layout.set_width(((W - 2.0 * M) * pango::SCALE as f64) as i32);
            layout.set_wrap(pango::WrapMode::WordChar);
            match markup {
                Some(m) => layout.set_markup(m),
                None => layout.set_text(&chunk),
            }
            let (_, h) = layout.pixel_size();
            if y.get() + h as f64 > H - M && y.get() > M {
                cr.show_page().map_err(|e| e.to_string())?;
                y.set(M);
            }
            cr.move_to(M, y.get());
            show_layout(&cr, &layout);
            y.set(y.get() + h as f64 + gap);
        }
        Ok(())
    };
    let esc = |s: &str| html_escape(s);
    emit(Some(&format!("<b>{}</b>", esc(&rep.title))), "", "Sans 18", 4.0)?;
    emit(None, &format!("Generated {} · Scan {}\n{}", rep.generated_at, rep.scan_taken_at, rep.privacy_note), "Sans 8", 10.0)?;
    for sec in &rep.sections {
        emit(Some(&format!("<b>{}</b>", esc(&sec.title))), "", "Sans 13", 4.0)?;
        for b in &sec.blocks {
            match b {
                Block::Para(p) => emit(None, p, "Sans 9", 4.0)?,
                Block::Kv(items) => {
                    let t: String = items.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
                    emit(None, t.trim_end(), "Sans 9", 6.0)?
                }
                Block::Table { .. } => {
                    let single = Report { title: String::new(), generated_at: String::new(), scan_taken_at: String::new(), privacy_note: String::new(), sections: vec![Section { title: String::new(), blocks: vec![b.clone()] }] };
                    let txt = to_text(&single);
                    let body: String = txt.lines().skip(5).collect::<Vec<_>>().join("\n");
                    emit(None, &body, "Monospace 6.5", 6.0)?
                }
                Block::Pre(t) => emit(None, t, "Monospace 6.5", 6.0)?,
            }
        }
        y.set(y.get() + 6.0);
    }
    drop(cr);
    surface.finish();
    Ok(())
}

/// Write a report in the chosen format.
pub fn export(rep: &Report, format: Format, path: &std::path::Path) -> Result<(), String> {
    match format {
        Format::Pdf => to_pdf(rep, path),
        Format::Html => std::fs::write(path, to_html(rep)).map_err(|e| e.to_string()),
        Format::Json => std::fs::write(path, to_json(rep)?).map_err(|e| e.to_string()),
        Format::Text => std::fs::write(path, to_text(rep)).map_err(|e| e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::runner::{CmdOutput, RunStatus};
    use crate::diagnostics::scheduler::ScanKind;

    fn out(id: &str, stdout: &str) -> CmdOutput {
        CmdOutput {
            id: id.into(),
            command: id.into(),
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 1,
            status: RunStatus::Success,
            detail: String::new(),
            missing_tool: None,
            finished_at: 0,
        }
    }

    #[test]
    fn report_never_leaks_serial_by_default() {
        let mut results = std::collections::BTreeMap::new();
        results.insert("dmidecode-serial".to_string(), out("dmidecode-serial", "5CD0SECRET99\n"));
        results.insert("dmidecode-system".to_string(), out("dmidecode-system", "Handle 0x1\nSystem Information\n\tProduct Name: HP\n\tSerial Number: 5CD0SECRET99\n"));
        results.insert("nvme-list".to_string(), out("nvme-list", "Node         SN\n------------ ------------\n/dev/nvme0n1 NVMESERIAL77\n"));
        let snap = Snapshot { taken_at: 1, kind: ScanKind::Full, system: Default::default(), sample: Default::default(), results };
        let health = crate::scoring::evaluate(&snap.sample, &snap.parsed(), &Default::default());
        let rep = build(&snap, &health, &[], &[], &ReportOptions::default());
        assert_eq!(rep.sections.len(), 23);
        for text in [to_text(&rep), to_html(&rep), to_json(&rep).expect("json")] {
            assert!(!text.contains("5CD0SECRET99"), "serial leaked");
            assert!(!text.contains("NVMESERIAL77"), "nvme serial leaked");
        }
        let rep = build(&snap, &health, &[], &[], &ReportOptions { include_serial: true, ..Default::default() });
        assert!(to_text(&rep).contains("5CD0SECRET99"));
    }

    #[test]
    fn pdf_renders() {
        let snap = Snapshot { taken_at: 1, kind: ScanKind::Quick, system: Default::default(), sample: Default::default(), results: Default::default() };
        let health = crate::scoring::evaluate(&snap.sample, &snap.parsed(), &Default::default());
        let rep = build(&snap, &health, &[], &[], &ReportOptions::default());
        let path = std::env::temp_dir().join(format!("sp-test-{}.pdf", std::process::id()));
        to_pdf(&rep, &path).expect("pdf");
        let bytes = std::fs::read(&path).expect("read");
        assert!(bytes.starts_with(b"%PDF"));
        let _ = std::fs::remove_file(path);
    }
}
