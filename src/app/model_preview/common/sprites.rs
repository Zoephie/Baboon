//! What the dx9 and gen3 sprite passes share: the sprite program set up, one
//! quad drawn through it, the effect simulations advanced once a frame,
//! antennas, a particle system through the particle program, and the state
//! put back after.

use eframe::glow::{self, HasContext as _};

use super::super::particles::{ParticleGl, gen3_particle_gl, particle_floats};

use super::super::gl::buffer::slice_bytes;

use super::super::gl::target::copy_frame;

use super::super::common::sprite_gl::{SpriteGl, link_sprite_gl};

use super::super::gl::pass_state::PassState;
use super::super::*;
use super::view::ViewFrame;

impl RenderCore {
    /// Link the sprite and particle programs on first use, bring the slot's
    /// sprite maps up to date, and set the sprite program's state. Whether
    /// the gen3 target is drawn to, or `None` with no sprite program.
    pub(in crate::app::model_preview) unsafe fn begin_sprites(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) -> Option<bool> {
        unsafe {
            let h3 = self.gen3_draws(frame);
            let flares = self
                .sprite_gl
                .get_or_insert_with(|| link_sprite_gl(gl, self.program))
                .as_ref()?;
            if !(frame.preview.particles.is_empty()
                && frame.preview.gen3.strips.is_empty()
                && frame.preview.h3.leaves.is_empty())
            {
                self.particle_gl
                    .get_or_insert_with(|| gen3_particle_gl(gl, self.program));
            }
            self.slots[slot].sync_sprite_textures(gl, frame);
            sprite_program_state(flares).apply(gl);
            gl.uniform_1_i32(flares.map.as_ref(), 0);
            sprite_blending_state(h3).apply(gl);
            // Plain color times map, not what the last flare left set.
            gl.uniform_1_f32(flares.highlight.as_ref(), -1.0);
            gl.uniform_2_f32(flares.tint.as_ref(), 0.0, 0.0);
            Some(h3)
        }
    }

    /// The state the model's own drawing expects back after a frame's last
    /// sprites, and the next frame's simulations let step again.
    pub(in crate::app::model_preview) unsafe fn end_sprites(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
        slot: usize,
    ) {
        unsafe {
            self.sprites_advanced = false;
            after_sprites_state(
                frame.show_backfaces,
                self.program,
                self.slots[slot].vertex_array,
            )
            .apply(gl);
        }
    }
}

/// The sprite program bound, its quad's buffer and the first map unit with it.
fn sprite_program_state(flares: &SpriteGl) -> PassState {
    PassState::new()
        .use_program(flares.program)
        .bind_vertex_array(flares.vertex_array)
        .bind_buffer(glow::ARRAY_BUFFER, flares.buffer)
        .active_texture(glow::TEXTURE0)
}

/// How sprites blend over the model: tested against its depth, not
/// writing it, both sides drawn.
fn sprite_blending_state(h3: bool) -> PassState {
    let state = PassState::new()
        .enable(glow::BLEND)
        .blend_equation(glow::FUNC_ADD)
        .color_mask([true, true, true, false])
        // Effects test against the model's depth, as the transparent passes
        // do. Bloom leaves the test off, so it's switched on here.
        .enable(glow::DEPTH_TEST)
        .depth_mask(false)
        .depth_func(glow::LEQUAL)
        .disable(glow::CULL_FACE)
        // `global_shader_effect_alpha_blended`, among the model's depth.
        .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
    if !h3 {
        return state;
    }
    // Halo 3's target is composited over the background by its alpha, so
    // what covers it has to say so.
    state
        .color_mask([true, true, true, true])
        .blend_func_separate(
            glow::SRC_ALPHA,
            glow::ONE_MINUS_SRC_ALPHA,
            glow::ONE,
            glow::ONE_MINUS_SRC_ALPHA,
        )
}

/// What the model's own drawing expects back after the sprites: depth
/// written and tested, blending off, its program and vertices bound.
fn after_sprites_state(
    show_backfaces: bool,
    program: glow::NativeProgram,
    vertex_array: glow::NativeVertexArray,
) -> PassState {
    PassState::new()
        .color_mask([true, true, true, true])
        .depth_mask(true)
        .depth_func(glow::LEQUAL)
        .enable(glow::DEPTH_TEST)
        // Sprites draw both sides; what comes after (bloom's masks) culls as
        // the model does.
        .culling_unless(show_backfaces)
        .disable(glow::BLEND)
        .bind_texture(glow::TEXTURE_2D, None)
        .use_program(program)
        .bind_vertex_array(vertex_array)
}

/// The lens flares' additive blend, through the sprite program, with a map
/// bound before any flare binds its own.
fn flares_state(flares: &SpriteGl, placeholder: Option<glow::NativeTexture>) -> PassState {
    PassState::new()
        .use_program(flares.program)
        .bind_vertex_array(flares.vertex_array)
        .bind_buffer(glow::ARRAY_BUFFER, flares.buffer)
        .active_texture(glow::TEXTURE0)
        // The occlusion tests draw with this program too, before any sprite
        // binds a map; an unbound sampler makes drivers complain.
        .bind_texture(glow::TEXTURE_2D, placeholder)
        .color_mask([true, true, true, false])
        .blend_equation(glow::FUNC_ADD)
        .blend_func(glow::SRC_ALPHA, glow::ONE)
}

/// One quad through the sprite program: its corners, each a place and a map
/// coordinate, taken to clip space and drawn in `color`.
pub(in crate::app::model_preview) unsafe fn draw_sprite_quad(
    gl: &glow::Context,
    flares: &SpriteGl,
    camera: &ModelGpuCamera,
    corners: [[f32; 5]; 4],
    [red, green, blue, alpha]: [f32; 4],
) {
    unsafe {
        let vertices = corners.map(|[x, y, z, u, v]| {
            let [a, b, c, d] = ce_flare_clip(camera, [x, y, z]);
            [a, b, c, d, u, v]
        });
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            slice_bytes(&vertices),
            glow::STREAM_DRAW,
        );
        gl.uniform_4_f32(flares.color.as_ref(), red, green, blue, alpha);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
    }
}

/// The sprites' simulations, stepped once a frame by whichever pass draws
/// first; `advanced` says whether one has.
pub(in crate::app::model_preview) fn advance_sprite_simulations(
    uploaded: &mut ModelGpuSlot,
    frame: &ModelGpuFrame,
    bones: Option<&[[f32; 4]]>,
    view: &ViewFrame,
    advanced: &mut bool,
) {
    if std::mem::replace(advanced, true) {
        return;
    }
    let simulations = &mut uploaded.simulations;
    if !frame.preview.particles.is_empty() {
        let systems = &frame.preview.particles;
        let values = |object: u8| frame.function_values_of_object(object).cloned();
        // The grid, or under the object if that hangs below it.
        let lowest = frame
            .preview
            .vertices
            .iter()
            .map(|vertex| vertex.position[2]);
        let ground = lowest.fold(0.0, f32::min);
        simulations.particles.advance(
            systems,
            values,
            &mut simulations.effect_onsets,
            bones,
            view.eye,
            ground,
            frame.time,
        );
    }
    if !frame.preview.gen3.cheap_emitters.is_empty() {
        let values = |object: u8| frame.function_values_of_object(object).cloned();
        let (emitters, shown) = (
            &frame.preview.gen3.cheap_emitters,
            frame
                .shown
                .list(super::features::paths::GEN3_CHEAP_EMITTERS),
        );
        simulations.reach_cheap.advance(
            emitters,
            shown,
            values,
            &mut simulations.effect_onsets,
            bones,
            view.eye,
            frame.time,
        );
    }
    if !frame.preview.gen3.strips.is_empty() {
        let values = |object: u8| frame.function_values_of_object(object).cloned();
        let (systems, particles) = (&frame.preview.particles, &simulations.particles);
        let tracks =
            |strip: &std::sync::Arc<gen3::effects::strips::Gen3StripAt>| -> Vec<(u64, [f32; 3])> {
                particles
                    .tracks_of(systems, strip)
                    .into_iter()
                    .map(|track| (track.id, track.place))
                    .collect()
            };
        simulations.h3_strips.advance(
            &frame.preview.gen3.strips,
            values,
            &mut simulations.effect_onsets,
            tracks,
            bones,
            frame.time,
        );
    }
}

/// The frame's antennas: Halo 2's and Halo 3's, and Reach's and Halo 4's,
/// each a strip of quads.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) unsafe fn draw_antennas(
    gl: &glow::Context,
    flares: &SpriteGl,
    springs: &mut antennas::spring::ReachAntennaField,
    textures: &[Option<glow::NativeTexture>],
    frame: &ModelGpuFrame,
    bones: Option<&[[f32; 4]]>,
    view: &ViewFrame,
    h3: bool,
) {
    unsafe {
        // Antennas are depth tested as well: `antenna_render_proper`
        // draws as a transparent group, tested and not written.
        if h3 {
            gl.blend_func_separate(
                glow::SRC_ALPHA,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
            );
        } else {
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
        gl.color_mask(true, true, true, h3);
        let tips = springs.advance(&frame.preview.antennas, bones, frame.time);
        for (quads, texture, reach) in h2_antennas(frame, textures, bones, view, &tips) {
            if reach {
                // `draw_textured_transparent_quad`'s blend mode 3: source
                // alpha over its inverse, which Xenon sets for color alone;
                // alpha is taken to blend the same. [inferred] Its
                // `transparent` shader writes color times map with no
                // exposure on it, into the lit target the composite
                // brings up by `REACH_DARK_MULTIPLIER`; that it shows at
                // eight times the tag's color follows the code, and is
                // not checked against a capture. [inferred]
                gen3::pipeline::gen3_set_blend(gl, gen3::shaders::Gen3Blend::AlphaBlend);
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for quad in quads {
                draw_sprite_quad(gl, flares, &frame.camera, quad.corners, quad.color);
            }
        }
    }
}

/// What a particle system draws with through the particle program.
pub(in crate::app::model_preview) struct ParticleTargets<'a> {
    /// The map an unmapped stage samples.
    pub(in crate::app::model_preview) no_map: Option<glow::NativeTexture>,
    /// The scene's depth, for the soft fade.
    pub(in crate::app::model_preview) depth_copy: Option<glow::NativeTexture>,
    /// Where Halo 2's distortion copies the frame.
    pub(in crate::app::model_preview) distortion_copy: &'a mut Option<gl::target::ColorTarget>,
    /// What Reach's area screen effects do to the exposure.
    pub(in crate::app::model_preview) stops: f32,
}

/// System `index` through the particle program, from its simulation: Halo
/// 2's, Halo 3's and Reach's.
pub(in crate::app::model_preview) unsafe fn draw_particle_system(
    gl: &glow::Context,
    particles: &ParticleGl,
    uploaded: &mut ModelGpuSlot,
    frame: &ModelGpuFrame,
    index: usize,
    view: &ViewFrame,
    targets: ParticleTargets<'_>,
) {
    unsafe {
        let system = &frame.preview.particles[index];
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let camera = &frame.camera;
        let values = frame.function_values_of_object(system.object);
        let lighting = gen3::pipeline::gen3_lighting_constants(frame);
        let onset = uploaded.simulations.effect_onsets.of(
            system.object,
            system.scale_name(),
            values,
            frame.time,
        );
        let drawn = uploaded.simulations.particles.vertices(
            index,
            system,
            values,
            bones,
            view,
            &lighting,
            frame.time,
            onset,
            targets.stops,
        );
        if drawn.is_empty() {
            return;
        }
        let vertices = particle_floats(camera, &drawn);
        let mut textures = uploaded
            .particle_textures
            .get(index)
            .copied()
            .unwrap_or_default();
        if system.look.shader.albedo == particles::ParticleAlbedo::H2Distortion {
            // `sub_915630`: before a pass that reads `GLOBAL particle
            // distortion`, the frame as it stands is copied there.
            textures[2] = copy_frame(gl, targets.distortion_copy);
        }
        particles.draw(
            gl,
            &system.look.shader,
            &vertices,
            textures,
            targets.no_map,
            targets.depth_copy,
            camera,
            &lighting,
            targets.stops,
        );
    }
}

/// The sprite program again, for lens flares, which add.
pub(in crate::app::model_preview) unsafe fn begin_flares(
    gl: &glow::Context,
    flares: &SpriteGl,
    placeholder: Option<glow::NativeTexture>,
) {
    unsafe { flares_state(flares, placeholder).apply(gl) }
}

/// One antenna sprite's corners, in strip order, with texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AntennaQuad {
    pub(crate) corners: [[f32; 5]; 4],
    pub(crate) color: [f32; 4],
}

/// A distance to sort by, far first, that puts `sort layer` before it.
pub(in crate::app::model_preview) fn layered_distance(layer: i32, distance: f32) -> f32 {
    distance - layer as f32 * SORT_LAYER_SPAN
}

/// More than any preview is deep.
const SORT_LAYER_SPAN: f32 = 1e6;

#[cfg(test)]
mod layer_tests {
    use super::layered_distance;

    #[test]
    fn a_later_layer_draws_after_however_far_it_lies() {
        let far_post = layered_distance(3, 500.0);
        let near_normal = layered_distance(2, 1.0);
        assert!(far_post < near_normal);
        assert!(layered_distance(2, 5.0) > layered_distance(2, 1.0));
    }
}

/// Halo 2's, 3's and Reach's antennas, whose maps follow Halo CE's in the
/// slot's, each with whether it is Reach's; Reach's far ends are at `tips`.
/// Those on a permutation not shown are left out.
pub(in crate::app::model_preview) fn h2_antennas(
    frame: &ModelGpuFrame,
    textures: &[Option<glow::NativeTexture>],
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
    tips: &[Option<[f32; 3]>],
) -> Vec<(
    Vec<crate::app::model_preview::common::sprites::AntennaQuad>,
    glow::NativeTexture,
    bool,
)> {
    let preview = &frame.preview;
    preview
        .antennas
        .iter()
        .zip(textures)
        .enumerate()
        .filter(|(index, _)| frame.shown.is(common::features::paths::ANTENNAS, *index))
        .filter_map(|(index, (antenna, texture))| {
            let tip = tips.get(index).copied().flatten();
            Some((
                antenna.quads(bones, view, tip),
                (*texture)?,
                antenna.is_reach(),
            ))
        })
        .collect()
}
