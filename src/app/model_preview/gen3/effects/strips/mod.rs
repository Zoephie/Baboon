//! Halo 3's beams and contrails: ribbons, crosses and tubes of profiles. A beam
//! is worked out whole each frame; a contrail leaves profiles behind as its
//! location moves, so one on a still object draws nothing, as in the game.

use std::collections::BTreeMap;
use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::super::common::fields::{elements, has_flag};
use super::super::super::common::view::{ViewFrame, posed_on};
use super::super::super::functions::FunctionValues;
use super::super::super::common::named_function::tag_function_at;
use super::super::super::effects::{EffectOnsets, EffectTimes, for_each_attached_part, part_frame, read_part_placement};
use super::super::super::particles::{
    H3ParticleVertex, H3SpriteShader, lightmap_tint, particle_exposure, read_ends, read_sprite_shader, slerped,
};
use super::super::super::*;
use crate::app::model_preview::common::math::{clamped_unit, cross, dot, minus};
use crate::app::model_preview::common::random::{high24_over_2_24, lcg_step};
use crate::app::model_preview::common::attached::attached;

mod load;
pub(in crate::app::model_preview) use load::*;
mod geometry;
mod contrail;
use contrail::*;
mod field;
pub(in crate::app::model_preview) use field::*;
#[cfg(test)]
mod tests;
