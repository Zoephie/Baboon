//! The cheap particle program Reach and Halo 4 draw their cheap particles with.

use super::super::*;
use super::super::super::gl::buffer::slice_bytes;
use super::super::super::gen3::shaders::Gen3Blend;
use eframe::glow::{self, HasContext as _};

/// The cheap particle program: the quads as built, the render texture's
/// layer sampled a half level sharp, by the vertex colour, at `g_exposure.x`.
pub(in crate::app::model_preview) const CHEAP_PARTICLE_VERTEX: &str = "#version 330 core
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec3 a_texcoord;
layout(location = 2) in vec4 a_color;
out vec3 v_texcoord;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_texcoord = a_texcoord;
    v_color = a_color;
}
";

/// `cheap_particles_render.hlsl`'s pixel shader with no fog to add. Its
/// alpha goes out whole, where the game's is a 32nd: the target's bias is
/// taken to undo that, as the tool's \"alpha of zero gives you additive\"
/// needs [inferred: not traced to the target's exponent bias].
pub(in crate::app::model_preview) const CHEAP_PARTICLE_FRAGMENT: &str = "#version 330 core
uniform sampler2DArray u_render;
uniform float u_exposure;
// Halo 4 runs from its exposure to its self-illumination one by alpha;
// Reach's two are the same.
uniform float u_self_illum;
in vec3 v_texcoord;
in vec4 v_color;
out vec4 out_color;
void main() {
    vec4 texel = texture(u_render, v_texcoord, -0.5) * v_color;
    out_color = vec4(texel.rgb * mix(u_exposure, u_self_illum, texel.a), texel.a);
}
";

/// A cheap particle vertex: clip position, texcoord and layer, colour.
pub(in crate::app::model_preview) const CHEAP_FLOATS: usize = 11;

/// Reach's cheap particle program and its stream.
pub(in crate::app::model_preview) struct CheapGl {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    buffer: glow::NativeBuffer,
    exposure: Option<glow::NativeUniformLocation>,
    self_illum: Option<glow::NativeUniformLocation>,
}

impl CheapGl {
    /// `render_particles` (the 360's 0x82DB3B58): `set_alpha_blend_mode(5)`,
    /// ONE and INVSRCALPHA, tested and not written against depth, at
    /// `g_exposure.x` for the area screen effects' `stops`. The game writes
    /// colour alone (`set_color_write_enable(0, 7)`); here alpha is what
    /// covers the preview's background, so it is written as the other
    /// premultiplied sprites write it.
    /// `exposures` are the exposure and the self-illumination exposure the
    /// colour runs between by alpha: Halo 4's two, or Reach's one twice.
    pub(in crate::app::model_preview) unsafe fn draw(&self, gl: &glow::Context, vertices: &[[f32; CHEAP_FLOATS]], texture: glow::NativeTexture, [exposure, self_illum]: [f32; 2]) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(vertices), glow::STREAM_DRAW);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.uniform_1_f32(self.exposure.as_ref(), exposure);
            gl.uniform_1_f32(self.self_illum.as_ref(), self_illum);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gen3::pipeline::gen3_set_blend(gl, Gen3Blend::PreMultiplied);
            gl.draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            gl.color_mask(true, true, true, false);
        }
    }
}

pub(in crate::app::model_preview) unsafe fn reach_cheap_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<CheapGl> {
    use particles::cheap::{CHEAP_PARTICLE_FRAGMENT, CHEAP_PARTICLE_VERTEX};
    unsafe {
        let program = link_h2_program(gl, main, CHEAP_PARTICLE_VERTEX, CHEAP_PARTICLE_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        let stride = (CHEAP_FLOATS * size_of::<f32>()) as i32;
        let mut offset = 0;
        for (attribute, size) in [4, 3, 4].into_iter().enumerate() {
            gl.enable_vertex_attrib_array(attribute as u32);
            gl.vertex_attrib_pointer_f32(attribute as u32, size, glow::FLOAT, false, stride, offset);
            offset += size * size_of::<f32>() as i32;
        }
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_render").as_ref(), 0);
        Some(CheapGl { program, vertex_array, buffer, exposure: gl.get_uniform_location(program, "u_exposure"), self_illum: gl.get_uniform_location(program, "u_self_illum") })
    }
}

/// The render texture array, every level: sampler 0 at address mode 1,
/// clamped (`shader_setup_type_library`, the 360's 0x82DB2B50) [inferred: by
/// the tag enum's order, as Baboon reads sampler modes elsewhere], linear
/// within a layer and between levels, and GL never filters across layers.
pub(in crate::app::model_preview) unsafe fn upload_cheap_render_texture(
    gl: &glow::Context,
    render: &particles::cheap::CheapRenderTexture,
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
        for (level, rgba) in render.levels.iter().enumerate() {
            let (width, height) = ((render.width >> level).max(1), (render.height >> level).max(1));
            if rgba.len() != width * height * render.layers * 4 {
                gl.delete_texture(texture);
                return None;
            }
            let data = glow::PixelUnpackData::Slice(Some(rgba));
            let [width, height, layers] = [width, height, render.layers].map(|size| size as i32);
            gl.tex_image_3d(glow::TEXTURE_2D_ARRAY, level as i32, glow::RGBA8 as i32, width, height, layers, 0, glow::RGBA, glow::UNSIGNED_BYTE, data);
        }
        let last = render.levels.len().saturating_sub(1) as i32;
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
            (glow::TEXTURE_MAX_LEVEL, last),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, name, value);
        }
        gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::model_preview::gl::glslang::assert_compiles_under_glslang;

    /// Reach's cheap particle program, sampling its layers, as the particle
    /// program is checked.
    #[test]
    fn the_cheap_particle_program_compiles_under_glslang() {
        use super::particles::cheap::{CHEAP_PARTICLE_FRAGMENT, CHEAP_PARTICLE_VERTEX};
        assert_compiles_under_glslang("cheap_particle", &[("vert", CHEAP_PARTICLE_VERTEX), ("frag", CHEAP_PARTICLE_FRAGMENT)]);
    }
}
