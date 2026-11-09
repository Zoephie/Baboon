use super::*;

#[test]
fn a_shadow_fades_as_the_probe_grows_even() {
    assert_eq!(gen3_shadow_alpha(2.0), 0.0);
    assert_eq!(gen3_shadow_alpha(1.0), 0.0);
    assert_eq!(gen3_shadow_alpha(5.0), 0.5);
    assert_eq!(gen3_shadow_alpha(10.0), 1.0);
    let mut even = DEFAULT_PROBE;
    even.iter_mut().for_each(|channel| channel[1..].fill(0.0));
    let contrast = |probe: &[[f32; 9]; 3]| dominant_light(probe).2;
    assert!(
        contrast(&DEFAULT_PROBE) > contrast(&even),
        "{}",
        contrast(&even)
    );
}

#[test]
fn a_shield_shows_once_either_reading_rises() {
    let values =
        |name: &str, value: f32| functions::FunctionValues::from([(name.to_owned(), value)]);
    assert!(gen3_shield_showing(Some(&values(H3_SHIELD_DAMAGE, 0.5))));
    assert!(gen3_shield_showing(Some(&values(H3_OVERSHIELD, 0.5))));
    assert!(!gen3_shield_showing(Some(&values(H3_OVERSHIELD, 0.0))));
    assert!(!gen3_shield_showing(Some(&values("body_vitality", 1.0))));
    assert!(!gen3_shield_showing(None));
}

#[test]
fn only_modes_that_add_light_leave_coverage_alone() {
    assert!(gen3_adds_light(Gen3Blend::Additive));
    assert!(gen3_adds_light(Gen3Blend::AddSrcTimesSrcAlpha));
    assert!(!gen3_adds_light(Gen3Blend::AlphaBlend));
    assert!(!gen3_adds_light(Gen3Blend::Opaque));
}

#[test]
fn the_dominant_light_points_at_the_bright_side_and_turns_with_it() {
    let (fixed, intensity, _) = dominant_light(&DEFAULT_PROBE);
    assert!(fixed[1] > 0.9 && fixed[0] < 0.0, "{fixed:?}");
    assert!(
        intensity[..3].iter().all(|value| *value > 0.0),
        "{intensity:?}"
    );
    let (turned, again, _) = dominant_light(&turned_probe(std::f32::consts::FRAC_PI_2));
    assert!(
        (turned[0] + fixed[1]).abs() < 1e-4 && (turned[1] - fixed[0]).abs() < 1e-4,
        "{turned:?}"
    );
    assert!((again[0] - intensity[0]).abs() < 1e-3);
}

#[test]
fn frames_have_their_own_upload_names() {
    assert_ne!(frame_key("base", 0), frame_key("base", 1));
    assert_ne!(frame_key("base", 0), "base");
}

#[test]
fn no_turn_leaves_the_probe_alone() {
    for (turned, stored) in turned_probe(0.0)
        .iter()
        .flatten()
        .zip(DEFAULT_PROBE.iter().flatten())
    {
        assert!((turned - stored).abs() < 1e-5);
    }
}

#[test]
fn a_turn_moves_the_bright_side_and_keeps_the_sky_and_the_energy() {
    let turned = turned_probe(std::f32::consts::FRAC_PI_2);
    let [x, y] = [turned[0][3], turned[0][1]];
    let [stored_x, stored_y] = [DEFAULT_PROBE[0][3], DEFAULT_PROBE[0][1]];
    assert!(
        (x + stored_y).abs() < 1e-5 && (y - stored_x).abs() < 1e-5,
        "{x} {y}"
    );
    assert_eq!(turned[0][0], DEFAULT_PROBE[0][0]);
    assert_eq!(turned[0][2], DEFAULT_PROBE[0][2]);
    let energy = |channel: &[f32; 9]| channel[4..].iter().map(|value| value * value).sum::<f32>();
    assert!((energy(&turned[0]) - energy(&DEFAULT_PROBE[0])).abs() < 1e-4);
    assert!((turned[0][4] - DEFAULT_PROBE[0][4]).abs() > 0.01);
}

#[test]
fn signed_texels_lose_the_engines_bias() {
    assert_eq!(
        &*signed_bytes(&[128, 255, 0, 1], ImageTexels::Signed),
        &[0, 127, 128, 129]
    );
}

#[test]
fn other_texels_pass_through_untouched() {
    assert_eq!(
        &*signed_bytes(&[128, 255], ImageTexels::Linear),
        &[128, 255]
    );
    assert_eq!(&*signed_bytes(&[128, 255], ImageTexels::Srgb), &[128, 255]);
}
