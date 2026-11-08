//! The particle systems on an attached effect, for Halo 2, Halo 3, ODST, Reach
//! and Halo 4. The game updates most of these on the card, but nothing in the
//! update needs it, so they run here, a fixed step at a time, with their
//! movement controllers, and draw as sprites, particle models or distortion.

/// Reach's cheap particles, which Halo 4 runs too.
pub(super) mod cheap;
mod gl;
pub(in crate::app::model_preview) use gl::*;
pub(in crate::app::model_preview) use cheap::{CHEAP_FLOATS, ReachCheapGl, reach_cheap_gl, upload_cheap_render_texture};

use std::sync::Arc;

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode, ParameterSource, ResolvedRenderMethod, ResolvedValue};
use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::common::fields::{elements, has_flag, has_flag_labelled};
use super::common::view::ViewFrame;
use super::functions::FunctionValues;
use super::common::named_function::tag_function_at;
use super::effects::EffectOnsets;
use super::effects::{EffectTimes, ObjectEffect, object_effects, part_runs_by};
use super::gen3::effects::strips::{H3StripAt, read_carried_strips};
use super::gen3::shaders::{H3Blend, read_h3_definition};
use crate::app::model_preview::common::image::H3Sampler;
use super::common::image::H2Image;
use super::materials::{TextureImage, decode_h2_image};
use super::gen3::materials::{H3Bitmap, read_h3_bitmap, read_resolved_map, resolve_embedded_method};
use super::gen3::pipeline::EXPOSURE as H3_EXPOSURE;
use super::reach::lighting::{reach_alt_exposure, reach_exposure, reach_lightmap_tint, reach_particle_light};
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::game::GameId;
use crate::app::model_preview::common::math::{clamped_unit, cross, dot, minus};
use crate::app::model_preview::common::random::{high_bits, lcg_step};
use crate::app::model_preview::common::attached::attached;

mod rules;
pub(in crate::app::model_preview) use rules::*;
mod property;
pub(in crate::app::model_preview) use property::*;
mod seeds;
pub(in crate::app::model_preview) use seeds::*;
mod shader;
pub(in crate::app::model_preview) use shader::*;
mod look;
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
mod h4_draws;
pub(in crate::app::model_preview) use h4_draws::*;
#[cfg(test)]
mod tests;
