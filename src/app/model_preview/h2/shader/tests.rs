use super::super::super::dx9::passes::{
    Dx9BlendFactor, Dx9Compare, Dx9Depth, Dx9Layer, Dx9ParameterValue, Dx9Pass, Dx9PassTexture,
    Dx9VertexConstant, animated_vector, elements, read_change_colors,
};
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::core::game::GameId;
use std::collections::HashMap;

#[test]
fn object_env_brightness_is_the_luminance_of_the_lighting() {
    let [brightness, ..] = extern_value("LIGHTMAP object env brightness", 1.0);
    assert!((brightness - 0.554).abs() < 0.001, "{brightness}");
}

fn level(entries: &[(&str, &str, &str, f32)]) -> Parameters {
    entries
        .iter()
        .map(|(name, kind, bitmap, value)| {
            (
                (*name).to_owned(),
                ParameterLevel {
                    animations: Vec::new(),
                    kind: (*kind).to_owned(),
                    bitmap: (*bitmap).to_owned(),
                    value: *value,
                    color: [*value; 3],
                },
            )
        })
        .collect()
}

#[test]
fn the_shader_wins_then_the_template_then_the_pass() {
    let shader = level(&[("a", "value", "", 1.0)]);
    let template = level(&[("a", "value", "", 2.0), ("b", "value", "", 2.0)]);
    let pass = level(&[
        ("a", "value", "", 3.0),
        ("b", "value", "", 3.0),
        ("c", "value", "", 3.0),
    ]);
    let levels = [&shader, &template, &pass];
    for (name, expected) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
        assert_eq!(
            resolve_parameter(levels, name, "value"),
            Dx9ParameterValue::Value(expected)
        );
    }
    assert_eq!(
        resolve_parameter(levels, "d", "value"),
        Dx9ParameterValue::Missing
    );
}

#[test]
fn a_bitmap_with_no_tag_falls_through_and_kinds_do_not_cross() {
    let shader = level(&[("map", "bitmap", "", 0.0)]);
    let template = level(&[("map", "value", "", 5.0)]);
    let pass = level(&[("map", "bitmap", "default", 0.0)]);
    let levels = [&shader, &template, &pass];
    assert_eq!(
        resolve_parameter(levels, "map", "bitmap"),
        Dx9ParameterValue::Bitmap("default".to_owned())
    );
}

#[test]
fn named_blends_match_the_games_tables() {
    use Dx9BlendFactor::*;
    assert_eq!(named_blend("add"), Some((One, One)));
    assert_eq!(named_blend("multiply"), Some((DstColor, Zero)));
    assert_eq!(named_blend("add src times dstalpha"), Some((DstAlpha, One)));
    assert_eq!(named_blend("add src times srcalpha"), Some((SrcAlpha, One)));
    assert_eq!(
        named_blend("add dst times srcalpha inverse"),
        Some((One, OneMinusSrcAlpha))
    );
    assert_eq!(
        named_blend("alpha blend"),
        Some((SrcAlpha, OneMinusSrcAlpha))
    );
    assert_eq!(named_blend("disabled"), None);
    assert_eq!(blend_equation("logic-op"), None);
}

#[test]
fn rows_and_constants_come_from_the_pass() {
    let pass = Dx9Pass {
        path: String::new(),
        implementation: 0,
        vertex_shader: "rasterizer\\vertex_shaders_dx9\\transparent_generic_default".to_owned(),
        blend: None,
        depth: Dx9Depth {
            compare: None,
            write: false,
        },
        channels: "color only".to_owned(),
        custom_channels: None,
        alpha_test: None,
        textures: Vec::new(),
        parameters: vec![
            (
                "tint".to_owned(),
                Dx9ParameterValue::Color([0.25, 0.5, 0.75]),
            ),
            ("glow".to_owned(), Dx9ParameterValue::Value(0.5)),
            (
                "primary".to_owned(),
                Dx9ParameterValue::Extern("OBJECT change color primary".to_owned()),
            ),
        ],
        vertex_constants: vec![
            Dx9VertexConstant {
                parameter: "map".to_owned(),
                bank: "Cn [0,12]".to_owned(),
                register: 2,
                components: "xyw row1 (2D affine xform)".to_owned(),
            },
            Dx9VertexConstant {
                parameter: "map".to_owned(),
                bank: "Cn [0,12]".to_owned(),
                register: 3,
                components: "xyw row2 (2D affine xform)".to_owned(),
            },
            Dx9VertexConstant {
                parameter: "tint".to_owned(),
                bank: "Vn [0,15]".to_owned(),
                register: 10,
                components: "xyzw".to_owned(),
            },
        ],
        animations: Default::default(),
    };
    let psh = "#const C1.rgb= $tint\n#const C1.a= $glow\n#const C2.rgb= $primary\n\
                   float4 c1 : register(C1);\nfloat4 c2 : register(C2);\n\
                   float4 main(\n)\n{\n    return c1 * c2;\n}\n";
    let drawn =
        prepare_draw_pass(&pass, psh, HashMap::new(), 1.0, |_, _| None, |_| [4.0, 8.0]).unwrap();
    assert_eq!(drawn.stage, Dx9VertexStage::Default);
    assert_eq!(drawn.channels, [true, true, true, false]);
    assert_eq!(
        drawn.constants,
        [(1, [0.25, 0.5, 0.75, 0.5]), (2, [1.0, 1.0, 1.0, 0.0])]
    );
    assert_eq!(drawn.texture_rows[2], [4.0, 0.0, 0.0, 0.0]);
    assert_eq!(drawn.texture_rows[3], [0.0, 8.0, 0.0, 0.0]);
    assert_eq!(drawn.texture_rows[0], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(drawn.tint_facing, [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(drawn.tint_glancing, [1.0; 4]);
    assert_eq!(drawn.change_colors, [(2, 0, 0), (2, 1, 0), (2, 2, 0)]);
}

#[test]
fn tint_color_and_brightness_share_a_register() {
    let pass = Dx9Pass {
        path: String::new(),
        implementation: 0,
        vertex_shader: "rasterizer\\vertex_shaders_dx9\\transparent_generic_default".to_owned(),
        blend: None,
        depth: Dx9Depth {
            compare: None,
            write: false,
        },
        channels: "color only".to_owned(),
        custom_channels: None,
        alpha_test: None,
        textures: Vec::new(),
        parameters: vec![
            (
                "color".to_owned(),
                Dx9ParameterValue::Color([0.5, 0.0, 0.25]),
            ),
            ("bright".to_owned(), Dx9ParameterValue::Value(0.8)),
        ],
        vertex_constants: vec![
            Dx9VertexConstant {
                parameter: "color".to_owned(),
                bank: "Vn [0,15]".to_owned(),
                register: 10,
                components: "xyz/rgb (color)".to_owned(),
            },
            Dx9VertexConstant {
                parameter: "bright".to_owned(),
                bank: "Vn [0,15]".to_owned(),
                register: 10,
                components: "w (value)".to_owned(),
            },
        ],
        animations: Default::default(),
    };
    let drawn = prepare_draw_pass(
        &pass,
        "float4 main(\n)\n{\n    return 0;\n}\n",
        HashMap::new(),
        1.0,
        |_, _| None,
        |_| [1.0, 1.0],
    )
    .unwrap();
    assert_eq!(drawn.tint_facing, [0.5, 0.0, 0.25, 0.8]);
}

#[test]
fn frame_textures_other_than_normalization_are_refused() {
    let pass = |name: &str| Dx9Pass {
        path: String::new(),
        implementation: 3,
        vertex_shader: String::new(),
        blend: None,
        depth: Dx9Depth {
            compare: None,
            write: false,
        },
        channels: String::new(),
        custom_channels: None,
        alpha_test: None,
        textures: vec![Dx9PassTexture {
            parameter: String::new(),
            mode: String::new(),
            value: Dx9ParameterValue::Extern(name.to_owned()),
        }],
        parameters: Vec::new(),
        vertex_constants: Vec::new(),
        animations: Default::default(),
    };
    assert!(h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), false));
    assert!(!h2_pass_needs_frame_texture(&pass(TEXACCUM_TARGET), true));
    assert!(h2_pass_needs_frame_texture(
        &pass("GLOBAL something unsupplied"),
        true
    ));
    // A camera feed draws as a dark screen rather than costing the material.
    assert!(!h2_pass_needs_frame_texture(
        &pass("GLOBAL texture camera"),
        false
    ));
    assert!(!h2_pass_needs_frame_texture(
        &pass("FIRST-PERSON scope"),
        false
    ));
    assert!(!h2_pass_needs_frame_texture(
        &pass(VECTOR_NORMALIZATION),
        false
    ));
}

/// With `BLAM_TEST_H2EK`: a change-color shader fills texaccum on its
/// light albedo layer, the only thing its diffuse pass has to read.
#[test]
fn jackal_legs_diffuse_reads_its_light_albedo() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let definitions = crate::core::bundled::locate_definitions_root();
    let read = |path: &str, extension: &str, group: &[u8; 4]| {
        let file = root.join(format!("{}.{extension}", path.replace('\\', "/")));
        crate::core::source::read_tag_at_path(
            &file,
            Some(crate::core::game::GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*group),
        )
        .ok()
    };
    let shader = read(
        "objects\\characters\\jackal\\shaders\\jackal_legs",
        "shader",
        b"shad",
    )
    .unwrap();
    let layer = |layer: Dx9Layer| {
        resolve_h2_layer_passes(&shader, read, layer.tag_name(), layer.implementation()).unwrap()
    };
    assert!(layer(Dx9Layer::Texaccum).is_empty());
    let albedo = layer(Dx9Layer::LightAlbedo);
    assert_eq!(albedo.len(), 1);
    assert_eq!(
        Dx9VertexStage::from_path(&albedo[0].vertex_shader),
        Some(Dx9VertexStage::Lightmap)
    );
    let diffuse = layer(Dx9Layer::LightDiffuse);
    assert_eq!(diffuse.len(), 1);
    assert!(h2_pass_needs_frame_texture(&diffuse[0], false));
    assert!(!h2_pass_needs_frame_texture(&diffuse[0], true));
}

/// With `BLAM_TEST_H2EK`: the tag's constant is 1, so a resolve that
/// ignores the shader's input would leave the emblem on.
#[test]
fn a_player_emblem_is_off_without_a_player() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let definitions = crate::core::bundled::locate_definitions_root();
    let read = |path: &str, extension: &str, group: &[u8; 4]| {
        let file = root.join(format!("{}.{extension}", path.replace('\\', "/")));
        crate::core::source::read_tag_at_path(
            &file,
            Some(crate::core::game::GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*group),
        )
        .ok()
    };
    for path in [
        "objects\\characters\\masterchief\\shaders\\mc_emblem",
        "objects\\characters\\elite\\shaders\\elite_emblem",
    ] {
        let shader = read(path, "shader", b"shad").unwrap();
        let passes =
            resolve_h2_layer_passes(&shader, read, Dx9Layer::Overlay.tag_name(), 0).unwrap();
        let value = passes[0]
            .parameters
            .iter()
            .find(|(name, _)| name == EMBLEM_INPUT);
        assert_eq!(
            value.map(|(_, value)| value),
            Some(&Dx9ParameterValue::Value(0.0)),
            "{path}"
        );
    }
}

/// With `BLAM_TEST_H2EK`: this pass's pixel shader is solid white, which
/// shows through whatever is translucent over it unless the mask stops it.
#[test]
fn a_depth_prepass_writes_no_color() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let definitions = crate::core::bundled::locate_definitions_root();
    let file = root.join("shaders/shader_passes/z_prepass/z_prepass.shader_pass");
    let pass = crate::core::source::read_tag_at_path(
        &file,
        Some(crate::core::game::GameId::Halo2),
        Some(&definitions),
        u32::from_be_bytes(*b"spas"),
    )
    .unwrap();
    let root = pass.root();
    let implementation = elements(root, "implementations").next().unwrap();
    assert_eq!(read_custom_channels(implementation), Some([false; 4]));
}

/// With `BLAM_TEST_H2EK`: the constant is 1, and reading it instead of
/// the function leaves a parked ghost boosting.
#[test]
fn an_idle_ghost_is_not_boosting() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let definitions = crate::core::bundled::locate_definitions_root();
    let file = root.join("objects/vehicles/ghost/shaders/plasma_vent.shader");
    let shader = crate::core::source::read_tag_at_path(
        &file,
        Some(crate::core::game::GameId::Halo2),
        Some(&definitions),
        u32::from_be_bytes(*b"shad"),
    )
    .unwrap();
    let root = shader.root();
    let levels = shader_parameters(root);
    let animations = &levels["framebuffer_fade"].animations;
    assert_eq!(
        animated_vector(animations, 3.0.into()).map(|value| value[0]),
        Some(0.0)
    );
}

/// With `BLAM_TEST_H2EK`: the brute hair that drew untextured.
#[test]
fn brute_hair_resolves_from_the_kit() {
    let Some(root) = kit_root(GameId::Halo2) else {
        return;
    };
    let definitions = crate::core::bundled::locate_definitions_root();
    let read = |path: &str, extension: &str, group: &[u8; 4]| {
        let file = root.join(format!("{}.{extension}", path.replace('\\', "/")));
        crate::core::source::read_tag_at_path(
            &file,
            Some(crate::core::game::GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*group),
        )
        .ok()
    };
    let shader = read(
        "objects\\characters\\brute\\shaders\\brute_hair",
        "shader",
        b"shad",
    )
    .unwrap();
    let passes =
        resolve_h2_layer_passes(&shader, read, Dx9Layer::Transparent.tag_name(), 0).unwrap();
    assert_eq!(passes.len(), 1);
    let pass = &passes[0];
    let blend = pass.blend.unwrap();
    assert_eq!(blend.source, Dx9BlendFactor::One);
    assert_eq!(blend.destination, Dx9BlendFactor::OneMinusSrcAlpha);
    assert_eq!(
        pass.depth,
        Dx9Depth {
            compare: Some(Dx9Compare::LessOrEqual),
            write: false
        }
    );
    assert_eq!(pass.vertex_constants.len(), 4);
    assert_eq!(pass.vertex_constants[2].parameter, "change_color_map");
    assert_eq!(pass.vertex_constants[2].register, 2);
    assert_eq!(pass.channels, "color only");
    assert_eq!(pass.alpha_test, None);
    assert_eq!(pass.textures.len(), 2);
    assert_eq!(pass.textures[0].parameter, "alpha_blend_map");
    for texture in &pass.textures {
        assert!(
            matches!(&texture.value, Dx9ParameterValue::Bitmap(path) if path.contains("brute")),
            "{texture:?}"
        );
    }
    let extern_of = |name: &str| {
        pass.parameters
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        extern_of("change_color_primary"),
        Some(Dx9ParameterValue::Extern(
            "OBJECT change color primary".to_owned()
        ))
    );
    assert_eq!(
        extern_of("global_light_hack"),
        Some(Dx9ParameterValue::Extern(
            "LIGHTMAP object direct color".to_owned()
        ))
    );

    let custom = "shaders\\shader_passes\\transparent\\alpha_detail_masked";
    let tag = read(custom, "shader_pass", b"spas").unwrap();
    let blend = read_pass(
        custom,
        tag.root(),
        0,
        &Parameters::new(),
        &Parameters::new(),
    )
    .unwrap()
    .blend
    .unwrap();
    assert_eq!(blend.source, Dx9BlendFactor::ConstantColor);
    assert_eq!(
        blend.constant,
        [0, 255, 255, 255],
        "custom blends carry their constant"
    );

    let file = root
        .join("rasterizer/pixel_shaders_dx9")
        .join(pass.pixel_shader_file());
    let source = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
    let translated = crate::core::h2_pixel_shader::translate(
        &source,
        "#version 330\n",
        &crate::core::h2_pixel_shader::PixelShaderOptions::default(),
    )
    .unwrap();
    assert_eq!(translated.samplers.len(), 2);

    let drawn = prepare_draw_pass(
        pass,
        &source,
        HashMap::new(),
        1.0,
        |path, _| {
            read(path, "bitmap", b"bitm")?;
            Some(std::sync::Arc::new(Image::Flat(TextureImage {
                rgba: vec![0; 4],
                width: 1,
                height: 1,
                repeat_x: true,
                repeat_y: true,
                scale: [1.0, 1.0],
            })))
        },
        |_| [1.0, 1.0],
    )
    .unwrap();
    assert_eq!(
        drawn.stage,
        Dx9VertexStage::from_path(&pass.vertex_shader).unwrap()
    );
    assert!(!drawn.change_colors.is_empty());
    let object = read("objects\\characters\\brute\\brute", "biped", b"bipd").unwrap();
    let colors = read_change_colors(&object);
    assert!(
        colors.iter().any(|channel| !channel.is_empty()),
        "the brute has change colors"
    );
    assert_eq!(drawn.textures.len(), 2);
    assert!(
        drawn.textures.iter().all(|texture| texture.image.is_some()),
        "both maps decode"
    );
    assert!(!drawn.constants.is_empty());
    for (register, value) in &drawn.constants {
        assert!(
            value.iter().any(|component| *component != 0.0),
            "c{register} is unset: {drawn:?}"
        );
    }
}
