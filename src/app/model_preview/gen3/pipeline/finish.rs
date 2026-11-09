//! The picture finished: distortion, the suck and vision effects, and the composite.

use crate::app::model_preview::gl::blit::empty_vertex_array;
use crate::app::model_preview::gl::blit::blit_depth;
use super::*;

const DISTORTION_FRAGMENT: &str = "#version 410 core
uniform sampler2D scene;
uniform sampler2D displacement;
out vec4 color;
void main() {
    vec2 lean = texelFetch(displacement, ivec2(gl_FragCoord.xy), 0).xy;
    if (lean == vec2(0.0)) {
        discard;
    }
    vec2 place = gl_FragCoord.xy / vec2(textureSize(scene, 0));
    color = texture(scene, place + vec2(lean.x, -lean.y));
}
";

/// `c_screen_postprocess::blur_display`, for how much of it covers the frame.
const BLUR_FRAGMENT: &str = "#version 410 core
uniform sampler2D picture;
uniform vec2 spread;
uniform float cover;
out vec4 color;
void main() {
    vec2 pixel = 1.0 / vec2(textureSize(picture, 0));
    vec2 place = gl_FragCoord.xy * pixel;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int y = -8; y <= 8; y++) {
        for (int x = -8; x <= 8; x++) {
            vec2 at = vec2(x, y) / 4.0;
            float weight = exp(-0.5 * dot(at, at));
            sum += texture(picture, place + at * spread * pixel * 8.0).rgb * weight;
            total += weight;
        }
    }
    color = vec4(sum / total, cover);
}
";

/// The preview lens' half angle, in `PERSPECTIVE_FOV_Y`'s terms.
pub(in crate::app::model_preview) const HALF_LENS: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// The finished picture's targets, and the copies a step over it reads.
pub(in crate::app::model_preview) struct PictureCopies {
    pub(in crate::app::model_preview) size: [i32; 2],
    pub(in crate::app::model_preview) picture_framebuffer: glow::NativeFramebuffer,
    /// What `screen_copy` copies the picture into.
    pub(in crate::app::model_preview) behind: glow::NativeTexture,
    behind_framebuffer: glow::NativeFramebuffer,
    accumulation_framebuffer: glow::NativeFramebuffer,
    /// What `copy_depth` copies the scene's depth into.
    pub(in crate::app::model_preview) depth_copy: glow::NativeTexture,
    depth_copy_framebuffer: glow::NativeFramebuffer,
}

impl PictureCopies {
    /// The picture into `behind`. A shader reads the picture through
    /// texcoords that run top down, so it is turned over unless `upright`.
    pub(in crate::app::model_preview) unsafe fn screen_copy(&self, gl: &glow::Context, upright: bool) {
        let size = self.size;
        unsafe {
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.picture_framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.behind_framebuffer));
            let (top, bottom) = if upright { (0, size[1]) } else { (size[1], 0) };
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, top, size[0], bottom, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.picture_framebuffer));
            gl.enable(glow::FRAMEBUFFER_SRGB);
        }
    }

    /// The scene's depth and stencil into `depth_copy`.
    pub(in crate::app::model_preview) unsafe fn copy_depth(&self, gl: &glow::Context) {
        unsafe {
            blit_depth(gl, self.accumulation_framebuffer, self.depth_copy_framebuffer, self.size);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.picture_framebuffer));
        }
    }
}

impl Gen3State {
    /// `apply_distortions`: each pixel the distortions leaned reads the scene
    /// from where they leaned it.
    pub(super) unsafe fn apply_distortions(
        &mut self,
        gl: &glow::Context,
        accumulation_target: glow::NativeFramebuffer,
        size: [i32; 2],
    ) {
        unsafe {
            if self.distortion_program.is_none() {
                self.distortion_program = screen_program(gl, DISTORTION_FRAGMENT);
            }
            let (Some(program), Some(targets)) = (self.distortion_program, self.targets.as_ref()) else {
                return;
            };
            let [width, height] = size;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(accumulation_target));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.scene_copy_framebuffer));
            gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accumulation_target));
            gl.use_program(Some(program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(targets.scene_copy));
            gl.uniform_1_i32(gl.get_uniform_location(program, "scene").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(targets.distortion));
            gl.uniform_1_i32(gl.get_uniform_location(program, "displacement").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_player_view`'s last steps: `render_screen_shaders`, then the blur, on
    /// a surface that holds light rather than the encoded frame, as the game's
    /// sRGB one does.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn finish_picture(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        framebuffer: glow::NativeFramebuffer,
        size: [i32; 2],
        screen: &gen3::effects::screen::Gen3ScreenSettings,
        shaders: &[gen3::effects::screen::Gen3ScreenShader],
    ) {
        let blur = screen.blur();
        if !shaders.iter().any(gen3::effects::screen::Gen3ScreenShader::at_frame_end)
            && blur.is_none()
            && !game.picture_effects(frame, screen)
        {
            return;
        }
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let (behind, picture_framebuffer, behind_framebuffer) =
            (targets.behind, targets.picture_framebuffer, targets.behind_framebuffer);
        let (accumulation_framebuffer, depth_copy, depth_copy_framebuffer) =
            (targets.accumulation_framebuffer, targets.depth_copy, targets.depth_copy_framebuffer);
        unsafe {
            // Raw bytes across, so what was encoded reads back as light.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(picture_framebuffer));
            gl.disable(glow::FRAMEBUFFER_SRGB);
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
            gl.enable(glow::FRAMEBUFFER_SRGB);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.depth_mask(false);
            gl.color_mask(true, true, true, false);
            gl.bind_vertex_array(Some(self.empty_array(gl)));
            let copies = PictureCopies {
                size,
                picture_framebuffer,
                behind,
                behind_framebuffer,
                accumulation_framebuffer,
                depth_copy,
                depth_copy_framebuffer,
            };
            game.before_screen_shaders(&mut Gen3Cx { gl, frame, core, slot, state: self }, screen, &copies);
            for shader in shaders.iter().filter(|shader| shader.at_frame_end()) {
                let screen = u32::from_be_bytes(*b"rmss");
                let found = frame
                    .preview
                    .materials
                    .iter()
                    .position(|material| material.shader_group == screen && material.shader_path == shader.path);
                let textures = frame.textures.as_ref();
                let material = found.and_then(|at| textures?.get(at)?.h3.as_ref().map(|material| (at, material)));
                let Some((at, material)) = material else {
                    continue;
                };
                copies.screen_copy(gl, false);
                gen3_set_blend(gl, material.blend);
                self.screen_alpha = Some(shader.alpha);
                let batch = RenderModelPreviewBatch { material_index: at as u16, ..Default::default() };
                self.draw_pass(core, gl, frame, slot, &batch, material, &material.lit, None);
                self.screen_alpha = None;
            }
            if let Some((spread, cover)) = blur {
                if self.blur_program.is_none() {
                    self.blur_program = screen_program(gl, BLUR_FRAGMENT);
                }
                if let Some(program) = self.blur_program {
                    copies.screen_copy(gl, true);
                    gl.use_program(Some(program));
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(behind));
                    gl.uniform_1_i32(gl.get_uniform_location(program, "picture").as_ref(), 0);
                    gl.uniform_2_f32_slice(gl.get_uniform_location(program, "spread").as_ref(), &spread);
                    gl.uniform_1_f32(gl.get_uniform_location(program, "cover").as_ref(), cover);
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    gl.bind_vertex_array(Some(self.empty_array(gl)));
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }
            gl.disable(glow::FRAMEBUFFER_SRGB);
            game.after_blur(&mut Gen3Cx { gl, frame, core, slot, state: self }, screen, &copies);
            gl.color_mask(true, true, true, true);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(picture_framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuffer));
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.enable(glow::DEPTH_TEST);
        }
    }

    pub(in crate::app::model_preview) unsafe fn empty_array(&mut self, gl: &glow::Context) -> glow::NativeVertexArray {
        unsafe {
            empty_vertex_array(gl, &mut self.empty_vertex_array)
        }
    }
}
