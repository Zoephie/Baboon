//! The program Halo CE's effect sprites draw with.

use super::super::super::renderer::link_pass_program;
use eframe::glow::{self, HasContext as _};

/// `fx/eff_common.h`: what Halo CE's particles, contrails and light volumes
/// draw with. The fog it folds into the vertex alpha is 1 with no level.
/// `effect.vsh`, `effect_multitexture.vsh`, `effect_multitexture_screenspace.vsh`
/// and `effect_zsprite.vsh` as far as a sprite needs: its place, its map's
/// coordinates, and the corner of the sprite the zsprite stream gives it.
const CE_EFFECT_VERTEX: &str = include_str!("shaders/effect.vert");

const CE_EFFECT_FRAGMENT: &str = include_str!("shaders/effect.frag");

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

pub(in crate::app::model_preview) unsafe fn ce_effect_gl(
    gl: &glow::Context,
    main: glow::NativeProgram,
) -> Option<CeEffectGl> {
    unsafe {
        let program = link_pass_program(gl, main, CE_EFFECT_VERTEX, CE_EFFECT_FRAGMENT)
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
