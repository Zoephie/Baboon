//! The vertex stage for Halo 2's transparent passes.
//!
//! Every transparent pass a model uses names one of five vertex shaders, and
//! they are variations on one program, written out here: a Fresnel tint in
//! `color0`, fog in `color1`, and texture coordinates through per-stage 2D
//! transforms. The preview has no fog, and the pixel shaders scale the scene
//! by `color1.a`, so `color1` is left at 1.

use super::super::h2::prt::H2_PRT_REGISTERS;
use super::super::renderer::model_vertex_prelude;

mod kind;
pub(in crate::app::model_preview) use kind::*;
mod ce;
pub(crate) use ce::CE_POINT_LIGHT_ROWS;
mod h2;
mod source;
#[cfg(test)]
mod tests;
