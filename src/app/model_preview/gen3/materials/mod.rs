//! The gen3 games' render methods resolved to the materials and bitmaps the
//! gen3 pipeline draws.

use super::super::*;
use blam_tags::render_method::{
    BitmapAddressMode, BitmapFilterMode, ParameterSource, RenderMethod, RenderMethodAnimatedParameterType,
    RenderMethodBitmapExternMode,
    RenderMethodDefinition, RenderMethodParameterType, ResolvedRenderMethod,
    ResolvedValue,
};
use super::super::common::fields::{elements, has_flag};
use super::super::gen3::shaders::{H3Animated, H3Animation, H3Blend, H3Clock, H3Material, H3Pass, H3Shield, H3ShieldSettings, H3Sources, H3Texture, H3Value, RenderMethodDialect};
use crate::app::model_preview::common::image::{H3Sampler, H3Texels};
use super::super::common::image::H2Image;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::hlsl::parse::TextureKind;
use std::sync::Arc;
use std::collections::HashMap;

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
