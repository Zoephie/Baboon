//! Halo CE's effects on an object: the `effect` tags `attachments_new` hangs
//! on a marker through `effect_new_looping`, and the `particle`s their events
//! let go (`effect_generate_particles`), stepped by `particles_update` and drawn
//! by `render_particles` through the `effect` shaders of the kit's `fx` folder.
//! Everything read from the Xbox 360 build's symbols (HCEX_Release); what the
//! tags on disk lack, tool's postprocess (`haloce_mcc_tool`) bakes as noted.

use std::sync::Arc;

use blam_tags::{TagFieldData, TagFile, TagStruct};

use super::ce_attachments::{elements, has_flag, object_block};
use super::ce_lights::{CeEffectLightDef, CeFlareView, CeLight, posed_on, read_effect_light};
use super::ce_particle_systems::{
    CePctlDef, CePctlQuad, CeRunningSystem, MOST_SYSTEM_PARTICLES, Random as SystemRandom, new_unattached, read_pctl_def,
    unattached_quads, update_unattached,
};
use super::ce_passes::{CeMapAnimation, argb, read_effect_map_animation};
use super::h2_passes::{H2Image, ce_color_interpolate};
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// A game tick, which is what `effects_update` steps by.
const STEP: f32 = 1.0 / 30.0;
const PRE_ROLL: f32 = 4.0;
/// `effect_update` runs at most this many events in a tick.
const MOST_EVENTS_A_TICK: usize = 8;
/// The game's pool is shared by the whole level.
const MOST_PARTICLES: usize = 1024;
/// `physics_constants_reset`: gravity, and air in a level without water.
const GRAVITY: f32 = 4.171_259_4;
const AIR_MASS_OVER_RADIUS_CUBED: f32 = 0.0011 * MASS_OF_DENSITY;
/// What `point_physics` postprocess (`sub_525660`) makes of a density.
const MASS_OF_DENSITY: f32 = 0.001 * 4.188_790_3 * 28_316_846.0;

/// `particle` flags.
const CAN_ANIMATE_BACKWARDS: i128 = 0x1;
const ANIMATION_STOPS_AT_REST: i128 = 0x2;
const STARTS_ON_RANDOM_FRAME: i128 = 0x4;
const ANIMATE_ONCE_PER_FRAME: i128 = 0x8;
const DIES_AT_REST: i128 = 0x10;
const DIES_ON_STRUCTURE: i128 = 0x20;
const DIES_IN_AIR: i128 = 0x100;
const TINT_FROM_DIFFUSE: i128 = 0x40;
const SELF_ILLUMINATED: i128 = 0x200;
const RANDOM_HORIZONTAL_MIRROR: i128 = 0x400;
const RANDOM_VERTICAL_MIRROR: i128 = 0x800;

/// `particle_datum` flags.
const BACKWARDS: u8 = 1;
const AT_REST: u8 = 2;
const MIRROR_U: u8 = 4;
const MIRROR_V: u8 = 8;

/// An effect particle's `flags`.
const STAY_ATTACHED: i128 = 1;
const RANDOM_INITIAL_ANGLE: i128 = 2;
const TINT_FROM_OBJECT: i128 = 4;

/// `A scales values` bits a particle's ranges and tint read, by first bit.
const VELOCITY_BITS: u32 = 0;
const CONE_BIT: u32 = 2;
const ANGULAR_BITS: u32 = 3;
const COUNT_BITS: u32 = 5;
const DISTRIBUTION_BITS: u32 = 7;
const RADIUS_BITS: u32 = 9;
const TINT_BIT: u32 = 11;

/// A `point_physics`, as its postprocess leaves it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CePointPhysics {
    mass_over_radius_cubed: f32,
    air_buoyancy: f32,
    air_friction: f32,
    surface_friction: f32,
    elasticity: f32,
    collides_with_structure: bool,
}

impl CePointPhysics {
    pub(super) fn read(tag: &TagFile) -> Self {
        let root = tag.root();
        let density = root.read_real("density").unwrap_or(0.0);
        let flags = root.read_int_any("flags").unwrap_or(0);
        Self {
            mass_over_radius_cubed: density * MASS_OF_DENSITY,
            air_buoyancy: if flags & 0x20 != 0 { 0.0 } else { 1.0 - 2.0 * density / (density + 0.0011) },
            air_friction: root.read_real("air friction").unwrap_or(0.0) * 10_000.0,
            surface_friction: root.read_real("surface friction").unwrap_or(0.0),
            elasticity: root.read_real("elasticity").unwrap_or(0.0),
            collides_with_structure: flags & 0x2 != 0,
        }
    }

    fn mass(&self, radius: f32) -> f32 {
        self.mass_over_radius_cubed * radius * radius * radius
    }

    /// `point_physics_definition_interpolate`: the runtime values from one to
    /// the other by `t`; the flags stay the first's.
    pub(super) fn lerp(&self, other: &Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a * (1.0 - t) + b * t;
        Self {
            mass_over_radius_cubed: mix(self.mass_over_radius_cubed, other.mass_over_radius_cubed),
            air_buoyancy: mix(self.air_buoyancy, other.air_buoyancy),
            air_friction: mix(self.air_friction, other.air_friction),
            surface_friction: mix(self.surface_friction, other.surface_friction),
            elasticity: mix(self.elasticity, other.elasticity),
            collides_with_structure: self.collides_with_structure,
        }
    }

    /// The air friction tool leaves, scaled by 10000.
    pub(super) fn air_friction(&self) -> f32 {
        self.air_friction
    }

    /// `point_physics_update` with a force on the point, which first moves its
    /// velocity by the force over the mass of it and the air it displaces.
    pub(super) fn step_with_force(
        &self,
        place: &mut [f32; 3],
        velocity: &mut [f32; 3],
        force: [f32; 3],
        radius: f32,
        ground: f32,
        dt: f32,
    ) -> bool {
        if dt == 0.0 {
            return false;
        }
        let bulk = (AIR_MASS_OVER_RADIUS_CUBED + self.mass_over_radius_cubed) * radius * radius * radius;
        if bulk != 0.0 {
            for (velocity, force) in velocity.iter_mut().zip(force) {
                *velocity += force * (dt / bulk);
            }
        }
        self.step(place, velocity, radius, ground, dt)
    }

    /// `point_physics_update` in still air over a flat floor at `ground`: true
    /// where it touched it, the only structure a preview has.
    pub(super) fn step(&self, place: &mut [f32; 3], velocity: &mut [f32; 3], radius: f32, ground: f32, dt: f32) -> bool {
        let bulk = (AIR_MASS_OVER_RADIUS_CUBED + self.mass_over_radius_cubed) * radius * radius * radius;
        let drag = self.air_friction * radius * radius;
        velocity[2] += GRAVITY * self.air_buoyancy * dt;
        let slowing = if bulk != 0.0 {
            (dt / bulk * drag).clamp(0.0, 1.0)
        } else if drag != 0.0 {
            1.0
        } else {
            0.0
        };
        for axis in 0..3 {
            velocity[axis] -= velocity[axis] * slowing;
        }
        let mut left = dt;
        let mut touched = false;
        for _ in 0..3 {
            let next: [f32; 3] = std::array::from_fn(|axis| place[axis] + velocity[axis] * left);
            if !self.collides_with_structure || next[2] >= ground || velocity[2] >= 0.0 {
                *place = next;
                return touched;
            }
            touched = true;
            let t = ((place[2] - ground) / (place[2] - next[2])).clamp(0.0, 1.0);
            let hit: [f32; 3] = std::array::from_fn(|axis| place[axis] + (next[axis] - place[axis]) * t);
            // `component_vectors_from_normal3d`: along the floor, and into it.
            let into = velocity[2];
            velocity[0] *= 1.0 - self.surface_friction;
            velocity[1] *= 1.0 - self.surface_friction;
            velocity[2] = -into * self.elasticity;
            *place = [hit[0], hit[1], hit[2] + radius.min(0.005)];
            left -= t * left;
            if left == 0.0 {
                break;
            }
        }
        touched
    }
}

/// One sprite of a bitmap's sequence: its rect across the bitmap, the point
/// it hangs from, and the image it is cut from.
#[derive(Debug, Clone, Copy)]
struct CeSpriteFrame {
    rect: [f32; 4],
    registration: [f32; 2],
    image: usize,
}

/// An effect shader's second map: drawn over the sprite where anchored to
/// it or to the screen, or read as depth for a `zsprite`.
#[derive(Debug, Clone)]
pub(crate) struct CeSecondaryMap {
    /// `anchor`: with primary, with screen space, zsprite.
    anchor: i128,
    pub(crate) image: Arc<H2Image>,
    animation: CeMapAnimation,
    /// `zsprite radius scale`, where nothing means one.
    zsprite_scale: f32,
}

/// What a sprite draws its second map with.
#[derive(Debug, Clone)]
pub(crate) struct CeSecondaryDraw {
    pub(crate) map: Arc<CeSecondaryMap>,
    /// 1 over the sprite, 2 over the screen, 3 as depth.
    pub(crate) mode: i32,
    /// `C_TEXTURE1_XFORM`'s two rows.
    pub(crate) rows: [[f32; 4]; 2],
    /// How far toward the camera the map's green moves a zsprite's pixels.
    pub(crate) radius: f32,
}

/// An effect shader's `bitmap` and how it is laid, by the field's name.
pub(super) fn read_secondary_map(source: &TagSource, holder: TagStruct<'_>, bitmap_field: &str) -> Option<Arc<CeSecondaryMap>> {
    let path = holder.read_tag_ref_path(bitmap_field).filter(|path| !path.is_empty())?;
    let image = decode_h2_image(source, &path, SamplerKind::Texture2D, 0)?;
    Some(Arc::new(CeSecondaryMap {
        anchor: holder.read_int_any("anchor").unwrap_or(0),
        image: Arc::new(image),
        animation: read_effect_map_animation(holder),
        zsprite_scale: holder.read_real("zsprite radius scale").unwrap_or(0.0),
    }))
}

impl CeSecondaryMap {
    /// What `rasterizer_transparent_geometry_group_draw` sets for it, with
    /// the `secondary map radius` the draw gives the shader.
    /// Sprites draw with no object's animation, which reads every source as one.
    pub(crate) fn draw(self: &Arc<Self>, time: f32, radius: f32) -> CeSecondaryDraw {
        let scale = if self.zsprite_scale == 0.0 { 1.0 } else { self.zsprite_scale };
        CeSecondaryDraw {
            map: self.clone(),
            mode: match self.anchor {
                1 => 2,
                2 => 3,
                _ => 1,
            },
            rows: self.animation.rows(time, &[1.0; 4]),
            radius: radius * scale,
        }
    }
}

/// A bitmap's sprites by sequence, each image's size, and the images any
/// sprite is cut from, decoded.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeSpriteSheet {
    sizes: Vec<[f32; 2]>,
    sequences: Vec<Vec<CeSpriteFrame>>,
    pub(crate) images: Vec<Option<Arc<H2Image>>>,
}

pub(super) fn read_sprite_sheet(source: &TagSource, bitmap_path: &str) -> Option<CeSpriteSheet> {
    let bitmap = load_referenced_tag_from_source(source, bitmap_path, "bitmap", b"bitm").ok()?;
    let sizes: Vec<[f32; 2]> = elements(bitmap.root(), "bitmaps")
        .iter()
        .map(|image| {
            let size = |name: &str| image.read_int_any(name).unwrap_or(1).max(1) as f32;
            [size("width"), size("height")]
        })
        .collect();
    let sequences: Vec<Vec<CeSpriteFrame>> = elements(bitmap.root(), "sequences")
        .iter()
        .map(|sequence| {
            elements(*sequence, "sprites")
                .iter()
                .map(|sprite| {
                    let read = |name: &str| sprite.read_real(name).unwrap_or(0.0);
                    let registration = sprite.read_point2d("registration point");
                    CeSpriteFrame {
                        rect: [read("left"), read("right"), read("top"), read("bottom")],
                        registration: [registration.x, registration.y],
                        image: sprite.read_int_any("bitmap index").unwrap_or(0).max(0) as usize,
                    }
                })
                .collect()
        })
        .collect();
    let used: Vec<usize> = sequences.iter().flatten().map(|sprite| sprite.image).collect();
    let images = (0..sizes.len())
        .map(|index| {
            used.contains(&index)
                .then(|| decode_h2_image(source, bitmap_path, SamplerKind::Texture2D, index))
                .flatten()
                .map(Arc::new)
        })
        .collect();
    Some(CeSpriteSheet { sizes, sequences, images })
}

impl CeSpriteSheet {
    /// How many sprites a sequence has; none past the end.
    pub(crate) fn count(&self, sequence: i32) -> usize {
        usize::try_from(sequence).ok().and_then(|at| self.sequences.get(at)).map_or(0, Vec::len)
    }

    /// `build_sprite` for one sprite, facing the view as mode 0 does: the
    /// corners (place, then u and v) about `center`, `scale` world units to
    /// a pixel of its image, turned by `rotation` and mirrored across.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn corners(
        &self,
        sequence: i32,
        sprite: usize,
        center: [f32; 3],
        rotation: f32,
        scale: f32,
        mirror: bool,
        view: &CeFlareView,
    ) -> Option<(usize, [[f32; 5]; 4])> {
        let frame = usize::try_from(sequence).ok().and_then(|at| self.sequences.get(at)).and_then(|frames| frames.get(sprite))?;
        let size = self.sizes.get(frame.image).map_or(1.0, |size| size[0]) * scale;
        let [left, right, top, bottom] = frame.rect;
        let center_uv = [left + frame.registration[0], top + frame.registration[1]];
        let (sine, cosine) = rotation.sin_cos();
        let corner = |u: f32, v: f32| {
            let x0 = u - center_uv[0];
            let y0 = center_uv[1] - v;
            let mut x = x0 * cosine - y0 * sine;
            let y = y0 * cosine + x0 * sine;
            if mirror {
                x = -x;
            }
            let place: [f32; 3] = std::array::from_fn(|axis| center[axis] + (view.right[axis] * x + view.up[axis] * y) * size);
            [place[0], place[1], place[2], u, v]
        };
        Some((frame.image, [corner(left, bottom), corner(right, bottom), corner(left, top), corner(right, top)]))
    }
}

/// A `particle` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleDef {
    flags: i128,
    physics: Option<CePointPhysics>,
    lifespan: [f32; 2],
    fade_in: f32,
    fade_out: f32,
    minimum_size: f32,
    radius_animation: [f32; 2],
    animation_rate: [f32; 2],
    contact_deterioration: f32,
    fade_end_size: f32,
    first_sequence: i32,
    sequence_counts: [i32; 3],
    orientation: i32,
    /// `framebuffer blend function`, `framebuffer fade mode`.
    pub(crate) blend: i32,
    fade_mode: i32,
    pub(crate) nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    pub(crate) sort_bias: bool,
    /// One over the widest of its sprites in pixels, which tool bakes at +168.
    sprite_scale: f32,
    sequences: Vec<Vec<CeSpriteFrame>>,
    image_widths: Vec<f32>,
    pub(crate) images: Vec<Option<Arc<H2Image>>>,
    /// `collision effect` and `death effect`, where they are effects: a
    /// sound in their place makes nothing to see.
    collision_effect: Option<Arc<CeEffectDef>>,
    death_effect: Option<Arc<CeEffectDef>>,
    /// The shader's second `bitmap`, after the particle's own.
    secondary: Option<Arc<CeSecondaryMap>>,
}

/// The path of the `n`th field named `name` that has one.
fn nth_field_path(holder: TagStruct<'_>, name: &str, n: usize) -> Option<String> {
    let field = holder.fields().filter(|field| field.name() == name).nth(n)?;
    Some(format!("{name}#{}", field.ordinal()))
}

/// Effects read so far, by path; none while one is being read, so an effect
/// whose particles lead back to it stops there.
#[derive(Default)]
pub(super) struct CeEffectCache(std::collections::HashMap<String, Option<Arc<CeEffectDef>>>);

impl CeEffectCache {
    fn effect(&mut self, source: &TagSource, path: &str) -> Option<Arc<CeEffectDef>> {
        if let Some(known) = self.0.get(path) {
            return known.clone();
        }
        self.0.insert(path.to_owned(), None);
        let def = read_effect_def(source, path, self).map(Arc::new);
        self.0.insert(path.to_owned(), def.clone());
        def
    }
}

impl CeParticleDef {
    fn radius_scale(&self, age: f32, lifespan: f32) -> f32 {
        let [lower, upper] = self.radius_animation;
        lower + (upper - lower) * age / lifespan
    }

    fn sprites_in(&self, sequence: i32) -> usize {
        usize::try_from(sequence).ok().and_then(|at| self.sequences.get(at)).map_or(0, Vec::len)
    }
}

fn read_particle_def(source: &TagSource, path: &str, cache: &mut CeEffectCache) -> Option<CeParticleDef> {
    let tag = load_referenced_tag_from_source(source, path, "particle", b"part").ok()?;
    let root = tag.root();
    let flags = root.read_int_any("flags").unwrap_or(0);
    let bitmap_path = root.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    let CeSpriteSheet { sizes, sequences, images } = read_sprite_sheet(source, &bitmap_path)?;
    let short = |name: &str| root.read_int_any(name).unwrap_or(0) as i32;
    let first_sequence = short("first sequence index");
    let sequence_counts = [short("initial sequence count"), short("looping sequence count"), short("final sequence count")];
    let bounds = |name: &str| bounds_of(root, name);
    // Tool's postprocess (`sub_524C80`) fills these in for the game.
    let mut radius_animation = bounds("radius animation");
    radius_animation.iter_mut().filter(|scale| **scale <= 0.0).for_each(|scale| *scale = 1.0);
    let rate = bounds("animation rate");
    let rate_upper_raw = rate[1];
    let rate_lower = rate[0].clamp(0.0, 90.0);
    let animation_rate = [rate_lower, if rate_lower <= rate[1] { rate[1].min(90.0) } else { rate_lower }];
    let frame_step = if flags & ANIMATE_ONCE_PER_FRAME != 0 { 0.04 } else { rate_upper_raw };
    let [_, looping, last] = sequence_counts;
    let final_frames = (0..last)
        .filter_map(|at| sequences.get((first_sequence + sequence_counts[0] + looping + at) as usize))
        .map(Vec::len)
        .max()
        .unwrap_or(0) as f32;
    let mut lifespan = bounds("lifespan");
    lifespan.iter_mut().for_each(|end| *end -= final_frames * frame_step);
    let mut widest = 0.0f32;
    let shown = sequence_counts.iter().sum::<i32>();
    for at in 0..shown {
        for sprite in sequences.get((first_sequence + at) as usize).into_iter().flatten() {
            let [width, height] = sizes.get(sprite.image).copied().unwrap_or([1.0, 1.0]);
            let [left, right, top, bottom] = sprite.rect;
            widest = widest.max(((right - left) * width).max((bottom - top) * height)).floor();
        }
    }
    let (mut fade_start, mut fade_end) = (root.read_real("fade start size").unwrap_or(0.0), root.read_real("fade end size").unwrap_or(0.0));
    if fade_end <= 0.0 || fade_start <= 0.0 {
        (fade_start, fade_end) = (5.0, 4.0);
    }
    let _ = fade_start;
    let physics = root
        .read_tag_ref_path("physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| load_referenced_tag_from_source(source, &path, "point_physics", b"pphy").ok())
        .map(|tag| CePointPhysics::read(&tag));
    let shader_flags = root.read_int_any("shader flags").unwrap_or(0);
    Some(CeParticleDef {
        flags,
        physics,
        lifespan,
        fade_in: root.read_real("fade in time").unwrap_or(0.0),
        fade_out: root.read_real("fade out time").unwrap_or(0.0),
        minimum_size: root.read_real("minimum size").unwrap_or(0.0),
        radius_animation,
        animation_rate,
        contact_deterioration: root.read_real("contact deterioration").unwrap_or(0.0).max(0.0),
        fade_end_size: fade_end,
        first_sequence,
        sequence_counts,
        orientation: short("orientation"),
        blend: short("framebuffer blend function"),
        fade_mode: short("framebuffer fade mode"),
        nonlinear_tint: shader_flags & 2 != 0,
        sort_bias: has_flag(root, "shader flags", "sort bias"),
        sprite_scale: if widest > 0.0 { 1.0 / widest } else { 1.0 },
        secondary: nth_field_path(root, "bitmap", 1).and_then(|field| read_secondary_map(source, root, &field)),
        collision_effect: spawned_effect(source, root, "collision effect", cache),
        death_effect: spawned_effect(source, root, "death effect", cache),
        sequences,
        image_widths: sizes.iter().map(|size| size[0]).collect(),
        images,
    })
}

/// One `particles` entry of an event.
#[derive(Debug, Clone)]
struct CeEffectParticle {
    location: usize,
    /// `create`: only in first person, which a preview never is.
    first_person_only: bool,
    /// Whether the preview's object would make it; see [`created_here`].
    created: bool,
    direction: [f32; 3],
    offset: [f32; 3],
    def: usize,
    flags: i128,
    distribution: i32,
    count: [f32; 2],
    distribution_radius: [f32; 2],
    velocity: [f32; 2],
    cone: f32,
    angular_velocity: [f32; 2],
    radius: [f32; 2],
    /// Red, green, blue, alpha.
    tint: [[f32; 4]; 2],
    scales_a: u32,
    scales_b: u32,
}

/// One of an event's `parts` that makes a light.
#[derive(Debug, Clone)]
struct CeLightPart {
    location: usize,
    /// Whether the preview's object would make it; see [`created_here`].
    created: bool,
    light: usize,
    scales_a: u32,
    scales_b: u32,
}

#[derive(Debug, Clone)]
struct CeEvent {
    skip: f32,
    delay: [f32; 2],
    duration: [f32; 2],
    particles: Vec<CeEffectParticle>,
    lights: Vec<CeLightPart>,
    systems: Vec<CeSystemPart>,
}

/// One of an event's `parts` that makes a particle system.
#[derive(Debug, Clone)]
struct CeSystemPart {
    location: usize,
    /// Whether the preview's object would make it; see [`created_here`].
    created: bool,
    /// `face down regardless of location`.
    face_down: bool,
    velocity: [f32; 2],
    cone: f32,
    system: usize,
    scales_a: u32,
    scales_b: u32,
}

/// An `effect` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectDef {
    /// `deleted when attachment deactivates`: `effect_update` deletes it,
    /// for good, once its function goes off.
    deleted_when_off: bool,
    loop_start: Option<usize>,
    loop_stop: Option<usize>,
    locations: Vec<String>,
    events: Vec<CeEvent>,
    pub(crate) particles: Vec<CeParticleDef>,
    pub(crate) lights: Vec<CeEffectLightDef>,
    pub(crate) systems: Vec<Arc<CePctlDef>>,
}

fn bounds_of(holder: TagStruct<'_>, name: &str) -> [f32; 2] {
    match holder.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealBounds(bounds) | TagFieldData::AngleBounds(bounds) | TagFieldData::FractionBounds(bounds)) => {
            [bounds.lower, bounds.upper]
        }
        Some(TagFieldData::ShortIntegerBounds(bounds)) => [f32::from(bounds.lower), f32::from(bounds.upper)],
        _ => [0.0; 2],
    }
}

/// What tool bakes from `relative direction` for the effect to read.
fn direction_of(holder: TagStruct<'_>, name: &str) -> [f32; 3] {
    let (yaw, pitch) = match holder.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealEulerAngles2d(angles)) => (angles.yaw, angles.pitch),
        _ => (0.0, 0.0),
    };
    [yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()]
}

/// A particle's effect reference, where it names an effect.
fn spawned_effect(source: &TagSource, root: TagStruct<'_>, name: &str, cache: &mut CeEffectCache) -> Option<Arc<CeEffectDef>> {
    let (group, path) = root.read_tag_ref_with_group(name)?;
    (group == u32::from_be_bytes(*b"effe") && !path.is_empty()).then(|| cache.effect(source, &path)).flatten()
}

fn read_effect_def(source: &TagSource, path: &str, cache: &mut CeEffectCache) -> Option<CeEffectDef> {
    let tag = load_referenced_tag_from_source(source, path, "effect", b"effe").ok()?;
    let root = tag.root();
    let index = |name: &str| usize::try_from(root.read_int_any(name).unwrap_or(-1)).ok();
    let locations = elements(root, "locations")
        .iter()
        .map(|location| location.read_string("marker name").unwrap_or_default())
        .collect();
    let mut particles: Vec<CeParticleDef> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut events = Vec::new();
    let mut lights: Vec<CeEffectLightDef> = Vec::new();
    let mut light_paths: Vec<String> = Vec::new();
    let light_group = u32::from_be_bytes(*b"ligh");
    let system_group = u32::from_be_bytes(*b"pctl");
    let mut systems: Vec<Arc<CePctlDef>> = Vec::new();
    let mut system_paths: Vec<String> = Vec::new();
    for event in elements(root, "events") {
        let mut light_parts = Vec::new();
        let mut system_parts = Vec::new();
        for part in elements(event, "parts") {
            let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                continue;
            };
            if path.is_empty() {
                continue;
            }
            if group == system_group {
                let system = match system_paths.iter().position(|known| *known == path) {
                    Some(at) => at,
                    None => {
                        let Some(def) = read_pctl_def(source, &path) else {
                            continue;
                        };
                        systems.push(Arc::new(def));
                        system_paths.push(path);
                        systems.len() - 1
                    }
                };
                let flags = |name: &str| part.read_int_any(name).unwrap_or(0) as u32;
                system_parts.push(CeSystemPart {
                    location: part.read_int_any("location").unwrap_or(-1).max(0) as usize,
                    created: created_here(part),
                    face_down: part.read_int_any("flags").unwrap_or(0) & 1 != 0,
                    velocity: bounds_of(part, "velocity bounds"),
                    cone: part.read_real("velocity cone angle").unwrap_or(0.0),
                    system,
                    scales_a: flags("A scales values"),
                    scales_b: flags("B scales values"),
                });
                continue;
            }
            if group != light_group {
                continue;
            }
            let light = match light_paths.iter().position(|known| *known == path) {
                Some(at) => at,
                None => {
                    let Some(light) = read_effect_light(source, &path) else {
                        continue;
                    };
                    lights.push(light);
                    light_paths.push(path);
                    lights.len() - 1
                }
            };
            let flags = |name: &str| part.read_int_any(name).unwrap_or(0) as u32;
            light_parts.push(CeLightPart {
                location: part.read_int_any("location").unwrap_or(-1).max(0) as usize,
                created: created_here(part),
                light,
                scales_a: flags("A scales values"),
                scales_b: flags("B scales values"),
            });
        }
        let mut entries = Vec::new();
        for entry in elements(event, "particles") {
            let Some(path) = entry.read_tag_ref_path("particle type").filter(|path| !path.is_empty()) else {
                continue;
            };
            let def = match paths.iter().position(|known| *known == path) {
                Some(at) => at,
                None => {
                    let Some(def) = read_particle_def(source, &path, cache) else {
                        continue;
                    };
                    particles.push(def);
                    paths.push(path);
                    particles.len() - 1
                }
            };
            let tint = |name: &str| argb(entry, name);
            let flags = |name: &str| entry.read_int_any(name).unwrap_or(0) as u32;
            entries.push(CeEffectParticle {
                location: entry.read_int_any("location").unwrap_or(0).max(0) as usize,
                first_person_only: entry.read_enum_name("create").as_deref() == Some("only in first person"),
                created: created_here(entry),
                direction: direction_of(entry, "relative direction"),
                offset: match entry.field("relative offset").and_then(|field| field.value()) {
                    Some(TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
                    _ => [0.0; 3],
                },
                def,
                flags: entry.read_int_any("flags").unwrap_or(0),
                distribution: entry.read_int_any("distribution function").unwrap_or(0) as i32,
                count: bounds_of(entry, "count"),
                distribution_radius: bounds_of(entry, "distribution radius"),
                velocity: bounds_of(entry, "velocity"),
                cone: entry.read_real("velocity cone angle").unwrap_or(0.0),
                angular_velocity: bounds_of(entry, "angular velocity"),
                radius: bounds_of(entry, "radius"),
                tint: [tint("tint lower bound"), tint("tint upper bound")],
                scales_a: flags("A scales values"),
                scales_b: flags("B scales values"),
            });
        }
        events.push(CeEvent {
            skip: event.read_real("skip fraction").unwrap_or(0.0),
            delay: bounds_of(event, "delay bounds"),
            duration: bounds_of(event, "duration bounds"),
            particles: entries,
            lights: light_parts,
            systems: system_parts,
        });
    }
    if events.is_empty() || (particles.is_empty() && lights.is_empty() && systems.is_empty()) {
        return None;
    }
    let flags = root.read_flag_names("flags").unwrap_or_default();
    Some(CeEffectDef {
        deleted_when_off: flags.iter().any(|(_, name)| name == "deleted when attachment deactivates"),
        loop_start: index("loop start event"),
        loop_stop: index("loop stop event"),
        locations,
        events,
        particles,
        lights,
        systems,
    })
}

/// Where a location sits on the model: one per marker of its name.
#[derive(Debug, Clone, Copy)]
struct CeLocation {
    node: usize,
    position: [f32; 3],
    /// Forward, left and up.
    axes: [[f32; 3]; 3],
}

/// What `effect_new_unattached_from_markers` gives an effect a particle
/// makes: its scales, its color, and its velocity per tick.
#[derive(Debug, Clone, Copy)]
struct CeSpawn {
    scales: [f32; 2],
    color: [f32; 3],
    velocity: [f32; 3],
}

/// An effect hung on the object by one of its `attachments`, or made by a
/// particle on its own.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectAt {
    pub(crate) object: u8,
    pub(crate) def: Arc<CeEffectDef>,
    /// What made it, if a particle did: it then runs once, on no object.
    spawn: Option<CeSpawn>,
    /// `A out`.. of `primary scale`, `secondary scale` and `change color`.
    primary: Option<usize>,
    secondary: Option<usize>,
    change_color: Option<usize>,
    locations: Vec<Vec<CeLocation>>,
}

impl super::ce_functions::ReadsOutputs for CeEffectAt {
    fn object(&self) -> u8 {
        self.object
    }
    /// One a particle made runs on no object and reads none.
    fn outputs_read(&self) -> Vec<usize> {
        if self.spawn.is_some() {
            return Vec::new();
        }
        [self.primary, self.secondary].into_iter().flatten().collect()
    }
}

fn function_slot(value: Option<i128>) -> Option<usize> {
    usize::try_from(value? - 1).ok().filter(|slot| *slot < 4)
}

/// The effects among `object`'s attachments.
pub(super) fn read_ce_effects(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeEffectAt> {
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    let effect_group = u32::from_be_bytes(*b"effe");
    let mut effects = Vec::new();
    let mut cache = CeEffectCache::default();
    let cache = &mut cache;
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != effect_group || path.is_empty() {
            continue;
        }
        let Some(def) = cache.effect(source, &path) else {
            continue;
        };
        // `object_get_marker_by_name`: no name is the origin; a missing one,
        // nothing at all.
        let locations = def
            .locations
            .iter()
            .map(|name| {
                if name.is_empty() {
                    return vec![CeLocation { node: 0, position: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }];
                }
                markers
                    .iter()
                    .filter(|marker| marker.name.eq_ignore_ascii_case(name))
                    .map(|marker| CeLocation {
                        node: marker.node_index.max(0) as usize,
                        position: marker.position,
                        axes: marker.axes,
                    })
                    .collect()
            })
            .collect();
        effects.push(CeEffectAt {
            object: owner,
            def,
            spawn: None,
            primary: function_slot(attachment.read_int_any("primary scale")),
            secondary: function_slot(attachment.read_int_any("secondary scale")),
            change_color: function_slot(attachment.read_int_any("change color")),
            locations,
        });
    }
    effects
}

/// `effect_evaluate_function_integral`: the share of an event's particles
/// let go by `fraction` of it.
fn integral(function: i32, fraction: f32) -> f32 {
    if fraction == -1.0 {
        return 0.0;
    }
    match function {
        0 => 1.0,
        1 => {
            if fraction >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        2 => fraction,
        3 => fraction * fraction,
        4 => (2.0 - fraction) * fraction,
        _ => (3.0 - 2.0 * fraction) * fraction * fraction,
    }
}

struct Random(u32);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    fn bits(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 16
    }

    fn direction(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let (sin, cos) = self.range(0.0, std::f32::consts::TAU).sin_cos();
        let flat = (1.0 - z * z).max(0.0).sqrt();
        [flat * cos, flat * sin, z]
    }
}

/// `effect_datum`, the parts the preview keeps.
#[derive(Debug, Clone)]
struct RunningEffect {
    event: usize,
    in_event: bool,
    time: f32,
    duration: f32,
    last_fraction: f32,
    counts: Vec<f32>,
    finished: bool,
    stopping: bool,
    /// Deleted by `effect_update`: finished and never set going again.
    deleted: bool,
    scale_a: f32,
    scale_b: f32,
    color: [f32; 3],
}

/// `particle_datum`, the parts the preview keeps.
#[derive(Debug, Clone)]
struct CeParticle {
    effect: usize,
    def: usize,
    /// The node it rides, when `stay attached to marker`; place and
    /// direction are then the node's.
    node: Option<usize>,
    place: [f32; 3],
    direction: [f32; 3],
    velocity: [f32; 3],
    age: f32,
    lifespan: f32,
    frame_time: f32,
    frame_span: f32,
    sequence: i32,
    frame: i32,
    phase: u8,
    rotation: f32,
    angular_velocity: f32,
    radius: f32,
    /// Alpha, then red, green and blue.
    color: [f32; 4],
    flags: u8,
    /// Render frames since `render_particles` last drew it.
    undrawn: u32,
}

/// `particles_update` (360 and MCC) deletes a particle not drawn for more
/// than this many render frames; `render_particles` leaves one at or below
/// its `fade end size` on screen undrawn, so small and distant ones die.
const MOST_UNDRAWN_FRAMES: u32 = 15;

/// What a sprite draws: the effect and particle it came from, its corners
/// with texture coordinates, and the vertex color the `effect` shader reads.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleQuad {
    /// [`def_key`] of the effect it came from, and which of its particles.
    pub(crate) effect: usize,
    pub(crate) def: usize,
    pub(crate) blend: i32,
    pub(crate) nonlinear_tint: bool,
    pub(crate) sort_bias: bool,
    pub(crate) secondary: Option<CeSecondaryDraw>,
    pub(crate) image: usize,
    pub(crate) corners: [[f32; 5]; 4],
    pub(crate) color: [f32; 4],
}

/// A light an effect's part made: `light_new_unattached` on its location.
#[derive(Debug, Clone)]
struct CeRunningLight {
    effect: usize,
    def: usize,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    /// The tick it was made.
    born: u32,
    scale: f32,
}

/// Every effect's particles, carried from frame to frame.
#[derive(Default)]
pub(crate) struct CeParticleField {
    time: Option<f32>,
    effects: Vec<RunningEffect>,
    particles: Vec<CeParticle>,
    lights: Vec<CeRunningLight>,
    /// Particle systems parts have made: the effect, which of its systems,
    /// and the system as it runs.
    systems: Vec<(usize, usize, CeRunningSystem)>,
    /// Effects particles have made, after the object's in every index.
    spawned: Vec<CeEffectAt>,
    /// `game_time_get`.
    tick: u32,
    random: Option<Random>,
    system_random: Option<SystemRandom>,
}

/// What an attached effect reads of its object.
pub(crate) struct CeObjectInputs {
    pub(crate) outputs: [f32; 4],
    /// Which outputs are on.
    pub(crate) active: [bool; 4],
    pub(crate) change_colors: [[f32; 3]; 4],
}

fn scaled(value: [f32; 2], effect: &RunningEffect, first_bit: u32, a: u32, b: u32) -> [f32; 2] {
    let mut lower = value[0];
    let mut delta = value[1] - value[0];
    if a & (1 << first_bit) != 0 {
        lower *= effect.scale_a;
    }
    if b & (1 << first_bit) != 0 {
        lower *= effect.scale_b;
    }
    if a & (1 << (first_bit + 1)) != 0 {
        delta *= effect.scale_a;
    }
    if b & (1 << (first_bit + 1)) != 0 {
        delta *= effect.scale_b;
    }
    [lower, delta]
}

fn rotate_about(vector: [f32; 3], axis: [f32; 3], sine: f32, cosine: f32) -> [f32; 3] {
    let along = dot(axis, vector);
    let across = cross(axis, vector);
    std::array::from_fn(|at| vector[at] * cosine + across[at] * sine + axis[at] * along * (1.0 - cosine))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    if length > 0.0 { a.map(|axis| axis / length) } else { a }
}

/// A location's point and axes, in its node's space, turned onto `local`.
fn from_location(location: &CeLocation, local: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|at| {
        location.position[at] + (0..3).map(|row| location.axes[row][at] * local[row]).sum::<f32>()
    })
}

fn turn_by_location(location: &CeLocation, local: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|at| (0..3).map(|row| location.axes[row][at] * local[row]).sum::<f32>())
}

impl CeParticleField {
    /// Step every effect and particle to `time`, by ticks.
    /// How many particles the systems its effects made hold.
    pub(crate) fn system_particle_count(&self) -> usize {
        self.systems.iter().map(|(_, _, running)| running.particle_count()).sum()
    }

    /// Step every effect to `time`, by ticks; `others` particles of the
    /// systems' shared array are in the object's attached systems.
    pub(crate) fn advance(
        &mut self,
        effects: &[CeEffectAt],
        inputs: impl Fn(u8) -> CeObjectInputs,
        bones: Option<&[[f32; 4]]>,
        ground: f32,
        time: f32,
        others: usize,
    ) {
        let own = effects.len();
        if self.effects.len() != own + self.spawned.len() || self.time.is_none_or(|last| time < last) {
            self.effects = effects.iter().map(|_| new_running()).collect();
            self.particles.clear();
            self.lights.clear();
            self.systems.clear();
            self.spawned.clear();
            self.system_random = Some(SystemRandom::seeded(0x5eed_4321));
            self.tick = 0;
            self.random = Some(Random(0x1234_5678));
            let mut random = self.random.take().unwrap();
            for (running, effect) in self.effects.iter_mut().zip(effects) {
                set_event(running, &effect.def, 0, &mut random);
            }
            self.random = Some(random);
            self.time = Some(time - PRE_ROLL);
        }
        let Some(now) = self.time else {
            return;
        };
        let mut now = resume_point(now, time, PRE_ROLL);
        let mut random = self.random.take().unwrap_or(Random(1));
        while now + STEP <= time {
            now += STEP;
            self.tick += 1;
            let effects: Vec<CeEffectAt> = effects.iter().chain(&self.spawned).cloned().collect();
            let effects = effects.as_slice();
            let mut system_room = MOST_SYSTEM_PARTICLES.saturating_sub(self.system_particle_count() + others);
            for (index, effect) in effects.iter().enumerate() {
                let state = inputs(effect.object);
                let running = &mut self.effects[index];
                let system_random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
                let made = Made {
                    particles: &mut self.particles,
                    lights: &mut self.lights,
                    systems: &mut self.systems,
                    system_random,
                    system_room: &mut system_room,
                    ground,
                    tick: self.tick,
                };
                update_effect(index, running, effect, &state, made, &mut random, bones, STEP);
            }
            // `particle_systems_update`; those with no type in a state go.
            let system_random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
            self.systems.retain_mut(|(effect, def, running)| {
                effects
                    .get(*effect)
                    .and_then(|effect| effect.def.systems.get(*def))
                    .is_some_and(|def| update_unattached(def, running, ground, STEP, system_random, &mut system_room))
            });
            // `lights_preprocess_scene` lets go of those past their duration.
            let tick = self.tick;
            self.lights.retain(|light| {
                effects.get(light.effect).and_then(|effect| effect.def.lights.get(light.def)).is_some_and(|def| def.alive(tick - light.born))
            });
            let mut made: Vec<CeEffectAt> = Vec::new();
            self.particles.retain_mut(|particle| {
                if particle.undrawn > MOST_UNDRAWN_FRAMES {
                    return false;
                }
                let def = &effects[particle.effect].def.particles[particle.def];
                let (fate, collided) = update_particle(particle, def, &mut random, ground, STEP);
                // `particle_effect_new` for what it hit, then what it died of.
                if let (Some(scale), Some(effect)) = (collided, &def.collision_effect) {
                    made.extend(spawn_effect(effect, particle, scale));
                }
                if let (Fate::Died, Some(effect)) = (fate, &def.death_effect) {
                    made.extend(spawn_effect(effect, particle, 0.0));
                }
                fate == Fate::Alive
            });
            for effect in made {
                if self.spawned.len() >= MOST_SPAWNED {
                    break;
                }
                let mut running = new_running();
                set_event(&mut running, &effect.def, 0, &mut random);
                self.effects.push(running);
                self.spawned.push(effect);
            }
            self.forget_spawned(own);
        }
        self.time = Some(now);
        self.random = Some(random);
    }

    /// Lets go of the effects particles made that have run their events and
    /// left nothing behind, renumbering what came after them. `first` is
    /// how many are the object's own.
    fn forget_spawned(&mut self, first: usize) {
        let total = self.effects.len();
        let mut used = vec![false; total];
        for particle in &self.particles {
            used[particle.effect] = true;
        }
        for light in &self.lights {
            used[light.effect] = true;
        }
        for (effect, _, _) in &self.systems {
            used[*effect] = true;
        }
        let gone: Vec<bool> = (0..total).map(|at| at >= first && self.effects[at].finished && !used[at]).collect();
        if !gone.contains(&true) {
            return;
        }
        let mut renumbered = Vec::with_capacity(total);
        let mut next = 0;
        for &dropped in &gone {
            renumbered.push(next);
            if !dropped {
                next += 1;
            }
        }
        let effects = std::mem::take(&mut self.effects);
        self.effects = effects.into_iter().zip(&gone).filter(|(_, dropped)| !**dropped).map(|(effect, _)| effect).collect();
        let spawned = std::mem::take(&mut self.spawned);
        self.spawned =
            spawned.into_iter().zip(&gone[first..]).filter(|(_, dropped)| !**dropped).map(|(effect, _)| effect).collect();
        for particle in &mut self.particles {
            particle.effect = renumbered[particle.effect];
        }
        for light in &mut self.lights {
            light.effect = renumbered[light.effect];
        }
        for (effect, _, _) in &mut self.systems {
            *effect = renumbered[*effect];
        }
    }

    /// The sprites of the particle systems effects' parts have made, with
    /// the effect and system each came from.
    pub(crate) fn system_quads(
        &mut self,
        effects: &[CeEffectAt],
        view: &CeFlareView,
        pixel: impl Fn([f32; 3]) -> f32,
    ) -> Vec<(usize, usize, CePctlQuad)> {
        let effects = self.all(effects);
        let mut quads = Vec::new();
        let time = self.tick as f32 * STEP;
        let random = self.system_random.get_or_insert_with(|| SystemRandom::seeded(1));
        for (effect, def, running) in &mut self.systems {
            let Some(at) = effects.get(*effect) else {
                continue;
            };
            let Some(system) = at.def.systems.get(*def) else {
                continue;
            };
            // Each system numbers its groups from nought.
            let base = quads.len();
            for mut quad in unattached_quads(system, running, 0, view, &pixel, random, time) {
                quad.group += base;
                quads.push((def_key(&at.def), *def, quad));
            }
        }
        quads
    }

    /// The object's effects, then those particles have made.
    fn all(&self, effects: &[CeEffectAt]) -> Vec<CeEffectAt> {
        effects.iter().chain(&self.spawned).cloned().collect()
    }

    /// The lights effects' parts have made, as `lights_preprocess_scene`
    /// has them now, with the effect and light each came from.
    pub(crate) fn lights(&self, effects: &[CeEffectAt]) -> Vec<(usize, usize, CeLight)> {
        let effects = self.all(effects);
        self.lights
            .iter()
            .filter_map(|light| {
                let effect = effects.get(light.effect)?;
                let def = effect.def.lights.get(light.def)?;
                let made = def.at(effect.object, light.node, light.position, light.forward, self.tick - light.born, light.scale)?;
                Some((def_key(&effect.def), light.def, made))
            })
            .collect()
    }

    /// `render_particles` and `build_sprite`: each particle's quad, with
    /// what `pixels` says a sphere of a radius at a point covers on screen,
    /// counting the frames each goes undrawn.
    pub(crate) fn quads(
        &mut self,
        effects: &[CeEffectAt],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        pixels: impl Fn([f32; 3], f32) -> f32,
    ) -> Vec<CeParticleQuad> {
        let effects = self.all(effects);
        let mut quads = Vec::new();
        // `render_particles` gives a particle's shader the average radius of
        // its kind drawn with it: sum and count, by effect and kind.
        let mut radii: std::collections::HashMap<(usize, usize), (f32, f32)> = std::collections::HashMap::new();
        for particle in &mut self.particles {
            let Some(effect) = effects.get(particle.effect) else {
                continue;
            };
            let def = &effect.def.particles[particle.def];
            let (center, direction) = match particle.node {
                Some(node) => posed_on(node, particle.place, particle.direction, bones),
                None => (particle.place, particle.direction),
            };
            let radius = def.radius_scale(particle.age, particle.lifespan) * particle.radius;
            let diameter = pixels(center, radius);
            if diameter <= def.fade_end_size {
                particle.undrawn = particle.undrawn.saturating_add(1);
                continue;
            }
            particle.undrawn = 0;
            let entry = radii.entry((def_key(&effect.def), particle.def)).or_insert((0.0, 0.0));
            entry.0 += radius;
            entry.1 += 1.0;
            let mut scale = radius * 2.0 * def.sprite_scale;
            if diameter < def.minimum_size {
                scale *= def.minimum_size / diameter;
            }
            let mut fade = 1.0;
            if def.fade_in > 0.0 && particle.age < def.fade_in {
                fade = particle.age / def.fade_in;
            }
            if def.fade_out > 0.0 && particle.lifespan - particle.age < def.fade_out {
                fade *= (particle.lifespan - particle.age) / def.fade_out;
            }
            let Some(sprite) = usize::try_from(particle.sequence)
                .ok()
                .and_then(|at| def.sequences.get(at))
                .and_then(|frames| usize::try_from(particle.frame).ok().and_then(|at| frames.get(at)))
            else {
                continue;
            };
            let to_particle = unit(std::array::from_fn(|axis| center[axis] - view.eye[axis]));
            let (across, up) = sprite_basis(def.orientation, direction, to_particle, view);
            // `build_sprite_compute_vertex_fade`, which only a turned sprite takes.
            if def.fade_mode != 0 && def.orientation != 0 {
                let normal = unit(cross(across, up));
                let facing = dot(to_particle, normal).abs();
                fade *= if def.fade_mode == 2 { 1.0 - facing } else { facing };
            }
            let [alpha, red, green, blue] = particle.color;
            let shown_alpha =
                if def.blend != 0 && !def.nonlinear_tint { fade } else { alpha * fade };
            let size = def.image_widths.get(sprite.image).copied().unwrap_or(1.0) * scale;
            let [left, right, top, bottom] = sprite.rect;
            let center_uv = [left + sprite.registration[0], top + sprite.registration[1]];
            let (sine, cosine) = particle.rotation.sin_cos();
            let corner = |u: f32, v: f32| {
                let x0 = u - center_uv[0];
                let y0 = center_uv[1] - v;
                let mut x = x0 * cosine - y0 * sine;
                let mut y = y0 * cosine + x0 * sine;
                if particle.flags & MIRROR_U != 0 {
                    x = -x;
                }
                if particle.flags & MIRROR_V != 0 {
                    y = -y;
                }
                let place: [f32; 3] = std::array::from_fn(|axis| center[axis] + (across[axis] * x + up[axis] * y) * size);
                [place[0], place[1], place[2], u, v]
            };
            quads.push(CeParticleQuad {
                effect: def_key(&effect.def),
                def: particle.def,
                blend: def.blend,
                nonlinear_tint: def.nonlinear_tint,
                sort_bias: def.sort_bias,
                secondary: None,
                image: sprite.image,
                corners: [corner(left, bottom), corner(right, bottom), corner(left, top), corner(right, top)],
                color: [red, green, blue, shown_alpha],
            });
        }
        // Then each kind's second map, with what its kind's sprites average.
        let time = self.tick as f32 * STEP;
        for quad in &mut quads {
            let Some(effect) = effects.iter().find(|effect| def_key(&effect.def) == quad.effect) else {
                continue;
            };
            let Some(map) = effect.def.particles.get(quad.def).and_then(|def| def.secondary.as_ref()) else {
                continue;
            };
            let (sum, count) = radii.get(&(quad.effect, quad.def)).copied().unwrap_or((0.0, 0.0));
            let average = if count > 0.0 { sum / count } else { 0.0 };
            quad.secondary = Some(map.draw(time, average));
        }
        quads
    }
}

/// `effect_new` leaves an effect before its first event.
fn new_running() -> RunningEffect {
    RunningEffect {
        event: 0,
        in_event: false,
        time: 0.0,
        duration: 0.0,
        last_fraction: -1.0,
        counts: Vec::new(),
        finished: false,
        stopping: false,
        deleted: false,
        scale_a: 1.0,
        scale_b: 1.0,
        color: [1.0; 3],
    }
}

/// `global_real_rgb_grey`, which `light_particle` gives where no structure is.
const PARTICLE_GREY: f32 = 0.5;

/// The most effects particles may have made at once. The game's effect
/// array is shared with every other effect; the preview keeps its own.
const MOST_SPAWNED: usize = 128;

/// Where none of an effect's own markers stand: [`posed_on`] leaves a place
/// on no node of the skeleton where it is.
const WORLD_NODE: usize = usize::MAX / 8;

/// How a particle's update ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    Alive,
    /// `datum_delete`: gone, and nothing more.
    Deleted,
    /// `particle_die`: gone, making its death effect.
    Died,
}

/// `perpendicular3d`: a vector square to `a`, from its two largest parts.
fn perpendicular(a: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = a.map(f32::abs);
    if x > y || x > z {
        if y > z { [a[1], -a[0], 0.0] } else { [-a[2], 0.0, a[0]] }
    } else {
        [0.0, a[2], -a[1]]
    }
}

/// `particle_effect_new` for an effect: `effect_new_unattached_from_markers`
/// with two markers where the particle is, `velocity` along its way and
/// `gravity` straight down; a location naming neither takes the first.
fn spawn_effect(def: &Arc<CeEffectDef>, particle: &CeParticle, scale: f32) -> Option<CeEffectAt> {
    let along = unit(particle.direction);
    let frame = |forward: [f32; 3]| {
        let up = unit(perpendicular(forward));
        CeLocation { node: WORLD_NODE, position: particle.place, axes: [forward, cross(up, forward), up] }
    };
    let locations = def
        .locations
        .iter()
        .map(|name| vec![if name == "gravity" { frame([0.0, 0.0, -1.0]) } else { frame(along) }])
        .collect();
    Some(CeEffectAt {
        object: u8::MAX,
        def: def.clone(),
        spawn: Some(CeSpawn {
            scales: [scale, 0.0],
            color: [1.0; 3],
            velocity: particle.velocity.map(|axis| axis * 0.033_333_335),
        }),
        primary: None,
        secondary: None,
        change_color: None,
        locations,
    })
}

/// What identifies an effect's tag among those the preview has read.
pub(crate) fn def_key(def: &Arc<CeEffectDef>) -> usize {
    Arc::as_ptr(def) as usize
}

/// Every effect tag the object's effects can come to run: their own, and
/// those their particles make when they hit something or die.
pub(crate) fn reachable_defs(effects: &[CeEffectAt]) -> Vec<Arc<CeEffectDef>> {
    let mut found: Vec<Arc<CeEffectDef>> = Vec::new();
    let mut pending: Vec<Arc<CeEffectDef>> = effects.iter().map(|effect| effect.def.clone()).collect();
    while let Some(def) = pending.pop() {
        if found.iter().any(|known| Arc::ptr_eq(known, &def)) {
            continue;
        }
        for particle in &def.particles {
            pending.extend(particle.collision_effect.iter().chain(&particle.death_effect).cloned());
        }
        found.push(def);
    }
    found
}

/// What an effect's update makes: particles and lights, and the tick.
struct Made<'a> {
    particles: &'a mut Vec<CeParticle>,
    lights: &'a mut Vec<CeRunningLight>,
    systems: &'a mut Vec<(usize, usize, CeRunningSystem)>,
    system_random: &'a mut SystemRandom,
    /// What is left of the particle systems' shared particles.
    system_room: &'a mut usize,
    ground: f32,
    tick: u32,
}

/// Whether an event's particle or part is made for the preview's object:
/// violent (`effect_part_allowed_by_disposition` leaves out what is for
/// nonviolent mode only) and in air with no level about it
/// (`effect_allowed_by_environment`: water needs water under the point, and
/// space only is never made).
fn created_here(entry: TagStruct<'_>) -> bool {
    let environment = entry.read_enum_name("create in");
    let violence = entry.read_enum_name("violence mode");
    matches!(environment.as_deref(), None | Some("any environment" | "air only"))
        && violence.as_deref() != Some("nonviolent mode only")
}

/// `effect_generate_parts`, for the parts that make lights: one at every
/// instance of the location, of the parts [`created_here`].
fn generate_parts(index: usize, running: &RunningEffect, effect: &CeEffectAt, lights: &mut Vec<CeRunningLight>, tick: u32) {
    for part in &effect.def.events[running.event].lights {
        if !part.created {
            continue;
        }
        let Some(instances) = effect.locations.get(part.location) else {
            continue;
        };
        // The type-specific scale.
        let mut scale = 1.0;
        if part.scales_a & 0x20 != 0 {
            scale = running.scale_a;
        }
        if part.scales_b & 0x20 != 0 {
            scale *= running.scale_b;
        }
        for instance in instances {
            lights.push(CeRunningLight {
                effect: index,
                def: part.light,
                node: instance.node,
                position: instance.position,
                forward: instance.axes[0],
                born: tick,
                scale,
            });
        }
    }
}

/// `effect_real_random_range`, with a pair of bits of the scale flags.
fn effect_random_range(value: [f32; 2], running: &RunningEffect, first_bit: u32, a: u32, b: u32, random: &mut Random) -> f32 {
    let [lower, delta] = scaled(value, running, first_bit, a, b);
    random.range(0.0, delta) + lower
}

/// `effect_generate_parts` for the parts that make particle systems:
/// `particle_system_new_unattached` at every instance of the location,
/// going by `effect_random_translational_velocity` along the location's
/// forward (or straight down), tinted by the effect's color.
#[allow(clippy::too_many_arguments)]
fn generate_systems(
    index: usize,
    running: &RunningEffect,
    effect: &CeEffectAt,
    systems: &mut Vec<(usize, usize, CeRunningSystem)>,
    random: &mut Random,
    system_random: &mut SystemRandom,
    room: &mut usize,
    bones: Option<&[[f32; 4]]>,
    ground: f32,
) {
    for part in &effect.def.events[running.event].systems {
        if !part.created {
            continue;
        }
        let (Some(instances), Some(def)) = (effect.locations.get(part.location), effect.def.systems.get(part.system)) else {
            continue;
        };
        let mut scale = 1.0;
        if part.scales_a & 0x20 != 0 {
            scale = running.scale_a;
        }
        if part.scales_b & 0x20 != 0 {
            scale *= running.scale_b;
        }
        for instance in instances {
            let (point, mut forward) = posed_on(instance.node, instance.position, instance.axes[0], bones);
            if part.face_down {
                forward = [0.0, 0.0, -1.0];
            }
            let speed = effect_random_range(part.velocity, running, VELOCITY_BITS, part.scales_a, part.scales_b, random);
            let mut cone = part.cone;
            if part.scales_a & (1 << CONE_BIT) != 0 {
                cone *= running.scale_a;
            }
            if part.scales_b & (1 << CONE_BIT) != 0 {
                cone *= running.scale_b;
            }
            let angle = random.next() * cone;
            let mut direction = forward;
            if angle != 0.0 {
                let (sine, cosine) = angle.sin_cos();
                direction = rotate_about(direction, random.direction(), sine, cosine);
            }
            let inherited = effect.spawn.map_or([0.0; 3], |spawn| spawn.velocity);
            let velocity: [f32; 3] = std::array::from_fn(|axis| direction[axis] * speed + inherited[axis]);
            let [red, green, blue] = running.color;
            let made = new_unattached(def, point, velocity, [1.0, red, green, blue], scale, ground, system_random, room);
            systems.push((index, part.system, made));
        }
    }
}

/// `build_sprite_compute_basis`: the sprite's across (its u) and up (its v).
/// Screen facing takes the view's; parallel to its direction runs across
/// along it, up square to it and the way to the eye; perpendicular lies
/// across it, square to world up, or to world left (which the game calls
/// `viewer_space_world_forward`) where it is near vertical, turned a quarter
/// about it.
fn sprite_basis(orientation: i32, direction: [f32; 3], to_particle: [f32; 3], view: &CeFlareView) -> ([f32; 3], [f32; 3]) {
    match orientation {
        1 => {
            let along = unit(direction);
            (along, unit(cross(to_particle, along)))
        }
        2 => {
            let along = unit(direction);
            let world_up = [0.0, 0.0, 1.0];
            let reference = if dot(along, world_up).powi(2) > 0.99 { [0.0, 1.0, 0.0] } else { world_up };
            let side = unit(cross(along, reference));
            (side, rotate_about(side, along, -1.0, 0.0))
        }
        _ => (view.right, view.up),
    }
}

/// `effect_set_event`.
fn set_event(running: &mut RunningEffect, def: &CeEffectDef, event: usize, random: &mut Random) {
    if let Some(found) = def.events.get(event) {
        running.event = event;
        running.time = 0.0;
        running.in_event = false;
        running.duration = random.range(found.delay[0], found.delay[1]);
    }
}

/// `effect_update` for an effect `effect_new_looping` made.
#[allow(clippy::too_many_arguments)]
fn update_effect(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    inputs: &CeObjectInputs,
    made: Made<'_>,
    random: &mut Random,
    bones: Option<&[[f32; 4]]>,
    dt: f32,
) {
    if let Some(spawn) = effect.spawn {
        // Made once, as the particle said: it runs its events and is done.
        [running.scale_a, running.scale_b] = spawn.scales;
        running.color = spawn.color;
    } else {
        attached_inputs(running, effect, inputs, random);
    }
    run_events(index, running, effect, made, random, bones, dt);
}

/// What an attached effect takes of its object each tick: on while its
/// primary scale is, restarting when it has run out, stopping when it goes
/// off.
fn attached_inputs(running: &mut RunningEffect, effect: &CeEffectAt, inputs: &CeObjectInputs, random: &mut Random) {
    let def = &effect.def;
    // `object_get_function_value`: none is on at one.
    let (scale_a, on) = match effect.primary {
        Some(output) => (inputs.outputs[output], inputs.active[output]),
        None => (1.0, true),
    };
    running.scale_a = scale_a;
    if running.deleted {
        return;
    }
    if !on && def.deleted_when_off {
        // The object's slot for it is cleared too, and `attachments_new`
        // runs only as the object is made: it never comes back. What it
        // made lives on.
        running.finished = true;
        running.deleted = true;
        return;
    }
    if on {
        if running.finished {
            // `effect_update` (360 and MCC) clears only "finished": an effect
            // that once stopped by going on past its loop stays marked as
            // stopping, and its function going off never stops it again.
            running.finished = false;
            set_event(running, def, 0, random);
        }
    } else if !running.finished && !running.stopping {
        // `effect_stop`: on to what follows the loop, or done.
        match def.loop_stop.map(|stop| stop + 1).filter(|next| *next < def.events.len()) {
            Some(next) => {
                set_event(running, def, next, random);
                running.stopping = true;
            }
            None => running.finished = true,
        }
    }
    running.scale_b = effect.secondary.map_or(1.0, |output| inputs.outputs[output]);
    running.color = effect.change_color.map_or([1.0; 3], |channel| inputs.change_colors[channel]);
}

/// The second half of [`update_effect`]: the effect's events through the tick.
#[allow(clippy::too_many_arguments)]
fn run_events(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    made: Made<'_>,
    random: &mut Random,
    bones: Option<&[[f32; 4]]>,
    dt: f32,
) {
    let Made { particles, lights, systems, system_random, system_room, ground, tick } = made;
    let def = &effect.def;
    let mut dt = dt;
    for _ in 0..MOST_EVENTS_A_TICK {
        if dt < 0.0 || running.finished {
            break;
        }
        let ended;
        if running.duration - running.time > dt {
            running.time += dt;
            ended = false;
            dt = -1.0;
        } else {
            dt -= running.duration - running.time;
            running.time = running.duration;
            ended = true;
        }
        if running.in_event {
            generate_particles(index, running, effect, particles, random, bones);
            if ended {
                let mut next = match (def.loop_start, def.loop_stop) {
                    (Some(start), Some(stop)) if running.event == stop && !running.stopping => start,
                    _ => running.event + 1,
                };
                while next < def.events.len() && random.next() < def.events[next].skip {
                    next += 1;
                }
                if next >= def.events.len() {
                    running.finished = true;
                    return;
                }
                set_event(running, def, next, random);
            }
        } else if ended {
            let event = &def.events[running.event];
            running.time = 0.0;
            running.in_event = true;
            running.last_fraction = -1.0;
            running.duration = random.range(event.duration[0], event.duration[1]);
            running.counts = event
                .particles
                .iter()
                .map(|entry| {
                    let [lower, delta] = scaled(entry.count, running, COUNT_BITS, entry.scales_a, entry.scales_b);
                    (lower + random.range(0.0, delta)).floor().clamp(0.0, 255.0)
                })
                .collect();
            generate_parts(index, running, effect, lights, tick);
            generate_systems(index, running, effect, systems, random, system_random, system_room, bones, ground);
        }
    }
}

/// `effect_generate_particles`.
fn generate_particles(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    particles: &mut Vec<CeParticle>,
    random: &mut Random,
    bones: Option<&[[f32; 4]]>,
) {
    let def = &effect.def;
    let fraction = if running.duration <= 0.0 { 1.0 } else { running.time / running.duration };
    let event = &def.events[running.event];
    for (at, entry) in event.particles.iter().enumerate() {
        let count = running.counts.get(at).copied().unwrap_or(0.0);
        let due = (integral(entry.distribution, fraction) * count) as i32
            - (integral(entry.distribution, running.last_fraction) * count) as i32;
        if due <= 0 || entry.first_person_only || !entry.created {
            continue;
        }
        let Some(instances) = effect.locations.get(entry.location) else {
            continue;
        };
        let particle_def = &def.particles[entry.def];
        for location in instances {
            for _ in 0..due {
                if particles.len() >= MOST_PARTICLES {
                    return;
                }
                let [lower, delta] = scaled(entry.distribution_radius, running, DISTRIBUTION_BITS, entry.scales_a, entry.scales_b);
                let distance = random.range(0.0, delta) + lower;
                let spread = random.direction();
                let offset = turn_by_location(location, entry.offset);
                let local: [f32; 3] =
                    std::array::from_fn(|axis| location.position[axis] + spread[axis] * distance + offset[axis]);
                // `effect_random_translational_velocity`.
                let [lower, delta] = scaled(entry.velocity, running, VELOCITY_BITS, entry.scales_a, entry.scales_b);
                let speed = random.range(0.0, delta) + lower;
                let mut cone = entry.cone;
                if entry.scales_a & (1 << CONE_BIT) != 0 {
                    cone *= running.scale_a;
                }
                if entry.scales_b & (1 << CONE_BIT) != 0 {
                    cone *= running.scale_b;
                }
                let mut direction = entry.direction;
                let angle = random.next() * cone;
                if angle != 0.0 {
                    let axis = random.direction();
                    let (sine, cosine) = angle.sin_cos();
                    direction = rotate_about(direction, axis, sine, cosine);
                }
                let direction = turn_by_location(location, direction);
                // Each particle goes with the effect too, which moves per tick.
                let inherited = effect.spawn.map_or([0.0; 3], |spawn| spawn.velocity.map(|axis| axis * 30.0));
                let velocity: [f32; 3] = std::array::from_fn(|axis| direction[axis] * speed + inherited[axis]);
                let attached = entry.flags & STAY_ATTACHED != 0;
                let (place, direction, velocity) = if attached {
                    (local, direction, velocity)
                } else {
                    let (place, direction) = posed_on(location.node, local, direction, bones);
                    let (_, velocity) = posed_on(location.node, [0.0; 3], velocity, bones);
                    (place, direction, velocity)
                };
                let [lower, delta] = scaled(entry.radius, running, RADIUS_BITS, entry.scales_a, entry.scales_b);
                let radius = random.range(0.0, delta) + lower;
                let [lower, delta] = scaled(entry.angular_velocity, running, ANGULAR_BITS, entry.scales_a, entry.scales_b);
                let angular_velocity = random.range(0.0, delta) + lower;
                let rotation =
                    if entry.flags & RANDOM_INITIAL_ANGLE != 0 { random.range(0.0, std::f32::consts::TAU) } else { 0.0 };
                let mix = if entry.scales_a & (1 << TINT_BIT) != 0 || entry.scales_b & (1 << TINT_BIT) != 0 {
                    let mut mix = 1.0;
                    if entry.scales_a & (1 << TINT_BIT) != 0 {
                        mix = running.scale_a;
                    }
                    if entry.scales_b & (1 << TINT_BIT) != 0 {
                        mix *= running.scale_b;
                    }
                    mix
                } else {
                    random.next()
                };
                let [low, high] = entry.tint;
                let rgb = ce_color_interpolate(
                    (entry.flags >> 3) & 3,
                    [low[0], low[1], low[2]],
                    [high[0], high[1], high[2]],
                    mix,
                );
                let tinted = if entry.flags & TINT_FROM_OBJECT != 0 {
                    std::array::from_fn(|channel| rgb[channel] * running.color[channel])
                } else {
                    rgb
                };
                // `particle_new`: `light_particle` finds no structure under a
                // preview's particle, so both its light and the diffuse color
                // below it are grey.
                let mut tinted = tinted;
                if particle_def.flags & SELF_ILLUMINATED == 0 {
                    tinted = tinted.map(|channel| channel * PARTICLE_GREY);
                }
                if particle_def.flags & TINT_FROM_DIFFUSE != 0 {
                    tinted = tinted.map(|channel| channel * PARTICLE_GREY);
                }
                let alpha = (1.0 - mix) * low[3] + high[3] * mix;
                let mut lifespan = random.range(particle_def.lifespan[0], particle_def.lifespan[1]);
                if lifespan <= 0.0 {
                    lifespan = 0.0001;
                }
                let frame_span = if particle_def.animation_rate[1] == 0.0 {
                    f32::MAX
                } else {
                    1.0 / random.range(particle_def.animation_rate[0], particle_def.animation_rate[1])
                };
                let mut flags = 0;
                if particle_def.flags & CAN_ANIMATE_BACKWARDS != 0 && random.bits() & 1 != 0 {
                    flags |= BACKWARDS;
                }
                if particle_def.flags & RANDOM_HORIZONTAL_MIRROR != 0 && random.bits() & 4 != 0 {
                    flags |= MIRROR_U;
                }
                if particle_def.flags & RANDOM_VERTICAL_MIRROR != 0 && random.bits() & 8 != 0 {
                    flags |= MIRROR_V;
                }
                let mut particle = CeParticle {
                    effect: index,
                    def: entry.def,
                    node: attached.then_some(location.node),
                    place,
                    direction,
                    velocity,
                    age: 0.0,
                    lifespan,
                    frame_time: -1.0,
                    frame_span,
                    sequence: -1,
                    frame: -1,
                    phase: 0,
                    rotation,
                    angular_velocity,
                    radius,
                    color: [alpha, tinted[0], tinted[1], tinted[2]],
                    flags,
                    undrawn: 0,
                };
                if next_sequence(&mut particle, particle_def, random) {
                    let count = particle_def.sprites_in(particle.sequence) as i32;
                    particle.frame = if particle_def.flags & STARTS_ON_RANDOM_FRAME != 0 {
                        let step = if particle.flags & BACKWARDS == 0 { -1 } else { 1 };
                        step + (random.next() * count as f32) as i32
                    } else if particle.flags & BACKWARDS != 0 {
                        count
                    } else {
                        -1
                    };
                    particles.push(particle);
                }
            }
        }
    }
    let _ = from_location;
    running.last_fraction = fraction;
}

/// `particle_next_sequence`: false where the particle dies.
fn next_sequence(particle: &mut CeParticle, def: &CeParticleDef, random: &mut Random) -> bool {
    let [initial, looping, last] = def.sequence_counts;
    let mut pick = |offset: i32, count: i32| def.first_sequence + offset + (random.next() * count as f32) as i32;
    particle.sequence = -1;
    if particle.phase == 0 {
        if initial > 0 {
            particle.sequence = pick(0, initial);
        }
        particle.phase += 1;
    }
    if particle.sequence == -1 && particle.phase == 1 {
        particle.phase = 2;
    }
    if particle.phase == 2 {
        if particle.age >= particle.lifespan || looping <= 0 {
            particle.phase += 1;
        } else {
            particle.sequence = pick(initial, looping);
        }
    }
    if particle.sequence == -1 && particle.phase == 3 {
        if last > 0 {
            particle.sequence = pick(initial + looping, last);
        }
        particle.phase += 1;
    }
    if particle.sequence == -1 || def.sequences.is_empty() {
        return false;
    }
    particle.sequence = particle.sequence.clamp(0, def.sequences.len() as i32 - 1);
    true
}

/// `particle_next_frame`: false where the particle dies.
fn next_frame(particle: &mut CeParticle, def: &CeParticleDef, random: &mut Random) -> bool {
    particle.frame_time = 0.0;
    if particle.flags & BACKWARDS != 0 {
        if particle.frame <= 0 {
            if !next_sequence(particle, def, random) {
                return false;
            }
            particle.frame = def.sprites_in(particle.sequence) as i32 - 1;
        } else {
            particle.frame -= 1;
        }
        true
    } else {
        let next = particle.frame + 1;
        if next >= def.sprites_in(particle.sequence) as i32 {
            let alive = next_sequence(particle, def, random);
            particle.frame = 0;
            alive
        } else {
            particle.frame = next;
            true
        }
    }
}

/// `particles_update` for one particle: how it ends, and the scale of any
/// collision effect its hitting structure makes.
fn update_particle(particle: &mut CeParticle, def: &CeParticleDef, random: &mut Random, ground: f32, dt: f32) -> (Fate, Option<f32>) {
    let first = particle.age == 0.0;
    particle.age += dt;
    if particle.age >= particle.lifespan && !first && def.sequence_counts[2] == 0 {
        return (Fate::Died, None);
    }
    // `particle_update_frame_time`.
    if def.flags & ANIMATION_STOPS_AT_REST == 0 || particle.flags & AT_REST == 0 {
        if def.flags & ANIMATE_ONCE_PER_FRAME != 0 {
            if !next_frame(particle, def, random) {
                return (Fate::Died, None);
            }
        } else {
            let mut alive = true;
            if particle.frame_time == -1.0 {
                alive = next_frame(particle, def, random);
                particle.frame_time = 0.0;
            }
            let mut left = dt;
            while alive {
                let span = particle.frame_span - particle.frame_time;
                if span > left {
                    particle.frame_time += left;
                    break;
                }
                alive = next_frame(particle, def, random);
                left -= span;
                if left <= 0.0 {
                    break;
                }
            }
            if !alive {
                return (Fate::Died, None);
            }
        }
    }
    // `particle_update_physics`.
    if particle.flags & AT_REST != 0 {
        return (Fate::Alive, None);
    }
    let radius = def.radius_scale(particle.age, particle.lifespan) * particle.radius;
    let mut resting = false;
    let mut collided = None;
    match (particle.node, def.physics) {
        (Some(_), Some(physics)) => {
            let drag = physics.air_friction * radius * radius;
            let mass = physics.mass(radius);
            let keep = if mass == 0.0 {
                if drag == 0.0 { 1.0 } else { 0.0 }
            } else {
                (1.0 - drag / mass * dt).clamp(0.0, 1.0)
            };
            for axis in 0..3 {
                particle.velocity[axis] *= keep;
                particle.place[axis] += particle.velocity[axis] * dt;
            }
            resting = true;
        }
        (Some(_), None) => {
            for axis in 0..3 {
                particle.place[axis] += particle.velocity[axis] * dt;
            }
            resting = true;
        }
        (None, Some(physics)) => {
            if physics.step(&mut particle.place, &mut particle.velocity, radius, ground, dt) {
                // `particle_update_physics`: what it hit sets off its
                // collision effect, as hard as it is going past half a unit.
                let scale = (dot(particle.velocity, particle.velocity).sqrt() - 0.5).clamp(0.0, 1.0);
                collided = Some(scale);
                if def.flags & DIES_ON_STRUCTURE != 0 {
                    let fate = if def.collision_effect.is_none() { Fate::Died } else { Fate::Deleted };
                    return (fate, collided);
                }
                resting = true;
                particle.frame_span += def.contact_deterioration;
            }
            // Always in air: the preview has no water.
            if def.flags & DIES_IN_AIR != 0 {
                return (Fate::Died, collided);
            }
        }
        (None, None) => {
            for axis in 0..3 {
                particle.place[axis] += particle.velocity[axis] * dt;
            }
        }
    }
    let speed = dot(particle.velocity, particle.velocity);
    if speed >= 0.0625 || !resting {
        if speed >= 0.0625 {
            particle.direction = particle.velocity;
        }
        particle.rotation += particle.angular_velocity * dt;
        return (Fate::Alive, collided);
    }
    if particle.node.is_none() {
        if def.flags & DIES_AT_REST != 0 {
            return (Fate::Died, collided);
        }
        particle.flags |= AT_REST;
    }
    particle.rotation += particle.angular_velocity * dt;
    (Fate::Alive, collided)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sprite_turns_as_build_sprite_compute_basis_turns_it() {
        let view = CeFlareView { eye: [0.0; 3], forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0] };
        // Parallel: across along the way it goes, up square to it and the eye.
        let (across, up) = sprite_basis(1, [0.0, 2.0, 0.0], [1.0, 0.0, 0.0], &view);
        assert_eq!(across, [0.0, 1.0, 0.0]);
        assert_eq!(up, [0.0, 0.0, 1.0]);
        // Perpendicular, falling: square to world left, not to the eye.
        let (across, up) = sprite_basis(2, [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], &view);
        assert_eq!(across, [1.0, 0.0, 0.0]);
        assert!(dot(up, across).abs() < 1e-6 && dot(up, [0.0, 0.0, 1.0]).abs() < 1e-6);
        // Perpendicular, level: square to world up.
        let (across, _) = sprite_basis(2, [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], &view);
        assert_eq!(across, [0.0, -1.0, 0.0]);
    }

    #[test]
    fn a_square_vector_comes_from_the_two_largest_parts() {
        assert_eq!(perpendicular([0.0, 0.0, -1.0]), [0.0, -1.0, -0.0]);
        assert_eq!(perpendicular([1.0, 0.0, 0.0]), [-0.0, 0.0, 1.0]);
        assert_eq!(perpendicular([0.6, 0.8, 0.0]), [0.8, -0.6, 0.0]);
        for a in [[0.3, -0.5, 0.8], [-0.9, 0.1, 0.2], [0.0, 1.0, 0.0]] {
            let b = perpendicular(a);
            assert!(dot(a, b).abs() < 1e-6);
            assert!(dot(b, b) > 0.0);
        }
    }

    #[test]
    fn an_event_lets_go_its_particles_along_its_distribution() {
        assert_eq!(integral(0, 0.0), 1.0);
        assert_eq!(integral(1, 0.5), 0.0);
        assert_eq!(integral(1, 1.0), 1.0);
        assert_eq!(integral(2, 0.25), 0.25);
        assert_eq!(integral(3, 0.5), 0.25);
        assert_eq!(integral(4, 0.5), 0.75);
        assert_eq!(integral(5, 0.5), 0.5);
        assert_eq!(integral(2, -1.0), 0.0);
    }

    #[test]
    fn a_point_on_the_floor_bounces_by_its_elasticity() {
        let physics = CePointPhysics {
            mass_over_radius_cubed: 1.0,
            air_buoyancy: 0.0,
            air_friction: 0.0,
            surface_friction: 0.5,
            elasticity: 0.5,
            collides_with_structure: true,
        };
        let mut place = [0.0, 0.0, 0.1];
        let mut velocity = [2.0, 0.0, -6.0];
        assert!(physics.step(&mut place, &mut velocity, 0.01, 0.0, 1.0 / 30.0));
        assert!((velocity[2] - 3.0).abs() < 1e-4, "{velocity:?}");
        assert!((velocity[0] - 1.0).abs() < 1e-4, "{velocity:?}");
        assert!(place[2] >= 0.0);
    }
}
