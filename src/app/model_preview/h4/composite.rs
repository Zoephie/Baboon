//! Halo 4's end of the frame: its colour grading, final composite, full camo
//! drawn after it, and shield.

use super::super::gen3::render::Gen3Cx;
use super::super::gen3::state::Gen3State;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use super::super::common::image::H2Image;
use crate::app::model_preview::common::image::{H3Sampler, H3Texels};
use super::super::gen3::pipeline::{H3_SHIELD_DAMAGE, HALF_LENS, h3_set_cull};
use crate::app::model_preview::gl::screen::screen_program;
use crate::app::model_preview::gl::texture::upload_h2_image_linear;
use eframe::glow::{self, HasContext};
use std::sync::Arc;
use super::render::H4Game;
use super::super::renderer::RenderCore;
use super::super::frame::ModelGpuFrame;

impl H4Game {
    /// Halo 4's `c_render_shield::render_object_internal`: every part that
    /// takes the shield through explicit 61, added (`ONE, ONE`) over what
    /// lies behind, no depth written, no face culled. The pass applies its
    /// own exposure.
    pub(super) unsafe fn draw_shield(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        let Some(shield) = frame.preview.h4.shield.clone() else {
            return;
        };
        let damage = frame.function_values.as_ref().and_then(|values| values.get(H3_SHIELD_DAMAGE)).copied().unwrap_or(0.0);
        if !shield.settings.showing(damage) {
            return;
        }
        let parts: Vec<_> = frame
            .shield_batches
            .iter()
            .filter_map(|&index| frame.preview.batches.get(index))
            .filter_map(|batch| {
                let index = batch.material_index as usize;
                let material = core.h3_material(frame, index)?;
                let textures = frame.textures.as_ref()?.get(index)?;
                super::super::gen3::shield::draws_shield(textures.shield_only, textures.never_shield, material.decal)
                    .then(|| (batch.clone(), Arc::clone(material)))
            })
            .collect();
        if parts.is_empty() {
            return;
        }
        let (low, high) = (frame.preview.bounds_min, frame.preview.bounds_max);
        let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
        let mut values = shield.values(damage, frame.time, frame.function_values.as_ref(), center);
        let camera = &frame.camera;
        let eye = if camera.perspective > 0.0 { 1.0 / camera.perspective } else { 1.0e5 };
        values.insert(
            super::shield::DEPTH_DECODE,
            [camera.depth_scale * camera.depth_map[0], camera.depth_map[1], camera.perspective, eye],
        );
        values.insert(super::shield::WORLD_SCALE, [camera.scale; 4]);
        unsafe {
            if let Some(targets) = state.targets.as_ref() {
                let [width, height] = targets.size;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(targets.depth_copy_framebuffer));
                let mask = glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT;
                gl.blit_framebuffer(0, 0, width, height, 0, 0, width, height, mask, glow::NEAREST);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(targets.accumulation_framebuffer));
                state.depth_copy = Some(targets.depth_copy);
            }
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.color_mask(true, true, true, true);
            state.pass_values = Some(values);
            for (batch, material) in &parts {
                state.draw_pass(core, gl, frame, slot, batch, material, &shield.pass, None);
            }
            state.pass_values = None;
            h3_set_cull(gl, frame);
        }
    }

    /// Halo 4's colour grading lookup, uploaded once per preview, and how a
    /// pass reads its texel centres (`SetColorGradingParameters`).
    pub(super) unsafe fn grading_texture(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) -> Option<super::bloom::H4BloomGrading> {
        unsafe {
            let grading = frame.preview.h4.color_grading.as_ref();
            let image = grading.map(|(image, ..)| image);
            if !grading_is_current(self.grading.as_ref().map(|(known, _)| known), image) {
                if let Some((_, texture)) = self.grading.take() {
                    gl.delete_texture(texture);
                }
                // Sampler 8: bilinear, clamped.
                let sampler = H3Sampler { address: [BitmapAddressMode::Clamp; 2], filter: BitmapFilterMode::Bilinear };
                let uploaded = image.and_then(|image| upload_h2_image_linear(gl, image, H3Texels::Linear, &[], Some(sampler)));
                self.grading = image.cloned().zip(uploaded);
            }
            let size = match grading.map(|(image, ..)| &**image) {
                Some(H2Image::Volume { width, height, depth, .. }) => [*width, *height, *depth],
                _ => return None,
            };
            let (scale, offset) = super::composite::h4_grading_scale_offset(size);
            self.grading.as_ref().map(|(_, texture)| super::bloom::H4BloomGrading { texture: *texture, scale, offset })
        }
    }

    /// Halo 4's `final_composite`: see `h4::composite::H4_COMPOSITE_FRAGMENT`.
    pub(super) unsafe fn draw_composite(
        &mut self,
        cx: &mut Gen3Cx<'_>,
        accumulation: glow::NativeTexture,
        bloom: Option<glow::NativeTexture>,
        depth: Option<glow::NativeTexture>,
        screen: super::super::gen3::effects::screen::H3ScreenSettings,
    ) {
        use super::lighting::H4_DARK_MULTIPLIER;
        let (gl, frame, core) = (cx.gl, cx.frame, &mut *cx.core);
        let state = &mut *cx.state;
        unsafe {
            let grading = self.grading_texture(gl, frame);
            if self.composite_program.is_none() {
                self.composite_program = screen_program(gl, H4_COMPOSITE_FRAGMENT);
            }
            let Some(program) = self.composite_program else {
                return;
            };
            gl.use_program(Some(program));
            let bind = |unit: u32, target: u32, texture: Option<glow::NativeTexture>, name: &str| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(target, texture);
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
            };
            bind(0, glow::TEXTURE_2D, Some(accumulation), "accumulation");
            bind(1, glow::TEXTURE_2D, bloom.or(core.black_flat), "bloom");
            bind(2, glow::TEXTURE_2D, depth, "scene_depth");
            bind(3, glow::TEXTURE_3D, grading.as_ref().map(|grading| grading.texture).or(core.white_volume), "grading");
            gl.active_texture(glow::TEXTURE0);
            let (scale, offset) = grading.as_ref().map_or(([1.0; 3], [0.0; 3]), |grading| (grading.scale, grading.offset));
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_scale").as_ref(), &scale);
            gl.uniform_3_f32_slice(gl.get_uniform_location(program, "grading_offset").as_ref(), &offset);
            gl.uniform_1_f32(gl.get_uniform_location(program, "dark_multiplier").as_ref(), H4_DARK_MULTIPLIER);
            let filmic = super::lighting::h4_filmic_params(super::lighting::H4_FILMIC_CURVE, false);
            gl.uniform_1_f32_slice(gl.get_uniform_location(program, "filmic").as_ref(), &filmic);
            gl.uniform_1_f32(gl.get_uniform_location(program, "view_scale").as_ref(), screen.view_scale(HALF_LENS));
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.bind_vertex_array(Some(state.empty_array(gl)));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DEPTH_TEST);
        }
    }
}

/// Whether the uploaded grading lookup is `now`'s: the same image, not one
/// equal to it, since another load's image can differ after an edit.
fn grading_is_current(held: Option<&Arc<H2Image>>, now: Option<&Arc<H2Image>>) -> bool {
    match (held, now) {
        (Some(held), Some(now)) => Arc::ptr_eq(held, now),
        (None, None) => true,
        _ => false,
    }
}

/// `final_composite` (`postprocess/final_composite_shared.fxh`), at the
/// preview's camera fx: the HDR target brought back up (`× (8, 8, 8, 32)`)
/// and the bloom added unscaled; hue, saturation and contrast are off (not
/// on PC); the filmic curve ([`h4_filmic_params`]); then `sqrt` and
/// `apply_color_grading` with one lookup, which is
/// what reaches the screen. The lookup is uploaded as its bytes stand, so
/// that its output is the displayed value [inferred: the back buffer's
/// format, O7, decides whether the game's view decodes it, and either way
/// the default lookup's output displays as sRGB(x²)]. Coverage is Reach's:
/// what the object drew covers what lies behind it.
pub(in crate::app::model_preview) const H4_COMPOSITE_FRAGMENT: &str = "#version 410 core
uniform sampler2D accumulation;
uniform sampler2D bloom;
uniform sampler2D scene_depth;
uniform sampler3D grading;
uniform vec3 grading_scale;
uniform vec3 grading_offset;
uniform float dark_multiplier;
uniform float view_scale;
uniform float filmic[5];
out vec4 color;
void main() {
    vec2 place = gl_FragCoord.xy / vec2(textureSize(accumulation, 0));
    place = (place - 0.5) * view_scale + 0.5;
    vec4 accumulated = texture(accumulation, place);
    if (place.x < 0.0 || place.x > 1.0 || place.y < 0.0 || place.y > 1.0) {
        accumulated = vec4(0.0);
    }
    vec3 combined = max(accumulated.rgb, vec3(0.0)) * dark_multiplier + texture(bloom, place).rgb;
    combined = combined * (combined * filmic[0] + filmic[1]) / (combined * (combined * filmic[2] + filmic[3]) + filmic[4]);
    vec3 encoded = sqrt(max(combined, vec3(0.0)));
    // `apply_color_grading` reads level 0 (`sample3DLOD`). An implicit level
    // follows the lookup coordinate's change across each 2×2 block, which
    // jumps at a silhouette, and drew a grey rim from the lookup's small mips.
    color.rgb = textureLod(grading, encoded * grading_scale + grading_offset, 0.0).rgb;
    bool drawn = texture(scene_depth, place).r < 1.0;
    color.a = drawn ? 1.0 : accumulated.a;
}
";

/// `SetColorGradingParameters` (X360 0x829B78E0): a lookup of `size` texels
/// a side is read at `x (size - 1) / size + 0.5 / size`, its texel centres.
pub(in crate::app::model_preview) fn h4_grading_scale_offset(size: [usize; 3]) -> ([f32; 3], [f32; 3]) {
    let scale = size.map(|side| (side.max(1) as f32 - 1.0) / side.max(1) as f32);
    let offset = size.map(|side| 0.5 / side.max(1) as f32);
    (scale, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Arc<H2Image> {
        Arc::new(H2Image::Volume { width: 1, height: 1, depth: 1, rgba: vec![0; 4] })
    }

    #[test]
    fn the_grading_lookup_uploads_again_for_a_new_load_but_not_for_the_same_image() {
        let held = image();
        assert!(grading_is_current(Some(&held), Some(&held.clone())));
        assert!(grading_is_current(None, None));
        // A later load reads an equal image into a new allocation.
        assert!(!grading_is_current(Some(&held), Some(&image())));
        assert!(!grading_is_current(None, Some(&held)));
        assert!(!grading_is_current(Some(&held), None));
    }
}
