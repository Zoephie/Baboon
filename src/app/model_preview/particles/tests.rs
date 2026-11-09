use super::super::{TagSource, load_referenced_tag_from_source};
use super::*;
use crate::app::model_preview::harness::kit::{kit_root, kit_source};
use crate::core::game::GameId;
use blam_tags::TagFunction;
use std::sync::Arc;

#[test]
fn a_custom_shape_picks_its_point_by_radius_and_strays_by_angle() {
    let mut random = Random(1);
    let point = |x: f32| CustomPoint {
        position: [x, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    };
    let points = [point(1.0), point(2.0), point(3.0), point(4.0)];
    assert_eq!(
        emit_from_points(&points, 0.0, 0.0, &mut random),
        (point(1.0).position, [0.0, 0.0, 1.0])
    );
    assert_eq!(
        emit_from_points(&points, 0.5, 0.0, &mut random).0,
        point(3.0).position
    );
    assert_eq!(
        emit_from_points(&points, 1.0, 0.0, &mut random).0,
        point(4.0).position,
        "the last, not past it"
    );
    assert_eq!(
        emit_from_points(&points, -1.0, 0.0, &mut random).0,
        point(1.0).position
    );
    let (_, strayed) = emit_from_points(&points, 0.0, 90.0, &mut random);
    assert!(strayed != [0.0, 0.0, 1.0]);
    assert_eq!(
        emit_from_points(&[], 0.5, 0.0, &mut random),
        ([0.0; 3], [0.0; 3])
    );
}

#[test]
fn only_odst_and_reach_let_a_particle_that_faces_nothing_stay_still() {
    let rules = |game| ParticleRules::of(Some(game));
    let (halo3, odst, reach) = (
        &rules(GameId::Halo3),
        &rules(GameId::Halo3Odst),
        &rules(GameId::HaloReach),
    );
    assert!(still_particles_move(halo3, Billboard::ScreenFacing));
    assert!(!still_particles_move(odst, Billboard::ScreenFacing));
    assert!(still_particles_move(odst, Billboard::ScreenParallel));
    assert!(still_particles_move(odst, Billboard::VelocityHorizontal));
    assert!(!still_particles_move(reach, Billboard::ScreenFacing));
    assert!(still_particles_move(reach, Billboard::ScreenPerpendicular));
}

#[test]
fn a_lod_left_empty_takes_the_postprocess_defaults() {
    assert_eq!(
        baked_lod([0.0; 4], true, [].into_iter()),
        [0.0, 20.0, 1.0, 5.0]
    );
    assert_eq!(
        baked_lod([0.0; 4], true, [150.0].into_iter()),
        [-0.1, 20.0, 3.0, 5.0]
    );
    assert_eq!(
        baked_lod([0.0; 4], false, [150.0].into_iter()),
        [0.0, 20.0, 1.0, 5.0],
        "not for one camera"
    );
    assert_eq!(
        baked_lod([1.0, 8.0, 2.0, 3.0], true, [150.0].into_iter()),
        [1.0, 8.0, 2.0, 3.0]
    );
}

#[test]
fn a_direction_turns_around_rather_than_across() {
    let ends = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let middle = slerped(ends, 0.5);
    assert!(
        (dot(middle, middle).sqrt() - 1.0).abs() < 1e-3,
        "{middle:?}"
    );
    assert!((slerped(ends, 0.0)[0] - 1.0).abs() < 1e-3);
    assert!((slerped(ends, 1.0)[1] - 1.0).abs() < 1e-3);
    let same = slerped([[0.0, 0.0, 2.0], [0.0, 0.0, 2.0]], 0.5);
    assert!(same.iter().all(|axis| axis.is_finite()));
}

#[test]
fn only_what_landed_slowly_rests() {
    assert!(comes_to_rest(true, [0.2, 0.0, 0.0]));
    assert!(!comes_to_rest(true, [0.3, 0.0, 0.0]));
    assert!(!comes_to_rest(false, [0.0; 3]));
}

#[test]
fn a_line_runs_along_y_and_a_breakable_has_nothing_of_its_own() {
    let mut random = Random(3);
    let (place, _) = emit(EmissionShape::Line, 2.0, 0.0, false, &mut random);
    assert_eq!(place[0], 0.0);
    assert!(place[1].abs() <= 2.0 && place[1] != 0.0);
    assert_eq!(
        emit(EmissionShape::Breakable, 2.0, 30.0, false, &mut random),
        ([0.0; 3], [0.0; 3])
    );
}

#[test]
fn an_impact_with_no_size_sprays_from_one_point_at_its_angle() {
    let mut random = Random(5);
    for shape in [EmissionShape::ImpactContour, EmissionShape::ImpactArea] {
        let (place, velocity) = emit(shape, 9.0, 60.0, false, &mut random);
        assert_eq!(place, [0.0; 3]);
        assert!((velocity[0] - 0.5).abs() < 1e-5);
        assert!((dot(velocity, velocity) - 1.0).abs() < 1e-5);
    }
}

#[test]
fn a_flat_distortion_keeps_its_lean_and_a_curved_one_tilts_it() {
    let plane = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    assert_eq!(curved_lean(plane, plane, [1.0, 0.0], 0.0)[0], plane[0]);
    let curved = curved_lean(plane, plane, [1.0, 0.0], 0.5);
    assert!(curved[0][2].abs() > 0.5, "{curved:?}");
    assert!(
        (curved[1][1] - 1.0).abs() < 1e-3,
        "the axis itself stays: {curved:?}"
    );
}

#[test]
fn a_particles_axes_look_along_it_and_a_still_one_gets_a_default() {
    let axes = axes_along([0.0, 2.0, 0.0]);
    assert_eq!(axes[0], [0.0, 1.0, 0.0]);
    assert!(dot(axes[1], axes[0]).abs() < 1e-6 && dot(axes[2], axes[0]).abs() < 1e-6);
    assert_eq!(axes_along([0.0; 3])[0], [1.0, 0.0, 0.0]);
}

#[test]
fn an_older_tags_names_read_as_the_same_inputs() {
    assert_eq!(
        ParticleInput::named("emitter age"),
        ParticleInput::SystemAge
    );
    assert_eq!(
        ParticleInput::named("emitter random"),
        ParticleInput::SystemRandom
    );
    assert_eq!(
        ParticleInput::named("emitter correlation 2"),
        ParticleInput::SystemSeed(1)
    );
    assert_eq!(ParticleInput::named("system lod"), ParticleInput::Lod);
    assert_eq!(ParticleInput::named("no such input"), ParticleInput::Other);
}

#[test]
fn a_scale_multiplies_what_a_property_reads() {
    let state = ParticleState::default();
    let rate = ParticleProperty::fixed(2.0).unwrap();
    assert_eq!(rate.scalar(&state), 2.0);
    assert_eq!(rate.scaled(30.0).scalar(&state), 60.0);
}

#[test]
fn a_lightmap_tint_is_the_probes_constant_and_keeps_only_two_stops() {
    let scale = (4.0 * std::f32::consts::PI).sqrt();
    let mut lighting = [[0.0; 4]; 10];
    lighting[0] = [0.5 * scale, 0.25 * scale, 0.0, 0.0];
    let tint = lightmap_tint(&lighting);
    assert!((tint[0] - 0.5).abs() < 1e-5 && (tint[1] - 0.25).abs() < 1e-5);
    lighting[0] = [16.0 * scale, 8.0 * scale, 0.0, 0.0];
    let tint = lightmap_tint(&lighting);
    assert!((tint[0] - 2.0).abs() < 1e-5, "{tint:?}");
}

#[test]
fn detail_fades_in_and_out_across_its_feathers() {
    let lod = Some([10.0, 100.0, 10.0, 20.0]);
    assert_eq!(lod_amount(None, 1000.0), 1.0, "always");
    assert_eq!(lod_amount(lod, 5.0), 0.0);
    assert_eq!(lod_amount(lod, 15.0), 0.5);
    assert_eq!(lod_amount(lod, 50.0), 1.0);
    assert_eq!(lod_amount(lod, 90.0), 0.5);
    assert_eq!(lod_amount(lod, 150.0), 0.0);
}

#[test]
fn a_particle_falling_through_the_ground_bounces_off_it() {
    let ground = ([0.0, 0.0, 1.0], 0.0);
    let (place, velocity) =
        bounce([0.0, 0.0, 0.1], [1.0, 0.0, -2.0], ground, 0.1, 0.5, 0.25).unwrap();
    assert!((place[2] - PUSH_OUT).abs() < 1e-6, "{place:?}");
    assert!(
        (place[0] - 0.05).abs() < 1e-6,
        "stopped where it met: {place:?}"
    );
    assert_eq!(velocity, [0.75, 0.0, 1.0]);
}

#[test]
fn a_step_that_stays_above_or_below_the_ground_meets_nothing() {
    let ground = ([0.0, 0.0, 1.0], 0.0);
    assert_eq!(
        bounce([0.0, 0.0, 1.0], [0.0, 0.0, -1.0], ground, 0.1, 0.5, 0.5),
        None
    );
    assert_eq!(
        bounce([0.0, 0.0, -1.0], [0.0, 0.0, -1.0], ground, 0.1, 0.5, 0.5),
        None
    );
    assert_eq!(
        bounce([0.0, 0.0, 0.1], [0.0, 0.0, 5.0], ground, 0.1, 0.5, 0.5),
        None
    );
}

#[test]
fn the_ground_is_found_from_a_lifted_frame_too() {
    // A frame 2 above the ground: its origin is 2 up from the plane.
    let hit = bounce(
        [0.0, 0.0, -1.9],
        [0.0, 0.0, -2.0],
        ([0.0, 0.0, 1.0], 2.0),
        0.1,
        0.0,
        0.0,
    );
    assert!(hit.is_some());
}

#[test]
fn a_turn_about_x_carries_y_to_z_and_leaves_x() {
    let turned = axis_rotation([1.0, 0.0, 0.0], std::f32::consts::FRAC_PI_2);
    assert!((turned[0][0] - 1.0).abs() < 1e-6);
    assert!((turned[1][2] - 1.0).abs() < 1e-6, "{turned:?}");
    assert!((turned[2][1] + 1.0).abs() < 1e-6, "{turned:?}");
}

#[test]
fn a_reference_axis_rotates_the_picked_axis_round() {
    let mut random = Random(1);
    assert_eq!(
        ParticleAxis::Constant.pick(0, 0.0, &mut random),
        [1.0, 0.0, 0.0]
    );
    assert_eq!(
        ParticleAxis::Constant.pick(1, 0.0, &mut random),
        [0.0, 1.0, 0.0]
    );
    assert_eq!(
        ParticleAxis::Constant.pick(2, 0.0, &mut random),
        [0.0, 0.0, 1.0]
    );
}

#[test]
fn a_whole_sprite_sits_around_its_place_and_an_offset_pins_its_edge() {
    let whole = [0.0, 1.0, 0.0, 1.0];
    assert_eq!(
        sprite_corner(whole, [0.5, 0.5], [0.0, 0.0], 1.0),
        [-1.0, -1.0, 2.0, 2.0]
    );
    // `blast_wave_half`: the left edge at the origin.
    assert_eq!(
        sprite_corner(whole, [0.5, 0.5], [-0.5, 0.0], 1.0),
        [0.0, -1.0, 2.0, 2.0]
    );
    // The sign of y is the other way.
    assert_eq!(
        sprite_corner(whole, [0.5, 0.5], [0.0, -0.5], 1.0),
        [-1.0, -2.0, 2.0, 2.0]
    );
}

#[test]
fn a_narrow_sprite_narrows_and_a_wide_plate_widens() {
    let narrow = sprite_corner([0.0, 0.5, 0.0, 1.0], [0.25, 0.5], [0.0, 0.0], 1.0);
    assert_eq!(narrow, [-0.5, -1.0, 1.0, 2.0]);
    let wide = sprite_corner([0.0, 1.0, 0.0, 1.0], [0.5, 0.5], [0.0, 0.0], 2.0);
    assert_eq!(wide, [-2.0, -1.0, 4.0, 2.0]);
}

#[test]
fn inputs_read_by_their_names_with_or_without_the_second_spelling() {
    assert_eq!(
        ParticleInput::named("system age{emitter age}"),
        ParticleInput::SystemAge
    );
    assert_eq!(ParticleInput::named("system age"), ParticleInput::SystemAge);
    assert_eq!(
        ParticleInput::named("particle correlation 3"),
        ParticleInput::Seed(2)
    );
    assert_eq!(ParticleInput::named("no such state"), ParticleInput::Other);
}

#[test]
fn randoms_take_seeds_in_turn_and_wrap() {
    let mut seeds = SeedAllocator::default();
    let mut take = |mut input| {
        seeds.allocate(&mut input);
        input
    };
    assert_eq!(take(ParticleInput::Random), ParticleInput::Seed(0));
    assert_eq!(take(ParticleInput::Random), ParticleInput::Seed(1));
    assert_eq!(
        take(ParticleInput::SystemRandom),
        ParticleInput::SystemSeed(0)
    );
    assert_eq!(take(ParticleInput::Age), ParticleInput::Age);
}

#[test]
fn a_sprayer_starts_at_the_origin_inside_its_cone() {
    let mut random = Random(7);
    for _ in 0..50 {
        let (place, direction) = emit(EmissionShape::Sprayer, 2.0, 10.0, false, &mut random);
        assert_eq!(place, [0.0; 3]);
        assert!(
            direction[0] >= 10f32.to_radians().cos() - 1e-5,
            "{direction:?}"
        );
    }
}

#[test]
fn a_globe_starts_on_its_radius_heading_out_and_an_implode_in() {
    let mut random = Random(9);
    let (place, direction) = emit(EmissionShape::Globe, 2.0, 0.0, false, &mut random);
    assert!((dot(place, place).sqrt() - 2.0).abs() < 1e-4);
    assert!(dot(place, direction) > 0.0);
    let (place, direction) = emit(EmissionShape::Implode, 2.0, 0.0, false, &mut random);
    assert!(dot(place, direction) < 0.0);
}

#[test]
fn a_one_shot_stops_short_of_wrapping_and_a_single_frame_never_moves() {
    assert_eq!(frame_variant(0.5, 4, false, false), 2.0);
    assert_eq!(frame_variant(0.5, 4, true, false), 1.5);
    assert_eq!(frame_variant(0.25, 4, false, true), 3.0);
    assert_eq!(frame_variant(0.9, 1, false, false), 0.0);
}

#[test]
fn the_same_seed_runs_the_same() {
    let (mut a, mut b) = (Random(3), Random(3));
    assert_eq!(a.next(), b.next());
    assert_ne!(a.next(), Random(4).next());
}

/// `decide_whether_cpu_or_gpu`: forcing the processor wins, then forcing
/// the card; otherwise the card takes a particle with no attachments
/// whose movement asks for nothing past `physics`, and a particle with no
/// attachments moves to the card when it rests unless something was forced.
#[test]
fn tool_puts_an_emitter_on_the_card_only_when_nothing_needs_the_processor() {
    use ParticleUpdate::{Cpu, Gpu};
    // The set options, as has_flag names them.
    let named = |names: &'static [&'static str]| move |wanted: &str| names.contains(&wanted);
    let decide =
        |system: &'static [&'static str], attachments, movement: &'static [&'static str]| {
            particle_update(named(system), attachments, named(movement))
        };
    assert_eq!(decide(&[], false, &[]), (Gpu, true));
    assert_eq!(decide(&[], false, &["physics"]), (Gpu, true));
    assert_eq!(
        decide(&[], true, &["physics"]),
        (Cpu, false),
        "attachments play on the processor"
    );
    assert_eq!(
        decide(&[], false, &["physics", "collide with structure"]),
        (Cpu, true),
        "on to the card at rest"
    );
    assert_eq!(decide(&[], false, &["swarm"]), (Cpu, true));
    assert_eq!(
        decide(&["force gpu updating"], true, &["wind"]),
        (Gpu, false)
    );
    let force_cpu = &["force cpu updating"];
    assert_eq!(
        decide(force_cpu, false, &["collide with structure"]),
        (Cpu, false),
        "forced stays put"
    );
    let both = &["force cpu updating", "force gpu updating"];
    assert_eq!(
        decide(both, false, &[]),
        (Cpu, false),
        "forcing the processor wins"
    );
}

/// Tool keeps one movement for each flagged kind, in its table's order
/// whatever order the tag lists them in.
#[test]
fn movements_follow_the_flags_in_tools_order() {
    use ParticleController::{Collider, Flock, Physics};
    // The set options, as has_flag names them.
    let named = |names: &'static [&'static str]| move |wanted: &str| names.contains(&wanted);
    assert_eq!(
        flagged_controllers(named(&["collide with bipeds", "swarm", "physics"])),
        [Physics, Flock, Collider]
    );
    assert_eq!(flagged_controllers(named(&[])), []);
}

/// The first emitter of an effect's first system, from the Halo 3 kit.
fn kit_emitter(root: &std::path::Path, path: &str) -> Arc<Emitter> {
    let source = TagSource::LooseFolder {
        root: root.into(),
        game: Some(GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let effect = load_referenced_tag_from_source(&source, path, "effect", b"effe").unwrap();
    let event = elements(effect.root(), "events")
        .into_iter()
        .next()
        .unwrap();
    let system = elements(event, "particle systems")
        .into_iter()
        .next()
        .unwrap();
    Arc::clone(
        &read_system_parts(
            &crate::app::model_preview::load::TagReader::new(&source),
            system,
        )
        .unwrap()
        .emitters[0],
    )
}

/// With `BLAM_TEST_H3EK`: plain physics goes on the card, colliding stays
/// on the processor, and a template's movements need the emitter's own
/// flags, as tool's `postprocess_internal` keeps them.
#[test]
fn kit_emitters_update_where_tool_puts_them_and_move_as_their_flags_say() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let plain = kit_emitter(
        &root,
        "fx\\cinematics\\120lc_activation\\02\\shot_03\\halochunk_impact",
    );
    assert_eq!(plain.update, ParticleUpdate::Gpu);
    assert_eq!(plain.controllers, [ParticleController::Physics]);
    let state = ParticleState {
        system_age: 0.5,
        effect_a: 1.0,
        effect_b: 1.0,
        lod: 1.0,
        ..Default::default()
    };
    let physics = plain
        .physics
        .as_ref()
        .map(|physics| physics.on_card(&state));
    assert_eq!(
        physics,
        Some([0.0, 2.0, 1.0]),
        "gravity, air and rotational friction"
    );
    let colliding = kit_emitter(
        &root,
        "fx\\cinematics\\100ld_confront_truth\\07\\shot_04\\tentacle_erupt",
    );
    assert_eq!(colliding.update, ParticleUpdate::Cpu);
    assert!(
        colliding
            .controllers
            .contains(&ParticleController::Collider)
    );
    // `_default`'s physics, with none of the emitter's own flags set.
    let unflagged = kit_emitter(
        &root,
        "fx\\cinematics\\040lb_cov_flee\\08\\shot_4\\flood_spew",
    );
    assert!(
        unflagged.controllers.is_empty(),
        "{:?}",
        unflagged.controllers
    );
    assert!(
        unflagged.physics.is_none(),
        "the template's physics is not the emitter's"
    );
}

/// On the card a particle's spawn frame moves nothing; after that it moves
/// by the velocity it had before its emitter's air friction (2 here, with
/// no gravity) slows it, where the processor slows it first.
#[test]
fn a_particle_on_the_card_waits_a_frame_then_moves_before_it_accelerates() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let emitter = kit_emitter(
        &root,
        "fx\\cinematics\\120lc_activation\\02\\shot_03\\halochunk_impact",
    );
    let particle = Particle {
        id: 1,
        colliding: false,
        landed: false,
        resting: false,
        spawned_this_frame: true,
        on_card: false,
        axis: [0.0, 0.0, 1.0],
        age: 0.0,
        inverse_lifespan: 0.1,
        position: [0.0; 3],
        velocity: [1.0, 0.0, 0.0],
        rotation: 0.0,
        rotation_rate: 0.0,
        frame: 0.0,
        frame_rate: 0.0,
        seeds: [0.0; SEEDS],
        emission_time: 0.0,
        flip: [false; 3],
        sequence: 0,
        system_seeds: [0.0; SYSTEM_SEEDS],
    };
    let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 1.0 / 60.0);
    let state = ParticleState {
        system_age: 0.5,
        effect_a: 1.0,
        effect_b: 1.0,
        lod: 1.0,
        ..Default::default()
    };
    emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, false);
    assert_eq!(
        (particles[0].position, particles[0].age),
        ([0.0; 3], 0.0),
        "the spawn frame"
    );
    emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, false);
    assert_eq!(
        particles[0].position[0], dt,
        "it moved by the velocity it had"
    );
    assert_eq!(
        particles[0].velocity[0],
        1.0 - 2.0 * dt,
        "then friction took some off"
    );
}

/// With `BLAM_TEST_H3EK`: a colliding emitter's particle that stops on the
/// ground moves to the card, with a thousandth of a unit a second left and
/// no physics; its first frame there moves nothing.
#[test]
fn a_particle_at_rest_moves_to_the_card_and_drifts() {
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let emitter = kit_emitter(
        &root,
        "fx\\cinematics\\100ld_confront_truth\\07\\shot_04\\tentacle_erupt",
    );
    assert_eq!(
        (emitter.update, emitter.card_at_rest),
        (ParticleUpdate::Cpu, true)
    );
    let particle = Particle {
        id: 1,
        colliding: false,
        landed: true,
        resting: false,
        spawned_this_frame: false,
        on_card: false,
        axis: [0.0, 0.0, 1.0],
        age: 0.0,
        inverse_lifespan: 0.1,
        position: [0.0; 3],
        velocity: [0.1, 0.0, 0.0],
        rotation: 0.0,
        rotation_rate: 0.5,
        frame: 0.0,
        frame_rate: 0.0,
        seeds: [0.0; SEEDS],
        emission_time: 0.0,
        flip: [false; 3],
        sequence: 0,
        system_seeds: [0.0; SYSTEM_SEEDS],
    };
    let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 1.0 / 60.0);
    let state = ParticleState {
        system_age: 0.5,
        effect_a: 1.0,
        effect_b: 1.0,
        lod: 1.0,
        ..Default::default()
    };
    emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
    let rested = particles[0].clone();
    assert!(rested.on_card && rested.rotation_rate == 0.0, "{rested:?}");
    assert!((dot(rested.velocity, rested.velocity).sqrt() - 0.001).abs() < 1e-6);
    emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
    assert_eq!(particles.len(), 1, "on the card it no longer dies at rest");
    assert_eq!(
        particles[0].position, rested.position,
        "its first frame there"
    );
    emitter.step(state, &mut particles, dt, [0.0; 3], &mut ended, None, true);
    let moved: [f32; 3] =
        std::array::from_fn(|axis| rested.position[axis] + rested.velocity[axis] * dt);
    assert_eq!(particles[0].position, moved, "it drifts by what it kept");
}

/// Halo 3 and ODST's state list writes `particle rotation` as 0 and
/// never writes `distance from emitter`; Halo 2 reads the particle's own
/// rotation.
#[test]
fn only_halo_2_reads_a_particles_rotation() {
    let resolved = |mut seeds: SeedAllocator, name: &str| {
        let mut input = ParticleInput::named(name);
        seeds.allocate(&mut input);
        input.of(&ParticleState {
            rotation: 0.25,
            ..Default::default()
        })
    };
    assert_eq!(
        resolved(SeedAllocator::default(), "particle rotation"),
        0.25,
        "Halo 2"
    );
    assert_eq!(
        resolved(
            SeedAllocator::halo3(),
            "particle rotation{physics rotation}"
        ),
        0.0
    );
    assert_eq!(
        resolved(SeedAllocator::halo3(), "distance from emitter"),
        0.0
    );
}

/// A property whose function is not constant, reading `input` (and
/// `range`, ranged or not).
fn varying(input: ParticleInput, range: ParticleInput, ranged: bool) -> ParticleProperty {
    use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
    let mut function = H2Function::new(FunctionType::Identity);
    function.set_clamp_range(0.0, 1.0).unwrap();
    function.set_ranged(ranged);
    let function = TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap();
    assert!(function.as_constant().is_none() && function.is_ranged() == ranged);
    ParticleProperty {
        input,
        range,
        function: Arc::new(function),
        ..ParticleProperty::fixed(0.0).unwrap()
    }
}

fn seeded(seeds: &mut SeedAllocator, mut input: ParticleInput) -> ParticleInput {
    seeds.allocate(&mut input);
    input
}

/// `particle_state_list.hlsl`'s state 20 is a tenth of the speed, read
/// by name only in Reach's schema; 17 to 19 have no writer.
#[test]
fn reach_reads_a_tenth_of_the_speed_as_particle_velocity() {
    assert_eq!(
        ParticleInput::named("particle velocity"),
        ParticleInput::Velocity
    );
    for unwritten in [
        "distance from emitter",
        "particle simulation a",
        "particle simulation b",
    ] {
        assert_eq!(
            ParticleInput::named(unwritten),
            ParticleInput::Other,
            "{unwritten}"
        );
    }
    let particle = Particle {
        velocity: [3.0, 0.0, 4.0],
        ..test_particle()
    };
    let state = particle.state(ParticleState::default());
    assert_eq!(ParticleInput::Velocity.of(&state), 0.5);
    assert_eq!(
        ParticleInput::Velocity.of(&ParticleState::default()),
        0.0,
        "nothing at a spawn"
    );
}

/// `g_render_time+0x18`: seconds summed, not Halo 3's fraction of one,
/// coming round at MCC's signed limit.
#[test]
fn reach_game_time_runs_in_seconds_and_comes_round_at_two_hours() {
    assert_eq!(particle_game_time(false, 2.5), 0.5);
    assert_eq!(particle_game_time(true, 2.5), 2.5);
    assert_eq!(particle_game_time(true, 7199.0), 7199.0);
    assert_eq!(particle_game_time(true, 7201.0), -7199.0);
}

/// Tool's `allocate_random_seed`: seed 8 first and down, and when all
/// eight are taken, the reuse table from its fifth entry on.
#[test]
fn reach_hands_out_the_highest_free_seed_then_reuses_from_the_fifth() {
    use ParticleInput::{Random, Seed};
    let mut seeds = SeedAllocator::reach(0);
    let taken: Vec<_> = (0..10).map(|_| seeded(&mut seeds, Random)).collect();
    let expected = [
        Seed(7),
        Seed(6),
        Seed(5),
        Seed(4),
        Seed(3),
        Seed(2),
        Seed(1),
        Seed(0),
        Seed(6),
        Seed(1),
    ];
    assert_eq!(
        taken, expected,
        "states 25 to 22, 7 to 4, then the table's 24 and 5"
    );
    let mut halo3 = SeedAllocator::halo3();
    assert_eq!(seeded(&mut halo3, Random), Seed(0), "Halo 3 counts up");
}

/// What a property reads by name is taken already, so a random goes
/// around it rather than reading the same stream.
#[test]
fn reach_steps_around_a_seed_read_by_name() {
    use ParticleInput::{Random, Seed, SystemRandom, SystemSeed};
    let explicit = ParticleInput::named("particle correlation 4").state_bit();
    let mut seeds = SeedAllocator::reach(explicit);
    let taken: Vec<_> = (0..8).map(|_| seeded(&mut seeds, Random)).collect();
    assert_eq!(
        taken[3..],
        [Seed(4), Seed(2), Seed(1), Seed(0), Seed(6)],
        "seed 4 skipped"
    );
    let mut seeds = SeedAllocator::reach(0);
    let systems: Vec<_> = (0..5).map(|_| seeded(&mut seeds, SystemRandom)).collect();
    let expected = [
        SystemSeed(3),
        SystemSeed(2),
        SystemSeed(1),
        SystemSeed(0),
        SystemSeed(3),
    ];
    assert_eq!(
        systems, expected,
        "27, 26, 9, 8, then the four-long table's fifth, 27"
    );
    assert_eq!(
        seeded(&mut seeds, ParticleInput::Rotation),
        ParticleInput::Other,
        "Reach writes 0 there"
    );
}

/// Reach's `used_states` and allocation read a range only when the
/// function is ranged; Halo 3's allocate it whatever.
#[test]
fn reach_seeds_a_range_only_when_the_function_is_ranged() {
    use ParticleInput::{Random, Seed};
    let flat = varying(Random, Random, false);
    assert_eq!(flat.used_states(), Random.state_bit());
    let mut reach = flat.clone();
    reach.allocate(&mut SeedAllocator::reach(0));
    assert_eq!((reach.input, reach.range), (Seed(7), Random));
    let mut halo3 = flat;
    halo3.allocate(&mut SeedAllocator::halo3());
    assert_eq!((halo3.input, halo3.range), (Seed(0), Seed(1)));
    let mut ranged = varying(Random, Random, true);
    ranged.allocate(&mut SeedAllocator::reach(0));
    assert_eq!((ranged.input, ranged.range), (Seed(7), Seed(6)));
}

/// A Reach controller marks each parameter's own states just before its
/// randoms take seeds: here the third parameter reads system seed 2 by
/// name, so its random modifier cannot take it too.
#[test]
fn a_reach_controller_marks_each_parameter_before_seeding_it() {
    use ParticleInput::{SystemRandom, SystemSeed};
    let named = ParticleInput::named("system correlation 2");
    let parameters = [
        varying(SystemRandom, SystemRandom, false),
        varying(SystemRandom, SystemRandom, false),
        ParticleProperty {
            adds: Some(true),
            modifier: SystemRandom,
            ..varying(named, SystemRandom, false)
        },
    ];
    let run = |mut seeds: SeedAllocator| {
        let mut parameters = parameters.clone();
        parameters
            .iter_mut()
            .for_each(|parameter| parameter.allocate(&mut seeds));
        (parameters[2].modifier, seeds.used())
    };
    let (modifier, used) = run(SeedAllocator::reach_controller());
    assert_eq!(
        modifier,
        SystemSeed(0),
        "state 8, since 9 is the parameter's own"
    );
    let seeds = [27, 26, 9, 8]
        .iter()
        .fold(0, |used, state| used | 1 << state);
    assert_eq!(
        used & REACH_ALLOCATED_STATES,
        seeds,
        "and `system random` itself, which the emitter drops"
    );
    assert_eq!(
        run(SeedAllocator::reach(0)).0,
        SystemSeed(1),
        "unmarked, it takes 9"
    );
}

/// Reach seeds an emitter's properties in tag order, the axis angle
/// fifth; Halo 3 and ODST seed the axis angle last.
#[test]
fn reach_seeds_the_emission_axis_angle_fifth_and_halo_3_last() {
    let seeded = |reach: bool| {
        let random = || varying(ParticleInput::Random, ParticleInput::Other, false);
        let mut properties: [_; 18] = std::array::from_fn(|_| random());
        let mut seeds = if reach {
            SeedAllocator::reach(0)
        } else {
            SeedAllocator::halo3()
        };
        for at in emitter_seed_order(reach) {
            properties[at].allocate(&mut seeds);
        }
        properties.map(|property| property.input)
    };
    assert_eq!(EMITTER_PROPERTIES[4], "emission axis angle");
    assert_eq!(
        seeded(true)[4],
        ParticleInput::Seed(3),
        "the fifth: state 7"
    );
    assert_eq!(
        seeded(false)[4],
        ParticleInput::Seed(17 % SEEDS),
        "the eighteenth, counted"
    );
    assert_eq!(seeded(false)[5], ParticleInput::Seed(4));
    assert_eq!(emitter_seed_order(false)[17], 4);
}

/// Reach's postprocess derives no near pair and keeps a feather of
/// nothing, so it does not fade in at all.
#[test]
fn a_reach_lod_keeps_its_feathers_and_takes_no_near_pair() {
    assert_eq!(reach_baked_lod([0.0; 4]), [0.0, 20.0, 0.0, 5.0]);
    assert_eq!(reach_baked_lod([1.0, 8.0, 2.0, 3.0]), [1.0, 8.0, 2.0, 3.0]);
    let reach = Some(reach_baked_lod([0.0; 4]));
    assert_eq!(lod_amount(reach, 0.5), 1.0, "no fade in");
    let halo3 = Some(baked_lod([0.0; 4], true, [150.0].into_iter()));
    assert!(
        lod_amount(halo3, 0.5) < 1.0,
        "Halo 3's busy any-camera system fades in"
    );
}

#[test]
fn a_reach_near_fade_of_nothing_fades_over_a_hair() {
    assert_eq!(near_fade_range(true, 0.0), 1.0 / POSTPROCESS_EPSILON);
    assert_eq!(near_fade_range(true, 2.0), 0.5);
    assert!(
        near_fade_range(false, 0.0).is_infinite(),
        "Halo 3's as it was"
    );
}

/// Reach's four new shapes, and the two it emits nothing from here.
#[test]
fn reachs_volumes_fill_their_boxes_and_a_plane_heads_along_x() {
    assert_eq!(EmissionShape::named("cube"), Some(EmissionShape::Cube));
    assert_eq!(
        EmissionShape::named("unweighted line"),
        Some(EmissionShape::UnweightedLine)
    );
    assert_eq!(EmissionShape::named("boat hull surface"), None);
    assert_eq!(EmissionShape::named("jetwash"), None);
    let mut random = Random(11);
    for _ in 0..50 {
        let (place, heading) = emit(EmissionShape::Cube, 2.0, 0.0, false, &mut random);
        assert!(
            place.iter().all(|axis| axis.abs() <= 2.0) && heading == place,
            "out from the centre"
        );
        let (place, heading) = emit(EmissionShape::Cylinder, 3.0, 0.0, false, &mut random);
        assert!(
            place[0].abs() <= 1.0,
            "two units long whatever the radius: {place:?}"
        );
        assert!((place[1] * place[1] + place[2] * place[2]).sqrt() <= 3.0 && heading == place);
        let (place, heading) = emit(EmissionShape::Plane, 2.0, 0.0, false, &mut random);
        assert!(place.iter().all(|axis| axis.abs() <= 2.0) && heading == [1.0, 0.0, 0.0]);
        let (place, _) = emit(EmissionShape::UnweightedLine, 2.0, 0.0, false, &mut random);
        assert!(place[0] == 0.0 && place[2] == 0.0 && place[1].abs() <= 2.0);
    }
    let (place, heading) = emit(EmissionShape::Cube, 2.0, 0.0, true, &mut random);
    assert_ne!(heading, place, "volume velocities are random");
}

/// An emitter with nothing but `shape`, for what reads its fields alone.
fn h4_terms(live_forever: bool, disable_velocity: bool, size_scale: f32) -> H4EmitterTerms {
    H4EmitterTerms {
        live_forever,
        disable_velocity,
        size_scale,
        scale: None,
        white_point: None,
    }
}

/// `liveForever` brings the age round, and from past 0.99 back to 0;
/// without it the age runs on to death.
#[test]
fn a_particle_that_lives_forever_comes_round() {
    let forever = h4_terms(true, false, 1.0);
    assert!((forever.age(1.25) - 0.25).abs() < 1e-6);
    assert_eq!(forever.age(0.995), 0.0);
    assert_eq!(forever.age(0.5), 0.5);
    assert_eq!(h4_terms(false, false, 1.0).age(1.25), 1.25);
}

/// `disableVelocity` holds it in place; otherwise the size scale
/// multiplies the step.
#[test]
fn velocity_moves_by_the_size_scale_unless_disabled() {
    assert_eq!(h4_terms(false, true, 2.0).travel(0.5), 0.0);
    assert_eq!(h4_terms(false, false, 2.0).travel(0.5), 1.0);
}

/// Scale y and the size scale go into the size, scale x over scale y
/// into the aspect; with neither the size scale alone.
#[test]
fn scale_x_and_y_bake_into_size_and_aspect() {
    let base = ParticleState::default();
    let none = h4_terms(false, false, 3.0);
    assert_eq!(none.size_and_aspect(&base), [3.0, 1.0]);
    let constant = |value: f32| ParticleProperty::fixed(value).unwrap();
    let scaled = H4EmitterTerms {
        scale: Some([constant(2.0), constant(0.5)]),
        ..h4_terms(false, false, 1.0)
    };
    assert_eq!(scaled.size_and_aspect(&base), [0.5, 4.0]);
}

fn test_emitter(shape: EmissionShape) -> Emitter {
    let fixed = || ParticleProperty::fixed(0.0).unwrap();
    Emitter {
        h4: None,
        update: ParticleUpdate::Gpu,
        card_lerps: false,
        card_at_rest: false,
        spawn_slack: 0.0,
        controllers: Vec::new(),
        shape,
        points: Vec::new(),
        scroll: [0.0; 2],
        axis: ParticleAxis::Constant,
        reference_axis: 0,
        axis_angle: fixed(),
        offset: fixed(),
        direction: fixed(),
        radius: fixed(),
        angle: fixed(),
        starting: fixed(),
        most: fixed(),
        rate: fixed(),
        lifespan: fixed(),
        acceleration: ParticleProperty::fixed_vector([0.0; 3]).unwrap(),
        speed: fixed(),
        rotation: fixed(),
        rotation_rate: fixed(),
        size: fixed(),
        scale: fixed(),
        tint: fixed(),
        alpha: fixed(),
        black_point: fixed(),
        physics: None,
        flock: None,
        collider: None,
        one_at_a_time: false,
        persistent: false,
        bunched: false,
        rotation_given: false,
        volume_random: false,
        axis_scale: [1.0; 3],
        turbulence: None,
    }
}

fn test_particle() -> Particle {
    Particle {
        id: 1,
        colliding: false,
        landed: false,
        resting: false,
        spawned_this_frame: false,
        on_card: false,
        axis: [0.0, 0.0, 1.0],
        age: 0.0,
        inverse_lifespan: 0.1,
        position: [0.0; 3],
        velocity: [0.0; 3],
        rotation: 0.0,
        rotation_rate: 0.0,
        frame: 0.0,
        frame_rate: 0.0,
        seeds: [0.0; SEEDS],
        emission_time: 0.0,
        flip: [false; 3],
        sequence: 0,
        system_seeds: [0.0; SYSTEM_SEEDS],
    }
}

/// `axis scale` scales the place after the shape, and all nothing is ones.
#[test]
fn reachs_axis_scale_flattens_what_its_shape_emits() {
    assert_eq!(reach_axis_scale([0.0; 3]), [1.0; 3]);
    assert_eq!(reach_axis_scale([0.0, 2.0, 2.0]), [0.0, 2.0, 2.0]);
    let emitter = Emitter {
        axis_scale: [2.0, 1.0, 0.0],
        ..test_emitter(EmissionShape::Plane)
    };
    let (place, _) = emitter.emitted(1.0, 0.0, &mut Random(5));
    let (unscaled, _) = emit(EmissionShape::Plane, 1.0, 0.0, false, &mut Random(5));
    assert_eq!(place, [unscaled[0] * 2.0, unscaled[1], 0.0]);
}

/// A 2 by 1 texture: red on the left, green on the right, half blue.
fn turbulence_image() -> TextureImage {
    TextureImage {
        rgba: vec![255, 0, 128, 255, 0, 255, 128, 255],
        width: 2,
        height: 1,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0; 2],
    }
}

/// The texel at `(birth, seed 5) · scale + offset`, wrapped and blended,
/// less a half and times the strength over the frame.
#[test]
fn turbulence_pushes_by_a_wrapped_blended_texel_less_a_half() {
    let image = turbulence_image();
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-3);
    assert!(close(
        sample_wrapped(&image, [0.25, 0.5]),
        [1.0, 0.0, 0.502]
    ));
    assert!(
        close(sample_wrapped(&image, [1.25, 0.5]), [1.0, 0.0, 0.502]),
        "wrapped"
    );
    assert!(
        close(sample_wrapped(&image, [0.5, 0.5]), [0.5, 0.5, 0.502]),
        "blended"
    );
    assert!(
        close(sample_wrapped(&image, [0.0, 0.5]), [0.5, 0.5, 0.502]),
        "blended across the wrap"
    );
    // u = birth · 1 + 0.25, the left texel.
    let push = turbulence_push(&image, [1.0, 0.3, 0.25, 0.0], [0.0, 0.7], 2.0);
    assert!(close(push, [1.0, -1.0, 0.004]), "{push:?}");
}

/// On the card, turbulence moves a particle after its velocity does and
/// before it accelerates, and not on its spawn frame.
#[test]
fn a_turbulent_particle_on_the_card_is_pushed_after_it_moves() {
    let turbulence = ParticleTurbulence {
        strength: ParticleProperty::fixed(2.0).unwrap(),
        xform: [1.0, 0.3, 0.25, 0.0].map(|value| ParticleProperty::fixed(value).unwrap()),
        image: turbulence_image(),
    };
    let emitter = Emitter {
        turbulence: Some(turbulence),
        ..test_emitter(EmissionShape::Sprayer)
    };
    let particle = Particle {
        spawned_this_frame: true,
        velocity: [1.0, 0.0, 0.0],
        ..test_particle()
    };
    let (mut particles, mut ended, dt) = (vec![particle], Vec::new(), 0.5);
    emitter.step(
        ParticleState::default(),
        &mut particles,
        dt,
        [0.0; 3],
        &mut ended,
        None,
        false,
    );
    assert_eq!(particles[0].position, [0.0; 3], "the spawn frame");
    emitter.step(
        ParticleState::default(),
        &mut particles,
        dt,
        [0.0; 3],
        &mut ended,
        None,
        false,
    );
    let [x, y, z] = particles[0].position;
    // Moved a half along x, then pushed by the left texel less a half,
    // (0.5, -0.5, 0.002), at a strength of 2 over half a second.
    let near =
        (x - (0.5 + 0.5)).abs() < 1e-4 && (y + 0.5).abs() < 1e-4 && (z - 0.00196).abs() < 1e-4;
    assert!(near, "{:?}", particles[0].position);
    let calm = test_emitter(EmissionShape::Sprayer);
    let mut particles = vec![Particle {
        velocity: [1.0, 0.0, 0.0],
        ..test_particle()
    }];
    calm.step(
        ParticleState::default(),
        &mut particles,
        dt,
        [0.0; 3],
        &mut ended,
        None,
        false,
    );
    assert_eq!(
        particles[0].position,
        [0.5, 0.0, 0.0],
        "none without the movement"
    );
}

/// Halo 4's `muzzle_smoke` pushes from (-0.1, 0.1, 0) to the opposite
/// way. On Reach's and Halo 4's card that is a lerp, through nothing, so
/// a particle keeps a sound velocity; Halo 3's turn has no way round and
/// is not asked to find one here.
#[test]
fn reach_and_halo_4_lerp_opposite_accelerations_on_the_card() {
    let ends = [[-0.1, 0.1, 0.0], [0.1, -0.1, 0.0]];
    let acceleration = ParticleProperty {
        ends,
        ..ParticleProperty::fixed(0.25).unwrap()
    };
    assert!(
        acceleration
            .around(&ParticleState::default())
            .iter()
            .any(|axis| axis.is_nan()),
        "the turn fails"
    );
    let emitter = Emitter {
        card_lerps: true,
        acceleration,
        ..test_emitter(EmissionShape::Sprayer)
    };
    let mut particles = vec![Particle {
        velocity: [1.0, 0.0, 0.0],
        ..test_particle()
    }];
    let mut ended = Vec::new();
    emitter.step(
        ParticleState::default(),
        &mut particles,
        1.0,
        [0.0; 3],
        &mut ended,
        None,
        false,
    );
    let [x, y, z] = particles[0].velocity;
    let near = (x - 0.95).abs() < 1e-6 && (y - 0.05).abs() < 1e-6 && z == 0.0;
    assert!(
        near,
        "a quarter of the way from one end to the other: {:?}",
        particles[0].velocity
    );
}

/// Reach's additive blends take `v_alt_exposure.y`, the rest
/// `v_exposure.x`, and multiply none; Halo 3's keep its own.
#[test]
fn reach_exposes_a_particle_by_its_blend() {
    assert_eq!(
        particle_exposure(Gen3Blend::Additive, true, true, 0.0),
        reach_alt_exposure(0.0)[1]
    );
    assert_eq!(
        particle_exposure(Gen3Blend::AddSrcTimesSrcAlpha, true, true, 0.0),
        reach_alt_exposure(0.0)[1]
    );
    assert_eq!(
        particle_exposure(Gen3Blend::AlphaBlend, true, true, 0.0),
        reach_exposure(0.0)[0]
    );
    assert_eq!(particle_exposure(Gen3Blend::Multiply, true, true, 0.0), 1.0);
    assert_eq!(
        particle_exposure(Gen3Blend::Additive, true, false, 0.0),
        H3_EXPOSURE
    );
    assert_eq!(
        particle_exposure(Gen3Blend::Additive, false, false, 0.0),
        1.0,
        "Halo 2's show as they are"
    );
}

/// The area screen effects' stops reach Reach's exposure and its
/// constant self-illumination, and not Halo 3's.
#[test]
fn the_stops_move_reachs_particle_exposure_alone() {
    assert_eq!(
        particle_exposure(Gen3Blend::Additive, true, true, 1.0),
        reach_alt_exposure(1.0)[1]
    );
    assert_ne!(reach_alt_exposure(1.0)[1], reach_alt_exposure(0.0)[1]);
    assert_eq!(
        particle_exposure(Gen3Blend::AlphaBlend, true, true, -1.0),
        reach_exposure(-1.0)[0]
    );
    assert_eq!(
        particle_exposure(Gen3Blend::Additive, true, false, 1.0),
        H3_EXPOSURE
    );
    let mut shader = SpriteShader::blank();
    shader.add = [1.0; 3];
    assert_eq!(shader.add(1.0), [1.0; 3], "Halo 3's add as authored");
    shader.add_at_illum = true;
    assert_eq!(shader.add(0.0), [reach_alt_exposure(0.0)[1]; 3]);
    assert_eq!(shader.add(1.0), [reach_alt_exposure(1.0)[1]; 3]);
}

#[test]
fn the_cheap_shader_faces_the_screen_or_camera_or_lies_flat() {
    assert_eq!(
        cheap_billboard(Billboard::ScreenFacing),
        Billboard::ScreenFacing
    );
    assert_eq!(
        cheap_billboard(Billboard::CameraFacing),
        Billboard::CameraFacing
    );
    assert_eq!(cheap_billboard(Billboard::ScreenParallel), Billboard::World);
    assert_eq!(cheap_billboard(Billboard::LocalVertical), Billboard::World);
}

/// Halo 4 births a vertically authored sprite a three-quarter turn round,
/// so `parallel to direction` flames lie along their velocity, not across
/// the view; a random start wins, and nothing else turns.
#[test]
fn halo_4_vertical_sprites_start_three_quarters_round() {
    assert_eq!(starting_rotation(false, true, || 0.3), 0.75);
    assert_eq!(starting_rotation(true, true, || 0.3), 0.3);
    assert_eq!(starting_rotation(false, false, || 0.3), 0.0);
}

/// `DOUBLE_SPRITE_TEXCOORD`: the next frame when frames blend, else a
/// Reach plasma's sixteenth of seed 1 on.
#[test]
fn a_reach_plasma_reads_its_second_noise_a_seeded_number_of_frames_on() {
    assert_eq!(
        second_frame(2.5, false, true, 0.2),
        5.0,
        "floor(2.5) + floor(3.2)"
    );
    assert_eq!(
        second_frame(2.5, true, true, 0.2),
        3.5,
        "blending, the next"
    );
    assert_eq!(
        second_frame(2.5, false, false, 0.2),
        3.5,
        "not Reach's plasma"
    );
}

/// With `BLAM_TEST_HREK`: Reach's particle options read by name, and its
/// seeds handed out from the top.
#[test]
fn kit_reach_particles_read_their_own_options_and_seeds() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let plasma = read_look(
        &crate::app::model_preview::load::TagReader::new(&source),
        "fx\\particles\\plasma\\plasma_blue_medium",
    )
    .unwrap();
    assert!(plasma.reach && !plasma.cheap);
    assert_eq!(plasma.shader.albedo, ParticleAlbedo::Palette2dPlasma);
    assert!(plasma.shader.plasma_second_sprite);
    assert_eq!(plasma.shader.sphere_warp, Some(0.2));
    assert_eq!(plasma.shader.depth_fade, Some(0.25));
    assert_ne!(
        plasma.used_states & 1 << 25,
        0,
        "its first random took seed 8: {:#x}",
        plasma.used_states
    );
    let paper = read_look(
        &crate::app::model_preview::load::TagReader::new(&source),
        "fx\\particles\\models\\debris\\blowing_paper\\blowing_paper",
    )
    .unwrap();
    assert!(paper.shader.smooth_contrast().is_some());
    let shards = read_look(
        &crate::app::model_preview::load::TagReader::new(&source),
        "fx\\particles\\models\\weapons\\needler_shards\\needler_shards",
    )
    .unwrap();
    assert!(
        shards.cheap
            && shards.shader.sphere_warp.is_none()
            && shards.shader.smooth_contrast().is_none()
    );
}

/// With `BLAM_TEST_HREK`: `fly_swarm` emits from a plane flattened by its
/// axis scale, with turbulence, on the card.
#[test]
fn kit_reach_swarm_emits_from_a_flattened_plane_with_turbulence() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let path = "fx\\fx_library\\ambient\\nature\\bug_swarm\\fly_swarm";
    let effect = load_referenced_tag_from_source(&source, path, "effect", b"effe").unwrap();
    let systems = elements(effect.root(), "events")
        .into_iter()
        .flat_map(|event| elements(event, "particle systems"));
    let parts = systems.filter_map(|system| {
        read_system_parts(
            &crate::app::model_preview::load::TagReader::new(&source),
            system,
        )
    });
    let emitters: Vec<_> = parts.flat_map(|parts| parts.emitters).collect();
    let plane = emitters
        .iter()
        .find(|emitter| emitter.shape == EmissionShape::Plane)
        .expect("a plane emitter");
    assert_eq!(plane.axis_scale, [0.0, 2.0, 2.0]);
    assert_eq!(plane.update, ParticleUpdate::Gpu);
    let turbulence = plane.turbulence.as_ref().expect("its turbulence");
    assert!(turbulence.image.width > 0 && turbulence.image.whole());
}
