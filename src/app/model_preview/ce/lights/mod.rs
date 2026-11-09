//! Halo CE's lens flares: what `attachments_new` hangs on an object as a
//! `light`, and the sprites `rasterizer_lens_flares_draw` makes of one.

use super::super::common::view::{ViewFrame, posed_on};
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::animation::ik::weapon::attached_marker_frame;
use super::super::common::fields::{elements, function_slot_of, has_flag, has_flag_in_any, object_block};
use super::super::common::flares::{FlareSprite, OcclusionOffset};
use super::shaders::periodic_evaluate;
use crate::app::model_preview::common::fields::argb;
use super::super::dx9::passes::{ce_color_interpolate, ce_color_interpolation};
use super::super::common::image::Image;
use super::super::materials::decode_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{UNSET, cross, dot, lerp};
use crate::app::model_preview::ce::transition::transition_evaluate;

mod light;
pub(in crate::app::model_preview) use light::*;
mod flare;
pub(in crate::app::model_preview) use flare::*;
mod point;
pub(in crate::app::model_preview) use point::*;
mod effect;
pub(in crate::app::model_preview) use effect::*;
#[cfg(test)]
pub(in crate::app::model_preview) mod tests;
