//! SystemHealthCheck — Linux Laptop Diagnostic & Performance Center.
//!
//! `SystemHealthCheck` starts the GUI; `SystemHealthCheck --helper ...` is the privileged
//! helper started through pkexec (see `systemhealthcheck::helper`).

mod ui;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--helper") {
        let code = systemhealthcheck::helper::main(&args[2..]);
        return std::process::ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    if args.get(1).map(String::as_str) == Some("--version") {
        println!("SystemHealthCheck {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    if ui::run() == gtk::glib::ExitCode::SUCCESS {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
