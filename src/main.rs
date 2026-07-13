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

use anyhow::Result;

fn main() -> Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([520.0, 360.0])
        .with_title("Baboon");
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
        Box::new(|cc| Ok(Box::new(app::Baboon::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

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
