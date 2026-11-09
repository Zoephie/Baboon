//! The model shaders' passes (`soso`, `senv`).

use super::*;

/// `model_common.h` translated, or `None` where the kit has no shaders.
pub(in crate::app::model_preview) type CeModelCommon = Arc<TranslatedFxShader>;

/// The kit's `shaders/fx` folder, beside `tags`.
pub(super) fn read_common(reader: &TagReader) -> Option<CeModelCommon> {
    let directory = shader_directory(reader)?;
    let read = |name: &str| read_shader_source(&directory, name);
    match translate_fx(COMMON_FILE, H2_GLSL_VERSION, &[], read) {
        Ok(shader) => Some(Arc::new(shader)),
        Err(error) => {
            eprintln!("model preview: {COMMON_FILE}: {error}");
            None
        }
    }
}

/// A `real_argb_color` of tag fields: brightness, then a color.
struct ReflectionColor {
    brightness: f32,
    color: [f32; 3],
}

/// What `ShaderModel` is called with and fed, whichever group asked.
struct ModelPass<'a> {
    environment: bool,
    detail_mask: i32,
    detail_function: i32,
    detail_before_reflection: bool,
    alpha_test: bool,
    decal: bool,
    two_sided: bool,
    base: Option<Arc<H2Image>>,
    detail: Option<Arc<H2Image>>,
    multipurpose: Option<Arc<H2Image>>,
    reflection: Option<Arc<H2Image>>,
    base_scale: [f32; 2],
    detail_scale: [f32; 2],
    self_illumination: [f32; 4],
    change_color: Option<usize>,
    animation: Option<CeModelAnimation>,
    translucency: f32,
    reflection_distances: [f32; 2],
    perpendicular: ReflectionColor,
    parallel: ReflectionColor,
    common: &'a TranslatedFxShader,
}

fn tinted(color: &ReflectionColor) -> [f32; 4] {
    let [alpha, red, green, blue] = REFLECTION_TINT;
    [
        color.color[0] * red,
        color.color[1] * green,
        color.color[2] * blue,
        color.brightness * alpha,
    ]
}

fn build_passes(pass: ModelPass<'_>) -> Vec<H2DrawPass> {
    let common = pass.common;
    let fragment = common.entry(&format!(
        "ShaderModel(i, {}, {}, {}, {}, false)",
        i32::from(pass.environment),
        pass.detail_mask,
        pass.detail_function,
        pass.detail_before_reflection,
    ));
    let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: !pass.decal };
    let mut draw = H2DrawPass::new(H2VertexStage::CeModel, fragment, depth);
    draw.channels = [true, true, true, false];
    if pass.decal {
        draw.depth_bias = MODEL_DECAL_DEPTH_BIAS;
        draw.blend = Some(H2Blend {
            equation: H2BlendEquation::Add,
            source: H2BlendFactor::SrcAlpha,
            destination: H2BlendFactor::OneMinusSrcAlpha,
            constant: [0; 4],
        });
    }
    draw.textures = vec![
        texture(0, SamplerKind::Texture2D, pass.base.or_else(|| Some(flat(WHITE)))),
        texture(1, SamplerKind::Texture2D, pass.detail.or_else(|| Some(flat(GREY)))),
        texture(2, SamplerKind::Texture2D, pass.multipurpose.or_else(|| Some(flat(WHITE)))),
        texture(3, SamplerKind::Cube, pass.reflection),
    ];
    let mut animation = pass.animation;
    if let (Some(animation), Some(register)) = (&mut animation, common.register("c_self_illumination_color")) {
        animation.colors.iter_mut().for_each(|color| color.register = register);
    }
    if let (Some(channel), Some(register)) = (pass.change_color, common.register("c_primary_change_color")) {
        draw.change_colors = (0..3).map(|component| (register, component, channel)).collect();
    }
    let mut constant = |name: &str, value: [f32; 4]| {
        if let Some(register) = common.register(name) {
            draw.constants.push((register, value));
        }
    };
    constant("c_primary_change_color", [1.0; 4]);
    // With no fog: the sky's share is none and the scene's all.
    constant("c_fog_color_correction_0", [0.0, 0.0, 0.0, 1.0]);
    constant("c_fog_color_correction_E", [0.0; 4]);
    constant("c_fog_color_correction_1", [0.0; 4]);
    constant("c_self_illumination_color", pass.self_illumination);
    constant("c_alpha_ref", [if pass.alpha_test { ALPHA_REFERENCE } else { 0.0 }; 4]);
    constant("c_fog_color", [0.0; 4]);

    let vector = |[x, y, z]: [f32; 3]| [x, y, z, 0.0];
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [pass.base_scale[0], 0.0, 0.0, 0.0];
    rows[1] = [0.0, pass.base_scale[1], 0.0, 0.0];
    rows[2] = [pass.detail_scale[0], pass.detail_scale[1], 0.0, 0.0];
    rows[3] = [pass.translucency, 1.0, 0.0, 0.0];
    rows[4] = vector(AMBIENT);
    rows[5] = vector(FIRST_LIGHT_COLOR);
    rows[6] = vector(SECOND_LIGHT_DIRECTION);
    rows[7] = vector(SECOND_LIGHT_COLOR);
    let [falloff, cutoff] = pass.reflection_distances;
    rows[8] = [cutoff, falloff, 0.0, 0.0];
    draw.texture_rows = rows;
    draw.light_direct = FIRST_LIGHT_DIRECTION;
    draw.ce_animation = animation.map(|animation| Arc::new(animation) as Arc<dyn PassAnimation>);

    let perpendicular = tinted(&pass.perpendicular);
    let parallel = tinted(&pass.parallel);
    draw.tint_facing = std::array::from_fn(|at| perpendicular[at] - parallel[at]);
    draw.tint_glancing = parallel;
    if !pass.two_sided {
        return vec![draw];
    }
    // The game draws the far side again, with its normals turned to face.
    let mut back = draw.clone();
    draw.cull = H2Cull::Back;
    back.cull = H2Cull::Front;
    back.texture_rows[3][1] = -1.0;
    vec![draw, back]
}

pub(super) const LAYER_GROUPS: [(&str, &[u8; 4]); 3] = [
    ("shader_transparent_generic", b"sotr"),
    ("shader_transparent_chicago", b"schi"),
    ("shader_transparent_chicago_extended", b"scex"),
];

/// The passes of a `shader_model` or `shader_environment`, empty for any other
/// group and where the kit has no shaders, which keeps the slots drawing.
pub(in crate::app::model_preview) fn ce_model_passes(
    reader: &TagReader,
    root: TagStruct<'_>,
    group: &[u8; 4],
    flags: &ShaderFlags<'_>,
    common: &mut Option<Option<CeModelCommon>>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Vec<H2DrawPass> {
    if !matches!(group, b"soso" | b"senv") {
        return Vec::new();
    }
    let Some(common) = common.get_or_insert_with(|| read_common(reader)).clone() else {
        return Vec::new();
    };
    let has = |flag: &str| flags.has(flag);
    let factor = |value: Option<f32>| value.filter(|v| v.is_finite() && *v != 0.0).unwrap_or(1.0);
    let mut map = |name: &str, kind| {
        root.read_tag_ref_path(name)
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, kind))
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let pass = if group == b"soso" {
        let detail_mask = option_index(root, "detail mask", &super::super::super::ce::shaders::resolve::CE_DETAIL_MASKS);
        let decal = has("alpha-blended decal");
        let scale = factor(root.read_real("detail map scale"));
        let base_scale = [
            factor(root.read_real("map u-scale")),
            factor(root.read_real("map v-scale")),
        ];
        let xbox_order = if has("multipurpose map uses OG Xbox channel order") { 1.0 } else { 0.0 };
        ModelPass {
            environment: false,
            detail_mask,
            detail_function: option_index(root, "detail function", &DETAIL_FUNCTIONS),
            detail_before_reflection: !has("detail after reflection"),
            alpha_test: !decal && !has("not alpha-tested"),
            decal,
            two_sided: has("two-sided"),
            base: map("base map", SamplerKind::Texture2D),
            detail: map("detail map", SamplerKind::Texture2D),
            multipurpose: map("multipurpose map", SamplerKind::Texture2D),
            reflection: map("reflection cube map", SamplerKind::Cube),
            base_scale,
            detail_scale: [scale, scale * factor(root.read_real("detail map v-scale"))],
            self_illumination: [0.0, 0.0, 0.0, xbox_order],
            change_color: color_channel(root, "change color source"),
            animation: Some(read_model_animation(root, base_scale, detail_mask)),
            translucency: real("translucency"),
            reflection_distances: [
                real("reflection falloff distance"),
                real("reflection cutoff distance"),
            ],
            perpendicular: ReflectionColor {
                brightness: real("perpendicular brightness"),
                color: rgb("perpendicular tint color"),
            },
            parallel: ReflectionColor {
                brightness: real("parallel brightness"),
                color: rgb("parallel tint color"),
            },
            common: &common,
        }
    } else {
        let alpha_tested = has("alpha-tested");
        let scale = factor(root.read_real("primary detail map scale"));
        ModelPass {
            environment: true,
            detail_mask: 0,
            detail_function: option_index(root, "detail map function", &DETAIL_FUNCTIONS),
            detail_before_reflection: true,
            alpha_test: alpha_tested,
            decal: false,
            two_sided: false,
            base: map("base map", SamplerKind::Texture2D),
            detail: map("primary detail map", SamplerKind::Texture2D),
            multipurpose: if alpha_tested {
                map("bump map", SamplerKind::Texture2D)
            } else {
                None
            },
            reflection: map("reflection cube map", SamplerKind::Cube),
            base_scale: [1.0; 2],
            detail_scale: [scale; 2],
            self_illumination: [0.0; 4],
            change_color: None,
            animation: None,
            translucency: 0.0,
            reflection_distances: [0.0; 2],
            // The reflection takes its colors from the specular section.
            perpendicular: ReflectionColor {
                brightness: real("perpendicular brightness"),
                color: rgb("perpendicular color"),
            },
            parallel: ReflectionColor {
                brightness: real("parallel brightness"),
                color: rgb("parallel color"),
            },
            common: &common,
        }
    };
    build_passes(pass)
}
