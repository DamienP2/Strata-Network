//! `strata-tray`: the Strata Network launcher in the macOS menu bar or the Windows notification
//! area. On other systems it prints a message and leaves.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod common;

#[cfg(any(target_os = "macos", windows))]
mod app;

#[cfg(any(target_os = "macos", windows))]
fn main() {
    app::run();
}

#[cfg(not(any(target_os = "macos", windows)))]
fn main() {
    let (_, port) = common::locate();
    eprintln!("strata-tray runs on macOS and Windows only. The server is managed with `strata-network service …`; its web page is http://localhost:{port}/");
}
