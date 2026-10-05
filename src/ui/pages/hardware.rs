//! Hardware: lshw tree and summary, PCI and USB devices, motherboard and
//! system model (serial number masked unless explicitly revealed).

use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, wide, DataTable};
use crate::ui::widgets::{self, hbox, kv, label, vbox, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::parsers::hardware::LshwNode;
use systemhealthcheck::privacy::{mask_value, Privacy};

const IDS: &[&str] = &[
    "lshw-short",
    "lshw",
    "lspci",
    "lspci-vmm",
    "lsusb",
    "lsusb-v",
    "dmidecode-baseboard",
    "dmidecode-board-mfr",
    "dmidecode-board-product",
    "dmidecode-system",
    "dmidecode-product",
    "dmidecode-version",
    "dmidecode-serial",
];

fn node_row(n: &LshwNode, privacy: &Privacy, depth: usize) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(&gtk::glib::markup_escape_text(&n.title()));
    let sub = [n.prop("product"), n.prop("vendor"), n.prop("logical name")].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    row.set_subtitle(&gtk::glib::markup_escape_text(&sub));
    if !n.flags.is_empty() {
        row.add_suffix(&widgets::badge(&n.flags, "status-unknown"));
    }
    if !n.props.is_empty() {
        let k = KvGrid::new();
        k.set(&n.props.iter().map(|(a, b)| (a.clone(), privacy.redact(&format!("{a}: {b}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect::<Vec<_>>());
        k.grid.set_margin_start(12);
        k.grid.set_margin_end(12);
        k.grid.set_margin_top(6);
        k.grid.set_margin_bottom(6);
        row.add_row(&k.grid);
    }
    if depth < 12 {
        for c in &n.children {
            row.add_row(&node_row(c, privacy, depth + 1));
        }
    }
    row
}

fn substack_page(stack: &adw::ViewStack, name: &str, title: &str, icon: &str, child: &impl IsA<gtk::Widget>) {
    stack.add_titled_with_icon(child, Some(name), title, icon);
}

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Hardware", "Everything lshw, lspci, lsusb and dmidecode report", IDS);
    let stack = adw::ViewStack::new();
    stack.set_vhomogeneous(false);
    let switcher = adw::ViewSwitcher::new();
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Start);
    page.overview.append(&switcher);
    page.overview.append(&stack);

    // Tree
    let tree_box = vbox(6);
    let tree_list = gtk::ListBox::new();
    tree_list.add_css_class("boxed-list");
    tree_list.set_selection_mode(gtk::SelectionMode::None);
    tree_box.append(&label("Expand nodes to see every property lshw reports.", &["dim-label"]));
    tree_box.append(&tree_list);
    substack_page(&stack, "tree", "Hardware tree", "view-list-symbolic", &tree_box);

    // Summary
    let summary = DataTable::new(&[col("H/W path"), col("Device"), col("Class"), wide("Description")], true, 0, 420);
    substack_page(&stack, "short", "Summary", "view-grid-symbolic", &summary.widget);

    // PCI
    let pci = DataTable::new(&[col("Bus"), col("Device ID"), col("Class"), wide("Vendor"), wide("Device"), col("Driver")], true, 0, 420);
    substack_page(&stack, "pci", "PCI", "preferences-system-devices-symbolic", &pci.widget);

    // USB
    let usb_box = vbox(6);
    let usb = DataTable::new(&[col("Bus"), col("Device"), col("ID"), wide("Vendor / product")], true, 0, 220);
    let usb_detail = gtk::TextView::new();
    usb_detail.set_editable(false);
    usb_detail.set_monospace(true);
    let usb_scroll = gtk::ScrolledWindow::new();
    usb_scroll.set_child(Some(&usb_detail));
    usb_scroll.set_min_content_height(260);
    usb_scroll.add_css_class("card");
    usb_box.append(&usb.widget);
    usb_box.append(&label("Activate a device (double-click / Enter) to show its `lsusb -v` descriptor.", &["dim-label"]));
    usb_box.append(&usb_scroll);
    substack_page(&stack, "usb", "USB", "media-removable-symbolic", &usb_box);

    // Board & model
    let bm = widgets::card_grid(1, 2);
    let board = Card::new("Motherboard", "preferences-system-devices-symbolic");
    let board_kv = KvGrid::new();
    board.body.append(&board_kv.grid);
    let model = Card::new("System model", "computer-symbolic");
    let model_kv = KvGrid::new();
    model.body.append(&model_kv.grid);
    let serial_row = hbox(8);
    let serial_lbl = label("", &["mono"]);
    let serial_btn = gtk::Button::with_label("Show serial number…");
    serial_row.append(&label("Serial number", &["dim-label"]));
    serial_row.append(&serial_lbl);
    serial_row.append(&serial_btn);
    model.body.append(&serial_row);
    model.body.append(&label("The serial number is sensitive. It is masked by default and never included in reports unless you opt in when exporting.", &["caption", "dim-label"]));
    bm.append(&board.root);
    bm.append(&model.root);
    substack_page(&stack, "board", "Motherboard & model", "computer-symbolic", &bm);

    let st = state.clone();
    let ud = usb_detail.clone();
    usb.connect_activate(move |row| {
        let key = format!("{}:{}", row.first().cloned().unwrap_or_default(), row.get(1).cloned().unwrap_or_default());
        let p = st.parsed();
        let text = p.usb_details.iter().find(|(k, _)| *k == key).map(|(_, v)| st.privacy().redact(v)).unwrap_or_else(|| {
            "No verbose descriptor collected. Run diagnostics on this page (runs `lsusb -v`).".into()
        });
        ud.buffer().set_text(&text);
    });

    let st = state.clone();
    let sl = serial_lbl.clone();
    let sb = serial_btn.clone();
    widgets::bind(state, &page.overview, |e| widgets::on_data(e) || matches!(e, Event::Settings), move || {
        let p = st.parsed();
        let privacy = st.privacy();
        while let Some(c) = tree_list.first_child() {
            tree_list.remove(&c);
        }
        if p.lshw.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title("No lshw data yet");
            r.set_subtitle(&st.result("lshw").map_or("`sudo lshw` needs administrator privileges — click “Run diagnostics”.".into(), |o| format!("{}: {}", o.status.label(), o.detail)));
            tree_list.append(&r);
        }
        for n in &p.lshw {
            let row = node_row(n, &privacy, 0);
            row.set_expanded(true);
            tree_list.append(&row);
        }
        summary.set_rows(p.lshw_short.iter().map(|r| vec![r.path.clone(), r.device.clone(), r.class.clone(), r.description.clone()]).collect());
        pci.set_rows(
            p.pci
                .iter()
                .map(|d| {
                    let ids = format!("{}:{}", bracket_id(&d.vendor), bracket_id(&d.device));
                    vec![d.slot.clone(), ids, d.class.clone(), d.vendor.clone(), d.device.clone(), d.driver.clone()]
                })
                .collect(),
        );
        let sysusb = sys_usb_names();
        usb.set_rows(
            p.usb
                .iter()
                .map(|d| {
                    let name = sysusb.iter().find(|(id, _)| *id == d.id).map(|(_, n)| n.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| d.description.clone());
                    vec![d.bus.clone(), d.device.clone(), d.id.clone(), name]
                })
                .collect(),
        );
        let s = &st.system;
        let one = |id: &str| st.result(id).and_then(|o| o.text().map(|t| t.trim().to_string())).unwrap_or_default();
        board_kv.set(&match &p.baseboard {
            Some(b) => b.fields.iter().map(|(k, v)| (k.clone(), privacy.redact(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect(),
            None => kv([
                ("Manufacturer", if one("dmidecode-board-mfr").is_empty() { s.board_vendor.clone() } else { one("dmidecode-board-mfr") }),
                ("Product", if one("dmidecode-board-product").is_empty() { s.board_name.clone() } else { one("dmidecode-board-product") }),
                ("Version", s.board_version.clone()),
                ("Note", "Full baseboard details need `sudo dmidecode -t baseboard`.".into()),
            ]),
        });
        model_kv.set(&match &p.system_dmi {
            Some(sec) => sec.fields.iter().filter(|(k, _)| k != "Serial Number").map(|(k, v)| (k.clone(), privacy.redact(&format!("{k}: {v}")).split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())).collect(),
            None => kv([
                ("Manufacturer", s.vendor.clone()),
                ("Product", if one("dmidecode-product").is_empty() { s.product.clone() } else { one("dmidecode-product") }),
                ("Version", if one("dmidecode-version").is_empty() { s.product_version.clone() } else { one("dmidecode-version") }),
            ]),
        });
        let serial = p.serial.clone().or_else(|| p.system_dmi.as_ref().and_then(|d| systemhealthcheck::parsers::kv_get(&d.fields, "Serial Number").map(str::to_string)));
        match serial {
            Some(sn) if st.reveal_serial.get() || !st.settings.borrow().mask_serial => {
                sl.set_text(&sn);
                sb.set_label("Hide");
            }
            Some(sn) => {
                sl.set_text(&mask_value(&sn));
                sb.set_label("Show serial number…");
            }
            None => {
                sl.set_text("Not collected (needs administrator)");
                sb.set_sensitive(false);
            }
        }
        if p.serial.is_some() {
            sb.set_sensitive(true);
        }
    });

    let st = state.clone();
    serial_btn.connect_clicked(move |_| {
        if st.reveal_serial.get() {
            st.reveal_serial.set(false);
            st.emit(Event::Settings);
            return;
        }
        let d = adw::AlertDialog::new(
            Some("Show serial number?"),
            Some("The serial number uniquely identifies this machine (warranty, ownership). Avoid showing it while screen sharing or posting screenshots. It will be shown in this window and in raw output until you hide it or restart the app."),
        );
        d.add_response("cancel", "Cancel");
        d.add_response("show", "Show");
        d.set_response_appearance("show", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let st2 = st.clone();
        d.connect_response(None, move |_, r| {
            if r == "show" {
                st2.reveal_serial.set(true);
                st2.emit(Event::Settings);
            }
        });
        let win = st.window.borrow().clone();
        d.present(win.as_ref());
    });
    page.root.upcast()
}

/// "Advanced Micro Devices, Inc. [AMD] [1022]" -> "1022"
fn bracket_id(s: &str) -> String {
    s.rsplit_once('[').map(|(_, r)| r.trim_end_matches(']').to_string()).unwrap_or_default()
}

/// (vendor:product id, "Manufacturer Product") from /sys/bus/usb/devices.
fn sys_usb_names() -> Vec<(String, String)> {
    let Ok(dir) = std::fs::read_dir("/sys/bus/usb/devices") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let p = e.path();
            let r = |f: &str| systemhealthcheck::util::read_trim(p.join(f));
            let id = format!("{}:{}", r("idVendor")?, r("idProduct")?);
            Some((id, format!("{} {}", r("manufacturer").unwrap_or_default(), r("product").unwrap_or_default()).trim().to_string()))
        })
        .collect()
}
