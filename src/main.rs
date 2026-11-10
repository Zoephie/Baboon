//! Baboon desktop application entry point.
//! It only starts the application; everything else lives in the library.

// Release builds run as a Windows GUI app (no console window). Debug builds
// keep the console so logs/diagnostics remain visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();
    baboon::run()
}

/// A release build is a GUI app, which Windows starts with no console, so
/// what it logs goes nowhere even when started from one. Started from a
/// terminal, write to that terminal instead; started any other way, or with
/// its output already redirected, change nothing.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
    };
    // SAFETY: both calls only read or set this process's console state.
    unsafe {
        // An error is a null or invalid handle: nothing redirected.
        if GetStdHandle(STD_ERROR_HANDLE).is_err() {
            // Fails when the parent has no console, which is the usual launch.
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
