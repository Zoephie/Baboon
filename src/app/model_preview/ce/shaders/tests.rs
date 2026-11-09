use super::glass::decal_bias;
use super::*;
use crate::app::export::load_referenced_tag_from_source;
use crate::app::model_preview::harness::kit::{kit_root, kit_source};
use crate::core::game::GameId;
use std::sync::Arc;

const SINGLE_GROUPS: [(&str, &[u8; 4]); 4] = [
    ("shader_transparent_glass", b"sgla"),
    ("shader_transparent_meter", b"smet"),
    ("shader_transparent_water", b"swat"),
    ("shader_transparent_plasma", b"spla"),
];

/// With `BLAM_TEST_HCEEK`: every `ShaderModel` the game can ask for links
/// with its stage.
#[test]
fn every_model_shader_links_with_its_stage() {
    let Some(root) = kit_root(GameId::HaloCe) else {
        return;
    };
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let source = TagSource::LooseFolder {
        root,
        game: Some(crate::core::game::GameId::HaloCe),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let common = read_common(&crate::app::model_preview::load::TagReader::new(&source))
        .expect("the kit's model shader translates");
    let dir = crate::core::test_kits::unique_temp_dir("ce_model_shaders");
    let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
    std::fs::write(&vertex, Dx9VertexStage::CeModel.source(PASS_GLSL_VERSION)).unwrap();
    for (environment, mask, function, before) in (0..2).flat_map(|environment| {
        (0..9).flat_map(move |mask| {
            (0..3).flat_map(move |function| {
                [true, false].map(move |before| (environment, mask, function, before))
            })
        })
    }) {
        let call = format!("ShaderModel(i, {environment}, {mask}, {function}, {before}, false)");
        std::fs::write(&pixel, common.entry(&call)).unwrap();
        let output = std::process::Command::new("glslangValidator")
            .arg("-l")
            .arg(&vertex)
            .arg(&pixel)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{call} does not link:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

/// With `BLAM_TEST_HCEEK`: the kit's camouflage reads and links.
#[test]
fn active_camo_links_with_its_stage() {
    let Some(root) = kit_root(GameId::HaloCe) else {
        return;
    };
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let source = TagSource::LooseFolder {
        root,
        game: Some(crate::core::game::GameId::HaloCe),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let settings = read_ce_camo_settings(&crate::app::model_preview::load::TagReader::new(&source))
        .expect("the kit's camo reads");
    assert!(settings.refraction > 0.0);
    assert!(settings.distortion.is_some());
    let pass = ce_camo_pass(&settings, false, &mut |_, _| None);
    let dir = crate::core::test_kits::unique_temp_dir("ce_active_camo");
    let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
    std::fs::write(&vertex, pass.stage.source(PASS_GLSL_VERSION)).unwrap();
    std::fs::write(&pixel, &pass.fragment).unwrap();
    let output = std::process::Command::new("glslangValidator")
        .arg("-l")
        .arg(&vertex)
        .arg(&pixel)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// With `BLAM_TEST_HCEEK`: every transparent shader in the kit translates
/// and links with its stage.
#[test]
fn every_transparent_shader_links_with_its_stage() {
    let Some(root) = kit_root(GameId::HaloCe) else {
        return;
    };
    if std::process::Command::new("glslangValidator")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping: glslangValidator is not on PATH");
        return;
    }
    let tags = root;
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some(crate::core::game::GameId::HaloCe),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let dir = crate::core::test_kits::unique_temp_dir("ce_transparent_shaders");
    let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
    let mut folders = vec![tags.clone()];
    let mut drawn = std::collections::HashSet::new();
    let mut shaders = 0;
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let extension = path
                .extension()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let Some((_, group)) = LAYER_GROUPS
                .iter()
                .chain(&SINGLE_GROUPS)
                .find(|(name, _)| *name == extension)
            else {
                continue;
            };
            let relative = path.strip_prefix(&tags).unwrap().with_extension("");
            let relative = relative.to_string_lossy().replace('/', "\\");
            let tag =
                load_referenced_tag_from_source(&source, &relative, extension, group).unwrap();
            shaders += 1;
            let passes = ce_transparent_passes(
                &crate::app::model_preview::load::TagReader::new(&source),
                tag.root(),
                group,
                &mut |_, kind| match kind {
                    SamplerKind::Cube => Some(Arc::new(Image::Cube {
                        size: 1,
                        faces: std::array::from_fn(|_| vec![0; 4]),
                    })),
                    SamplerKind::Volume => None,
                    SamplerKind::Texture2D => Some(flat(WHITE)),
                },
            );
            assert!(!passes.is_empty(), "{} has no passes", path.display());
            for pass in passes {
                if !drawn.insert((pass.stage, pass.fragment.clone())) {
                    continue;
                }
                std::fs::write(&vertex, pass.stage.source(PASS_GLSL_VERSION)).unwrap();
                std::fs::write(&pixel, &pass.fragment).unwrap();
                let output = std::process::Command::new("glslangValidator")
                    .arg("-l")
                    .arg(&vertex)
                    .arg(&pixel)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{} does not link:\n{}",
                    path.display(),
                    String::from_utf8_lossy(&output.stdout)
                );
            }
        }
    }
    assert!(shaders > 100, "only {shaders} shaders");
    eprintln!("{shaders} shaders, {} programs", drawn.len());
}

#[test]
fn periodic_functions_keep_their_shape_and_range() {
    assert_eq!(periodic_evaluate(PERIODIC_ONE, 0.3), 1.0);
    assert_eq!(periodic_evaluate(PERIODIC_ZERO, 0.3), 0.0);
    // Cosine starts at its peak; a slide at nothing, and it wraps.
    assert!(periodic_evaluate(2, 0.0) > 0.99);
    assert!(periodic_evaluate(6, 0.0) < 0.01);
    for function in 2..12 {
        for step in 0..4096 {
            let value = periodic_evaluate(function, step as f32 * 0.037);
            assert!((0.0..=1.0).contains(&value), "{function} gave {value}");
        }
    }
}

fn map(waves: [TextureWave; 3], sources: [Option<usize>; 3]) -> CeMapAnimation {
    CeMapAnimation {
        row: 0,
        sources,
        waves,
        center: [0.5, 0.5],
        scale: [2.0, 3.0],
        offset: [0.0; 2],
        rotation: 0.0,
    }
}

#[test]
fn a_full_turn_of_rotation_leaves_the_rows_square() {
    let still = TextureWave {
        function: PERIODIC_ONE,
        period: 1.0,
        phase: 0.0,
        scale: 360.0,
    };
    let idle = TextureWave {
        function: PERIODIC_ZERO,
        period: 1.0,
        phase: 0.0,
        scale: 1.0,
    };
    let animation = map([idle.clone(), idle, still], [None; 3]);
    let mut rows = [[0.0; 4]; DX9_TEXTURE_ROWS];
    animation.apply(&mut rows, 1.0, &[0.0; 4]);
    assert_eq!(rows[0], [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(rows[1], [0.0, 3.0, 0.0, 0.0]);
    assert!(!animation.moves(&[0.0; 4]));
}

#[test]
fn scrolling_moves_and_an_input_that_rests_does_not() {
    let scroll = TextureWave {
        function: 6,
        period: 2.0,
        phase: 0.0,
        scale: 1.0,
    };
    let idle = TextureWave {
        function: PERIODIC_ONE,
        period: 1.0,
        phase: 0.0,
        scale: 1.0,
    };
    let mut animation = map([scroll, idle.clone(), idle], [None; 3]);
    assert!(animation.moves(&[0.0; 4]));
    animation.sources[0] = Some(0);
    assert!(!animation.moves(&[0.0; 4]));
    assert!(animation.moves(&[1.0, 0.0, 0.0, 0.0]));
}

/// With `BLAM_TEST_HCEEK`: the blink reads `A out`, so eyes with no object
/// behind them stay open and its output shuts them.
#[test]
fn an_eye_shader_reads_its_blink_from_an_object_function() {
    let Some(source) = kit_source(GameId::HaloCe) else {
        return;
    };
    let tag = load_referenced_tag_from_source(
        &source,
        "characters\\marine\\shaders\\eyes_charles",
        "shader_model",
        b"soso",
    )
    .unwrap();
    let animation = read_map_animation(tag.root(), 0, [1.0; 2], [0.0; 2], 0.0);
    assert_eq!(animation.sources, [None, Some(0), None]);
    let mut rows = [[0.0; 4]; DX9_TEXTURE_ROWS];
    animation.apply(&mut rows, 0.0, &[0.0; 4]);
    assert_eq!(rows[1][3], 0.0);
    animation.apply(&mut rows, 0.0, &[1.0, 0.0, 0.0, 0.0]);
    assert_eq!(rows[1][3], -0.06);
    assert!(animation.moves(&[0.0; 4]));
}

#[test]
fn a_meter_follows_its_sources_and_an_unset_one_reads_one() {
    let meter = CeMeter {
        sources: [Some(0), None, Some(1), None],
        flash: [1.0, 0.5, 0.0],
        maximum: [0.0; 3],
        minimum: [0.0; 3],
        background: [0.0; 3],
        tint: [1.0; 3],
        transparency: None,
        negative: true,
    };
    let constants = meter.constants(&[0.25, 0.75, 0.0, 0.0]);
    assert_eq!(constants[0], [1.0, 0.5, 0.0, 0.25]);
    assert_eq!(constants[2][3], 0.75);
    assert_eq!(constants[1][3], 1.0 / 8.0);
    assert_eq!(constants[5], [-1.0, -0.5, -0.0, -1.0]);
}

#[test]
fn plasma_rests_unseen_and_its_output_brings_it_up() {
    let noise = CePlasmaNoise {
        period: 2.0,
        direction: [1.0, 0.0, 0.0],
        scale: 3.0,
    };
    let plasma = CePlasma {
        intensity_source: Some(1),
        intensity_exponent: 1.0,
        offset_source: Some(1),
        offset_exponent: 2.0,
        offset_amount: 0.03,
        perpendicular: [1.0; 4],
        parallel: [1.0; 4],
        primary: noise.clone(),
        secondary: noise,
    };
    let animation = CeModelAnimation {
        plasma: Some(plasma),
        ..Default::default()
    };
    let mut rows = [[0.0; 4]; DX9_TEXTURE_ROWS];
    animation.apply_rows(&mut rows, 4.0, &[0.0; 4]);
    assert_eq!(rows[7][3], 0.0);
    assert_eq!(rows[0], [3.0, 0.0, 0.0, 2.0]);
    assert!(!animation.moves_with(&[0.0; 4]));

    animation.apply_rows(&mut rows, 4.0, &[0.0, 0.5, 0.0, 0.0]);
    assert_eq!(rows[7][3], 0.5);
    assert!((rows[0][2] - 0.0075).abs() < 1e-6);
    assert!(animation.moves_with(&[0.0, 0.5, 0.0, 0.0]));
}

#[test]
fn a_pass_says_which_outputs_it_reads() {
    let idle = TextureWave {
        function: PERIODIC_ONE,
        period: 1.0,
        phase: 0.0,
        scale: 1.0,
    };
    let map = map([idle.clone(), idle.clone(), idle], [None, Some(2), None]);
    let animation = CeModelAnimation {
        maps: vec![map],
        fade_source: Some(1),
        ..Default::default()
    };
    assert_eq!(animation.outputs_read(), [false, true, true, false]);
    assert_eq!(CeModelAnimation::default().outputs_read(), [false; 4]);
}

#[test]
fn only_a_decal_shader_takes_the_depth_bias() {
    assert_eq!(decal_bias(true), TRANSPARENT_DECAL_DEPTH_BIAS);
    assert_eq!(decal_bias(false), 0.0);
    // A bias pulls toward the viewer, by what a 24 bit buffer can hold.
    const { assert!(MODEL_DECAL_DEPTH_BIAS < TRANSPARENT_DECAL_DEPTH_BIAS) };
    const { assert!(TRANSPARENT_DECAL_DEPTH_BIAS < 0.0) };
}

#[test]
fn a_numeric_pass_shows_its_digit_of_its_place() {
    let digit = |digit, place| CeNumeric {
        map: String::new(),
        count: 10,
        limit: 60,
        digit,
        place,
        timer: false,
    };
    let digit_at = |value| digit(value, 1);
    // Half of 60 is 30, read off the first output for ten images.
    let outputs = [0.5, 0.0, 0.0, 0.0];
    assert!(digit(0, 0).shows(&outputs));
    assert!(digit(3, 1).shows(&outputs));
    assert!(!digit(3, 0).shows(&outputs));
    assert!(digit(0, 2).shows(&outputs));
    // Eight images read the fourth output instead.
    let octal = CeNumeric {
        count: 8,
        ..digit(1, 0)
    };
    assert!(octal.shows(&[0.0, 0.0, 0.0, 1.0 / 60.0]));
    assert!(!octal.shows(&[1.0 / 60.0, 0.0, 0.0, 0.0]));
    // A countdown ignores the object, and with no script stays at zero.
    let timer = |digit| CeNumeric {
        timer: true,
        ..digit_at(digit)
    };
    assert!(timer(0).shows(&outputs));
    assert!(!timer(3).shows(&outputs));
}

#[test]
fn a_chicago_stage_reads_the_engine_tables() {
    // Add: next map, then the current map through the second product.
    let add = chicago_color_table(4);
    assert_eq!(add[0], 1.0);
    assert_eq!((add[3], add[9], add[10], add[11]), (1.0, 1.0, 1.0, 1.0));
    // Double multiply scales its only product.
    assert_eq!(chicago_color_table(3)[10], 2.0);
    // Subtract next map takes it from the current one.
    assert_eq!(chicago_alpha_table(8)[9], -1.0);
}

#[test]
fn the_fade_stage_follows_the_blend_function() {
    let blend = |index| generic_fade_stage(index, 0, 3);
    // Alpha blend fades the alpha alone: a stage with no color output.
    assert!(
        blend(0).contains(",0,0,0,0,0,0,0,0,false,0,0,0,0,0,0,11,0,9,0,"),
        "{}",
        blend(0)
    );
    // Multiply fades toward white.
    assert!(
        blend(1).contains(",11,0,19,0,1,0,19,1,false,"),
        "{}",
        blend(1)
    );
    // Double multiply fades toward grey.
    assert!(
        blend(2).contains(",11,0,19,0,2,0,19,1,false,"),
        "{}",
        blend(2)
    );
}
