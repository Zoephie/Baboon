//! The dx9 pass model Halo CE and Halo 2 draw through: the layers in the
//! game's order, the default and shadow lights, a pass's blend, depth,
//! textures and animated constants, the draw pass the renderer binds, and the
//! objects' change colors. Halo 2's templates are read into it in
//! `h2::shader`; Halo CE's shaders in `ce::shaders`.

use super::super::common::named_function::tag_function_at;
use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};
use blam_tags::tag_function::FunctionType;

use super::super::functions::FunctionValues;

use super::super::h2::prt::H2PrtLight;
use super::stage::{H2_TEXTURE_ROWS, Dx9VertexStage};
use super::super::common::image::Image;
use crate::core::h2_pixel_shader::{
    AlphaTest, SamplerKind,
};

mod layer;
pub(in crate::app::model_preview) use layer::*;
mod lights;
pub(in crate::app::model_preview) use lights::*;
mod shadow;
pub(in crate::app::model_preview) use shadow::*;
mod pass;
pub(in crate::app::model_preview) use pass::*;
mod animation;
pub(in crate::app::model_preview) use animation::*;
mod numeric;
pub(in crate::app::model_preview) use numeric::*;
mod globals;
pub(in crate::app::model_preview) use globals::*;
mod change_colors;
pub(in crate::app::model_preview) use change_colors::*;
#[cfg(test)]
mod tests;
