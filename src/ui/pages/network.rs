//! Network: native interface dashboard plus lshw -C network, iw dev and nmcli.

use crate::ui::state::{AppState, Event};
use crate::ui::table::{col, num, wide, DataTable};
use crate::ui::widgets::{self, kv, label, Card, KvGrid, Page};
use adw::prelude::*;
use std::rc::Rc;
use systemhealthcheck::util::fmt_bytes;

const IDS: &[&str] = &["lshw-network", "iw-dev", "nmcli-status"];

pub fn build(state: &Rc<AppState>) -> gtk::Widget {
    let page = Page::new(state, "Network", "Adapters, interfaces, Wi-Fi and NetworkManager", IDS);
    let show_sensitive = gtk::CheckButton::with_label("Show addresses");
    show_sensitive.set_tooltip_text(Some("MAC and IP addresses are hidden by default"));
    page.actions.prepend(&show_sensitive);

    page.overview.append(&widgets::section("Interfaces"));
    let ifaces = DataTable::new(&[col("Interface"), col("Type"), col("State"), col("Driver"), num("Speed"), num("↓ /s"), num("↑ /s"), num("Received"), num("Sent"), wide("Addresses")], true, 0, 0);
    page.overview.append(&ifaces.widget);

    let grid = widgets::card_grid(1, 3);
    page.overview.append(&grid);
    let nm = Card::new("NetworkManager (nmcli device status)", "network-wired-symbolic");
    let nm_table = DataTable::new(&[col("Device"), col("Type"), col("State"), wide("Connection")], false, 0, 0);
    nm.body.append(&nm_table.widget);
    page.overview.append(&nm.root);

    let t = ifaces.clone();
    let show = show_sensitive.clone();
    state.subscribe(move |e| {
        let Event::Sample(s) = e else { return };
        if !t.widget.is_mapped() {
            return;
        }
        let reveal = show.is_active();
        t.set_rows(
            s.net
                .iter()
                .map(|n| {
                    vec![
                        n.name.clone(),
                        n.kind.clone(),
                        n.state.clone(),
                        n.driver.clone(),
                        n.speed_mbps.map_or("—".into(), |v| format!("{v} Mb/s")),
                        fmt_bytes(n.rx_rate as u64),
                        fmt_bytes(n.tx_rate as u64),
                        fmt_bytes(n.rx_bytes),
                        fmt_bytes(n.tx_bytes),
                        if reveal { format!("{} {}", n.mac, n.addrs.join(", ")) } else { "hidden".into() },
                    ]
                })
                .collect(),
        );
    });

    let st = state.clone();
    let show = show_sensitive.clone();
    let refresh = move || {
        grid.remove_all();
        let p = st.parsed();
        let reveal = show.is_active();
        let privacy = systemhealthcheck::privacy::Privacy { mask_network: !reveal, ..st.privacy() };
        for w in &p.wifi {
            let c = Card::new(&format!("Wi-Fi {}", w.name), "network-wireless-symbolic");
            let k = KvGrid::new();
            k.set(&kv([
                ("PHY", w.phy.clone()),
                ("Type", w.iftype.clone()),
                ("SSID", if reveal { w.ssid.clone() } else { "hidden".into() }),
                ("Channel", w.channel.clone()),
                ("TX power", w.txpower.clone()),
                ("MAC", privacy.redact(&w.addr)),
            ]));
            c.body.append(&k.grid);
            grid.append(&c.root);
        }
        for n in p.network_hw.iter().flat_map(|r| r.walk()).filter(|n| n.class() == "network") {
            let c = Card::new(&n.prop("product").unwrap_or(&n.id).to_string(), "network-wired-symbolic");
            let k = KvGrid::new();
            k.set(&n.props.iter().map(|(a, b)| (a.clone(), privacy.redact(b))).collect::<Vec<_>>());
            c.body.append(&k.grid);
            grid.append(&c.root);
        }
        if p.network_hw.is_empty() {
            let c = Card::new("Network hardware (lshw -C network)", "network-wired-symbolic");
            c.body.append(&label(
                &st.result("lshw-network").map_or("Not collected yet — requires administrator privileges.".into(), |o| format!("{}: {}", o.status.label(), o.detail)),
                &["dim-label"],
            ));
            grid.append(&c.root);
        }
        nm_table.set_rows(p.nmcli.iter().map(|r| vec![r[0].clone(), r[1].clone(), r[2].clone(), if reveal { r[3].clone() } else if r[3] == "--" { "--".into() } else { "hidden".into() }]).collect());
    };
    let refresh = Rc::new(refresh);
    let r2 = refresh.clone();
    show_sensitive.connect_toggled(move |_| r2());
    widgets::bind(state, &page.overview, widgets::on_data, move || refresh());
    page.root.upcast()
}
