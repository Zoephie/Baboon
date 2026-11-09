//! The particle program H2 and the gen3 games draw their particles with: its GLSL, vertex floats and sprite maps.

use super::*;
use super::super::gl::buffer::slice_bytes;
use super::super::gl::glsl::{fill, VIEW_DEPTH};
use super::super::gen3::shaders::Gen3Blend;
use eframe::glow::{self, HasContext as _};
use super::super::reach::lighting::reach_particle_light;

const PARTICLE_VERTEX: &str = include_str!("shaders/particle.vert");

fn particle_fragment() -> String {
    fill(include_str!("shaders/particle.frag"), &[("view_depth", &VIEW_DEPTH)])
}

const PARTICLE_FLOATS: usize = 24;

/// A run of vertices as the particle program takes them.
/// A gen3 sprite shader's base, alpha, palette and second maps, each with
/// its own sampler.
pub(in crate::app::model_preview) unsafe fn sprite_maps(
    gl: &glow::Context,
    look: &particles::SpriteShader,
) -> [Option<glow::NativeTexture>; 4] {
    let [base, alpha, palette, second] = look.samplers;
    [(&look.base_map, base), (&look.alpha_map, alpha), (&look.palette_map, palette), (&look.second_map, second)].map(
        |(map, sampler)| {
            map.as_ref().and_then(|(image, texels, mips)| unsafe {
                crate::app::model_preview::gl::texture::upload_image_linear(gl, image, *texels, mips, sampler)
            })
        },
    )
}

pub(in crate::app::model_preview) fn particle_floats(camera: &ModelGpuCamera, drawn: &[particles::ParticleCorner]) -> Vec<[f32; PARTICLE_FLOATS]> {
    drawn
        .iter()
        .map(|vertex| {
            let [a, b, c, d] = ce_flare_clip(camera, vertex.place);
            let ([e, f], [g, h], [i, j]) = (vertex.sprite, vertex.next_sprite, vertex.billboard);
            let [k, l, m, n] = vertex.color;
            let [o, p, q] = vertex.extra;
            let lean = vertex.lean.map(|side| {
                let reach: [f32; 3] = std::array::from_fn(|axis| vertex.place[axis] + side[axis]);
                let [x, y, _, w] = ce_flare_clip(camera, reach);
                [(x / w - a / d) * 0.5, (y / w - b / d) * 0.5]
            });
            let [[r, s], [t, u]] = lean;
            let [v, w, x] = vertex.normal;
            [a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x]
        })
        .collect()
}

/// What Halo 3's particles draw with, made on first use.
pub(in crate::app::model_preview) struct ParticleGl {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    buffer: glow::NativeBuffer,
    albedo: Option<glow::NativeUniformLocation>,
    black_point: Option<glow::NativeUniformLocation>,
    multiply: Option<glow::NativeUniformLocation>,
    add: Option<glow::NativeUniformLocation>,
    lit: Option<glow::NativeUniformLocation>,
    premultiplied: Option<glow::NativeUniformLocation>,
    alpha_test: Option<glow::NativeUniformLocation>,
    tint: Option<glow::NativeUniformLocation>,
    modulation: Option<glow::NativeUniformLocation>,
    base_xform: Option<glow::NativeUniformLocation>,
    second_xform: Option<glow::NativeUniformLocation>,
    lighting: Option<glow::NativeUniformLocation>,
    distortion: Option<glow::NativeUniformLocation>,
    unsigned: Option<glow::NativeUniformLocation>,
    fade: Option<glow::NativeUniformLocation>,
    depth: Option<glow::NativeUniformLocation>,
    screen: Option<glow::NativeUniformLocation>,
    smooth: Option<glow::NativeUniformLocation>,
    reach_light: Option<glow::NativeUniformLocation>,
    contrast: Option<glow::NativeUniformLocation>,
    sphere_warp: Option<glow::NativeUniformLocation>,
    palette_shift: Option<glow::NativeUniformLocation>,
    plasma_second_sprite: Option<glow::NativeUniformLocation>,
    unit_particle_alpha: Option<glow::NativeUniformLocation>,
}

impl ParticleGl {
    pub(in crate::app::model_preview) unsafe fn draw(
        &self,
        gl: &glow::Context,
        look: &particles::SpriteShader,
        vertices: &[[f32; PARTICLE_FLOATS]],
        textures: [Option<glow::NativeTexture>; 4],
        no_map: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        camera: &ModelGpuCamera,
        lighting: &[[f32; 4]; 10],
        stops: f32,
    ) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(vertices), glow::STREAM_DRAW);
            for (unit, texture) in [0, 1, 2, 4].into_iter().zip(textures) {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, texture.or(no_map));
            }
            gl.active_texture(glow::TEXTURE3);
            // Halo 2 has no depth copy, and no fade to read one; an empty unit
            // still makes Apple's driver complain.
            gl.bind_texture(glow::TEXTURE_2D, depth.or(no_map));
            gl.active_texture(glow::TEXTURE0);
            let range = if depth.is_some() { look.depth_fade.unwrap_or(0.0) } else { 0.0 };
            gl.uniform_1_f32(self.fade.as_ref(), range);
            gl.uniform_4_f32_slice(self.depth.as_ref(), &scene_depth_terms(camera));
            gl.uniform_1_i32(self.albedo.as_ref(), look.albedo as i32);
            if look.albedo == particles::ParticleAlbedo::H2Distortion {
                let mut viewport = [0; 4];
                gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
                let [x, y, width, height] = viewport.map(|value| value as f32);
                gl.uniform_4_f32(self.screen.as_ref(), x, y, width.max(1.0), height.max(1.0));
            }
            let constants = look.constants;
            gl.uniform_3_f32_slice(self.tint.as_ref(), &constants.tint);
            gl.uniform_2_f32(self.modulation.as_ref(), constants.modulation, constants.alpha_modulation);
            gl.uniform_4_f32_slice(self.base_xform.as_ref(), &constants.base_xform);
            gl.uniform_4_f32_slice(self.second_xform.as_ref(), &constants.second_xform);
            gl.uniform_1_i32(self.black_point.as_ref(), i32::from(look.black_point));
            let multiply = look.blend == Gen3Blend::Multiply;
            gl.uniform_1_i32(self.multiply.as_ref(), i32::from(multiply));
            gl.uniform_3_f32_slice(self.add.as_ref(), &look.add(stops));
            gl.uniform_1_f32(self.distortion.as_ref(), look.distortion.unwrap_or(0.0));
            gl.uniform_1_i32(self.lit.as_ref(), i32::from(look.lit_pixels));
            let contrast = look.smooth_contrast();
            gl.uniform_1_i32(self.smooth.as_ref(), i32::from(contrast.is_some()));
            if let Some([scale, offset]) = contrast {
                let light = reach_particle_light();
                let rows = [light.direction, light.bright, light.dark];
                gl.uniform_3_f32_slice(self.reach_light.as_ref(), rows.as_flattened());
                gl.uniform_2_f32(self.contrast.as_ref(), scale, offset);
            }
            gl.uniform_1_f32(self.sphere_warp.as_ref(), look.sphere_warp.unwrap_or(0.0));
            gl.uniform_1_f32(self.palette_shift.as_ref(), look.palette_shift.unwrap_or(0.0));
            gl.uniform_1_i32(self.plasma_second_sprite.as_ref(), i32::from(look.plasma_second_sprite));
            gl.uniform_1_i32(self.unit_particle_alpha.as_ref(), i32::from(look.unit_particle_alpha));
            let premultiplied = look.blend == Gen3Blend::PreMultiplied;
            gl.uniform_1_i32(self.premultiplied.as_ref(), i32::from(premultiplied));
            gl.uniform_1_i32(self.alpha_test.as_ref(), i32::from(look.alpha_test));
            gl.depth_mask(look.alpha_test);
            gl.uniform_4_f32_slice(self.lighting.as_ref(), lighting.as_flattened());
            let unsigned = look.base_map.as_ref().is_some_and(|(_, texels, _)| *texels != crate::app::model_preview::common::image::ImageTexels::Signed);
            gl.uniform_1_i32(self.unsigned.as_ref(), i32::from(unsigned));
            if look.distortion.is_some() {
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func(glow::ONE, glow::ONE);
            } else {
                gen3::pipeline::gen3_set_blend(gl, look.blend);
            }
            // What covers the background has to say so, as an antenna does.
            let covers = matches!(look.blend, Gen3Blend::AlphaBlend | Gen3Blend::PreMultiplied);
            gl.color_mask(true, true, true, covers);
            gl.draw_arrays(glow::TRIANGLES, 0, vertices.len() as i32);
            gl.depth_mask(false);
            gl.color_mask(true, true, true, false);
        }
    }
}

pub(in crate::app::model_preview) unsafe fn gen3_particle_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<ParticleGl> {
    unsafe {
        let program = link_h2_program(gl, main, PARTICLE_VERTEX, &particle_fragment())
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        let stride = (PARTICLE_FLOATS * size_of::<f32>()) as i32;
        let mut offset = 0;
        for (attribute, size) in [4, 2, 2, 2, 4, 3, 4, 3].into_iter().enumerate() {
            gl.enable_vertex_attrib_array(attribute as u32);
            gl.vertex_attrib_pointer_f32(attribute as u32, size, glow::FLOAT, false, stride, offset);
            offset += size * size_of::<f32>() as i32;
        }
        gl.use_program(Some(program));
        for (unit, name) in ["u_base", "u_alpha", "u_palette", "u_scene_depth", "u_base2"].into_iter().enumerate() {
            gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
        }
        Some(ParticleGl {
            program,
            vertex_array,
            buffer,
            albedo: gl.get_uniform_location(program, "u_albedo"),
            black_point: gl.get_uniform_location(program, "u_black_point"),
            multiply: gl.get_uniform_location(program, "u_multiply"),
            add: gl.get_uniform_location(program, "u_add"),
            lit: gl.get_uniform_location(program, "u_lit"),
            premultiplied: gl.get_uniform_location(program, "u_premultiplied"),
            alpha_test: gl.get_uniform_location(program, "u_alpha_test"),
            tint: gl.get_uniform_location(program, "u_tint"),
            modulation: gl.get_uniform_location(program, "u_modulation"),
            base_xform: gl.get_uniform_location(program, "u_base_xform"),
            second_xform: gl.get_uniform_location(program, "u_base2_xform"),
            lighting: gl.get_uniform_location(program, "u_lighting"),
            distortion: gl.get_uniform_location(program, "u_distortion"),
            unsigned: gl.get_uniform_location(program, "u_unsigned"),
            fade: gl.get_uniform_location(program, "u_fade"),
            depth: gl.get_uniform_location(program, "u_depth"),
            screen: gl.get_uniform_location(program, "u_screen"),
            smooth: gl.get_uniform_location(program, "u_smooth"),
            reach_light: gl.get_uniform_location(program, "u_reach_light"),
            contrast: gl.get_uniform_location(program, "u_contrast"),
            sphere_warp: gl.get_uniform_location(program, "u_sphere_warp"),
            palette_shift: gl.get_uniform_location(program, "u_palette_shift"),
            plasma_second_sprite: gl.get_uniform_location(program, "u_plasma_second_sprite"),
            unit_particle_alpha: gl.get_uniform_location(program, "u_unit_particle_alpha"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::gl::glslang::assert_compiles_under_glslang;

    /// The particle program is linked on first use, so a compile error would
    /// only show as effects that draw nothing. Without `glslangValidator`,
    /// skip by name.
    #[test]
    fn the_particle_program_compiles_under_glslang() {
        assert_compiles_under_glslang("particle", &[("vert", PARTICLE_VERTEX), ("frag", &particle_fragment())]);
    }
}
