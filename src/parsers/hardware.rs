//! lshw, dmidecode memory, lspci, lsusb, iw, nmcli and ps parsers.

use super::{kv_get, parse_dmidecode, parse_header_table, DmiSection};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// lshw
// ---------------------------------------------------------------------------

/// One row of `lshw -short`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LshwShortRow {
    pub path: String,
    pub device: String,
    pub class: String,
    pub description: String,
}

/// `lshw -short` uses fixed columns given by the header.
pub fn parse_lshw_short(text: &str) -> Vec<LshwShortRow> {
    let mut lines = text.lines();
    let Some(header) = lines.find(|l| l.starts_with("H/W path")) else { return Vec::new() };
    let (Some(d), Some(c), Some(desc)) = (header.find("Device"), header.find("Class"), header.find("Description")) else {
        return Vec::new();
    };
    let cut = |l: &str, s: usize, e: usize| l.get(s.min(l.len())..e.min(l.len())).unwrap_or("").trim().to_string();
    lines
        .filter(|l| !l.starts_with("====") && !l.trim().is_empty())
        .map(|l| LshwShortRow {
            path: cut(l, 0, d),
            device: cut(l, d, c),
            class: cut(l, c, desc),
            description: l.get(desc.min(l.len())..).unwrap_or("").trim().to_string(),
        })
        .collect()
}

/// A node of the `lshw` tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LshwNode {
    /// "core", "memory", "bank:0", ...
    pub id: String,
    /// "DISABLED" / "UNCLAIMED" when flagged by lshw.
    pub flags: String,
    pub props: Vec<(String, String)>,
    pub children: Vec<LshwNode>,
}

impl LshwNode {
    pub fn prop(&self, k: &str) -> Option<&str> {
        kv_get(&self.props, k)
    }

    /// Human title: description/product or the id.
    pub fn title(&self) -> String {
        let d = self.prop("description").or(self.prop("product")).unwrap_or(&self.id);
        format!("{} ({})", d, self.id)
    }

    /// The node's class: the id without ":N" suffix.
    pub fn class(&self) -> &str {
        self.id.split(':').next().unwrap_or(&self.id)
    }

    /// Depth-first iterator over self and descendants.
    pub fn walk(&self) -> Vec<&LshwNode> {
        let mut v = vec![self];
        for c in &self.children {
            v.extend(c.walk());
        }
        v
    }
}

/// Parse full `lshw` (or `lshw -C class`) output. Returns top-level nodes.
pub fn parse_lshw_tree(text: &str) -> Vec<LshwNode> {
    // (indent, node) stack; a node is closed when a line with <= indent appears.
    let mut roots: Vec<LshwNode> = Vec::new();
    let mut stack: Vec<(usize, LshwNode)> = Vec::new();
    let close_to = |stack: &mut Vec<(usize, LshwNode)>, roots: &mut Vec<LshwNode>, indent: usize| {
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            if let Some((_, n)) = stack.pop() {
                match stack.last_mut() {
                    Some((_, p)) => p.children.push(n),
                    None => roots.push(n),
                }
            }
        }
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("*-") {
            close_to(&mut stack, &mut roots, indent);
            let (id, flags) = rest.split_once(' ').unwrap_or((rest, ""));
            stack.push((indent, LshwNode { id: id.to_string(), flags: flags.trim().to_string(), ..Default::default() }));
        } else if stack.is_empty() && !t.contains(": ") {
            // The first line is the hostname (root node) in full mode.
            stack.push((indent, LshwNode { id: t.to_string(), ..Default::default() }));
        } else if let Some((k, v)) = t.split_once(": ") {
            if let Some((_, n)) = stack.last_mut() {
                n.props.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    close_to(&mut stack, &mut roots, 0);
    roots
}

// ---------------------------------------------------------------------------
// dmidecode memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemModule {
    pub locator: String,
    pub bank: String,
    /// `None` when the slot is empty.
    pub size: Option<String>,
    pub mem_type: String,
    pub form_factor: String,
    pub speed: String,
    pub configured_speed: String,
    pub manufacturer: String,
    pub part_number: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DmiMemory {
    pub max_capacity: Option<String>,
    pub slots: Option<u32>,
    pub error_correction: Option<String>,
    pub modules: Vec<MemModule>,
}

/// `dmidecode -t memory`
pub fn parse_dmidecode_memory(text: &str) -> DmiMemory {
    let secs = parse_dmidecode(text);
    let mut m = DmiMemory::default();
    for s in &secs {
        let g = |k: &str| kv_get(&s.fields, k).unwrap_or("").to_string();
        match s.title.as_str() {
            "Physical Memory Array" => {
                m.max_capacity = kv_get(&s.fields, "Maximum Capacity").map(str::to_string);
                m.slots = kv_get(&s.fields, "Number Of Devices").and_then(|v| v.parse().ok());
                m.error_correction = kv_get(&s.fields, "Error Correction Type").map(str::to_string);
            }
            "Memory Device" => {
                let size = g("Size");
                m.modules.push(MemModule {
                    locator: g("Locator"),
                    bank: g("Bank Locator"),
                    size: (!size.is_empty() && !size.contains("No Module")).then_some(size),
                    mem_type: g("Type"),
                    form_factor: g("Form Factor"),
                    speed: g("Speed"),
                    configured_speed: {
                        let c = g("Configured Memory Speed");
                        if c.is_empty() { g("Configured Clock Speed") } else { c }
                    },
                    manufacturer: g("Manufacturer"),
                    part_number: g("Part Number"),
                    fields: s.fields.clone(),
                });
            }
            _ => {}
        }
    }
    m
}

/// First dmidecode section with the given title ("System Information", ...).
pub fn dmi_section(text: &str, title: &str) -> Option<DmiSection> {
    parse_dmidecode(text).into_iter().find(|s| s.title == title)
}

// ---------------------------------------------------------------------------
// PCI / USB
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PciDevice {
    pub slot: String,
    pub class: String,
    pub vendor: String,
    pub device: String,
    pub subsystem: String,
    pub driver: String,
    pub modules: String,
    pub rev: String,
}

impl PciDevice {
    pub fn is_display(&self) -> bool {
        let c = self.class.to_lowercase();
        c.contains("vga") || c.contains("3d") || c.contains("display")
    }
}

/// `lspci -vmmnnk` records separated by blank lines.
pub fn parse_lspci_vmm(text: &str) -> Vec<PciDevice> {
    text.split("\n\n")
        .filter_map(|block| {
            let kv: Vec<(String, String)> = block
                .lines()
                .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect();
            let g = |k: &str| kv_get(&kv, k).unwrap_or("").to_string();
            let slot = g("Slot");
            (!slot.is_empty()).then(|| PciDevice {
                slot,
                class: g("Class"),
                vendor: g("Vendor"),
                device: g("Device"),
                subsystem: [g("SVendor"), g("SDevice")].join(" ").trim().to_string(),
                driver: g("Driver"),
                modules: g("Module"),
                rev: g("Rev"),
            })
        })
        .collect()
}

/// Plain `lspci`: (slot, class, description).
pub fn parse_lspci(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let (slot, rest) = l.split_once(' ')?;
            let (class, desc) = rest.split_once(": ")?;
            Some((slot.into(), class.into(), desc.into()))
        })
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsbDevice {
    pub bus: String,
    pub device: String,
    pub id: String,
    pub description: String,
}

/// `lsusb`
pub fn parse_lsusb(text: &str) -> Vec<UsbDevice> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Bus ")?;
            let (bus, rest) = rest.split_once(" Device ")?;
            let (dev, rest) = rest.split_once(": ID ")?;
            let (id, desc) = rest.split_once(' ').unwrap_or((rest, ""));
            Some(UsbDevice { bus: bus.into(), device: dev.into(), id: id.into(), description: desc.trim().into() })
        })
        .collect()
}

/// Split `lsusb -v` into per-device blocks keyed by "bus:device".
pub fn split_lsusb_verbose(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in text.lines() {
        if l.starts_with("Bus ") {
            let key = parse_lsusb(l).first().map(|d| format!("{}:{}", d.bus, d.device)).unwrap_or_default();
            out.push((key, String::new()));
        }
        if let Some((_, body)) = out.last_mut() {
            body.push_str(l);
            body.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WifiIface {
    pub phy: String,
    pub name: String,
    pub iftype: String,
    pub addr: String,
    pub ssid: String,
    pub channel: String,
    pub txpower: String,
}

/// `iw dev`
pub fn parse_iw_dev(text: &str) -> Vec<WifiIface> {
    let mut out: Vec<WifiIface> = Vec::new();
    let mut phy = String::new();
    for l in text.lines() {
        let t = l.trim();
        if !l.starts_with('\t') && t.starts_with("phy#") {
            phy = t.to_string();
        } else if let Some(n) = t.strip_prefix("Interface ") {
            out.push(WifiIface { phy: phy.clone(), name: n.to_string(), ..Default::default() });
        } else if let Some(i) = out.last_mut() {
            let (k, v) = t.split_once(' ').unwrap_or((t, ""));
            match k {
                "type" => i.iftype = v.into(),
                "addr" => i.addr = v.into(),
                "ssid" => i.ssid = v.into(),
                "channel" => i.channel = v.into(),
                "txpower" => i.txpower = v.into(),
                _ => {}
            }
        }
    }
    out
}

/// `nmcli device status` rows: (device, type, state, connection).
pub fn parse_nmcli(text: &str) -> Vec<[String; 4]> {
    parse_header_table(text)
        .into_iter()
        .map(|r| {
            let g = |k: &str| r.get(k).cloned().unwrap_or_default();
            [g("DEVICE"), g("TYPE"), g("STATE"), g("CONNECTION")]
        })
        .collect()
}

/// One listening/bound socket from `ss -tulpn`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub proto: String,
    pub state: String,
    pub address: String,
    pub port: u16,
    /// (process name, pid) of every owner `ss` could see.
    pub owners: Vec<(String, u32)>,
}

impl Port {
    /// Reachable from other machines (not loopback-only).
    pub fn exposed(&self) -> bool {
        !(self.address.starts_with("127.") || self.address == "[::1]" || self.address == "::1")
    }
}

/// `ss -tulpn` (with or without header).
pub fn parse_ss(text: &str) -> Vec<Port> {
    text.lines()
        .filter(|l| !l.starts_with("Netid"))
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() < 5 {
                return None;
            }
            let (addr, port) = c[4].rsplit_once(':')?;
            let addr = addr.split('%').next().unwrap_or(addr).to_string();
            let mut owners = Vec::new();
            if let Some(rest) = l.split_once("users:(").map(|x| x.1) {
                for part in rest.split("),(") {
                    let name = part.split('"').nth(1).unwrap_or("").to_string();
                    let pid = part.split("pid=").nth(1).and_then(|p| p.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|p| p.parse().ok());
                    if let Some(pid) = pid {
                        owners.push((name, pid));
                    }
                }
            }
            Some(Port { proto: c[0].into(), state: c[1].into(), address: addr, port: port.parse().ok()?, owners })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ps aux
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PsRow {
    pub user: String,
    pub pid: u32,
    pub cpu: f64,
    pub mem: f64,
    pub rss_kb: u64,
    pub stat: String,
    pub command: String,
}

/// `ps aux` (header skipped).
pub fn parse_ps_aux(text: &str) -> Vec<PsRow> {
    text.lines()
        .filter(|l| !l.starts_with("USER"))
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            if c.len() < 11 {
                return None;
            }
            Some(PsRow {
                user: c[0].into(),
                pid: c[1].parse().ok()?,
                cpu: c[2].parse().unwrap_or(0.0),
                mem: c[3].parse().unwrap_or(0.0),
                rss_kb: c[5].parse().unwrap_or(0),
                stat: c[7].into(),
                command: c[10..].join(" "),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSHW_SHORT: &str = "H/W path           Device          Class          Description
==============================================================
                                   system         HP Laptop 15s-gr0xxx (440L7PA#ACJ)
/0                                 bus            87D1
/0/4                               processor      AMD Ryzen 5 3500U with Radeon Vega Mobile Gfx
/0/100/1.2/0       wlan0           network        RTL8822CE 802.11ac PCIe Wireless Network Adapter
";

    #[test]
    fn lshw_short() {
        let r = parse_lshw_short(LSHW_SHORT);
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].class, "system");
        assert_eq!(r[3].device, "wlan0");
        assert_eq!(r[3].path, "/0/100/1.2/0");
        assert!(r[2].description.starts_with("AMD Ryzen"));
    }

    const LSHW: &str = "kali
    description: Notebook
    product: HP Laptop 15s-gr0xxx (440L7PA#ACJ)
    vendor: HP
  *-core
       description: Motherboard
       product: 87D1
     *-firmware
          description: BIOS
          version: F.31
     *-memory
          description: System Memory
          size: 8GiB
        *-bank:0
             description: SODIMM DDR4 Synchronous 2400 MHz
             size: 8GiB
        *-bank:1 UNCLAIMED
             description: [empty]
     *-cpu
          product: AMD Ryzen 5 3500U
  *-battery
       product: Primary
";

    #[test]
    fn lshw_tree() {
        let roots = parse_lshw_tree(LSHW);
        assert_eq!(roots.len(), 1);
        let root = &roots[0];
        assert_eq!(root.id, "kali");
        assert_eq!(root.prop("vendor"), Some("HP"));
        assert_eq!(root.children.len(), 2);
        let core = &root.children[0];
        assert_eq!(core.children.len(), 3);
        let mem = &core.children[1];
        assert_eq!(mem.children.len(), 2);
        assert_eq!(mem.children[1].flags, "UNCLAIMED");
        assert_eq!(mem.children[0].class(), "bank");
        assert_eq!(root.walk().len(), 8);
        // `lshw -C network` has no root line.
        let net = parse_lshw_tree("  *-network\n       description: Wireless interface\n       logical name: wlan0\n");
        assert_eq!(net.len(), 1);
        assert_eq!(net[0].prop("logical name"), Some("wlan0"));
    }

    const DMI_MEM: &str = "# dmidecode 3.6
Getting SMBIOS data from sysfs.
SMBIOS 3.2.0 present.

Handle 0x000E, DMI type 16, 23 bytes
Physical Memory Array
	Location: System Board Or Motherboard
	Error Correction Type: None
	Maximum Capacity: 64 GB
	Number Of Devices: 2

Handle 0x000F, DMI type 17, 92 bytes
Memory Device
	Size: 8 GB
	Form Factor: SODIMM
	Locator: DIMM 0
	Bank Locator: P0 CHANNEL A
	Type: DDR4
	Type Detail: Synchronous Unbuffered (Unregistered)
	Speed: 2400 MT/s
	Manufacturer: Samsung
	Serial Number: 12345678
	Part Number: M471A1K43CB1-CTD
	Configured Memory Speed: 2400 MT/s

Handle 0x0010, DMI type 17, 92 bytes
Memory Device
	Size: No Module Installed
	Locator: DIMM 1
	Type: Unknown
	Speed: Unknown
";

    #[test]
    fn dmidecode_memory() {
        let m = parse_dmidecode_memory(DMI_MEM);
        assert_eq!(m.max_capacity.as_deref(), Some("64 GB"));
        assert_eq!(m.slots, Some(2));
        assert_eq!(m.modules.len(), 2);
        assert_eq!(m.modules[0].size.as_deref(), Some("8 GB"));
        assert_eq!(m.modules[0].mem_type, "DDR4");
        assert_eq!(m.modules[0].configured_speed, "2400 MT/s");
        assert_eq!(m.modules[1].size, None);
    }

    #[test]
    fn dmidecode_lists() {
        let t = "Handle 0x0001, DMI type 1, 27 bytes\nSystem Information\n\tManufacturer: HP\n\tProduct Name: HP Laptop 15s\n\tSerial Number: 5CD0000\n\tCharacteristics:\n\t\tPCI is supported\n\t\tUSB legacy is supported\n";
        let s = dmi_section(t, "System Information").expect("section");
        assert_eq!(kv_get(&s.fields, "Product Name"), Some("HP Laptop 15s"));
        assert_eq!(s.fields.last().map(|f| f.1.as_str()), Some("PCI is supported, USB legacy is supported"));
    }

    #[test]
    fn pci_usb() {
        let v = "Slot:\t00:00.0\nClass:\tHost bridge [0600]\nVendor:\tAdvanced Micro Devices, Inc. [AMD] [1022]\nDevice:\tRaven Root Complex [15d0]\nSVendor:\tHewlett-Packard Company [103c]\nSDevice:\tDevice [87d1]\n\nSlot:\t04:00.0\nClass:\tVGA compatible controller [0300]\nVendor:\tAMD [1002]\nDevice:\tPicasso [15d8]\nRev:\tc1\nDriver:\tamdgpu\nModule:\tamdgpu\n";
        let d = parse_lspci_vmm(v);
        assert_eq!(d.len(), 2);
        assert!(d[1].is_display());
        assert_eq!(d[1].driver, "amdgpu");
        assert_eq!(d[0].subsystem, "Hewlett-Packard Company [103c] Device [87d1]");
        let p = parse_lspci("00:00.0 Host bridge: AMD Raven Root Complex\n");
        assert_eq!(p[0].1, "Host bridge");
        let u = parse_lsusb("Bus 001 Device 004: ID 1c4f:0002 SiGma Micro Keyboard\nBus 001 Device 005: ID 3554:fc03  2.4G Receiver\n");
        assert_eq!(u[0].id, "1c4f:0002");
        assert_eq!(u[1].description, "2.4G Receiver");
    }

    #[test]
    fn ss_ports() {
        let t = "Netid State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\nudp   UNCONN 0      0      127.0.0.53%lo:53      0.0.0.0:*    users:((\"systemd-resolve\",pid=512,fd=13))\ntcp   LISTEN 0      4096         [::]:22         [::]:*    users:((\"sshd\",pid=900,fd=3),(\"systemd\",pid=1,fd=50))\ntcp   LISTEN 0      128       0.0.0.0:631       0.0.0.0:*\n";
        let p = parse_ss(t);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].address, "127.0.0.53");
        assert_eq!(p[0].port, 53);
        assert!(!p[0].exposed());
        assert_eq!(p[1].owners, vec![("sshd".to_string(), 900), ("systemd".to_string(), 1)]);
        assert!(p[1].exposed());
        assert!(p[2].owners.is_empty());
    }

    #[test]
    fn network_and_ps() {
        let iw = "phy#0\n\tInterface wlan0\n\t\tifindex 3\n\t\taddr 74:12:b3:00:00:01\n\t\tssid home\n\t\ttype managed\n\t\tchannel 48 (5240 MHz), width: 80 MHz\n";
        let w = parse_iw_dev(iw);
        assert_eq!(w[0].name, "wlan0");
        assert_eq!(w[0].ssid, "home");
        assert_eq!(w[0].phy, "phy#0");
        let ps = "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND\nkali        1234 12.5  3.1 123456 98765 ?        Sl   10:00   1:23 /usr/bin/firefox -new-window\n";
        let r = parse_ps_aux(ps);
        assert_eq!(r[0].pid, 1234);
        assert_eq!(r[0].command, "/usr/bin/firefox -new-window");
    }
}
