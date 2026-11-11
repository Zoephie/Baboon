//! The particle systems on an attached effect, for Halo 2, Halo 3, ODST, Reach
//! and Halo 4. The game updates most of these on the card, but nothing in the
//! update needs it, so they run here, a fixed step at a time, with their
//! movement controllers, and draw as sprites, particle models or distortion.

/// Reach's cheap particles, which Halo 4 runs too.
pub(super) mod cheap;
mod gl;
pub(in crate::app::model_preview) use cheap::{
    CHEAP_FLOATS, CheapGl, link_cheap_gl, upload_cheap_render_texture,
};
pub(in crate::app::model_preview) use gl::*;

use super::common::fields::{elements, has_flag, has_flag_labelled};
use super::common::image::Image;
use super::common::named_function::tag_function_at;
use super::common::view::ViewFrame;
use super::effects::EffectOnsets;
use super::effects::{EffectTimes, ObjectEffect, object_effects, part_runs_by};
use super::functions::FunctionValues;
use super::gen3::effects::strips::{Gen3StripAt, read_carried_strips};
use super::gen3::materials::{
    Gen3Bitmap, read_gen3_bitmap, read_resolved_map, resolve_embedded_method,
};
use super::gen3::pipeline::EXPOSURE as FIXED_EXPOSURE;
use super::gen3::pipeline::{
    probe_lightmap_tint, probe_particle_light, stops_alt_exposure, stops_exposure,
};
use super::gen3::shaders::{Gen3Blend, read_h3_definition};
use super::materials::{TextureImage, decode_image};
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::image::Sampler;
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::common::math::{clamped_unit, cross, dot, minus};
use crate::app::model_preview::common::random::{high_bits, lcg_step};
use crate::core::game::GameId;
use crate::core::h2_pixel_shader::SamplerKind;

mod rules;
pub(in crate::app::model_preview) use rules::*;
mod property;
pub(in crate::app::model_preview) use property::*;
mod seeds;
pub(in crate::app::model_preview) use seeds::*;
mod shader;
pub(in crate::app::model_preview) use shader::*;
pub(in crate::app::model_preview) mod look;
pub(in crate::app::model_preview) use look::*;
mod emitter;
pub(in crate::app::model_preview) use emitter::*;
mod movement;
pub(in crate::app::model_preview) use movement::*;
mod system;
pub(in crate::app::model_preview) use system::*;
mod sim;
pub(in crate::app::model_preview) use sim::*;
mod vertices;
pub(in crate::app::model_preview) use vertices::*;
mod live;
pub(in crate::app::model_preview) use live::*;
#[cfg(test)]
mod tests;
