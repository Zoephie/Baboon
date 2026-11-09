//! Reading a Halo CE effect: its particle definitions, parts, events and their flags.

use super::*;

/// A `particle`'s `flags`, by name.
#[derive(Debug, Clone, Copy)]
pub(super) struct ParticleFlags {
    pub(super) can_animate_backwards: bool,
    pub(super) animation_stops_at_rest: bool,
    pub(super) starts_on_random_frame: bool,
    pub(super) animate_once_per_frame: bool,
    pub(super) dies_at_rest: bool,
    pub(super) dies_on_structure: bool,
    pub(super) dies_in_air: bool,
    pub(super) tint_from_diffuse: bool,
    pub(super) self_illuminated: bool,
    pub(super) random_horizontal_mirror: bool,
    pub(super) random_vertical_mirror: bool,
}

impl ParticleFlags {
    fn read(root: TagStruct<'_>) -> Self {
        let set = |option: &str| has_flag(root, "flags", option);
        Self {
            can_animate_backwards: set("can animate backwards"),
            animation_stops_at_rest: set("animation stops at rest"),
            starts_on_random_frame: set("animation starts on random frame"),
            animate_once_per_frame: set("animate once per frame"),
            dies_at_rest: set("dies at rest"),
            dies_on_structure: set("dies on contact with structure"),
            dies_in_air: set("dies on contact with air"),
            tint_from_diffuse: set("tint from diffuse texture"),
            self_illuminated: set("self-illuminated"),
            random_horizontal_mirror: set("random horizontal mirroring"),
            random_vertical_mirror: set("random vertical mirroring"),
        }
    }
}

/// An effect particle's `flags`, by name.
#[derive(Debug, Clone, Copy)]
pub(super) struct EffectParticleFlags {
    pub(super) stay_attached: bool,
    pub(super) random_initial_angle: bool,
    pub(super) tint_from_object: bool,
    /// `rgb_colors_interpolate`'s word: `interpolate tint as HSV` in bit 0,
    /// `... across the long hue path` in bit 1.
    pub(super) tint_interpolation: i128,
}

impl EffectParticleFlags {
    fn read(entry: TagStruct<'_>) -> Self {
        let set = |option: &str| has_flag(entry, "flags", option);
        Self {
            stay_attached: set("stay attached to marker"),
            random_initial_angle: set("random initial angle"),
            tint_from_object: set("tint from object color"),
            tint_interpolation: ce_color_interpolation(
                set("interpolate tint as HSV"),
                set("... across the long hue path"),
            ),
        }
    }
}

/// Which of an effect part's or particle's ranges one of its scales
/// multiplies (`A scales values` or `B scales values`), each range as its
/// lower bound and its delta. A part's list stops at `type-specific scale`;
/// a particle's has `count` there and goes on.
#[derive(Debug, Clone, Copy)]
pub(super) struct Scales {
    pub(super) velocity: [bool; 2],
    pub(super) cone: bool,
    pub(super) angular_velocity: [bool; 2],
    pub(super) count: [bool; 2],
    pub(super) distribution_radius: [bool; 2],
    pub(super) radius: [bool; 2],
    pub(super) tint: bool,
    pub(super) type_specific: bool,
}

impl Scales {
    fn read(holder: TagStruct<'_>, field: &str) -> Self {
        let set = |option: &str| has_flag(holder, field, option);
        Self {
            velocity: [set("velocity"), set("velocity delta")],
            cone: set("velocity cone angle"),
            angular_velocity: [set("angular velocity"), set("angular velocity delta")],
            count: [set("count"), set("count delta")],
            distribution_radius: [set("distribution radius"), set("distribution radius delta")],
            radius: [set("particle radius"), set("particle radius delta")],
            tint: set("tint"),
            type_specific: set("type-specific scale"),
        }
    }
}

/// A `particle` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeParticleDef {
    pub(super) flags: ParticleFlags,
    pub(super) physics: Option<CePointPhysics>,
    pub(super) lifespan: [f32; 2],
    pub(super) fade_in: f32,
    pub(super) fade_out: f32,
    pub(super) minimum_size: f32,
    radius_animation: [f32; 2],
    pub(super) animation_rate: [f32; 2],
    pub(super) contact_deterioration: f32,
    pub(super) fade_end_size: f32,
    pub(super) first_sequence: i32,
    pub(super) sequence_counts: [i32; 3],
    pub(super) orientation: i32,
    /// `framebuffer blend function`, `framebuffer fade mode`.
    pub(crate) blend: FramebufferBlend,
    pub(super) fade_mode: i32,
    pub(crate) nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    pub(crate) sort_bias: bool,
    /// One over the widest of its sprites in pixels, which tool bakes at +168.
    pub(super) sprite_scale: f32,
    pub(super) sequences: Vec<Vec<CeSpriteFrame>>,
    pub(super) image_widths: Vec<f32>,
    pub(crate) images: Vec<Option<Arc<Image>>>,
    /// `collision effect` and `death effect`, where they are effects: a
    /// sound in their place makes nothing to see.
    pub(super) collision_effect: Option<Arc<CeEffectDef>>,
    pub(super) death_effect: Option<Arc<CeEffectDef>>,
    /// The shader's second `bitmap`, after the particle's own.
    pub(super) secondary: Option<Arc<CeSecondaryMap>>,
}

/// The path of the `n`th field named `name` that has one.
fn nth_field_path(holder: TagStruct<'_>, name: &str, n: usize) -> Option<String> {
    let field = holder
        .fields()
        .filter(|field| field.name() == name)
        .nth(n)?;
    Some(format!("{name}#{}", field.ordinal()))
}

/// Effects read so far, by path; none while one is being read, so an effect
/// whose particles lead back to it stops there.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeEffectCache(
    std::collections::HashMap<String, Option<Arc<CeEffectDef>>>,
);

impl CeEffectCache {
    pub(super) fn effect(&mut self, reader: &TagReader, path: &str) -> Option<Arc<CeEffectDef>> {
        if let Some(known) = self.0.get(path) {
            return known.clone();
        }
        self.0.insert(path.to_owned(), None);
        let def = read_effect_def(reader, path, self).map(Arc::new);
        self.0.insert(path.to_owned(), def.clone());
        def
    }
}

impl CeEffectDef {
    /// Whether it names events to loop, which only an attachment does.
    #[cfg(test)]
    pub(crate) fn loops(&self) -> bool {
        self.loop_start.is_some() && self.loop_stop.is_some()
    }
}

impl CeParticleDef {
    pub(super) fn radius_scale(&self, age: f32, lifespan: f32) -> f32 {
        let [lower, upper] = self.radius_animation;
        lower + (upper - lower) * age / lifespan
    }

    pub(super) fn sprites_in(&self, sequence: i32) -> usize {
        usize::try_from(sequence)
            .ok()
            .and_then(|at| self.sequences.get(at))
            .map_or(0, Vec::len)
    }
}

fn read_particle_def(
    reader: &TagReader,
    path: &str,
    cache: &mut CeEffectCache,
) -> Option<CeParticleDef> {
    let tag = reader.load(path, "particle", b"part").ok()?;
    let root = tag.root();
    let flags = ParticleFlags::read(root);
    let bitmap_path = root
        .read_tag_ref_path("bitmap")
        .filter(|path| !path.is_empty())?;
    let CeSpriteSheet {
        sizes,
        sequences,
        images,
    } = read_sprite_sheet(reader, &bitmap_path)?;
    let short = |name: &str| root.read_int_any(name).unwrap_or(0) as i32;
    let first_sequence = short("first sequence index");
    let sequence_counts = [
        short("initial sequence count"),
        short("looping sequence count"),
        short("final sequence count"),
    ];
    let bounds = |name: &str| bounds_of(root, name);
    // Tool's postprocess (`sub_524C80`) fills these in for the game.
    let mut radius_animation = bounds("radius animation");
    radius_animation
        .iter_mut()
        .filter(|scale| **scale <= 0.0)
        .for_each(|scale| *scale = 1.0);
    let rate = bounds("animation rate");
    let rate_upper_raw = rate[1];
    let rate_lower = rate[0].clamp(0.0, 90.0);
    let animation_rate = [
        rate_lower,
        if rate_lower <= rate[1] {
            rate[1].min(90.0)
        } else {
            rate_lower
        },
    ];
    let frame_step = if flags.animate_once_per_frame {
        0.04
    } else {
        rate_upper_raw
    };
    let [_, looping, last] = sequence_counts;
    let final_frames = (0..last)
        .filter_map(|at| {
            sequences.get((first_sequence + sequence_counts[0] + looping + at) as usize)
        })
        .map(Vec::len)
        .max()
        .unwrap_or(0) as f32;
    let mut lifespan = bounds("lifespan");
    lifespan
        .iter_mut()
        .for_each(|end| *end -= final_frames * frame_step);
    let mut widest = 0.0f32;
    let shown = sequence_counts.iter().sum::<i32>();
    for at in 0..shown {
        for sprite in sequences
            .get((first_sequence + at) as usize)
            .into_iter()
            .flatten()
        {
            let [width, height] = sizes.get(sprite.image).copied().unwrap_or([1.0, 1.0]);
            let [left, right, top, bottom] = sprite.rect;
            widest = widest
                .max(((right - left) * width).max((bottom - top) * height))
                .floor();
        }
    }
    // Either size unset, and both take their defaults (start 5, end 4); only
    // the end size is drawn by.
    let (fade_start, fade_end) = (
        root.read_real("fade start size").unwrap_or(0.0),
        root.read_real("fade end size").unwrap_or(0.0),
    );
    let fade_end = if fade_end <= 0.0 || fade_start <= 0.0 {
        4.0
    } else {
        fade_end
    };
    let physics = root
        .read_tag_ref_path("physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "point_physics", b"pphy").ok())
        .map(|tag| CePointPhysics::read(&tag));
    Some(CeParticleDef {
        flags,
        physics,
        lifespan,
        fade_in: root.read_real("fade in time").unwrap_or(0.0),
        fade_out: root.read_real("fade out time").unwrap_or(0.0),
        minimum_size: root.read_real("minimum size").unwrap_or(0.0),
        radius_animation,
        animation_rate,
        contact_deterioration: root
            .read_real("contact deterioration")
            .unwrap_or(0.0)
            .max(0.0),
        fade_end_size: fade_end,
        first_sequence,
        sequence_counts,
        orientation: short("orientation"),
        blend: FramebufferBlend::read(root),
        fade_mode: short("framebuffer fade mode"),
        nonlinear_tint: has_flag(root, "shader flags", "nonlinear tint"),
        sort_bias: has_flag(root, "shader flags", "sort bias"),
        sprite_scale: if widest > 0.0 { 1.0 / widest } else { 1.0 },
        secondary: nth_field_path(root, "bitmap", 1)
            .and_then(|field| read_secondary_map(reader, root, &field)),
        collision_effect: spawned_effect(reader, root, "collision effect", cache),
        death_effect: spawned_effect(reader, root, "death effect", cache),
        sequences,
        image_widths: sizes.iter().map(|size| size[0]).collect(),
        images,
    })
}

/// One `particles` entry of an event.
#[derive(Debug, Clone)]
pub(super) struct CeEffectParticle {
    pub(super) location: usize,
    /// `create`: only in first person, which a preview never is.
    pub(super) first_person_only: bool,
    /// Whether the preview's object would make it; see [`created_here`].
    pub(super) created: bool,
    pub(super) direction: [f32; 3],
    pub(super) offset: [f32; 3],
    pub(super) def: usize,
    pub(super) flags: EffectParticleFlags,
    pub(super) distribution: Distribution,
    pub(super) count: [f32; 2],
    pub(super) distribution_radius: [f32; 2],
    pub(super) velocity: [f32; 2],
    pub(super) cone: f32,
    pub(super) angular_velocity: [f32; 2],
    pub(super) radius: [f32; 2],
    /// Red, green, blue, alpha.
    pub(super) tint: [[f32; 4]; 2],
    pub(super) scales_a: Scales,
    pub(super) scales_b: Scales,
}

/// One of an event's `parts` that makes a light.
#[derive(Debug, Clone)]
pub(super) struct CeLightPart {
    pub(super) location: usize,
    /// Whether the preview's object would make it; see [`created_here`].
    pub(super) created: bool,
    pub(super) light: usize,
    pub(super) scales_a: Scales,
    pub(super) scales_b: Scales,
}

#[derive(Debug, Clone)]
pub(super) struct CeEvent {
    pub(super) skip: f32,
    pub(super) delay: [f32; 2],
    pub(super) duration: [f32; 2],
    pub(super) particles: Vec<CeEffectParticle>,
    pub(super) lights: Vec<CeLightPart>,
    pub(super) systems: Vec<CeSystemPart>,
}

/// One of an event's `parts` that makes a particle system.
#[derive(Debug, Clone)]
pub(super) struct CeSystemPart {
    pub(super) location: usize,
    /// Whether the preview's object would make it; see [`created_here`].
    pub(super) created: bool,
    /// `face down regardless of location`.
    pub(super) face_down: bool,
    pub(super) velocity: [f32; 2],
    pub(super) cone: f32,
    pub(super) system: usize,
    pub(super) scales_a: Scales,
    pub(super) scales_b: Scales,
}

/// An `effect` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectDef {
    /// `deleted when attachment deactivates`: `effect_update` deletes it,
    /// for good, once its function goes off.
    pub(super) deleted_when_off: bool,
    pub(super) loop_start: Option<usize>,
    pub(super) loop_stop: Option<usize>,
    pub(super) locations: Vec<String>,
    pub(super) events: Vec<CeEvent>,
    pub(crate) particles: Vec<CeParticleDef>,
    pub(crate) lights: Vec<CeEffectLightDef>,
    pub(crate) systems: Vec<Arc<CePctlDef>>,
}

fn bounds_of(holder: TagStruct<'_>, name: &str) -> [f32; 2] {
    match holder.field(name).and_then(|field| field.value()) {
        Some(
            TagFieldData::RealBounds(bounds)
            | TagFieldData::AngleBounds(bounds)
            | TagFieldData::FractionBounds(bounds),
        ) => [bounds.lower, bounds.upper],
        Some(TagFieldData::ShortIntegerBounds(bounds)) => {
            [f32::from(bounds.lower), f32::from(bounds.upper)]
        }
        _ => [0.0; 2],
    }
}

/// What tool bakes from `relative direction` for the effect to read.
fn direction_of(holder: TagStruct<'_>, name: &str) -> [f32; 3] {
    let (yaw, pitch) = match holder.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealEulerAngles2d(angles)) => (angles.yaw, angles.pitch),
        _ => (0.0, 0.0),
    };
    [
        yaw.cos() * pitch.cos(),
        yaw.sin() * pitch.cos(),
        pitch.sin(),
    ]
}

/// A particle's effect reference, where it names an effect.
fn spawned_effect(
    reader: &TagReader,
    root: TagStruct<'_>,
    name: &str,
    cache: &mut CeEffectCache,
) -> Option<Arc<CeEffectDef>> {
    let (group, path) = root.read_tag_ref_with_group(name)?;
    (group == u32::from_be_bytes(*b"effe") && !path.is_empty())
        .then(|| cache.effect(reader, &path))
        .flatten()
}

fn read_effect_def(
    reader: &TagReader,
    path: &str,
    cache: &mut CeEffectCache,
) -> Option<CeEffectDef> {
    let tag = reader.load(path, "effect", b"effe").ok()?;
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
                        let Some(def) = read_pctl_def(reader, &path) else {
                            continue;
                        };
                        systems.push(Arc::new(def));
                        system_paths.push(path);
                        systems.len() - 1
                    }
                };
                system_parts.push(CeSystemPart {
                    location: part.read_int_any("location").unwrap_or(-1).max(0) as usize,
                    created: created_here(part),
                    face_down: has_flag(part, "flags", "face down regardless of location (decals)"),
                    velocity: bounds_of(part, "velocity bounds"),
                    cone: part.read_real("velocity cone angle").unwrap_or(0.0),
                    system,
                    scales_a: Scales::read(part, "A scales values"),
                    scales_b: Scales::read(part, "B scales values"),
                });
                continue;
            }
            if group != light_group {
                continue;
            }
            let light = match light_paths.iter().position(|known| *known == path) {
                Some(at) => at,
                None => {
                    let Some(light) = read_effect_light(reader, &path) else {
                        continue;
                    };
                    lights.push(light);
                    light_paths.push(path);
                    lights.len() - 1
                }
            };
            light_parts.push(CeLightPart {
                location: part.read_int_any("location").unwrap_or(-1).max(0) as usize,
                created: created_here(part),
                light,
                scales_a: Scales::read(part, "A scales values"),
                scales_b: Scales::read(part, "B scales values"),
            });
        }
        let mut entries = Vec::new();
        for entry in elements(event, "particles") {
            let Some(path) = entry
                .read_tag_ref_path("particle type")
                .filter(|path| !path.is_empty())
            else {
                continue;
            };
            let def = match paths.iter().position(|known| *known == path) {
                Some(at) => at,
                None => {
                    let Some(def) = read_particle_def(reader, &path, cache) else {
                        continue;
                    };
                    particles.push(def);
                    paths.push(path);
                    particles.len() - 1
                }
            };
            let tint = |name: &str| argb(entry, name);
            entries.push(CeEffectParticle {
                location: entry.read_int_any("location").unwrap_or(0).max(0) as usize,
                first_person_only: entry
                    .field("create")
                    .is_some_and(|create| create.enum_is("only in first person")),
                created: created_here(entry),
                direction: direction_of(entry, "relative direction"),
                offset: match entry
                    .field("relative offset")
                    .and_then(|field| field.value())
                {
                    Some(TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
                    _ => [0.0; 3],
                },
                def,
                flags: EffectParticleFlags::read(entry),
                distribution: Distribution::read(entry),
                count: bounds_of(entry, "count"),
                distribution_radius: bounds_of(entry, "distribution radius"),
                velocity: bounds_of(entry, "velocity"),
                cone: entry.read_real("velocity cone angle").unwrap_or(0.0),
                angular_velocity: bounds_of(entry, "angular velocity"),
                radius: bounds_of(entry, "radius"),
                tint: [tint("tint lower bound"), tint("tint upper bound")],
                scales_a: Scales::read(entry, "A scales values"),
                scales_b: Scales::read(entry, "B scales values"),
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
    Some(CeEffectDef {
        deleted_when_off: has_flag(root, "flags", "deleted when attachment deactivates"),
        loop_start: index("loop start event"),
        loop_stop: index("loop stop event"),
        locations,
        events,
        particles,
        lights,
        systems,
    })
}
