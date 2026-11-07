//! Halo CE's and Halo 2's sprites over the shaded model: Halo 2's particle
//! systems far to near, its light volumes, liquids and contrails, the
//! antennas, CE's effect groups, and both games' lens flares.

use eframe::glow::{self, HasContext as _};

use super::super::common::sprites::{
    ParticleTargets, advance_sprite_simulations, begin_flares, draw_antennas, draw_particle_system, draw_sprite_quad,
};
use super::super::gen3::pipeline::{h3_layered_distance, h3_set_blend};
use super::super::gen3::shaders::H3Blend;
use super::super::renderer::*;

impl RenderCore {
    /// The frame's dx9 sprites, after the model and its wireframe.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_sprites(&mut self, gl: &glow::Context, frame: &ModelGpuFrame, slot: usize) {
        unsafe {
            if self.begin_sprites(gl, frame, slot).is_none() {
                return;
            }
            self.draw_dx9_particles(frame, gl, slot);
            let Some(h3) = self.begin_sprites(gl, frame, slot) else {
                return;
            };
            let Some(flares) = self.sprite_gl.as_ref().and_then(Option::as_ref) else {
                return;
            };
            let uploaded = &mut self.slots[slot];
            let camera = &frame.camera;
            let view = ce_flare_view(camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            // `rasterizer_light_volume_draw_sprites` sets its widget state
            // with `sub_936910(0, 3, 2)`: depth tested, less or equal, not
            // written. Liquids draw through it too, and contrails through
            // a transparent pass, which tests the same way.
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(false);
            let volumes = frame.preview.h2_light_volumes.iter().zip(&uploaded.h2_volume_textures);
            for (index, (at, textures)) in volumes.enumerate() {
                if !is_shown(&frame.h2_shown.volumes, index) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                let onset = uploaded.simulations.effect_onsets.of(at.object, at.scale_name(), values, frame.time);
                for (volume, texture) in at.volumes().iter().zip(textures) {
                    let Some(texture) = texture else {
                        continue;
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                    h3_set_blend(gl, if volume.fuzzy() { H3Blend::Additive } else { H3Blend::PreMultiplied });
                    let colors = frame.change_colors_of_object(at.object);
                    for quad in at.quads(volume, values, &colors, bones, &view, frame.time, onset) {
                        draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                    }
                }
            }
            let liquids = frame.preview.h2_liquids.iter().zip(&uploaded.h2_liquid_textures);
            for (index, (at, textures)) in liquids.enumerate() {
                if !is_shown(&frame.h2_shown.liquids, index) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                for (arc, texture) in at.arcs().iter().zip(textures) {
                    let Some(texture) = texture else {
                        continue;
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                    h3_set_blend(gl, H3Blend::PreMultiplied);
                    for quad in at.quads(arc, values, bones, &view, frame.time) {
                        draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                    }
                }
            }
            let contrails = frame.preview.h2_contrails.iter().zip(&uploaded.h2_contrail_textures);
            for (index, (at, texture)) in contrails.enumerate() {
                if !is_shown(&frame.h2_shown.contrails, index) {
                    continue;
                }
                let Some(texture) = texture else {
                    continue;
                };
                gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                h3_set_blend(gl, at.blend());
                for quad in uploaded.simulations.h2_contrails.quads(index, at, &view) {
                    draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                }
            }
            draw_antennas(gl, flares, uploaded, frame, bones, &view, h3);
            // Halo CE's groups, when the transparents did not sort them in.
            if ce_has_groups(&frame.preview) && !ce_sorts_transparents(frame) {
                for group in ce_groups(gl, uploaded, frame, bones, &view) {
                    draw_ce_group(gl, &mut self.ce_effect_gl, self.program, flares, uploaded, camera, group);
                }
                gl.use_program(Some(flares.program));
                gl.bind_vertex_array(Some(flares.vertex_array));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
                gl.blend_equation(glow::FUNC_ADD);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            }
            gl.color_mask(true, true, true, false);
            begin_flares(gl, flares, self.placeholder);
            for (index, light) in frame.preview.ce_lights.iter().enumerate() {
                let state = frame.object_states.get(usize::from(light.object));
                let outputs = state.map_or([0.0; 4], |state| state.outputs);
                let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
                let Some(patch) = light.occlusion_patch(&outputs, &colors, bones, &view) else {
                    continue;
                };
                let Some(occlusion) = ce_flare_occlusion(gl, flares, camera, &view, &patch) else {
                    continue;
                };
                let sprites = light.sprites(&outputs, &colors, bones, &view, frame.time, occlusion);
                let Some(textures) = uploaded.flare_textures.get(index) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, &view, &sprites, textures);
            }
            for (effect, def, light) in uploaded.simulations.ce_particles.lights(&frame.preview.ce_effects) {
                let state = frame.object_states.get(usize::from(light.object));
                let outputs = state.map_or([0.0; 4], |state| state.outputs);
                let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
                let Some(patch) = light.occlusion_patch(&outputs, &colors, bones, &view) else {
                    continue;
                };
                let Some(occlusion) = ce_flare_occlusion(gl, flares, camera, &view, &patch) else {
                    continue;
                };
                let sprites = light.sprites(&outputs, &colors, bones, &view, frame.time, occlusion);
                let Some(textures) = uploaded.ce_effect_flare_textures.get(&effect).and_then(|lights| lights.get(def)) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, &view, &sprites, textures);
            }
            // The slot's flare maps run CE's lights, then Halo 2's flares.
            let first_h2 = frame.preview.ce_lights.len();
            for (index, at) in frame.preview.h2_flares.iter().enumerate() {
                if !frame.h2_flares_shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let occlusion = match at.occlusion_patches(bones, &view) {
                    None => [1.0; 2],
                    Some((outer, inner)) => {
                        let Some(outer) = ce_flare_occlusion(gl, flares, camera, &view, &outer) else {
                            continue;
                        };
                        let inner = inner.map_or(Some(1.0), |inner| {
                            ce_flare_occlusion(gl, flares, camera, &view, &inner)
                        });
                        [outer * inner.unwrap_or(0.0), outer]
                    }
                };
                let values = frame.function_values_of_object(at.object);
                let colors = frame.change_colors_of_object(at.object);
                let onset = uploaded.simulations.effect_onsets.of(at.object, at.scale_name(), values, frame.time);
                let sprites = at.sprites(values, &colors, bones, &view, frame.time, onset, occlusion);
                let Some(textures) = uploaded.flare_textures.get(first_h2 + index) else {
                    continue;
                };
                draw_flare_sprites(gl, flares, camera, &view, &sprites, textures);
            }
            self.end_sprites(gl, frame, slot);
        }
    }

    /// `render_effects`, before the other sprites: Halo 2's particle systems,
    /// far to near, each from its simulation. What bends the scene draws
    /// only into gen3's distortion target, so none of it here.
    unsafe fn draw_dx9_particles(&mut self, frame: &ModelGpuFrame, gl: &glow::Context, slot: usize) {
        unsafe {
            let particle_gl = self.particle_gl.as_ref().and_then(Option::as_ref);
            let uploaded = &mut self.slots[slot];
            let view = ce_flare_view(&frame.camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            gl.color_mask(true, true, true, false);
            advance_sprite_simulations(uploaded, frame, bones, &view, &mut self.sprites_advanced);
            let systems = frame.preview.h3_particles.iter().enumerate();
            let mut sorted: Vec<_> = systems
                .filter(|(_, system)| system.look.shader.distortion.is_none())
                .map(|(at, system)| {
                    let distance = uploaded.simulations.particles.distance(at, system, bones, &view);
                    (h3_layered_distance(system.look.shader.sort_layer, distance), at)
                })
                .collect();
            sorted.sort_by(|(a, _), (b, _)| b.total_cmp(a));
            for (_, index) in sorted {
                if !frame.h3_systems_shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let Some(particles) = particle_gl else {
                    continue;
                };
                let targets = ParticleTargets {
                    no_map: self.gen3.no_bloom,
                    depth_copy: self.gen3.depth_copy,
                    distortion_copy: &mut self.h2_distortion_copy,
                    stops: self.gen3.stops,
                };
                draw_particle_system(gl, particles, uploaded, frame, index, &view, targets);
            }
        }
    }
}
