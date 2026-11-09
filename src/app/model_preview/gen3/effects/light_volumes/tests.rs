use super::*;
use crate::app::model_preview::harness::kit::{kit_root, kit_source};
use crate::core::game::GameId;

#[test]
fn the_count_truncates_while_the_shaders_keep_the_whole_product() {
    assert_eq!(profile_layout(1.5, 5.0), (7, 7.5, 0.2));
    assert_eq!(profile_layout(0.0, 5.0).0, 0);
    assert_eq!(profile_layout(1.0, 0.0).0, 0);
    assert_eq!(profile_layout(100.0, 100.0).0, MOST_PROFILES);
}

#[test]
fn the_last_profile_can_fall_short_of_the_end() {
    let (count, profiles, _) = profile_layout(1.5, 5.0);
    assert!(percentile(count - 1, profiles) < 1.0);
    assert_eq!(percentile(0, profiles), 0.0);
    assert_eq!(percentile(0, 1.0), 0.0, "a row of one divides by nothing");
}

#[test]
fn overdraw_is_every_profile_head_on_and_what_overlaps_from_the_side() {
    assert_eq!(overdraw(10.0, 0.5, 0.0), 10.0);
    assert_eq!(overdraw(10.0, 0.5, 0.1), 5.0);
    assert_eq!(overdraw(4.0, 0.5, 0.1), 4.0);
}

#[test]
fn a_ratio_nothing_gave_comes_from_how_far_a_function_peaks() {
    use blam_tags::tag_function::{FunctionEncoding, FunctionType, H2Function};
    let ranged = |low: f32, high: f32| {
        let mut function = H2Function::new(FunctionType::Identity);
        function.set_clamp_range(low, high).unwrap();
        let function =
            TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap();
        Gen3VolumeProperty {
            function: Arc::new(function),
            ..property(Gen3VolumeInput::Percentile, Gen3VolumeInput::Percentile)
        }
    };
    assert_eq!(ranged(0.5, 0.5).peak_over_mean(), 1.0);
    assert_eq!(ranged(0.0, 1.0).peak_over_mean(), 2.0);
}

fn property(input: Gen3VolumeInput, range: Gen3VolumeInput) -> Gen3VolumeProperty {
    Gen3VolumeProperty {
        input,
        range,
        adds: None,
        modifier_input: Gen3VolumeInput::Percentile,
        // A curve, for what allocates seeds only past a constant.
        function: Arc::new(
            TagFunction::parse(&[3; 32]).unwrap_or_else(|_| TagFunction::parse(&[0; 32]).unwrap()),
        ),
    }
}

#[test]
fn inputs_read_by_their_names() {
    let state = Gen3VolumeState {
        age: 0.3,
        effect_a: 0.7,
        effect_b: 0.2,
        seeds: [0.4, 0.6],
        ..Default::default()
    };
    assert_eq!(Gen3VolumeInput::named("effect a scale").of(&state), 0.7);
    assert_eq!(Gen3VolumeInput::named("effect b scale").of(&state), 0.2);
    assert_eq!(Gen3VolumeInput::named("light volume age").of(&state), 0.3);
    assert_eq!(
        Gen3VolumeInput::named("light volume correlation 2").of(&state),
        0.6
    );
}

#[test]
fn a_name_no_longer_in_the_game_reads_as_invalid() {
    let state = Gen3VolumeState {
        age: 0.3,
        ..Default::default()
    };
    assert_eq!(
        Gen3VolumeInput::named("light volume lifespan"),
        Gen3VolumeInput::Invalid
    );
    assert_eq!(Gen3VolumeInput::Invalid.of(&state), 0.0);
}

#[test]
fn what_runs_once_per_volume_cannot_read_a_profile_or_a_random() {
    assert_eq!(
        Gen3VolumeInput::Percentile.per_volume(),
        Gen3VolumeInput::Invalid
    );
    assert_eq!(
        Gen3VolumeInput::Random.per_volume(),
        Gen3VolumeInput::Invalid
    );
    assert_eq!(Gen3VolumeInput::Age.per_volume(), Gen3VolumeInput::Age);
    assert_eq!(
        Gen3VolumeInput::Seed(1).per_volume(),
        Gen3VolumeInput::Seed(1)
    );
}

#[test]
fn randoms_take_seeds_in_turn_after_any_a_tag_named() {
    let (mut input, mut next) = (Gen3VolumeInput::Random, 1);
    allocate_seed(&mut input, &mut next);
    assert_eq!(input, Gen3VolumeInput::Seed(1));
    let mut wrapped = Gen3VolumeInput::Random;
    allocate_seed(&mut wrapped, &mut next);
    assert_eq!(
        wrapped,
        Gen3VolumeInput::Seed(0),
        "two seeds, then round again"
    );
    let named = property(Gen3VolumeInput::Seed(0), Gen3VolumeInput::Age);
    assert_eq!(first_free_seed(&[&named]), 1);
    assert_eq!(
        first_free_seed(&[&property(Gen3VolumeInput::Age, Gen3VolumeInput::Age)]),
        0
    );
}

fn h2_function(
    kind: blam_tags::tag_function::FunctionType,
    low: f32,
    high: f32,
) -> Arc<TagFunction> {
    use blam_tags::tag_function::{FunctionEncoding, H2Function};
    let mut function = H2Function::new(kind);
    function.set_clamp_range(low, high).unwrap();
    Arc::new(TagFunction::parse_encoded(FunctionEncoding::H2, &function.to_bytes()).unwrap())
}

/// A constant, which reads nothing.
fn fixed(value: f32) -> Gen3VolumeProperty {
    use blam_tags::tag_function::FunctionType;
    let function = h2_function(FunctionType::Constant, value, value);
    Gen3VolumeProperty {
        function,
        ..property(Gen3VolumeInput::Invalid, Gen3VolumeInput::Invalid)
    }
}

/// Its input, from `low` to `high`.
fn reading(input: Gen3VolumeInput, low: f32, high: f32) -> Gen3VolumeProperty {
    use blam_tags::tag_function::FunctionType;
    Gen3VolumeProperty {
        function: h2_function(FunctionType::Identity, low, high),
        ..property(input, Gen3VolumeInput::Invalid)
    }
}

/// A row of 4 profiles a unit apart, white, alpha by `alpha`.
fn definition(reach: bool, alpha: Gen3VolumeProperty) -> Gen3VolumeDefinition {
    Gen3VolumeDefinition {
        length: fixed(4.0),
        offset: fixed(0.0),
        density: fixed(1.0),
        profile_length: fixed(0.5),
        thickness: fixed(0.5),
        color: fixed(1.0),
        alpha,
        intensity: fixed(2.0),
        brightness_ratio: 1.0,
        blend: Gen3Blend::AddSrcTimesSrcAlpha,
        bitmap: None,
        sampler: None,
        sort_layer: 2,
        reach,
        circular: None,
        depth_fade: None,
        lod: None,
        precompiled: None,
        h4: None,
    }
}

fn at(definition: Gen3VolumeDefinition) -> Gen3LightVolumeAt {
    Gen3LightVolumeAt {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        world_oriented: false,
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        seeds: [0.0; SEEDS],
        times: Arc::new(EffectTimes::read_nothing()),
        event: 0,
        definition: Arc::new(definition),
    }
}

/// Side on, from 10 units out.
fn view(distance: f32) -> ViewFrame {
    ViewFrame {
        eye: [0.0, -distance, 0.0],
        forward: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
    }
}

/// Reach's state 6 is the LOD amount, 1 with `lod enabled` off; Halo
/// 3's reads 0.
#[test]
fn reachs_lod_input_reads_the_amount_and_halo_3s_none() {
    let state = Gen3VolumeState {
        lod: 0.7,
        ..Default::default()
    };
    assert_eq!(Gen3VolumeInput::Lod.of(&state), 0.7);
    let alpha_of = |reach: bool| {
        let volume = at(definition(reach, reading(Gen3VolumeInput::Lod, 0.0, 1.0)));
        volume
            .quads(None, None, &view(10.0), 0.5, 0.0, 0.0)
            .first()
            .map(|quad| quad.color[3])
    };
    assert!(
        alpha_of(true).is_some_and(|alpha| alpha > 0.0),
        "{:?}",
        alpha_of(true)
    );
    assert_eq!(alpha_of(false), Some(0.0), "Halo 3's lod is the nearest, 0");
}

/// `calc_lod_amount`: in from the in distance over its feather, out to
/// the out distance over its, and nothing with `lod always 1.0`.
#[test]
fn a_reach_volume_fades_in_and_out_by_its_lod_distances() {
    let lod = ReachVolumeLod::postprocessed([30.0, 15.0], [100.0, 0.0], false);
    assert_eq!(lod_amount(None, 1000.0), 1.0, "no lod, always whole");
    assert_eq!(lod_amount(Some(&lod), 30.0), 0.0);
    assert_eq!(lod_amount(Some(&lod), 37.5), 0.5);
    assert_eq!(lod_amount(Some(&lod), 50.0), 1.0);
    assert_eq!(lod_amount(Some(&lod), 100.0), 0.0);
    let quirk = ReachVolumeLod {
        always_one: true,
        ..lod
    };
    assert_eq!(
        lod_amount(Some(&quirk), 50.0),
        0.0,
        "enabled and always 1 draws nothing"
    );
    let gated = |lod: Option<ReachVolumeLod>, reach: bool, distance: f32| {
        let volume = at(Gen3VolumeDefinition {
            lod,
            ..definition(reach, fixed(1.0))
        });
        volume
            .quads(None, None, &view(distance), 0.5, 0.0, 0.0)
            .len()
    };
    assert_eq!(
        gated(Some(lod), true, 10.0),
        0,
        "nearer than its in distance"
    );
    assert_eq!(gated(Some(lod), true, 50.0), 4);
    assert_eq!(gated(None, false, 10.0), 4, "Halo 3's has no gate");
}

/// The postprocess's defaults: an out distance of none is 20, the in
/// distance at least epsilon, and each feather inverted over at least it.
#[test]
fn reachs_postprocess_fills_in_lod_distances_and_inverses() {
    let lod = ReachVolumeLod::postprocessed([0.0, 0.0], [0.0, 10.0], false);
    assert_eq!(lod.far, DEFAULT_LOD_OUT);
    assert_eq!(lod.near, EPSILON);
    assert_eq!(lod.inverse_in, EPSILON.recip());
    assert_eq!(lod.inverse_out, 0.1);
    assert_eq!(
        ReachVolumeLod::postprocessed([5.0, 2.0], [15.0, 10.0], false).far,
        15.0
    );
}

/// `lights__light_volume.hlsl`: Reach's alpha is by intensity too and its
/// colour at `V_ILLUM_EXPOSURE` and the stops; a baked row's carries its
/// own. Halo 3's is as it was.
#[test]
fn a_profile_colour_takes_intensity_on_alpha_and_each_games_exposure() {
    let state = Gen3VolumeState::default();
    let reach = definition(true, fixed(0.5));
    let halo3 = definition(false, fixed(0.5));
    // What a scalar function makes of a colour, at intensity 2.
    let lit = reach.color.color(&state)[0] * 2.0;
    let [red, _, _, alpha] = reach.profile_color(&state, None, [1.0, 2.0], 0.0);
    assert_eq!(red, lit * reach_alt_exposure(0.0)[1]);
    assert_eq!(alpha, 0.5 * 2.0 / 2.0);
    let [brighter, ..] = reach.profile_color(&state, None, [1.0, 2.0], 1.0);
    assert_eq!(brighter, lit * reach_alt_exposure(1.0)[1]);
    assert_ne!(brighter, red, "the stops reach it");
    let [red, _, _, alpha] = halo3.profile_color(&state, None, [1.0, 2.0], 1.0);
    assert_eq!(red, lit * H3_EXPOSURE);
    assert_eq!(
        alpha,
        0.5 * 2.0 / 2.0,
        "Halo 3's float4 colour scales alpha by intensity too"
    );
    let [red, _, _, alpha] = reach.profile_color(&state, Some([3.0, 0.0, 0.0]), [1.0, 2.0], 0.0);
    assert_eq!(red, 3.0 * reach_alt_exposure(0.0)[1], "unclamped");
    assert_eq!(
        alpha, 0.5,
        "a baked row's alpha is the ratio over the overdraw"
    );
    let multiply = Gen3VolumeDefinition {
        blend: Gen3Blend::Multiply,
        ..definition(true, fixed(0.5))
    };
    assert_eq!(
        multiply.profile_color(&state, None, [1.0, 1.0], 1.0)[0],
        lit,
        "a multiply is not exposed"
    );
}

/// `get_uses_any_of_these_states`: a constant reads nothing, a range
/// only when ranged, a modifier only with one.
#[test]
fn what_a_property_reads_besides_its_place() {
    assert!(!fixed(1.0).reads_more_than_its_place());
    assert!(!reading(Gen3VolumeInput::Percentile, 0.0, 1.0).reads_more_than_its_place());
    assert!(reading(Gen3VolumeInput::Age, 0.0, 1.0).reads_more_than_its_place());
    let unranged = Gen3VolumeProperty {
        range: Gen3VolumeInput::Age,
        ..reading(Gen3VolumeInput::Percentile, 0.0, 1.0)
    };
    assert!(
        !unranged.reads_more_than_its_place(),
        "an unranged function never reads its range"
    );
    let modified = Gen3VolumeProperty {
        adds: Some(true),
        modifier_input: Gen3VolumeInput::EffectA,
        ..unranged.clone()
    };
    assert!(modified.reads_more_than_its_place());
    let unmodified = Gen3VolumeProperty {
        adds: None,
        ..modified
    };
    assert!(!unmodified.reads_more_than_its_place());
}

/// `max profile count`: the most density and length can make, held to
/// 128; and a volume bakes when what it reads is the profile's place,
/// unless told not to.
#[test]
fn a_reach_volume_reading_only_its_place_precompiles_up_to_its_max() {
    assert_eq!(max_profile_count(&fixed(2.0), &fixed(3.5)), 7);
    assert_eq!(
        max_profile_count(&reading(Gen3VolumeInput::Age, 1.0, 4.0), &fixed(3.0)),
        12,
        "a curve's high end"
    );
    assert_eq!(
        max_profile_count(&fixed(100.0), &fixed(100.0)),
        MOST_PROFILES as usize
    );
    assert_eq!(max_profile_count(&fixed(-1.0), &fixed(3.0)), 0);
    let place = reading(Gen3VolumeInput::Percentile, 0.0, 1.0);
    let constant = fixed(1.0);
    let only_place = [
        &constant, &constant, &constant, &constant, &place, &place, &place, &place,
    ];
    let none = |_: &str| false;
    assert!(precompiles(none, &only_place, 4));
    assert!(!precompiles(none, &only_place, 0), "no rows to bake");
    assert!(!precompiles(
        |flag: &str| flag == "disable precompiled profiles",
        &only_place,
        4
    ));
    let age = reading(Gen3VolumeInput::Age, 0.0, 1.0);
    let live = [
        &constant, &constant, &constant, &constant, &place, &place, &age, &place,
    ];
    assert!(!precompiles(none, &live, 4));
    assert!(precompiles(
        |flag: &str| flag == "force precompile profiles",
        &live,
        4
    ));
}

/// The baked rows sit at `i / (max − 0.5)` and hold colour by intensity
/// squared by alpha, and draw `max` of them on the live spacing.
#[test]
fn a_baked_row_is_placed_by_its_max_and_draws_that_many() {
    let mut volume = definition(true, reading(Gen3VolumeInput::Percentile, 0.0, 1.0));
    let rows = volume.bake(3);
    assert_eq!(rows.len(), 3);
    let alpha = |index: f32| index / 2.5;
    // Intensity 2, squared.
    let white = volume.color.color(&Gen3VolumeState::default())[0] * 4.0;
    for (index, (color, thickness)) in rows.iter().enumerate() {
        assert!(
            (color[0] - white * alpha(index as f32)).abs() < 1e-5,
            "{index}: {color:?}"
        );
        assert_eq!(*thickness, 0.5);
    }
    volume.precompiled = Some(rows);
    let quads = at(volume).quads(None, None, &view(10.0), 0.5, 0.0, 0.0);
    assert_eq!(quads.len(), 3, "the max, not the live 4");
    let middle =
        |quad: &AntennaQuad| quad.corners.iter().map(|corner| corner[0]).sum::<f32>() / 4.0;
    assert!(
        (middle(&quads[1]) - middle(&quads[0]) - 1.0).abs() < 1e-5,
        "a unit apart, as live"
    );
}

/// With `BLAM_TEST_HREK`: Reach's volumes draw the `circular` disc from
/// its option's defaults with no map, fade on `on`, and a `lod enabled`
/// one reads its distances.
#[test]
fn kit_reach_volumes_read_circular_depth_fade_and_lod() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let first = |path: &str| {
        let tag = load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl")
            .expect(path);
        read_definition(
            &crate::app::model_preview::load::TagReader::new(&source),
            elements(tag.root(), "light_volumes")[0],
        )
        .expect(path)
    };
    let headlights = first("objects\\vehicles\\human\\civilian\\cargo_truck\\fx\\headlights");
    assert!(headlights.reach);
    assert_eq!(headlights.circular, Some([1.1, 2.0]));
    assert!(headlights.bitmap.is_none(), "a disc samples no map");
    assert_eq!(headlights.depth_fade, Some(0.1));
    assert!(headlights.lod.is_none());
    let roof = first("levels\\solo\\m52\\fx\\roof_glow\\roof_glow");
    assert_eq!(
        roof.lod,
        Some(ReachVolumeLod::postprocessed(
            [30.0, 15.0],
            [100.0, 0.0],
            false
        ))
    );
}

/// With `BLAM_TEST_H3EK`: Halo 3's volumes keep their map, and none of
/// Reach's disc, fade, lod or baked rows.
#[test]
fn kit_halo_3_volumes_keep_their_map() {
    let Some(source) = kit_source(GameId::Halo3) else {
        return;
    };
    let path = "objects\\characters\\masterchief\\fx\\flashlight";
    let tag =
        load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl").expect(path);
    let volume = read_definition(
        &crate::app::model_preview::load::TagReader::new(&source),
        elements(tag.root(), "light_volumes")[0],
    )
    .expect(path);
    assert!(!volume.reach && volume.bitmap.is_some());
    assert!(volume.circular.is_none() && volume.depth_fade.is_none() && volume.lod.is_none());
    assert!(volume.precompiled.is_none());
}

/// Halo 4's frame: profiles placed along the direction from the offset a
/// spacing apart, each with its own place, and the overall state the
/// whole product of length and density, the LOD amount in state 6.
#[test]
fn a_halo_4_frame_places_its_profiles_and_fills_its_inputs() {
    let color = reading(Gen3VolumeInput::Percentile, 0.0, 1.0);
    let base = Gen3VolumeDefinition {
        length: fixed(3.5),
        offset: fixed(0.5),
        density: fixed(2.0),
        color,
        reach: true,
        ..definition(true, fixed(1.0))
    };
    assert!(
        at(definition(true, fixed(1.0)))
            .h4_draw(None, None, &view(10.0), 0.5, 0.0)
            .is_none(),
        "not Halo 4's"
    );
    let volume = at(base);
    let draw = volume
        .h4_frame(3, None, None, &view(10.0), 0.5, 0.0)
        .expect("a frame");
    // 3.5 × 2 = 7 profiles, 0.5 apart.
    assert_eq!(draw.profiles, 7.0);
    assert_eq!(draw.distance, 0.5);
    assert_eq!(draw.states.len(), 7);
    assert_eq!(draw.appearance, 3.0);
    let x = |words: &[u32; 8]| f32::from_bits(words[0]);
    assert_eq!(x(&draw.states[0]), 0.5);
    assert_eq!(x(&draw.states[6]), 0.5 + 0.5 * 6.0);
    let place = |words: &[u32; 8]| f32::from_bits(words[3]);
    assert_eq!(place(&draw.states[0]), 0.0);
    assert_eq!(place(&draw.states[6]), 1.0);
    // The colour reads the place: red rises along the row.
    let red = |words: &[u32; 8]| (words[5] >> 16) & 0xFF;
    assert!(red(&draw.states[6]) > red(&draw.states[0]));
    assert_eq!(draw.inputs[0], 0.0, "the place is the profile's");
    assert_eq!(draw.inputs[6], 1.0, "no lod enabled: the full amount");
    assert_eq!(draw.inputs[7], 1.0, "no scale named: effect a is 1");
}

/// With `BLAM_TEST_H4EK`: Halo 4's volumes draw through their
/// material's `light_volume` pass and sort by its layer, with none of
/// Reach's map, disc or baked rows.
#[test]
fn kit_halo_4_volumes_draw_through_their_material() {
    let Some(root) = kit_root(GameId::Halo4) else {
        return;
    };
    let source = super::super::super::super::particles::loose_kit(&root, GameId::Halo4);
    let path = "objects\\vehicles\\covenant\\turrets\\plasma_turret\\fx\\tripod_light_volume";
    let tag =
        load_referenced_tag_from_source(&source, path, "light_volume_system", b"ltvl").expect(path);
    let volume = read_definition(
        &crate::app::model_preview::load::TagReader::new(&source),
        elements(tag.root(), "light_volumes")[0],
    )
    .expect(path);
    let (material, _) = volume.h4.as_ref().expect("a material");
    assert!(
        material
            .pass
            .vertex
            .contains("vs_light_volume_state_buffer")
    );
    assert!(material.pass.vertex.contains("vs_lightVolumeOverallState"));
    assert!(volume.bitmap.is_none() && volume.circular.is_none() && volume.precompiled.is_none());
    let draw = at(volume)
        .h4_draw(None, None, &view(2.0), 0.5, 0.0)
        .expect("it draws at rest");
    assert!(!draw.states.is_empty());
    // Every volume in the kit reads onto its material.
    let mut failures = Vec::new();
    let mut read = 0;
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
                .is_none_or(|extension| extension != "light_volume_system")
            {
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap().with_extension("");
            let relative = relative.to_string_lossy().replace('/', "\\");
            let tag =
                load_referenced_tag_from_source(&source, &relative, "light_volume_system", b"ltvl")
                    .unwrap();
            for element in elements(tag.root(), "light_volumes") {
                match read_definition(
                    &crate::app::model_preview::load::TagReader::new(&source),
                    element,
                ) {
                    Some(volume) if volume.h4.is_some() => read += 1,
                    _ => failures.push(relative.clone()),
                }
            }
        }
    }
    eprintln!(
        "NOTE KIT {read} Halo 4 volumes read, {} failed: {failures:?}",
        failures.len()
    );
    assert!(failures.is_empty());
    assert!(read > 150, "volumes read: {read}");
}

#[test]
fn two_volumes_get_different_seeds_inside_the_unit_range() {
    let (first, second) = (volume_seeds(0), volume_seeds(1));
    assert_ne!(first, second);
    assert!(
        first
            .iter()
            .chain(&second)
            .all(|seed| (0.0..=1.0).contains(seed))
    );
}
