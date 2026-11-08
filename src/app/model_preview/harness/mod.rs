//! What the preview's tests stand on: finding the editing kits, and, on
//! macOS, a headless CGL context the real renderer paints into. `shot`'s
//! docs list the `H2_SHOT_*` variables a manual shot reads.

pub(in crate::app::model_preview) mod kit;
#[cfg(target_os = "macos")]
pub(in crate::app::model_preview) mod gl;
#[cfg(target_os = "macos")]
mod gl_state;
#[cfg(target_os = "macos")]
mod links;
#[cfg(target_os = "macos")]
pub(in crate::app::model_preview) mod shot;
#[cfg(target_os = "macos")]
mod sweeps;
