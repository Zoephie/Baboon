//! An optional glow for Halo 2 materials. Bloom passes write a mask to the
//! frame's alpha; `rasterizer_bloom_postprocess` then blurs what the mask
//! lets through at a quarter of the size and adds it back. Its blur comes from
//! here, but its brightness tracks the scene's exposure, which a lone model
//! has none of, so `BRIGHTNESS` is this preview's own.

use eframe::glow::{self, HasContext};

/// The blur amount `rasterizer_postprocess_scene` passes, in downsampled texels.
const BLUR_RADIUS: f32 = 3.992;
const BRIGHTNESS: f32 = 1.0;
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
uniform vec2 u_curve;
void main() {
    if (u_mode == 0) {
        vec4 color = texture(u_source, v_uv);
        out_color = vec4(color.rgb * color.a, 1.0);
    } else if (u_mode == 3) {
        vec3 color = texture(u_source, v_uv).rgb;
        float maximum = max(max(color.r, color.g), color.b);
        float overwhite = max(maximum * u_curve.y, maximum - u_curve.x);
        out_color = vec4(maximum > 0.0 ? color * overwhite / maximum : vec3(0.0), 1.0);
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
        out_color = texture(u_source, v_uv) * u_scale;
    }
}
";

/// A color texture to draw into.
struct Surface {
    framebuffer: glow::NativeFramebuffer,
    color: glow::NativeTexture,
}

impl Surface {
    unsafe fn new(gl: &glow::Context, width: i32, height: i32, hdr: bool) -> Option<Self> {
        unsafe {
            let color = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                if hdr { glow::RGBA16F } else { glow::RGBA8 } as i32,
                width,
                height,
                0,
                glow::RGBA,
                if hdr { glow::FLOAT } else { glow::UNSIGNED_BYTE },
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
    hdr: bool,
}

impl H2Bloom {
    pub(super) unsafe fn new(gl: &glow::Context, version: &str, hdr: bool) -> Option<Self> {
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
            Some(Self { program, vertex_array, surfaces: None, hdr })
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
    ) {
        unsafe {
            let Some(blurred) = self.blur(gl, scene_framebuffer, scene_color, size, 0) else {
                return;
            };
            let uniform = |name: &str| gl.get_uniform_location(self.program, name);
            gl.viewport(0, 0, size[0], size[1]);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(true, true, true, false);
            gl.uniform_1_f32(uniform("u_scale").as_ref(), BRIGHTNESS);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_framebuffer));
            gl.bind_texture(glow::TEXTURE_2D, Some(blurred));
            gl.uniform_1_i32(uniform("u_mode").as_ref(), 2);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            gl.depth_mask(true);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// `apply_bloom_curve` over an HDR frame, blurred: what the final composite
    /// adds ahead of its tone curve. `curve` is the bloom point, then inherent
    /// bloom. Leaves the caller's scissor and viewport as they were.
    pub(super) unsafe fn blur_hdr(
        &mut self,
        gl: &glow::Context,
        scene_framebuffer: glow::NativeFramebuffer,
        accumulation: glow::NativeTexture,
        size: [i32; 2],
        curve: [f32; 2],
    ) -> Option<glow::NativeTexture> {
        unsafe {
            let scissored = gl.is_enabled(glow::SCISSOR_TEST);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(self.program));
            let location = gl.get_uniform_location(self.program, "u_curve");
            gl.uniform_2_f32_slice(location.as_ref(), &curve);
            let blurred = self.blur(gl, scene_framebuffer, accumulation, size, 3);
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            if scissored {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_framebuffer));
            gl.bind_texture(glow::TEXTURE_2D, None);
            blurred
        }
    }

    /// Run `source` through `first_mode` and the blur at a quarter of the size.
    unsafe fn blur(
        &mut self,
        gl: &glow::Context,
        scene_framebuffer: glow::NativeFramebuffer,
        source: glow::NativeTexture,
        size: [i32; 2],
        first_mode: i32,
    ) -> Option<glow::NativeTexture> {
        let small = [(size[0] / DOWNSAMPLE).max(1), (size[1] / DOWNSAMPLE).max(1)];
        unsafe {
            if self.surfaces.as_ref().is_none_or(|(_, current)| *current != small) {
                if let Some((old, _)) = self.surfaces.take() {
                    old.iter().for_each(|surface| surface.delete(gl));
                }
                let (Some(first), Some(second)) = (
                    Surface::new(gl, small[0], small[1], self.hdr),
                    Surface::new(gl, small[0], small[1], self.hdr),
                ) else {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_framebuffer));
                    return None;
                };
                self.surfaces = Some(([first, second], small));
            }
            let ([first, second], _) = self.surfaces.as_ref()?;
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
            draw(first.framebuffer, source, first_mode);
            // The nine taps reach four texels, as far as the game's blur.
            let reach = BLUR_RADIUS / 4.0;
            for _ in 0..BLUR_PASSES {
                gl.uniform_2_f32(uniform("u_step").as_ref(), reach / small[0] as f32, 0.0);
                draw(second.framebuffer, first.color, 1);
                gl.uniform_2_f32(uniform("u_step").as_ref(), 0.0, reach / small[1] as f32);
                draw(first.framebuffer, second.color, 1);
            }
            Some(first.color)
        }
    }
}
