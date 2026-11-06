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

use super::passes::FramebufferBlend;
use super::super::common::fields::{elements, function_slot_of, has_flag, has_flag_after, object_block};
use super::lights::{CeFlareView, posed_on};
use super::particles::{CePointPhysics, CeSecondaryDraw, CeSecondaryMap, CeSpriteSheet, read_secondary_map, read_sprite_sheet};
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, dot, unit_or_unchanged};
use crate::app::model_preview::common::random::{high24_over_2_24, lcg_step};

/// How long a system runs before the preview first shows it.
const PRE_ROLL: f32 = 4.0;
/// `particle_system_new_particles` makes at most this many a call.
const MOST_A_CALL: usize = 128;
/// `particle_systems_initialize`'s `particle system particles` array, which
/// every system shares, attached or made by an effect.
pub(crate) const MOST_SYSTEM_PARTICLES: usize = 512;
/// `object_get_marker_by_name` finds at most this many.
const MOST_MARKERS: usize = 8;

/// A particle type's `flags`, by name. Each kind of state's
/// `... forward/backward` shares its name with the other's, so each is the
/// option after its loop.
#[derive(Debug, Clone, Copy)]
struct ParticleTypeFlags {
    type_states_loop: bool,
    type_states_bounce: bool,
    particle_states_loop: bool,
    particle_states_bounce: bool,
    die_in_air: bool,
    die_on_ground: bool,
    rotational_sideways: bool,
    disabled: bool,
    tint_by_effect_color: bool,
    initial_count_scales: bool,
    minimum_count_scales: bool,
    creation_rate_scales: bool,
    scale_scales: bool,
    animation_rate_scales: bool,
    rotation_rate_scales: bool,
}

impl ParticleTypeFlags {
    fn read(entry: TagStruct<'_>) -> Self {
        let set = |option: &str| has_flag(entry, "flags", option);
        Self {
            type_states_loop: set("type states loop"),
            type_states_bounce: has_flag_after(entry, "flags", "type states loop"),
            particle_states_loop: set("particle states loop"),
            particle_states_bounce: has_flag_after(entry, "flags", "particle states loop"),
            die_in_air: set("particles die in air"),
            die_on_ground: set("particles die on ground"),
            rotational_sideways: set("rotational sprites animate sideways"),
            disabled: set("disabled"),
            tint_by_effect_color: set("tint by effect color"),
            initial_count_scales: set("initial count scales with effect"),
            minimum_count_scales: set("minimum count scales with effect"),
            creation_rate_scales: set("creation rate scales with effect"),
            scale_scales: set("scale scales with effect"),
            animation_rate_scales: set("animation rate scales with effect"),
            rotation_rate_scales: set("rotation rate scales with effect"),
        }
    }
}

/// `particle creation physics`: where a new particle starts and how fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreationPhysics {
    Default,
    Explosion,
    Jet,
}

impl CreationPhysics {
    fn read(holder: TagStruct<'_>) -> Self {
        match holder.read_enum_option("particle creation physics").as_deref() {
            Some("explosion") => Self::Explosion,
            Some("jet") => Self::Jet,
            _ => Self::Default,
        }
    }
}

/// A type state's multipliers, as `particle_system_type_state_interpolated_variables`
/// keeps them: scale, animation rate, rotation rate, color (alpha, red,
/// green, blue), radius, minimum count, creation rate.
type TypeVariables = [f32; 10];
const SCALE: usize = 0;
const ANIMATION: usize = 1;
const ROTATION: usize = 2;
const COLOR: usize = 3;
const RADIUS: usize = 7;
const MINIMUM: usize = 8;
const RATE: usize = 9;

/// A particle's own: scale, animation rate, rotation rate, color (alpha,
/// red, green, blue).
type ParticleVariables = [f32; 7];

#[derive(Debug, Clone)]
struct TypeState {
    duration: [f32; 2],
    transition: [f32; 2],
    variables: TypeVariables,
    creation_physics: CreationPhysics,
}

#[derive(Debug, Clone)]
struct ParticleState {
    duration: [f32; 2],
    transition: [f32; 2],
    sheet: Arc<CeSpriteSheet>,
    /// Which of the system's sheets it is, for the textures.
    sheet_index: usize,
    sequence: i32,
    scale: [f32; 2],
    animation_rate: [f32; 2],
    rotation_rate: [f32; 2],
    /// Alpha, red, green, blue, lower and upper.
    colors: [[f32; 4]; 2],
    radius: f32,
    physics: Option<CePointPhysics>,
    blend: FramebufferBlend,
    map_flags: i128,
    nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    sort_bias: bool,
    /// The shader's second `bitmap`.
    secondary: Option<Arc<CeSecondaryMap>>,
}

#[derive(Debug, Clone)]
struct ParticleType {
    flags: ParticleTypeFlags,
    initial_count: i32,
    /// `complex sprite render modes` is `rotational`, not `simple`.
    rotational: bool,
    radius: f32,
    initial_physics: CreationPhysics,
    constants: [f32; 3],
    states: Vec<TypeState>,
    particle_states: Vec<ParticleState>,
}

/// A `particle_system` tag.
#[derive(Debug, Clone)]
pub(crate) struct CePctlDef {
    types: Vec<ParticleType>,
    /// `point physics`, which moves a system on no object.
    physics: Option<CePointPhysics>,
    /// Every bitmap its particle states name, for the textures.
    pub(crate) sheets: Vec<Arc<CeSpriteSheet>>,
}

/// A particle system on one of the object's attachments.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleSystemAt {
    /// Which object's functions and change colors drive it; 0 is the opened.
    pub(crate) object: u8,
    pub(crate) def: Arc<CePctlDef>,
    /// The attachment's marker, each place it is on the model.
    markers: Vec<(usize, [f32; 3], [f32; 3])>,
    primary: Option<usize>,
    change_color: Option<usize>,
}

impl super::functions::ReadsOutputs for CeParticleSystemAt {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        self.primary.into_iter().collect()
    }
}

/// The most states one step may pass through. The game has no such bound
/// (`particle_system_update`), but a state whose durations are all zero or
/// less would never let it out; tool rejects such tags, the editor does not.
const MOST_STATE_CHANGES: u32 = 256;

/// Counts a state change; past [`MOST_STATE_CHANGES`] the step stops where it
/// is, as caught up.
fn caught_up(changes: &mut u32, time_left: &mut f32) -> bool {
    *changes += 1;
    if *changes > MOST_STATE_CHANGES {
        *time_left = 0.0;
        return true;
    }
    false
}

fn bounds_of(holder: TagStruct<'_>, name: &str) -> [f32; 2] {
    match holder.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealBounds(bounds) | TagFieldData::AngleBounds(bounds) | TagFieldData::FractionBounds(bounds)) => {
            [bounds.lower, bounds.upper]
        }
        _ => [0.0; 2],
    }
}

fn argb_of(holder: TagStruct<'_>, name: &str) -> [f32; 4] {
    let [red, green, blue, alpha] = super::passes::argb(holder, name);
    [alpha, red, green, blue]
}

fn constants_of(holder: TagStruct<'_>) -> [f32; 3] {
    let found = elements(holder, "physics constants");
    std::array::from_fn(|at| found.get(at).and_then(|constant| constant.read_real("k")).unwrap_or(0.0))
}

fn read_physics(source: &TagSource, holder: TagStruct<'_>) -> Option<CePointPhysics> {
    let path = holder.read_tag_ref_path("point physics").filter(|path| !path.is_empty())?;
    let tag = load_referenced_tag_from_source(source, &path, "point_physics", b"pphy").ok()?;
    Some(CePointPhysics::read(&tag))
}

pub(in crate::app::model_preview) fn read_pctl_def(source: &TagSource, path: &str) -> Option<CePctlDef> {
    let tag = load_referenced_tag_from_source(source, path, "particle_system", b"pctl").ok()?;
    let root = tag.root();
    let mut sheets: Vec<Arc<CeSpriteSheet>> = Vec::new();
    let mut sheet_paths: Vec<String> = Vec::new();
    let mut types = Vec::new();
    for entry in elements(root, "particle types") {
        let int = |name: &str| entry.read_int_any(name).unwrap_or(0);
        let states = elements(entry, "states")
            .iter()
            .map(|state| {
                let real = |name: &str| state.read_real(name).unwrap_or(0.0);
                // Tool's `sub_49BFC0`.
                let radius = Some(real("radius multiplier")).filter(|radius| *radius > 0.0).unwrap_or(1.0);
                let [red, green, blue, alpha] = super::passes::argb(*state, "color multiplier");
                TypeState {
                    duration: bounds_of(*state, "duration bounds"),
                    transition: bounds_of(*state, "transition time bounds"),
                    variables: [
                        real("scale multiplier"),
                        real("animation_rate_multiplier"),
                        real("rotation rate multiplier"),
                        alpha,
                        red,
                        green,
                        blue,
                        radius,
                        real("minimum particle count"),
                        real("particle creation rate"),
                    ],
                    creation_physics: CreationPhysics::read(*state),
                }
            })
            .collect();
        let mut particle_states = Vec::new();
        for state in elements(entry, "particle states") {
            let Some(bitmap) = state.read_tag_ref_path("bitmaps").filter(|path| !path.is_empty()) else {
                continue;
            };
            let sheet_index = match sheet_paths.iter().position(|known| *known == bitmap) {
                Some(at) => at,
                None => {
                    let Some(sheet) = read_sprite_sheet(source, &bitmap) else {
                        continue;
                    };
                    sheets.push(Arc::new(sheet));
                    sheet_paths.push(bitmap);
                    sheets.len() - 1
                }
            };
            let real = |name: &str| state.read_real(name).unwrap_or(0.0);
            particle_states.push(ParticleState {
                duration: bounds_of(state, "duration bounds"),
                transition: bounds_of(state, "transition time bounds"),
                sheet: sheets[sheet_index].clone(),
                sheet_index,
                sequence: state.read_int_any("sequence index").unwrap_or(0) as i32,
                scale: bounds_of(state, "scale"),
                animation_rate: bounds_of(state, "animation rate"),
                rotation_rate: bounds_of(state, "rotation rate"),
                colors: [argb_of(state, "color 1"), argb_of(state, "color 2")],
                // Tool's `sub_49BFE0`.
                radius: Some(real("radius multiplier")).filter(|radius| *radius > 0.0).unwrap_or(1.0),
                physics: read_physics(source, state),
                blend: FramebufferBlend::read(state),
                map_flags: state.read_int_any("map flags").unwrap_or(0),
                nonlinear_tint: has_flag(state, "shader flags", "nonlinear tint"),
                sort_bias: has_flag(state, "shader flags", "sort bias"),
                secondary: read_secondary_map(source, state, "bitmap"),
            });
        }
        types.push(ParticleType {
            flags: ParticleTypeFlags::read(entry),
            initial_count: int("initial particle count") as i32,
            rotational: entry.field("complex sprite render modes").is_some_and(|field| field.enum_is("rotational")),
            radius: entry.read_real("radius").unwrap_or(0.0),
            initial_physics: CreationPhysics::read(entry),
            constants: constants_of(entry),
            states,
            particle_states,
        });
    }
    // `particle_system_initialize`: every type needs a state, and one at least.
    if types.is_empty() || types.iter().any(|kind| kind.states.is_empty()) {
        return None;
    }
    Some(CePctlDef { types, physics: read_physics(source, root), sheets })
}

/// The particle systems among the object's attachments.
pub(in crate::app::model_preview) fn read_ce_particle_systems(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeParticleSystemAt> {
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"pctl");
    let mut systems = Vec::new();
    for attachment in elements(block, "attachments") {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if kind != group || path.is_empty() {
            continue;
        }
        let Some(def) = read_pctl_def(source, &path) else {
            continue;
        };
        let name = attachment.read_string("marker").unwrap_or_default();
        // `object_get_marker_by_name`: no name is the origin.
        let found: Vec<(usize, [f32; 3], [f32; 3])> = if name.is_empty() {
            vec![(0, [0.0; 3], [1.0, 0.0, 0.0])]
        } else {
            markers
                .iter()
                .filter(|marker| marker.name.eq_ignore_ascii_case(&name))
                .take(MOST_MARKERS)
                .map(|marker| (marker.node_index.max(0) as usize, marker.position, marker.axes[0]))
                .collect()
        };
        systems.push(CeParticleSystemAt {
            object: owner,
            def: Arc::new(def),
            markers: found,
            primary: function_slot_of(attachment, "primary scale"),
            change_color: function_slot_of(attachment, "change color"),
        });
    }
    systems
}

/// `ps_particle_datum`.
#[derive(Debug, Clone)]
struct PsParticle {
    forward: bool,
    valid: bool,
    state: Option<usize>,
    transition: Option<usize>,
    time_left: f32,
    length: f32,
    position: [f32; 3],
    velocity: [f32; 3],
    axis: [f32; 3],
    rotation: f32,
    /// -1 until it is first drawn, which picks one at random.
    sprite: f32,
    variables: ParticleVariables,
    transition_variables: ParticleVariables,
}

/// `particle_type`.
#[derive(Debug, Clone, Default)]
struct RunningType {
    state: Option<usize>,
    transition: Option<usize>,
    time_left: f32,
    length: f32,
    variables: TypeVariables,
    fractional: f32,
    forward: bool,
    /// The first made is last: each new one goes in front.
    particles: VecDeque<PsParticle>,
}

/// `particle_system_datum`.
#[derive(Debug, Clone)]
pub(crate) struct CeRunningSystem {
    on: bool,
    scale: f32,
    /// Alpha, red, green, blue.
    color: [f32; 4],
    /// What `light_particle` found under it; alpha-blended sprites take it.
    lighting: [f32; 3],
    /// On an object, which its type states may loop on; else on its own.
    attached: bool,
    position: [f32; 3],
    velocity: [f32; 3],
    types: Vec<RunningType>,
    first: bool,
}

impl CeRunningSystem {
    /// How many particles it holds.
    pub(crate) fn particle_count(&self) -> usize {
        self.types.iter().map(|run| run.particles.len()).sum()
    }
}

#[derive(Debug)]
pub(crate) struct Random(u32);

impl Random {
    pub(crate) fn seeded(seed: u32) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high24_over_2_24(self.0)
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    /// `seed_random_range`: an integer in [low, high).
    fn index(&mut self, high: usize) -> usize {
        ((self.next() * high as f32) as usize).min(high.saturating_sub(1))
    }

    fn direction(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let (sin, cos) = self.range(0.0, std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }
}

/// Every particle system of the object, carried from frame to frame.
#[derive(Debug, Default)]
pub(crate) struct CeParticleSystemField {
    time: Option<f32>,
    systems: Vec<CeRunningSystem>,
    random: Option<Random>,
}




/// `randomize_particle_variables`: scale, rates and alpha at random, the
/// color's channels together.
fn randomize(state: &ParticleState, random: &mut Random) -> ParticleVariables {
    let together = random.range(0.0, 1.0);
    let animation = random.range(state.animation_rate[0], state.animation_rate[1]);
    let rotation = random.range(state.rotation_rate[0], state.rotation_rate[1]);
    let scale = random.range(state.scale[0], state.scale[1]);
    let [low, high] = state.colors;
    let alpha = random.range(low[0], high[0]);
    [
        scale,
        animation,
        rotation,
        alpha,
        (high[1] - low[1]) * together + low[1],
        (high[2] - low[2]) * together + low[2],
        (high[3] - low[3]) * together + low[3],
    ]
}

/// `particle_system_next_type_state_index` and
/// `particle_system_next_particle_state_index`: the next state, round again
/// or back again if the flags say, else none.
fn next_state(state: usize, forward: &mut bool, count: usize, looping: bool, bounce: bool) -> Option<usize> {
    let step: isize = if *forward { 1 } else { -1 };
    let next = state as isize + step;
    if next >= 0 && (next as usize) < count {
        return Some(next as usize);
    }
    if !looping || count == 0 {
        return None;
    }
    if bounce {
        *forward = !*forward;
        let back = state as isize - step;
        return Some(if back >= 0 { (back as usize).min(count - 1) } else { 0 });
    }
    Some(0)
}

impl CeParticleSystemField {
    /// How many particles its systems hold.
    pub(crate) fn particle_count(&self) -> usize {
        self.systems.iter().map(CeRunningSystem::particle_count).sum()
    }

    /// Step every system to `time`, a frame at a time as `game_frame` runs
    /// `particle_systems_update`; `others` particles of the shared array are
    /// in effects' systems.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        systems: &[CeParticleSystemAt],
        inputs: impl Fn(u8) -> ([f32; 4], [bool; 4], [[f32; 3]; 4]),
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
        others: usize,
    ) {
        if self.systems.len() != systems.len() || self.time.is_none_or(|last| time < last) {
            let mut random = Random(0x5eed_1234);
            self.systems = systems.iter().map(|system| new_system(&system.def, true, &mut random)).collect();
            self.random = Some(random);
            self.time = Some(time - PRE_ROLL);
        }
        let Some(now) = self.time else {
            return;
        };
        let now = resume_point(now, time, PRE_ROLL);
        let mut random = self.random.take().unwrap_or(Random(1));
        for step in frame_steps(now, time) {
            let mut room = MOST_SYSTEM_PARTICLES.saturating_sub(self.particle_count() + others);
            for (system, running) in systems.iter().zip(&mut self.systems) {
                let (outputs, active, change_colors) = inputs(system.object);
                let dt = if running.first { 0.001 } else { step };
                let inputs = (&outputs, &active, &change_colors);
                update_system(system, running, inputs, bones, ground, dt, &mut random, &mut room);
                running.first = false;
            }
        }
        self.time = Some(time);
        self.random = Some(random);
    }

    /// `particle_system_render`: each particle a sprite, or two while it
    /// passes between states that draw differently. `pixel` is the world
    /// size of a pixel at a place, for a scale of nothing. Gives the system,
    /// the sheet, the image, the blend, the nonlinear tint, the color and
    /// the corners.
    pub(crate) fn quads(
        &mut self,
        systems: &[CeParticleSystemAt],
        view: &CeFlareView,
        pixel: impl Fn([f32; 3]) -> f32,
    ) -> Vec<CePctlQuad> {
        let mut quads = Vec::new();
        let mut random = self.random.take().unwrap_or(Random(1));
        let time = self.time.unwrap_or(0.0);
        for (index, (system, running)) in systems.iter().zip(&mut self.systems).enumerate() {
            system_quads(&system.def, running, index, view, &pixel, &mut random, time, &mut quads);
        }
        self.random = Some(random);
        quads
    }
}

/// `particle_system_render` for one system.
#[allow(clippy::too_many_arguments)]
fn system_quads(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    index: usize,
    view: &CeFlareView,
    pixel: &impl Fn([f32; 3]) -> f32,
    random: &mut Random,
    time: f32,
    quads: &mut Vec<CePctlQuad>,
) {
    let lighting = running.lighting;
    for (kind, run) in def.types.iter().zip(&mut running.types) {
        if run.state.is_none() || kind.flags.disabled {
            continue;
        }
        let multipliers = run.variables;
        for particle in &mut run.particles {
            if !particle.valid {
                continue;
            }
            let Some(current) = particle.state.and_then(|at| kind.particle_states.get(at)) else {
                continue;
            };
            let next = particle.transition.and_then(|at| kind.particle_states.get(at));
            let mut fade = 1.0;
            if next.is_some() {
                fade = if particle.length != 0.0 { (particle.time_left / particle.length).clamp(0.0, 1.0) } else { 0.0 };
            }
            let mix = |at: usize| match next {
                Some(_) => particle.transition_variables[at] * (1.0 - fade) + particle.variables[at] * fade,
                None => particle.variables[at],
            };
            let mut color: [f32; 4] = std::array::from_fn(|channel| mix(3 + channel) * multipliers[COLOR + channel]);
            // Alpha blend takes the light where the system is.
            if current.blend == FramebufferBlend::AlphaBlend {
                for channel in 0..3 {
                    color[1 + channel] *= lighting[channel];
                }
            }
            let scale = mix(0) * multipliers[SCALE];
            let mut fade_next = 0.0;
            match next {
                Some(next)
                    if next.blend != current.blend
                        || next.map_flags != current.map_flags
                        || next.sequence != current.sequence => fade_next = 1.0 - fade,
                _ => fade = 1.0,
            }
            let sequence = current.sequence + i32::from(kind.rotational);
            let count = current.sheet.count(sequence);
            if count == 0 {
                continue;
            }
            let sprite = if particle.sprite == -1.0 {
                let at = random.index(count);
                particle.sprite = at as f32;
                at
            } else {
                (particle.sprite as i32).rem_euclid(count as i32) as usize
            };
            let size = if scale == 0.0 { pixel(particle.position) } else { scale };
            for (state, fade, nudge) in [(current, fade, 0.0), (next.unwrap_or(current), fade_next, 0.001)] {
                if fade <= 0.01 {
                    continue;
                }
                // The second sits a thousandth nearer, in view space.
                let center: [f32; 3] = std::array::from_fn(|axis| particle.position[axis] - view.forward[axis] * nudge);
                let group = quads.len();
                let shown = Shown {
                    state,
                    time,
                    radius: current.radius,
                    center,
                    axis: particle.axis,
                    rotation: particle.rotation,
                    size,
                    color,
                    fade,
                    group,
                };
                if kind.rotational {
                    shown.rotational(sprite, kind.flags.rotational_sideways, view, index, quads);
                } else {
                    shown.simple(sprite, view, index, quads);
                }
            }
        }
    }
}

/// A sprite `particle_system_render` builds.
#[derive(Debug, Clone)]
pub(crate) struct CePctlQuad {
    pub(crate) system: usize,
    /// Shared by the quads of one draw of one particle state, which
    /// `particle_system_render` sends as a batch of its own.
    pub(crate) group: usize,
    pub(crate) sheet: usize,
    pub(crate) image: usize,
    pub(crate) blend: FramebufferBlend,
    pub(crate) nonlinear_tint: bool,
    pub(crate) sort_bias: bool,
    pub(crate) secondary: Option<CeSecondaryDraw>,
    pub(crate) color: [f32; 4],
    pub(crate) corners: [[f32; 5]; 4],
}

/// What one draw of a particle takes.
struct Shown<'a> {
    state: &'a ParticleState,
    /// The game's time, for the second map's animation.
    time: f32,
    /// The current state's radius multiplier.
    radius: f32,
    center: [f32; 3],
    axis: [f32; 3],
    rotation: f32,
    size: f32,
    /// Alpha, red, green, blue.
    color: [f32; 4],
    fade: f32,
    /// What its quads carry as [`CePctlQuad::group`].
    group: usize,
}

impl Shown<'_> {
    /// `build_sprite`'s color: the fade is the alpha unless the blend is alpha
    /// blend or the tint nonlinear.
    #[allow(clippy::too_many_arguments)]
    fn push(&self, sequence: i32, sprite: usize, rotation: f32, fade: f32, mirror: bool, view: &CeFlareView, system: usize, quads: &mut Vec<CePctlQuad>) {
        let Some((image, corners)) = self.state.sheet.corners(sequence, sprite, self.center, rotation, self.size, mirror, view) else {
            return;
        };
        let [alpha, red, green, blue] = self.color;
        let alpha = if self.state.blend != FramebufferBlend::AlphaBlend && !self.state.nonlinear_tint { fade } else { alpha * fade };
        // `particle_system_render` gives the shader the first state's radius
        // multiplier for its second map's radius, for either draw.
        let secondary = self.state.secondary.as_ref().map(|map| map.draw(self.time, self.radius));
        quads.push(CePctlQuad {
            system,
            group: self.group,
            sheet: self.state.sheet_index,
            image,
            blend: self.state.blend,
            nonlinear_tint: self.state.nonlinear_tint,
            sort_bias: self.state.sort_bias,
            secondary,
            color: [red, green, blue, alpha.clamp(0.0, 1.0)],
            corners,
        });
    }

    fn simple(&self, sprite: usize, view: &CeFlareView, system: usize, quads: &mut Vec<CePctlQuad>) {
        self.push(self.state.sequence, sprite, self.rotation, self.fade, false, view, system, quads);
    }

    /// `build_sprite_rotational`: end on, the next sequence's sprite; side
    /// on, the sequence's sprite for the angle it turns through, laid along
    /// the axis on screen; between, both.
    fn rotational(&self, sprite: usize, sideways: bool, view: &CeFlareView, system: usize, quads: &mut Vec<CePctlQuad>) {
        let sheet = &self.state.sheet;
        let first = self.state.sequence;
        let toward = unit_or_unchanged(std::array::from_fn(|axis| self.center[axis] - view.eye[axis]));
        let axis = unit_or_unchanged(self.axis);
        let angle = dot(toward, axis).clamp(-1.0, 1.0).acos();
        let off = angle - std::f32::consts::FRAC_PI_2;
        let end_on = (off * off * 0.405_284_7).clamp(0.0, 1.0);
        let turns = self.rotation * 0.159_154_94;
        // `fmod` keeps the sign, and `build_sprite` draws no sprite below 0.
        if end_on > 0.05 {
            let (sprite, rotation, mirror) = if sideways {
                let count = sheet.count(first + 1) as f32;
                let at = ((count * turns + 0.5) % count + sprite as f32) as i32;
                (if off < 0.0 { count as i32 - sprite as i32 } else { at }, 0.0, false)
            } else {
                (sprite as i32, self.rotation, off < 0.0)
            };
            if let Ok(sprite) = usize::try_from(sprite) {
                self.push(first + 1, sprite, rotation, end_on * self.fade, mirror, view, system, quads);
            }
        }
        if 1.0 - end_on > 0.05 {
            let count = sheet.count(first) as f32;
            let at = ((count * turns + 0.5) % count) as i32;
            let screen = [dot(axis, view.right), dot(axis, view.up)];
            let rotation = screen[1].atan2(screen[0]);
            if let Ok(at) = usize::try_from(at) {
                self.push(first, at, rotation, (1.0 - end_on) * self.fade, false, view, system, quads);
            }
        }
    }
}

/// `particle_system_new_attached` and `particle_system_initialize`: every
/// type in its first state for a time from its bounds.
fn new_system(def: &CePctlDef, attached: bool, random: &mut Random) -> CeRunningSystem {
    let types = def
        .types
        .iter()
        .map(|kind| {
            let first = &kind.states[0];
            let length = random.range(first.duration[0], first.duration[1]);
            RunningType { state: Some(0), time_left: length, length, forward: true, ..RunningType::default() }
        })
        .collect();
    CeRunningSystem {
        on: true,
        scale: 1.0,
        color: [1.0; 4],
        lighting: [1.0; 3],
        attached,
        position: [0.0; 3],
        velocity: [0.0; 3],
        types,
        first: true,
    }
}

/// `particle_system_new_unattached`, as an effect's part makes one: at a
/// point, going, tinted and scaled as the effect says, lit by what
/// `light_particle` finds below it, which in a preview is nothing: grey.
/// `particle_system_initialize` steps it a thousandth of a second at once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_unattached(
    def: &CePctlDef,
    position: [f32; 3],
    velocity: [f32; 3],
    color: [f32; 4],
    scale: f32,
    ground: f32,
    random: &mut Random,
    room: &mut usize,
) -> CeRunningSystem {
    let mut running = new_system(def, false, random);
    running.position = position;
    running.velocity = velocity;
    running.color = color;
    running.scale = scale;
    // `light_particle` with no structure under it: `global_real_rgb_grey`.
    running.lighting = [0.5; 3];
    update_unattached(def, &mut running, ground, 0.001, random, room);
    running.first = false;
    running
}

/// `particle_system_update` for a system on no object: its own point physics
/// moves it, and it makes particles where it is. False once no type is in
/// a state, when `particle_system_delete` takes it. It makes no more than
/// `room` particles, which it takes from.
pub(crate) fn update_unattached(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    ground: f32,
    dt: f32,
    random: &mut Random,
    room: &mut usize,
) -> bool {
    // `particle_system_update_default`, which explosion uses too.
    if let Some(physics) = def.physics {
        physics.step(&mut running.position, &mut running.velocity, 1.0, ground, dt);
    }
    let markers = [(running.position, [0.0; 3])];
    step_types(def, running, &markers, ground, dt, random, room)
}

/// `particle_system_render` for a system on no object.
pub(crate) fn unattached_quads(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    index: usize,
    view: &CeFlareView,
    pixel: impl Fn([f32; 3]) -> f32,
    random: &mut Random,
    time: f32,
) -> Vec<CePctlQuad> {
    let mut quads = Vec::new();
    system_quads(def, running, index, view, &pixel, random, time, &mut quads);
    quads
}

/// What an object's functions give its systems: its outputs, which are on,
/// and its change colors.
type SystemInputs<'a> = (&'a [f32; 4], &'a [bool; 4], &'a [[f32; 3]; 4]);

#[allow(clippy::too_many_arguments)]
fn update_system(
    system: &CeParticleSystemAt,
    running: &mut CeRunningSystem,
    (outputs, active, change_colors): SystemInputs<'_>,
    bones: Option<&[[f32; 4]]>,
    ground: f32,
    dt: f32,
    random: &mut Random,
    room: &mut usize,
) {
    // `object_get_function_value`: none is on at one.
    (running.scale, running.on) = match system.primary {
        Some(slot) => (outputs[slot], active[slot]),
        None => (1.0, true),
    };
    running.color = match system.change_color {
        Some(slot) => [1.0, change_colors[slot][0], change_colors[slot][1], change_colors[slot][2]],
        None => [1.0; 4],
    };
    let markers: Vec<([f32; 3], [f32; 3])> = system
        .markers
        .iter()
        .map(|&(node, position, forward)| posed_on(node, position, forward, bones))
        .collect();
    step_types(&system.def, running, &markers, ground, dt, random, room);
}

/// The types of a system through their states, making and moving their
/// particles: true while any type is in a state.
fn step_types(
    def: &CePctlDef,
    running: &mut CeRunningSystem,
    markers: &[([f32; 3], [f32; 3])],
    ground: f32,
    dt: f32,
    random: &mut Random,
    room: &mut usize,
) -> bool {
    let mut active = false;
    let (velocity, attached) = (running.velocity, running.attached);
    for (kind, run) in def.types.iter().zip(&mut running.types) {
        if kind.flags.disabled {
            continue;
        }
        run.time_left -= dt;
        let mut changes = 0;
        while let Some(state) = run.state {
            if run.time_left >= 0.0 {
                break;
            }
            if caught_up(&mut changes, &mut run.time_left) {
                break;
            }
            let bounds = match run.transition {
                None => {
                    run.transition = next_state(
                        state,
                        &mut run.forward,
                        kind.states.len(),
                        // A system on no object never goes round again.
                        kind.flags.type_states_loop && attached,
                        kind.flags.type_states_bounce,
                    );
                    if run.transition.is_none() {
                        run.state = None;
                    }
                    kind.states[state].transition
                }
                Some(next) => {
                    run.state = Some(next);
                    run.transition = None;
                    kind.states[next].duration
                }
            };
            let length = random.range(bounds[0], bounds[1]);
            run.time_left += length;
            run.length = length;
        }
        let Some(state) = run.state else {
            continue;
        };
        run.variables = match run.transition {
            None => kind.states[state].variables,
            Some(next) => {
                let t = if run.length != 0.0 { (run.time_left / run.length).clamp(0.0, 1.0) } else { 0.0 };
                std::array::from_fn(|at| kind.states[state].variables[at] * t + kind.states[next].variables[at] * (1.0 - t))
            }
        };
        if kind.flags.tint_by_effect_color {
            for channel in 0..4 {
                run.variables[COLOR + channel] *= running.color[channel];
            }
        }
        let scale = running.scale;
        let flags = &kind.flags;
        for (scales, at) in [
            (flags.minimum_count_scales, MINIMUM),
            (flags.creation_rate_scales, RATE),
            (flags.scale_scales, SCALE),
            (flags.animation_rate_scales, ANIMATION),
            (flags.rotation_rate_scales, ROTATION),
        ] {
            if scales {
                run.variables[at] *= scale;
            }
        }
        if running.on {
            new_particles(kind, run, running.first, scale, markers, velocity, dt, random, room);
        }
        update_particles(kind, run, ground, dt, random);
        active = true;
    }
    active
}

/// `particle_system_new_particles`: the type's initial count the first time,
/// else what its creation rate makes of the time, at random among the
/// attachment's markers.
#[allow(clippy::too_many_arguments)]
fn new_particles(
    kind: &ParticleType,
    run: &mut RunningType,
    first: bool,
    scale: f32,
    markers: &[([f32; 3], [f32; 3])],
    system_velocity: [f32; 3],
    dt: f32,
    random: &mut Random,
    room: &mut usize,
) {
    let Some(state) = run.state else {
        return;
    };
    let wanted = if first {
        if kind.flags.initial_count_scales {
            (kind.initial_count as f32 * scale + 0.5) as i32
        } else {
            kind.initial_count
        }
    } else {
        let made = run.variables[RATE] * dt;
        let whole = made as i32;
        let mut wanted = run.particles.len() as i32 + whole;
        run.fractional += made - whole as f32;
        if run.fractional > 1.0 {
            run.fractional -= 1.0;
            wanted += 1;
        }
        wanted
    };
    let physics = if first { kind.initial_physics } else { kind.states[state].creation_physics };
    let mut made = 0;
    while (run.particles.len() as i32) < wanted && !markers.is_empty() && made < MOST_A_CALL && *room > 0 {
        made += 1;
        *room -= 1;
        let rotation = random.range(0.0, std::f32::consts::TAU);
        let (position, forward) = markers[random.index(markers.len())];
        let (position, mut velocity, axis) = match physics {
            CreationPhysics::Explosion => explosion(kind, position, random),
            CreationPhysics::Jet => jet(kind, position, forward, random),
            CreationPhysics::Default => (position, [0.0; 3], [0.0; 3]),
        };
        // Each creation adds the system's own velocity.
        velocity = std::array::from_fn(|axis| velocity[axis] + system_velocity[axis]);
        run.particles.push_front(PsParticle {
            forward: true,
            valid: true,
            state: None,
            transition: None,
            time_left: 0.0,
            length: 0.0,
            position,
            velocity,
            axis,
            rotation,
            sprite: -1.0,
            variables: [0.0; 7],
            transition_variables: [0.0; 7],
        });
    }
    // Short of the minimum, the state hurries on.
    if (run.particles.len() as f32) < run.variables[MINIMUM] {
        run.time_left *= 0.3;
    }
}

/// `particle_system_new_particle_explosion`: out from the marker at random,
/// spread across by the first constant and up by the second, going at the
/// third times that; the axis lies flat, square to the way out.
fn explosion(kind: &ParticleType, marker: [f32; 3], random: &mut Random) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let [across, up, speed] = kind.constants;
    let direction = random.direction();
    let out = [direction[0] * across, direction[1] * across, direction[2] * up];
    let position = std::array::from_fn(|axis| marker[axis] + out[axis]);
    let velocity = out.map(|axis| axis * speed);
    // `rotate_vector_about_axis` by a quarter turn about up.
    let axis = [-out[1], out[0], 0.0];
    (position, velocity, axis)
}

/// `particle_system_new_particle_jet`: along the marker's forward at the
/// first constant per tick, a share (the second) of it at random; the axis
/// is square to the way it goes and the forward, or to up when the third
/// constant is set.
fn jet(kind: &ParticleType, marker: [f32; 3], forward: [f32; 3], random: &mut Random) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let [speed, spread, upright] = kind.constants;
    let per_tick = speed * 0.033_333_335;
    let scattered = spread * per_tick;
    let straight = (1.0 - spread) * per_tick;
    let direction = random.direction();
    let velocity: [f32; 3] = std::array::from_fn(|axis| direction[axis] * scattered + forward[axis] * straight);
    let axis = if upright == 0.0 { cross(forward, velocity) } else { cross(velocity, [0.0, 0.0, 1.0]) };
    (marker, velocity, axis)
}

/// The particles of one type through their states, turned, animated and
/// moved; those out of states, or killed where the flags say, go.
fn update_particles(kind: &ParticleType, run: &mut RunningType, ground: f32, dt: f32, random: &mut Random) {
    let multipliers = run.variables;
    let states = &kind.particle_states;
    run.particles.retain_mut(|particle| {
        particle.time_left -= dt;
        if particle.state.is_none() && !states.is_empty() {
            particle.state = Some(0);
            let length = random.range(states[0].duration[0], states[0].duration[1]);
            particle.time_left = length;
            particle.length = length;
            particle.variables = randomize(&states[0], random);
        }
        if !particle.valid {
            particle.state = None;
        }
        let mut changes = 0;
        while let Some(state) = particle.state {
            if particle.time_left >= 0.0 {
                break;
            }
            if caught_up(&mut changes, &mut particle.time_left) {
                break;
            }
            let bounds = match particle.transition {
                None => {
                    particle.transition = next_state(
                        state,
                        &mut particle.forward,
                        states.len(),
                        kind.flags.particle_states_loop,
                        kind.flags.particle_states_bounce,
                    );
                    if particle.transition.is_none() {
                        particle.state = None;
                    }
                    states[state].transition
                }
                Some(next) => {
                    particle.state = Some(next);
                    particle.transition = None;
                    states[next].duration
                }
            };
            let length = random.range(bounds[0], bounds[1]);
            particle.time_left += length;
            particle.length = length;
            match particle.transition {
                None => particle.variables = particle.transition_variables,
                Some(next) => particle.transition_variables = randomize(&states[next], random),
            }
        }
        let Some(state) = particle.state else {
            return false;
        };
        let t = if particle.length != 0.0 { (particle.time_left / particle.length).clamp(0.0, 1.0) } else { 0.0 };
        let rate = |at: usize| match particle.transition {
            None => particle.variables[at],
            Some(_) => particle.transition_variables[at] * (1.0 - t) + particle.variables[at] * t,
        };
        particle.rotation += rate(ROTATION) * multipliers[ROTATION] * dt;
        particle.sprite += rate(ANIMATION) * multipliers[ANIMATION] * dt;
        // `particle_system_update_particle_default`.
        let current = &states[state];
        let (physics, radius) = match particle.transition.map(|next| &states[next]) {
            None => (current.physics, current.radius * multipliers[RADIUS] * kind.radius),
            Some(next) => {
                let physics = match (current.physics, next.physics) {
                    (Some(a), Some(b)) => Some(a.lerp(&b, t)),
                    (a, b) => a.or(b),
                };
                (physics, (current.radius * t + next.radius * (1.0 - t)) * multipliers[RADIUS] * kind.radius)
            }
        };
        if let Some(physics) = physics {
            // Always in air: the preview has no water. Touching the floor is
            // touching structure.
            let grounded = physics.step(&mut particle.position, &mut particle.velocity, radius, ground, dt);
            if kind.flags.die_in_air || (grounded && kind.flags.die_on_ground) {
                particle.valid = false;
            }
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_end_go_round_or_come_back_as_the_flags_say() {
        let mut forward = true;
        assert_eq!(next_state(0, &mut forward, 3, false, false), Some(1));
        assert_eq!(next_state(2, &mut forward, 3, false, false), None);
        assert_eq!(next_state(2, &mut forward, 3, true, false), Some(0));
        assert!(forward);
        assert_eq!(next_state(2, &mut forward, 3, true, true), Some(1));
        assert!(!forward);
        assert_eq!(next_state(1, &mut forward, 3, true, true), Some(0));
        assert_eq!(next_state(0, &mut forward, 3, true, true), Some(1));
        assert!(forward);
    }
}
