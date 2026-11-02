//! An optional glow for Halo 2 materials. Bloom passes (the `selfibloomination`
//! layer) write a mask to the frame's alpha. `rasterizer_bloom_postprocess`
//! downsamples it with its third built-in shader (`off_EBC81C[2]`), which keeps
//! `0.25 * mask` (`rasterizer_selfibloomination_scale` 0.25) and none of the
//! colour under it (`..._to_bloom_amount` 0), blurs that, and its fourth adds
//! twice the blurred mask back to every channel: a white glow. The same passes
//! bloom what is brighter than a threshold, `saturate(4 * (color - threshold))`,
//! added back at twice a brightness. A level's sky or fog may override both
//! (08b's control room: 0.3 and 0.2); a lone model takes what
//! `rasterizer_postprocess_scene` uses with no override.

use eframe::glow::{self, HasContext};

/// The blur amount `rasterizer_postprocess_scene` passes, in downsampled texels.
const BLUR_RADIUS: f32 = 3.992;
/// `rasterizer_bloom_default_threshold`.
const THRESHOLD: f32 = 0.9;
/// `sub_90C570`'s default 0.5 for both, as `rasterizer_postprocess_scene`
/// mixes them: (0.5 + 1) * 0.5 * 0.5.
const BRIGHTNESS: f32 = 0.375;
/// `rasterizer_selfibloomination_scale`, halved and doubled by the downsample.
const MASK_SCALE: f32 = 0.25;
const DOWNSAMPLE: i32 = 4;
const BLUR_PASSES: usize = 2;

const VERTEX: &str = "
out vec2 v_uv;
void main() {
    v_uv = vec2(gl_VertexID == 1 ? 2.0 : 0.0, gl_VertexID == 2 ? 2.0 : 0.0);
    gl_Position = vec4(v_uv * 2.0 - 1.0, 0.0, 1.0);
}
";

const FRAGMENT: &str = "
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_source;
uniform int u_mode;
uniform vec2 u_step;
uniform float u_scale;
uniform float u_threshold;
uniform float u_brightness;
void main() {
    if (u_mode == 0) {
        vec4 color = texture(u_source, v_uv);
        out_color = vec4(clamp(4.0 * (color.rgb - u_threshold), 0.0, 1.0), color.a * u_scale);
    } else if (u_mode == 1) {
        vec4 sum = texture(u_source, v_uv) * 0.2270270;
        const vec2 taps[4] = vec2[4](
            vec2(1.0, 0.1945946), vec2(2.0, 0.1216216),
            vec2(3.0, 0.0540541), vec2(4.0, 0.0162162));
        for (int tap = 0; tap < 4; tap++) {
            vec2 offset = u_step * taps[tap].x;
            sum += (texture(u_source, v_uv + offset) + texture(u_source, v_uv - offset)) * taps[tap].y;
        }
        out_color = sum;
    } else {
        // `pPixelShader_6`: twice the bright part and the self-bloom.
        vec4 bloom = texture(u_source, v_uv);
        out_color = vec4(2.0 * (bloom.rgb * u_brightness + bloom.a), 1.0);
    }
}
";

/// A color texture to draw into.
struct Surface {
    framebuffer: glow::NativeFramebuffer,
    color: glow::NativeTexture,
}

impl Surface {
    unsafe fn new(gl: &glow::Context, width: i32, height: i32) -> Option<Self> {
        unsafe {
            let color = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            let framebuffer = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            Some(Self { framebuffer, color })
        }
    }

    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.color);
        }
    }
}

pub(super) struct H2Bloom {
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
    surfaces: Option<([Surface; 2], [i32; 2])>,
}

impl H2Bloom {
    pub(super) unsafe fn new(gl: &glow::Context, version: &str) -> Option<Self> {
        unsafe {
            let program = gl.create_program().ok()?;
            let mut shaders = Vec::new();
            for (kind, source) in
                [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)]
            {
                let shader = gl.create_shader(kind).ok()?;
                gl.shader_source(shader, &format!("{version}{source}"));
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    eprintln!("model preview: bloom: {}", gl.get_shader_info_log(shader));
                    return None;
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            if !gl.get_program_link_status(program) {
                eprintln!("model preview: bloom: {}", gl.get_program_info_log(program));
                return None;
            }
            let vertex_array = gl.create_vertex_array().ok()?;
            Some(Self { program, vertex_array, surfaces: None })
        }
    }

    /// Add the glow of `scene`, whose alpha holds the mask, to the framebuffer
    /// bound for it. Leaves that framebuffer bound.
    pub(super) unsafe fn apply(
        &mut self,
        gl: &glow::Context,
        scene_framebuffer: glow::NativeFramebuffer,
        scene_color: glow::NativeTexture,
        size: [i32; 2],
        show_backfaces: bool,
    ) {
        let small = [(size[0] / DOWNSAMPLE).max(1), (size[1] / DOWNSAMPLE).max(1)];
        unsafe {
            if self.surfaces.as_ref().is_none_or(|(_, current)| *current != small) {
                if let Some((old, _)) = self.surfaces.take() {
                    old.iter().for_each(|surface| surface.delete(gl));
                }
                let (Some(first), Some(second)) = (
                    Surface::new(gl, small[0], small[1]),
                    Surface::new(gl, small[0], small[1]),
                ) else {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_framebuffer));
                    return;
                };
                self.surfaces = Some(([first, second], small));
            }
            let Some(([first, second], _)) = &self.surfaces else {
                return;
            };
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertex_array));
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);
            gl.depth_mask(false);
            gl.color_mask(true, true, true, true);
            gl.active_texture(glow::TEXTURE0);
            let uniform = |name: &str| gl.get_uniform_location(self.program, name);
            gl.uniform_1_i32(uniform("u_source").as_ref(), 0);
            let draw = |target: glow::NativeFramebuffer, source, mode: i32| {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target));
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                gl.uniform_1_i32(uniform("u_mode").as_ref(), mode);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            };

            gl.viewport(0, 0, small[0], small[1]);
            gl.uniform_1_f32(uniform("u_scale").as_ref(), MASK_SCALE);
            gl.uniform_1_f32(uniform("u_threshold").as_ref(), THRESHOLD);
            gl.uniform_1_f32(uniform("u_brightness").as_ref(), BRIGHTNESS);
            draw(first.framebuffer, scene_color, 0);
            // The nine taps reach four texels, as far as the game's blur.
            let reach = BLUR_RADIUS / 4.0;
            for _ in 0..BLUR_PASSES {
                gl.uniform_2_f32(uniform("u_step").as_ref(), reach / small[0] as f32, 0.0);
                draw(second.framebuffer, first.color, 1);
                gl.uniform_2_f32(uniform("u_step").as_ref(), 0.0, reach / small[1] as f32);
                draw(first.framebuffer, second.color, 1);
            }

            gl.viewport(0, 0, size[0], size[1]);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            // `sub_9206D0(19, 10)`, `(20, 2)`: inverse destination color, one.
            gl.blend_func(glow::ONE_MINUS_DST_COLOR, glow::ONE);
            gl.color_mask(true, true, true, false);
            draw(scene_framebuffer, first.color, 2);
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            gl.depth_mask(true);
            // What it switched off for the full-screen passes, back for
            // whatever draws the model after.
            gl.enable(glow::DEPTH_TEST);
            if show_backfaces {
                gl.disable(glow::CULL_FACE);
            } else {
                gl.enable(glow::CULL_FACE);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }
}
