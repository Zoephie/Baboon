use super::*;

/// `effect_update`'s byte: truncated, then its low eight bits.
#[test]
fn an_event_particle_count_wraps_as_the_byte_it_is_kept_in() {
    assert_eq!(event_particle_count(12.9), 12.0);
    assert_eq!(event_particle_count(255.5), 255.0);
    assert_eq!(
        event_particle_count(256.0),
        0.0,
        "past a byte it comes round"
    );
    assert_eq!(event_particle_count(300.0), 44.0);
}

/// `effects_update` runs per 30 Hz tick however the frames fall: a 120
/// fps frame owes a quarter of one, and the tick lands on the fourth.
#[test]
fn effects_tick_at_30_whatever_the_frame_rate() {
    let inputs = |_| CeObjectInputs {
        outputs: [0.0; 4],
        active: [false; 4],
        change_colors: [[0.0; 3]; 4],
    };
    let mut field = CeParticleField::default();
    field.advance(&[], &CeEventPlays::default(), inputs, None, 0.0, 10.0, 0);
    let settled = field.tick;
    let mut ticks = Vec::new();
    for frame in 1..=8 {
        field.advance(
            &[],
            &CeEventPlays::default(),
            inputs,
            None,
            0.0,
            10.0 + frame as f32 / 120.0,
            0,
        );
        ticks.push(field.tick - settled);
    }
    assert_eq!(ticks, [0, 0, 0, 1, 1, 1, 1, 2]);
}

#[test]
fn a_sprite_turns_as_build_sprite_compute_basis_turns_it() {
    let view = ViewFrame {
        eye: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        right: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, 1.0],
    };
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
    use Distribution::*;
    assert_eq!(integral(Start, 0.0), 1.0);
    assert_eq!(integral(End, 0.5), 0.0);
    assert_eq!(integral(End, 1.0), 1.0);
    assert_eq!(integral(Constant, 0.25), 0.25);
    assert_eq!(integral(Buildup, 0.5), 0.25);
    assert_eq!(integral(Falloff, 0.5), 0.75);
    assert_eq!(integral(BuildupAndFalloff, 0.5), 0.5);
    assert_eq!(integral(Constant, -1.0), 0.0);
}

#[test]
fn a_point_on_the_floor_bounces_by_its_elasticity() {
    let physics = PointPhysics {
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
