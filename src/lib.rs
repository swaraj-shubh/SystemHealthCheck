//! SystemHealthCheck backend: everything that touches the system lives here and is
//! free of GTK, so it can be unit-tested headless. The GTK front-end lives in
//! `src/ui` and only consumes these modules.

pub mod collectors;
pub mod database;
pub mod diagnostics;
pub mod helper;
pub mod packages;
pub mod parsers;
pub mod privacy;
pub mod reports;
pub mod scoring;
pub mod settings;
pub mod util;
