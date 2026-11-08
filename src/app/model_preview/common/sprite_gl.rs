//! The sprite and flare program every game's sprite pass draws through: its
//! GLSL, how it links, and the quads and occlusion it draws flares with.

use eframe::glow::{self, HasContext as _};

use super::super::gl::glsl::glsl_view_depth;
use super::super::renderer::*;
use super::super::gl::buffer::*;
use super::super::*;
use super::camera::*;

/// The vertex stream of Halo CE's lens flares: clip position, then texcoord.
pub(in crate::app::model_preview) const CE_FLARE_STRIDE: i32 = 6 * 4;

pub(in crate::app::model_preview) const CE_FLARE_VERTEX: &str = "#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
}
";

pub(in crate::app::model_preview) const CE_FLARE_FRAGMENT: &str = concat!("#version 330
in vec2 v_uv;
uniform sampler2D u_map;
uniform vec4 u_color;
uniform float u_highlight;
uniform vec2 u_tint;
uniform vec2 u_circular;
uniform float u_fade;
uniform vec4 u_depth;
uniform sampler2D u_scene_depth;
out vec4 out_color;
", glsl_view_depth!(), "void main() {
    vec4 map = texture(u_map, v_uv);
    if (u_circular.x > 0.0) {
        // Reach's light volume `circular` albedo: a disc, and no map.
        vec2 delta = 2.0 * v_uv - 1.0;
        float disc = pow(clamp(u_circular.x - u_circular.x * dot(delta, delta), 0.0, 1.0), u_circular.y);
        map = vec4(vec3(disc), 1.0);
    }
    if (u_fade > 0.0) {
        // Reach's light volume depth fade, as the particles'.
        float scene = texelFetch(u_scene_depth, ivec2(gl_FragCoord.xy), 0).r;
        float behind = (view_depth(scene) - view_depth(gl_FragCoord.z)) / u_depth.w;
        map.a *= clamp(behind / u_fade, 0.0, 1.0);
    }
    if (u_tint.y > 0.0) {
        vec4 lifted = pow(map, vec4(u_tint.y)) * map;
        out_color = vec4((lifted.rgb * u_tint.x + map.rgb * u_color.rgb) * u_color.a, 1.0);
    } else if (u_highlight < 0.0) {
        out_color = vec4(u_color.rgb * map.rgb, u_color.a * map.a);
    } else {
        // The pixel shader of `sub_92E4A0`, read from its bytecode.
        vec3 square = map.rgb * map.rgb;
        vec3 sixth = square * square * square;
        out_color = vec4((1.0 - u_color.rgb) * u_highlight * sixth + map.rgb * u_color.rgb, u_color.a);
    }
}
");

/// What every game's camera-facing quads draw with (flares, antennas, effect
/// sprites), made on first use.
pub(in crate::app::model_preview) struct SpriteGl {
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    pub(in crate::app::model_preview) buffer: glow::NativeBuffer,
    pub(in crate::app::model_preview) color: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) highlight: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) tint: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) map: Option<glow::NativeUniformLocation>,
    /// Reach's light volumes': the `circular` albedo and the depth fade.
    pub(in crate::app::model_preview) circular: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) fade: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) depth: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) scene_depth: Option<glow::NativeUniformLocation>,
    /// A patch's samples past the depth buffer, and in all.
    pub(in crate::app::model_preview) queries: [glow::NativeQuery; 2],
}

/// A sprite's four corners, all at the depth of its center.
pub(in crate::app::model_preview) fn ce_flare_quad(
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    sprite: &crate::app::model_preview::common::flares::FlareSprite,
) -> [[f32; 6]; 4] {
    let (sine, cosine) = sprite.rotation.sin_cos();
    let center = sprite.center;
    let depth = ce_flare_clip(camera, center);
    [[0.0, 0.0, -1.0, -1.0], [1.0, 0.0, 1.0, -1.0], [0.0, 1.0, -1.0, 1.0], [1.0, 1.0, 1.0, 1.0]].map(
        |[u, v, x, y]| {
            let across = x * sprite.extent[0];
            let upward = y * sprite.extent[1];
            let (right, up) = (across * cosine - upward * sine, across * sine + upward * cosine);
            let corner = std::array::from_fn(|axis| {
                center[axis] + view.right[axis] * right + view.up[axis] * up
            });
            let clip = ce_flare_clip(camera, corner);
            let scale = depth[3] / clip[3];
            [clip[0] * scale, clip[1] * scale, depth[2], depth[3], u, 1.0 - v]
        },
    )
}

/// How much of `patch` the model leaves showing, if any of it is on screen.
pub(in crate::app::model_preview) unsafe fn ce_flare_occlusion(
    gl: &glow::Context,
    flares: &SpriteGl,
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    patch: &crate::app::model_preview::common::flares::FlareSprite,
) -> Option<f32> {
    unsafe {
        let vertices = ce_flare_quad(camera, view, patch);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
        gl.enable(glow::DEPTH_TEST);
        gl.color_mask(false, false, false, false);
        let mut samples = [0.0; 2];
        for (sample, (query, compare)) in samples.iter_mut().zip([
            (flares.queries[0], glow::LEQUAL),
            (flares.queries[1], glow::ALWAYS),
        ]) {
            gl.depth_func(compare);
            gl.begin_query(glow::SAMPLES_PASSED, query);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.end_query(glow::SAMPLES_PASSED);
            // glow's plain reads take the EXT entry point below GL 4.5,
            // which panics on a Mac. This one checks the core entry
            // point, and with no query buffer bound its offset is
            // where the result goes.
            let mut passed = 0_u64;
            let result = &raw mut passed as usize;
            gl.get_query_parameter_u64_with_offset(query, glow::QUERY_RESULT, result);
            *sample = passed as f32;
        }
        gl.disable(glow::DEPTH_TEST);
        gl.color_mask(true, true, true, false);
        (samples[1] > 0.0).then(|| samples[0] / samples[1])
    }
}

pub(in crate::app::model_preview) unsafe fn draw_flare_sprites(
    gl: &glow::Context,
    flares: &SpriteGl,
    camera: &ModelGpuCamera,
    view: &common::view::ViewFrame,
    sprites: &[crate::app::model_preview::common::flares::FlareSprite],
    textures: &[Option<glow::NativeTexture>],
) {
    unsafe {
        for sprite in sprites {
            let Some(texture) = textures.get(sprite.image).copied().flatten() else {
                continue;
            };
            let vertices = ce_flare_quad(camera, view, sprite);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let [red, green, blue] = sprite.color;
            gl.uniform_4_f32(flares.color.as_ref(), red, green, blue, sprite.alpha);
            gl.uniform_1_f32(flares.highlight.as_ref(), sprite.highlight.unwrap_or(-1.0));
            let [modulation, power] = sprite.tint.unwrap_or([0.0; 2]);
            gl.uniform_2_f32(flares.tint.as_ref(), modulation, power);
            if sprite.depth_tested {
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

pub(in crate::app::model_preview) unsafe fn link_sprite_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<SpriteGl> {
    unsafe {
        let program = link_h2_program(gl, main, CE_FLARE_VERTEX, CE_FLARE_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, CE_FLARE_STRIDE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, CE_FLARE_STRIDE, 16);
        Some(SpriteGl {
            program,
            vertex_array,
            buffer,
            color: gl.get_uniform_location(program, "u_color"),
            highlight: gl.get_uniform_location(program, "u_highlight"),
            tint: gl.get_uniform_location(program, "u_tint"),
            map: gl.get_uniform_location(program, "u_map"),
            circular: gl.get_uniform_location(program, "u_circular"),
            fade: gl.get_uniform_location(program, "u_fade"),
            depth: gl.get_uniform_location(program, "u_depth"),
            scene_depth: gl.get_uniform_location(program, "u_scene_depth"),
            queries: [gl.create_query().ok()?, gl.create_query().ok()?],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::gl::glslang::assert_compiles_under_glslang;

    /// The flare program, which Reach's light volumes' disc and depth fade
    /// share, and Reach's composite with its gamma and noise, as the
    /// particle program is checked.
    #[test]
    fn the_flare_and_reach_composite_programs_compile_under_glslang() {
        assert_compiles_under_glslang("flare", &[("vert", CE_FLARE_VERTEX), ("frag", CE_FLARE_FRAGMENT)]);
        assert_compiles_under_glslang("reach_composite", &[("frag", super::reach::lighting::REACH_COMPOSITE_FRAGMENT)]);
    }
}
