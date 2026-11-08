use super::super::shaders::H3MaterialDialect;
use super::*;

/// Reach's foliage draws both faces of its own passes; any other group
/// (a halogram among them), Halo 3's foliage, and a pass laid over the
/// part keep culling.
#[test]
fn only_reach_foliages_own_passes_draw_two_sided() {
    let pass = || Arc::new(H3Pass { fullscreen: false, vertex: String::new(), pixel: String::new(), uniforms: Vec::new(), textures: Vec::new() });
    let material = |dialect| H3Material {
        dialect,
        albedo: None,
        lit: pass(),
        screen_maps: Vec::new(),
        decal: false,
        decal_modulates_specular: false,
        after_lighting: false,
        sort_layer: 0,
        prt: [None, None, Some(pass())],
        camo: None,
        shadow_generate: None,
        dynamic_light: None,
        blend: H3Blend::Additive,
        values: Default::default(),
        textures: Vec::new(),
        change_colors: Vec::new(),
        animations: Vec::new(),
        shield: None,
        fringe: None,
    };
    let foliage = Some(u32::from_be_bytes(*b"rmfl"));
    let reach = material(H3MaterialDialect::Reach);
    assert!(reach_draws_two_sided(&reach, &reach.lit, foliage));
    assert!(reach_draws_two_sided(&reach, reach.prt[2].as_ref().unwrap(), foliage));
    assert!(!reach_draws_two_sided(&reach, &pass(), foliage), "a shield over it culls");
    assert!(!reach_draws_two_sided(&reach, &reach.lit, Some(u32::from_be_bytes(*b"rmhg"))), "a halogram culls");
    let halo3 = material(H3MaterialDialect::Halo3);
    assert!(!reach_draws_two_sided(&halo3, &halo3.lit, foliage));
}

#[test]
fn a_shadow_fades_as_the_probe_grows_even() {
    assert_eq!(h3_shadow_alpha(2.0), 0.0);
    assert_eq!(h3_shadow_alpha(1.0), 0.0);
    assert_eq!(h3_shadow_alpha(5.0), 0.5);
    assert_eq!(h3_shadow_alpha(10.0), 1.0);
    let mut even = DEFAULT_PROBE;
    even.iter_mut().for_each(|channel| channel[1..].fill(0.0));
    let contrast = |probe: &[[f32; 9]; 3]| dominant_light(probe).2;
    assert!(contrast(&DEFAULT_PROBE) > contrast(&even), "{}", contrast(&even));
}


#[test]
fn a_shield_shows_once_either_reading_rises() {
    let values = |name: &str, value: f32| {
        functions::FunctionValues::from([(name.to_owned(), value)])
    };
    assert!(h3_shield_showing(Some(&values(H3_SHIELD_DAMAGE, 0.5))));
    assert!(h3_shield_showing(Some(&values(H3_OVERSHIELD, 0.5))));
    assert!(!h3_shield_showing(Some(&values(H3_OVERSHIELD, 0.0))));
    assert!(!h3_shield_showing(Some(&values("body_vitality", 1.0))));
    assert!(!h3_shield_showing(None));
}

#[test]
fn only_modes_that_add_light_leave_coverage_alone() {
    assert!(h3_adds_light(H3Blend::Additive));
    assert!(h3_adds_light(H3Blend::AddSrcTimesSrcAlpha));
    assert!(!h3_adds_light(H3Blend::AlphaBlend));
    assert!(!h3_adds_light(H3Blend::Opaque));
}

#[test]
fn the_dominant_light_points_at_the_bright_side_and_turns_with_it() {
    let (fixed, intensity, _) = dominant_light(&DEFAULT_PROBE);
    assert!(fixed[1] > 0.9 && fixed[0] < 0.0, "{fixed:?}");
    assert!(intensity[..3].iter().all(|value| *value > 0.0), "{intensity:?}");
    let (turned, again, _) = dominant_light(&turned_probe(std::f32::consts::FRAC_PI_2));
    assert!((turned[0] + fixed[1]).abs() < 1e-4 && (turned[1] - fixed[0]).abs() < 1e-4, "{turned:?}");
    assert!((again[0] - intensity[0]).abs() < 1e-3);
}

#[test]
fn frames_have_their_own_upload_names() {
    assert_ne!(frame_key("base", 0), frame_key("base", 1));
    assert_ne!(frame_key("base", 0), "base");
}

#[test]
fn no_turn_leaves_the_probe_alone() {
    for (turned, stored) in turned_probe(0.0).iter().flatten().zip(DEFAULT_PROBE.iter().flatten()) {
        assert!((turned - stored).abs() < 1e-5);
    }
}

#[test]
fn a_turn_moves_the_bright_side_and_keeps_the_sky_and_the_energy() {
    let turned = turned_probe(std::f32::consts::FRAC_PI_2);
    let [x, y] = [turned[0][3], turned[0][1]];
    let [stored_x, stored_y] = [DEFAULT_PROBE[0][3], DEFAULT_PROBE[0][1]];
    assert!((x + stored_y).abs() < 1e-5 && (y - stored_x).abs() < 1e-5, "{x} {y}");
    assert_eq!(turned[0][0], DEFAULT_PROBE[0][0]);
    assert_eq!(turned[0][2], DEFAULT_PROBE[0][2]);
    let energy = |channel: &[f32; 9]| channel[4..].iter().map(|value| value * value).sum::<f32>();
    assert!((energy(&turned[0]) - energy(&DEFAULT_PROBE[0])).abs() < 1e-4);
    assert!((turned[0][4] - DEFAULT_PROBE[0][4]).abs() > 0.01);
}

#[test]
fn signed_texels_lose_the_engines_bias() {
    assert_eq!(&*signed_bytes(&[128, 255, 0, 1], H3Texels::Signed), &[0, 127, 128, 129]);
}

#[test]
fn other_texels_pass_through_untouched() {
    assert_eq!(&*signed_bytes(&[128, 255], H3Texels::Linear), &[128, 255]);
    assert_eq!(&*signed_bytes(&[128, 255], H3Texels::Srgb), &[128, 255]);
}
