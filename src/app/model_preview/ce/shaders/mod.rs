//! Halo CE's model shaders as draw passes.
//!
//! `shader_model` and a `shader_environment` on a model both draw through
//! `ShaderModel()` in the kit's `fx/model_common.h`. What feeds it follows
//! `rasterizer_model_draw_model_shader_pp` and
//! `rasterizer_model_draw_environment_shader_pp`; the lighting is
//! `default_object_lighting`, what the game falls back on with no level.

use crate::app::model_preview::load::TagReader;
use std::sync::{Arc, LazyLock};

use blam_tags::{TagBlock, TagFieldData, TagStruct};

use super::super::dx9::passes::{
    Dx9Blend, Dx9BlendEquation, Dx9BlendFactor, Dx9Compare, Dx9Cull, Dx9Depth, Dx9DrawPass, Dx9DrawTexture,
    H2_GLSL_VERSION,
};
use super::super::common::image::Image;
use super::super::dx9::passes::{CeNumeric, PassAnimation};
use super::super::common::fields::{argb, has_flag, has_flag_in_any};
use super::super::dx9::stage::{H2_TEXTURE_ROWS, Dx9VertexStage};
use crate::core::ce_pixel_shader::{TranslatedFxShader, translate_fx};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;
use crate::app::model_preview::common::random::{high16_times_inverse_65535, lcg_step};
use crate::app::model_preview::ce::blend::{FADE_MODES, FramebufferBlend};

pub(in crate::app::model_preview) mod resolve;
mod base;
pub(in crate::app::model_preview) use base::*;
mod periodic;
pub(in crate::app::model_preview) use periodic::*;
mod animation;
pub(in crate::app::model_preview) use animation::*;
mod model;
pub(in crate::app::model_preview) use model::*;
mod transparent;
pub(in crate::app::model_preview) use transparent::*;
mod glass;
use glass::*;
mod special;
use special::*;
mod camo;
pub(in crate::app::model_preview) use camo::*;
#[cfg(test)]
mod tests;
