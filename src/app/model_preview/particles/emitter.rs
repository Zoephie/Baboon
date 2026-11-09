//! Emitters: their shapes, custom points, triggers and axes, and how an emitter is read.

use super::super::TagReader;
use super::look::{ParticleLook, unallocated_properties, used_states_of};
use super::movement::{
    ParticleCollider, ParticleFlock, ParticlePhysics, ParticleTurbulence, read_collider,
    read_flock, read_turbulence,
};
use super::property::{ParticleProperty, ParticleState};
use super::rules::{
    ParticleController, ParticleRules, ParticleUpdate, flagged_controllers, movement_parameter,
    particle_update,
};
use super::seeds::{REACH_ALLOCATED_STATES, SeedAllocator};
use super::sim::{Random, reach_axis_scale};
use super::{elements, has_flag};
use blam_tags::TagStruct;

/// `c_particle_emitter_definition`'s emission shapes, by option name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum EmissionShape {
    Sprayer,
    Disc,
    Globe,
    Implode,
    Tube,
    Halo,
    Debris,
    Line,
    ImpactContour,
    ImpactArea,
    Breakable,
    /// ODST's: from the points of the emitter's `custom shape`.
    CustomPoints,
    /// Reach's: anywhere in a box `emission radius` out each way.
    Cube,
    /// Reach's: along x from -1 to 1, out to `emission radius` around it.
    Cylinder,
    /// Reach's: as `Line`, drawn from the local random seed.
    UnweightedLine,
    /// Reach's: a box of `emission radius` that `axis scale` flattens.
    Plane,
}

/// ODST's `s_particle_emitter_custom_points::s_point`, decompressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct CustomPoint {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
}

/// A `particle_emitter_custom_points`.
fn read_custom_points(reader: &TagReader, emitter: TagStruct<'_>) -> Vec<CustomPoint> {
    let path = emitter
        .read_tag_ref_path("custom shape")
        .filter(|path| !path.is_empty());
    let tag = path.and_then(|path| {
        reader
            .load(&path, "particle_emitter_custom_points", b"pecp")
            .ok()
    });
    let Some(tag) = tag else {
        return Vec::new();
    };
    let root = tag.root();
    let (scale, offset) = (
        root.read_vec3("compression scale"),
        root.read_vec3("compression offset"),
    );
    let word = |point: TagStruct<'_>, name: &str| point.read_int_any(name).unwrap_or(0) as f32;
    let points = elements(root, "points")
        .into_iter()
        .map(|point| CustomPoint {
            position: [
                word(point, "position_x") * scale.i + offset.i,
                word(point, "position_y") * scale.j + offset.j,
                word(point, "position_z") * scale.k + offset.k,
            ],
            normal: ["normal_x", "normal_y", "normal_z"].map(|name| word(point, name) / 127.0),
        });
    points.collect()
}

/// `initialize_particle`'s custom shape: `emission radius` chooses the point
/// and `emission angle` how far its normal leans to a random direction, in a
/// straight line rather than around the sphere.
pub(super) fn emit_from_points(
    points: &[CustomPoint],
    radius: f32,
    angle_degrees: f32,
    random: &mut Random,
) -> ([f32; 3], [f32; 3]) {
    if points.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let at = ((points.len() as f32 * radius) as i32).clamp(0, points.len() as i32 - 1) as usize;
    let point = points[at];
    let stray = (angle_degrees / 180.0).clamp(0.0, 1.0);
    let toward = random.direction();
    (
        point.position,
        std::array::from_fn(|axis| (1.0 - stray) * point.normal[axis] + stray * toward[axis]),
    )
}

impl EmissionShape {
    pub(in crate::app::model_preview) fn named(name: &str) -> Option<Self> {
        Some(match name {
            "sprayer" => Self::Sprayer,
            "disc" => Self::Disc,
            "globe" => Self::Globe,
            "implode" => Self::Implode,
            "tube" => Self::Tube,
            "halo" => Self::Halo,
            "debris" => Self::Debris,
            "line" => Self::Line,
            "custom points" => Self::CustomPoints,
            "impact contour" => Self::ImpactContour,
            "impact area" => Self::ImpactArea,
            "breakable surface" => Self::Breakable,
            "cube" => Self::Cube,
            "cylinder" => Self::Cylinder,
            "unweighted line" => Self::UnweightedLine,
            "plane" => Self::Plane,
            // Reach's `particle_simulation_initialize_particle`, which needs
            // a boat or a jetwash simulation the preview does not run.
            "boat hull surface" | "jetwash" => return None,
            _ => return None,
        })
    }
}

/// `e_particle_attachment_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ParticleTrigger {
    Birth,
    Death,
    Collision,
    /// Halo 2's `attached particle systems`: made with the particle, they
    /// follow it, and their age runs over its lifespan (`sub_5E0850`).
    Carried,
}

/// `emission_axis_enum`, for what turns about an axis of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ParticleAxis {
    Constant,
    Cone,
    Disc,
    Globe,
}

impl ParticleAxis {
    pub(in crate::app::model_preview) fn named(name: &str) -> Self {
        match name {
            "cone" => Self::Cone,
            "disc" => Self::Disc,
            "globe" => Self::Globe,
            _ => Self::Constant,
        }
    }

    /// `initialize_particle`'s, about `reference` of x, y and z.
    pub(super) fn pick(self, reference: usize, angle: f32, random: &mut Random) -> [f32; 3] {
        let [i, j, k] = match self {
            Self::Constant => [1.0, 0.0, 0.0],
            Self::Cone => {
                let cone = random.range(0.0, angle.to_radians());
                let turn = random.range(0.0, std::f32::consts::TAU);
                [cone.cos(), turn.sin() * cone.sin(), turn.cos() * cone.sin()]
            }
            Self::Disc => {
                let turn = random.range(0.0, std::f32::consts::TAU);
                [0.0, turn.sin(), turn.cos()]
            }
            Self::Globe => random.direction(),
        };
        match reference {
            1 => [k, i, j],
            2 => [j, k, i],
            _ => [i, j, k],
        }
    }
}

#[derive(Debug)]
pub(in crate::app::model_preview) struct Emitter {
    pub(in crate::app::model_preview) update: ParticleUpdate,
    /// On the card, Reach's and Halo 4's self-acceleration is a straight lerp
    /// between its ends (Reach's `map_to_vector3d_range` has its slerp
    /// commented out; Halo 4's `ParticleMapToVector3dLerp`); Halo 3's and
    /// ODST's turn from one to the other.
    pub(in crate::app::model_preview) card_lerps: bool,
    /// `becomes gpu when at rest`: a particle that stops moves on to the card.
    pub(in crate::app::model_preview) card_at_rest: bool,
    /// What a pulse's count may fall short of a whole particle by and still
    /// make one: Halo 2's `c_particle_emitter::pulse` (0x68d160) spawns while
    /// `count + 0.0001 >= 1`; Halo 3's floors it.
    pub(in crate::app::model_preview) spawn_slack: f32,
    pub(in crate::app::model_preview) controllers: Vec<ParticleController>,
    pub(in crate::app::model_preview) shape: EmissionShape,
    pub(in crate::app::model_preview) points: Vec<CustomPoint>,
    pub(in crate::app::model_preview) scroll: [f32; 2],
    pub(in crate::app::model_preview) axis: ParticleAxis,
    pub(in crate::app::model_preview) reference_axis: usize,
    pub(in crate::app::model_preview) axis_angle: ParticleProperty,
    pub(in crate::app::model_preview) offset: ParticleProperty,
    pub(in crate::app::model_preview) direction: ParticleProperty,
    pub(in crate::app::model_preview) radius: ParticleProperty,
    pub(in crate::app::model_preview) angle: ParticleProperty,
    pub(in crate::app::model_preview) starting: ParticleProperty,
    pub(in crate::app::model_preview) most: ParticleProperty,
    pub(in crate::app::model_preview) rate: ParticleProperty,
    pub(in crate::app::model_preview) lifespan: ParticleProperty,
    pub(in crate::app::model_preview) acceleration: ParticleProperty,
    pub(in crate::app::model_preview) speed: ParticleProperty,
    pub(in crate::app::model_preview) rotation: ParticleProperty,
    pub(in crate::app::model_preview) rotation_rate: ParticleProperty,
    pub(in crate::app::model_preview) size: ParticleProperty,
    pub(in crate::app::model_preview) scale: ParticleProperty,
    pub(in crate::app::model_preview) tint: ParticleProperty,
    pub(in crate::app::model_preview) alpha: ParticleProperty,
    pub(in crate::app::model_preview) black_point: ParticleProperty,
    pub(in crate::app::model_preview) physics: Option<ParticlePhysics>,
    pub(in crate::app::model_preview) flock: Option<ParticleFlock>,
    pub(in crate::app::model_preview) collider: Option<ParticleCollider>,
    /// Halo 2's `looping particle`: another only when none are left.
    pub(in crate::app::model_preview) one_at_a_time: bool,
    /// Halo 2's `persistent particle`: age comes round, and nothing dies of it.
    pub(in crate::app::model_preview) persistent: bool,
    /// Without Halo 2's `spread between ticks`, a tick's all start at its end.
    pub(in crate::app::model_preview) bunched: bool,
    /// Halo 2's rotation is all in `rotation`, and what spins it reads the
    /// `particle rotation` input.
    pub(in crate::app::model_preview) rotation_given: bool,
    /// Reach's `volume emitter particle velocities are random`.
    pub(in crate::app::model_preview) volume_random: bool,
    /// Reach's `axis scale`; ones elsewhere.
    pub(in crate::app::model_preview) axis_scale: [f32; 3],
    /// Reach's `turbulence` movement, which only the card runs.
    pub(in crate::app::model_preview) turbulence: Option<ParticleTurbulence>,
    /// Halo 4's own update terms.
    pub(in crate::app::model_preview) h4: Option<H4EmitterTerms>,
}

/// What Halo 4's `particle_update.fx` adds over Reach's: its system's
/// `particles live forever` and `disable velocity`, its `size scale`
/// (`m_scaleMultiplier`), and the emitter's scale x and y and alpha white
/// point.
#[derive(Debug)]
pub(in crate::app::model_preview) struct H4EmitterTerms {
    pub(super) live_forever: bool,
    pub(super) disable_velocity: bool,
    pub(super) size_scale: f32,
    pub(super) scale: Option<[ParticleProperty; 2]>,
    pub(super) white_point: Option<ParticleProperty>,
}

impl H4EmitterTerms {
    /// `liveForever`: the age comes round, and from just short of 1 back to 0.
    pub(super) fn age(&self, age: f32) -> f32 {
        if !self.live_forever {
            return age;
        }
        let age = age.fract();
        if age > 0.99 { 0.0 } else { age }
    }

    /// The step a frame moves a particle along its velocity.
    pub(super) fn travel(&self, dt: f32) -> f32 {
        if self.disable_velocity {
            0.0
        } else {
            dt * self.size_scale
        }
    }

    /// `m_size`'s and `m_aspect`'s factors: scale y and the size scale, and
    /// scale x over scale y.
    pub(super) fn size_and_aspect(&self, state: &ParticleState) -> [f32; 2] {
        let [x, y] = self
            .scale
            .as_ref()
            .map_or([1.0; 2], |[x, y]| [x.scalar(state), y.scalar(state)]);
        let aspect = if y != 0.0 { x / y } else { 1.0 };
        [y * self.size_scale, aspect]
    }
}

/// A Halo 3 or ODST emitter's `physics` movement, when its flags name one:
/// gravity is parameter 0, air friction 1 and rotational friction 2.
fn read_physics(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    flagged: impl Fn(&str) -> bool,
    seeds: &mut SeedAllocator,
) -> Option<ParticlePhysics> {
    let movement = emitter
        .descend("particle movement")
        .filter(|_| flagged("physics"))?;
    let mut parameter = |id: i128| movement_parameter(reader, movement, "physics", id, seeds);
    Some(ParticlePhysics {
        gravity: parameter(0),
        air_friction: parameter(1),
        rotational_friction: parameter(2),
    })
}

/// A Halo 2 `particle_physics`' physics movement, its own or its template's.
pub(in crate::app::model_preview) fn read_movement(
    reader: &TagReader,
    movement: TagStruct<'_>,
    seeds: &mut SeedAllocator,
) -> Option<ParticlePhysics> {
    let template = movement
        .read_tag_ref_path("template")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "particle_physics", b"pmov").ok());
    let own = elements(movement, "movements");
    let movements = if own.is_empty() {
        template
            .as_ref()
            .map_or(Vec::new(), |tag| elements(tag.root(), "movements"))
    } else {
        own
    };
    let physics = movements
        .into_iter()
        .find(|entry| entry.read_enum_option("type").as_deref() == Some("physics"))?;
    let by_id = |id: i128, seeds: &mut SeedAllocator| {
        let parameters = elements(physics, "parameters");
        let found = parameters
            .into_iter()
            .find(|parameter| parameter.read_int_any("parameter id") == Some(id))?;
        ParticleProperty::named(found, "property", seeds)
    };
    Some(ParticlePhysics {
        gravity: by_id(0, seeds),
        air_friction: by_id(1, seeds),
        rotational_friction: by_id(2, seeds),
    })
}

/// An emitter's properties, in tag order.
pub(super) const EMITTER_PROPERTIES: [&str; 18] = [
    "translational offset",
    "relative direction",
    "emission radius",
    "emission angle",
    "emission axis angle",
    "particle starting count",
    "particle max count",
    "particle emission rate",
    "particle lifespan",
    "particle self-acceleration",
    "particle initial velocity",
    "particle rotation",
    "particle initial rotation rate",
    "particle size",
    "particle scale",
    "particle tint",
    "particle alpha",
    "particle alpha black point",
];

/// The order an emitter's properties take their seeds in, by
/// [`EMITTER_PROPERTIES`]: Reach's `effect_postprocess` (tool 0x140DD5070)
/// goes in tag order, and Halo 3's and ODST's allocate `emission axis angle`
/// last.
pub(super) fn emitter_seed_order(reach_postprocess: bool) -> [usize; 18] {
    std::array::from_fn(|at| match at {
        _ if reach_postprocess => at,
        0..4 => at,
        4..17 => at + 1,
        _ => 4,
    })
}

/// A Halo 3, ODST or Reach emitter of `look`'s particles; `system_flagged`
/// reads its system's flags.
pub(super) fn read_emitter(
    reader: &TagReader,
    emitter: TagStruct<'_>,
    system_flagged: impl Fn(&str) -> bool,
    look: &ParticleLook,
    size_scale: f32,
) -> Option<Emitter> {
    // Halo 4 seeds and moves as Reach does, with terms of its own.
    let rules = ParticleRules::of(reader.source.game());
    let shape = EmissionShape::named(
        &emitter
            .read_enum_option("emission shape")
            .unwrap_or_default(),
    )?;
    let mut properties = unallocated_properties(emitter, EMITTER_PROPERTIES)?;
    let movement = emitter.descend("particle movement");
    let movement_flagged =
        |wanted: &str| movement.is_some_and(|movement| has_flag(movement, "flags", wanted));
    let (live_forever, disable_velocity) = (
        system_flagged("particles live forever"),
        system_flagged("disable velocity"),
    );
    let (update, card_at_rest) =
        particle_update(system_flagged, look.any_attachments, movement_flagged);
    let controllers = flagged_controllers(movement_flagged);
    let (physics, flock, collider, turbulence);
    if rules.reach_postprocess_seeds {
        // `c_particle_controller::fully_index_parameters_and_postprocess`
        // (tool 0x140DD92D0): each controller first, with a mask of its own.
        // The preview reads no wind, nor swarm's parameter 5, so what those
        // take is not counted.
        let mut seeds = [(); 4].map(|_| SeedAllocator::reach_controller());
        physics = read_physics(reader, emitter, movement_flagged, &mut seeds[0]);
        flock = read_flock(reader, emitter, movement_flagged, &mut seeds[1]);
        collider = read_collider(reader, emitter, movement_flagged, &mut seeds[2]);
        turbulence = read_turbulence(reader, emitter, movement_flagged, &mut seeds[3]);
        // `get_used_particle_states` (0x140DD9B40) of the movement, the
        // emitter's own postprocess (0x140AE36E0), then its particle's.
        let movement_used = seeds.iter().fold(0, |used, seeds| used | seeds.used());
        let own = (movement_used | used_states_of(&properties)) & REACH_ALLOCATED_STATES;
        let mut seeds = SeedAllocator::reach(own | look.used_states);
        for at in emitter_seed_order(true) {
            properties[at].allocate(&mut seeds);
        }
    } else {
        let mut seeds = SeedAllocator::halo3();
        for at in emitter_seed_order(false) {
            properties[at].allocate(&mut seeds);
        }
        physics = read_physics(reader, emitter, movement_flagged, &mut seeds);
        flock = read_flock(reader, emitter, movement_flagged, &mut seeds);
        collider = read_collider(reader, emitter, movement_flagged, &mut seeds);
        turbulence = None;
    }
    let [
        offset,
        direction,
        radius,
        angle,
        axis_angle,
        starting,
        most,
        rate,
        lifespan,
        acceleration,
        speed,
        rotation,
        rotation_rate,
        size,
        scale,
        tint,
        alpha,
        black_point,
    ] = properties;
    let scroll = emitter.read_vec2("uv scrolling");
    let raw_scale = emitter.read_point3d("axis scale");
    Some(Emitter {
        update,
        card_lerps: rules.card_lerps,
        card_at_rest,
        spawn_slack: 0.0,
        axis: ParticleAxis::named(
            &emitter
                .read_enum_option("particle axis (for models)")
                .unwrap_or_default(),
        ),
        reference_axis: match emitter
            .read_enum_option("particle reference axis")
            .as_deref()
        {
            Some("y") => 1,
            Some("z") => 2,
            _ => 0,
        },
        axis_angle,
        scroll: [scroll.i, scroll.j],
        controllers,
        points: read_custom_points(reader, emitter),
        shape,
        offset,
        direction,
        radius,
        angle,
        starting,
        most,
        rate,
        lifespan,
        acceleration,
        speed,
        rotation,
        rotation_rate,
        size,
        scale,
        tint,
        alpha,
        black_point,
        physics,
        flock,
        collider,
        one_at_a_time: false,
        persistent: false,
        bunched: false,
        rotation_given: false,
        volume_random: rules.random_volume_velocities
            && has_flag(
                emitter,
                "emitter flags",
                "volume emitter particle velocities are random",
            ),
        axis_scale: if rules.scales_axes {
            reach_axis_scale([raw_scale.x, raw_scale.y, raw_scale.z])
        } else {
            [1.0; 3]
        },
        turbulence,
        h4: rules.h4_emitter_terms.then(|| {
            // Read past Reach's allocation [inferred: their seeds follow].
            let property = |name: &str| {
                emitter
                    .descend(name)
                    .and_then(|holder| ParticleProperty::unallocated(holder))
            };
            let scale = property("particle scale x")
                .zip(property("particle scale y"))
                .map(|(x, y)| [x, y]);
            H4EmitterTerms {
                live_forever,
                disable_velocity,
                // `c_particle_system_definition::postprocess_block_proc`.
                size_scale: if size_scale < 1e-4 { 1.0 } else { size_scale },
                scale,
                white_point: property("particle alpha white point"),
            }
        }),
    })
}
