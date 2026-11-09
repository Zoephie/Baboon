//! Halo CE's `particle_system` (`pctl`) on an object's attachments: types of
//! particle, each stepping through states of its own while its particles
//! step through theirs, made by default, explosion or jet physics and moved
//! by point physics. Read from the Xbox 360 build's symbols:
//! `particle_system_new_attached`, `particle_system_initialize`,
//! `particle_system_update`, `particle_system_new_particles`, the
//! `particle_system_new_particle_*` and `particle_system_update_particle_default`
//! functions, `randomize_particle_variables`, `particle_system_render` and
//! `build_sprite_rotational`. What tool's postprocess (`sub_49BFC0`,
//! `sub_49BFE0`) fills in for the game is filled in here.

use std::collections::VecDeque;
use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use super::super::common::fields::{elements, function_slot_of, has_flag, has_flag_after, object_block};
use super::super::common::view::{ViewFrame, posed_on};
use crate::app::model_preview::common::point_physics::CePointPhysics;
use super::super::*;
use super::super::ce::random::CeRandom;
use crate::app::model_preview::common::math::{cross, dot, unit_or_unchanged};
use crate::app::model_preview::ce::blend::FramebufferBlend;
use crate::app::model_preview::ce::sprite_sheet::{CeSecondaryDraw, CeSecondaryMap, CeSpriteSheet, read_secondary_map, read_sprite_sheet};

mod read;
pub(in crate::app::model_preview) use read::*;
mod sim;
pub(in crate::app::model_preview) use sim::*;
mod draw;
pub(in crate::app::model_preview) use draw::*;
#[cfg(test)]
mod tests;
