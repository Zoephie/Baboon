//! Baboon desktop application entry point.
//! It only starts the application; everything else lives in the library.

// Release builds run as a Windows GUI app (no console window). Debug builds
// keep the console so logs/diagnostics remain visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    baboon::run()
}
