//! The picture finished: distortion, the suck and vision effects, and the composite.

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

/// `displacement_motion_blur`'s `default_ps`, with the previous frame's view
/// being this one's with the world moved by `moved`. Every fragment here has
/// the alpha the shader's mask looks for.
const SUCK_FRAGMENT: &str = "#version 410 core
uniform sampler2D picture;
uniform sampler2D depth;
uniform mat4 screen_to_world;
uniform mat4 world_to_screen;
uniform vec3 moved;
uniform float most_pixels;
uniform float blur_step;
uniform vec2 center_falloff;
out vec4 color;
void main() {
    vec2 size = vec2(textureSize(picture, 0));
    ivec2 at = ivec2(gl_FragCoord.xy);
    vec3 center = texelFetch(picture, at, 0).rgb;
    vec2 from_center = (gl_FragCoord.xy - size * 0.5) * sqrt(center_falloff) / size;
    float weight = clamp(clamp(dot(from_center, from_center), 0.0, 1.0) - 0.1, 0.0, 1.0);
    color = vec4(center, 1.0);
    if (weight > 0.0) {
        float raw = texelFetch(depth, at, 0).r;
        vec4 place = screen_to_world * vec4(gl_FragCoord.xy / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
        vec4 before = world_to_screen * vec4(place.xyz / place.w + moved, 1.0);
        vec2 delta = gl_FragCoord.xy - (before.xy / before.w * 0.5 + 0.5) * size;
        // Still and with no blur allowed is 0 / 0, which would poison the steps.
        float scale = clamp(most_pixels / max(length(delta), 1e-6), 0.0, 1.0);
        vec2 step_by = delta * blur_step * scale * weight;
        vec3 sum = center;
        for (int sample_at = 1; sample_at <= 6; sample_at++) {
            ivec2 there = clamp(ivec2(gl_FragCoord.xy + step_by * float(sample_at)), ivec2(0), ivec2(size) - 1);
            sum += texelFetch(picture, there, 0).rgb;
        }
        color.rgb = sum / 7.0;
    }
}
";

/// `hud_camera_nightvision`'s `default_ps`, its narrow filter. The game's depth
/// runs far to near, so the edges are of `game_depth`, not of this one.
const VISION_FRAGMENT: &str = "#version 410 core
uniform sampler2D depth;
uniform sampler2D mask;
uniform mat4 screen_to_world;
uniform vec3 eye;
uniform vec3 forward;
uniform vec2 planes;
uniform float falloff;
uniform vec3 ping;
uniform vec4 default_color;
uniform vec4 ping_color;
out vec4 color;
vec3 world_at(ivec2 at, float raw) {
    vec2 size = vec2(textureSize(depth, 0));
    vec4 place = screen_to_world * vec4((vec2(at) + 0.5) / size * 2.0 - 1.0, raw * 2.0 - 1.0, 1.0);
    return place.xyz / place.w;
}
float game_depth(ivec2 at) {
    float raw = texelFetch(depth, at, 0).r;
    if (raw >= 1.0) {
        return 0.0;
    }
    float along = dot(world_at(at, raw) - eye, forward);
    return planes.x * (planes.y - along) / (along * (planes.y - planes.x));
}
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float raw = texelFetch(depth, at, 0).r;
    vec2 place = gl_FragCoord.xy / vec2(textureSize(depth, 0));
    float cover = texture(mask, vec2(place.x, 1.0 - place.y)).r;
    if (raw >= 1.0 || cover <= 0.0) {
        discard;
    }
    float middle = game_depth(at);
    vec2 laplacian = vec2(
        game_depth(at + ivec2(1, 0)) + game_depth(at - ivec2(1, 0)) - 2.0 * middle,
        game_depth(at + ivec2(0, 1)) + game_depth(at - ivec2(0, 1)) - 2.0 * middle);
    float gradient = clamp(length(laplacian) / middle, 0.0, 1.0);
    if (gradient <= 1.0 / 255.0) {
        discard;
    }
    if (laplacian.x + laplacian.y > 0.0) {
        gradient *= ping_color.a;
    }
    float distance_away = length(world_at(at, raw) - eye);
    cover *= pow(2.0, -pow(distance_away * falloff, 8.0));
    float after = ping.x - distance_away;
    float boost = pow(clamp(1.0 + ping.z * after, 0.0, 1.0), 4.0) * step(distance_away, ping.x);
    cover *= step(distance_away, ping.y);
    color = vec4(gradient * (default_color.rgb + ping_color.rgb * boost), default_color.a) * cover;
}
";

/// The preview lens' half angle, in `PERSPECTIVE_FOV_Y`'s terms.
pub(in crate::app::model_preview) const HALF_LENS: f32 = 30.0 * std::f32::consts::PI / 180.0;

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
        screen: &gen3::screen_effects::H3ScreenSettings,
        shaders: &[gen3::screen_effects::H3ScreenShader],
    ) {
        let blur = screen.blur();
        let vision = frame.preview.h3.vision.clone().filter(|_| screen.vision() > 0.0);
        let suck = frame.preview.h3.motion_blur.zip(screen.suck());
        if !shaders.iter().any(gen3::screen_effects::H3ScreenShader::at_frame_end)
            && blur.is_none()
            && vision.is_none()
            && suck.is_none()
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
            // A shader reads the picture through texcoords that run top down.
            let screen_copy = |gl: &glow::Context, upright: bool| {
                gl.disable(glow::FRAMEBUFFER_SRGB);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(picture_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(behind_framebuffer));
                let (top, bottom) = if upright { (0, size[1]) } else { (size[1], 0) };
                gl.blit_framebuffer(0, 0, size[0], size[1], 0, top, size[0], bottom, glow::COLOR_BUFFER_BIT, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
                gl.enable(glow::FRAMEBUFFER_SRGB);
            };
            let copy_depth = |gl: &glow::Context| {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(picture_framebuffer));
            };
            // `distortion_apply` comes before everything else here.
            if let Some((blur, moved)) = suck {
                copy_depth(gl);
                screen_copy(gl, true);
                self.draw_suck(gl, frame, &blur, moved, behind, depth_copy);
            }
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
                screen_copy(gl, false);
                h3_set_blend(gl, material.blend);
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
                    screen_copy(gl, true);
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
            if let Some(vision) = vision {
                copy_depth(gl);
                let on_for = game.vision_on_for(slot, core.slot_uploads(slot), frame.time);
                self.draw_vision(gl, frame, &vision, screen.vision(), on_for, depth_copy);
            }
            gl.color_mask(true, true, true, true);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(picture_framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(framebuffer));
            gl.blit_framebuffer(0, 0, size[0], size[1], 0, 0, size[0], size[1], glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.enable(glow::DEPTH_TEST);
        }
    }

    /// `c_view::distortion_apply`'s motion blur, for what moved the view and not
    /// for the preview's own camera.
    unsafe fn draw_suck(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        blur: &h3::vision::H3MotionBlur,
        moved: [f32; 3],
        picture: glow::NativeTexture,
        depth: glow::NativeTexture,
    ) {
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = h3::vision::inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.suck_program.is_none() {
                self.suck_program = screen_program(gl, SUCK_FRAGMENT);
            }
            let Some(program) = self.suck_program else {
                return;
            };
            gl.use_program(Some(program));
            let spot = |name: &str| gl.get_uniform_location(program, name);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.uniform_1_i32(spot("depth").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(picture));
            gl.uniform_1_i32(spot("picture").as_ref(), 0);
            gl.uniform_matrix_4_f32_slice(spot("screen_to_world").as_ref(), false, &to_world);
            gl.uniform_matrix_4_f32_slice(spot("world_to_screen").as_ref(), false, &matrix);
            gl.uniform_3_f32_slice(spot("moved").as_ref(), &moved);
            gl.uniform_1_f32(spot("most_pixels").as_ref(), blur.most_pixels);
            gl.uniform_1_f32(spot("blur_step").as_ref(), blur.step);
            gl.uniform_2_f32_slice(spot("center_falloff").as_ref(), &blur.center_falloff);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// `c_hud_camera_view`: over the frame as it will show, not over its light.
    unsafe fn draw_vision(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        vision: &h3::vision::H3Vision,
        amount: f32,
        on_for: f32,
        depth: glow::NativeTexture,
    ) {
        let constants = vision.constants(amount, on_for);
        let matrix = view_projection(&frame.camera);
        let Some(to_world) = h3::vision::inverted(&matrix) else {
            return;
        };
        unsafe {
            if self.vision_program.is_none() {
                self.vision_program = screen_program(gl, VISION_FRAGMENT);
            }
            if self.vision_mask.is_none() {
                let Some((image, texels, mips)) = vision.mask() else {
                    return;
                };
                self.vision_mask = upload_h2_image_linear(gl, image, *texels, mips, None);
            }
            let (Some(program), Some(mask)) = (self.vision_program, self.vision_mask) else {
                return;
            };
            let view = ce_flare_view(&frame.camera);
            gl.use_program(Some(program));
            let spot = |name: &str| gl.get_uniform_location(program, name);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.uniform_1_i32(spot("depth").as_ref(), 0);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(mask));
            gl.uniform_1_i32(spot("mask").as_ref(), 1);
            gl.active_texture(glow::TEXTURE0);
            gl.uniform_matrix_4_f32_slice(spot("screen_to_world").as_ref(), false, &to_world);
            gl.uniform_3_f32_slice(spot("eye").as_ref(), &view.eye);
            gl.uniform_3_f32_slice(spot("forward").as_ref(), &view.forward);
            gl.uniform_2_f32(spot("planes").as_ref(), h3::vision::GAME_NEAR, h3::vision::GAME_FAR);
            gl.uniform_1_f32(spot("falloff").as_ref(), constants.falloff);
            gl.uniform_3_f32_slice(spot("ping").as_ref(), &constants.ping);
            gl.uniform_4_f32_slice(spot("default_color").as_ref(), &constants.color);
            gl.uniform_4_f32_slice(spot("ping_color").as_ref(), &constants.ping_color);
            // `_alpha_blend_multiply_add`, which is premultiplied.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(self.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    pub(in crate::app::model_preview) unsafe fn empty_array(&mut self, gl: &glow::Context) -> glow::NativeVertexArray {
        unsafe {
            *self.empty_vertex_array.get_or_insert_with(|| gl.create_vertex_array().expect("a vertex array"))
        }
    }
}
