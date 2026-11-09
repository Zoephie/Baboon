//! The hologram pass: the screen maps uploaded and the hologram drawn.

use super::super::super::{ModelGpuFrame, RenderCore, RenderModelPreviewBatch, ce_flare_clip};
use super::super::lighting::h4_frame_values;
use super::screen::{
    H4Holograms, HologramPicture, IDENTITY_CURVE, hologram_screen, hologram_sphere,
    set_alpha_blend_max,
};
use super::{
    DrawnObject, Gen3Material, Gen3Pass, Gen3State, Gen3Value, H4BloomSettings, H4Draw, H4Game,
    H4HologramPost, H4ScreenMaterial, ce_flare_view, gen3_set_cull, set_value, upload_image_linear,
};
use crate::app::model_preview::gl::blit::blit_depth;
use eframe::glow;
use eframe::glow::HasContext as _;
use std::collections::HashMap;
use std::sync::Arc;

impl H4Game {
    /// `RenderHolograms`' generate pass, after the albedo (`c_player_view::render`
    /// X360 0x82A3B5F4): the holograms into their own surface, then its bloom
    /// and `final_combine_hologram`. `scene` holds the depth to test against.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn generate_holograms(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        holograms: &[(RenderModelPreviewBatch, Arc<Gen3Material>)],
        scene: glow::NativeFramebuffer,
        size: [i32; 2],
    ) -> Option<HologramPicture> {
        let (first, material) = holograms.first()?;
        let post = Arc::clone(material.h4()?.post()?);
        let owner = frame
            .preview
            .materials
            .get(first.material_index as usize)
            .map_or(0, |material| material.object);
        let (center, radius) = hologram_sphere(frame);
        let view = ce_flare_view(&frame.camera);
        let window = |point: [f32; 3]| {
            let clip = ce_flare_clip(&frame.camera, point);
            let depth: f32 = (0..3)
                .map(|axis| (point[axis] - view.eye[axis]) * view.forward[axis])
                .sum();
            (clip[3] > 1e-4 && depth > 1e-4).then(|| {
                let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
                (
                    [
                        (ndc[0] * 0.5 + 0.5) * size[0] as f32,
                        (ndc[1] * 0.5 + 0.5) * size[1] as f32,
                    ],
                    depth,
                )
            })
        };
        // The lens, from how far one unit up the view at the centre's depth
        // reaches on the picture.
        let depth: f32 = (0..3)
            .map(|axis| (center[axis] - view.eye[axis]) * view.forward[axis])
            .sum();
        let ahead: [f32; 3] =
            std::array::from_fn(|axis| view.eye[axis] + view.forward[axis] * depth.max(1e-3));
        let raised: [f32; 3] = std::array::from_fn(|axis| ahead[axis] + view.up[axis]);
        let reach = |point| {
            let clip = ce_flare_clip(&frame.camera, point);
            clip[1] / clip[3].max(1e-6)
        };
        let ndc = (reach(raised) - reach(ahead)).abs().max(1e-6);
        let vertical_fov = 2.0 * (1.0 / (depth.max(1e-3) * ndc)).atan();
        let inside = (0..3).all(|axis| (view.eye[axis] - center[axis]).abs() <= radius);
        let screen = hologram_screen(
            center,
            radius,
            window,
            view.right,
            view.up,
            inside,
            vertical_fov,
            size,
        )?;
        unsafe {
            if self
                .surfaces
                .as_ref()
                .is_none_or(|holograms| holograms.size != size)
            {
                if let Some(old) = self.surfaces.take() {
                    old.delete(gl);
                }
                self.surfaces = H4Holograms::new(gl, size);
            }
            let (color, framebuffer, combined_framebuffer) = {
                let holograms = self.surfaces.as_ref()?;
                (
                    holograms.color,
                    holograms.framebuffer,
                    holograms.combined_framebuffer,
                )
            };
            // MCC: the scene's depth copied in (`copy_depth`), the colour
            // cleared to nothing.
            blit_depth(gl, scene, framebuffer, size);
            let [width, height] = size;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.viewport(0, 0, width, height);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            let [x0, y0, x1, y1] = screen.crop;
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            // `handle_object_part`: what sorts after the bloom (layer 3)
            // waits for it.
            let post_bloom = |material: &Gen3Material| material.sort_layer == 3;
            // Pass 0, "hologram opaque": depth written, back faces culled,
            // no blending.
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gen3_set_cull(gl, frame);
            super::super::pass_hooks::h4_pass_hooks(state).submit_draw =
                Some(H4Draw::HologramOpaque);
            for (batch, material) in holograms
                .iter()
                .filter(|(_, material)| !post_bloom(material))
            {
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            super::super::pass_hooks::h4_pass_hooks(state).submit_draw = None;
            // Mode 1 (`g_disableHologramOrderIndependentTransparency`, the
            // default) then draws pass 2, "hologram blend": blended holograms
            // alone, tested but not written, through `alpha_blend_max`.
            gl.enable(glow::BLEND);
            set_alpha_blend_max(gl);
            gl.depth_mask(false);
            super::super::pass_hooks::h4_pass_hooks(state).submit_draw =
                Some(H4Draw::HologramBlend);
            for (batch, material) in holograms.iter().filter(|(_, material)| {
                !post_bloom(material)
                    && material
                        .h4()
                        .is_some_and(|submit| submit.is_blended_hologram())
            }) {
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            super::super::pass_hooks::h4_pass_hooks(state).submit_draw = None;
            gl.blend_equation(glow::FUNC_ADD);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            // Its bloom, by the hologram camera fx.
            let vertices = state.empty_array(gl);
            if self.bloom.is_none() {
                self.bloom =
                    super::super::bloom::H4Bloom::new(gl, core.black_flat, core.white_volume);
            }
            let settings = H4BloomSettings::hologram(screen.bloom_scale);
            let bloom =
                self.bloom
                    .as_mut()?
                    .build(gl, vertices, framebuffer, color, size, None, &settings);
            // Pass 3, "hologram postbloom": layer 3, by its own blend.
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            gl.enable(glow::DEPTH_TEST);
            for (batch, material) in holograms
                .iter()
                .filter(|(_, material)| post_bloom(material))
            {
                let blends =
                    material.blend != super::super::super::gen3::shaders::Gen3Blend::Opaque;
                if blends {
                    gl.enable(glow::BLEND);
                    super::super::super::gen3::pipeline::gen3_set_blend(gl, material.blend);
                } else {
                    gl.disable(glow::BLEND);
                }
                gl.depth_mask(!blends);
                state.draw_pass(core, gl, frame, slot, batch, material, &material.lit, None);
            }
            gl.disable(glow::BLEND);
            gl.color_mask(true, true, true, true);
            // `postprocess_final_composite(22)` into `_surface_hologram_temp`.
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(combined_framebuffer));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.disable(glow::DEPTH_TEST);
            gl.depth_mask(false);
            let window_constants = [
                width as f32,
                height as f32,
                1.0 / width as f32,
                1.0 / height as f32,
            ];
            let generic = vec![screen.coverage, screen.texcoords, [0.0; 4], [0.0; 4]];
            let engine = [
                (
                    "ps_player_window_constants",
                    Gen3Value::Floats(window_constants),
                ),
                (
                    "ps_material_generic_parameters",
                    Gen3Value::FloatArray(generic),
                ),
                (
                    "ps_filmic_tone_curve_params",
                    Gen3Value::FloatArray(IDENTITY_CURVE.map(|value| [value; 4]).to_vec()),
                ),
            ];
            let textures = [
                ("ps_surface_sampler_texture", Some(color)),
                ("ps_bloom_sampler_texture", bloom.or(core.black_flat)),
            ];
            self.screen_pass(
                core,
                state,
                gl,
                frame,
                &post,
                &post.combine,
                owner,
                &engine,
                &textures,
                size,
            );
            gl.disable(glow::SCISSOR_TEST);
            gl.depth_mask(true);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene));
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
            Some(HologramPicture {
                screen,
                post,
                owner,
                distance: depth,
            })
        }
    }

    /// `hologram_apply_callback` (X360 0x83AF4CF0): `hologram_composite` over
    /// the crop into the scene's light, premultiplied (blend mode 5), with no
    /// depth. Leaves the blend as it found it off.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn composite_holograms(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        picture: &HologramPicture,
        size: [i32; 2],
    ) {
        let Some(combined) = self.surfaces.as_ref().map(|holograms| holograms.combined) else {
            return;
        };
        unsafe {
            use super::super::lighting::{H4_FILMIC_CURVE, h4_filmic_params};
            let view_inverse = h4_filmic_params(H4_FILMIC_CURVE, true);
            let [x0, y0, x1, y1] = picture.screen.crop;
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x0, y0, x1 - x0, y1 - y0);
            gl.disable(glow::DEPTH_TEST);
            gl.depth_mask(false);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.color_mask(true, true, true, true);
            let engine = [
                (
                    "ps_filmic_tone_curve_params",
                    Gen3Value::FloatArray(view_inverse.map(|value| [value; 4]).to_vec()),
                ),
                ("ps_material_blend_constant", Gen3Value::Floats([0.0; 4])),
            ];
            let textures = [("source_samplerTexture", Some(combined))];
            let post = Arc::clone(&picture.post);
            self.screen_pass(
                core,
                state,
                gl,
                frame,
                &post,
                &post.composite,
                picture.owner,
                &engine,
                &textures,
                size,
            );
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(core.slots[slot].vertex_array));
        }
    }

    /// One screen material's pass over the picture: the engine's values
    /// first, then the frame's, then the material's functions on `owner`'s
    /// values, then its bake.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app::model_preview) unsafe fn screen_pass(
        &mut self,
        core: &mut RenderCore,
        state: &mut Gen3State,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        post: &Arc<H4HologramPost>,
        material: &H4ScreenMaterial,
        owner: u8,
        engine: &[(&str, Gen3Value)],
        targets: &[(&str, Option<glow::NativeTexture>)],
        size: [i32; 2],
    ) {
        unsafe {
            let holograms = self.surfaces.as_mut().expect("the hologram surfaces");
            if !uploaded_from(holograms.uploaded.as_ref().map(|(known, _)| known), post) {
                for texture in holograms
                    .uploaded
                    .take()
                    .into_iter()
                    .flat_map(|(_, map)| map.into_values())
                {
                    gl.delete_texture(texture);
                }
                let mut map = HashMap::new();
                for screen in [&post.combine, &post.composite] {
                    for texture in &screen.textures {
                        if let Some(uploaded) = upload_image_linear(
                            gl,
                            &texture.image,
                            texture.texels,
                            &texture.mips,
                            texture.sampler,
                        ) {
                            map.insert(texture.uniform.clone(), uploaded);
                        }
                    }
                }
                holograms.uploaded = Some((post.clone(), map));
            }
            let uploaded = holograms
                .uploaded
                .as_ref()
                .map(|(_, map)| map.clone())
                .unwrap_or_default();
            let pass: &Arc<Gen3Pass> = &material.pass;
            state.with_program(core, gl, pass, |state, this, program| {
                gl.use_program(Some(program.program));
                let drawn = DrawnObject {
                    values: frame.function_values_of_object(owner),
                    time: frame.time,
                    owner,
                };
                let node = [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ];
                let functions = material.submit.draw_values(H4Draw::Lit, &drawn, node);
                for (name, ty) in &pass.uniforms {
                    let location = program.location(gl, name);
                    if location.is_none() {
                        continue;
                    }
                    let value = engine
                        .iter()
                        .find(|(known, _)| known == name)
                        .map(|(_, value)| value.clone())
                        .or_else(|| h4_frame_values(frame, name, size, 0.0))
                        .or_else(|| functions.get(name).cloned())
                        .or_else(|| material.values.get(name).cloned());
                    if let Some(value) = value {
                        set_value(gl, location.as_ref(), ty, &value);
                    }
                }
                for (unit, (name, _)) in pass.textures.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + unit as u32);
                    let texture = targets
                        .iter()
                        .find(|(known, _)| known == name)
                        .map(|(_, texture)| *texture)
                        .unwrap_or_else(|| uploaded.get(name).copied())
                        .or(this.placeholder);
                    gl.bind_texture(glow::TEXTURE_2D, texture);
                    gl.uniform_1_i32(program.location(gl, name).as_ref(), unit as i32);
                }
                gl.active_texture(glow::TEXTURE0);
                gl.bind_vertex_array(Some(state.empty_array(gl)));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            });
        }
    }
}

/// Whether the uploaded bitmaps are `post`'s: the same post, not one equal to
/// it, since another load's can differ after an edit.
pub(super) fn uploaded_from<T>(held: Option<&Arc<T>>, post: &Arc<T>) -> bool {
    held.is_some_and(|held| Arc::ptr_eq(held, post))
}
