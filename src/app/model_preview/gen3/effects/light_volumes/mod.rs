//! Halo 3's light volumes: a row of camera-facing profiles along a direction.
//! The game works these out on the card every frame, but nothing in one
//! depends on the last, so they are worked out here a frame at a time.
//! Halo 4 draws them through their material from each profile's packed
//! state (`h4_light_volume_pass`), which is worked out here the same way.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::super::common::fields::{elements, has_flag};
use crate::app::model_preview::common::sprites::AntennaQuad;
use super::super::super::common::view::{ViewFrame, posed_on};
use super::super::super::functions::FunctionValues;
use super::super::super::common::named_function::tag_function_at;
use super::super::super::effects::{
    EffectTimes, for_each_attached_part, part_frame, read_part_placement,
};
use super::super::super::particles::inline_real;
use super::super::shaders::{Gen3Blend, read_h3_definition};
use crate::app::model_preview::common::image::Sampler;
use super::super::effects::strips::EPSILON;
use super::super::super::h4::materials::{H4EffectMaterial, resolve_h4_effect_material};
use super::super::super::gen3::materials::{Gen3Bitmap, read_h3_bitmap, resolve_embedded_method};
use super::super::pipeline::EXPOSURE as H3_EXPOSURE;
use super::super::super::reach::lighting::reach_alt_exposure;
use super::super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{clamped_unit, cross, length};
use crate::app::model_preview::common::random::{high16_over_65535, lcg_step};
use crate::app::model_preview::common::attached::attached;

mod property;
pub(in crate::app::model_preview) use property::*;
mod lod;
use lod::*;
mod load;
pub(in crate::app::model_preview) use load::*;
mod eval;
pub(in crate::app::model_preview) use eval::*;
#[cfg(test)]
mod tests;
