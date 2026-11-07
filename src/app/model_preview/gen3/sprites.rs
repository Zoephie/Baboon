//! The gen3 games' sprites, which the pipeline enters by stage from inside
//! its blended sort: light volumes, Halo 4's tracers, particle systems,
//! strips and leaves far to near, then the antennas, Reach's and Halo 4's
//! cheap particles and the lens flares.

use eframe::glow::{self, HasContext as _};

use super::super::common::sprites::{
    ParticleTargets, advance_sprite_simulations, begin_flares, draw_antennas, draw_particle_system, draw_sprite_quad,
};
use super::super::renderer::*;
use super::super::*;
use super::super::common::view::ViewFrame;
use super::super::gen3::effects::H3EffectOnsets;
use super::super::particles::H3ParticleField;
use super::pipeline::{h3_layered_distance, h3_lighting_constants, h3_set_blend};
use super::render::Gen3Game;

/// Which part of the object's sprites a call draws.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) enum SpriteStage {
    /// Volumes and particles not yet drawn, down to this far away.
    FartherThan(f32),
    /// Antennas and lens flares, and any left of those.
    Rest,
    /// What bends the scene, to its own target.
    Distortions,
}

impl RenderCore {
    /// The part of the frame's sprites `stage` names.
    pub(in crate::app::model_preview) unsafe fn draw_gen3_sprites(
        &mut self,
        game: &mut dyn Gen3Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        stage: SpriteStage,
    ) {
        unsafe {
            let Some(h3) = self.begin_sprites(gl, frame, slot) else {
                return;
            };
            let Some(flares) = self.sprite_gl.as_ref().and_then(Option::as_ref) else {
                return;
            };
            let particle_gl = self.particle_gl.as_ref().and_then(Option::as_ref);
            let no_map = self.black_flat;
            // What Reach's area screen effects do to the sprites' exposure.
            let stops = self.gen3.stops;
            let placeholder = self.placeholder;
            let size = self.gen3.targets.as_ref().map_or([1, 1], |targets| targets.size);
            let depth_copy = self.gen3.depth_copy;
            let uploaded = &mut self.slots[slot];
            let uploads = uploaded.uploads();
            let camera = &frame.camera;
            let view = ce_flare_view(camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            macro_rules! sprites {
                ($uploaded:expr) => {
                    Gen3Sprites {
                        gl,
                        frame,
                        flares,
                        view: &view,
                        bones,
                        onsets: &mut $uploaded.simulations.effect_onsets,
                        particles: &mut $uploaded.simulations.particles,
                        particle_gl,
                        no_map,
                        stops,
                        depth_copy,
                        size,
                        placeholder,
                        slot,
                        uploads,
                    }
                };
            }
            if matches!(stage, SpriteStage::Rest) {
                draw_antennas(gl, flares, &mut uploaded.simulations.antennas, &uploaded.antenna_textures, frame, bones, &view, h3);
            }
            gl.color_mask(true, true, true, false);
            // `render_effects`, among the transparents: every profile is its own quad.
            let first = !self.sprites_advanced;
            advance_sprite_simulations(uploaded, frame, bones, &view, &mut self.sprites_advanced);
            if first {
                game.advance_own_sprites(&mut sprites!(uploaded));
            }
            let cursor = &mut self.gen3.sprite_cursor;
            // `c_transparency_renderer` sorts far to near, each from its location.
            enum Sprites {
                Volume(usize),
                Tracer(usize),
                System(usize),
                Strip(usize),
                Leaves(usize),
            }
            let eye_distance = |origin: [f32; 3]| {
                (0..3).map(|axis| (origin[axis] - view.eye[axis]) * view.forward[axis]).sum::<f32>()
            };
            let layered = h3_layered_distance;
            let volumes = frame.preview.h3_light_volumes.iter().enumerate();
            let volumes = volumes
                .filter(|_| !matches!(stage, SpriteStage::Distortions))
                .map(|(at, volume)| (layered(volume.sort_layer(), eye_distance(volume.origin(bones))), Sprites::Volume(at)));
            let tracers = frame.preview.h4_tracers.iter().enumerate();
            let tracers = tracers.filter(|_| !matches!(stage, SpriteStage::Distortions)).map(|(at, tracer)| {
                let distance = eye_distance(tracer.origin(bones)) - tracer.sort_bias();
                (layered(tracer.sort_layer(), distance), Sprites::Tracer(at))
            });
            let systems = frame.preview.h3_particles.iter().enumerate();
            let distorting = matches!(stage, SpriteStage::Distortions);
            let systems = systems
                .filter(|(_, system)| system.look.shader.distortion.is_some() == distorting)
                .map(|(at, system)| (layered(system.look.shader.sort_layer, uploaded.simulations.particles.distance(at, system, bones, &view)), Sprites::System(at)));
            let strips = frame.preview.h3_strips.iter().enumerate();
            let strips = strips
                .filter(|_| !matches!(stage, SpriteStage::Distortions))
                .map(|(at, strip)| {
                    let distance = eye_distance(strip.origin(bones)) - strip.sort_bias();
                    (layered(strip.shader().sort_layer, distance), Sprites::Strip(at))
                });
            let leaves = frame.preview.h3_leaves.iter().enumerate();
            let leaves = leaves
                .filter(|_| !matches!(stage, SpriteStage::Distortions))
                .map(|(at, leaves)| (layered(2, eye_distance(leaves.origin())), Sprites::Leaves(at)));
            let mut sorted: Vec<_> = volumes.chain(tracers).chain(systems).chain(strips).chain(leaves).collect();
            sorted.sort_by(|(a, _), (b, _)| b.total_cmp(a));
            let limit = match stage {
                SpriteStage::FartherThan(limit) => limit,
                SpriteStage::Rest => f32::INFINITY,
                SpriteStage::Distortions => f32::NEG_INFINITY,
            };
            let mut distorted = 0;
            let skip = if distorting { 0 } else { *cursor };
            for (distance, sprites) in sorted.into_iter().skip(skip) {
                if distance < limit {
                    break;
                }
                if distorting {
                    distorted += 1;
                } else {
                    *cursor += 1;
                }
                match sprites {
                    Sprites::Tracer(index) => {
                        if frame.h4_tracers_shown.get(index).copied().unwrap_or(true) {
                            game.draw_own_sprite(&mut sprites!(uploaded), Gen3Sprite::Tracer(index));
                        }
                    }
                    Sprites::Volume(index) => {
                        let at = &frame.preview.h3_light_volumes[index];
                        if !frame.h3_volumes_shown.get(index).copied().unwrap_or(true) {
                            continue;
                        }
                        if game.draw_own_sprite(&mut sprites!(uploaded), Gen3Sprite::Volume(index)) {
                            continue;
                        }
                        let texture = uploaded.volume_textures.get(index).copied().flatten();
                        let circular = at.circular();
                        if texture.is_none() && circular.is_none() {
                            continue;
                        }
                        gl.use_program(Some(flares.program));
                        gl.bind_vertex_array(Some(flares.vertex_array));
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
                        gl.active_texture(glow::TEXTURE0);
                        h3_set_blend(gl, at.blend());
                        gl.bind_texture(glow::TEXTURE_2D, texture.or(self.placeholder));
                        let [center, falloff] = circular.unwrap_or([0.0; 2]);
                        gl.uniform_2_f32(flares.circular.as_ref(), center, falloff);
                        // With no depth copy, no fade to read one.
                        let fade = at.depth_fade().filter(|_| self.gen3.depth_copy.is_some());
                        if let Some(range) = fade {
                            gl.active_texture(glow::TEXTURE1);
                            gl.bind_texture(glow::TEXTURE_2D, self.gen3.depth_copy);
                            gl.active_texture(glow::TEXTURE0);
                            gl.uniform_1_i32(flares.scene_depth.as_ref(), 1);
                            gl.uniform_4_f32_slice(flares.depth.as_ref(), &scene_depth_terms(camera));
                            gl.uniform_1_f32(flares.fade.as_ref(), range);
                        }
                        let values = frame.function_values_of_object(at.object);
                        let onset = uploaded.simulations.effect_onsets.of(at.object, at.scale_name(), values, frame.time);
                        for quad in at.quads(values, bones, &view, frame.time, onset, stops) {
                            draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
                        }
                        // The flare program's other users draw maps, unfaded.
                        gl.uniform_2_f32(flares.circular.as_ref(), 0.0, 0.0);
                        if fade.is_some() {
                            gl.uniform_1_f32(flares.fade.as_ref(), 0.0);
                            gl.uniform_1_i32(flares.scene_depth.as_ref(), 0);
                        }
                    }
                    Sprites::System(index) => {
                        if !frame.h3_systems_shown.get(index).copied().unwrap_or(true) {
                            continue;
                        }
                        if game.draw_own_sprite(&mut sprites!(uploaded), Gen3Sprite::System(index)) {
                            continue;
                        }
                        let Some(particles) = particle_gl.as_ref() else {
                            continue;
                        };
                        let targets = ParticleTargets {
                            no_map,
                            depth_copy: self.gen3.depth_copy,
                            distortion_copy: &mut self.h2_distortion_copy,
                            stops,
                        };
                        draw_particle_system(gl, particles, uploaded, frame, index, &view, targets);
                    }
                    Sprites::Leaves(index) => {
                        game.draw_own_sprite(&mut sprites!(uploaded), Gen3Sprite::Leaves(index));
                    }
                    Sprites::Strip(index) => {
                        let strip = &frame.preview.h3_strips[index];
                        if !frame.h3_strips_shown.get(index).copied().unwrap_or(true) {
                            continue;
                        }
                        let Some(particles) = particle_gl.as_ref() else {
                            continue;
                        };
                        let values = frame.function_values_of_object(strip.object);
                        let onset = uploaded.simulations.effect_onsets.of(strip.object, strip.scale_name(), values, frame.time);
                        let lighting = h3_lighting_constants(frame);
                        let drawn = uploaded.simulations.h3_strips.vertices(index, strip, values, bones, &view, &lighting, frame.time, onset, stops);
                        if drawn.is_empty() {
                            continue;
                        }
                        let vertices = particle_floats(camera, &drawn);
                        let textures = uploaded.strip_textures.get(index).copied().unwrap_or_default();
                        particles.draw(gl, strip.shader(), &vertices, textures, no_map, self.gen3.depth_copy, camera, &lighting, stops);
                    }
                }
            }
            if distorting {
                self.gen3.distorted = distorted > 0;
            }
            // `s_cheap_particle_system::render`: after every other
            // transparent, in one batch, in ring-slot order.
            if matches!(stage, SpriteStage::Rest)
                && let (Some(first), Some(texture)) = (frame.preview.reach_cheap_emitters.first(), uploaded.reach_cheap_texture)
            {
                let drawn = uploaded.simulations.reach_cheap.vertices(first.library(), &view);
                if !drawn.is_empty()
                    && let Some(cheap) = self.reach_cheap_gl.get_or_insert_with(|| reach_cheap_gl(gl, self.program)).as_ref()
                {
                    let vertices: Vec<[f32; CHEAP_FLOATS]> = drawn
                        .iter()
                        .map(|vertex| {
                            let [a, b, c, d] = ce_flare_clip(camera, vertex.place);
                            let ([e, f, g], [h, i, j, k]) = (vertex.texcoord, vertex.color);
                            [a, b, c, d, e, f, g, h, i, j, k]
                        })
                        .collect();
                    // `cheap_particles_render.fx`: `lerp(ps_view_exposure.x,
                    // ps_view_self_illum_exposure.y, alpha)`.
                    let exposures = if first.library().halo4() {
                        let channel = |name: &str, at: usize| match h4::lighting::h4_frame_value(name) {
                            Some(gen3::shaders::H3Value::Floats(value)) => value[at],
                            _ => 1.0,
                        };
                        [channel("ps_view_exposure", 0), channel("ps_view_self_illum_exposure", 1)]
                    } else {
                        let exposure = reach_exposure(stops)[0];
                        [exposure, exposure]
                    };
                    cheap.draw(gl, &vertices, texture, exposures);
                }
            }
            if matches!(stage, SpriteStage::FartherThan(_) | SpriteStage::Distortions) {
                gl.blend_equation(glow::FUNC_ADD);
                gl.color_mask(true, true, true, true);
                gl.use_program(Some(self.program));
                return;
            }
            begin_flares(gl, flares, self.placeholder);
            let viewport = self.scene.as_ref().map_or([1, 1], |scene| scene.size);
            let pixels = |center: [f32; 3], half: [f32; 2]| ce_flare_pixels(camera, viewport, center, half);
            for (index, at) in frame.preview.h3_flares.iter().enumerate() {
                if !frame.h3_flares_shown.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let values = frame.function_values_of_object(at.object);
                let onset = uploaded.simulations.effect_onsets.of(at.object, at.scale_name(), values, frame.time);
                let event_time = at.event_time(frame.time, onset);
                // A light's own flare goes out with its light.
                let colors = frame.change_colors_of_object(at.object);
                let light = match at.light().map(|light| light.state(values, &colors, frame.time, onset)) {
                    Some(None) => continue,
                    state => state.flatten(),
                };
                let occlusion = match at.occlusion_patch(light, values, bones, &view, event_time, &pixels) {
                    None => Some(1.0),
                    Some(patch) => ce_flare_occlusion(gl, flares, camera, &view, &patch),
                };
                let Some(occlusion) = occlusion else {
                    continue;
                };
                if let Some(sprite) = at.sprite(light, values, bones, &view, event_time, occlusion, 0)
                    && let Some(textures) = uploaded.flare_textures.get(index)
                {
                    draw_flare_sprites(gl, flares, camera, &view, &[sprite], textures);
                }
            }
            self.gen3.sprite_cursor = 0;
            self.end_sprites(gl, frame, slot);
        }
    }
}

/// A sprite a game may draw as its own, inside the gen3 sprite pass.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) enum Gen3Sprite {
    /// Of `h4_tracers`.
    Tracer(usize),
    /// Of `h3_light_volumes`.
    Volume(usize),
    /// Of `h3_particles`.
    System(usize),
    /// Of `h3_leaves`.
    Leaves(usize),
}

/// What a game's own sprites draw with, inside the gen3 sprite pass: the
/// sprite program bound, which a sprite drawn through a program of its own
/// binds again after, and the shared simulations.
pub(in crate::app::model_preview) struct Gen3Sprites<'a> {
    pub(in crate::app::model_preview) gl: &'a glow::Context,
    pub(in crate::app::model_preview) frame: &'a ModelGpuFrame,
    pub(in crate::app::model_preview) flares: &'a SpriteGl,
    pub(in crate::app::model_preview) view: &'a ViewFrame,
    pub(in crate::app::model_preview) bones: Option<&'a [[f32; 4]]>,
    /// When each effect began.
    pub(in crate::app::model_preview) onsets: &'a mut H3EffectOnsets,
    /// Every game's particle systems.
    pub(in crate::app::model_preview) particles: &'a mut H3ParticleField,
    /// The particle program, the map an unmapped stage samples, and the
    /// exposure stops.
    pub(in crate::app::model_preview) particle_gl: Option<&'a H3ParticleGl>,
    pub(in crate::app::model_preview) no_map: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) stops: f32,
    /// The scene's depth, for the soft fade, and the targets' size.
    pub(in crate::app::model_preview) depth_copy: Option<glow::NativeTexture>,
    pub(in crate::app::model_preview) size: [i32; 2],
    pub(in crate::app::model_preview) placeholder: Option<glow::NativeTexture>,
    /// The frame's slot, and how many models it has taken.
    pub(in crate::app::model_preview) slot: usize,
    pub(in crate::app::model_preview) uploads: u64,
}
