//! Resolving a render_model's materials to the textures a shaded preview samples.
//! This is the shared part: the dispatcher, the texture slots, the caches and
//! the bitmap binding and decode every game uses. Each game's own shader
//! resolution is its folder's `materials.rs`. GL upload and drawing belong to
//! the renderer, and geometry to the preview conversion beside it.

use super::ce::shaders::{CeCamoSettings, CeModelCommon};
use super::common::image::Image;
use super::dx9::passes::{Dx9DrawPass, Dx9Layer, DX9_LAYER_COUNT, dx9_shadow_passes};
use super::dx9::stage::Dx9VertexStage;
use super::gen3::shaders::{Gen3Material, Gen3Shield, Gen3Sources};
use crate::core::h2_pixel_shader::SamplerKind;

use super::gen3::materials::{
    Gen3Bitmap, gen3_xform, prepare_h3_material, reach_reference, reach_version_update,
};

mod slots;
pub(in crate::app) use slots::*;
mod textures;
pub(in crate::app) use textures::*;
mod caches;
pub(in crate::app::model_preview) use caches::*;
mod resolve;
pub(in crate::app) use resolve::*;
mod binding;
pub(in crate::app::model_preview) use binding::*;
#[cfg(test)]
mod tests;
