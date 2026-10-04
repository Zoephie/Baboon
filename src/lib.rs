//! Baboon, a Halo tag editor: the application library behind the `Baboon`
//! binary and the build tools in `src/bin`.
//! It owns process startup and the module tree; `main.rs` only calls [`run`].

/// `include_str!` with a path from the package root, so a file can move
/// without its includes changing.
macro_rules! include_root_str {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path))
    };
}

/// `include_bytes!` with a path from the package root; see `include_root_str!`.
macro_rules! include_root_bytes {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path))
    };
}

mod app;
mod core;
pub mod script_docs_import;
pub mod tag_compat_build;
#[cfg(test)]
mod test_kits;
mod window_state;

use anyhow::Result;

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
}

/// Start the application and run it until its window closes.
pub fn run() -> Result<()> {
    set_windows_app_user_model_id();
    let startup_arguments = app::parse_startup_arguments(std::env::args_os().skip(1));
    let window_state::StartupWindowState { restored, tracker } = window_state::load_startup_state();
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
        // The shared model viewport renders through an egui Glow callback and
        // relies on hardware depth testing for dense, overlapping geometry.
        depth_buffer: 24,
        ..Default::default()
    };

    eframe::run_native(
        "Baboon",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::Baboon::new(cc, tracker, startup_arguments)))),
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
        include_root_bytes!("icon/baboon.ico"),
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
mod tests;
