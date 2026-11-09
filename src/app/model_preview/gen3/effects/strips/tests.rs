use super::super::super::super::gen3::pipeline::{stops_alt_exposure, stops_exposure};
use super::super::super::super::{TagSource, load_referenced_tag_from_source};
use super::super::super::pipeline::EXPOSURE as FIXED_EXPOSURE;
use super::super::super::shaders::Gen3Blend;
use super::contrail::RunningContrail;
use super::geometry::*;
use super::*;
use crate::app::model_preview::harness::kit::{kit_root, kit_source};
use crate::core::game::GameId;
use std::sync::Arc;

#[test]
fn a_beam_always_has_two_profiles_and_never_more_than_its_rows() {
    assert_eq!(beam_profiles(0.1, 0.1), 2);
    assert_eq!(beam_profiles(3.0, 2.2), 7);
    assert_eq!(beam_profiles(100.0, 100.0), MOST_PROFILES);
}

#[test]
fn a_ribbon_runs_across_its_first_axis_and_a_cross_adds_the_second() {
    assert_eq!(StripShape::Ribbon.strips(), 1);
    assert_eq!(StripShape::Cross.across(0, 1), [0.5, 0.0]);
    assert_eq!(StripShape::Cross.across(1, 0), [0.0, -0.5]);
    assert_eq!(StripShape::Ngon(6).strips(), 6);
}

#[test]
fn straight_up_has_a_basis_of_its_own() {
    assert_eq!(
        world_basis([0.0, 0.0, 1.0]),
        [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]
    );
    assert_eq!(world_basis([1.0, 0.0, 0.0])[0], [0.0, -1.0, 0.0]);
}

fn constant(value: f32) -> StripProperty {
    StripProperty::fixed(value).unwrap()
}

fn contrail(rate: f32, lifespan: f32) -> ContrailDefinition {
    ContrailDefinition {
        emission_rate: constant(rate),
        lifespan: constant(lifespan),
        acceleration: constant(0.0),
        size: constant(1.0),
        offset: constant(0.0),
        rotation: constant(0.0),
        color: constant(1.0),
        alpha: constant(1.0),
        secondary_alpha: constant(1.0),
        black_point: constant(0.0),
        palette: constant(0.0),
        intensity: constant(1.0),
        tint_from_lightmap: false,
        opacity_from_a: false,
        random_u: false,
        random_v: false,
    }
}

fn look(origin_fade: Option<[f32; 2]>) -> StripLook {
    StripLook {
        shape: StripShape::Ribbon,
        double_sided: true,
        tiling: [1.0; 2],
        scroll: [0.0; 2],
        origin_fade,
        edge_fade: None,
        angle_fade: None,
        sort_bias: 0.0,
        own_terms: false,
        shader: Arc::new(SpriteShader::blank()),
    }
}

#[test]
fn a_property_a_tag_is_too_old_for_stays_at_its_default() {
    let state = StripState {
        profile_age: 0.5,
        length: 3.0,
        ..Default::default()
    };
    assert_eq!(StripProperty::fixed(1.0).unwrap().scalar(&state), 1.0);
    assert_eq!(StripProperty::fixed(0.0).unwrap().scalar(&state), 0.0);
}

#[test]
fn a_vector_goes_around_and_opposite_ends_have_no_way_to() {
    let state = StripState::default();
    let mut property = constant(0.5);
    property.ends = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let turned = property.vector(&state);
    assert!(
        (dot(turned, turned) - 1.0).abs() < 1e-3,
        "{turned:?} is not across the corner"
    );
    property.ends = [[0.0, 0.0, 0.1], [0.0, 0.0, -0.1]];
    assert!(!property.vector(&state)[2].is_finite());
    property.ends = [[0.0, 0.0, 0.2], [0.0; 3]];
    assert!((property.vector(&state)[2] - 0.2 * std::f32::consts::FRAC_PI_4.sin()).abs() < 1e-5);
}

/// A new system's first pulse lays a permanent profile where it starts,
/// so moving on extends from there instead of replacing it.
#[test]
fn a_new_contrail_is_anchored_where_it_starts() {
    let (contrail, look) = (contrail(20.0, 10.0), look(None));
    let state = StripState::default();
    let mut running = RunningContrail {
        previous: [2.0, 0.0, 0.0],
        ..Default::default()
    };
    running.pulse(&contrail, &look, &state, [2.0, 0.0, 0.0], 0.1);
    assert_eq!(running.profiles.len(), 1);
    assert!(!running.profiles[0].temporary, "the start is permanent");
    assert!(running.initialized);
    running.pulse(&contrail, &look, &state, [2.01, 0.0, 0.0], 0.001);
    assert_eq!(
        running.profiles[0].place,
        [2.0, 0.0, 0.0],
        "the start stays put"
    );
    assert!(
        running.profiles.last().unwrap().temporary,
        "the far end is the tip"
    );
}

#[test]
fn a_contrail_left_where_it_stands_lays_down_nothing_after_its_start() {
    let (contrail, look) = (contrail(30.0, 1.0), look(None));
    let mut running = RunningContrail::default();
    let state = StripState::default();
    running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
    let first = running.profiles.len();
    assert!(first > 0);
    for _ in 0..10 {
        running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
    }
    assert_eq!(running.profiles.len(), first);
}

#[test]
fn a_move_under_the_epsilon_is_no_move() {
    let (contrail, look) = (contrail(100.0, 10.0), look(None));
    let state = StripState::default();
    let mut running = RunningContrail::default();
    running.pulse(&contrail, &look, &state, [0.0; 3], 0.1);
    let started = running.profiles.len();
    running.pulse(&contrail, &look, &state, [0.00005, 0.0, 0.0], 0.1);
    assert_eq!(
        running.profiles.len(),
        started,
        "too little to have a direction"
    );
    running.pulse(&contrail, &look, &state, [0.0002, 0.0, 0.0], 0.1);
    assert!(running.profiles.len() > started, "and this is enough");
}

#[test]
fn moving_lays_profiles_along_the_way_and_the_tip_is_replaced() {
    let (contrail, look) = (contrail(20.0, 10.0), look(None));
    let mut running = RunningContrail {
        initialized: true,
        ..Default::default()
    };
    let state = StripState::default();
    running.pulse(&contrail, &look, &state, [1.0, 0.0, 0.0], 0.1);
    assert_eq!(
        running.profiles.len(),
        2,
        "two laid, none left over for a tip"
    );
    assert!((running.profiles[1].place[0] - 1.0).abs() < 1e-5);
    assert!((running.length - 1.0).abs() < 1e-5);
    running.pulse(&contrail, &look, &state, [1.5, 0.0, 0.0], 0.025);
    assert_eq!(running.profiles.len(), 3);
    assert!(running.profiles[2].temporary);
    running.pulse(&contrail, &look, &state, [1.75, 0.0, 0.0], 0.0125);
    assert_eq!(running.profiles.len(), 3, "the tip moved rather than grew");
    assert!((running.profiles[2].place[0] - 1.75).abs() < 1e-5);
}

#[test]
fn a_fade_gets_profiles_at_both_its_ends() {
    let (contrail, look) = (contrail(0.0, 10.0), look(Some([2.0, 0.25])));
    let mut running = RunningContrail {
        initialized: true,
        ..Default::default()
    };
    running.pulse(
        &contrail,
        &look,
        &StripState::default(),
        [1.0, 0.0, 0.0],
        0.1,
    );
    let lengths: Vec<f32> = running
        .profiles
        .iter()
        .map(|profile| profile.length)
        .collect();
    assert_eq!(lengths, [0.25, 0.75, 1.0]);
}

fn carried_strip() -> Arc<Gen3StripAt> {
    Arc::new(Gen3StripAt {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        world_oriented: false,
        carried: true,
        end: None,
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        seeds: [0.0; OWN_SEEDS],
        times: Arc::new(EffectTimes::read_nothing()),
        event: 0,
        look: Arc::new(look(None)),
        kind: Arc::new(StripKind::Contrail(contrail(60.0, 0.5))),
    })
}

#[test]
fn a_carried_contrail_trails_its_particle_and_fades_after_it_dies() {
    let strip = carried_strip();
    let strips = [Arc::clone(&strip)];
    let mut field = Gen3StripField::default();
    let mut onsets = EffectOnsets::default();
    let view = ViewFrame {
        eye: [0.0, -5.0, 0.0],
        forward: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
    };
    let lighting = [[0.0; 4]; 10];
    let mut advance = |field: &mut Gen3StripField, place: Option<[f32; 3]>, time: f32| {
        let tracks = |_: &Arc<Gen3StripAt>| place.map(|place| (7, place)).into_iter().collect();
        field.advance(&strips, |_| None, &mut onsets, tracks, None, time);
    };
    advance(&mut field, Some([0.0; 3]), 0.0);
    for step in 1..=10 {
        let time = step as f32 / 30.0;
        advance(&mut field, Some([time, 0.0, 0.0]), time);
    }
    let drawn = field.vertices(0, &strip, None, None, &view, &lighting, 0.33, 0.0, 0.0);
    assert!(!drawn.is_empty(), "a moving particle leaves a trail");
    let reach = drawn
        .iter()
        .map(|vertex| vertex.place[0])
        .fold(0.0, f32::max);
    assert!(reach > 0.2, "along where it went: {reach}");
    advance(&mut field, None, 0.4);
    assert!(
        !field
            .vertices(0, &strip, None, None, &view, &lighting, 0.4, 0.0, 0.0)
            .is_empty(),
        "it outlives the particle"
    );
    for step in 0..30 {
        advance(&mut field, None, 0.45 + step as f32 * 0.1);
    }
    assert!(
        field
            .vertices(0, &strip, None, None, &view, &lighting, 3.5, 0.0, 0.0)
            .is_empty()
    );
    assert!(field.carried[0].is_empty(), "nothing is left to run");
}

fn reach_look(origin_fade: Option<[f32; 2]>) -> StripLook {
    StripLook {
        own_terms: true,
        ..look(origin_fade)
    }
}

/// Reach's beams fade with a range alone, as `laser_emitter`'s origin
/// does; its contrails and Halo 3's want a cutoff.
#[test]
fn reach_beams_fade_with_a_range_or_a_cutoff_and_contrails_with_a_cutoff() {
    assert_eq!(strip_fade(20.0, 0.0, 1.0, true), Some([0.05, 0.0]));
    assert_eq!(
        strip_fade(20.0, 0.0, 1.0, false),
        None,
        "a contrail's, or Halo 3's"
    );
    assert_eq!(strip_fade(0.0, 0.0, 1.0, true), None, "neither set");
    assert_eq!(
        strip_fade(0.0, 2.0, 1.0, false),
        Some([f32::INFINITY, 2.0]),
        "a cutoff alone is a step"
    );
}

/// A range of 0 makes the fade a step, with a NaN on it that Reach's
/// `saturate` takes to 0 and Halo 3's clamp passes on.
#[test]
fn a_reach_step_fade_is_0_at_its_cutoff() {
    let step = f32::INFINITY * 0.0;
    assert_eq!(reach_look(None).fade(step), 0.0);
    assert!(look(None).fade(step).is_nan(), "Halo 3's is as it was");
    assert_eq!(reach_look(None).fade(f32::INFINITY), 1.0);
    assert_eq!(reach_look(None).fade(0.25), look(None).fade(0.25));
}

/// `effects__beam.hlsl` l.296–305: 0 where the side faces the camera,
/// fading over the range past the begin angle.
#[test]
fn a_reach_beam_fades_its_sides_seen_edge_on() {
    let profile = Profile {
        place: [0.0; 3],
        direction: [1.0, 0.0, 0.0],
        ..Default::default()
    };
    // Far down -y, so every corner is seen along +y.
    let eye = [0.0, -1.0e4, 0.0];
    let (range, cutoff) = (20f32.to_radians(), 45f32.to_radians());
    // Toward the eye from the axis, its side faces it; above, it is edge on.
    let facing = angle_fade([0.0, -0.5, 0.0], &profile, eye, range, cutoff);
    let edge_on = angle_fade([0.0, 0.0, 0.5], &profile, eye, range, cutoff);
    assert_eq!(facing, 1.0);
    assert_eq!(edge_on, 0.0);
    // 55 degrees round from facing: half way through 45 to 65.
    let turned = 55f32.to_radians();
    let between = [0.0, -turned.cos(), turned.sin()].map(|axis| axis * 0.5);
    let half = angle_fade(between, &profile, eye, range, cutoff);
    assert!((half - 0.5).abs() < 1e-3, "{half}");
    assert_eq!(
        angle_fade(between, &profile, eye, 0.0, cutoff),
        0.0,
        "past the begin, a step"
    );
    assert_eq!(
        angle_fade([0.0, -0.5, 0.0], &profile, eye, 0.0, cutoff),
        1.0,
        "before it"
    );
}

/// `particle_exposure`'s rule, at the stops: Halo 3's at its own.
#[test]
fn a_reach_strip_is_exposed_by_its_blend_and_the_stops() {
    let blended = |blend: Gen3Blend, reach: bool| {
        let mut shader = SpriteShader::blank();
        shader.blend = blend;
        StripLook {
            own_terms: reach,
            shader: Arc::new(shader),
            ..look(None)
        }
    };
    let red = |look: &StripLook, stops: f32| exposed(look, [1.0, 0.0, 0.0], 1.0, 1.0, stops)[0];
    let additive = blended(Gen3Blend::Additive, true);
    assert_eq!(red(&additive, 0.0), stops_alt_exposure(0.0)[1]);
    assert_eq!(red(&additive, 1.0), stops_alt_exposure(1.0)[1]);
    assert_ne!(red(&additive, 1.0), red(&additive, 0.0));
    assert_eq!(
        red(&blended(Gen3Blend::AlphaBlend, true), 1.0),
        stops_exposure(1.0)[0]
    );
    assert_eq!(red(&blended(Gen3Blend::Multiply, true), 1.0), 1.0);
    assert_eq!(
        red(&blended(Gen3Blend::Additive, false), 1.0),
        FIXED_EXPOSURE,
        "Halo 3's ignore the stops"
    );
}

/// Reach's beams scroll on a minute's clock and its contrails on 131
/// seconds'; Halo 3's run on.
#[test]
fn reach_strips_scroll_on_a_clock_that_wraps() {
    assert_eq!(REACH_CONTRAIL_SCROLL_PERIOD, 131.0);
    assert_eq!(
        reach_look(None).scroll_time(REACH_BEAM_SCROLL_PERIOD, 61.5),
        1.5
    );
    assert_eq!(
        reach_look(None).scroll_time(REACH_CONTRAIL_SCROLL_PERIOD, 132.0),
        1.0
    );
    assert_eq!(look(None).scroll_time(REACH_BEAM_SCROLL_PERIOD, 61.5), 61.5);
}

/// `c_contrail::pulse` holds Reach's emission rate to 100000.
#[test]
fn reach_holds_a_contrails_emission_rate_down() {
    let contrail = contrail(1.0e6, 10.0);
    let state = StripState::default();
    let pulsed = |look: &StripLook| {
        let mut running = RunningContrail {
            initialized: true,
            ..Default::default()
        };
        running.pulse(&contrail, look, &state, [1.0, 0.0, 0.0], 5.0e-6);
        running.accumulator
    };
    assert!(
        (pulsed(&reach_look(None)) - 0.5).abs() < 1e-4,
        "{}",
        pulsed(&reach_look(None))
    );
    assert!(
        pulsed(&look(None)) < 1.0,
        "Halo 3's laid five, and has what is left"
    );
    let laid = |look: &StripLook| {
        let mut running = RunningContrail {
            initialized: true,
            ..Default::default()
        };
        running.pulse(&contrail, look, &state, [1.0, 0.0, 0.0], 5.0e-6);
        running.profiles.len()
    };
    assert_eq!(laid(&reach_look(None)), 1, "only the tip");
    assert_eq!(laid(&look(None)), 5);
}

fn reach_kit() -> Option<TagSource> {
    let root = kit_root(GameId::HaloReach)?;
    Some(super::super::super::super::particles::loose_kit(
        &root,
        GameId::HaloReach,
    ))
}

fn looks_of(
    source: &TagSource,
    path: &str,
    group: &str,
    code: &[u8; 4],
    blocks: &str,
) -> Vec<StripLook> {
    let tag = load_referenced_tag_from_source(source, path, group, code).expect(path);
    let beam = blocks == "beams";
    elements(tag.root(), blocks)
        .into_iter()
        .map(|element| {
            read_strip_look(
                &crate::app::model_preview::load::TagReader::new(source),
                element,
                beam,
            )
            .expect(path)
        })
        .collect()
}

/// With `BLAM_TEST_HREK`: the focus rifle's beams read Reach's plasma
/// without its shift or second sprite, at a particle alpha of 1, and its
/// pulse's angle fade in radians.
#[test]
fn kit_reach_focus_rifle_beams_take_one_texcoord_and_an_angle_fade() {
    let Some(source) = reach_kit() else {
        return;
    };
    let beams = looks_of(
        &source,
        "objects\\weapons\\rifle\\focus_rifle\\fx\\projectile_3p",
        "beam_system",
        b"beam",
        "beams",
    );
    use super::super::super::super::particles::ParticleAlbedo;
    assert_eq!(beams[1].shader.albedo, ParticleAlbedo::Palette2dPlasma);
    assert_eq!(beams[2].shader.albedo, ParticleAlbedo::PalettePlasma);
    for plasma in &beams[1..3] {
        assert!(
            plasma.own_terms
                && plasma.shader.unit_particle_alpha
                && !plasma.shader.plasma_second_sprite
        );
        assert!(plasma.shader.palette_shift.is_none());
    }
    let [range, begin] = beams[3].angle_fade.expect("the pulse's angle fade");
    assert!((range - 20f32.to_radians()).abs() < 1e-6 && (begin - 45f32.to_radians()).abs() < 1e-6);
    assert!(beams[0].angle_fade.is_none());
    let laser = looks_of(
        &source,
        "levels\\dlc\\cex_beaver_creek\\fx\\laser_emitter",
        "beam_system",
        b"beam",
        "beams",
    );
    assert_eq!(
        laser[0].origin_fade,
        Some([0.05, 0.0]),
        "a range and no cutoff"
    );
    let trail = looks_of(
        &source,
        "cinematics\\050lb_reunited\\fx\\missile_trail",
        "contrail_system",
        b"cntl",
        "contrails",
    );
    assert!(
        (trail[0].sort_bias - 0.15).abs() < 1e-6,
        "3 steps of 0.05: {}",
        trail[0].sort_bias
    );
    assert!(
        !trail[0].shader.unit_particle_alpha,
        "a contrail's alpha is its own"
    );
}

/// With `BLAM_TEST_H3EK`: Halo 3's beams keep their particle alpha, sort
/// bias and fades as they were.
#[test]
fn kit_halo_3_beams_keep_their_own_look() {
    let Some(source) = kit_source(GameId::Halo3) else {
        return;
    };
    let beams = looks_of(
        &source,
        "objects\\equipment\\autoturret\\fx\\projectile",
        "beam_system",
        b"beam",
        "beams",
    );
    assert!(!beams.is_empty());
    for beam in &beams {
        assert!(!beam.own_terms && !beam.shader.unit_particle_alpha && beam.angle_fade.is_none());
        assert_eq!(beam.sort_bias, 0.0);
    }
}

#[test]
fn profiles_age_out() {
    let (contrail, look) = (contrail(10.0, 0.5), look(None));
    let mut running = RunningContrail {
        initialized: true,
        ..Default::default()
    };
    let state = StripState::default();
    running.pulse(&contrail, &look, &state, [1.0, 0.0, 0.0], 0.1);
    assert!(!running.profiles.is_empty());
    running.update(&contrail, &state, 0.25);
    assert!(!running.profiles.is_empty());
    running.update(&contrail, &state, 0.25);
    assert!(running.profiles.is_empty());
}
