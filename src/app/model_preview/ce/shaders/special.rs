//! The meter, water and plasma shaders' passes, and the pass for a shader nothing draws.

use super::*;

/// The `shader_transparent_meter` case of
/// `rasterizer_transparent_geometry_group_draw`.
pub(super) fn meter_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Dx9DrawPass, String> {
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let has = |flag: &str| flags.has(flag);
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let meter = CeMeter {
        sources: [
            function_source(root, "meter brightness source"),
            function_source(root, "flash brightness source"),
            function_source(root, "value source"),
            function_source(root, "flash-extension source"),
        ],
        flash: rgb("flash color"),
        maximum: rgb("gradient max color"),
        minimum: rgb("gradient min color"),
        background: rgb("background color"),
        tint: rgb("tint color"),
        transparency: has("tint mode-2")
            .then(|| [real("meter transparency"), real("background transparency")]),
        negative: has("flash color is negative"),
    };
    let image = root
        .read_tag_ref_path("map")
        .filter(|path| !path.is_empty())
        .and_then(|path| bitmap(&path, SamplerKind::Texture2D));
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(METER_FILE, H2_GLSL_VERSION, &[], read)?;
    let mut pass = SimplePass {
        stage: Dx9VertexStage::CeMeter,
        fragment: shader.entry("pixel_main(i)"),
        blend: plain_blend(Dx9BlendFactor::One, Dx9BlendFactor::One),
        two_sided: has("two-sided"),
        write_depth: false,
        channels: [true, true, true, false],
        textures: vec![texture(
            0,
            SamplerKind::Texture2D,
            image.or_else(|| Some(flat(WHITE))),
        )],
        constants: (0..6).map(|register| (register, [0.0; 4])).collect(),
        rows: [[0.0; 4]; H2_TEXTURE_ROWS],
    }
    .build();
    pass.ce_animation = Some(Arc::new(CeModelAnimation {
        meter: Some(meter),
        ..Default::default()
    }));
    Ok(pass)
}

/// `rasterizer_water_draw`, without its ripples: the game composes those into
/// a target each frame, which this draws flat.
pub(super) fn water_passes(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Vec<Dx9DrawPass>, String> {
    let read = |name: &str| read_shader_source(directory, name);
    let has = |flag: &str| flags.has(flag);
    let path = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
    let base = path("base map")
        .and_then(|path| bitmap(&path, SamplerKind::Texture2D))
        .or_else(|| Some(flat(WHITE)));
    let reflection = path("reflection map").and_then(|path| bitmap(&path, SamplerKind::Cube));
    let perpendicular = root
        .read_real("view perpendicular brightness")
        .unwrap_or(0.0);
    let parallel = nonzero(root.read_real("view parallel brightness"), 1.0);
    let modulates_reflection = has("base map alpha modulates reflection");
    let opacity = translate_fx(WATER_OPACITY_FILE, H2_GLSL_VERSION, &[], read)?;
    let constants = vec![(0, [perpendicular; 4]), (1, [parallel; 4]), (2, [0.0; 4])];
    let mut passes = Vec::new();
    let surface = |call: &str, blend, channels, textures, constants| {
        SimplePass {
            stage: Dx9VertexStage::CeWaterOpacity,
            fragment: opacity.entry(call),
            blend,
            two_sided: true,
            write_depth: false,
            channels,
            textures,
            constants,
            rows: [[0.0; 4]; H2_TEXTURE_ROWS],
        }
        .build()
    };
    if modulates_reflection {
        passes.push(surface(
            "main_T0_P0(i)",
            plain_blend(Dx9BlendFactor::One, Dx9BlendFactor::Zero),
            [false, false, false, true],
            vec![
                texture(0, SamplerKind::Texture2D, base.clone()),
                normalization(1),
            ],
            constants.clone(),
        ));
    }
    if has("base map color modulates background") {
        passes.push(surface(
            "main_T0_P1(i)",
            plain_blend(Dx9BlendFactor::Zero, Dx9BlendFactor::SrcColor),
            [true, true, true, false],
            vec![texture(0, SamplerKind::Texture2D, base), normalization(1)],
            constants,
        ));
    }
    let shader = translate_fx(WATER_REFLECTION_FILE, H2_GLSL_VERSION, &[], read)?;
    let source = if modulates_reflection {
        Dx9BlendFactor::DstAlpha
    } else {
        Dx9BlendFactor::One
    };
    passes.push(
        SimplePass {
            stage: Dx9VertexStage::Bumped,
            fragment: shader.entry("pixel_main(i)"),
            blend: plain_blend(source, Dx9BlendFactor::One),
            two_sided: true,
            write_depth: true,
            channels: [true, true, true, false],
            textures: vec![
                texture(0, SamplerKind::Texture2D, Some(flat_normal())),
                texture(3, SamplerKind::Cube, reflection),
            ],
            constants: vec![(0, [1.0, 1.0, 1.0, 0.0]), (1, [0.0; 4])],
            rows: bumped_rows(1.0),
        }
        .build(),
    );
    Ok(passes)
}

/// `rasterizer_plasma_energy_draw`. All the kit's are shield hits, unseen until
/// `recent shield damage` raises their intensity. `tint color source` is none
/// on all of them, so it is not read.
pub(super) fn plasma_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Dx9DrawPass, String> {
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let tinted = |brightness: &str, tint: &str| {
        let color = root.read_rgb(tint);
        [color.red, color.green, color.blue, real(brightness)]
    };
    let noise = |name: &str| {
        let direction = root.read_vec3(&format!("{name} animation direction"));
        CePlasmaNoise {
            period: nonzero(root.read_real(&format!("{name} animation period")), 1.0),
            direction: [direction.i, direction.j, direction.k],
            scale: real(&format!("{name} noise map scale")),
        }
    };
    let plasma = CePlasma {
        intensity_source: function_source(root, "intensity source"),
        intensity_exponent: nonzero(root.read_real("intensity exponent"), 1.0),
        offset_source: function_source(root, "offset source"),
        offset_exponent: nonzero(root.read_real("offset exponent"), 1.0),
        offset_amount: real("offset amount"),
        perpendicular: tinted("perpendicular brightness", "perpendicular tint color"),
        parallel: tinted("parallel brightness", "parallel tint color"),
        primary: noise("primary"),
        secondary: noise("secondary"),
    };
    let mut volume = |slot, name: &str| {
        let image = root
            .read_tag_ref_path(name)
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, SamplerKind::Volume));
        texture(slot, SamplerKind::Volume, image)
    };
    let textures = vec![
        volume(0, "primary noise map"),
        volume(1, "secondary noise map"),
    ];
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(PLASMA_FILE, H2_GLSL_VERSION, &[], read)?;
    let mut pass = SimplePass {
        stage: Dx9VertexStage::CePlasma,
        fragment: shader.entry("pixel_main(i)"),
        blend: plain_blend(Dx9BlendFactor::SrcAlpha, Dx9BlendFactor::One),
        two_sided: true,
        write_depth: false,
        channels: [true, true, true, false],
        textures,
        constants: Vec::new(),
        rows: [[0.0; 4]; H2_TEXTURE_ROWS],
    }
    .build();
    pass.ce_animation = Some(Arc::new(CeModelAnimation {
        plasma: Some(plasma),
        ..Default::default()
    }));
    Ok(pass)
}

/// A pass that draws nothing, for a surface with nothing to show a lone model.
pub(super) fn unseen_pass() -> Dx9DrawPass {
    let fragment = format!(
        "{H2_GLSL_VERSION}in vec4 v_color0;\nout vec4 out_color;\nvoid main() {{ discard; }}\n"
    );
    let depth = Dx9Depth {
        compare: Some(Dx9Compare::LessOrEqual),
        write: false,
    };
    let mut pass = Dx9DrawPass::new(Dx9VertexStage::CeMeter, fragment, depth);
    pass.channels = [false; 4];
    pass
}
