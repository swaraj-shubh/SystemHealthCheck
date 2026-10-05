//! One module per sidebar page. Each exposes `build(&Rc<AppState>) -> gtk::Widget`.

pub mod battery;
pub mod benchmarks;
pub mod boot;
pub mod commands;
pub mod cpu;
pub mod dashboard;
pub mod gpu;
pub mod hardware;
pub mod history;
pub mod kernel;
pub mod live;
pub mod memory;
pub mod network;
pub mod overview;
pub mod packages;
pub mod ports;
pub mod processes;
pub mod reports;
pub mod settings;
pub mod storage;
pub mod stress;
pub mod thermals;
