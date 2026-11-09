use super::*;
use super::super::super::functions::object_input;

fn animation(kind: H2AnimationKind, period: f32, value: f32) -> H2Animation {
    let function = blam_tags::tag_function::H2Function::new_constant(value);
    H2Animation {
        kind,
        period,
        held_input: None,
        range_input: None,
        function: Arc::new(TagFunction::parse_encoded(
            blam_tags::tag_function::FunctionEncoding::H2,
            &function.to_bytes(),
        ).unwrap()),
    }
}

#[test]
fn a_period_wraps_time_and_no_period_reads_it_raw() {
    let cycling = animation(H2AnimationKind::Value, 4.0, 0.0);
    assert_eq!(cycling.input(1.0.into()), 0.25);
    assert_eq!(cycling.input(5.0.into()), 0.25);
    assert_eq!(animation(H2AnimationKind::Value, 0.0, 0.0).input(5.0.into()), 5.0);
}

#[test]
fn a_texture_transform_moves_scales_and_turns_as_the_game_builds_it() {
    let row = |form, animations| {
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        H2AnimatedRow { register: 1, form, scale: [2.0, 4.0, 8.0], animations }
            .apply(&mut rows, 0.0.into());
        rows[1]
    };
    let moved = vec![
        animation(H2AnimationKind::TranslationX, 0.0, 0.25),
        animation(H2AnimationKind::TranslationY, 0.0, 0.5),
    ];
    // `sub_6D8B60` leaves a translation unscaled.
    assert_eq!(row(H2RowForm::SimpleXform, moved.clone()), [2.0, 4.0, 0.25, 0.5]);
    assert_eq!(row(H2RowForm::Row1, moved.clone()), [2.0, 0.0, 0.0, 0.25]);
    assert_eq!(row(H2RowForm::Row2, moved), [0.0, 4.0, 0.0, 0.5]);

    let deep = vec![animation(H2AnimationKind::TranslationZ, 0.0, 0.5)];
    assert_eq!(row(H2RowForm::Row3, deep), [0.0, 0.0, 8.0, 0.5]);
    let even = vec![animation(H2AnimationKind::ScaleUniform, 0.0, 3.0)];
    assert_eq!(row(H2RowForm::Row3, even), [0.0, 0.0, 3.0, 0.0], "uniform scale reaches z");

    let turned = row(
        H2RowForm::Row1,
        vec![animation(H2AnimationKind::Rotation, 0.0, std::f32::consts::FRAC_PI_2)],
    );
    // A quarter turn about the middle of the map keeps the middle in place.
    assert!(turned[0].abs() < 1e-6, "{turned:?}");
    assert!((turned[1] + 2.0).abs() < 1e-6, "{turned:?}");
    assert!((turned[3] - 2.0).abs() < 1e-6, "{turned:?}");
}

#[test]
fn an_object_input_holds_still_at_what_a_whole_object_exports() {
    assert_eq!(object_input("shield_vitality"), 1.0);
    assert_eq!(object_input("object_overshield_amount"), 0.0);
    assert_eq!(object_input("boost"), 0.0);
    let held = H2Animation { held_input: Some(("boost".to_owned(), 1.0)), ..animation(H2AnimationKind::Value, 4.0, 0.0) };
    assert_eq!(held.input(3.0.into()), 1.0);
    assert!(!held.moves());
    // An object's functions take over from the resting value.
    let values = FunctionValues::from([("boost".to_owned(), 0.25)]);
    assert_eq!(held.input(H2Clock { time: 3.0, values: Some(&values) }), 0.25);
}

#[test]
fn only_a_non_constant_function_moves() {
    assert!(!animation(H2AnimationKind::Value, 1.0, 0.5).moves());
}

#[test]
fn the_lights_choose_between_the_game_s_two_lightmap_paths() {
    assert_eq!(Dx9Layer::Lightmap.implementation(), 3);
    assert_eq!(Dx9Layer::ObjectLightmap.implementation(), 2);
    assert_eq!(Dx9Layer::ObjectLightmap.tag_name(), Dx9Layer::Lightmap.tag_name());
}

#[test]
fn change_colors_filter_by_variant_and_fall_back_to_white() {
    let entry = |weight, value: f32, variant: &str| ChangePermutation {
        weight,
        lower: [value; 3],
        upper: [value + 0.25; 3],
        variant: variant.to_owned(),
    };
    let list = [entry(1.0, 0.25, "minor"), entry(1.0, 0.5, "major"), entry(0.0, 0.75, "")];
    assert_eq!(change_color_for(&list, "minor"), [0.375; 3]);
    assert_eq!(change_color_for(&list, "major"), [0.625; 3]);
    assert_eq!(change_color_for(&list, "ultra"), [1.0; 3], "nothing allowed");
    assert_eq!(change_color_for(&[], "minor"), [1.0; 3]);
    let shared = [entry(1.0, 0.0, ""), entry(2.0, 0.5, ""), entry(1.0, 0.75, "")];
    assert_eq!(change_color_for(&shared, "any"), [0.625; 3], "the middle of the weights");
}

#[test]
fn a_block_with_no_weights_shares_them_evenly() {
    let entry = |value: f32, variant: &str| ChangePermutation {
        weight: 0.0,
        lower: [value; 3],
        upper: [value; 3],
        variant: variant.to_owned(),
    };
    let list = [entry(0.25, "minor"), entry(0.5, "major"), entry(0.75, "major")];
    assert_eq!(change_color_for(&list, "minor"), [0.25; 3]);
    assert_eq!(change_color_for(&list, "major"), [0.5; 3]);
    assert_eq!(change_color_for(&list, "ultra"), [1.0; 3], "no entry for this variant");
}

#[test]
fn normalization_faces_hold_their_own_direction() {
    let center = |face: usize, size: usize| -> [u8; 3] {
        let rgba = normalization_face(face, size);
        let at = (size / 2 * size + size / 2) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2]]
    };
    assert!(center(0, 33)[0] > 250, "+X");
    assert!(center(3, 33)[1] < 5, "-Y");
    assert!(center(4, 33)[2] > 250, "+Z");
    assert!(center(5, 33)[2] < 5, "-Z");
}

#[test]
fn the_source_file_follows_the_pass_path() {
    let pass = Dx9Pass {
        path: "shaders\\shader_passes\\transparent\\lit".to_owned(),
        implementation: 0,
        vertex_shader: String::new(),
        blend: None,
        depth: Dx9Depth { compare: None, write: false },
        channels: String::new(),
        custom_channels: None,
        alpha_test: None,
        textures: Vec::new(),
        parameters: Vec::new(),
        vertex_constants: Vec::new(),
        animations: Default::default(),
    };
    assert_eq!(pass.pixel_shader_file(), "shaders_shader_passes_transparent_lit_0.psh");
}

fn permutation(weight: f32, lower: [f32; 3], upper: [f32; 3]) -> ChangePermutation {
    ChangePermutation { weight, lower, upper, variant: String::new() }
}

#[test]
fn a_ce_object_at_the_origin_picks_by_channel() {
    let red = [1.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 1.0];
    let list = [permutation(0.3, red, red), permutation(0.7, blue, blue)];
    // Channel 0 hashes to 0, and 1 to 0.129: both inside the first share.
    assert_eq!(ce_change_color(&list, 0), red);
    assert_eq!(ce_change_color(&list, 1), red);
    // Channel 3 hashes to 0.387, into the second.
    assert_eq!(ce_change_color(&list, 3), blue);
    assert_eq!(ce_change_color(&[], 3), [1.0; 3]);
    // No weights shares evenly: 0.387 is the first of two.
    let even = [permutation(0.0, red, red), permutation(0.0, blue, blue)];
    assert_eq!(ce_change_color(&even, 3), red);
}

#[test]
fn ce_colors_blend_the_short_way_round_the_hue() {
    // Red and a red just shy of the wrap meet near red, not across cyan.
    let mixed = hsv_interpolate([1.0, 0.0, 0.0], [1.0, 0.0, 0.2], 0.5, false);
    assert!(mixed[0] > 0.99 && mixed[1] < 0.01, "{mixed:?}");
    // Unlike an RGB blend, yellow keeps its brightness between red and green.
    let middle = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, false);
    assert!((middle[0] - 1.0).abs() < 1e-5 && (middle[1] - 1.0).abs() < 1e-5, "{middle:?}");
    // The long way from red to green goes through blue.
    let long = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, true);
    assert!(long[2] > 0.99 && long[1] < 0.01, "{long:?}");
    // A color survives the trip.
    let color = [0.34, 0.4, 0.15];
    let back = hsv_to_rgb(rgb_to_hsv(color));
    assert!(color.iter().zip(back).all(|(a, b)| (a - b).abs() < 1e-5), "{back:?}");
}

/// The dx9 pipeline is neutral: Halo CE's shaders hook into it through
/// `PassAnimation`, and only its game table (`render.rs`) names Halo CE.
#[test]
fn only_the_dx9_game_table_names_halo_ce() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/model_preview/dx9");
    let mut files = vec![root.clone()];
    let mut naming = Vec::new();
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().map(|entry| entry.unwrap().path()));
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") || path == root.join("render.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (number, line) in source.lines().enumerate() {
            // Spelled apart, so this test's own source is not a match.
            if [concat!("super::", "ce::"), concat!("model_preview::", "ce::")].iter().any(|path| line.contains(path)) {
                naming.push(format!("{}:{}", path.display(), number + 1));
            }
        }
    }
    assert!(naming.is_empty(), "dx9 names Halo CE at {naming:?}");
}
