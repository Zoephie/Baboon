//! What Reach's and Halo 4's object self shadows share around their own maps
//! and applies: the casters, `render_shadow_generate`'s biased depth, the
//! scene's depth, and an apply blended by its least into the mask.

use eframe::glow::{self, HasContext as _};
use super::state::Gen3State;
use super::super::*;
use super::shaders::{H3Material, H3Pass};
use super::super::dx9::shadow::ShadowStyle;

/// Where Halo 3's and ODST's shadow apply reads the albedo pass's normals.
pub(in crate::app::model_preview) const H3_NORMAL_TARGET_UNIT: u32 = 9;

/// `c_rasterizer::render_globals.k_shadow_resolution`.
pub(in crate::app::model_preview) const H3_SHADOW_MAP_SIZE: i32 = 512;

/// `set_shadow_generate_mode`'s bias, the second in depth units.
pub(in crate::app::model_preview) const H3_SHADOW_SLOPE_BIAS: f32 = 1.414;

pub(in crate::app::model_preview) const H3_SHADOW_DEPTH_BIAS: f32 = 0.003;

/// Halo 3's and ODST's self shadow through the dx9 passes: a smaller map,
/// biased, from the dominant probe light, and applied after the opaque pass
/// rather than masked in the stencil.
pub(in crate::app::model_preview) fn h3_shadow_style(frame: &ModelGpuFrame) -> ShadowStyle {
    let (toward, alpha) = gen3::pipeline::h3_shadow_light(frame);
    ShadowStyle {
        map_size: H3_SHADOW_MAP_SIZE,
        bias: Some((H3_SHADOW_SLOPE_BIAS, H3_SHADOW_DEPTH_BIAS * (1 << 24) as f32)),
        stencil_mask: false,
        toward,
        alpha,
    }
}

/// A batch that casts, its material, and that material's `shadow_generate`.
pub(in crate::app::model_preview) type ShadowCaster<'a> = (&'a RenderModelPreviewBatch, &'a Arc<H3Material>, Arc<H3Pass>);

/// The batches that cast: opaque, no decal, and with a `shadow_generate`.
pub(in crate::app::model_preview) fn shadow_casters(batches: &[(RenderModelPreviewBatch, Arc<H3Material>)]) -> Vec<ShadowCaster<'_>> {
    batches
        .iter()
        .filter(|(_, material)| material.blend == gen3::shaders::H3Blend::Opaque && !material.decal)
        .filter_map(|(batch, material)| Some((batch, material, Arc::clone(material.shadow_generate.as_ref()?))))
        .collect()
}

impl Gen3State {
    /// `render_shadow_generate`: `casters`' depth alone into `framebuffer`, a
    /// square of `pixels`, seen through `view_projection`, biased by `slope`
    /// and by `depth` in depth units, culled as ever. Leaves depth testing on
    /// and every channel writable.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn generate_shadow_depth(
        &mut self,
        core: &mut RenderCore,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        casters: &[ShadowCaster<'_>],
        framebuffer: glow::NativeFramebuffer,
        pixels: i32,
        view_projection: [f32; 16],
        [slope, depth]: [f32; 2],
    ) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.viewport(0, 0, pixels, pixels);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(false, false, false, false);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(slope, depth * (1 << 24) as f32);
            super::pipeline::h3_set_cull(gl, frame);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            self.shadow_view_projection = Some(view_projection);
            for (batch, material, pass) in casters {
                self.draw_pass(core, gl, frame, slot, batch, material, pass, None);
            }
            self.shadow_view_projection = None;
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.color_mask(true, true, true, true);
        }
    }

    /// The scene's depth and stencil, `size` of them, from `scene` (a pass's
    /// framebuffer that holds them) into `depth_copy`, to find each pixel's
    /// place in the world. Leaves the viewport at `size`.
    pub(in crate::app::model_preview) unsafe fn copy_scene_depth(
        gl: &glow::Context,
        size: [i32; 2],
        scene: glow::NativeFramebuffer,
        depth_copy: glow::NativeFramebuffer,
    ) {
        unsafe {
            gl.viewport(0, 0, size[0], size[1]);
            super::super::gl::blit::blit_depth(gl, scene, depth_copy, size);
        }
    }

    /// The apply's start: `mask` cleared to `clear` (`setup_targets_shadow_mask`),
    /// `program` bound with `textures` on units 0 up, and the frame's
    /// `screen_to_world` and the light's `shadow_view_projection` set.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn begin_shadow_apply(
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        size: [i32; 2],
        mask: glow::NativeFramebuffer,
        clear: [f32; 4],
        program: glow::NativeProgram,
        textures: [(&str, glow::NativeTexture); 3],
        view_projection: &[f32; 16],
    ) -> Option<()> {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(mask));
            gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(program));
            let at = |name: &str| gl.get_uniform_location(program, name);
            for (unit, (name, texture)) in textures.into_iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.uniform_1_i32(at(name).as_ref(), unit as i32);
            }
            gl.active_texture(glow::TEXTURE0);
            let screen_to_world =
                super::screen_lights::screen_to_relative_world(&super::pipeline::view_projection(&frame.camera), size, [0.0; 3])?;
            gl.uniform_matrix_4_f32_slice(at("screen_to_world").as_ref(), true, screen_to_world.as_flattened());
            gl.uniform_matrix_4_f32_slice(at("shadow_view_projection").as_ref(), false, view_projection);
            Some(())
        }
    }

    /// The apply's triangle, kept to `channels` and the least of each, then
    /// the state the scene draws with again and the slot's vertex array.
    pub(in crate::app::model_preview) unsafe fn draw_shadow_apply(&mut self, core: &mut RenderCore, gl: &glow::Context, slot: usize, [red, green, blue, alpha]: [bool; 4]) {
        unsafe {
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::MIN);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(red, green, blue, alpha);
            gl.bind_vertex_array(Some(self.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.blend_equation(glow::FUNC_ADD);
            gl.color_mask(true, true, true, true);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
        }
    }
}

pub(in crate::app::model_preview) const H3_SHADOW_APPLY_BIAS: f32 = -0.00002;

/// `shadow_apply_bilinear`'s pixel shader in the preview's box: one object
/// filling the map. `FASTER_SHADOWS` at `DX_VERSION` 11 leaves no slope to bias
/// by, that being the shadow generate's own.
pub(in crate::app::model_preview) const H3_SHADOW_APPLY: &str = "in vec4 v_tc0;
uniform sampler2D u_shadow_map;
uniform sampler2D u_normal_target;
uniform vec3 u_shadow_toward;
uniform float u_shadow_size;
uniform float u_shadow_alpha;
out vec4 color;
float closer(ivec2 texel, ivec2 offset, float depth) {
    return step(depth, texelFetch(u_shadow_map, texel + offset, 0).r);
}
void main() {
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    vec3 normal = texelFetch(u_normal_target, ivec2(gl_FragCoord.xy), 0).xyz * 2.0 - 1.0;
    float cosine = dot(normal, u_shadow_toward);
    float falloff = clamp(place.z * 2.0 - 1.0, 0.0, 1.0);
    falloff *= falloff;
    float darkness = u_shadow_alpha * (1.0 - falloff * falloff) * clamp(cosine, 0.0, 1.0);
    float darken = 1.0;
    vec2 pixel = place.xy * u_shadow_size;
    if (darkness > 0.001 && all(greaterThan(pixel, vec2(1.0))) && all(lessThan(pixel, vec2(u_shadow_size - 2.0)))) {
        ivec2 t = ivec2(floor(pixel));
        vec2 f = fract(pixel);
        vec2 r = 1.0 - f;
        float lit = r.x * r.y * closer(t, ivec2(-1, -1), place.z)
            + r.y * closer(t, ivec2(0, -1), place.z)
            + f.x * r.y * closer(t, ivec2(1, -1), place.z)
            + r.x * closer(t, ivec2(-1, 0), place.z)
            + closer(t, ivec2(0, 0), place.z)
            + f.x * closer(t, ivec2(1, 0), place.z)
            + r.x * f.y * closer(t, ivec2(-1, 1), place.z)
            + f.y * closer(t, ivec2(0, 1), place.z)
            + f.x * f.y * closer(t, ivec2(1, 1), place.z);
        lit *= 0.25;
        darken = clamp(1.01 - darkness + lit * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(0.0, 0.0, 0.0, darken);
}
";
