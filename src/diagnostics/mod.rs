//! Diagnostic engine: the allowlisted command registry, the argv-based
//! runner (with pkexec elevation through the built-in helper), the scan
//! scheduler (Quick Scan / Full Diagnostic) and snapshots.

pub mod registry;
pub mod runner;
pub mod scheduler;
pub mod snapshot;

pub use registry::{Category, CommandSpec, Danger, REGISTRY};
pub use runner::{CmdOutput, Params, RunControl, RunStatus};
pub use snapshot::{Parsed, Snapshot};
