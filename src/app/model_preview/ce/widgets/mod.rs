//! Halo CE's widgets beyond antennas: `light_volume` (`mgs2`), drawn by
//! `light_volume_render` as a row of screen-facing sprites along a marker.
//! Read from the Xbox 360 build's symbols; what tool's postprocess
//! (`sub_5251C0`) fills in for the game is filled in here.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::super::ce::random::CeRandom;
use super::super::common::fields::{
    elements, function_slot, function_slot_of, has_flag, object_block,
};
use super::super::common::image::Image;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::dx9::passes::{ce_color_interpolate, ce_color_interpolation};
use super::super::materials::decode_image;
use super::super::*;
use crate::app::model_preview::ce::blend::FramebufferBlend;
use crate::app::model_preview::common::fields::argb;
use crate::app::model_preview::common::math::{cross, dot, nonzero_unit};
use crate::app::model_preview::common::sprites::AntennaQuad;
use crate::core::h2_pixel_shader::SamplerKind;

mod light_volume;
pub(in crate::app::model_preview) use light_volume::*;
mod lightning;
pub(in crate::app::model_preview) use lightning::*;
mod glow;
pub(in crate::app::model_preview) use glow::*;
