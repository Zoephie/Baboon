//! Tests that read a kit and cross games.

use super::*;

mod kit_objects;
mod sprite_reads;
#[cfg(target_os = "macos")]
mod material_highlight;
#[cfg(target_os = "macos")]
mod snapshot;
