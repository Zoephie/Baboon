//! Halo CE's effects on an object: the `effect` tags `attachments_new` hangs
//! on a marker through `effect_new_looping`, and the `particle`s their events
//! let go (`effect_generate_particles`), stepped by `particles_update` and drawn
//! by `render_particles` through the `effect` shaders of the kit's `fx` folder.
//! Everything read from the Xbox 360 build's symbols (HCEX_Release); what the
//! tags on disk lack, tool's postprocess (`haloce_mcc_tool`) bakes as noted.
//!
//! One departure on purpose: the game makes no effect or particle system
//! particle outside a level, since `particle_new` (0x8373e808) and
//! `particle_system_new_particles` (0x83739cc0) need a leaf or cluster to put
//! it in. The preview has no level, and simulates as though the object stood
//! in one.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use crate::app::model_preview::common::math::perpendicular;
use super::super::common::fields::{elements, function_slot_of, has_flag, object_block};
use super::super::common::point_physics::CePointPhysics;
use super::lights::{CeEffectLightDef, CeLight, read_effect_light};
use super::super::common::view::{ViewFrame, posed_on};
use super::particle_systems::{
    CePctlDef, CePctlQuad, CeRunningSystem, MOST_SYSTEM_PARTICLES, new_unattached, read_pctl_def,
    unattached_quads, update_unattached,
};
use crate::app::model_preview::common::fields::argb;
use super::super::dx9::passes::{ce_color_interpolate, ce_color_interpolation};
use super::super::common::image::Image;
use super::super::*;
use super::super::ce::random::CeRandom;
use crate::app::model_preview::common::math::{cross, dot, unit_or_unchanged};
use crate::app::model_preview::ce::sprite_sheet::{CeSecondaryDraw, CeSecondaryMap, CeSpriteFrame, CeSpriteSheet, read_secondary_map, read_sprite_sheet};
use crate::app::model_preview::ce::ticks::{CeTicks, PRE_ROLL, STEP};
use crate::app::model_preview::ce::blend::FramebufferBlend;

mod read;
pub(in crate::app::model_preview) use read::*;
mod place;
pub(in crate::app::model_preview) use place::*;
mod events;
pub(in crate::app::model_preview) use events::*;
mod sim;
pub(in crate::app::model_preview) use sim::*;
mod particle;
use particle::*;
mod draw;
pub(in crate::app::model_preview) use draw::*;
#[cfg(test)]
mod tests;
