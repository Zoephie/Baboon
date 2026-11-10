use super::super::super::load_referenced_tag_from_source;
use super::properties::{
    INVALID, MOST_PROFILES, ORIGIN_FADED, PROFILE_RANDOM, TRACER_RANDOM, TRACER_SEED_1,
    TRACER_SEED_2, TracerProperty, TracerState, state_named,
};
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use blam_tags::TagFunction;
use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
use std::sync::Arc;

fn function(kind: FunctionType, low: f32, high: f32) -> TagFunction {
    let mut function = H2Function::new(kind);
    function.set_clamp_range(low, high).unwrap();
    TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap()
}

fn property(kind: FunctionType, input: u8, range: u8) -> TracerProperty {
    TracerProperty {
        input,
        range,
        adds: None,
        modifier: 0,
        function: function(kind, 0.0, 1.0),
        ends: [[0.0; 3]; 2],
    }
}

#[test]
fn inputs_read_by_their_names() {
    assert_eq!(state_named("profile position"), 1);
    assert_eq!(state_named("tracer random"), TRACER_RANDOM);
    assert_eq!(state_named("effect b scale"), 17);
    assert_eq!(state_named("invalid state --- please set again"), INVALID);
}

/// `profile random` takes the correlation above the highest used, and
/// `tracer random` the first free of two; a constant takes none.
#[test]
fn randoms_take_seeds_as_the_postprocess_hands_them_out() {
    let mut used = 0;
    let mut first = property(FunctionType::Identity, PROFILE_RANDOM, 0);
    first.allocate_seeds(&mut used);
    assert_eq!(first.input, 4);
    let mut second = property(FunctionType::Identity, PROFILE_RANDOM, 0);
    second.allocate_seeds(&mut used);
    assert_eq!(second.input, 5);
    let mut tracer = property(FunctionType::Identity, TRACER_RANDOM, 0);
    tracer.allocate_seeds(&mut used);
    assert_eq!(tracer.input, TRACER_SEED_1);
    let mut again = property(FunctionType::Identity, TRACER_RANDOM, 0);
    again.allocate_seeds(&mut used);
    assert_eq!(again.input, TRACER_SEED_2);
    let mut constant = property(FunctionType::Constant, PROFILE_RANDOM, 0);
    constant.allocate_seeds(&mut used);
    assert_eq!(constant.input, PROFILE_RANDOM, "a constant reads nothing");
    // A tag that names correlation 3 itself pushes the next past it.
    let mut named = 1 << 6;
    let mut after = property(FunctionType::Identity, PROFILE_RANDOM, 0);
    after.allocate_seeds(&mut named);
    assert_eq!(after.input, 7);
}

/// `numProfiles`: at least two, at most a strip's 128; the step between
/// them the inverse of the product, held to 1.
#[test]
fn a_beam_has_two_profiles_at_least_and_a_strip_at_most() {
    assert_eq!(profile_count(0.1, 1.0), 2);
    assert_eq!(profile_count(2.5, 2.0), 5);
    assert_eq!(profile_count(1000.0, 1.0), MOST_PROFILES);
    assert_eq!(percentile_step(2.5, 2.0), 0.2);
    assert_eq!(percentile_step(0.1, 1.0), 1.0);
    assert_eq!(percentile_step(0.0, 1.0), 1.0, "an infinite step is 1");
    assert_eq!(percentile_step(-1.0, 1.0), 0.0);
}

/// The far LOD runs from its start value to its end value between its
/// distances; with neither value set it is 1 near and 0 far.
#[test]
fn the_far_lod_ramps_between_its_distances() {
    let lod = TracerLod {
        start: 10.0,
        end: 20.0,
        start_value: 1.0,
        end_value: 0.0,
    };
    assert_eq!(lod.at(5.0), 1.0);
    assert_eq!(lod.at(15.0), 0.5);
    assert_eq!(lod.at(25.0), 0.0);
}

/// `RawTracerProfileState`: position and age as they are, velocity and
/// initial alpha, size and intensity, offset, length and lifespan as
/// halves, random, rotation, black point, palette and place as
/// truncated ushorts, the colours as ARGB8.
#[test]
fn a_profile_packs_as_the_card_reads_it() {
    let profile = ProfileState {
        position: [1.0, 2.0, 3.0],
        age: 0.5,
        velocity: [0.0, 0.0, 2.0],
        initial_alpha: 1.0,
        random: [1.0, 0.0, 0.5, 0.0],
        size: 0.5,
        intensity: 2.0,
        rotation: 0.25,
        palette: 0.5,
        percentile: 1.0,
        length: 3.0,
        lifespan: 2.0,
        color: [1.0, 0.0, 0.0, 0.5],
        initial_color: [1.0, 1.0, 1.0, 0.0],
        ..Default::default()
    };
    let words = pack_profile(&profile);
    assert_eq!(words[..4], [1.0f32, 2.0, 3.0, 0.5].map(f32::to_bits));
    assert_eq!(
        words[5],
        0x4000 | (0x3C00 << 16),
        "velocity z 2, initial alpha 1"
    );
    assert_eq!(words[6], 65535);
    assert_eq!(words[7], 32767);
    assert_eq!(words[8], 0x3800 | (0x4000 << 16), "size 0.5, intensity 2");
    assert_eq!(words[10] & 0xFFFF, 16383, "a quarter turn");
    assert_eq!(words[11] >> 16, 65535, "the last profile's place");
    assert_eq!(words[12], 0x4200 | (0x4000 << 16), "length 3, lifespan 2");
    assert_eq!(words[13], (127 << 24) | (255 << 16));
    assert_eq!(words[14], 0x00FF_FFFF, "white at an exponent of 0");
    let other = pack_profile(&ProfileState {
        rotation: 0.5,
        ..profile
    });
    assert_ne!(other[10], words[10]);
}

fn fixed(value: f32) -> TracerProperty {
    TracerProperty {
        function: function(FunctionType::Constant, value, value),
        ..property(FunctionType::Constant, INVALID, INVALID)
    }
}

/// A ribbon two profiles a unit, each living `lifespan`.
fn trail(lifespan: f32, origin_fade: Option<[f32; 2]>) -> TracerDefinition {
    TracerDefinition {
        shape: 0,
        ngon_sides: 0,
        appearance: 0,
        fade: [0.0; 4],
        origin_fade,
        lifespan: fixed(lifespan),
        acceleration: fixed(0.0),
        length: fixed(1.0),
        offset: fixed(0.0),
        density: fixed(2.0),
        size: fixed(1.0),
        profile_offset: fixed(0.0),
        rotation: fixed(0.0),
        color: fixed(1.0),
        alpha: fixed(1.0),
        alpha2: fixed(1.0),
        black_point: fixed(0.0),
        palette: fixed(0.0),
        intensity: fixed(1.0),
        uv_tiling: [1.0; 2],
        uv_scroll: [0.0; 2],
        sort_bias: 0.0,
        most_profiles: 2,
    }
}

/// Run `running` along `path`, a frame of `dt` at each point.
fn run(tracer: &TracerDefinition, running: &mut RunningTracer, path: &[[f32; 3]], dt: f32) {
    let state = TracerState {
        effect_a: 1.0,
        ..Default::default()
    };
    for &at in path {
        if running.location.replace(at).is_none() {
            running.last_placed = at;
        }
        let mut spawned = 0;
        running.place_to(tracer, &state, at, &mut spawned);
        running.update(tracer, &state, dt);
    }
}

/// At rest only the temporary leading profile sits at the origin, so
/// `Render` has one profile and draws nothing.
#[test]
fn a_trail_at_rest_keeps_one_profile() {
    let tracer = trail(1.0, None);
    let mut running = RunningTracer {
        random: 1,
        ..Default::default()
    };
    run(&tracer, &mut running, &[[0.0; 3]; 10], 0.1);
    assert_eq!(running.count(), 1);
    assert!(running.leading_temporary);
}

/// Moving lays profiles a density apart behind the location, oldest
/// first along the trail, with the leading one written over each frame.
#[test]
fn a_moving_trail_lays_profiles_behind_it() {
    let tracer = trail(10.0, None);
    let mut running = RunningTracer {
        random: 1,
        ..Default::default()
    };
    let path: Vec<[f32; 3]> = (0..4).map(|step| [step as f32, 0.0, 0.0]).collect();
    run(&tracer, &mut running, &path, 0.1);
    // The tiny first stretch over the leading profile, two in the first
    // unit, then one (1.9996 a unit, floored) and two: the floor drops
    // what a frame's distance falls short of.
    assert_eq!(running.count(), 6);
    assert!((running.length - 3.0).abs() < 1e-3, "{}", running.length);
    let lengths: Vec<f32> = running
        .rows
        .iter()
        .flat_map(|row| &row.slots)
        .map(|profile| profile.length)
        .collect();
    assert!(
        lengths.windows(2).all(|pair| pair[0] <= pair[1]),
        "{lengths:?}"
    );
    let x = running.rows.last().unwrap().slots.last().unwrap().position[0];
    assert!((x - 3.0).abs() < 1e-3, "the newest is at the location: {x}");
}

/// An origin fade gets real profiles at both its ends, so the fade runs
/// straight between them. Before the first real one a profile's length
/// stays 0 (`InitializeProfile`), so the first lands at the cutoff with
/// none.
#[test]
fn an_origin_fade_gets_profiles_at_its_ends() {
    let tracer = TracerDefinition {
        density: fixed(0.1),
        ..trail(10.0, Some([1.0, 0.5]))
    };
    let mut running = RunningTracer {
        random: 1,
        ..Default::default()
    };
    run(&tracer, &mut running, &[[0.0; 3], [3.0, 0.0, 0.0]], 0.1);
    let profiles: Vec<&ProfileState> = running.rows.iter().flat_map(|row| &row.slots).collect();
    let places: Vec<f32> = profiles.iter().map(|profile| profile.position[0]).collect();
    assert_eq!(places.len(), 3, "{places:?}");
    assert!(
        (places[0] - 1.0).abs() < 1e-4 && (places[1] - 1.5).abs() < 1e-4,
        "{places:?}"
    );
    let lengths: Vec<f32> = profiles.iter().map(|profile| profile.length).collect();
    assert_eq!(lengths, [0.0, 0.5, 2.0]);
    assert!(
        running.leading_temporary,
        "the last reaches the location for now"
    );
}

/// No more than ten profiles a frame, two once the frame has had 25.
#[test]
fn a_frame_lays_down_no_more_than_its_allowance() {
    let tracer = TracerDefinition {
        density: fixed(100.0),
        ..trail(10.0, None)
    };
    let state = TracerState {
        effect_a: 1.0,
        ..Default::default()
    };
    let mut running = RunningTracer {
        random: 1,
        placed_real: true,
        ..Default::default()
    };
    let mut spawned = 0;
    running.place_to(&tracer, &state, [5.0, 0.0, 0.0], &mut spawned);
    assert_eq!(spawned, 10);
    let mut late = RunningTracer {
        random: 1,
        placed_real: true,
        ..Default::default()
    };
    let mut spawned = 30;
    late.place_to(&tracer, &state, [5.0, 0.0, 0.0], &mut spawned);
    assert_eq!(spawned, 32);
}

/// A row goes as a whole once its longest lifespan has run; until then a
/// dead profile stays, aged past 1, for the vertex shader to drop.
#[test]
fn profiles_die_a_row_at_a_time() {
    let tracer = trail(0.5, None);
    let mut running = RunningTracer {
        random: 1,
        ..Default::default()
    };
    run(&tracer, &mut running, &[[0.0; 3], [2.0, 0.0, 0.0]], 0.1);
    let laid = running.count();
    assert!(laid > 1);
    let state = TracerState::default();
    for _ in 0..5 {
        running.update(&tracer, &state, 0.1);
    }
    assert_eq!(running.count(), laid, "still within the row's time");
    assert!(running.rows[0].slots[0].age >= 1.0);
    for _ in 0..3 {
        running.update(&tracer, &state, 0.1);
    }
    assert_eq!(running.count(), 0);
}

#[test]
fn two_tracers_get_different_seeds_inside_the_unit_range() {
    let (first, second) = (seeds_of(0), seeds_of(1));
    assert_ne!(first, second);
    assert!(
        first
            .iter()
            .chain(&second)
            .all(|seed| (0.0..1.0).contains(seed))
    );
}

/// With `BLAM_TEST_H4EK`: the beam hole's tracer reads, draws its
/// profiles from one marker to the other, and the kit's point-to-point
/// systems all read onto their materials.
#[test]
fn kit_halo_4_tracers_read_and_run_between_their_markers() {
    let Some(root) = kit_root(GameId::Halo4) else {
        return;
    };
    let source = super::super::super::particles::loose_kit(&root, crate::core::game::GameId::Halo4);
    let path = "environments\\solo\\m90_sacrifice\\fx\\beams\\parts\\beam_beamhole";
    let tag = load_referenced_tag_from_source(&source, path, "tracer_system", b"trac").expect(path);
    let element = elements(tag.root(), "tracers")[0];
    let tracer = read_tracer(
        &crate::app::model_preview::load::TagReader::new(&source),
        element,
    )
    .expect("its tracer");
    assert!(
        tracer
            .material
            .pass
            .vertex
            .contains("vs_tracer_profile_state_buffer")
    );
    assert!(
        tracer.appearance & ORIGIN_FADED != 0,
        "the beam hole fades from its origin"
    );
    let at = H4TracerAt {
        object: 0,
        start: (usize::MAX, [0.0; 3]),
        path: TracerPath::PointToPoint {
            end: (usize::MAX, [0.0, 0.0, 4.0]),
        },
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        seeds: seeds_of(0),
        times: Arc::new(EffectTimes::read_nothing()),
        event: 0,
        lod: TracerLod::read(tag.root()),
        definition: Arc::new(tracer),
    };
    let draw = at
        .draw(None, None, [10.0, 0.0, 0.0], 1.0, 0.0)
        .expect("it draws");
    assert!(draw.states.len() >= 2);
    assert_eq!(draw.direction, [0.0, 0.0, 1.0]);
    let z = |words: &[u32; 16]| f32::from_bits(words[2]);
    let last = draw.states.last().unwrap();
    assert!(
        z(last) > z(&draw.states[0]),
        "the profiles run toward the end"
    );
    assert!(
        draw.capped_length <= 4.0 + 1e-4,
        "held to the gap: {}",
        draw.capped_length
    );
    // Every point-to-point tracer in the kit reads onto its material.
    let (mut read, mut disabled, mut failures) = (0, 0, Vec::new());
    let mut stack = vec![std::path::PathBuf::from(&root)];
    while let Some(folder) = stack.pop() {
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .is_none_or(|extension| extension != "tracer_system")
            {
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap().with_extension("");
            let relative = relative.to_string_lossy().replace('/', "\\");
            let tag = load_referenced_tag_from_source(&source, &relative, "tracer_system", b"trac")
                .unwrap();
            if !has_flag(tag.root(), "tracer system flags", "point-to-point") {
                continue;
            }
            for element in elements(tag.root(), "tracers") {
                // The postprocess deletes these.
                if has_flag(element, "flags", "disabled for debugging") {
                    disabled += 1;
                    continue;
                }
                match read_tracer(
                    &crate::app::model_preview::load::TagReader::new(&source),
                    element,
                ) {
                    Some(_) => read += 1,
                    None => failures.push(relative.clone()),
                }
            }
        }
    }
    eprintln!(
        "NOTE KIT {read} point-to-point tracers read, {disabled} disabled, {} failed: {failures:?}",
        failures.len()
    );
    assert!(failures.is_empty());
    assert!(read > 100, "tracers read: {read}");
}
