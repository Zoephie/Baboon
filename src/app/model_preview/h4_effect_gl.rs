//! What Halo 4's effect systems share on the GPU: each draws through its
//! material's translated passes, reading its per-element state from a
//! buffer texture (`StructuredBuffer` in the game), with the material's
//! textures and the frame's constants. Particles (`h4_particle_pass`) and
//! light volumes (`h4_light_volume_pass`) set their own constants over
//! these.

use std::collections::HashMap;

use super::h3_shaders::{H3Pass, H3Value, H3Values};
use super::materials::H4EffectMaterial;
use super::*;
use crate::core::hlsl::parse::{Scalar, TextureKind, Type};

/// A linked pass and its uniforms' locations, by name.
type LinkedProgram = (
    glow::NativeProgram,
    HashMap<String, Option<glow::NativeUniformLocation>>,
);

/// The GL a slot keeps for its Halo 4 effects: programs by pass, textures
/// by material and sampler, and the buffer the states go through.
#[derive(Default)]
pub(super) struct H4EffectGl {
    programs: HashMap<usize, Option<LinkedProgram>>,
    textures: HashMap<(usize, String), Option<glow::NativeTexture>>,
    buffer: Option<(glow::NativeBuffer, glow::NativeTexture)>,
    /// `mesh_vertices`: a particle model's strip, 20 bytes a vertex.
    mesh: Option<(glow::NativeBuffer, glow::NativeTexture)>,
    vertex_array: Option<glow::NativeVertexArray>,
}

impl H4EffectGl {
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
                            "h4 effect shader failed: {}",
                            gl.get_shader_info_log(shader)
                        );
                    }
                    gl.attach_shader(program, shader);
                    gl.delete_shader(shader);
                }
                gl.link_program(program);
                if !gl.get_program_link_status(program) {
                    eprintln!(
                        "h4 effect program failed: {}",
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

    /// Set the float uniform, or struct member, `name` to `value`: one to
    /// four components.
    pub(super) unsafe fn set_floats(
        &mut self,
        gl: &glow::Context,
        pass: &H3Pass,
        name: &str,
        value: &[f32],
    ) {
        unsafe {
            let location = self.location(gl, pass, name);
            match value.len() {
                1 => gl.uniform_1_f32(location.as_ref(), value[0]),
                2 => gl.uniform_2_f32_slice(location.as_ref(), value),
                3 => gl.uniform_3_f32_slice(location.as_ref(), value),
                _ => gl.uniform_4_f32_slice(location.as_ref(), value),
            }
        }
    }

    pub(super) unsafe fn location(
        &mut self,
        gl: &glow::Context,
        pass: &H3Pass,
        name: &str,
    ) -> Option<glow::NativeUniformLocation> {
        let key = pass as *const H3Pass as usize;
        let (program, locations) = self.programs.get_mut(&key)?.as_mut()?;
        if let Some(found) = locations.get(name) {
            return *found;
        }
        let found = unsafe { gl.get_uniform_location(*program, name) };
        locations.insert(name.to_owned(), found);
        found
    }

    /// A sampler's bitmap: a texture array of all its images where the pass
    /// samples an array.
    unsafe fn texture(
        &mut self,
        gl: &glow::Context,
        material: &H4EffectMaterial,
        uniform: &str,
        kind: &TextureKind,
    ) -> Option<glow::NativeTexture> {
        let key = (
            material as *const H4EffectMaterial as usize,
            uniform.to_owned(),
        );
        if let Some(known) = self.textures.get(&key) {
            return *known;
        }
        let texture = material
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

    /// `bytes`, a word to a texel of an `R32UI` buffer texture, through
    /// `slot`'s buffer, which it makes the first time.
    unsafe fn upload_words(
        gl: &glow::Context,
        slot: &mut Option<(glow::NativeBuffer, glow::NativeTexture)>,
        bytes: &[u8],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            if slot.is_none() {
                let buffer = gl.create_buffer().ok()?;
                let texture = gl.create_texture().ok()?;
                *slot = Some((buffer, texture));
            }
            let (buffer, texture) = (*slot)?;
            gl.bind_buffer(glow::TEXTURE_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(glow::TEXTURE_BUFFER, bytes, glow::STREAM_DRAW);
            gl.bind_texture(glow::TEXTURE_BUFFER, Some(texture));
            gl.tex_buffer(glow::TEXTURE_BUFFER, glow::R32UI, Some(buffer));
            gl.bind_buffer(glow::TEXTURE_BUFFER, None);
            Some(texture)
        }
    }

    /// Ready `material`'s pass to draw into the bound light target: its
    /// program, its blend, the depth test without writes, no culling and
    /// colour only. None when the pass does not link.
    pub(super) unsafe fn begin(
        &mut self,
        gl: &glow::Context,
        material: &H4EffectMaterial,
    ) -> Option<()> {
        unsafe {
            let program = self.program(gl, &material.pass)?;
            if self.vertex_array.is_none() {
                self.vertex_array = gl.create_vertex_array().ok();
            }
            gl.bind_vertex_array(self.vertex_array);
            gl.use_program(Some(program));
            set_blend(gl, material.blend_mode);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            // `double-sided` turns culling off; a facing quad has nothing to
            // cull [inferred: the rest draw with the transparents' culling,
            // none here].
            gl.disable(glow::CULL_FACE);
            gl.color_mask(true, true, true, false);
        }
        Some(())
    }

    /// Upload `states` and bind every sampler the pass reads: the states
    /// as `buffer`, a particle model's strip as `mesh_vertices`, the scene's
    /// `depth` copy, and the material's maps.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn bind_textures<const WORDS: usize>(
        &mut self,
        gl: &glow::Context,
        material: &H4EffectMaterial,
        buffer: &str,
        states: &[[u32; WORDS]],
        mesh: Option<&[[u32; 5]]>,
        depth: Option<glow::NativeTexture>,
        placeholder: Option<glow::NativeTexture>,
    ) -> Option<()> {
        unsafe {
            let uploaded = Self::upload_words(gl, &mut self.buffer, slice_bytes(states))?;
            let mesh = match mesh {
                Some(mesh) => Some(Self::upload_words(gl, &mut self.mesh, slice_bytes(mesh))?),
                None => None,
            };
            let pass = &*material.pass;
            for (unit, (name, kind)) in pass.textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                match name.as_str() {
                    name if name == buffer => gl.bind_texture(glow::TEXTURE_BUFFER, Some(uploaded)),
                    "mesh_vertices" if mesh.is_some() => gl.bind_texture(glow::TEXTURE_BUFFER, mesh),
                    "psDepthSampler_texture" => {
                        gl.bind_texture(glow::TEXTURE_2D, depth.or(placeholder))
                    }
                    _ => {
                        let texture = self.texture(gl, material, name, kind);
                        match kind {
                            TextureKind::FlatArray => {
                                gl.bind_texture(glow::TEXTURE_2D_ARRAY, texture)
                            }
                            _ => gl.bind_texture(glow::TEXTURE_2D, texture.or(placeholder)),
                        }
                    }
                }
                let location = self.location(gl, pass, name);
                gl.uniform_1_i32(location.as_ref(), unit as i32);
            }
        }
        Some(())
    }

    /// Set `name`, of type `ty`, to what every effect gives it: the view's
    /// matrices and constants, the blend, the depth fade's terms, then the
    /// material's `functions` and baked values. False when none of them
    /// knows it.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn set_shared_uniform(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        material: &H4EffectMaterial,
        functions: &H3Values,
        name: &str,
        ty: &Type,
        size: [i32; 2],
    ) -> bool {
        let pass = &*material.pass;
        unsafe {
            let location = self.location(gl, pass, name);
            if let Some(matrix) = h3_draw::h4_matrix(frame, name) {
                match matrix.len() {
                    9 => gl.uniform_matrix_3_f32_slice(location.as_ref(), false, &matrix),
                    _ => gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &matrix),
                }
                return true;
            }
            let Some(value) = shared_value(frame, material, functions, name, size) else {
                return false;
            };
            // A float the game hands a `uint` register is not one of ours.
            if !matches!(
                (ty, &value),
                (Type::Vector(Scalar::Uint, _), H3Value::Floats(_))
            ) {
                h3_draw::set_uniform_value(gl, location.as_ref(), ty, &value);
            }
            true
        }
    }

    /// Put the texture units back as the other stages expect them.
    pub(super) unsafe fn end(&self, gl: &glow::Context) {
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_BUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.blend_equation(glow::FUNC_ADD);
        }
    }
}

/// The material's per-draw function values for `object`'s draw.
pub(super) fn material_functions(
    frame: &ModelGpuFrame,
    material: &H4EffectMaterial,
    object: u8,
) -> H3Values {
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
    material
        .submit
        .draw_values(super::h4_submit::H4Draw::Lit, &drawn, identity)
}

/// What every effect's constant `name` is, before the material's own.
fn shared_value(
    frame: &ModelGpuFrame,
    material: &H4EffectMaterial,
    functions: &H3Values,
    name: &str,
    size: [i32; 2],
) -> Option<H3Value> {
    let floats = |value: [f32; 4]| Some(H3Value::Floats(value));
    let camera = &frame.camera;
    let eye = if camera.perspective > 0.0 {
        1.0 / camera.perspective
    } else {
        1.0e5
    };
    let mode = material.blend_mode;
    match name {
        "vs_hiddenFromCompilerNaN" => floats([f32::NAN, 0.0, 0.0, f32::NAN]),
        // No level: no atmosphere, no fog.
        "s_atmosphere_fog_enable" | "s_atmosphere_fog_light_enable" => Some(H3Value::Integer(0)),
        name if name.starts_with("s_atmosphere_constant_") => floats([0.0; 4]),
        "vs_material_blend_constant" => floats(blend_constant(mode)),
        "vs_bungie_exposure" => super::h4_lighting::h4_frame_value("ps_view_exposure"),
        "vs_bungie_additive_scale_and_exposure" => {
            super::h4_lighting::h4_frame_value("ps_view_self_illum_exposure")
        }
        "vs_alpha_blend_mode" | "ps_alpha_blend_mode" => floats([f32::from(mode), 0.0, 0.0, 0.0]),
        "ps_tiling_vpos_offset" => floats([0.0; 4]),
        // `ComputeDepthFade(fragpos · z)`: whole pixels.
        "psDepthConstants" => floats([0.0, 0.0, 1.0, 0.0]),
        super::h4_shield::DEPTH_DECODE => floats([
            camera.depth_scale * camera.depth_map[0],
            camera.depth_map[1],
            camera.perspective,
            eye,
        ]),
        super::h4_shield::WORLD_SCALE => floats([camera.scale; 4]),
        other => h3_draw::frame_value(frame, h3_shaders::H3Dialect::Halo4, other, size, 0.0)
            .or_else(|| functions.get(other).cloned())
            .or_else(|| material.values.get(other).cloned()),
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
                || image.rgba.len() != (width * height * 4)
        })
    {
        return None;
    }
    let format = match texels {
        h3_shaders::H3Texels::Srgb => glow::SRGB8_ALPHA8,
        _ => glow::RGBA8,
    };
    let mut rgba = Vec::with_capacity(flats.len() * (width * height * 4));
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
/// modes effects use; the packed states of the rest are not decoded, and
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
