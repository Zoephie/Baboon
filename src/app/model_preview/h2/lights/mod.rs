//! Halo 2's lens flares on objects: `render_submit`'s attachments, then
//! `rasterizer_lens_flares.cpp`. The shape is Halo CE's, in
//! [`super::super::ce::lights`], but the sums are not: brightness, color and rotation
//! animate, occlusion comes in two sizes, and the pixel shader adds a highlight.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, function_slot_of, has_flag, has_flag_in_any, object_block};
use crate::app::model_preview::common::flares::FlareSprite;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::super::effects::{for_each_attached_part, part_markers};
use blam_tags::tag_function::h2::transition_evaluate;
use super::super::dx9::passes::{H2DefaultLight, H2Layer, ce_color_interpolate};
use super::super::common::image::Image;
use super::super::functions::object_input;
use super::super::common::named_function::tag_function_at;
use super::super::materials::decode_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{UNSET, cross, dot, lerp};
use crate::app::model_preview::common::random::lcg_step;
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::flares::OcclusionOffset;
use crate::app::model_preview::dx9::passes::ce_color_interpolation;
use crate::app::model_preview::h2::effect_part::{H2EffectLight, H2EffectPart};

mod place;
pub(in crate::app::model_preview) use place::*;
mod flares;
pub(in crate::app::model_preview) use flares::*;
mod light;
pub(in crate::app::model_preview) use light::*;
#[cfg(test)]
mod tests;
