//! The glass shader's passes: tint, reflection and diffuse.

use super::*;

fn lit_rows(scale: [f32; 4]) -> [[f32; 4]; H2_TEXTURE_ROWS] {
    let vector = |[x, y, z]: [f32; 3]| [x, y, z, 0.0];
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[2] = scale;
    rows[4] = vector(AMBIENT);
    rows[5] = vector(FIRST_LIGHT_COLOR);
    rows[6] = vector(SECOND_LIGHT_DIRECTION);
    rows[7] = vector(SECOND_LIGHT_COLOR);
    rows
}

/// `Dx9VertexStage::Bumped`'s rows for one map at `scale`.
pub(super) fn bumped_rows(scale: f32) -> [[f32; 4]; H2_TEXTURE_ROWS] {
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [1.0, 1.0, 0.0, 0.0];
    rows[2] = [scale, scale, 0.0, 0.0];
    rows
}

/// `rasterizer_glass_draw_tint`, `_reflection` and `_diffuse`, in that order.
pub(super) fn glass_passes(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<Image>>,
) -> Result<Vec<Dx9DrawPass>, String> {
    let read = |name: &str| read_shader_source(directory, name);
    let has = |flag: &str| flags.has(flag);
    let two_sided = has("two-sided");
    let path = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
    let mut map = |name: &str, kind| path(name).and_then(|path| bitmap(&path, kind));
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let scale = |name: &str| nonzero(root.read_real(name), 1.0);
    let mut passes = Vec::new();

    let tint = rgb("background tint color");
    let tint_map = map("background tint map", SamplerKind::Texture2D);
    if tint_map.is_some() || tint != [0.0; 3] {
        let shader = translate_fx(GLASS_TINT_FILE, H2_GLSL_VERSION, &[], read)?;
        let register = shader.register("c_tint_and_intensity").unwrap_or(0);
        let size = scale("background tint map scale");
        passes.push(
            SimplePass {
                stage: Dx9VertexStage::CeGlassLit,
                fragment: shader.entry("pixel_main(i)"),
                blend: plain_blend(Dx9BlendFactor::Zero, Dx9BlendFactor::SrcColor),
                two_sided,
                write_depth: false,
                channels: [true, true, true, false],
                textures: vec![texture(
                    0,
                    SamplerKind::Texture2D,
                    tint_map.or_else(|| Some(flat(WHITE))),
                )],
                constants: vec![(register, [tint[0], tint[1], tint[2], 1.0])],
                rows: lit_rows([size, size, 0.0, 0.0]),
            }
            .build(),
        );
    }

    let perpendicular = real("perpendicular brightness");
    let parallel = real("parallel brightness");
    let reflection = map("reflection map", SamplerKind::Cube);
    let reflection_type = option_index(
        root,
        "reflection type",
        &["bumped cube-map", "flat cube-map", "dynamic mirror"],
    );
    if (perpendicular > 0.0 || parallel > 0.0) && reflection.is_some() && reflection_type != 2 {
        let bump = map("bump map", SamplerKind::Texture2D);
        let specular_mask = has("bump map is specular mask");
        let flat_surface = reflection_type != 0 || specular_mask || bump.is_none();
        let file = if flat_surface {
            GLASS_FLAT_FILE
        } else {
            GLASS_BUMPED_FILE
        };
        let shader = translate_fx(file, H2_GLSL_VERSION, &[], read)?;
        let mut constants = Vec::new();
        let mut constant = |name: &str, value: [f32; 4]| {
            if let Some(register) = shader.register(name) {
                constants.push((register, value));
            }
        };
        let [red, green, blue] = rgb("perpendicular tint color");
        constant(
            "c_view_perpendicular_color",
            [red, green, blue, perpendicular],
        );
        let [red, green, blue] = rgb("parallel tint color");
        constant("c_view_parallel_color", [red, green, blue, parallel]);
        let truth = |on: bool| f32::from(u8::from(on));
        constant(
            "params",
            [1.0, truth(bump.is_none()), truth(specular_mask), 0.0],
        );
        constant("c_eye_forward", [0.0; 4]);
        let size = real("bump map scale");
        let size = if size == 0.0 { 1.0 } else { size };
        let mut draw = SimplePass {
            stage: if flat_surface {
                Dx9VertexStage::CeGlassFlat
            } else {
                Dx9VertexStage::Bumped
            },
            fragment: shader.entry("pixel_main(i)"),
            blend: plain_blend(Dx9BlendFactor::SrcAlpha, Dx9BlendFactor::One),
            two_sided,
            write_depth: false,
            channels: [true, true, true, false],
            textures: vec![
                texture(
                    0,
                    SamplerKind::Texture2D,
                    bump.or_else(|| Some(flat_normal())),
                ),
                normalization(1),
                normalization(2),
                texture(3, SamplerKind::Cube, reflection),
            ],
            constants,
            rows: if flat_surface {
                lit_rows([size, size, 0.0, 0.0])
            } else {
                bumped_rows(size)
            },
        }
        .build();
        draw.ce_animation = Some(Arc::new(CeModelAnimation {
            eye_register: shader.register("c_eye_forward"),
            ..Default::default()
        }));
        passes.push(draw);
    }

    let diffuse = map("diffuse map", SamplerKind::Texture2D);
    let detail = map("diffuse detail map", SamplerKind::Texture2D);
    if diffuse.is_some() || detail.is_some() {
        let shader = translate_fx(GLASS_DIFFUSE_FILE, H2_GLSL_VERSION, &[], read)?;
        let (base, fine) = (
            scale("diffuse map scale"),
            scale("diffuse detail map scale"),
        );
        passes.push(
            SimplePass {
                stage: Dx9VertexStage::CeGlassLit,
                fragment: shader.entry("main_T0_P0(i)"),
                blend: plain_blend(Dx9BlendFactor::SrcAlpha, Dx9BlendFactor::OneMinusSrcAlpha),
                two_sided,
                write_depth: false,
                channels: [true, true, true, false],
                textures: vec![
                    texture(
                        0,
                        SamplerKind::Texture2D,
                        diffuse.or_else(|| Some(flat(WHITE))),
                    ),
                    texture(
                        1,
                        SamplerKind::Texture2D,
                        detail.or_else(|| Some(flat(GREY))),
                    ),
                ],
                constants: Vec::new(),
                rows: lit_rows([base, base, fine, fine]),
            }
            .build(),
        );
    }
    // A dynamic mirror alone: there is nothing here for it to reflect.
    if passes.is_empty() {
        passes.push(unseen_pass());
    }
    for pass in &mut passes {
        pass.hidden_by_camo = true;
        pass.depth_bias = decal_bias(flags.has("decal"));
    }
    Ok(passes)
}

/// The depth bias of a transparent shader by its `decal` flag.
pub(super) fn decal_bias(decal: bool) -> f32 {
    if decal {
        TRANSPARENT_DECAL_DEPTH_BIAS
    } else {
        0.0
    }
}
