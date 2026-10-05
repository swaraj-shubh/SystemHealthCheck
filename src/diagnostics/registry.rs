//! The allowlisted command registry.
//!
//! Every command from `Laptop_Command_Checklist.pdf` is listed here with its
//! original string preserved in `original`. Nothing is ever executed through a
//! shell: each entry carries an argv-based [`Step`] plus native post-processing
//! [`Filter`]s that stand in for `| grep`, `| head`, `| tail` and `| sort -h`.
//! Only entries in this table can be run, and only parameters declared in
//! `params` can be substituted (validated in [`super::runner::build_argv`]).

use serde::{Deserialize, Serialize};

/// How risky running a command is. Drives confirmation and privilege prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Danger {
    ReadOnly,
    PrivilegedRead,
    ModifySystem,
    Destructive,
}

impl Danger {
    pub fn label(self) -> &'static str {
        match self {
            Danger::ReadOnly => "Read-only",
            Danger::PrivilegedRead => "Privileged read",
            Danger::ModifySystem => "Modifies system",
            Danger::Destructive => "Destructive",
        }
    }
}

/// Command Center categories (spec §29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    System,
    Cpu,
    Memory,
    Gpu,
    Storage,
    Smart,
    Thermals,
    Battery,
    Boot,
    Kernel,
    Pci,
    Usb,
    Network,
    Processes,
    Benchmark,
    Stress,
    Packages,
}

impl Category {
    pub const ALL: [Category; 17] = [
        Category::System,
        Category::Cpu,
        Category::Memory,
        Category::Gpu,
        Category::Storage,
        Category::Smart,
        Category::Thermals,
        Category::Battery,
        Category::Boot,
        Category::Kernel,
        Category::Pci,
        Category::Usb,
        Category::Network,
        Category::Processes,
        Category::Benchmark,
        Category::Stress,
        Category::Packages,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::System => "System",
            Category::Cpu => "CPU",
            Category::Memory => "Memory",
            Category::Gpu => "GPU",
            Category::Storage => "Storage",
            Category::Smart => "SMART",
            Category::Thermals => "Thermals",
            Category::Battery => "Battery",
            Category::Boot => "Boot",
            Category::Kernel => "Kernel",
            Category::Pci => "PCI",
            Category::Usb => "USB",
            Category::Network => "Network",
            Category::Processes => "Processes",
            Category::Benchmark => "Benchmark",
            Category::Stress => "Stress",
            Category::Packages => "Packages",
        }
    }
}

/// What kind of output the command produces (for display hints).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    Text,
    KeyValue,
    Table,
    Tree,
}

/// The executable part of a command.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    /// Run argv directly (argv[0] resolved via PATH + sbin dirs). `{key}`
    /// placeholders are substituted with validated parameters.
    Run(&'static [&'static str]),
    /// `cat FILE` implemented as a native file read.
    Read(&'static str),
    /// `upower -i $(upower -e | grep BAT)` as two argv invocations.
    UpowerBattery,
    /// Interactive full-screen tools (`htop`) that cannot run inside a GUI;
    /// the string names the native page that replaces them.
    Interactive(&'static str),
}

/// Native replacement for shell pipeline stages.
#[derive(Debug, Clone, Copy)]
pub enum Filter {
    /// `grep -E 'a|b'` (literal alternatives), optional `-i` and `-A n`.
    Grep {
        any_of: &'static [&'static str],
        ignore_case: bool,
        after: usize,
    },
    Head(usize),
    Tail(usize),
    /// `sort -h` on the first column (human-readable sizes).
    SortHuman,
}

/// Kinds of user-editable parameters; each is strictly validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// `/dev/nvmeN`
    NvmeController,
    /// `/dev/nvmeNnM`, `/dev/sdX`, `/dev/mmcblkN`, ...
    BlockDevice,
    /// Positive integer within an inclusive range.
    Int { min: u32, max: u32 },
    /// Package name from [`crate::packages::INSTALLABLE`].
    Package,
    /// systemd unit name (`name.service` / `name.socket`).
    Unit,
}

#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
}

/// One allowlisted command.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The command string exactly as written in the checklist (or a note for
    /// internal helpers that are not from the checklist).
    pub original: &'static str,
    pub category: Category,
    pub requires_sudo: bool,
    pub danger: Danger,
    pub timeout_s: u64,
    /// Name of the parser that turns the output into structured data.
    pub parser: Option<&'static str>,
    pub output_type: OutputType,
    pub step: Step,
    pub filters: &'static [Filter],
    pub params: &'static [Param],
    /// When set, the user must confirm this text before running.
    pub warning: Option<&'static str>,
    /// Output contains sensitive identifiers (serial numbers).
    pub sensitive: bool,
    /// `true` if the command appears in the checklist PDF.
    pub from_checklist: bool,
}

const fn base(id: &'static str, name: &'static str, original: &'static str, category: Category, step: Step) -> CommandSpec {
    CommandSpec {
        id,
        name,
        description: "",
        original,
        category,
        requires_sudo: false,
        danger: Danger::ReadOnly,
        timeout_s: 30,
        parser: None,
        output_type: OutputType::Text,
        step,
        filters: &[],
        params: &[],
        warning: None,
        sensitive: false,
        from_checklist: true,
    }
}

impl CommandSpec {
    const fn desc(mut self, d: &'static str) -> Self {
        self.description = d;
        self
    }
    const fn sudo(mut self) -> Self {
        self.requires_sudo = true;
        if matches!(self.danger, Danger::ReadOnly) {
            self.danger = Danger::PrivilegedRead;
        }
        self
    }
    const fn danger(mut self, d: Danger) -> Self {
        self.danger = d;
        self
    }
    const fn timeout(mut self, s: u64) -> Self {
        self.timeout_s = s;
        self
    }
    const fn parser(mut self, p: &'static str, t: OutputType) -> Self {
        self.parser = Some(p);
        self.output_type = t;
        self
    }
    const fn filters(mut self, f: &'static [Filter]) -> Self {
        self.filters = f;
        self
    }
    const fn params(mut self, p: &'static [Param]) -> Self {
        self.params = p;
        self
    }
    const fn warn(mut self, w: &'static str) -> Self {
        self.warning = Some(w);
        self
    }
    const fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }
    const fn internal(mut self) -> Self {
        self.from_checklist = false;
        self
    }

    /// Commands the user must explicitly confirm.
    pub fn needs_confirmation(&self) -> bool {
        self.warning.is_some() || self.danger >= Danger::ModifySystem
    }

    /// Long-running / heavy commands that are never part of automatic scans.
    pub fn is_manual_only(&self) -> bool {
        self.needs_confirmation() || matches!(self.step, Step::Interactive(_))
    }
}

const NVME_CTRL: &[Param] = &[Param { key: "ctrl", label: "NVMe controller", kind: ParamKind::NvmeController }];
const BLOCK_DEV: &[Param] = &[Param { key: "dev", label: "Block device", kind: ParamKind::BlockDevice }];
const BENCH_PARAMS: &[Param] = &[
    Param { key: "threads", label: "Threads", kind: ParamKind::Int { min: 1, max: 1024 } },
    Param { key: "secs", label: "Duration (s)", kind: ParamKind::Int { min: 1, max: 3600 } },
];
const STRESS_PARAMS: &[Param] = &[
    Param { key: "workers", label: "CPU workers", kind: ParamKind::Int { min: 1, max: 1024 } },
    Param { key: "secs", label: "Duration (s)", kind: ParamKind::Int { min: 1, max: 3600 } },
];
const SWAPPINESS_PARAM: &[Param] = &[Param { key: "value", label: "Swappiness", kind: ParamKind::Int { min: 0, max: 200 } }];
const PKG_PARAM: &[Param] = &[Param { key: "pkg", label: "Package", kind: ParamKind::Package }];
const PID_PARAM: &[Param] = &[Param { key: "pid", label: "PID", kind: ParamKind::Int { min: 2, max: 4_194_304 } }];
const UNIT_PARAM: &[Param] = &[Param { key: "unit", label: "Unit", kind: ParamKind::Unit }];

const GREP_GPU: &[&str] = &["vga", "3d", "display"];

const STRESS_WARNING: &str = "This test intentionally places heavy load on the CPU.\nTemperature may increase significantly.\nStop the test if abnormal temperatures or behavior occur.";
const APT_WARNING: &str = "This installs software as root using APT. It needs administrator authentication and network access.";

use Category as C;
use Danger as D;
use OutputType as O;

/// The full registry. Order is the display order in the Command Center.
pub static REGISTRY: &[CommandSpec] = &[
    // 1. Full hardware overview
    base("lshw-short", "Hardware list (short)", "sudo lshw -short", C::System, Step::Run(&["lshw", "-short"]))
        .desc("One line per detected device: path, class and description.").sudo().timeout(120).parser("lshw_short", O::Table),
    base("lshw", "Hardware details", "sudo lshw", C::System, Step::Run(&["lshw"]))
        .desc("Complete hardware tree with capabilities, drivers and firmware.").sudo().timeout(180).parser("lshw_tree", O::Tree).sensitive(),
    // 2. CPU
    base("lscpu", "CPU details", "lscpu", C::Cpu, Step::Run(&["lscpu"]))
        .desc("CPU architecture, topology, caches and flags.").parser("lscpu", O::KeyValue),
    base("lscpu-filtered", "CPU summary (filtered)", "lscpu | grep -E 'Model name|CPU\\(s\\)|Core|Thread|MHz|Cache'", C::Cpu, Step::Run(&["lscpu"]))
        .desc("Only model, CPU counts, cores, threads, MHz and caches.")
        .filters(&[Filter::Grep { any_of: &["Model name", "CPU(s)", "Core", "Thread", "MHz", "Cache"], ignore_case: false, after: 0 }]),
    base("cpu-governor", "Current CPU governor", "cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor", C::Cpu,
        Step::Read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")).desc("Frequency scaling policy in use on cpu0."),
    base("cpu-governors", "Available CPU governors", "cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors", C::Cpu,
        Step::Read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")).desc("Governors the cpufreq driver supports."),
    base("cpupower", "CPU frequency information", "cpupower frequency-info", C::Cpu, Step::Run(&["cpupower", "frequency-info"]))
        .desc("Driver, hardware limits, boost state and current policy.").parser("cpupower", O::KeyValue),
    base("watch-cpu-mhz", "Watch CPU frequency (snapshot)", "watch -n 1 \"grep 'cpu MHz' /proc/cpuinfo\"", C::Cpu, Step::Read("/proc/cpuinfo"))
        .desc("Single snapshot of per-core MHz. Continuous monitoring is native: see the CPU and Live Monitor pages.")
        .filters(&[Filter::Grep { any_of: &["cpu MHz"], ignore_case: false, after: 0 }]),
    // 3. RAM
    base("free", "Memory usage", "free -h", C::Memory, Step::Run(&["free", "-h"])).desc("RAM and swap totals.").parser("native:/proc/meminfo", O::Table),
    base("dmidecode-memory-long", "Physical RAM (dmidecode --type memory)", "sudo dmidecode --type memory", C::Memory,
        Step::Run(&["dmidecode", "--type", "memory"])).desc("Firmware-reported memory array and modules.").sudo().parser("dmidecode_memory", O::KeyValue).sensitive(),
    base("dmidecode-memory", "Physical RAM (dmidecode -t memory)", "sudo dmidecode -t memory", C::Memory,
        Step::Run(&["dmidecode", "-t", "memory"])).desc("Same as above, short form.").sudo().parser("dmidecode_memory", O::KeyValue).sensitive(),
    base("dmidecode-memory-filtered", "Physical RAM (filtered)",
        "sudo dmidecode --type memory | grep -E \"Maximum Capacity|Number Of Devices|Size:|Type:|Speed:|Configured Memory Speed\"", C::Memory,
        Step::Run(&["dmidecode", "--type", "memory"])).desc("Capacity, slot count, module sizes, types and speeds.").sudo()
        .filters(&[Filter::Grep { any_of: &["Maximum Capacity", "Number Of Devices", "Size:", "Type:", "Speed:", "Configured Memory Speed"], ignore_case: false, after: 0 }]),
    // 4. Swap
    base("swapon", "Swap devices", "swapon --show", C::Memory, Step::Run(&["swapon", "--show"])).desc("Active swap areas.").parser("native:/proc/swaps", O::Table),
    base("swappiness", "Swappiness", "cat /proc/sys/vm/swappiness", C::Memory, Step::Read("/proc/sys/vm/swappiness"))
        .desc("Kernel preference for swapping (0-200)."),
    // 5. Processes
    base("htop", "Interactive monitor (htop)", "htop", C::Processes, Step::Interactive("Processes"))
        .desc("htop is interactive; SystemHealthCheck provides the same view natively on the Processes page."),
    base("ps-cpu-15", "CPU-heavy processes", "ps aux --sort=-%cpu | head -15", C::Processes, Step::Run(&["ps", "aux", "--sort=-%cpu"]))
        .desc("Top 14 processes by CPU.").filters(&[Filter::Head(15)]).parser("ps_aux", O::Table),
    base("ps-mem-15", "Memory-heavy processes", "ps aux --sort=-%mem | head -15", C::Processes, Step::Run(&["ps", "aux", "--sort=-%mem"]))
        .desc("Top 14 processes by memory.").filters(&[Filter::Head(15)]).parser("ps_aux", O::Table),
    base("ps-mem-20", "Memory-heavy processes (20)", "ps aux --sort=-%mem | head -20", C::Processes, Step::Run(&["ps", "aux", "--sort=-%mem"]))
        .desc("Top 19 processes by memory.").filters(&[Filter::Head(20)]).parser("ps_aux", O::Table),
    // 6. GPU
    base("lspci-gpu", "GPU identification", "lspci | grep -Ei 'vga|3d|display'", C::Gpu, Step::Run(&["lspci"]))
        .desc("PCI display controllers.").filters(&[Filter::Grep { any_of: GREP_GPU, ignore_case: true, after: 0 }]),
    base("lspci-gpu-driver", "GPU + driver", "lspci -nnk | grep -A 3 -Ei 'vga|3d|display'", C::Gpu, Step::Run(&["lspci", "-nnk"]))
        .desc("Display controllers with vendor/device IDs and kernel driver.").filters(&[Filter::Grep { any_of: GREP_GPU, ignore_case: true, after: 3 }]),
    base("glxinfo", "OpenGL information", "glxinfo -B", C::Gpu, Step::Run(&["glxinfo", "-B"]))
        .desc("OpenGL renderer, version and video memory.").parser("glxinfo", O::KeyValue),
    // 7. Storage / NVMe
    base("lsblk", "Block devices", "lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS,MODEL", C::Storage,
        Step::Run(&["lsblk", "-o", "NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS,MODEL"])).desc("Disks, partitions, filesystems and mount points.").parser("native:/sys/block", O::Tree),
    base("nvme-list", "NVMe drives", "sudo nvme list", C::Storage, Step::Run(&["nvme", "list"]))
        .desc("NVMe namespaces with model, capacity and firmware.").sudo().parser("nvme_list", O::Table).sensitive(),
    base("nvme-smart-log", "NVMe health", "sudo nvme smart-log /dev/nvme0", C::Smart, Step::Run(&["nvme", "smart-log", "{ctrl}"]))
        .desc("NVMe SMART / health log: wear, temperature, errors, power-on hours.").sudo().params(NVME_CTRL).parser("nvme_smart_log", O::KeyValue),
    base("hdparm", "Disk read performance", "sudo hdparm -Tt /dev/nvme0n1", C::Storage, Step::Run(&["hdparm", "-Tt", "{dev}"]))
        .desc("Cached and buffered sequential read timing.").sudo().params(BLOCK_DEV).timeout(120).parser("hdparm", O::KeyValue)
        .warn("hdparm reads from the disk for several seconds. Results are affected by other disk activity."),
    base("df", "Filesystem usage", "df -h", C::Storage, Step::Run(&["df", "-h"])).desc("Size, used and available space per filesystem.").parser("native:statvfs", O::Table),
    base("du-root", "Large top-level directories", "sudo du -xh / --max-depth=1 2>/dev/null | sort -h", C::Storage,
        Step::Run(&["du", "-xh", "/", "--max-depth=1"])).desc("Recursive scan of the root filesystem.").sudo().timeout(900)
        .filters(&[Filter::SortHuman]).parser("du", O::Table)
        .warn("This recursively scans the whole root filesystem and can take several minutes of heavy disk I/O."),
    base("du-home", "Large directories in home", "du -xh ~ --max-depth=1 2>/dev/null | sort -h", C::Storage,
        Step::Run(&["du", "-xh", "{home}", "--max-depth=1"])).desc("Recursive scan of your home directory.").timeout(900)
        .filters(&[Filter::SortHuman]).parser("du", O::Table)
        .warn("This recursively scans your home directory and may take a while."),
    // 8. SMART
    base("smartctl-a", "SMART (basic)", "sudo smartctl -a /dev/nvme0n1", C::Smart, Step::Run(&["smartctl", "-a", "{dev}"]))
        .desc("SMART health, attributes and error log.").sudo().params(BLOCK_DEV).parser("smartctl", O::KeyValue).sensitive(),
    base("smartctl-x", "SMART (extended)", "sudo smartctl -x /dev/nvme0n1", C::Smart, Step::Run(&["smartctl", "-x", "{dev}"]))
        .desc("All SMART and device information.").sudo().params(BLOCK_DEV).parser("smartctl", O::KeyValue).sensitive(),
    // 9. Thermals
    base("sensors-detect", "Detect sensors", "sudo sensors-detect", C::Thermals, Step::Run(&["sensors-detect", "--auto"]))
        .desc("Probes buses for hardware monitoring chips (runs with --auto, accepting safe defaults).").sudo().danger(D::ModifySystem).timeout(300)
        .warn("sensors-detect probes I2C/SMBus and Super-I/O hardware. On rare hardware probing can hang the machine. It may also write module configuration under /etc."),
    base("sensors", "Temperatures & fans", "sensors", C::Thermals, Step::Run(&["sensors"])).desc("lm-sensors readings.").parser("sensors", O::KeyValue),
    base("watch-sensors", "Watch sensors (snapshot)", "watch -n 1 sensors", C::Thermals, Step::Run(&["sensors"]))
        .desc("Single snapshot. Continuous monitoring is native: see the Thermals and Live Monitor pages.").parser("sensors", O::KeyValue),
    // 10. Battery
    base("upower-battery", "Battery information", "upower -i $(upower -e | grep BAT)", C::Battery, Step::UpowerBattery)
        .desc("UPower battery state, energy and capacity.").parser("upower", O::KeyValue),
    // 11. Boot
    base("systemd-analyze", "Boot time", "systemd-analyze", C::Boot, Step::Run(&["systemd-analyze"])).desc("Firmware, loader, kernel and userspace time.").parser("systemd_analyze", O::Text),
    base("systemd-blame", "Boot services by time", "systemd-analyze blame", C::Boot, Step::Run(&["systemd-analyze", "blame", "--no-pager"]))
        .desc("Units ordered by initialization time.").parser("blame", O::Table),
    base("systemd-blame-20", "Boot services top 20", "systemd-analyze blame | head -20", C::Boot, Step::Run(&["systemd-analyze", "blame", "--no-pager"]))
        .desc("20 slowest units.").filters(&[Filter::Head(20)]).parser("blame", O::Table),
    base("systemd-critical-chain", "Critical boot chain", "systemd-analyze critical-chain", C::Boot, Step::Run(&["systemd-analyze", "critical-chain", "--no-pager"]))
        .desc("Time-critical chain of units.").parser("critical_chain", O::Tree),
    base("systemctl-enabled", "Enabled services", "systemctl list-unit-files --state=enabled", C::Boot,
        Step::Run(&["systemctl", "list-unit-files", "--state=enabled", "--no-pager"])).desc("Unit files enabled to start.").parser("unit_files", O::Table),
    // 12. Linux / kernel
    base("uname", "Kernel", "uname -a", C::System, Step::Run(&["uname", "-a"])).desc("Kernel name, release, version and architecture."),
    base("os-release", "OS release", "cat /etc/os-release", C::System, Step::Read("/etc/os-release")).desc("Distribution identification.").parser("os_release", O::KeyValue),
    base("hostnamectl", "System information", "hostnamectl", C::System, Step::Run(&["hostnamectl"])).desc("Hostname, chassis, OS, kernel, hardware.").parser("hostnamectl", O::KeyValue).sensitive(),
    // 13/14. PCI / USB
    base("lspci", "PCI devices", "lspci", C::Pci, Step::Run(&["lspci"])).desc("All PCI devices.").parser("lspci", O::Table),
    base("lspci-vmm", "PCI devices with drivers", "lspci -vmmnnk (internal: machine-readable form that drives the PCI table)", C::Pci,
        Step::Run(&["lspci", "-vmmnnk"])).desc("All PCI devices with vendor, device, IDs and kernel drivers.").parser("lspci_vmm", O::Table).internal(),
    base("lsusb", "USB devices", "lsusb", C::Usb, Step::Run(&["lsusb"])).desc("All USB devices.").parser("lsusb", O::Table),
    base("lsusb-v", "USB device details", "lsusb -v (internal: expandable device details)", C::Usb, Step::Run(&["lsusb", "-v"]))
        .desc("Verbose descriptors for every USB device.").internal().sensitive(),
    // 15. Network
    base("lshw-network", "Network hardware", "sudo lshw -C network", C::Network, Step::Run(&["lshw", "-C", "network"]))
        .desc("Network adapters, drivers and capabilities.").sudo().timeout(120).parser("lshw_tree", O::Tree).sensitive(),
    base("iw-dev", "Wireless interfaces", "iw dev", C::Network, Step::Run(&["iw", "dev"])).desc("Wi-Fi interfaces, SSID, channel.").parser("iw_dev", O::Tree).sensitive(),
    base("nmcli-status", "NetworkManager status", "nmcli device status", C::Network, Step::Run(&["nmcli", "device", "status"]))
        .desc("Devices known to NetworkManager.").parser("nmcli", O::Table),
    // 16. Kernel errors
    base("dmesg-errwarn", "Kernel errors & warnings", "dmesg --level=err,warn", C::Kernel, Step::Run(&["dmesg", "--level=err,warn"]))
        .desc("Kernel ring buffer, errors and warnings (may need privileges).").parser("dmesg", O::Text),
    base("dmesg-errwarn-50", "Last 50 kernel errors & warnings", "sudo dmesg --level=err,warn | tail -50", C::Kernel,
        Step::Run(&["dmesg", "--level=err,warn"])).desc("Most recent 50 lines.").sudo().filters(&[Filter::Tail(50)]).parser("dmesg", O::Text),
    base("dmesg-decoded", "Kernel errors & warnings (decoded)", "dmesg --level=err,warn -x (internal: adds the severity of every line)", C::Kernel,
        Step::Run(&["dmesg", "--level=err,warn", "-x"])).desc("Same messages with facility and level decoded.").internal().parser("dmesg", O::Text),
    // 17. Motherboard
    base("dmidecode-baseboard", "Baseboard details", "sudo dmidecode -t baseboard", C::System, Step::Run(&["dmidecode", "-t", "baseboard"]))
        .desc("Motherboard manufacturer, product, version.").sudo().parser("dmidecode_kv", O::KeyValue).sensitive(),
    base("dmidecode-board-mfr", "Baseboard manufacturer", "sudo dmidecode -s baseboard-manufacturer", C::System,
        Step::Run(&["dmidecode", "-s", "baseboard-manufacturer"])).sudo(),
    base("dmidecode-board-product", "Baseboard product", "sudo dmidecode -s baseboard-product-name", C::System,
        Step::Run(&["dmidecode", "-s", "baseboard-product-name"])).sudo(),
    // 18. System model
    base("dmidecode-system", "System details", "sudo dmidecode -t system", C::System, Step::Run(&["dmidecode", "-t", "system"]))
        .desc("Product, version, SKU, family (serial masked by default).").sudo().parser("dmidecode_kv", O::KeyValue).sensitive(),
    base("dmidecode-product", "System product", "sudo dmidecode -s system-product-name", C::System, Step::Run(&["dmidecode", "-s", "system-product-name"])).sudo(),
    base("dmidecode-version", "System version", "sudo dmidecode -s system-version", C::System, Step::Run(&["dmidecode", "-s", "system-version"])).sudo(),
    base("dmidecode-serial", "System serial number", "sudo dmidecode -s system-serial-number", C::System,
        Step::Run(&["dmidecode", "-s", "system-serial-number"])).desc("Sensitive. Masked by default and never exported unless you opt in.").sudo().sensitive(),
    // 19. Benchmark
    base("sysbench-cpu", "CPU benchmark", "sysbench cpu --threads=8 --time=30 run", C::Benchmark,
        Step::Run(&["sysbench", "cpu", "--threads={threads}", "--time={secs}", "run"])).desc("Prime-number CPU benchmark.")
        .params(BENCH_PARAMS).timeout(3700).parser("sysbench", O::KeyValue)
        .warn("The benchmark keeps the selected CPU threads fully busy for the chosen duration."),
    // 20. Stress
    base("stress-ng-cpu", "CPU stress test", "stress-ng --cpu 4 --timeout 60s --metrics-brief", C::Stress,
        Step::Run(&["stress-ng", "--cpu", "{workers}", "--timeout", "{secs}s", "--metrics-brief"])).desc("Sustained full CPU load with metrics.")
        .params(STRESS_PARAMS).timeout(3700).parser("stress_ng", O::Table).warn(STRESS_WARNING),
    // 21. Package management
    base("apt-install-htop", "Install htop", "sudo apt install htop", C::Packages, Step::Run(&["apt-get", "install", "-y", "htop"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-update", "Refresh package lists", "sudo apt update", C::Packages, Step::Run(&["apt-get", "update"]))
        .desc("Downloads the latest package indexes.").sudo().danger(D::ModifySystem).timeout(900).warn("Refreshes APT package lists from your configured mirrors (network access)."),
    base("apt-upgradable", "Upgradable packages", "apt list --upgradable", C::Packages, Step::Run(&["apt", "list", "--upgradable"]))
        .desc("Packages with newer versions available.").timeout(60).parser("apt_upgradable", O::Table),
    base("apt-install-smart", "Install SMART tools", "sudo apt install smartmontools nvme-cli", C::Packages,
        Step::Run(&["apt-get", "install", "-y", "smartmontools", "nvme-cli"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-nvme", "Install NVMe tools", "sudo apt install nvme-cli", C::Packages, Step::Run(&["apt-get", "install", "-y", "nvme-cli"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-sensors", "Install lm-sensors", "sudo apt install lm-sensors", C::Packages, Step::Run(&["apt-get", "install", "-y", "lm-sensors"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-mesa", "Install mesa-utils", "sudo apt install mesa-utils", C::Packages, Step::Run(&["apt-get", "install", "-y", "mesa-utils"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-upower", "Install upower", "sudo apt install upower", C::Packages, Step::Run(&["apt-get", "install", "-y", "upower"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-cpupower", "Install cpupower", "sudo apt install linux-cpupower", C::Packages, Step::Run(&["apt-get", "install", "-y", "linux-cpupower"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-sysbench", "Install sysbench", "sudo apt install sysbench", C::Packages, Step::Run(&["apt-get", "install", "-y", "sysbench"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-stress", "Install stress-ng", "sudo apt install stress-ng", C::Packages, Step::Run(&["apt-get", "install", "-y", "stress-ng"])).sudo().danger(D::ModifySystem).timeout(900).warn(APT_WARNING),
    base("apt-install-pkg", "Install diagnostic package", "sudo apt install <package> (internal: allowlisted diagnostic packages only)", C::Packages,
        Step::Run(&["apt-get", "install", "-y", "{pkg}"])).sudo().danger(D::ModifySystem).timeout(900).params(PKG_PARAM).warn(APT_WARNING).internal(),
    // 22. Cleanup
    base("apt-autoremove-preview", "Preview autoremove", "apt-get --simulate autoremove (internal: review before cleanup)", C::Packages,
        Step::Run(&["apt-get", "--simulate", "autoremove"])).desc("Lists packages autoremove would remove, without changing anything.").timeout(120).parser("apt_simulate", O::Table).internal(),
    base("apt-autoremove", "Remove unneeded packages", "sudo apt autoremove", C::Packages, Step::Run(&["apt-get", "autoremove", "-y"]))
        .desc("Removes automatically installed packages that nothing depends on.").sudo().danger(D::Destructive).timeout(900)
        .warn("The checklist warns: don't run this blindly. Review the list of packages first. Removed packages may include ones you rely on but never marked as manually installed."),
    // Internal helpers
    base("apt-policy", "Package availability", "apt-cache policy <packages> (internal: dependency page)", C::Packages,
        Step::Run(&["apt-cache", "policy", "smartmontools", "nvme-cli", "lm-sensors", "mesa-utils", "upower", "linux-cpupower", "sysbench", "stress-ng", "htop", "lshw", "dmidecode", "hdparm", "pciutils", "usbutils", "iw", "network-manager"]))
        .desc("Installed and candidate versions for diagnostic packages.").timeout(60).parser("apt_policy", O::KeyValue).internal(),
    // Open ports (internal: Ports page)
    base("ss-listening", "Listening ports", "ss -tulpn (internal: Ports page)", C::Network, Step::Run(&["ss", "-tulpn"]))
        .desc("TCP/UDP sockets listening on this machine; process names only for your own processes.").parser("ss", O::Table).internal(),
    base("ss-listening-root", "Listening ports (all owners)", "sudo ss -tulpn (internal: Ports page)", C::Network, Step::Run(&["ss", "-tulpn"]))
        .desc("Same list with the owning process of every socket, including root services.").sudo().parser("ss", O::Table).internal(),
    base("kill-pid", "Stop process", "sudo kill -TERM <pid> (internal: Ports page)", C::Processes, Step::Run(&["kill", "-TERM", "{pid}"]))
        .desc("Sends SIGTERM to a process owned by another user (e.g. root).").sudo().danger(D::Destructive).params(PID_PARAM)
        .warn("The process will be asked to exit and may lose unsaved work. If it belongs to a systemd service, systemd may restart it — stop the service instead.").internal(),
    base("systemctl-stop", "Stop service", "sudo systemctl stop <unit> (internal: Ports page)", C::Network, Step::Run(&["systemctl", "stop", "{unit}"]))
        .desc("Stops a systemd unit until next boot (or until something starts it again).").sudo().danger(D::ModifySystem).params(UNIT_PARAM)
        .warn("Stopping a service closes its ports and interrupts anything using it. It will start again at the next boot unless you also disable it.").internal(),
    base("systemctl-disable", "Stop and disable service", "sudo systemctl disable --now <unit> (internal: Ports page)", C::Network, Step::Run(&["systemctl", "disable", "--now", "{unit}"]))
        .desc("Stops a systemd unit and prevents it from starting at boot.").sudo().danger(D::ModifySystem).params(UNIT_PARAM)
        .warn("The service is stopped now and will no longer start at boot. Re-enable it later with: sudo systemctl enable --now <unit>.").internal(),
    base("set-swappiness", "Change swappiness", "sysctl -w vm.swappiness=<value> (internal: explicit swappiness change)", C::Memory,
        Step::Run(&["sysctl", "-w", "vm.swappiness={value}"])).desc("Sets vm.swappiness until next reboot.").sudo().danger(D::ModifySystem).params(SWAPPINESS_PARAM)
        .warn("Changing swappiness alters how aggressively the kernel moves memory to swap. The change lasts until reboot; you can restore the original value at any time.").internal(),
];

/// Look up an entry by id.
pub fn get(id: &str) -> Option<&'static CommandSpec> {
    REGISTRY.iter().find(|c| c.id == id)
}

/// All entries in a category.
pub fn by_category(cat: Category) -> impl Iterator<Item = &'static CommandSpec> {
    REGISTRY.iter().filter(move |c| c.category == cat)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command string from Laptop_Command_Checklist.pdf.
    const CHECKLIST: &[&str] = &[
        "sudo lshw -short",
        "sudo lshw",
        "lscpu",
        "lscpu | grep -E 'Model name|CPU\\(s\\)|Core|Thread|MHz|Cache'",
        "cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
        "cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
        "cpupower frequency-info",
        "sudo apt install linux-cpupower",
        "watch -n 1 \"grep 'cpu MHz' /proc/cpuinfo\"",
        "free -h",
        "sudo dmidecode --type memory",
        "sudo dmidecode -t memory",
        "sudo dmidecode --type memory | grep -E \"Maximum Capacity|Number Of Devices|Size:|Type:|Speed:|Configured Memory Speed\"",
        "swapon --show",
        "cat /proc/sys/vm/swappiness",
        "htop",
        "sudo apt install htop",
        "ps aux --sort=-%cpu | head -15",
        "ps aux --sort=-%mem | head -15",
        "ps aux --sort=-%mem | head -20",
        "lspci | grep -Ei 'vga|3d|display'",
        "lspci -nnk | grep -A 3 -Ei 'vga|3d|display'",
        "glxinfo -B",
        "sudo apt install mesa-utils",
        "lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS,MODEL",
        "sudo apt install nvme-cli",
        "sudo nvme list",
        "sudo nvme smart-log /dev/nvme0",
        "sudo hdparm -Tt /dev/nvme0n1",
        "df -h",
        "sudo du -xh / --max-depth=1 2>/dev/null | sort -h",
        "du -xh ~ --max-depth=1 2>/dev/null | sort -h",
        "sudo apt install smartmontools nvme-cli",
        "sudo smartctl -a /dev/nvme0n1",
        "sudo smartctl -x /dev/nvme0n1",
        "sudo apt install lm-sensors",
        "sudo sensors-detect",
        "sensors",
        "watch -n 1 sensors",
        "sudo apt install upower",
        "upower -i $(upower -e | grep BAT)",
        "systemd-analyze",
        "systemd-analyze blame",
        "systemd-analyze blame | head -20",
        "systemd-analyze critical-chain",
        "systemctl list-unit-files --state=enabled",
        "uname -a",
        "cat /etc/os-release",
        "hostnamectl",
        "lspci",
        "lsusb",
        "sudo lshw -C network",
        "iw dev",
        "nmcli device status",
        "dmesg --level=err,warn",
        "sudo dmesg --level=err,warn | tail -50",
        "sudo dmidecode -t baseboard",
        "sudo dmidecode -s baseboard-manufacturer",
        "sudo dmidecode -s baseboard-product-name",
        "sudo dmidecode -t system",
        "sudo dmidecode -s system-product-name",
        "sudo dmidecode -s system-version",
        "sudo dmidecode -s system-serial-number",
        "sudo apt install sysbench",
        "sysbench cpu --threads=8 --time=30 run",
        "sudo apt install stress-ng",
        "stress-ng --cpu 4 --timeout 60s --metrics-brief",
        "sudo apt update",
        "apt list --upgradable",
        "sudo apt autoremove",
    ];

    #[test]
    fn every_checklist_command_is_registered() {
        for cmd in CHECKLIST {
            assert!(REGISTRY.iter().any(|c| c.original == *cmd && c.from_checklist), "missing from registry: {cmd}");
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = REGISTRY.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len());
    }

    #[test]
    fn sudo_entries_match_original_and_danger() {
        for c in REGISTRY.iter().filter(|c| c.from_checklist) {
            assert_eq!(c.requires_sudo, c.original.starts_with("sudo "), "{}", c.id);
            if c.requires_sudo {
                assert!(c.danger >= Danger::PrivilegedRead, "{}", c.id);
            }
        }
    }

    #[test]
    fn dangerous_entries_need_confirmation() {
        for id in ["stress-ng-cpu", "apt-autoremove", "sensors-detect", "apt-update", "apt-install-sysbench", "set-swappiness"] {
            assert!(get(id).is_some_and(|c| c.needs_confirmation()), "{id}");
        }
        assert_eq!(get("apt-autoremove").map(|c| c.danger), Some(Danger::Destructive));
    }

    #[test]
    fn placeholders_are_declared() {
        for c in REGISTRY {
            if let Step::Run(argv) = c.step {
                for a in argv {
                    if let Some(start) = a.find('{') {
                        let key = &a[start + 1..a.find('}').unwrap_or(a.len())];
                        assert!(key == "home" || c.params.iter().any(|p| p.key == key), "{}: undeclared {key}", c.id);
                    }
                }
            }
        }
    }
}
