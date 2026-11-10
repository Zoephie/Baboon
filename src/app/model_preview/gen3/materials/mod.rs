//! The gen3 games' render methods resolved to the materials and bitmaps the
//! gen3 pipeline draws.

use super::super::common::fields::{elements, has_flag};
use super::super::common::image::Image;
use super::super::gen3::shaders::{
    Gen3Animated, Gen3Animation, Gen3Blend, Gen3Clock, Gen3Material, Gen3Pass, Gen3Shield,
    Gen3ShieldSettings, Gen3Sources, Gen3Texture, Gen3Value, RenderMethodDialect,
};
use crate::app::model_preview::common::image::{ImageTexels, Sampler};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::parse::TextureKind;

mod bitmap;
pub(in crate::app::model_preview) use bitmap::*;
mod resolve;
pub(in crate::app::model_preview) use resolve::*;
mod parameters;
pub(in crate::app::model_preview) use parameters::*;
mod prepare;
pub(in crate::app::model_preview) use prepare::*;
mod shield;
pub(in crate::app::model_preview) use shield::*;
#[cfg(test)]
mod tests;
