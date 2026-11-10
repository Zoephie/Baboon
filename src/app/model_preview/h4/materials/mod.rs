//! Halo 4's `material`s: baked as the game bakes them at load (blam-tags
//! `material::bake`), then drawn through their own material shader's
//! translated passes. Objects take the path every loaded level gives them,
//! `albedo` then `static_probe` (`select_mesh_entry_point_for_pass`, MCC
//! 0x1408CE100: lightmaps available, no per-object policy flags); with no
//! level at all the game picks `static_per_vertex_color`, which no material
//! shader compiles. What blends draws `single_pass_single_probe`.

use super::super::common::fields::elements;
use super::super::gen3::shaders::{
    Gen3Blend, Gen3Material, Gen3MaterialDialect, Gen3Pass, Gen3Texture,
};
use crate::app::model_preview::common::image::Sampler;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::parse::TextureKind;

use crate::app::model_preview::h4::materials::shaders::{
    H4_DEFINES, H4ShaderSource, translate_pass,
};
use crate::app::model_preview::h4::materials::submit::{
    H4HologramPost, H4ScreenMaterial, H4Submit,
};
use crate::core::hlsl::emit::Stage;
use crate::core::hlsl::parse::Program;

mod options;
pub(in crate::app::model_preview) use options::*;
mod resolve;
pub(in crate::app::model_preview) use resolve::*;
mod effect;
pub(in crate::app::model_preview) use effect::*;
pub(crate) mod shaders;
pub(in crate::app::model_preview) mod submit;
#[cfg(test)]
mod tests;
