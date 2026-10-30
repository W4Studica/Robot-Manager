#![cfg(target_os = "linux")]

pub mod alerts;
pub mod config;
pub mod config_tool;
pub mod hwmon;
pub mod inventory;
pub mod irqconf;
pub mod manager;
pub mod mrccomm;
pub mod network;
pub mod output;
pub mod process;
pub mod realtime;
pub mod riolog;
pub mod storage;
pub fn log(level: &str, message: impl std::fmt::Display) {
    use std::io::Write;
    // A closed journal must never panic in the supervisor.
    let _ = writeln!(std::io::stderr().lock(), "{level} {message}");
}
