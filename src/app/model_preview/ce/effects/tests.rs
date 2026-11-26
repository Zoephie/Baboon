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

/// With `BLAM_TEST_HCEEK`: the Scorpion's cannon has no model, so it stays
/// hidden in the tank and `weapon_effect_new` plays its firing on the tank,
/// object 0, though the triggers are the cannon's, object 1:
/// both triggers fire, and the shell's effect goes at the tank's own
/// markers, `primary trigger` among them, where the barrel is. Its muzzle
/// flash still lights: `light_reconnect_to_map` puts it at the gun's origin,
/// on the tank where the gun is held.
#[test]
fn a_hidden_vehicle_gun_fires_at_the_vehicles_markers() {
    use crate::app::model_preview::harness::kit::{kit_preview, kit_root};
    use crate::core::game::GameId;
    let Some(root) = kit_root(GameId::HaloCe) else {
        return;
    };
    let data = kit_preview(
        &root,
        GameId::HaloCe,
        "vehicles\\scorpion\\scorpion",
        "vehicle",
        b"vehi",
    );
    let firing = &data.preview.ce.firing;
    assert_eq!(firing.len(), 1, "the cannon's triggers are read");
    assert_eq!(firing[0].object, 1);
    assert_eq!(firing[0].triggers.len(), 2);
    let fire =
        data.preview
            .ce
            .effects
            .iter()
            .find(|effect| {
                effect.object == 0
                    && effect.event.as_ref().is_some_and(|event| {
                        event.path == "vehicles\\scorpion\\scorpion cannon fire"
                    })
            })
            .expect("the cannon's firing effect is listed");
    let markers = &data.preview.markers;
    let at = |position: &[f32; 3], name: Option<&str>| {
        markers.iter().any(|marker| {
            marker.position == *position && name.is_none_or(|name| marker.name == name)
        })
    };
    let placed: Vec<[f32; 3]> = fire
        .locations
        .iter()
        .flatten()
        .map(|location| location.position)
        .collect();
    assert!(
        placed.iter().all(|position| at(position, None)),
        "{placed:?} are not all the tank's markers"
    );
    assert!(
        placed
            .iter()
            .any(|position| at(position, Some("primary trigger"))),
        "{placed:?} misses the barrel"
    );
    assert!(
        data.preview.ce.lights.iter().any(|light| light.object == 1),
        "the cannon's muzzle flash is read"
    );
    // And a shot lights it, as the held rifle's does.
    use super::super::firing::{CeShots, fired_input};
    let functions = data.ce.attached[0]
        .functions
        .as_ref()
        .expect("the cannon's functions");
    let flash = functions
        .output_named("muzzle flash")
        .expect("the cannon's muzzle flash function");
    let mut shots = CeShots::new();
    let lit = |shots: &CeShots, time: f32| {
        let fired = fired_input(firing, shots, data.geometry_id, 1, time);
        let inputs = functions.with_fired(
            functions.inputs_for(&Default::default(), (data.geometry_id, 1)),
            fired,
        );
        functions.outputs(&inputs, time)[flash]
    };
    assert_eq!(lit(&shots, 10.0), 0.0, "unfired, the muzzle is dark");
    shots
        .entry((data.geometry_id, 1))
        .or_default()
        .fire(&firing[0], 0, 1.0, 10.0)
        .expect("the main trigger has a firing effect");
    assert!(lit(&shots, 10.01) > 0.0, "the shot lights the muzzle");
}
