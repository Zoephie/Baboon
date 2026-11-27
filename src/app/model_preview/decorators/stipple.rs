//! The stipple pattern in the stencil, as `rasterizer_stipple_initialize`
//! writes it before the decorators: a value a pixel from the rasterizer
//! globals' `default bitmaps[9]`, tiled over the picture, in the stencil's
//! low bits, which each level's draw then tests against.
//!
//! The game draws the bitmap through a copy shader into the stencil in one
//! pass. GL cannot write a stencil value from a shader without an extension,
//! so this draws one pass a value, each keeping only that value's pixels.

use super::super::gl::link::link;
use super::Stipple;
use eframe::glow::{self, HasContext as _};

const VERTEX: &str = include_str!("../gl/shaders/composite.vert");

/// One value of the pattern's pixels: those whose value is `u_value`,
/// tiled from the target's corner.
const FRAGMENT: &str = "#version 410 core
uniform int u_pattern[64];
uniform ivec2 u_size;
uniform int u_value;
out vec4 o_color;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy) % u_size;
    if (u_pattern[at.y * u_size.x + at.x] != u_value) {
        discard;
    }
    o_color = vec4(0.0);
}
";

/// The pattern's largest edge the program holds: the game's bitmaps are
/// 8 × 8.
const MOST: usize = 8;

/// What a target's stencil was last given the pattern for: its framebuffer,
/// viewport and the frame's time. The decorators' draws test the stencil
/// and never write it, so a target keeps its pattern until another frame.
type Written = (Option<glow::NativeFramebuffer>, [i32; 4], u32);

pub(in crate::app::model_preview) struct StippleWriter {
    program: glow::NativeProgram,
    array: glow::NativeVertexArray,
    written: Option<Written>,
}

impl StippleWriter {
    pub(in crate::app::model_preview) unsafe fn new(gl: &glow::Context) -> Option<Self> {
        unsafe {
            let program = match link(
                gl,
                &[
                    (glow::VERTEX_SHADER, VERTEX),
                    (glow::FRAGMENT_SHADER, FRAGMENT),
                ],
                &[],
            ) {
                Ok(program) => program,
                Err(log) => {
                    eprintln!("model preview: decorator stipple: {log}");
                    return None;
                }
            };
            let Ok(array) = gl.create_vertex_array() else {
                gl.delete_program(program);
                return None;
            };
            Some(Self {
                program,
                array,
                written: None,
            })
        }
    }

    /// `stipple` in the bound target's stencil, within `mask`, unless this
    /// frame already gave it; the state it changes put back.
    pub(in crate::app::model_preview) unsafe fn write(
        &mut self,
        gl: &glow::Context,
        stipple: &Stipple,
        mask: u32,
        time: f32,
    ) {
        let [width, height] = stipple.size;
        if width == 0 || height == 0 || width > MOST || height > MOST {
            return;
        }
        unsafe {
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let written = (
                gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
                viewport,
                time.to_bits(),
            );
            if self.written == Some(written) {
                return;
            }
            self.written = Some(written);
            let enabled: Vec<(u32, bool)> = [
                glow::DEPTH_TEST,
                glow::BLEND,
                glow::CULL_FACE,
                glow::STENCIL_TEST,
            ]
            .into_iter()
            .map(|capability| (capability, gl.is_enabled(capability)))
            .collect();
            let depth_write = gl.get_parameter_i32(glow::DEPTH_WRITEMASK) != 0;
            let mut color_write = [0; 4];
            gl.get_parameter_i32_slice(glow::COLOR_WRITEMASK, &mut color_write);
            let stencil_write = gl.get_parameter_i32(glow::STENCIL_WRITEMASK) as u32;
            let program = gl.get_parameter_program(glow::CURRENT_PROGRAM);
            let array = gl.get_parameter_vertex_array(glow::VERTEX_ARRAY_BINDING);

            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::STENCIL_TEST);
            gl.depth_mask(false);
            gl.color_mask(false, false, false, false);
            gl.stencil_mask(mask);
            gl.stencil_op(glow::KEEP, glow::KEEP, glow::REPLACE);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.array));
            let mut pattern = [0_i32; MOST * MOST];
            for (slot, value) in pattern.iter_mut().zip(&stipple.values) {
                *slot = i32::from(*value);
            }
            let location = |name: &str| gl.get_uniform_location(self.program, name);
            gl.uniform_1_i32_slice(location("u_pattern").as_ref(), &pattern);
            gl.uniform_2_i32(location("u_size").as_ref(), width as i32, height as i32);
            let value_location = location("u_value");
            let mut values: Vec<u8> = stipple.values.clone();
            values.sort_unstable();
            values.dedup();
            for value in values {
                gl.uniform_1_i32(value_location.as_ref(), i32::from(value));
                gl.stencil_func(glow::ALWAYS, i32::from(value), mask);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }

            for (capability, on) in enabled {
                if on {
                    gl.enable(capability);
                } else {
                    gl.disable(capability);
                }
            }
            gl.depth_mask(depth_write);
            let [red, green, blue, alpha] = color_write.map(|channel| channel != 0);
            gl.color_mask(red, green, blue, alpha);
            gl.stencil_mask(stencil_write);
            gl.stencil_func(glow::ALWAYS, 0, !0);
            gl.use_program(program);
            gl.bind_vertex_array(array);
        }
    }
}
