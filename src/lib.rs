#![cfg(target_os = "linux")]

pub mod config;
pub mod manager;
pub mod output;
pub mod process;
pub mod riolog;

pub fn log(level: &str, message: impl std::fmt::Display) {
    use std::io::Write;
    // A closed journal must never panic in the supervisor.
    let _ = writeln!(std::io::stderr().lock(), "{level} {message}");
}
