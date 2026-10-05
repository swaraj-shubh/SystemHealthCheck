# SystemHealthCheck

**Linux Laptop Diagnostic & Performance Center** — a native desktop app for checking your laptop's health without typing commands. Built for Kali Linux and other Debian-based distros.

## Features

- **Dashboard and live monitor:** CPU, per-core load and frequency, RAM, swap, temperatures, fans, GPU, disk I/O, network, battery and processes, read directly from `/proc` and `/sys`.
- **Health score:** every finding shows the value it used, the rule applied, its severity and the reason.
- **Quick Scan / Full Diagnostic:** runs the hardware, SMART/NVMe, sensors, battery, boot, kernel log, PCI, USB and network checks, with one password prompt per scan.
- **Ports:** lists every open TCP/UDP port with its process and service. You can stop the process, or stop or disable the service, after confirming.
- **Command Center:** about 80 allowlisted commands, each with its exact command line, a raw output view, search, copy and save.
- **Benchmarks and stress tests:** `sysbench` and `stress-ng`, with live temperatures and an automatic emergency stop.
- **Packages:** shows which diagnostic tools are installed or missing, installs them one by one, and lets you review `apt autoremove` before running it.
- **History and reports:** scans are saved locally in SQLite so you can compare them. Reports export as PDF, HTML, JSON or text.
- **Privacy:** serial numbers, hostname and network addresses can be masked. No telemetry, and nothing leaves your machine.

## Tech stack

Rust · GTK4 · libadwaita · Cairo/Pango (charts and PDF) · SQLite (rusqlite) · polkit/pkexec

## Download

Get the latest `.deb` from **[Releases](https://github.com/swaraj-shubh/SystemHealthCheck/releases/latest)** and install it:

```sh
sudo apt install ./systemhealthcheck_*_amd64.deb
SystemHealthCheck
```

Requires GTK ≥ 4.12 and libadwaita ≥ 1.6 (Kali Rolling, Debian 13, Ubuntu 25.04 or newer).

## Build from source

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev cargo
git clone https://github.com/swaraj-shubh/SystemHealthCheck.git
cd SystemHealthCheck
cargo run --release          # or: ./target/release/SystemHealthCheck
```

Install it as a `.deb` package (adds a menu entry and the polkit policy):

```sh
./packaging/build-deb.sh
sudo apt install ./target/deb/systemhealthcheck_*.deb
SystemHealthCheck
```

After installing, open **SystemHealthCheck** from the app menu or run `SystemHealthCheck` in a terminal.

Optional tools the app can install for you from its Packages page: `smartmontools nvme-cli lm-sensors mesa-utils upower linux-cpupower sysbench stress-ng`.

## Tests

```sh
cargo test
```

## Security

Commands run only from a built-in allowlist and never through a shell. Anything that needs root goes through `pkexec` to a helper that checks every request again. Actions that change the system always ask for confirmation first.
# SystemHealthCheck
