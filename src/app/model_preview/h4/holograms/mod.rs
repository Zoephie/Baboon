//! Halo 4's holograms, as `c_render_hologram::RenderHolograms` draws them
//! (X360 0x829B63C8, MCC 0x1408FB6B0; `docs/h4-preview-research/holograms.md`).
//! They leave the scene's passes for a surface of their own, depth-tested
//! against a copy of the scene's depth (MCC), take a bloom of their own, go
//! through `final_combine_hologram` (explicit shader 22) and blend into the
//! scene's light through `hologram_composite` (24) as one transparent, placed
//! at their sphere.
//!
//! The screen passes run on a texcoord that climbs the picture, as GL's
//! targets are stored. The game's screen rectangles run down it, so each is
//! handed over mirrored: a coverage `(x, top, width, height)` becomes
//! `(x, H - top, width, -height)`, and the shader's aligned texcoord and
//! distortion come out as the game's.

use super::super::gen3::state::Gen3State;
use super::render::H4Game;
use std::collections::HashMap;
use std::sync::Arc;

use eframe::glow::{self, HasContext};

use super::super::gen3::pipeline::{h3_set_cull, set_value};

use crate::app::model_preview::gl::texture::upload_h2_image_linear;
use super::super::gl::target::{Made, Sampling, Texels};
use super::super::gen3::shaders::{H3Material, H3Pass, H3Value};
use super::bloom::H4BloomSettings;
use super::materials::submit::{DrawnObject, H4Draw, H4HologramPost, H4ScreenMaterial};
use super::super::common::view::ce_flare_view;
use super::super::*;

mod screen;
pub(in crate::app::model_preview) use screen::*;
mod pass;
#[cfg(test)]
mod tests;
