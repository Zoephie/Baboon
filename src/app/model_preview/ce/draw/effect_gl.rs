//! The program Halo CE's effect sprites draw with.

use super::super::super::renderer::link_h2_program;
use eframe::glow::{self, HasContext as _};

/// `fx/eff_common.h`: what Halo CE's particles, contrails and light volumes
/// draw with. The fog it folds into the vertex alpha is 1 with no level.
/// `effect.vsh`, `effect_multitexture.vsh`, `effect_multitexture_screenspace.vsh`
/// and `effect_zsprite.vsh` as far as a sprite needs: its place, its map's
/// coordinates, and the corner of the sprite the zsprite stream gives it.
const CE_EFFECT_VERTEX: &str = "#version 330
layout(location = 0) in vec4 a_clip;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec2 a_unit;
layout(location = 3) in vec4 a_color;
out vec2 v_uv;
out vec2 v_unit;
out vec4 v_clip;
out vec4 v_color;
void main() {
    gl_Position = a_clip;
    v_uv = a_uv;
    v_unit = a_unit;
    v_clip = a_clip;
    v_color = a_color;
}
";

const CE_EFFECT_FRAGMENT: &str = "#version 330
in vec2 v_uv;
in vec2 v_unit;
in vec4 v_clip;
// The vertex's color, as the game's effect vertices carry it.
in vec4 v_color;
uniform sampler2D u_map;
uniform sampler2D u_second;
uniform int u_blend;
uniform int u_nonlinear;
// 0 none, 1 a second color map over the sprite, 2 over the screen, 3 zsprite.
uniform int u_mode;
uniform vec4 u_rows[2];
// The zsprite's radius, and how clip z and w move per unit toward the eye.
uniform vec3 u_zsprite;
out vec4 out_color;
void main() {
    vec4 t0 = texture(u_map, v_uv);
    vec4 r0 = vec4(u_nonlinear != 0 ? mix(pow(t0.rgb, vec3(4.0)), t0.rgb, v_color.rgb) : t0.rgb * v_color.rgb, t0.a);
    if (u_mode == 1 || u_mode == 2) {
        vec4 at = u_mode == 1 ? vec4(v_uv, 0.0, 1.0) : vec4(v_clip.xy / v_clip.w, 1.0, 1.0);
        vec4 t1 = texture(u_second, vec2(dot(at, u_rows[0]), dot(at, u_rows[1])));
        r0 *= t1;
    }
    float depth = gl_FragCoord.z;
    if (u_mode == 3) {
        // The map's green says how far toward the eye the pixel stands, never
        // through the near plane.
        float toward = u_zsprite.x * texture(u_second, v_unit).g;
        if (u_zsprite.z > 0.0) {
            toward = min(toward, max((v_clip.w - 0.001) / u_zsprite.z, 0.0));
        }
        float z = v_clip.z - toward * u_zsprite.y;
        float w = v_clip.w - toward * u_zsprite.z;
        depth = (z / w) * 0.5 + 0.5;
    }
    gl_FragDepth = depth;
    float a = v_color.a;
    // `sub_85F3B0` draws min with the multiply shader, and subtract and max
    // with the add one.
    if (u_blend == 0) {
        r0.a *= a;
    } else if (u_blend == 1 || u_blend == 5) {
        r0.rgb = mix(vec3(1.0), r0.rgb, a);
    } else if (u_blend == 2) {
        r0.rgb = mix(vec3(0.5), r0.rgb, a);
    } else if (u_blend == 3 || u_blend == 4 || u_blend == 6) {
        r0.rgb *= a;
    } else if (u_blend == 7) {
        r0 *= a;
    }
    out_color = r0;
}
";

pub(in crate::app::model_preview) struct CeEffectGl {
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    pub(in crate::app::model_preview) vertex_array: glow::NativeVertexArray,
    pub(in crate::app::model_preview) buffer: glow::NativeBuffer,
    pub(in crate::app::model_preview) blend: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) nonlinear: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) mode: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) rows: Option<glow::NativeUniformLocation>,
    pub(in crate::app::model_preview) zsprite: Option<glow::NativeUniformLocation>,
}

/// A CE effect sprite's vertex: clip position, map coordinates, the
/// sprite's corner for a zsprite's map, and color.
pub(in crate::app::model_preview) const CE_EFFECT_FLOATS: usize = 12;

const CE_EFFECT_STRIDE: i32 = CE_EFFECT_FLOATS as i32 * 4;

pub(in crate::app::model_preview) unsafe fn ce_effect_gl(gl: &glow::Context, main: glow::NativeProgram) -> Option<CeEffectGl> {
    unsafe {
        let program = link_h2_program(gl, main, CE_EFFECT_VERTEX, CE_EFFECT_FRAGMENT)
            .map_err(|error| eprintln!("model preview: {error}"))
            .ok()?;
        let vertex_array = gl.create_vertex_array().ok()?;
        let buffer = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, CE_EFFECT_STRIDE, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, CE_EFFECT_STRIDE, 16);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, CE_EFFECT_STRIDE, 24);
        gl.enable_vertex_attrib_array(3);
        gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, CE_EFFECT_STRIDE, 32);
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_map").as_ref(), 0);
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_second").as_ref(), 1);
        Some(CeEffectGl {
            program,
            vertex_array,
            buffer,
            blend: gl.get_uniform_location(program, "u_blend"),
            nonlinear: gl.get_uniform_location(program, "u_nonlinear"),
            mode: gl.get_uniform_location(program, "u_mode"),
            rows: gl.get_uniform_location(program, "u_rows"),
            zsprite: gl.get_uniform_location(program, "u_zsprite"),
        })
    }
}
