//! Halo CE's and Halo 2's sprites over the shaded model: Halo 2's particle
//! systems far to near, the game's own sprites around the antennas, and its
//! lens flares.

use eframe::glow::{self, HasContext as _};

use super::super::common::sprite_gl::SpriteGl;
use super::super::common::sprites::{
    ParticleTargets, advance_sprite_simulations, begin_flares, draw_antennas, draw_particle_system,
};
use super::super::common::view::ViewFrame;
use super::super::common::view::camera_view_frame;
use super::super::effects::EffectOnsets;
use super::super::frame::ModelGpuFrame;
use super::super::gl::pass_state::PassState;
use super::super::renderer::RenderCore;
use super::pipeline::Dx9Game;
use crate::app::model_preview::common::sprites::layered_distance;

impl RenderCore {
    /// The frame's dx9 sprites, after the model and its wireframe, with
    /// `game`'s own among them.
    pub(in crate::app::model_preview) unsafe fn draw_dx9_sprites(
        &mut self,
        game: &mut dyn Dx9Game,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        unsafe {
            if self.begin_sprites(gl, frame, slot).is_none() {
                return;
            }
            self.draw_dx9_particles(frame, gl, slot);
            let Some(gen3) = self.begin_sprites(gl, frame, slot) else {
                return;
            };
            let Some(flares) = self.sprite_gl.as_ref().and_then(Option::as_ref) else {
                return;
            };
            let uploaded = &mut self.slots[slot];
            let camera = &frame.camera;
            let view = camera_view_frame(camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            // `rasterizer_light_volume_draw_sprites` sets its widget state
            // with `sub_936910(0, 3, 2)`: depth tested, less or equal, not
            // written. Liquids draw through it too, and contrails through
            // a transparent pass, which tests the same way.
            widget_state().apply(gl);
            let uploads = uploaded.uploads();
            let mut sprites = Dx9Sprites {
                gl,
                frame,
                flares,
                view: &view,
                bones,
                program: self.program,
                slot,
                uploads,
                onsets: &mut uploaded.simulations.effect_onsets,
            };
            game.draw_before_antennas(&mut sprites);
            draw_antennas(
                gl,
                flares,
                &mut uploaded.simulations.antennas,
                &uploaded.antenna_textures,
                frame,
                bones,
                &view,
                gen3,
            );
            game.draw_own_sprites(&mut sprites);
            gl.color_mask(true, true, true, false);
            begin_flares(gl, flares, self.placeholder);
            game.draw_flares(&mut sprites);
            self.end_sprites(gl, frame, slot);
        }
    }

    /// `render_effects`, before the other sprites: Halo 2's particle systems,
    /// far to near, each from its simulation. What bends the scene draws
    /// only into gen3's distortion target, so none of it here.
    unsafe fn draw_dx9_particles(
        &mut self,
        frame: &ModelGpuFrame,
        gl: &glow::Context,
        slot: usize,
    ) {
        unsafe {
            let particle_gl = self.particle_gl.as_ref().and_then(Option::as_ref);
            let uploaded = &mut self.slots[slot];
            let view = camera_view_frame(&frame.camera);
            let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
            gl.color_mask(true, true, true, false);
            advance_sprite_simulations(uploaded, frame, bones, &view, &mut self.sprites_advanced);
            let systems = frame.preview.particles.iter().enumerate();
            let mut sorted: Vec<_> = systems
                .filter(|(_, system)| system.look.shader.distortion.is_none())
                .map(|(at, system)| {
                    let distance = uploaded
                        .simulations
                        .particles
                        .distance(at, system, bones, &view);
                    (
                        layered_distance(system.look.shader.sort_layer, distance),
                        at,
                    )
                })
                .collect();
            sorted.sort_by(|(a, _), (b, _)| b.total_cmp(a));
            for (_, index) in sorted {
                if !frame
                    .shown
                    .is(super::super::common::features::paths::PARTICLES, index)
                {
                    continue;
                }
                let Some(particles) = particle_gl else {
                    continue;
                };
                let targets = ParticleTargets {
                    no_map: self.black_flat,
                    depth_copy: None,
                    distortion_copy: &mut self.distortion_copy,
                    stops: 0.0,
                };
                draw_particle_system(gl, particles, uploaded, frame, index, &view, targets);
            }
        }
    }
}

/// The widgets' state: depth tested, less or equal, not written.
fn widget_state() -> PassState {
    PassState::new()
        .enable(glow::DEPTH_TEST)
        .depth_func(glow::LEQUAL)
        .depth_mask(false)
}

/// What a game's own sprites draw with, inside the dx9 sprite pass: the
/// sprite program bound, and the frame's view and pose.
pub(in crate::app::model_preview) struct Dx9Sprites<'a> {
    pub(in crate::app::model_preview) gl: &'a glow::Context,
    pub(in crate::app::model_preview) frame: &'a ModelGpuFrame,
    pub(in crate::app::model_preview) flares: &'a SpriteGl,
    pub(in crate::app::model_preview) view: &'a ViewFrame,
    pub(in crate::app::model_preview) bones: Option<&'a [[f32; 4]]>,
    /// The shared program, for what draws through a program of its own and
    /// must hand state back.
    pub(in crate::app::model_preview) program: glow::NativeProgram,
    /// The frame's slot, and how many models it has taken.
    pub(in crate::app::model_preview) slot: usize,
    pub(in crate::app::model_preview) uploads: u64,
    /// When each effect began.
    pub(in crate::app::model_preview) onsets: &'a mut EffectOnsets,
}
