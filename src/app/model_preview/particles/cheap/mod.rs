//! Reach's cheap particles (`cheap_particle_emitter`, `cpem`): one pool of
//! 8192 the card spawns, moves and draws every frame, from emitters that an
//! effect's parts submit while their event runs. Here they run on the
//! processor a preview frame at a time, and draw as one batch after every
//! other transparent. They collide only with what is on screen, which the
//! preview leaves out (see [`ReachCheapField::update`]).

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::super::common::named_function::tag_function_at;
use super::super::effects::{
    CHEAP_PARTICLE_EMITTER, EffectOnsets, EffectTimes, for_each_attached_part, part_frame,
    read_part_placement,
};
use super::super::particles::{PRE_ROLL, Random, particle_game_time};
use super::super::reach::lights::object_value;
use super::super::*;
use crate::app::model_preview::common::math::{clamped_unit, cross, dot, minus};
use crate::app::model_preview::common::attached::attached;

mod library;
pub(in crate::app::model_preview) use library::*;
mod emitter;
pub(in crate::app::model_preview) use emitter::*;
mod field;
pub(in crate::app::model_preview) use field::*;
mod gl;
pub(in crate::app::model_preview) use gl::*;
#[cfg(test)]
mod tests;

/// `s_cheap_particle_system::k_max_particles`: the ring every emitter writes.
pub(super) const MOST_PARTICLES: usize = 0x2000;
/// `add_emitter`'s room (the 360's 0x82DB1D68, MCC 0x1403E7E10): past it,
/// "too many cheap particle emitters".
const MOST_EMITTERS: usize = 200;
/// `shader_setup_global_state` (the 360's 0x82DB2368): the shaders'
/// `delta_time.x` is the frame's, at most this.
const MOST_SHADER_STEP: f32 = 0.5;
/// `s_emitter::spawn`'s string id 440: the spawn rate reads the emitter's
/// scale.
const ALPHA: &str = "alpha";
/// What `effects_submit_cheap_attachments` (the 360's 0x82EA9570) passes for
/// an effect with no lightprobe; the preview has none [inferred: the game's
/// sample is a level's lightmap].
const NO_PROBE_INTENSITY: f32 = 1.0;
/// `postprocess`'s textures for an emitter that names none.
const DEFAULT_POSITION: &str = "shaders\\default_bitmaps\\bitmaps\\random_in_sphere";
const DEFAULT_VELOCITY: &str = "shaders\\default_bitmaps\\bitmaps\\random_in_sphere_center_biased";

/// `cheap_particle_emitter_flags`, by option name.
const CORRELATE: &str = "correlate position and velocity";
const POSITION_LOCAL: &str = "position texture in local space";
const VELOCITY_LOCAL: &str = "velocity texture in local space";
const NORMALIZE: &str = "normalize velocity before scaling";
const ROTATE_UP: &str = "randomly rotate emitter about up vector";

/// The type texture's rows (`cheap_particles_common.hlsl`'s `TYPE_DATA_*`).
const PHYSICS: usize = 0;
const COLLISION: usize = 1;
const COLOR0: usize = 2;
const FADE: usize = 3;
const RENDER: usize = 4;

/// `cheap_particles_initialize.hlsl`: an empty slot, `(0, 0, 0, 2)` and
/// `(0, 1, 0, 0)`, dead by its age.
const EMPTY: CheapParticle =
    CheapParticle { position: [0.0; 3], age: 2.0, velocity: [0.0, 1.0, 0.0], delta_age: 0.0, parameters: [0; 4] };

/// HLSL's `frac`.
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// HLSL's `sign`: 0 at 0, which Rust's `signum` is not.
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}
