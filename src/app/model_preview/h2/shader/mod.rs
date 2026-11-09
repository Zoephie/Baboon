//! Halo 2's shader templates read into passes: a shader names a template,
//! whose LODs list passes by layer. Each pass reads its bitmaps, values and
//! colors by parameter name, and the game looks a name up in the shader, then
//! the template, then the pass. A parameter the pass gives a `source extern`
//! is filled at runtime instead. The passes become [`Dx9DrawPass`]es here too.
//!
//! Only LOD 0 is read. An object draws transparent passes with implementation
//! 0 and lightmap passes with `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`, as the game
//! chooses them for something with no lightmap of its own.

use super::super::common::image::Image;
use super::super::dx9::stage::{Dx9VertexStage, DX9_TEXTURE_ROWS};
use super::super::functions::object_input;
use super::super::materials::TextureImage;
use crate::app::model_preview::common::fields::has_flag;
use crate::core::h2_pixel_shader::{
    AlphaCompare, AlphaTest, ConstantBinding, PixelShaderOptions, SamplerBinding, SamplerKind,
    translate,
};

mod passes;
pub(in crate::app::model_preview) mod resolve;
pub(in crate::app::model_preview) use passes::*;
mod constants;
pub(in crate::app::model_preview) use constants::*;
#[cfg(test)]
mod tests;
