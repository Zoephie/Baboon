use super::*;
use crate::app::model_preview::harness::kit::kit_source;
use crate::core::game::GameId;

fn texels(width: usize, height: usize, texels: Vec<[f32; 4]>) -> CheapTexels {
    CheapTexels {
        width,
        height,
        texels,
    }
}

/// A type of drag, gravity and turbulence, white, a unit at most.
fn kind(drag: f32, gravity: f32, turbulence: f32) -> CheapType {
    CheapTypeTag {
        drag,
        gravity,
        turbulence,
        velocity_facing: false,
        color: [1.0; 4],
        intensity: 2.0,
        size: 1.0,
        y_scale: 1.0,
        texture: 3,
        change: -1,
        ..Default::default()
    }
    .compiled(0, 14, false)
}

fn library(types: Vec<CheapType>, turbulence: [f32; 4]) -> CheapLibrary {
    CheapLibrary {
        names: (0..types.len()).map(|at| format!("type{at}")).collect(),
        types,
        random: texels(1, 1, vec![[0.5, 0.5, 0.5, 0.5]]),
        turbulence: vec![CheapTurbulence {
            texels: texels(1, 1, vec![turbulence]),
            rates: [0.1, 0.05, 0.1, 0.05],
            centered: false,
        }],
        render: None,
        halo4: false,
    }
}

fn definition(library: CheapLibrary) -> CheapEmitterDefinition {
    CheapEmitterDefinition {
        correlate: true,
        position_local: false,
        velocity_local: false,
        normalize: false,
        rotate_up: false,
        spawn_rate: Arc::new(constant_function(60.0).unwrap()),
        input: String::new(),
        range: String::new(),
        lod: distance_fade([100.0, 150.0]),
        parameters: [
            [0.0, 0.0, 0.0, 254.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0; 4],
            [0.0; 4],
        ],
        lighting: [1.0, 0.0, -11.0, 11.0],
        position: Arc::new(texels(1, 1, vec![[0.0; 4]])),
        velocity: Arc::new(texels(1, 1, vec![[0.0, 0.0, 1.0, 0.0]])),
        library: Arc::new(library),
    }
}

fn times(delay: f32, duration: f32) -> Arc<EffectTimes> {
    Arc::new(EffectTimes::of_events(&[([delay; 2], [duration; 2])], true))
}

fn emitter(definition: CheapEmitterDefinition, times: Arc<EffectTimes>) -> CheapEmitterAt {
    CheapEmitterAt {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        scaled: false,
        b_scaled: false,
        times,
        event: 0,
        definition: Arc::new(definition),
    }
}

fn view() -> ViewFrame {
    ViewFrame {
        eye: [0.0, -10.0, 0.0],
        forward: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
    }
}

fn live(field: &CheapField) -> usize {
    field
        .particles
        .iter()
        .filter(|particle| particle.age.abs() < 1.0)
        .count()
}

/// `postprocess`'s upgrade runs from the version it finds: 0 fades from
/// 5 to 20 and spawns at its old constant rate, 1 takes the lighting
/// defaults too, 2 only the rotation and scale; 3 is left alone.
#[test]
fn an_old_emitter_is_upgraded_from_its_version_on() {
    let tag = |version: i64| {
        CheapEmitterTag {
            version,
            spawn_rate: 12.0,
            function: None,
            fade: [1.0, 2.0],
            types: Default::default(),
            weights: [0.0; 4],
            lifetime: [1.0; 2],
            position_scale: 1.0,
            position_flatten: 0.0,
            directionality: [0.0; 2],
            speed: [0.0; 2],
            subframe: [0.0; 2],
            rotation_randomness: 0.5,
            scale_modifier: 0.5,
            lighting: [2.0, 3.0, -1.0, 1.0],
        }
        .upgraded()
    };
    let zero = tag(0);
    assert_eq!(zero.fade, [5.0, 20.0]);
    assert_eq!(
        zero.function
            .as_ref()
            .map(|function| function.evaluate(0.3, 0.7)),
        Some(12.0)
    );
    assert_eq!(
        (zero.lighting, zero.rotation_randomness, zero.scale_modifier),
        ([1.0, 0.0, -11.0, 11.0], 0.0, 1.0)
    );
    let one = tag(1);
    assert_eq!(
        (one.fade, one.lighting),
        ([1.0, 2.0], [1.0, 0.0, -11.0, 11.0])
    );
    assert!(
        one.function.is_none(),
        "only version 0's rate becomes a function"
    );
    let two = tag(2);
    assert_eq!(
        (two.lighting, two.rotation_randomness, two.scale_modifier),
        ([2.0, 3.0, -1.0, 1.0], 0.0, 1.0)
    );
    let three = tag(3);
    assert_eq!(
        (three.rotation_randomness, three.scale_modifier),
        (0.5, 0.5)
    );
}

/// The fade is whole at the nearer bound and gone at the farther,
/// whichever field holds which, and by the Manhattan distance.
#[test]
fn the_distance_fade_runs_from_the_nearer_bound_by_manhattan_distance() {
    let fade = distance_fade([100.0, 150.0]);
    assert_eq!(lod_scale(fade, 100.0), 1.0);
    assert_eq!(lod_scale(fade, 125.0), 0.5);
    assert_eq!(lod_scale(fade, 150.0), 0.0);
    let backwards = distance_fade([30.0, 25.0]);
    assert_eq!(lod_scale(backwards, 25.0), 1.0);
    assert_eq!(lod_scale(backwards, 27.5), 0.5);
    assert_eq!(
        manhattan([3.0, -4.0, 0.0], [0.0; 3]),
        7.0,
        "not the 5 of a straight line"
    );
    let point = distance_fade([10.0, 10.0]);
    assert!(
        (point[0] + 1000.0).abs() < 1e-3,
        "a range under 0.001 is 0.001: {point:?}"
    );
}

/// `spawn_params3`/`4`: thresholds from the weight before each found
/// type, steps from the last found; a type not found holds the last
/// threshold, and no weight at all is type 0's whole.
#[test]
fn types_pick_by_cumulative_weights_and_step_between_indices() {
    let (thresholds, steps) = type_thresholds([Some(5), Some(9), None, None], [0.5, 1.0, 0.0, 0.0]);
    assert_eq!(thresholds, [0.0, 0.5 / 1.5, 0.5 / 1.5, 0.5 / 1.5]);
    assert_eq!(steps, [5.0, 4.0, 0.0, 0.0]);
    // Sic: type 2 adds type 1's weight, though type 1 was not found.
    let (thresholds, steps) = type_thresholds([Some(1), None, Some(3), None], [1.0, 3.0, 1.0, 0.0]);
    assert_eq!(thresholds, [0.0, 0.0, 1.5, 1.5]);
    assert_eq!(steps, [1.0, 0.0, 2.0, 0.0]);
    let (thresholds, steps) = type_thresholds([Some(7), None, None, None], [0.0; 4]);
    assert_eq!(
        (thresholds, steps),
        ([0.0; 4], [0.0; 4]),
        "no weight: type 0"
    );
    let (_, steps) = type_thresholds([None, Some(4), None, None], [1.0, 1.0, 0.0, 0.0]);
    assert_eq!(steps, [0.0, 4.0, 0.0, 0.0], "a first type not found is 0");
}

/// `spawn_params0`–`2`: rotation held to 0..1, the modifier to 0..2 and
/// packed by 254, lifetimes inverted over at least 1e-4.
#[test]
fn spawn_parameters_clamp_and_invert_as_postprocess_does() {
    let tag = CheapEmitterTag {
        version: 3,
        spawn_rate: 0.0,
        function: None,
        fade: [0.0; 2],
        types: ["a".to_owned(), String::new(), String::new(), String::new()],
        weights: [1.0, 0.0, 0.0, 0.0],
        lifetime: [0.0, 4.0],
        position_scale: 50.0,
        position_flatten: 0.25,
        directionality: [0.2, 0.6],
        speed: [2.0, 9.0],
        subframe: [0.0, 1.0],
        rotation_randomness: 3.0,
        scale_modifier: 7.5,
        lighting: [0.0; 4],
    };
    let [p0, p1, p2, ..] = tag.spawn_parameters(&["a".to_owned()]);
    assert_eq!(
        p0,
        [50.0, 0.25, 1.0, 508.0],
        "a modifier of 7.5 packs as 2's"
    );
    assert!(
        (p1[1] - 0.4).abs() < 1e-6 && p1[2] == 2.0 && p1[3] == 7.0,
        "{p1:?}"
    );
    assert!(
        (p2[2] - 10_000.0).abs() < 1.0,
        "a lifetime of none is 1e-4: {p2:?}"
    );
    assert!((p2[3] - (0.25 - p2[2])).abs() < 1e-3);
}

/// The compile's rows: gravity as authored, change none as its own
/// index, the colour by intensity, the fade over what is left, and the
/// layer's place, the largest size, the stretch, and a screen-facing
/// type's y scale negated.
#[test]
fn a_type_compiles_to_its_five_rows() {
    let tag = CheapTypeTag {
        drag: 1.5,
        gravity: 4.0,
        turbulence: 0.5,
        depth_range: 0.1,
        elasticity: 0.5,
        death: 0.7,
        change: -1,
        velocity_facing: false,
        color: [1.0, 0.5, 0.25, 0.0],
        intensity: 30.0,
        fade_start: 0.75,
        size: 0.2,
        stretch: 2.5,
        texture: 2,
        y_scale: 0.5,
        ..Default::default()
    };
    let rows = tag.compiled(9, 14, false).rows;
    assert_eq!(rows[PHYSICS], [1.5, 4.0, 0.5, 0.1]);
    assert_eq!(
        rows[COLLISION],
        [0.0, 0.5, 0.7, 9.0],
        "none changes to itself"
    );
    assert_eq!(rows[COLOR0], [30.0, 15.0, 7.5, 0.0]);
    assert_eq!(rows[FADE], [0.0, 0.0, 0.0, 4.0]);
    assert_eq!(rows[RENDER], [2.5 / 14.0, 0.2, 2.5, -0.5]);
    let other = CheapTypeTag {
        change: 3,
        velocity_facing: true,
        texture: 20,
        fade_start: 1.0,
        ..tag
    };
    let rows = other.compiled(9, 14, false).rows;
    assert_eq!(rows[COLLISION][3], 3.0);
    assert_eq!(
        rows[RENDER][0],
        0.5 / 14.0,
        "a texture out of range is the first"
    );
    assert_eq!(rows[RENDER][3], 0.5, "velocity facing keeps its sign");
    assert_eq!(rows[FADE][3], 10_000.0, "held to 1e-4 left");
}

/// On MCC the lighting is a signed byte: an exposure above −0.086 packs
/// to 127 and draws at 0.942, never brighter; below it, as computed.
#[test]
fn mcc_holds_the_lighting_byte_at_127() {
    let packed =
        |lighting: [f32; 4]| pack_sbyte4([0.0, packed_lighting(lighting, 1.0), 0.0, 0.0])[1];
    assert_eq!(
        packed([1.0, 0.0, 0.0, 0.0]),
        127,
        "exposure 0 is 128 before the clamp"
    );
    assert!(
        (illumination(127) - 0.942).abs() < 1e-3,
        "{}",
        illumination(127)
    );
    assert_eq!(packed([1.0, -0.25, -11.0, 11.0]), 125);
    assert_eq!(packed([1.0, 5.0, -11.0, 11.0]), 127);
    assert_eq!(packed([1.0, -20.0, -11.0, 11.0]), 0, "held at the minimum");
    assert_eq!(
        pack_sbyte4([3.9, -2.7, 200.0, -300.0]),
        [3, -2, 127, -128],
        "truncated and held"
    );
}

/// A rate by the frame and scale, rounded by the roll.
#[test]
fn the_count_rounds_stochastically() {
    assert_eq!(spawn_count(60.0, 1.0 / 60.0, 1.0, 0.0), 1);
    assert_eq!(spawn_count(30.0, 1.0 / 60.0, 1.0, 0.4), 0);
    assert_eq!(spawn_count(30.0, 1.0 / 60.0, 1.0, 0.6), 1);
    assert_eq!(
        spawn_count(30.0, 1.0 / 60.0, 0.5, 0.6),
        0,
        "the scale counts"
    );
}

/// Each scale is ±[1, 2)/64 and each offset in [0, 1).
#[test]
fn a_texture_transform_scales_by_a_64th_either_way() {
    let mut random = Random(7);
    let (mut negative, mut positive) = (false, false);
    for _ in 0..64 {
        let [u, v, z, w] = texture_transform(&mut random);
        for scale in [u, v] {
            assert!((1.0..=2.0).contains(&(scale.abs() * 64.0)), "{scale}");
            negative |= scale < 0.0;
            positive |= scale > 0.0;
        }
        assert!((0.0..=1.0).contains(&z) && (0.0..=1.0).contains(&w));
    }
    assert!(negative && positive);
}

/// Correlated, the velocity reads the position's texel; otherwise its
/// own roll.
#[test]
fn a_correlated_emitter_reads_velocity_where_it_reads_position() {
    let [_, position, velocity] = spawn_transforms(&mut Random(3), true);
    assert_eq!(position, velocity);
    let [_, position, velocity] = spawn_transforms(&mut Random(3), false);
    assert_ne!(position, velocity);
}

/// The rate reads the game time, the emitter's scale for `alpha`, and
/// another name off the object.
#[test]
fn the_spawn_rate_reads_game_time_alpha_or_an_object_function() {
    let values = FunctionValues::from([("power".to_owned(), 0.25)]);
    assert_eq!(
        spawn_input("", Some(&values), 12.5, 0.5),
        12.5,
        "Reach's game time, in seconds"
    );
    assert_eq!(
        spawn_input("", None, 7300.0, 0.5),
        -7100.0,
        "which comes round at two hours"
    );
    assert_eq!(spawn_input(ALPHA, Some(&values), 12.5, 0.5), 0.5);
    assert_eq!(spawn_input("power", Some(&values), 12.5, 0.5), 0.25);
    assert_eq!(
        spawn_input("heat", Some(&values), 12.5, 0.5),
        0.0,
        "not answered"
    );
}

/// The turned up lies across forward and is not made a unit.
#[test]
fn a_rotated_emitters_up_crosses_its_forward_unnormalized() {
    let mut random = Random(11);
    let mut lengths = Vec::new();
    for _ in 0..16 {
        let up = turned_up([1.0, 0.0, 0.0], &mut random);
        assert_eq!(up[0], 0.0, "across forward");
        lengths.push(dot(up, up).sqrt());
    }
    assert!(
        lengths.iter().any(|length| (length - 1.0).abs() > 0.05),
        "{lengths:?}"
    );
}

/// Spawned with no speed and no flatten, a particle sits at the emitter,
/// takes a tenth of the subframe as its age, and its type from where the
/// random's w falls among the thresholds.
#[test]
fn a_spawned_particle_takes_its_subframe_and_type() {
    let mut emitter = definition(library(
        vec![kind(0.0, 0.0, 0.0), kind(0.0, 40.0, 0.0)],
        [0.0; 4],
    ));
    let at = CheapSubmission {
        position: [1.0, 2.0, 3.0],
        forward: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        scale: 1.0,
    };
    let transforms = [[0.0; 4]; 3];
    let particle = emitter.spawn_particle(&at, &transforms, 128.0, 0, 0.5);
    assert_eq!(particle.position, [1.0, 2.0, 3.0]);
    assert_eq!(particle.age, 0.0001, "no subframe: the least age");
    assert_eq!(
        particle.parameters,
        [0, 127, -63, 0],
        "type 0, the lighting held, no rotation"
    );
    // A subframe of half the step: the age is a tenth of it, not by the life.
    emitter.parameters[2] = [0.5, 0.0, 1.0, 0.0];
    emitter.parameters[3] = [0.0, 0.25, 1.0, 1.0];
    emitter.parameters[4] = [0.0, 1.0, 0.0, 0.0];
    let particle = emitter.spawn_particle(&at, &transforms, 128.0, 0, 0.5);
    assert_eq!(particle.age, 0.1 * 0.25);
    assert_eq!(particle.parameters[0], 1, "w 0.5 passes 0.25, not 1");
    assert!(
        particle.position[2] < 3.0 && particle.velocity[2] < 0.0,
        "type 1 falls over its subframe"
    );
}

/// In local space a texture's x runs along up, y along left and z along
/// forward; flatten takes forward's share away.
#[test]
fn local_space_and_flatten_follow_the_emitters_axes() {
    let mut emitter = definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]));
    emitter.position = Arc::new(texels(1, 1, vec![[1.0, 0.0, 0.0, 0.0]]));
    emitter.parameters[0] = [2.0, 0.0, 0.0, 0.0];
    let at = CheapSubmission {
        position: [0.0; 3],
        forward: [0.0, 1.0, 0.0],
        up: [0.0, 0.0, 1.0],
        scale: 1.0,
    };
    let world = emitter
        .spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0)
        .position;
    assert_eq!(world, [2.0, 0.0, 0.0], "world space: x is x");
    emitter.position_local = true;
    let local = emitter
        .spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0)
        .position;
    assert_eq!(local, [0.0, 0.0, 2.0], "local space: x is up");
    emitter.position = Arc::new(texels(1, 1, vec![[0.0, 0.0, 1.0, 0.0]]));
    emitter.parameters[0] = [2.0, 1.0, 0.0, 0.0];
    let flat = emitter
        .spawn_particle(&at, &[[0.0; 4]; 3], 0.0, 0, 0.0)
        .position;
    assert_eq!(flat, [0.0, 0.0, 0.0], "forward flattened away");
}

/// Drag slows, a positive gravity pulls down, turbulence moves the place
/// and not the velocity, and at rest nothing moves but the age.
#[test]
fn the_update_drags_falls_and_turbulates_the_place_only() {
    let library = library(vec![kind(1.0, 4.0, 2.0)], [0.5, 0.0, 0.0, 0.0]);
    let mut particle = CheapParticle {
        position: [0.0; 3],
        age: 0.5,
        velocity: [1.0, 0.0, 0.0],
        delta_age: 0.5,
        parameters: [0; 4],
    };
    update_particle(&mut particle, 0, &library, &[[0.0; 4]], 0.1);
    assert!((particle.age - 0.55).abs() < 1e-6);
    assert!(
        (particle.position[0] - (1.0 + 0.5 * 2.0) * 0.1).abs() < 1e-6,
        "{:?}",
        particle.position
    );
    assert!(
        (particle.position[2] + 4.0 * 0.01).abs() < 1e-6,
        "gravity pulls down"
    );
    assert!(
        (particle.velocity[0] - 0.9).abs() < 1e-6,
        "dragged, and the turbulence is not in it"
    );
    assert!((particle.velocity[2] + 0.4).abs() < 1e-6);
    let mut resting = CheapParticle {
        age: -0.5,
        ..particle
    };
    update_particle(&mut resting, 0, &library, &[[0.0; 4]], 0.1);
    assert_eq!(resting.position, particle.position, "at rest it stays");
    assert!((resting.age + 0.55).abs() < 1e-6, "and ages away from 0");
}

/// The shaders' step is held to half a second; the turbulence's phase
/// moves by the whole frame.
#[test]
fn a_long_frame_moves_particles_half_a_second() {
    let library = library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]);
    let mut field = CheapField::fresh(1);
    field.particles[0] = CheapParticle {
        position: [0.0; 3],
        age: 0.1,
        velocity: [1.0, 0.0, 0.0],
        delta_age: 0.0,
        parameters: [0; 4],
    };
    field.update(&library, 2.0);
    assert_eq!(field.particles[0].position[0], 0.5, "not 2");
    assert!(
        (field.phases[0][0] - 0.2).abs() < 1e-6,
        "{:?}",
        field.phases
    );
    field.update(&library, 0.25);
    assert_eq!(field.particles[0].position[0], 0.75);
}

/// Spawning past the end of the ring comes round and overwrites the
/// oldest slot, alive or not.
#[test]
fn the_ring_overwrites_from_the_start() {
    let definition = definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4]));
    let at = emitter(definition, times(0.0, 100.0));
    let mut field = CheapField::fresh(1);
    field.cursor = MOST_PARTICLES - 1;
    field.particles[0].age = 0.5;
    let submission = CheapSubmission {
        position: [9.0; 3],
        forward: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        scale: 1.0,
    };
    // 60 a second over two 60ths, at most one more from the roll.
    field.spawn(&at, &submission, None, 0.0, 2.0 / 60.0, 0);
    assert!(field.cursor == 1 || field.cursor == 2, "{}", field.cursor);
    assert_eq!(field.particles[MOST_PARTICLES - 1].position, [9.0; 3]);
    assert_eq!(field.particles[0].position, [9.0; 3], "slot 0 written over");
}

/// An event of no length never emits; one with a duration emits while
/// in it, at the type-specific scales it asks for.
#[test]
fn only_an_event_with_a_duration_submits_and_scales_as_asked() {
    let make = |times| {
        emitter(
            definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4])),
            times,
        )
    };
    let eye = [0.0; 3];
    assert!(
        make(times(0.0, 0.0))
            .submission(None, None, eye, 0.0, 0.0)
            .is_none(),
        "an instant"
    );
    assert!(
        make(times(0.0, 1.0))
            .submission(None, None, eye, 0.5, 0.0)
            .is_some()
    );
    assert!(
        make(times(1.0, 1.0))
            .submission(None, None, eye, 0.5, 0.0)
            .is_none(),
        "in its delay"
    );
    let values = FunctionValues::from([("power".to_owned(), 0.5), ("heat".to_owned(), 0.25)]);
    let mut scaled = make(times(0.0, 1.0));
    scaled.scale_by = "power".to_owned();
    scaled.secondary_by = "heat".to_owned();
    let scale = |at: &CheapEmitterAt| {
        at.submission(Some(&values), None, eye, 0.5, 0.0)
            .map(|at| at.scale)
    };
    assert_eq!(
        scale(&scaled),
        Some(1.0),
        "the effect runs, but the part takes no scale"
    );
    scaled.scaled = true;
    assert_eq!(scale(&scaled), Some(0.5));
    scaled.b_scaled = true;
    assert_eq!(scale(&scaled), Some(0.125));
    let off = FunctionValues::from([("power".to_owned(), 0.0)]);
    assert!(
        scaled.submission(Some(&off), None, eye, 0.5, 0.0).is_none(),
        "its effect is off"
    );
    let far = make(times(0.0, 1.0)).submission(None, None, [100.0, 50.0, 0.0], 0.5, 0.0);
    assert!(
        far.is_none(),
        "150 away by Manhattan distance, past the fade"
    );
}

/// At most 200 emitters a frame spawn; the rest wait.
#[test]
fn no_more_than_200_emitters_spawn_a_frame() {
    let one = emitter(
        definition(library(vec![kind(0.0, 0.0, 0.0)], [0.0; 4])),
        times(0.0, 100.0),
    );
    let count = |emitters: usize| {
        let all = vec![one.clone(); emitters];
        let mut field = CheapField::fresh(emitters);
        field.time = Some(0.0);
        field.advance(
            &all,
            &[],
            |_| None,
            &mut EffectOnsets::default(),
            None,
            [0.0; 3],
            1.0 / 60.0,
        );
        live(&field)
    };
    assert_eq!(count(200), 200, "a particle each at 60 a second");
    assert_eq!(count(250), 200);
}

/// Facing the velocity, a quad stretches along it by its speed; facing
/// the screen it lies in the camera's right and up and flips v; a quad
/// at rest facing its velocity draws nothing.
#[test]
fn a_quad_faces_its_velocity_or_the_screen() {
    let mut moving = kind(0.0, 0.0, 0.0);
    moving.rows[RENDER] = [0.0, 1.0, 2.0, 1.0];
    let screen = kind(0.0, 0.0, 0.0);
    let library = library(vec![moving, screen], [0.0; 4]);
    let mut field = CheapField::fresh(0);
    // Rotation (−63.5, 0): a unit along the basis's first axis.
    let particle = CheapParticle {
        position: [0.0; 3],
        age: 0.0,
        velocity: [0.0, 0.0, 3.0],
        delta_age: 0.0,
        parameters: [0, 127, -64, 0],
    };
    field.particles[0] = particle;
    let quad = field.vertices(&library, &view());
    assert_eq!(quad.len(), 6);
    let span = |quad: &[CheapVertex], axis: usize| {
        let values = quad.iter().map(|vertex| vertex.place[axis]);
        values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
    };
    let length = 64.0 / 63.5 * 6.0;
    assert!(
        (span(&quad, 2) - length).abs() < 1e-4,
        "stretched by 2 · 3: {}",
        span(&quad, 2)
    );
    field.particles[0].velocity = [0.0; 3];
    assert!(field.vertices(&library, &view()).is_empty(), "NaN at rest");
    field.particles[0] = CheapParticle {
        parameters: [1, 127, -64, 0],
        ..particle
    };
    let quad = field.vertices(&library, &view());
    let side = 64.0 / 63.5;
    assert!(
        (span(&quad, 0) - side).abs() < 1e-4 && (span(&quad, 2) - side).abs() < 1e-4,
        "the screen's right and up, unstretched"
    );
    assert!(span(&quad, 1) < 1e-4, "square to the view");
    let flipped = quad
        .iter()
        .find(|vertex| vertex.texcoord[0] == 0.0 && vertex.place[0] > 0.0)
        .unwrap();
    assert_eq!(flipped.texcoord[2], 3.0, "the type's layer");
    assert!(
        quad.iter().any(|vertex| vertex.texcoord[1] == 1.0),
        "{quad:?}"
    );
}

/// Colour is the type's by its intensity, the packed lighting and the
/// fade from `fade start`; alpha by the fade alone.
#[test]
fn a_quads_colour_fades_from_its_fade_start() {
    let mut tag = CheapTypeTag {
        color: [1.0, 1.0, 1.0, 0.5],
        intensity: 2.0,
        size: 1.0,
        y_scale: 1.0,
        fade_start: 0.5,
        ..Default::default()
    };
    tag.change = -1;
    let library = library(vec![tag.compiled(0, 14, false)], [0.0; 4]);
    let mut field = CheapField::fresh(0);
    let at = |age: f32| CheapParticle {
        position: [0.0; 3],
        age,
        velocity: [0.0; 3],
        delta_age: 0.0,
        parameters: [0, 127, -64, 0],
    };
    field.particles[0] = at(0.25);
    let full = field.vertices(&library, &view())[0].color;
    assert!(
        (full[0] - 2.0 * illumination(127)).abs() < 1e-5 && full[3] == 0.5,
        "{full:?}"
    );
    field.particles[0] = at(0.75);
    let half = field.vertices(&library, &view())[0].color;
    assert!(
        (half[0] - full[0] * 0.5).abs() < 1e-5 && (half[3] - 0.25).abs() < 1e-6,
        "{half:?}"
    );
}

/// Halo 4's turbulence type is a place in an array of eight: the layer
/// is the type, the transform `(int)(4 · place)` only type 0's.
#[test]
fn halo_4_takes_its_turbulence_layer_and_transform_as_the_shader_does() {
    let place = |kind: usize| (kind as f32 + 0.5) / MOST_TURBULENCE_TYPES as f32;
    assert_eq!(turbulence_of(place(0)), (0, 0));
    assert_eq!(
        turbulence_of(place(1)),
        (1, 0),
        "type 1 borrows type 0's transform"
    );
    assert_eq!(turbulence_of(place(2)), (2, 1));
    assert_eq!(turbulence_of(place(3)), (3, 1));
}

/// Halo 4's push is its type's own turbulence, spanning -1 to 1 of the
/// map; Reach's one map pushes as it is.
#[test]
fn halo_4_pushes_by_its_types_turbulence_centered() {
    let tag = CheapTypeTag {
        turbulence: 1.0,
        turbulence_type: 2,
        change: -1,
        size: 1.0,
        ..Default::default()
    };
    let mut library = library(vec![tag.compiled(0, 14, true)], [0.0; 4]);
    let kind = |value: f32| CheapTurbulence {
        texels: texels(1, 1, vec![[value, value, value, 1.0]]),
        rates: [0.0; 4],
        centered: true,
    };
    library.turbulence = vec![kind(0.5), kind(0.5), kind(1.0)];
    library.halo4 = true;
    let still = CheapParticle {
        position: [0.0; 3],
        age: 0.5,
        velocity: [0.0; 3],
        delta_age: 0.0,
        parameters: [0; 4],
    };
    let mut particle = still;
    update_particle(&mut particle, 0, &library, &[[0.0; 4]; 2], 0.1);
    assert!(
        (particle.position[0] - 0.1).abs() < 1e-6,
        "layer 2, 1 centered to 1: {:?}",
        particle.position
    );
    library.turbulence[2] = kind(0.5);
    let mut centered = still;
    update_particle(&mut centered, 0, &library, &[[0.0; 4]; 2], 0.1);
    assert_eq!(
        centered.position, [0.0; 3],
        "the map's middle pushes nothing"
    );
    library.halo4 = false;
    library.turbulence = vec![CheapTurbulence {
        centered: false,
        ..kind(0.5)
    }];
    let mut reach = still;
    update_particle(&mut reach, 0, &library, &[[0.0; 4]], 0.1);
    assert!(
        (reach.position[0] - 0.05).abs() < 1e-6,
        "Reach's map as it is: {:?}",
        reach.position
    );
}

/// Halo 4's quad is sized between `size`'s bounds by its slot; Reach's
/// is the upper bound.
#[test]
fn halo_4_sizes_each_quad_between_its_bounds() {
    let tag = CheapTypeTag {
        size_lower: 1.0,
        size: 3.0,
        y_scale: 1.0,
        change: -1,
        color: [1.0; 4],
        ..Default::default()
    };
    let mut library = library(vec![tag.compiled(0, 14, true)], [0.0; 4]);
    library.halo4 = true;
    let mut field = CheapField::fresh(0);
    let particle = CheapParticle {
        position: [0.0; 3],
        age: 0.0,
        velocity: [0.0; 3],
        delta_age: 0.0,
        parameters: [0, 127, -64, 0],
    };
    field.particles[0] = particle;
    field.particles[1] = particle;
    let width = |quad: &[CheapVertex]| {
        let values = quad.iter().map(|vertex| vertex.place[0]);
        (values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)) * 63.5 / 64.0
    };
    let quads = field.vertices(&library, &view());
    let (first, second) = (width(&quads[..6]), width(&quads[6..]));
    // Slot 0 hashes to 0, slot 1 to frac(0.863…).
    assert!((first - 1.0).abs() < 1e-4, "{first}");
    assert!(
        (second - (1.0 + 2.0 * 0.863_182_4)).abs() < 1e-3,
        "{second}"
    );
    library.halo4 = false;
    let reach = field.vertices(&library, &view());
    assert!((width(&reach[..6]) - 3.0).abs() < 1e-4);
}

/// With `BLAM_TEST_H4EK`: Halo 4's shipped type texture is the compile of
/// its library's types, all five rows of all 152, and its four kinds of
/// turbulence read.
#[test]
fn kit_halo_4s_type_table_matches_the_shipped_type_texture() {
    let Some(source) = kit_source(GameId::Halo4) else {
        return;
    };
    let library = CheapLibrary::read(
        &crate::app::model_preview::load::TagReader::new(&source),
        "fx\\cheap_particles\\global",
    )
    .expect("the library");
    assert!(library.halo4());
    assert_eq!((library.types.len(), library.turbulence.len()), (152, 4));
    let tag = load_referenced_tag_from_source(
        &source,
        "fx\\cheap_particles\\global_type_texture",
        "bitmap",
        b"bitm",
    )
    .unwrap();
    let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
    let image = bitmap.image(0).unwrap();
    let pixels = image.pixel_bytes().unwrap();
    let half = |at: usize| half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]));
    let mut checked = 0;
    for (index, found) in library.types.iter().enumerate() {
        for row in 0..5 {
            let texel: [f32; 4] =
                std::array::from_fn(|channel| half(((row * 256 + index) * 4 + channel) * 2));
            for (got, want) in texel.iter().zip(found.rows[row]) {
                assert!(
                    (got - want).abs() <= (0.01 * want.abs()).max(2e-3),
                    "type {index} row {row}: {texel:?} against {:?}",
                    found.rows[row]
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 760);
    assert!(
        library.render_texture().is_some(),
        "the render texture array reads"
    );
}

/// The type texture the kit ships is the compile of the library's types:
/// all five rows of every type agree with `globals\global_type_texture`.
#[test]
fn kit_the_type_table_matches_the_shipped_type_texture() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let library = CheapLibrary::read(
        &crate::app::model_preview::load::TagReader::new(&source),
        "globals\\global",
    )
    .expect("the library");
    assert_eq!(library.types.len(), 60);
    let tag =
        load_referenced_tag_from_source(&source, "globals\\global_type_texture", "bitmap", b"bitm")
            .unwrap();
    let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
    let image = bitmap.image(0).unwrap();
    assert_eq!((image.width(), image.height()), (256, 8));
    let pixels = image.pixel_bytes().unwrap();
    let half = |at: usize| half_to_f32(u16::from_le_bytes([pixels[at], pixels[at + 1]]));
    let mut checked = 0;
    for (index, found) in library.types.iter().enumerate() {
        for row in 0..5 {
            let texel: [f32; 4] =
                std::array::from_fn(|channel| half(((row * 256 + index) * 4 + channel) * 2));
            for (got, want) in texel.iter().zip(found.rows[row]) {
                assert!(
                    (got - want).abs() <= (0.01 * want.abs()).max(2e-3),
                    "type {index} row {row}: {texel:?} against {:?}",
                    found.rows[row]
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 300);
    assert!(
        library
            .render_texture()
            .is_some_and(|render| render.layers == 14 && render.levels.len() == 7)
    );
}

/// With `BLAM_TEST_HREK`: m52's electrical fire reads its sparks as
/// `postprocess` would leave them, at the effect scenery's marker, and
/// they are submitted only in the first tenth of a second of its cycle.
#[test]
fn kit_an_electrical_fire_reads_its_postprocessed_sparks() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let path = "levels\\solo\\m52\\fx\\city_damage_fx\\building_electrical_fire";
    let object = load_referenced_tag_from_source(&source, path, "effect_scenery", b"efsc").unwrap();
    let marker = RenderModelPreviewMarker {
        name: "marker".to_owned(),
        node_index: 0,
        position: [0.0; 3],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        scale: 1.0,
        only_under: None,
    };
    let frame = |marker: &RenderModelPreviewMarker| {
        (
            MarkerFrame {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index,
            },
            0,
        )
    };
    let emitters = read_reach_cheap_emitters(
        &crate::app::model_preview::load::TagReader::new(&source),
        &object,
        0,
        &[marker],
        frame,
    );
    assert_eq!(emitters.len(), 1, "one cpem part");
    let definition = &*emitters[0].definition;
    assert!(
        definition.correlate
            && definition.position_local
            && definition.velocity_local
            && definition.rotate_up
    );
    assert!(!definition.normalize);
    assert_eq!(definition.lod, distance_fade([180.0, 200.0]));
    let [p0, p1, p2, p3, p4] = definition.parameters;
    assert_eq!(p0, [0.0, 1.0, 0.0, 381.0]);
    assert_eq!(p1, [0.0, 0.0, 20.0, 0.0]);
    assert!(
        (p2[1] - 0.8).abs() < 1e-6
            && (p2[2] - 1.0 / 1.5).abs() < 1e-6
            && (p2[3] - (0.25 - 1.0 / 1.5)).abs() < 1e-6,
        "{p2:?}"
    );
    let names = &definition.library.names;
    let first = names
        .iter()
        .position(|known| known == "spark_electrical01")
        .unwrap() as f32;
    assert_eq!((p3, p4), ([0.0; 4], [first, 0.0, 0.0, 0.0]));
    assert_eq!(
        definition.spawn_rate.evaluate(0.0, 0.5),
        5000.0,
        "the mapping's, not version 0's `spawn rate`"
    );
    assert_eq!(
        pack_sbyte4([0.0, packed_lighting(definition.lighting, 1.0), 0.0, 0.0])[1],
        127
    );
    let on = |time: f32| {
        emitters[0]
            .submission(None, None, [0.0; 3], time, 0.0)
            .is_some()
    };
    assert!(on(0.05), "in its event of a tenth of a second");
    assert!(!on(0.5), "after it");
    assert!(on(15.05), "the next cycle, 15 seconds on");
}

/// With `BLAM_TEST_HREK`: m45's huge space junk attaches its effect on
/// `marker`, and its cheap particle part sits at `up`, a vector
/// `build_gravity_effect_vectors` makes there, not a marker of the model.
#[test]
fn kit_space_junk_emits_from_its_up_vector() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let path = "levels\\solo\\m45\\fx\\space_junk_huge";
    let object = load_referenced_tag_from_source(&source, path, "effect_scenery", b"efsc").unwrap();
    let marker = RenderModelPreviewMarker {
        name: "marker".to_owned(),
        node_index: 0,
        position: [1.0, 2.0, 3.0],
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        scale: 1.0,
        only_under: None,
    };
    let frame = |marker: &RenderModelPreviewMarker| {
        (
            MarkerFrame {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index,
            },
            0,
        )
    };
    let emitters = read_reach_cheap_emitters(
        &crate::app::model_preview::load::TagReader::new(&source),
        &object,
        0,
        &[marker],
        frame,
    );
    assert_eq!(
        emitters.len(),
        1,
        "one cpem part, though the model has no `up` marker"
    );
    let emitter = &emitters[0];
    assert_eq!(
        emitter.position,
        [1.0, 2.0, 3.0],
        "at the attachment's marker"
    );
    assert_eq!(
        (emitter.forward, emitter.up),
        ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        "facing up"
    );
}

/// IEEE half to single, for the kit's `abgrfp16` type texture.
fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa / 1024.0 * 2f32.powi(-14),
        31 => f32::INFINITY,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}
