//! Halo 4's object self shadow on the GPU (`c_lightmap_shadows_view::render`,
//! X360 0x829C7E18): the object's depth from the light through its materials'
//! `shadow_generate`, blurred into a variance map (explicit 85
//! `shadow_generate_variance`), then `shadow_apply_variance` (explicit 84, or
//! 87 without its normal test) into the mask the static pass reads. See
//! `docs/h4-preview-research/shadow-camo-shields.md` §1.

use super::h4_shadow::{DEPTH_BIAS, FADE, H4ShadowCamera, SLOPE_BIAS, VSM};
use super::*;
use h3_shaders::H3Material;

/// `HorizontalBlurVariancePS`: five depths across, weighted by
/// `ps_gaussian_parameters[0]`, and their squares.
const HORIZONTAL_FRAGMENT: &str = "#version 410 core
uniform sampler2D depth_map;
uniform vec4 gaussian;
out vec4 color;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float d[5];
    for (int tap = 0; tap < 5; tap++) {
        d[tap] = texelFetch(depth_map, clamp(at + ivec2(tap - 2, 0), ivec2(0), textureSize(depth_map, 0) - 1), 0).r;
    }
    vec4 weights = vec4(gaussian.x, gaussian.y, gaussian.z, gaussian.y);
    float z = dot(vec4(d[0], d[1], d[2], d[3]), weights) + d[4] * gaussian.x;
    float z2 = dot(vec4(d[0], d[1], d[2], d[3]) * vec4(d[0], d[1], d[2], d[3]), weights) + d[4] * d[4] * gaussian.x;
    color = vec4(z, z2, 0.0, 0.0);
}
";

/// `VerticalBlurVariancePS`: four filtered reads down at
/// `ps_gaussian_parameters[1]`, weighted (2, 6, 6, 2) / 16.
const VERTICAL_FRAGMENT: &str = "#version 410 core
uniform sampler2D source_map;
uniform vec4 offsets;
out vec4 color;
void main() {
    vec2 uv = gl_FragCoord.xy / vec2(textureSize(source_map, 0));
    vec2 t0 = texture(source_map, uv + vec2(0.0, offsets.x)).xy;
    vec2 t1 = texture(source_map, uv + vec2(0.0, offsets.y)).xy;
    vec2 t2 = texture(source_map, uv + vec2(0.0, offsets.z)).xy;
    vec2 t3 = texture(source_map, uv + vec2(0.0, offsets.w)).xy;
    vec4 weights = vec4(2.0, 6.0, 6.0, 2.0) / 16.0;
    color = vec4(dot(vec4(t0.x, t1.x, t2.x, t3.x), weights), dot(vec4(t0.y, t1.y, t2.y, t3.y), weights), 0.0, 0.0);
}
";

/// `shadow_apply.fx`'s `default_ps` with `shadow_apply_variance.fx`'s
/// `ComputeVarianceAttenuation` (`FASTER_SHADOWS`, `VARIANCE`). The pixels it
/// touches are the scene's inside the shadow's box, which the game finds
/// with a stencil over the box's faces. It returns `(darken, knockout +
/// darken, darken, 0)`, of which the game keeps G and B, the least of each.
const APPLY_FRAGMENT: &str = "#version 410 core
uniform sampler2D scene_depth;
uniform sampler2D normals;
uniform sampler2D variance;
uniform mat4 screen_to_world;
uniform mat4 shadow_view_projection;
uniform vec3 shadow_direction;
uniform float shadow_alpha;
uniform vec2 fade;
uniform vec3 vsm;
uniform float knockout;
uniform int normal_test;
out vec4 color;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    float depth = texelFetch(scene_depth, at, 0).r;
    if (depth >= 1.0) {
        discard;
    }
    vec4 world = screen_to_world * vec4(gl_FragCoord.xy, depth, 1.0);
    world /= world.w;
    vec4 clip = shadow_view_projection * vec4(world.xyz, 1.0);
    if (any(greaterThan(abs(clip.xyz), vec3(1.0)))) {
        discard;
    }
    vec3 shadow = vec3(clip.xy * 0.5 + 0.5, clip.z * 0.5 + 0.5);
    float falloff = max(shadow.z * fade.x + fade.y, 0.0);
    falloff = falloff * falloff / 2.25;
    float darkness = shadow_alpha * (1.0 - falloff);
    // `DecodeNormal`, into the space the normal buffer was written in.
    vec2 expanded = texelFetch(normals, at, 0).xy * 2.0 - 1.0;
    float f = dot(expanded, expanded);
    vec3 normal = vec3(expanded * sqrt(max(4.0 - 4.0 * f, 0.0)), 1.0 - 2.0 * f);
    float cosine = dot(normal, shadow_direction);
    if (normal_test != 0) {
        darkness *= clamp(0.65 + cosine * 5.0, 0.0, 1.0);
    }
    float darken = 1.0;
    if (darkness > 0.001) {
        shadow.z = min(shadow.z, 0.99);
        vec2 moments = texture(variance, shadow.xy).xy;
        float closer = 1.0;
        if (shadow.z > moments.x) {
            float spread = clamp(moments.y - moments.x * moments.x + vsm.x, 0.0, 1.0);
            float d2 = pow(shadow.z - moments.x, vsm.y);
            closer = pow(spread / (spread + d2), vsm.z);
        }
        darken = clamp(1.01 - darkness + closer * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(darken, knockout + darken, darken, 0.0);
}
";

/// The depth map, its two blurs, and the mask, kept between frames.
pub(super) struct H4ShadowTargets {
    map_size: i32,
    depth: glow::NativeTexture,
    depth_framebuffer: glow::NativeFramebuffer,
    blurs: [(glow::NativeTexture, glow::NativeFramebuffer); 2],
    mask_size: [i32; 2],
    mask: glow::NativeTexture,
    mask_framebuffer: glow::NativeFramebuffer,
}

unsafe fn colour_target(
    gl: &glow::Context,
    size: [i32; 2],
    format: u32,
    filter: u32,
) -> Option<(glow::NativeTexture, glow::NativeFramebuffer)> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        let (layout, kind) = if format == glow::RGBA8 {
            (glow::RGBA, glow::UNSIGNED_BYTE)
        } else {
            (glow::RG, glow::FLOAT)
        };
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            format as i32,
            size[0],
            size[1],
            0,
            layout,
            kind,
            glow::PixelUnpackData::Slice(None),
        );
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, filter),
            (glow::TEXTURE_MAG_FILTER, filter),
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
            Some(texture),
            0,
        );
        Some((texture, framebuffer))
    }
}

impl H4ShadowTargets {
    unsafe fn new(gl: &glow::Context, map_size: i32, mask_size: [i32; 2]) -> Option<Self> {
        unsafe {
            let depth = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::DEPTH_COMPONENT24 as i32,
                map_size,
                map_size,
                0,
                glow::DEPTH_COMPONENT,
                glow::UNSIGNED_INT,
                glow::PixelUnpackData::Slice(None),
            );
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::NEAREST as i32);
            }
            let depth_framebuffer = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(depth_framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(depth),
                0,
            );
            gl.draw_buffers(&[glow::NONE]);
            gl.read_buffer(glow::NONE);
            let square = [map_size, map_size];
            let first = colour_target(gl, square, glow::RG32F, glow::NEAREST)?;
            // The vertical blur reads filtered, as does the apply.
            let second = colour_target(gl, square, glow::RG32F, glow::LINEAR)?;
            gl.bind_texture(glow::TEXTURE_2D, Some(first.0));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            let (mask, mask_framebuffer) =
                colour_target(gl, mask_size, glow::RGBA8, glow::NEAREST)?;
            gl.bind_texture(glow::TEXTURE_2D, None);
            let framebuffers = [depth_framebuffer, first.1, second.1, mask_framebuffer];
            let complete = framebuffers.iter().all(|framebuffer| {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(*framebuffer));
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE
            });
            let targets = Self {
                map_size,
                depth,
                depth_framebuffer,
                blurs: [first, second],
                mask_size,
                mask,
                mask_framebuffer,
            };
            if !complete {
                targets.delete(gl);
                return None;
            }
            Some(targets)
        }
    }

    pub(super) unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.depth_framebuffer);
            gl.delete_texture(self.depth);
            for (texture, framebuffer) in self.blurs {
                gl.delete_framebuffer(framebuffer);
                gl.delete_texture(texture);
            }
            gl.delete_framebuffer(self.mask_framebuffer);
            gl.delete_texture(self.mask);
        }
    }
}

impl ModelGlRenderer {
    /// The object's shadow into its mask, if it casts one: the mask, or none.
    /// Reads the depth and normals the albedo pass left.
    pub(super) unsafe fn draw_h4_shadow(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        batches: &[(RenderModelPreviewBatch, Arc<H3Material>)],
    ) -> Option<glow::NativeTexture> {
        let caster = frame.preview.h4_shadow_caster?;
        let (size, albedo_framebuffer, normal, depth_copy, depth_copy_framebuffer) =
            self.h3_targets.as_ref().map(|targets| {
                (
                    targets.size,
                    targets.albedo_framebuffer,
                    targets.normal,
                    targets.depth_copy,
                    targets.depth_copy_framebuffer,
                )
            })?;
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let radius = (0..3)
            .map(|axis| (high[axis] - center[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        let camera = H4ShadowCamera::new(H4ShadowCamera::DOWN, center, radius);
        let casters: Vec<_> = batches
            .iter()
            .filter(|(_, material)| {
                material.blend == h3_shaders::H3Blend::Opaque && !material.decal
            })
            .filter_map(|(batch, material)| {
                Some((
                    batch,
                    material,
                    Arc::clone(material.shadow_generate.as_ref()?),
                ))
            })
            .collect();
        if casters.is_empty() {
            return None;
        }
        unsafe {
            let stale = self.h4_shadow.as_ref().is_some_and(|targets| {
                targets.map_size != camera.pixels || targets.mask_size != size
            });
            if stale && let Some(old) = self.h4_shadow.take() {
                old.delete(gl);
            }
            if self.h4_shadow.is_none() {
                self.h4_shadow = H4ShadowTargets::new(gl, camera.pixels, size);
            }
            let targets = self.h4_shadow.as_ref()?;
            let (depth, depth_framebuffer, blurs, mask, mask_framebuffer) = (
                targets.depth,
                targets.depth_framebuffer,
                targets.blurs,
                targets.mask,
                targets.mask_framebuffer,
            );
            if self.h4_shadow_programs.is_none() {
                self.h4_shadow_programs = Some(
                    [HORIZONTAL_FRAGMENT, VERTICAL_FRAGMENT, APPLY_FRAGMENT]
                        .map(|fragment| h3_draw::screen_program(gl, fragment)),
                );
            }
            let [Some(horizontal), Some(vertical), Some(apply)] = self.h4_shadow_programs? else {
                return None;
            };
            // `render_shadow_generate` (X360 0x83A99FA0): depth alone, biased.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(depth_framebuffer));
            gl.viewport(0, 0, camera.pixels, camera.pixels);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(false, false, false, false);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(SLOPE_BIAS, DEPTH_BIAS * (1 << 24) as f32);
            h3_draw::h3_set_cull(gl, frame);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            self.reach_view_projection = Some(camera.view_projection);
            for (batch, material, pass) in &casters {
                self.h3_draw_pass(gl, frame, slot, batch, material, pass, None);
            }
            self.reach_view_projection = None;
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.bind_vertex_array(Some(self.h3_empty_array(gl)));
            // Explicit 85: the horizontal blur from the depth, then the vertical.
            let gaussian = camera.gaussian();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blurs[0].1));
            gl.use_program(Some(horizontal));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(depth));
            gl.uniform_1_i32(gl.get_uniform_location(horizontal, "depth_map").as_ref(), 0);
            gl.uniform_4_f32_slice(
                gl.get_uniform_location(horizontal, "gaussian").as_ref(),
                &gaussian[0],
            );
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blurs[1].1));
            gl.use_program(Some(vertical));
            gl.bind_texture(glow::TEXTURE_2D, Some(blurs[0].0));
            gl.uniform_1_i32(gl.get_uniform_location(vertical, "source_map").as_ref(), 0);
            gl.uniform_4_f32_slice(
                gl.get_uniform_location(vertical, "offsets").as_ref(),
                &gaussian[1],
            );
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.viewport(0, 0, size[0], size[1]);
            // The scene's depth, to find each pixel's place in the world.
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(albedo_framebuffer));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(depth_copy_framebuffer));
            let bits = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
            gl.blit_framebuffer(
                0,
                0,
                size[0],
                size[1],
                0,
                0,
                size[0],
                size[1],
                bits,
                glow::NEAREST,
            );
            // `setup_targets_shadow_mask`'s white, then the apply: blend MIN,
            // green and blue alone.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(mask_framebuffer));
            gl.clear_color(1.0, 1.0, 1.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(apply));
            let at = |name: &str| gl.get_uniform_location(apply, name);
            for (unit, (name, texture)) in [
                ("scene_depth", depth_copy),
                ("normals", normal),
                ("variance", blurs[1].0),
            ]
            .into_iter()
            .enumerate()
            {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.uniform_1_i32(at(name).as_ref(), unit as i32);
            }
            gl.active_texture(glow::TEXTURE0);
            let screen_to_world = reach_light_pass::screen_to_relative_world(
                &h3_draw::view_projection(&frame.camera),
                size,
                [0.0; 3],
            )?;
            gl.uniform_matrix_4_f32_slice(
                at("screen_to_world").as_ref(),
                true,
                screen_to_world.as_flattened(),
            );
            gl.uniform_matrix_4_f32_slice(
                at("shadow_view_projection").as_ref(),
                false,
                &camera.view_projection,
            );
            let axis = h3_draw::h4_matrix(frame, "ps_worldspace_normal_axis")?;
            let axis: [f32; 9] = axis.try_into().ok()?;
            gl.uniform_3_f32_slice(at("shadow_direction").as_ref(), &camera.direction_in(&axis));
            // `min(1, 2·fade)` with the fade byte full at a preview's framing
            // [inferred: its H4 formula is not read].
            gl.uniform_1_f32(at("shadow_alpha").as_ref(), 1.0);
            gl.uniform_2_f32_slice(at("fade").as_ref(), &FADE);
            gl.uniform_3_f32_slice(at("vsm").as_ref(), &VSM);
            // No level puts the object in no sun.
            gl.uniform_1_f32(at("knockout").as_ref(), 0.0);
            gl.uniform_1_i32(at("normal_test").as_ref(), i32::from(!caster.hires));
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::MIN);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(false, true, true, false);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(self.slots[slot].vertex_array));
            Some(mask)
        }
    }
}
