//! Halo 4's particles on the GPU (`c_particle_emitter_gpu::render`, X360
//! 0x82A88A30, MCC 0x1408E5560): each emitter's packed particles in a buffer
//! texture, one instanced four-vertex strip each, through the particle's
//! material with `SetMaterialRenderState`'s constants. See
//! `docs/h4-preview-research/particles.md`.

use std::collections::HashMap;

use super::h3_particles::H4EmitterDraw;
use super::h3_shaders::{H3Pass, H3Value};
use super::h4_particles::{H4ParticleRender, custom_vert_registers};
use super::*;
use crate::core::hlsl::parse::{Scalar, TextureKind, Type};

/// `effect_lightprobe_setup_lighting_constants` with the default probe
/// (`particles.md` §4): bright direction, bright and dark colours, ambient.
const DEFAULT_LIGHTING: [[f32; 3]; 4] = [
    [0.0, 0.0, 1.0],
    [0.413_803, 0.381_972, 0.381_972],
    [1.177_747, 1.145_916, 1.145_916],
    [0.5, 0.48, 0.48],
];

/// The system postprocess's far fade when the tag has none:
/// range at its floor, the cutoff 10000.
const NO_FAR_FADE: [f32; 2] = [1e-4, 10_000.0];

/// The GL a slot keeps for its Halo 4 particles: programs by pass, textures
/// by particle tag and sampler, and the buffer the states go through.
#[derive(Default)]
pub(super) struct H4ParticleGl {
    programs: HashMap<
        usize,
        Option<(
            glow::NativeProgram,
            HashMap<String, Option<glow::NativeUniformLocation>>,
        )>,
    >,
    textures: HashMap<(usize, String), Option<glow::NativeTexture>>,
    buffer: Option<(glow::NativeBuffer, glow::NativeTexture)>,
    vertex_array: Option<glow::NativeVertexArray>,
}

impl H4ParticleGl {
    pub(super) unsafe fn clear(&mut self, gl: &glow::Context) {
        unsafe {
            for (program, _) in self.programs.drain().filter_map(|(_, known)| known) {
                gl.delete_program(program);
            }
            for texture in self.textures.drain().filter_map(|(_, known)| known) {
                gl.delete_texture(texture);
            }
        }
    }

    unsafe fn program(&mut self, gl: &glow::Context, pass: &H3Pass) -> Option<glow::NativeProgram> {
        let key = pass as *const H3Pass as usize;
        if let Some(known) = self.programs.get(&key) {
            return known.as_ref().map(|(program, _)| *program);
        }
        let linked = unsafe {
            (|| {
                let program = gl.create_program().ok()?;
                for (kind, text) in [
                    (glow::VERTEX_SHADER, &pass.vertex),
                    (glow::FRAGMENT_SHADER, &pass.pixel),
                ] {
                    let shader = gl.create_shader(kind).ok()?;
                    gl.shader_source(shader, text);
                    gl.compile_shader(shader);
                    if !gl.get_shader_compile_status(shader) {
                        eprintln!(
                            "h4 particle shader failed: {}",
                            gl.get_shader_info_log(shader)
                        );
                    }
                    gl.attach_shader(program, shader);
                    gl.delete_shader(shader);
                }
                gl.link_program(program);
                if !gl.get_program_link_status(program) {
                    eprintln!(
                        "h4 particle program failed: {}",
                        gl.get_program_info_log(program)
                    );
                    gl.delete_program(program);
                    return None;
                }
                Some(program)
            })()
        };
        self.programs
            .insert(key, linked.map(|program| (program, HashMap::new())));
        linked
    }

    unsafe fn location(
        &mut self,
        gl: &glow::Context,
        pass: &H3Pass,
        name: &str,
    ) -> Option<glow::NativeUniformLocation> {
        let key = pass as *const H3Pass as usize;
        let (program, locations) = self.programs.get_mut(&key)?.as_mut()?;
        if let Some(found) = locations.get(name) {
            return found.clone();
        }
        let found = unsafe { gl.get_uniform_location(*program, name) };
        locations.insert(name.to_owned(), found.clone());
        found
    }

    /// A sampler's bitmap: a texture array of all its images where the pass
    /// samples an array.
    unsafe fn texture(
        &mut self,
        gl: &glow::Context,
        render: &H4ParticleRender,
        uniform: &str,
        kind: &TextureKind,
    ) -> Option<glow::NativeTexture> {
        let key = (
            render as *const H4ParticleRender as usize,
            uniform.to_owned(),
        );
        if let Some(known) = self.textures.get(&key) {
            return *known;
        }
        let texture = render
            .material
            .textures
            .iter()
            .find(|texture| texture.uniform == uniform);
        let uploaded = texture.and_then(|texture| unsafe {
            if *kind == TextureKind::FlatArray {
                let layers: Vec<&H2Image> = if texture.frames.is_empty() {
                    vec![&*texture.image]
                } else {
                    texture.frames.iter().map(|image| &**image).collect()
                };
                upload_array(gl, &layers, texture.texels, texture.sampler)
            } else {
                h3_draw::upload_h2_image_linear(
                    gl,
                    &texture.image,
                    texture.texels,
                    &texture.mips,
                    texture.sampler,
                )
            }
        });
        self.textures.insert(key, uploaded);
        uploaded
    }

    /// The states, a word to a texel of an `R32UI` buffer texture.
    unsafe fn upload_states(
        &mut self,
        gl: &glow::Context,
        states: &[[u32; 20]],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if self.buffer.is_none() {
                let buffer = gl.create_buffer().ok()?;
                let texture = gl.create_texture().ok()?;
                self.buffer = Some((buffer, texture));
            }
            let (buffer, texture) = self.buffer?;
            gl.bind_buffer(glow::TEXTURE_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(glow::TEXTURE_BUFFER, slice_bytes(states), glow::STREAM_DRAW);
            gl.bind_texture(glow::TEXTURE_BUFFER, Some(texture));
            gl.tex_buffer(glow::TEXTURE_BUFFER, glow::R32UI, Some(buffer));
            gl.bind_buffer(glow::TEXTURE_BUFFER, None);
            Some(texture)
        }
    }
}

/// Every image as a layer of one texture array, all of the first's size.
unsafe fn upload_array(
    gl: &glow::Context,
    layers: &[&H2Image],
    texels: h3_shaders::H3Texels,
    sampler: Option<h3_shaders::H3Sampler>,
) -> Option<glow::NativeTexture> {
    let flats: Vec<&TextureImage> = layers
        .iter()
        .filter_map(|image| match image {
            H2Image::Flat(image) => Some(image),
            _ => None,
        })
        .collect();
    let first = flats.first()?;
    let (width, height) = (first.width, first.height);
    if width == 0
        || height == 0
        || flats.iter().any(|image| {
            image.width != width
                || image.height != height
                || image.rgba.len() != (width * height * 4) as usize
        })
    {
        return None;
    }
    let format = match texels {
        h3_shaders::H3Texels::Srgb => glow::SRGB8_ALPHA8,
        _ => glow::RGBA8,
    };
    let mut rgba = Vec::with_capacity(flats.len() * (width * height * 4) as usize);
    for image in &flats {
        rgba.extend_from_slice(&image.rgba);
    }
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
        gl.tex_image_3d(
            glow::TEXTURE_2D_ARRAY,
            0,
            format as i32,
            width as i32,
            height as i32,
            flats.len() as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&rgba)),
        );
        gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
        gl.tex_parameter_i32(
            glow::TEXTURE_2D_ARRAY,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D_ARRAY,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        if let Some(sampler) = sampler {
            h3_draw::h3_apply_sampler(gl, glow::TEXTURE_2D_ARRAY, sampler);
        }
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        Some(texture)
    }
}

/// `set_alpha_blend_mode_custom_device_no_cache` (X360 0x82BCCEC8) for the
/// modes particles use; the packed states of the rest are not decoded, and
/// draw as `alpha_blend`.
unsafe fn set_blend(gl: &glow::Context, mode: u8) {
    unsafe {
        gl.enable(glow::BLEND);
        let (source, destination, equation) = match mode {
            0 => {
                gl.disable(glow::BLEND);
                return;
            }
            1 | 24 => (glow::ONE, glow::ONE, glow::FUNC_ADD),
            2 => (glow::DST_COLOR, glow::ZERO, glow::FUNC_ADD),
            5 | 7 => (glow::ONE, glow::ONE_MINUS_SRC_ALPHA, glow::FUNC_ADD),
            6 => (glow::ONE, glow::ONE, glow::MAX),
            8 => (glow::DST_ALPHA, glow::ONE, glow::FUNC_ADD),
            9 => (glow::SRC_ALPHA, glow::ONE, glow::FUNC_ADD),
            10 => (glow::ONE_MINUS_SRC_ALPHA, glow::SRC_ALPHA, glow::FUNC_ADD),
            17 => (glow::ONE, glow::ONE, glow::MIN),
            18 => (glow::ONE, glow::ONE, glow::FUNC_REVERSE_SUBTRACT),
            _ => (glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::FUNC_ADD),
        };
        gl.blend_equation(equation);
        gl.blend_func(source, destination);
    }
}

/// `SetAlphaBlendShaderConstants` (X360 0x82BC8728) with no atmosphere:
/// z on for the multiplies, w turned for `alpha_blend` and its additive kin.
fn blend_constant(mode: u8) -> [f32; 4] {
    let z = if matches!(mode, 2 | 4 | 7) { 1.0 } else { 0.0 };
    let w = if matches!(mode, 3 | 24) { -1.0 } else { 1.0 };
    [0.0, 1.0, z, w]
}

/// Draw `draws` of one particle system, whose look is `render`, into the
/// bound light target. `depth` is a copy of the scene's depth for the fade.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_h4_particles(
    gl: &glow::Context,
    state: &mut H4ParticleGl,
    frame: &ModelGpuFrame,
    render: &H4ParticleRender,
    draws: &[H4EmitterDraw],
    depth: Option<glow::NativeTexture>,
    size: [i32; 2],
    placeholder: Option<glow::NativeTexture>,
    object: u8,
) {
    let pass = &*render.material.pass;
    unsafe {
        let Some(program) = state.program(gl, pass) else {
            return;
        };
        if state.vertex_array.is_none() {
            state.vertex_array = gl.create_vertex_array().ok();
        }
        gl.bind_vertex_array(state.vertex_array);
        gl.use_program(Some(program));
        set_blend(gl, render.material.blend_mode);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        // `double-sided` turns culling off; a facing quad has nothing to cull
        // [inferred: the rest draw with the transparents' culling, none here].
        gl.disable(glow::CULL_FACE);
        gl.color_mask(true, true, true, false);
        let mode = render.material.blend_mode as f32;
        let camera = &frame.camera;
        let eye = if camera.perspective > 0.0 {
            1.0 / camera.perspective
        } else {
            1.0e5
        };
        let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
        let bools = render.pixel_bools();
        let lit = |index: usize| {
            let [r, g, b] = DEFAULT_LIGHTING[index];
            [r, g, b, 0.0]
        };
        let drawn = super::h4_submit::DrawnObject {
            values: frame.h2_values_of_object(object),
            time: frame.time,
            owner: object,
        };
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let functions =
            render
                .material
                .submit
                .draw_values(super::h4_submit::H4Draw::Lit, &drawn, identity);
        for draw in draws {
            let Some(buffer) = state.upload_states(gl, &draw.states) else {
                continue;
            };
            let mut unit = 0u32;
            for (name, kind) in &pass.textures {
                gl.active_texture(glow::TEXTURE0 + unit);
                match name.as_str() {
                    "vs_particle_state_buffer" => {
                        gl.bind_texture(glow::TEXTURE_BUFFER, Some(buffer))
                    }
                    "psDepthSampler_texture" => {
                        gl.bind_texture(glow::TEXTURE_2D, depth.or(placeholder))
                    }
                    _ => {
                        let texture = state.texture(gl, render, name, kind);
                        match kind {
                            TextureKind::FlatArray => {
                                gl.bind_texture(glow::TEXTURE_2D_ARRAY, texture)
                            }
                            _ => gl.bind_texture(glow::TEXTURE_2D, texture.or(placeholder)),
                        }
                    }
                }
                let location = state.location(gl, pass, name);
                gl.uniform_1_i32(location.as_ref(), unit as i32);
                unit += 1;
            }
            for (name, ty) in &pass.uniforms {
                let location = state.location(gl, pass, name);
                if location.is_none() && !matches!(ty, Type::Named(_)) {
                    continue;
                }
                match name.as_str() {
                    "vsRenderState" => {
                        set_render_state(gl, state, pass, render, draw, frame.time);
                        continue;
                    }
                    "particle_index_range" => {
                        gl.uniform_2_u32(location.as_ref(), 0, 0);
                        continue;
                    }
                    "vs_emitter_to_world_matrix" => {
                        let [forward, left, up, position] = draw.to_world;
                        let rows: [f32; 12] =
                            std::array::from_fn(|at| [forward, left, up, position][at % 4][at / 4]);
                        gl.uniform_matrix_3x4_f32_slice(location.as_ref(), false, &rows);
                        continue;
                    }
                    "vs_sprite_list_sprites" => {
                        let mut sprites = [[0.0f32; 4]; 15];
                        for (at, sprite) in render.sprites.iter().take(15).enumerate() {
                            sprites[at] = *sprite;
                        }
                        gl.uniform_4_f32_slice(location.as_ref(), sprites.as_flattened());
                        continue;
                    }
                    _ => {}
                }
                if let Some(matrix) = h3_draw::h4_matrix(frame, name) {
                    match matrix.len() {
                        9 => gl.uniform_matrix_3_f32_slice(location.as_ref(), false, &matrix),
                        _ => gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix),
                    }
                    continue;
                }
                let value = match name.as_str() {
                    "vs_hiddenFromCompilerNaN" => floats([f32::NAN, 0.0, 0.0, f32::NAN]),
                    "vs_sprite_offset_and_scale" => floats(render.sprite),
                    "vs_cameraVelocity" => floats([0.0; 4]),
                    "vs_sprite_list_count_and_max" => floats([render.sprite_count, 0.0, 0.0, 0.0]),
                    "vsLightingBrightDirection" | "psLightingBrightDirection" => floats(lit(0)),
                    "vsLightingBrightColor" | "psLightingBrightColor" => floats(lit(1)),
                    "vsLightingDarkColor" | "psLightingDarkColor" => floats(lit(2)),
                    "vsLightingAmbient" | "psLightingAmbient" => floats(lit(3)),
                    // No level: no atmosphere, no fog.
                    "s_atmosphere_fog_enable" | "s_atmosphere_fog_light_enable" => {
                        Some(H3Value::Integer(0))
                    }
                    name if name.starts_with("s_atmosphere_constant_") => floats([0.0; 4]),
                    "vs_material_blend_constant" => {
                        floats(blend_constant(render.material.blend_mode))
                    }
                    "vs_bungie_exposure" => super::h4_lighting::h4_frame_value("ps_view_exposure"),
                    "vs_bungie_additive_scale_and_exposure" => {
                        super::h4_lighting::h4_frame_value("ps_view_self_illum_exposure")
                    }
                    "vs_alpha_blend_mode" | "ps_alpha_blend_mode" => floats([mode, 0.0, 0.0, 0.0]),
                    "ps_tiling_vpos_offset" => floats([0.0; 4]),
                    "psDepthFadeEnabled" => {
                        Some(H3Value::Integer(i32::from(bools[0] && depth.is_some())))
                    }
                    "psSphereWarpEnabled" => Some(H3Value::Integer(i32::from(bools[1]))),
                    "psNewSchoolFrameIndex" => Some(H3Value::Integer(i32::from(bools[2]))),
                    "psBlackOrWhitePointEnabled" => {
                        Some(H3Value::Integer(i32::from(draw.black_or_white)))
                    }
                    "psTintFactor" => floats([render.tint_factor, 0.0, 0.0, 0.0]),
                    // `ComputeDepthFade(fragpos · z)`: whole pixels.
                    "psDepthConstants" => floats([0.0, 0.0, 1.0, 0.0]),
                    super::h4_shield::DEPTH_DECODE => floats([
                        camera.depth_scale * camera.depth_map[0],
                        camera.depth_map[1],
                        camera.perspective,
                        eye,
                    ]),
                    super::h4_shield::WORLD_SCALE => floats([camera.scale; 4]),
                    other => {
                        h3_draw::frame_value(frame, h3_shaders::H3Dialect::Halo4, other, size, 0.0)
                            .or_else(|| functions.get(other).cloned())
                            .or_else(|| render.material.values.get(other).cloned())
                    }
                };
                if let Some(value) = value {
                    if let (Type::Vector(Scalar::Uint, _), H3Value::Floats(_)) = (ty, &value) {
                        continue;
                    }
                    h3_draw::set_uniform_value(gl, location.as_ref(), ty, &value);
                }
            }
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, draw.states.len() as i32);
        }
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_BUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        gl.blend_equation(glow::FUNC_ADD);
    }
}

/// `SetMaterialRenderState` (X360 0x83AD5618): `vsRenderState`, member by
/// member.
unsafe fn set_render_state(
    gl: &glow::Context,
    state: &mut H4ParticleGl,
    pass: &H3Pass,
    render: &H4ParticleRender,
    draw: &H4EmitterDraw,
    time: f32,
) {
    let [main, appearance, animation] = render.flags.map(|flags| flags as f32);
    let [near_range, near_cutoff] = draw.near;
    let [edge_range, edge_cutoff] = render.edge_fade;
    let members: Vec<(String, Vec<f32>)> = [
        (
            "flags_main_appearance_animation".to_owned(),
            vec![main, appearance, animation],
        ),
        // Third person, at the unzoomed field of view.
        ("firstPerson_fadeDepthMultiplier".to_owned(), vec![0.0, 1.0]),
        // `camera offset` is not in the kit's system layout.
        (
            "curvature_cameraOffset".to_owned(),
            vec![render.curvature, 0.0],
        ),
        ("uvScrollRate".to_owned(), vec![0.0, 0.0]),
        // Render seconds, coming round at four hours.
        ("gameTime".to_owned(), vec![time.rem_euclid(14_400.0)]),
        ("billboardType".to_owned(), vec![render.billboard as f32]),
        ("vertexCount".to_owned(), vec![4.0]),
        (
            "fade.nearRange_nearCutoff_edgeRange_edgeCutoff".to_owned(),
            vec![near_range.max(1e-4), near_cutoff, edge_range, edge_cutoff],
        ),
        ("fade.farRange_farCutoff".to_owned(), NO_FAR_FADE.to_vec()),
    ]
    .into_iter()
    .chain(
        custom_vert_registers(&render.custom_verts)
            .iter()
            .enumerate()
            .map(|(register, value)| {
                (
                    format!("customVerts[{}].customVertex{}", register / 2, register % 2),
                    value.to_vec(),
                )
            }),
    )
    .collect();
    unsafe {
        for (member, value) in members {
            let location = state.location(gl, pass, &format!("vsRenderState.{member}"));
            match value.len() {
                1 => gl.uniform_1_f32(location.as_ref(), value[0]),
                2 => gl.uniform_2_f32_slice(location.as_ref(), &value),
                3 => gl.uniform_3_f32_slice(location.as_ref(), &value),
                _ => gl.uniform_4_f32_slice(location.as_ref(), &value),
            }
        }
    }
}
