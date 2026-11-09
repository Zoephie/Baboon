//! The per-game particle rules and the rules the tool's postprocess bakes: rest, fades, controllers and update kinds.

use super::super::TagReader;
use super::emitter::Emitter;
use super::look::{Billboard, ParticleLook, ParticleMaterialReader};
use super::property::ParticleProperty;
use super::seeds::SeedAllocator;
use super::{GameId, dot, elements};
use blam_tags::TagStruct;
use std::sync::Arc;

/// Seconds run before the first frame, so a held pose has its particles.
pub(in crate::app::model_preview) const PRE_ROLL: f32 = 6.0;

/// `s_sprite_frame_list`'s room.
pub(in crate::app::model_preview) const MOST_FRAMES: usize = 15;

/// What each game's particles do differently, from the game once. Each rule
/// holds for the games named on it.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct ParticleRules {
    /// Reach: a sprite system's options read their parameters through the
    /// resolver, with Reach's depth fade, warp, palette shift and plasma.
    pub(in crate::app::model_preview) reach_render_method: bool,
    /// Halo 4: drawn through the particle's own `actual material`; its
    /// `actual shader` is the deprecated render method's remains.
    pub(in crate::app::model_preview) read_material: Option<ParticleMaterialReader>,
    /// Reach and Halo 4: `c_particle_definition::postprocess` (tool
    /// 0x140DD6FA0) and `effect_postprocess` (0x140DD5070) seed the
    /// properties, controllers first.
    pub(in crate::app::model_preview) reach_postprocess_seeds: bool,
    /// Reach and Halo 4: `uses cheap shader` draws `particle_render_fast.hlsl`.
    pub(in crate::app::model_preview) cheap_shader: bool,
    /// Reach and Halo 4: the sprite adds its constant at illumination.
    pub(in crate::app::model_preview) adds_at_illum: bool,
    /// Reach and Halo 4: the look's own game time, lighting, exposure and
    /// shapes (stored as `ParticleLook::reach`).
    pub(in crate::app::model_preview) reach_look: bool,
    /// Reach and Halo 4: a card's acceleration lerps between its ends.
    pub(in crate::app::model_preview) card_lerps: bool,
    /// Reach and Halo 4: `volume emitter particle velocities are random`.
    pub(in crate::app::model_preview) random_volume_velocities: bool,
    /// Reach and Halo 4: the emitter's `axis scale`.
    pub(in crate::app::model_preview) scales_axes: bool,
    /// Halo 4: the emitter's own terms past Reach's.
    pub(in crate::app::model_preview) h4_emitter_terms: bool,
    /// Halo 2: a system's particle and emitters read as Halo 2 lays them
    /// out, with its own LOD defaults.
    pub(in crate::app::model_preview) own_system: Option<OwnParticleSystem>,
    /// ODST and Reach (the 360's 0x82EE3C98): `initialize_particle` nudges
    /// a still particle to a speed only where it faces its velocity; Halo 3
    /// nudges every one.
    pub(in crate::app::model_preview) only_facing_still_particles_move: bool,
    /// Halo 4: a cheap particle library binds its per-type turbulence, rows,
    /// sizes and exposure.
    pub(in crate::app::model_preview) cheap_turbulence_types: bool,
}

impl ParticleRules {
    /// A game's with none of its own.
    pub(in crate::app::model_preview) const DEFAULT: Self = Self {
        reach_render_method: false,
        read_material: None,
        reach_postprocess_seeds: false,
        cheap_shader: false,
        adds_at_illum: false,
        reach_look: false,
        card_lerps: false,
        random_volume_velocities: false,
        scales_axes: false,
        h4_emitter_terms: false,
        own_system: None,
        only_facing_still_particles_move: false,
        cheap_turbulence_types: false,
    };

    /// The kit's game's, by its rules.
    pub(in crate::app::model_preview) fn of(game: Option<GameId>) -> Self {
        super::super::load::rules(game).particles
    }
}

/// A system's look and its emitters.
pub(in crate::app::model_preview) type SystemParts = (Arc<ParticleLook>, Vec<Arc<Emitter>>);

/// A game whose particle systems read their own way.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct OwnParticleSystem {
    /// The system's look and emitters, from the system and its particle's
    /// path.
    pub(in crate::app::model_preview) read:
        fn(&TagReader, TagStruct<'_>, &str) -> Option<SystemParts>,
    /// The postprocess' LOD: the raw four, whether it ignores the camera
    /// mode, and the emitters' rates.
    pub(in crate::app::model_preview) baked_lod:
        fn([f32; 4], bool, &mut dyn Iterator<Item = f32>) -> [f32; 4],
}

/// `global_gravity_get`, in world units a second squared.
pub(super) const GRAVITY: f32 = 4.171_259_4;

/// The most an emitter keeps alive, whatever its tag allows.
pub(super) const MOST_PARTICLES: usize = 512;

pub(super) const SEEDS: usize = 8;

pub(super) const SYSTEM_SEEDS: usize = 4;

/// `c_particle_system_definition::postprocess`' LOD: in, out, feather in and
/// feather out. Only what ignores the camera mode gets the near pair, and that
/// from how fast it emits. Out and its feather default to 20 and 5 in both
/// MCC tools (Halo 3 0x1407E4A30, ODST 0x140828DE0); the tag's "defaults to
/// 30.0 / 10.0" is Halo 2's text, carried over.
pub(super) fn baked_lod(
    raw: [f32; 4],
    any_camera: bool,
    rates: impl Iterator<Item = f32>,
) -> [f32; 4] {
    let empty = |value: f32| value.abs() < 0.0001;
    let [mut near, mut far, mut feather_in, mut feather_out] = raw;
    if empty(far) {
        far = 20.0;
    }
    if empty(feather_out) {
        feather_out = 5.0;
    }
    if any_camera {
        let busy = ((rates.map(|rate| rate / 30.0).sum::<f32>() - 2.0) / 3.0).clamp(0.0, 1.0);
        if empty(near) {
            near = busy * -0.1;
        }
        if empty(feather_in) {
            feather_in = (1.0 - busy) + busy * 3.0;
        }
    }
    // Its inverses, which a feather of nothing leaves at 1.
    let feather = |value: f32| if empty(value) { 1.0 } else { value };
    [near, far, feather(feather_in), feather(feather_out)]
}

/// What Reach tool's postprocesses take as nothing.
pub(super) const POSTPROCESS_EPSILON: f32 = 0.0001;

/// Reach tool's `c_particle_system_definition::postprocess_block_proc`
/// (0x140AE3220): out and its feather default to 20 and 5, and nothing more,
/// whatever the camera mode. The feathers stay as authored, nothing included:
/// `calc_lod_amount` (the 360's 0x82EE59C8) tests `in + feather in` with the
/// raw one, and only multiplies by the inverse it bakes.
pub(super) fn reach_baked_lod(raw: [f32; 4]) -> [f32; 4] {
    let [near, far, feather_in, feather_out] = raw;
    let or = |value: f32, default: f32| {
        if value.abs() < POSTPROCESS_EPSILON {
            default
        } else {
            value
        }
    };
    [near, or(far, 20.0), feather_in, or(feather_out, 5.0)]
}

/// `1 / near fade range`, as it comes out on the card. Reach's postprocess
/// takes it of `max(range, ε)`, so a range of nothing fades over a hair.
pub(super) fn near_fade_range(clamped: bool, range: f32) -> f32 {
    if clamped {
        range.max(POSTPROCESS_EPSILON).recip()
    } else {
        range.recip()
    }
}

/// Where MCC's Reach particle clock comes round, in seconds.
const REACH_GAME_TIME_WRAP: f32 = 7200.0;

/// The `game time` state at `time` seconds in. Halo 3's is the clock's
/// fraction of a second. Reach's particles read `g_render_time+0x18`
/// (`s_render_time::update`, the 360's 0x827477A8, MCC sub_14084E170), the
/// frames' seconds summed whole; MCC converts its counter as a signed int, so
/// it runs to 7200 s and comes round from -7200 s.
pub(in crate::app::model_preview) fn particle_game_time(reach_clock: bool, time: f32) -> f32 {
    if reach_clock {
        (time + REACH_GAME_TIME_WRAP).rem_euclid(2.0 * REACH_GAME_TIME_WRAP) - REACH_GAME_TIME_WRAP
    } else {
        time.rem_euclid(1.0)
    }
}

/// `frame_advance`: what has landed stops once it is this slow.
pub(super) fn comes_to_rest(landed: bool, velocity: [f32; 3]) -> bool {
    landed && dot(velocity, velocity) <= SPEED_AT_REST * SPEED_AT_REST
}

/// `frame_advance`: 0.0625 of speed squared stops what has landed.
const SPEED_AT_REST: f32 = 0.25;

/// A `particle_physics` entry, in the order `update_particles` runs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ParticleController {
    Physics,
    Collider,
    Flock,
}

/// Where an emitter's particles are updated, as tool's `effect_postprocess`
/// decides it (`c_particle_emitter_definition::decide_whether_cpu_or_gpu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ParticleUpdate {
    /// `c_particle_emitter::frame_advance`, with every movement.
    Cpu,
    /// `particle_update.hlsl`: no collision, and physics once per emitter.
    Gpu,
}

/// `decide_whether_cpu_or_gpu`, and whether a particle that comes to rest
/// moves on to the card (`becomes gpu when at rest`). `force cpu updating`
/// wins, then `force gpu updating`, and either rules out the move; otherwise an
/// emitter goes on the card when its particle lists no attachments (H3's
/// particle flag 0x20, ODST's 0x40) and its own movement asks for nothing past
/// `physics` (movement flag 0x100), and its resting particles move there when
/// the particle lists no attachments. The emitter's and the system's own "can
/// be gpu" bits are set by every postprocess, so always hold.
pub(super) fn particle_update(
    system_flagged: impl Fn(&str) -> bool,
    any_attachments: bool,
    movement_flagged: impl Fn(&str) -> bool,
) -> (ParticleUpdate, bool) {
    let plain_movement =
        !(collides(&movement_flagged) || movement_flagged("swarm") || movement_flagged("wind"));
    if system_flagged("force cpu updating") {
        (ParticleUpdate::Cpu, false)
    } else if system_flagged("force gpu updating") {
        (ParticleUpdate::Gpu, false)
    } else if !any_attachments && plain_movement {
        (ParticleUpdate::Gpu, true)
    } else {
        (ParticleUpdate::Cpu, !any_attachments)
    }
}

/// The movements a Halo 3 or ODST emitter runs: tool's `postprocess_internal`
/// keeps one for each kind its own flags name (adding any the tag lacks) and
/// sorts them by `sub_140A846A0`'s table: physics, wind, swarm, collider. The
/// preview has no wind.
pub(super) fn flagged_controllers(flagged: impl Fn(&str) -> bool) -> Vec<ParticleController> {
    [
        (flagged("physics"), ParticleController::Physics),
        (flagged("swarm"), ParticleController::Flock),
        (collides(&flagged), ParticleController::Collider),
    ]
    .into_iter()
    .filter_map(|(on, controller)| on.then_some(controller))
    .collect()
}

/// The movement flags that each add a collider.
const COLLIDE_WITH: [&str; 6] = [
    "collide with structure",
    "collide with media",
    "collide with water",
    "collide with scenery",
    "collide with vehicles",
    "collide with bipeds",
];

/// Whether any of [`COLLIDE_WITH`] is set (Halo 2 and 3 call water `media`).
fn collides(flagged: impl Fn(&str) -> bool) -> bool {
    COLLIDE_WITH.iter().any(|flag| flagged(flag))
}

/// One parameter of a flagged movement kind as tool leaves it
/// (`sub_140A84700`): the emitter's own, else its template's and on up the
/// template chain (`sub_140A848F0`), else a constant 1.
pub(super) fn movement_parameter(
    reader: &TagReader,
    movement: TagStruct<'_>,
    kind: &str,
    id: i128,
    seeds: &mut SeedAllocator,
) -> Option<ParticleProperty> {
    movement_parameter_or(reader, movement, kind, id, 1.0, seeds)
}

/// [`movement_parameter`], with the constant tool fills in where it differs.
pub(super) fn movement_parameter_or(
    reader: &TagReader,
    movement: TagStruct<'_>,
    kind: &str,
    id: i128,
    default: f32,
    seeds: &mut SeedAllocator,
) -> Option<ParticleProperty> {
    chained_parameter(reader, movement, kind, id, seeds, MOST_TEMPLATES)
        .or_else(|| ParticleProperty::fixed(default))
}

/// How deep a template chain is followed, against one that names itself.
const MOST_TEMPLATES: usize = 8;

fn chained_parameter(
    reader: &TagReader,
    holder: TagStruct<'_>,
    kind: &str,
    id: i128,
    seeds: &mut SeedAllocator,
    depth: usize,
) -> Option<ParticleProperty> {
    let of_kind = |entry: &TagStruct<'_>| entry.read_enum_option("type").as_deref() == Some(kind);
    let entry = elements(holder, "movements").into_iter().find(of_kind);
    let parameters = entry
        .map(|entry| elements(entry, "parameters"))
        .unwrap_or_default();
    if let Some(found) = parameters
        .into_iter()
        .find(|parameter| parameter.read_int_any("parameter id") == Some(id))
    {
        return ParticleProperty::named(found, "property", seeds);
    }
    let path = holder
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty() && depth > 0)?;
    let template = reader.load(&path, "particle_physics", b"pmov").ok()?;
    chained_parameter(reader, template.root(), kind, id, seeds, depth - 1)
}

/// A particle's physical rotation at birth, in turns: random with `random
/// starting rotation`, else Halo 4's three-quarter turn for `bitmap authored
/// vertically` (`c_particle_definition::initialize_particle`, the 360's
/// 0x82AFA548; `particles.md` §1.3). Halo 4's shaders never read that flag,
/// so this is its only quarter turn; Halo 3 and Reach turn at the draw instead.
pub(super) fn starting_rotation(
    random_rotation: bool,
    quarter_turn: bool,
    random: impl FnOnce() -> f32,
) -> f32 {
    if random_rotation {
        random()
    } else if quarter_turn {
        0.75
    } else {
        0.0
    }
}

/// `initialize_particle`'s nudge to a speed of none, which what faces its
/// velocity needs one to have; see
/// [`ParticleRules::only_facing_still_particles_move`].
pub(super) fn still_particles_move(rules: &ParticleRules, billboard: Billboard) -> bool {
    use Billboard::{ScreenParallel, ScreenPerpendicular, VelocityHorizontal};
    !rules.only_facing_still_particles_move
        || matches!(
            billboard,
            ScreenParallel | ScreenPerpendicular | VelocityHorizontal
        )
}
