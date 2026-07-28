//! Baboon desktop application entry point and process-level startup wiring.
//! It owns process startup only; application state and feature behavior belong under `app`.

// Release builds run as a Windows GUI app (no console window). Debug builds
// keep the console so logs/diagnostics remain visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod format;
mod source;
mod storage;
mod tool_commands;
mod window_state;

use anyhow::Result;

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
}

fn main() -> Result<()> {
    set_windows_app_user_model_id();
    let window_state::StartupWindowState { restored, tracker } =
        window_state::load_startup_state();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size(window_state::DEFAULT_INNER_SIZE)
        .with_min_inner_size(window_state::MIN_INNER_SIZE)
        .with_title("Baboon");
    if let Some(restored) = restored {
        viewport = viewport
            .with_inner_size(restored.inner_size)
            .with_maximized(restored.maximized);
        // Fullscreen is issued by WindowStateTracker during eframe's hidden
        // first frame. winit 0.30 can cache a ViewportBuilder fullscreen flag
        // without applying the native transition, making later retries no-ops.
        if let Some(position) = restored.position {
            viewport = viewport.with_position(position);
        }
    }
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "Baboon",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::Baboon::new(cc, tracker)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(windows)]
fn set_windows_app_user_model_id() {
    use std::os::windows::ffi::OsStrExt;

    let app_id = std::ffi::OsStr::new("Zoephie.Baboon")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // A stable process identity keeps debug, release and portable launches in
    // Baboon's own taskbar group instead of inheriting a generic launcher icon.
    let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr()) };
}

#[cfg(not(windows))]
fn set_windows_app_user_model_id() {}

fn app_icon() -> Option<eframe::egui::IconData> {
    let image = image::load_from_memory_with_format(
        include_bytes!("../icon/baboon.ico"),
        image::ImageFormat::Ico,
    )
    .ok()?
    .to_rgba8();
    Some(eframe::egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::app_icon;

    #[test]
    fn bundled_application_icon_decodes_for_the_native_window() {
        let icon = app_icon().expect("bundled Baboon icon should decode");
        assert!(icon.width >= 32);
        assert!(icon.height >= 32);
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_uses_baboon_taskbar_identity() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn GetCurrentProcessExplicitAppUserModelID(app_id: *mut *mut u16) -> i32;
        }
        #[link(name = "ole32")]
        unsafe extern "system" {
            fn CoTaskMemFree(memory: *mut std::ffi::c_void);
        }

        super::set_windows_app_user_model_id();
        let mut app_id = std::ptr::null_mut();
        assert_eq!(
            unsafe { GetCurrentProcessExplicitAppUserModelID(&mut app_id) },
            0
        );
        assert!(!app_id.is_null());
        let length = unsafe {
            (0..)
                .find(|offset| *app_id.add(*offset) == 0)
                .expect("AppUserModelID should be null terminated")
        };
        let value = unsafe { OsString::from_wide(std::slice::from_raw_parts(app_id, length)) };
        unsafe { CoTaskMemFree(app_id.cast()) };
        assert_eq!(value, "Zoephie.Baboon");
    }
}
