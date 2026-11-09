//! Halo 3's objects, as `render_objects` draws them: an albedo pass that lays
//! albedo and normal into targets, a lit pass reading them back into an HDR
//! accumulation target, and the final composite's tone curve onto the scene.

use super::render::{Gen3Batch, Gen3Cx, Gen3Game};
use super::sprites::SpriteStage;
use super::super::gl::screen::screen_program;
use super::super::common::sprites::layered_distance;
use super::super::gl::pass_state::PassState;
use super::super::gl::program::Linked;
use super::super::gl::target::{Made, Sampling, Texels};
use super::prt::{Gen3PrtKind, second_order_rotation};

use super::shaders::{Gen3Animated, Gen3Blend, Gen3Dialect, Gen3Material, Gen3Pass, Gen3Shield, Gen3Value};

use crate::app::model_preview::common::image::ImageTexels;
use eframe::glow::{self, HasContext as _};
use super::state::Gen3State;
use super::super::gl::buffer::model_draw_range;
use super::super::gl::texture::upload_image_linear;
#[cfg(test)]
use super::super::gl::texture::signed_bytes;
use super::super::common::view::ce_flare_view;
use super::super::*;
use crate::core::hlsl::parse::{Scalar, TextureKind, Type};

mod targets;
pub(in crate::app::model_preview) use targets::*;
mod programs;
pub(in crate::app::model_preview) use programs::*;
mod uniforms;
pub(in crate::app::model_preview) use uniforms::*;
mod lighting;
pub(in crate::app::model_preview) use lighting::*;
mod blend;
pub(in crate::app::model_preview) use blend::*;
mod sort;
use sort::*;
mod hooks;
pub(in crate::app::model_preview) use hooks::*;
mod stages;
pub(in crate::app::model_preview) use stages::*;
mod finish;
pub(in crate::app::model_preview) use finish::*;
#[cfg(test)]
mod tests;
