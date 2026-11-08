//! Halo 4's tracers (`tracer_system`), which replace Halo 3's and Reach's
//! beams and contrails. A point-to-point tracer runs between an effect part's
//! two locations and is worked out whole each frame, as
//! `tracer_stateless_setup.fx` does on the card; it is drawn through the
//! tracer's material from each profile's packed state
//! (`h4_tracer_pass`). A trailing tracer leaves profiles behind as its
//! location moves, so one on an object at rest draws nothing (the game's
//! `TracerGpu::Render` skips one profile or fewer); those are not built.
//! See `docs/h4-preview-research/tracers.md`.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::common::view::posed_on;
use super::super::functions::FunctionValues;
use super::super::common::named_function::tag_function_at;
use super::super::effects::{
    EffectOnsets, EffectTimes, for_each_attached_part, part_frame,
    read_part_placement,
};
use super::super::particles::{read_ends, slerped};
use super::particles::{pack_argb8, pack_half2, pack_ushort2n};
use super::super::h4::materials::{H4EffectMaterial, resolve_h4_effect_material};
use super::super::*;
use crate::app::model_preview::common::math::length;
use crate::app::model_preview::common::random::{high16_over_65535, high24_over_2_24, lcg_step};
use crate::app::model_preview::common::attached::attached;

mod read;
pub(in crate::app::model_preview) use read::*;
mod properties;
use properties::*;
mod definition;
pub(in crate::app::model_preview) use definition::*;
mod profile;
pub(in crate::app::model_preview) use profile::*;
mod draw;
use draw::*;
mod trail;
pub(in crate::app::model_preview) use trail::*;
#[cfg(test)]
mod tests;
