//! Halo 2's shader templates read into passes: a shader names a template,
//! whose LODs list passes by layer. Each pass reads its bitmaps, values and
//! colors by parameter name, and the game looks a name up in the shader, then
//! the template, then the pass. A parameter the pass gives a `source extern`
//! is filled at runtime instead. The passes become [`H2DrawPass`]es here too.
//!
//! Only LOD 0 is read. An object draws transparent passes with implementation
//! 0 and lightmap passes with `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`, as the game
//! chooses them for something with no lightmap of its own.

use super::super::functions::object_input;
use std::collections::HashMap;
use std::sync::Arc;
use blam_tags::{TagFieldData, TagFile, TagStruct};
use super::super::dx9::stage::{H2VertexStage, H2_TEXTURE_ROWS};
use super::super::materials::TextureImage;
use crate::app::model_preview::common::fields::has_flag;
use crate::core::h2_pixel_shader::{ConstantBinding, SamplerBinding, 
    AlphaCompare, AlphaTest, PixelShaderOptions, SamplerKind, translate,
};
use super::super::dx9::passes::*;
use super::super::common::image::Image;

pub(in crate::app::model_preview) mod resolve;
mod passes;
pub(in crate::app::model_preview) use passes::*;
mod constants;
pub(in crate::app::model_preview) use constants::*;
#[cfg(test)]
mod tests;
