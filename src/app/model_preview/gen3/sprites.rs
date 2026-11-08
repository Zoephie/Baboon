//! The gen3 games' sprites, which the pipeline enters by stage from inside
//! its blended sort: light volumes, Halo 4's tracers, particle systems,
//! strips and leaves far to near, then the antennas, Reach's and Halo 4's
//! cheap particles and the lens flares.

use eframe::glow::{self, HasContext as _};
use super::state::Gen3State;

use super::super::common::sprites::{
    ParticleTargets, advance_sprite_simulations, begin_flares, draw_antennas, draw_particle_system, draw_sprite_quad,
};
use super::super::renderer::*;
use super::super::common::camera::*;
use super::super::common::view::ce_flare_view;
use super::super::*;
use super::super::common::view::ViewFrame;
use super::super::super::effects::EffectOnsets;
use super::super::particles::H3ParticleField;
use super::super::gl::pass_state::PassState;
use super::pipeline::{h3_layered_distance, h3_lighting_constants, h3_set_blend};
use super::super::reach::lighting::reach_exposure;
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

impl Gen3State {
    /// The part of the frame's sprites `stage` names.
    pub(in crate::app::model_preview) unsafe fn draw_sprites(
        &mut self,
        core: &mut RenderCore,
        game: &mut dyn Gen3Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
        stage: SpriteStage,
    ) {
        // A run with nothing farther than its limit sets nothing up and hands
        // nothing back. Only once the frame's queue is built, so the call that
        // builds it (the frame's first) still steps the simulations, syncs the
        // maps and sorts, as before.
        if let SpriteStage::FartherThan(limit) = stage
            && self.sprite_queue.as_ref().is_some_and(|queue| !queue.has_farther_than(limit))
        {
            return;
        }
        unsafe {
            let Some(h3) = core.begin_sprites(gl, frame, slot) else {
                return;
            };
            let Some(flares) = core.sprite_gl.as_ref().and_then(Option::as_ref) else {
                return;
            };
            let uploaded = &mut core.slots[slot];
            let camera = &frame.camera;
            let pass = SpritePass {
                gl,
                frame,
                flares,
                particle_gl: core.particle_gl.as_ref().and_then(Option::as_ref),
                no_map: core.black_flat,
                // What Reach's area screen effects do to the sprites' exposure.
                stops: self.stops,
                placeholder: core.placeholder,
                size: self.targets.as_ref().map_or([1, 1], |targets| targets.size),
                depth_copy: self.depth_copy,
                slot,
                uploads: uploaded.uploads(),
                camera,
                view: ce_flare_view(camera),
                bones: frame.bones.as_ref().map(|bones| bones.as_slice()),
            };
            if matches!(stage, SpriteStage::Rest) {
                let (gl, flares, bones, view) = (pass.gl, pass.flares, pass.bones, &pass.view);
                draw_antennas(gl, flares, &mut uploaded.simulations.antennas, &uploaded.antenna_textures, frame, bones, view, h3);
            }
            gl.color_mask(true, true, true, false);
            pass.advance(game, uploaded, &mut core.sprites_advanced);
            // Sorted once a frame, the first time its sprites draw: the
            // simulations have stepped by then, and drawing moves nothing a
            // sort reads.
            let queue = self.sprite_queue.get_or_insert_with(|| SpriteQueue {
                sorted: pass.sorted(SpriteStage::Rest, uploaded),
                next: 0,
                distortions: pass.sorted(SpriteStage::Distortions, uploaded),
            });
            if matches!(stage, SpriteStage::Distortions) {
                let mut distorted = 0;
                for &(_, sprite) in &queue.distortions {
                    distorted += 1;
                    pass.draw_sorted(game, uploaded, &mut core.h2_distortion_copy, sprite);
                }
                self.distorted = distorted > 0;
            } else {
                while let Some(sprite) = queue.next_farther_than(stage_limit(stage)) {
                    pass.draw_sorted(game, uploaded, &mut core.h2_distortion_copy, sprite);
                }
            }
            if matches!(stage, SpriteStage::Rest) {
                pass.draw_cheap_particles(uploaded, &mut core.reach_cheap_gl, core.program);
            }
            if matches!(stage, SpriteStage::FartherThan(_) | SpriteStage::Distortions) {
                back_to_materials_state(core.program).apply(gl);
                return;
            }
            begin_flares(gl, flares, core.placeholder);
            let viewport = core.scene.as_ref().map_or([1, 1], |scene| scene.size);
            pass.draw_flares(uploaded, viewport);
            self.sprite_queue = None;
            core.end_sprites(gl, frame, slot);
        }
    }
}

/// What a partial sprite run hands the blended materials back: the add
/// equation, every channel written, the model's program bound.
fn back_to_materials_state(program: glow::NativeProgram) -> PassState {
    PassState::new().blend_equation(glow::FUNC_ADD).color_mask([true, true, true, true]).use_program(program)
}

/// How near the sorted sprites a stage draws come: `FartherThan`'s limit; for
/// the rest, none (a finite distance is nearer than infinity: the last
/// `FartherThan`, at minus infinity, has drawn them all); the distortions draw
/// their own list whole.
fn stage_limit(stage: SpriteStage) -> f32 {
    match stage {
        SpriteStage::FartherThan(limit) => limit,
        SpriteStage::Rest => f32::INFINITY,
        SpriteStage::Distortions => f32::NEG_INFINITY,
    }
}

/// One frame's sprites in `c_transparency_renderer`'s order, sorted once and
/// drawn in runs as the blended sort reaches them.
pub(in crate::app::model_preview) struct SpriteQueue {
    /// What draws among the blended materials, far to near.
    sorted: Vec<(f32, SortedSprite)>,
    /// How many of `sorted` have drawn.
    next: usize,
    /// What bends the scene, far to near, drawn whole to its own target.
    distortions: Vec<(f32, SortedSprite)>,
}

impl SpriteQueue {
    /// Whether the next sprite not yet drawn is at least `limit` away, as
    /// [`next_farther_than`](Self::next_farther_than) would draw it.
    fn has_farther_than(&self, limit: f32) -> bool {
        // Not nearer, which an incomparable distance is not either.
        self.sorted.get(self.next).is_some_and(|&(distance, _)| distance.partial_cmp(&limit) != Some(std::cmp::Ordering::Less))
    }

    /// The next sprite not yet drawn, if it is at least `limit` away.
    fn next_farther_than(&mut self, limit: f32) -> Option<SortedSprite> {
        let &(distance, sprite) = self.sorted.get(self.next)?;
        if distance < limit {
            return None;
        }
        self.next += 1;
        Some(sprite)
    }
}

/// One of the sorted sprites, by kind and index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) enum SortedSprite {
    Volume(usize),
    Tracer(usize),
    System(usize),
    Strip(usize),
    Leaves(usize),
}

/// What one call of the sprite pass draws with: the sprite and particle
/// programs, the frame's view and pose, and the exposure and targets.
struct SpritePass<'a> {
    gl: &'a glow::Context,
    frame: &'a ModelGpuFrame,
    flares: &'a SpriteGl,
    particle_gl: Option<&'a H3ParticleGl>,
    /// The map an unmapped stage samples.
    no_map: Option<glow::NativeTexture>,
    stops: f32,
    placeholder: Option<glow::NativeTexture>,
    size: [i32; 2],
    depth_copy: Option<glow::NativeTexture>,
    slot: usize,
    uploads: u64,
    camera: &'a ModelGpuCamera,
    view: ViewFrame,
    bones: Option<&'a [[f32; 4]]>,
}

impl SpritePass<'_> {
    /// What a game's own sprites draw with, over the slot's shared simulations.
    fn sprites<'b>(&'b self, uploaded: &'b mut ModelGpuSlot) -> Gen3Sprites<'b> {
        Gen3Sprites {
            gl: self.gl,
            frame: self.frame,
            flares: self.flares,
            view: &self.view,
            bones: self.bones,
            onsets: &mut uploaded.simulations.effect_onsets,
            particles: &mut uploaded.simulations.particles,
            particle_gl: self.particle_gl,
            no_map: self.no_map,
            stops: self.stops,
            depth_copy: self.depth_copy,
            size: self.size,
            placeholder: self.placeholder,
            slot: self.slot,
            uploads: self.uploads,
        }
    }

    /// `render_effects`, among the transparents: every profile is its own
    /// quad. The shared simulations step once a frame, by whichever call draws
    /// first, and the game's own with them.
    unsafe fn advance(&self, game: &mut dyn Gen3Game, uploaded: &mut ModelGpuSlot, advanced: &mut bool) {
        let first = !*advanced;
        advance_sprite_simulations(uploaded, self.frame, self.bones, &self.view, advanced);
        if first {
            unsafe { game.advance_own_sprites(&mut self.sprites(uploaded)) };
        }
    }

    /// `c_transparency_renderer` sorts far to near, each from its location:
    /// the light volumes, Halo 4's tracers, the particle systems (only those
    /// that bend the scene for the distortions), strips and leaves.
    fn sorted(&self, stage: SpriteStage, uploaded: &ModelGpuSlot) -> Vec<(f32, SortedSprite)> {
        let (frame, bones, view) = (self.frame, self.bones, &self.view);
        let eye_distance = |origin: [f32; 3]| {
            (0..3).map(|axis| (origin[axis] - view.eye[axis]) * view.forward[axis]).sum::<f32>()
        };
        let layered = h3_layered_distance;
        let volumes = frame.preview.gen3.light_volumes.iter().enumerate();
        let volumes = volumes
            .filter(|_| !matches!(stage, SpriteStage::Distortions))
            .map(|(at, volume)| (layered(volume.sort_layer(), eye_distance(volume.origin(bones))), SortedSprite::Volume(at)));
        let tracers = frame.preview.h4.tracers.iter().enumerate();
        let tracers = tracers.filter(|_| !matches!(stage, SpriteStage::Distortions)).map(|(at, tracer)| {
            let distance = eye_distance(tracer.origin(bones)) - tracer.sort_bias();
            (layered(tracer.sort_layer(), distance), SortedSprite::Tracer(at))
        });
        let systems = frame.preview.particles.iter().enumerate();
        let distorting = matches!(stage, SpriteStage::Distortions);
        let systems = systems
            .filter(|(_, system)| system.look.shader.distortion.is_some() == distorting)
            .map(|(at, system)| (layered(system.look.shader.sort_layer, uploaded.simulations.particles.distance(at, system, bones, view)), SortedSprite::System(at)));
        let strips = frame.preview.gen3.strips.iter().enumerate();
        let strips = strips
            .filter(|_| !matches!(stage, SpriteStage::Distortions))
            .map(|(at, strip)| {
                let distance = eye_distance(strip.origin(bones)) - strip.sort_bias();
                (layered(strip.shader().sort_layer, distance), SortedSprite::Strip(at))
            });
        let leaves = frame.preview.h3.leaves.iter().enumerate();
        let leaves = leaves
            .filter(|_| !matches!(stage, SpriteStage::Distortions))
            .map(|(at, leaves)| (layered(2, eye_distance(leaves.origin())), SortedSprite::Leaves(at)));
        let mut sorted: Vec<_> = volumes.chain(tracers).chain(systems).chain(strips).chain(leaves).collect();
        sorted.sort_by(|(a, _), (b, _)| b.total_cmp(a));
        sorted
    }

    /// One sorted sprite: the game's own first, through its hook, else the
    /// shared draw for its kind.
    unsafe fn draw_sorted(
        &self,
        game: &mut dyn Gen3Game,
        uploaded: &mut ModelGpuSlot,
        distortion_copy: &mut Option<super::super::gl::target::ColorTarget>,
        sprite: SortedSprite,
    ) {
        let frame = self.frame;
        unsafe {
            match sprite {
                SortedSprite::Tracer(index) => {
                    if frame.shown.is(super::super::common::features::paths::H4_TRACERS, index) {
                        game.draw_own_sprite(&mut self.sprites(uploaded), Gen3Sprite::Tracer(index));
                    }
                }
                SortedSprite::Volume(index) => self.draw_volume(game, uploaded, index),
                SortedSprite::System(index) => self.draw_system(game, uploaded, distortion_copy, index),
                SortedSprite::Leaves(index) => {
                    game.draw_own_sprite(&mut self.sprites(uploaded), Gen3Sprite::Leaves(index));
                }
                SortedSprite::Strip(index) => self.draw_strip(uploaded, index),
            }
        }
    }

    /// A light volume, through the sprite program: its map or its circular
    /// falloff, faded by the scene's depth where it asks.
    unsafe fn draw_volume(&self, game: &mut dyn Gen3Game, uploaded: &mut ModelGpuSlot, index: usize) {
        let (gl, frame, flares, camera) = (self.gl, self.frame, self.flares, self.camera);
        let at = &frame.preview.gen3.light_volumes[index];
        if !frame.shown.is(super::super::common::features::paths::GEN3_LIGHT_VOLUMES, index) {
            return;
        }
        unsafe {
            if game.draw_own_sprite(&mut self.sprites(uploaded), Gen3Sprite::Volume(index)) {
                return;
            }
            let texture = uploaded.volume_textures.get(index).copied().flatten();
            let circular = at.circular();
            if texture.is_none() && circular.is_none() {
                return;
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
            let fade = at.depth_fade().filter(|_| self.depth_copy.is_some());
            if let Some(range) = fade {
                gl.active_texture(glow::TEXTURE1);
                gl.bind_texture(glow::TEXTURE_2D, self.depth_copy);
                gl.active_texture(glow::TEXTURE0);
                gl.uniform_1_i32(flares.scene_depth.as_ref(), 1);
                gl.uniform_4_f32_slice(flares.depth.as_ref(), &scene_depth_terms(camera));
                gl.uniform_1_f32(flares.fade.as_ref(), range);
            }
            let values = frame.function_values_of_object(at.object);
            let onset = uploaded.simulations.effect_onsets.of(at.object, at.scale_name(), values, frame.time);
            for quad in at.quads(values, self.bones, &self.view, frame.time, onset, self.stops) {
                draw_sprite_quad(gl, flares, camera, quad.corners, quad.color);
            }
            // The flare program's other users draw maps, unfaded.
            gl.uniform_2_f32(flares.circular.as_ref(), 0.0, 0.0);
            if fade.is_some() {
                gl.uniform_1_f32(flares.fade.as_ref(), 0.0);
                gl.uniform_1_i32(flares.scene_depth.as_ref(), 0);
            }
        }
    }

    /// A particle system, the game's own or through the particle program.
    unsafe fn draw_system(
        &self,
        game: &mut dyn Gen3Game,
        uploaded: &mut ModelGpuSlot,
        distortion_copy: &mut Option<super::super::gl::target::ColorTarget>,
        index: usize,
    ) {
        if !self.frame.shown.is(super::super::common::features::paths::PARTICLES, index) {
            return;
        }
        unsafe {
            if game.draw_own_sprite(&mut self.sprites(uploaded), Gen3Sprite::System(index)) {
                return;
            }
            let Some(particles) = self.particle_gl else {
                return;
            };
            let targets = ParticleTargets { no_map: self.no_map, depth_copy: self.depth_copy, distortion_copy, stops: self.stops };
            draw_particle_system(self.gl, particles, uploaded, self.frame, index, &self.view, targets);
        }
    }

    /// A strip, through the particle program, from its simulation.
    unsafe fn draw_strip(&self, uploaded: &mut ModelGpuSlot, index: usize) {
        let (frame, camera, stops) = (self.frame, self.camera, self.stops);
        let strip = &frame.preview.gen3.strips[index];
        if !frame.shown.is(super::super::common::features::paths::GEN3_STRIPS, index) {
            return;
        }
        let Some(particles) = self.particle_gl else {
            return;
        };
        let values = frame.function_values_of_object(strip.object);
        let onset = uploaded.simulations.effect_onsets.of(strip.object, strip.scale_name(), values, frame.time);
        let lighting = h3_lighting_constants(frame);
        let drawn = uploaded.simulations.h3_strips.vertices(index, strip, values, self.bones, &self.view, &lighting, frame.time, onset, stops);
        if drawn.is_empty() {
            return;
        }
        let vertices = particle_floats(camera, &drawn);
        let textures = uploaded.strip_textures.get(index).copied().unwrap_or_default();
        unsafe { particles.draw(self.gl, strip.shader(), &vertices, textures, self.no_map, self.depth_copy, camera, &lighting, stops) };
    }

    /// `s_cheap_particle_system::render`: after every other transparent, in
    /// one batch, in ring-slot order.
    unsafe fn draw_cheap_particles(
        &self,
        uploaded: &mut ModelGpuSlot,
        cheap_gl: &mut Option<Option<ReachCheapGl>>,
        program: glow::NativeProgram,
    ) {
        let (gl, frame, camera) = (self.gl, self.frame, self.camera);
        let (Some(first), Some(texture)) = (frame.preview.gen3.cheap_emitters.first(), uploaded.reach_cheap_texture) else {
            return;
        };
        let drawn = uploaded.simulations.reach_cheap.vertices(first.library(), &self.view);
        unsafe {
            if !drawn.is_empty()
                && let Some(cheap) = cheap_gl.get_or_insert_with(|| reach_cheap_gl(gl, program)).as_ref()
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
                    let exposure = reach_exposure(self.stops)[0];
                    [exposure, exposure]
                };
                cheap.draw(gl, &vertices, texture, exposures);
            }
        }
    }

    /// The lens flares, under the flares' added blend: each tested for
    /// occlusion, and a light's own going out with its light.
    unsafe fn draw_flares(&self, uploaded: &mut ModelGpuSlot, viewport: [i32; 2]) {
        let (gl, frame, flares, camera, bones, view) = (self.gl, self.frame, self.flares, self.camera, self.bones, &self.view);
        let pixels = |center: [f32; 3], half: [f32; 2]| ce_flare_pixels(camera, viewport, center, half);
        for (index, at) in frame.preview.gen3.flares.iter().enumerate() {
            if !frame.shown.is(super::super::common::features::paths::GEN3_FLARES, index) {
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
            let occlusion = match at.occlusion_patch(light, values, bones, view, event_time, &pixels) {
                None => Some(1.0),
                Some(patch) => unsafe { ce_flare_occlusion(gl, flares, camera, view, &patch) },
            };
            let Some(occlusion) = occlusion else {
                continue;
            };
            if let Some(sprite) = at.sprite(light, values, bones, view, event_time, occlusion, 0)
                && let Some(textures) = uploaded.flare_textures.get(index)
            {
                unsafe { draw_flare_sprites(gl, flares, camera, view, &[sprite], textures) };
            }
        }
    }
}

/// A sprite a game may draw as its own, inside the gen3 sprite pass.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) enum Gen3Sprite {
    /// Of `h4.tracers`.
    Tracer(usize),
    /// Of `gen3.light_volumes`.
    Volume(usize),
    /// Of `particles`.
    System(usize),
    /// Of `h3.leaves`.
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
    pub(in crate::app::model_preview) onsets: &'a mut EffectOnsets,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Today's staged draw before the queue: each call sorted the whole list
    /// again, skipped the `cursor` already drawn, and drew while not nearer
    /// than the limit.
    fn drawn_by_cursor(sorted: &[(f32, SortedSprite)], cursor: &mut usize, limit: f32) -> Vec<SortedSprite> {
        let mut drawn = Vec::new();
        for &(distance, sprite) in sorted.iter().skip(*cursor) {
            if distance < limit {
                break;
            }
            *cursor += 1;
            drawn.push(sprite);
        }
        drawn
    }

    #[test]
    fn the_queue_draws_each_stage_as_the_cursor_did() {
        // Far to near, as `sorted` leaves them, with ties and a distance that
        // is no number (it compares as not nearer, so it draws).
        let sorted = vec![
            (9.0, SortedSprite::Volume(0)),
            (7.5, SortedSprite::System(2)),
            (7.5, SortedSprite::Strip(1)),
            (f32::NAN, SortedSprite::Tracer(0)),
            (3.0, SortedSprite::Leaves(0)),
            (3.0, SortedSprite::System(0)),
            (-2.0, SortedSprite::Volume(1)),
        ];
        // A frame's stages: the blended batches' distances, minus infinity
        // after the last, then the rest.
        let stages = [
            SpriteStage::FartherThan(10.0),
            SpriteStage::FartherThan(7.5),
            SpriteStage::FartherThan(7.5),
            SpriteStage::FartherThan(4.0),
            SpriteStage::FartherThan(3.0),
            SpriteStage::FartherThan(f32::NEG_INFINITY),
            SpriteStage::Rest,
        ];
        let mut queue = SpriteQueue { sorted: sorted.clone(), next: 0, distortions: Vec::new() };
        let mut cursor = 0;
        for stage in stages {
            let before = drawn_by_cursor(&sorted, &mut cursor, stage_limit(stage));
            let mut now = Vec::new();
            while let Some(sprite) = queue.next_farther_than(stage_limit(stage)) {
                now.push(sprite);
            }
            assert_eq!(now, before, "{stage:?}");
        }
        assert_eq!(queue.next, sorted.len(), "every sprite drew once");
    }

    #[test]
    fn a_run_is_empty_exactly_when_the_queue_would_draw_nothing() {
        let sorted = vec![
            (9.0, SortedSprite::Volume(0)),
            (f32::NAN, SortedSprite::Tracer(0)),
            (3.0, SortedSprite::Leaves(0)),
        ];
        let limits = [10.0, 9.0, 5.0, f32::NAN, 3.0, 2.0, f32::NEG_INFINITY, f32::INFINITY];
        for start in 0..=sorted.len() {
            for limit in limits {
                let queue = SpriteQueue { sorted: sorted.clone(), next: start, distortions: Vec::new() };
                let mut drawing = SpriteQueue { sorted: sorted.clone(), next: start, distortions: Vec::new() };
                let draws = drawing.next_farther_than(limit).is_some();
                assert_eq!(queue.has_farther_than(limit), draws, "from {start} with limit {limit}");
            }
        }
    }
}
