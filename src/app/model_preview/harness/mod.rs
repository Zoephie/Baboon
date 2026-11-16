//! What the preview's tests stand on: finding the editing kits, and, on
//! macOS and Windows, a headless GL context the real renderer paints into. `shot`'s
//! docs list the `H2_SHOT_*` variables a manual shot reads.

#[cfg(target_os = "macos")]
pub(in crate::app::model_preview) mod gl;
#[cfg(windows)]
#[path = "gl_win.rs"]
pub(in crate::app::model_preview) mod gl;
#[cfg(any(target_os = "macos", windows))]
mod gl_state;
pub(in crate::app::model_preview) mod kit;
#[cfg(target_os = "macos")]
mod links;
#[cfg(any(target_os = "macos", windows))]
pub(in crate::app::model_preview) mod shot;
#[cfg(target_os = "macos")]
mod sweeps;
