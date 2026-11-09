//! Halo CE's effect sprites: advancing the effects, grouping their sprites by key and depth, and drawing each group.

use super::*;
use std::cell::RefCell;
use super::super::super::dx9::draw::{gl_blend_equation, gl_blend_factor};
use super::super::super::gl::texture::upload_h2_image;
use super::super::super::gl::buffer::slice_bytes;
use super::super::super::common::sprite_gl::SpriteGl;
use eframe::glow::{self, HasContext as _};
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    /// Models whose played Halo CE event effects still have something
    /// running, as the last frame drawn left them.
    pub(in crate::app::model_preview) static CE_EVENTS_RUNNING: RefCell<std::collections::HashSet<u64>> = RefCell::new(Default::default());
}

/// Whether a played event effect on `geometry_id` still needs frames.
pub(in crate::app::model_preview) fn ce_events_running(geometry_id: u64) -> bool {
    CE_EVENTS_RUNNING.with(|running| running.borrow().contains(&geometry_id))
}

/// `effect_update` for Halo CE's attached effects, up to the frame's time.
/// Stepping to a time it has reached does nothing, so the model's lights
/// and the sprites can both ask for it.
pub(in crate::app::model_preview) fn advance_ce_effects(ce: &mut ce::slot::CeSlot, frame: &ModelGpuFrame, bones: Option<&[[f32; 4]]>) {
    let states = &frame.object_states;
    let inputs = |object: u8| {
        let state = states.get(object as usize);
        ce::effects::CeObjectInputs {
                active: state.map_or([false; 4], |state| state.active),
            outputs: state.map_or([0.0; 4], |state| state.outputs),
            change_colors: state.map_or([[1.0; 3]; 4], |state| state.change_colors),
        }
    };
    let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
    let ground = lowest.fold(0.0, f32::min);
    let others = ce.simulations.pctl.particle_count();
    let effects = &frame.preview.ce.effects;
    ce.simulations.particles.advance(effects, &frame.ce_plays, inputs, bones, ground, frame.time, others);
    let running = ce.simulations.particles.runs_played(effects.len());
    CE_EVENTS_RUNNING.with(|set| {
        let mut set = set.borrow_mut();
        if running {
            set.insert(frame.geometry_id);
        } else {
            set.remove(&frame.geometry_id);
        }
    });
}

/// A Halo CE sprite through the `effect` shader: its map, framebuffer blend
/// function and nonlinear tint, each corner's color, its corners, and the
/// shader's second map.
type CeSprite = (glow::NativeTexture, ce::blend::FramebufferBlend, bool, [[f32; 4]; 4], [[f32; 5]; 4], Option<ce::sprite_sheet::CeSecondaryDraw>);

/// What one of Halo CE's transparent groups draws.
pub(in crate::app::model_preview) enum CeGroupDraw {
    /// Through the `effect` shader.
    Sprites(Vec<CeSprite>),
    /// Through the flares' plain color-times-map program: an antenna, or with
    /// `additive` a light volume, which `rasterizer_widget_begin(5, 1)` adds.
    Plain { quads: Vec<crate::app::model_preview::common::sprites::AntennaQuad>, texture: glow::NativeTexture, additive: bool },
}

/// One of Halo CE's transparent groups, which `rasterizer_transparent_geometry_draw`
/// sorts with the model's transparent parts by `z_sort`, far to near.
pub(in crate::app::model_preview) struct CeGroup {
    pub(in crate::app::model_preview) z_sort: f32,
    pub(in crate::app::model_preview) draw: CeGroupDraw,
}

/// How much nearer `sort bias` sorts a group (`_rasterizer_dynamic_unlit_geometry_draw`).
const CE_SORT_BIAS: f32 = 0.25;

/// `z_sort`: how far in front of the eye the point lies along the view,
/// negated, so the farthest sorts first; `sort bias` brings it nearer.
pub(in crate::app::model_preview) fn ce_z_sort(view: &common::view::ViewFrame, point: [f32; 3], sort_bias: bool) -> f32 {
    let along: f32 = (0..3).map(|axis| (point[axis] - view.eye[axis]) * view.forward[axis]).sum();
    if sort_bias { CE_SORT_BIAS - along } else { -along }
}

/// The mean of the quads' corners: `build_sprites_end`'s centroid, the mean
/// of its sprites' centers.
fn mean_corner<'a>(quads: impl Iterator<Item = &'a [[f32; 5]; 4]>) -> [f32; 3] {
    let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
    for corner in quads.flatten() {
        for axis in 0..3 {
            sum[axis] += corner[axis];
        }
        count += 1.0;
    }
    sum.map(|axis| axis / count.max(1.0))
}

/// The middle of the quads' bounds, as `lightning_submit` takes a bolt's.
fn bounds_middle<'a>(quads: impl Iterator<Item = &'a [[f32; 5]; 4]>) -> [f32; 3] {
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for corner in quads.flatten() {
        for axis in 0..3 {
            low[axis] = low[axis].min(corner[axis]);
            high[axis] = high[axis].max(corner[axis]);
        }
    }
    std::array::from_fn(|axis| if low[axis] <= high[axis] { (low[axis] + high[axis]) * 0.5 } else { 0.0 })
}

/// Which source a sprite group comes from, in [`CeSpriteGroups`]' keys.
const CE_LIGHTNING: u8 = 0;

const CE_EFFECT_SYSTEM: u8 = 1;

const CE_SYSTEM: u8 = 2;

const CE_GLOW: u8 = 3;

const CE_CONTRAIL: u8 = 4;

const CE_PARTICLE: u8 = 5;

/// Sprites gathered into the groups the game submits them in, in the order
/// each group first appears.
#[derive(Default)]
struct CeSpriteGroups {
    /// Each group's `sort bias`, whether its centroid is the middle of its
    /// bounds rather than the mean, and its sprites.
    groups: Vec<(bool, bool, Vec<CeSprite>)>,
    by_key: HashMap<(u8, usize, usize), usize>,
}

impl CeSpriteGroups {
    fn push(&mut self, key: (u8, usize, usize), sort_bias: bool, by_bounds: bool, sprite: CeSprite) {
        let at = *self.by_key.entry(key).or_insert_with(|| {
            self.groups.push((sort_bias, by_bounds, Vec::new()));
            self.groups.len() - 1
        });
        self.groups[at].2.push(sprite);
    }

    fn sorted_by(self, view: &common::view::ViewFrame) -> impl Iterator<Item = CeGroup> {
        self.groups.into_iter().map(move |(sort_bias, by_bounds, sprites)| {
            let corners = sprites.iter().map(|sprite| &sprite.4);
            let centroid = if by_bounds { bounds_middle(corners) } else { mean_corner(corners) };
            CeGroup { z_sort: ce_z_sort(view, centroid, sort_bias), draw: CeGroupDraw::Sprites(sprites) }
        })
    }
}

/// Halo CE's effects, particle systems, glows, contrails, lightning, antennas
/// and light volumes brought up to the frame, as the groups the game submits
/// them in: a bolt, a particle state's draw, a glow, a contrail, the
/// particles of each definition, an antenna, a light volume. The slot's maps
/// are uploaded already.
pub(in crate::app::model_preview) fn ce_groups(
    gl: &glow::Context,
    ce: &mut ce::slot::CeSlot,
    frame: &ModelGpuFrame,
    bones: Option<&[[f32; 4]]>,
    view: &common::view::ViewFrame,
) -> Vec<CeGroup> {
    let camera = &frame.camera;
    let effects = &frame.preview.ce.effects;
    let states = &frame.object_states;
    let lowest = frame.preview.vertices.iter().map(|vertex| vertex.position[2]);
    let ground = lowest.fold(0.0, f32::min);
    let mut viewport = [0; 4];
    unsafe { gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport) };
    let half_width = viewport[2].max(1) as f32 * 0.5;
    // `render_frustum_sphere_diameter_in_pixels`.
    let pixels = |center: [f32; 3], radius: f32| {
        let middle = ce_flare_clip(camera, center);
        let side = ce_flare_clip(camera, std::array::from_fn(|axis| center[axis] + view.right[axis] * radius));
        ((side[0] / side[3] - middle[0] / middle[3]).abs() * half_width * 2.0).max(0.0)
    };
    let contrails = &frame.preview.ce.contrails;
    let switched = |object: u8| {
        states.get(object as usize).map_or(([0.0; 4], [false; 4]), |state| (state.outputs, state.active))
    };
    ce.simulations.contrails.advance(contrails, switched, bones, ground, frame.time);
    let change_colors = |object: u8| states.get(object as usize).map_or([[1.0; 3]; 4], |state| state.change_colors);
    let mut sprites = CeSpriteGroups::default();
    // Made afresh every frame, as the game draws it.
    let seed = (frame.time * 1000.0) as u32;
    for (index, lightning) in frame.preview.ce.lightning.iter().enumerate() {
        let Some(texture) = ce.textures.lightning.get(index).copied().flatten() else {
            continue;
        };
        let seed = seed.wrapping_add(index as u32 * 7919);
        let state = states.get(usize::from(lightning.object));
        let outputs = state.map_or([0.0; 4], |state| state.outputs);
        let colors = state.map_or([[1.0; 3]; 4], |state| state.change_colors);
        for quad in lightning.quads(&outputs, &colors, bones, view, seed) {
            let sprite = (texture, lightning.blend, lightning.nonlinear_tint, quad.colors, quad.corners, None);
            sprites.push((CE_LIGHTNING, index, 0), lightning.sort_bias, true, sprite);
        }
    }
    // `particle_systems_render`, for those effects' parts made.
    let pixel_of = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
    for (effect, system, quad) in ce.simulations.particles.system_quads(effects, view, pixel_of) {
        let texture = ce
            .textures.effect_pctl
            .get(&effect)
            .and_then(|systems| systems.get(system))
            .and_then(|sheets| sheets.get(quad.sheet))
            .and_then(|images| images.get(quad.image))
            .copied()
            .flatten();
        if let Some(texture) = texture {
            let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
            sprites.push((CE_EFFECT_SYSTEM, quad.group, 0), quad.sort_bias, false, sprite);
        }
    }
    // And those on the object's attachments.
    let systems = &frame.preview.ce.particle_systems;
    if !systems.is_empty() {
        let inputs = |object: u8| {
            let state = states.get(usize::from(object));
            (
                state.map_or([0.0; 4], |state| state.outputs),
                state.map_or([false; 4], |state| state.active),
                state.map_or([[1.0; 3]; 4], |state| state.change_colors),
            )
        };
        let others = ce.simulations.particles.system_particle_count();
        ce.simulations.pctl.advance(systems, inputs, bones, ground, frame.time, others);
        let pixel = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
        for quad in ce.simulations.pctl.quads(systems, view, pixel) {
            let texture = ce.textures.pctl.get(quad.system).and_then(|sheets| sheets.get(quad.sheet)).and_then(|images| images.get(quad.image)).copied().flatten();
            if let Some(texture) = texture {
                let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
                sprites.push((CE_SYSTEM, quad.group, 0), quad.sort_bias, false, sprite);
            }
        }
    }
    // `glow_render`, through `global_shader_effect_additive`.
    let glows = &frame.preview.ce.glows;
    // `glow_update` reads an output only while it is on, else nothing.
    let read_if_on = |object: u8| {
        let (outputs, active) = switched(object);
        std::array::from_fn(|slot| if active[slot] { outputs[slot] } else { 0.0 })
    };
    ce.simulations.glows.advance(glows, read_if_on, bones, frame.time);
    let pixel = |center: [f32; 3]| 2.0 / pixels(center, 1.0).max(0.0001);
    for (index, quad) in ce.simulations.glows.quads(glows, view, pixel) {
        if let Some(texture) = ce.textures.glows.get(index).copied().flatten() {
            sprites.push((CE_GLOW, index, 0), false, false, (texture, ce::blend::FramebufferBlend::Add, false, quad.colors, quad.corners, None));
        }
    }
    for quad in ce.simulations.contrails.quads(contrails, change_colors, view) {
        let Some(contrail) = contrails.get(quad.contrail) else {
            continue;
        };
        let texture = ce.textures.contrails.get(quad.contrail).and_then(|images| images.get(quad.image)).copied().flatten();
        if let Some(texture) = texture {
            let sprite = (texture, contrail.def.blend, contrail.def.nonlinear_tint, quad.colors, quad.corners, None);
            sprites.push((CE_CONTRAIL, quad.contrail, 0), contrail.def.sort_bias, false, sprite);
        }
    }
    for quad in ce.simulations.particles.quads(effects, bones, view, pixels) {
        let texture = ce
            .textures.particles
            .get(&quad.effect)
            .and_then(|defs| defs.get(quad.def))
            .and_then(|images| images.get(quad.image))
            .copied()
            .flatten();
        if let Some(texture) = texture {
            let sprite = (texture, quad.blend, quad.nonlinear_tint, [quad.color; 4], quad.corners, quad.secondary);
            sprites.push((CE_PARTICLE, quad.effect, quad.def), quad.sort_bias, false, sprite);
        }
    }
    let mut groups: Vec<CeGroup> = sprites.sorted_by(view).collect();
    // `antenna_render_proper`: one group each, at the mean of its sprites.
    for (antenna, texture) in frame.preview.ce.antennas.iter().zip(&ce.textures.antennas) {
        let Some(texture) = *texture else {
            continue;
        };
        let quads = antenna.quads(bones, view);
        let centroid = mean_corner(quads.iter().map(|quad| &quad.corners));
        let draw = CeGroupDraw::Plain { quads, texture, additive: false };
        groups.push(CeGroup { z_sort: ce_z_sort(view, centroid, false), draw });
    }
    // `light_volume_submit`: one group each, at its marker.
    for (volume, texture) in frame.preview.ce.light_volumes.iter().zip(&ce.textures.light_volumes) {
        let Some(texture) = *texture else {
            continue;
        };
        let (outputs, active) = switched(volume.object);
        let quads = volume.quads(&outputs, &active, bones, view);
        let draw = CeGroupDraw::Plain { quads, texture, additive: true };
        groups.push(CeGroup { z_sort: ce_z_sort(view, volume.marker(bones), false), draw });
    }
    groups
}

/// `rasterizer_transparent_geometry_group_draw` for one of Halo CE's groups,
/// tested against the model's depth and not written.
#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) unsafe fn draw_ce_group(
    gl: &glow::Context,
    effect_gl: &mut Option<Option<CeEffectGl>>,
    main: glow::NativeProgram,
    flares: &SpriteGl,
    ce: &mut ce::slot::CeSlot,
    camera: &ModelGpuCamera,
    group: CeGroup,
) {
    unsafe {
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        gl.disable(glow::CULL_FACE);
        gl.color_mask(true, true, true, false);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.active_texture(glow::TEXTURE0);
        match group.draw {
            CeGroupDraw::Sprites(sprites) => {
                if let Some(effect_gl) = effect_gl.get_or_insert_with(|| ce_effect_gl(gl, main)).as_ref() {
                    draw_ce_sprites(gl, effect_gl, ce, camera, sprites);
                }
            }
            CeGroupDraw::Plain { quads, texture, additive } => {
                gl.use_program(Some(flares.program));
                gl.bind_vertex_array(Some(flares.vertex_array));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(flares.buffer));
                gl.uniform_1_i32(flares.map.as_ref(), 0);
                // Plain color times map.
                gl.uniform_1_f32(flares.highlight.as_ref(), -1.0);
                gl.uniform_2_f32(flares.tint.as_ref(), 0.0, 0.0);
                gl.blend_func(glow::SRC_ALPHA, if additive { glow::ONE } else { glow::ONE_MINUS_SRC_ALPHA });
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for quad in quads {
                    let vertices = quad.corners.map(|[x, y, z, u, v]| {
                        let [a, b, c, d] = ce_flare_clip(camera, [x, y, z]);
                        [a, b, c, d, u, v]
                    });
                    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
                    let [red, green, blue, alpha] = quad.color;
                    gl.uniform_4_f32(flares.color.as_ref(), red, green, blue, alpha);
                    gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                }
            }
        }
    }
}

/// `render_particles`' draw: each sprite through the `effect` shader.
unsafe fn draw_ce_sprites(
    gl: &glow::Context,
    effect_gl: &CeEffectGl,
    ce: &mut ce::slot::CeSlot,
    camera: &ModelGpuCamera,
    sprites: Vec<CeSprite>,
) {
    unsafe {
        gl.use_program(Some(effect_gl.program));
        gl.bind_vertex_array(Some(effect_gl.vertex_array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(effect_gl.buffer));
        gl.active_texture(glow::TEXTURE0);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(false);
        gl.enable(glow::BLEND);
        // `C_ZSPRITE_PARAMS`' radius moves a pixel toward the eye; these are
        // how far clip z and w go for each unit of that.
        let zsprite_z = camera.scale * camera.depth_scale * camera.depth_map[0];
        let zsprite_w = camera.scale * camera.perspective;
        for (texture, function, nonlinear, colors, corners, secondary) in sprites {
            // A second map that would not upload draws as none, not as black.
            let secondary = secondary.as_ref().and_then(|draw| {
                let key = Arc::as_ptr(&draw.map) as usize;
                if !ce.textures.secondary.contains_key(&key)
                    && let Some(made) = upload_h2_image(gl, &draw.map.image)
                {
                    ce.textures.secondary.insert(key, made);
                }
                ce.textures.secondary.get(&key).map(|map| (draw, *map))
            });
            match secondary {
                Some((draw, map)) => {
                    gl.active_texture(glow::TEXTURE1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(map));
                    gl.active_texture(glow::TEXTURE0);
                    gl.uniform_1_i32(effect_gl.mode.as_ref(), draw.mode);
                    gl.uniform_4_f32_slice(effect_gl.rows.as_ref(), draw.rows.as_flattened());
                    gl.uniform_3_f32(effect_gl.zsprite.as_ref(), draw.radius, zsprite_z, zsprite_w);
                }
                None => gl.uniform_1_i32(effect_gl.mode.as_ref(), 0),
            }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let blend = function.gl();
            gl.blend_equation(gl_blend_equation(blend.equation));
            gl.blend_func(gl_blend_factor(blend.source), gl_blend_factor(blend.destination));
            gl.uniform_1_i32(effect_gl.blend.as_ref(), function.shader_number());
            gl.uniform_1_i32(effect_gl.nonlinear.as_ref(), i32::from(nonlinear));
            // The zsprite stream's corners, in `build_sprite`'s vertex order
            // (left bottom, right bottom, right top, left top): 00, 01, 11, 10.
            let units = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]];
            let mut vertices = [[0.0f32; CE_EFFECT_FLOATS]; 4];
            for ((vertex, ([x, y, z, u, v], unit)), [red, green, blue, alpha]) in
                vertices.iter_mut().zip(corners.into_iter().zip(units)).zip(colors)
            {
                let [a, b, c, d] = ce_flare_clip(camera, [x, y, z]);
                *vertex = [a, b, c, d, u, v, unit[0], unit[1], red, green, blue, alpha];
            }
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, slice_bytes(&vertices), glow::STREAM_DRAW);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }
}

/// Whether the preview has any of Halo CE's effects, particle systems, glows,
/// contrails, lightning, antennas or light volumes.
pub(in crate::app::model_preview) fn ce_has_groups(preview: &RenderModelPreview) -> bool {
    !(preview.ce.effects.is_empty()
        && preview.ce.particle_systems.is_empty()
        && preview.ce.glows.is_empty()
        && preview.ce.contrails.is_empty()
        && preview.ce.lightning.is_empty()
        && preview.ce.antennas.is_empty()
        && preview.ce.light_volumes.is_empty())
}

/// Whether Halo CE's effects sort in among the model's transparent parts, as
/// the game draws them, rather than after: a CE model's batches know where
/// they sort, and the transparents draw only when shading does.
pub(in crate::app::model_preview) fn ce_sorts_transparents(frame: &ModelGpuFrame) -> bool {
    frame.render_mode.draws_shading() && frame.preview.batches.iter().any(|batch| batch.sort_position.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An eye at the origin looking along +X.
    fn looking_along_x() -> common::view::ViewFrame {
        common::view::ViewFrame { eye: [0.0; 3], forward: [1.0, 0.0, 0.0], right: [0.0, 1.0, 0.0], up: [0.0, 0.0, 1.0] }
    }

    /// `z_sort` puts the farther first, and `sort bias` a quarter unit nearer.
    #[test]
    fn ce_z_sort_runs_far_to_near_and_sort_bias_draws_later() {
        let view = looking_along_x();
        let far = ce_z_sort(&view, [10.0, 3.0, 0.0], false);
        let near = ce_z_sort(&view, [2.0, -5.0, 1.0], false);
        assert!(far < near, "farther sorts first");
        assert_eq!(ce_z_sort(&view, [2.0, 0.0, 0.0], true), near + CE_SORT_BIAS);
    }

    /// Sprites of one key make one group, at the mean of their corners, or
    /// for a bolt at the middle of their bounds; the groups keep the order
    /// they first appeared in.
    #[test]
    fn ce_sprites_group_by_key_at_their_centroid() {
        let view = looking_along_x();
        let texture = glow::NativeTexture(std::num::NonZeroU32::new(1).unwrap());
        let at = |x: f32| (texture, ce::blend::FramebufferBlend::AlphaBlend, false, [[1.0; 4]; 4], [[x, 0.0, 0.0, 0.0, 0.0]; 4], None);
        let mut sprites = CeSpriteGroups::default();
        sprites.push((CE_PARTICLE, 7, 0), false, false, at(2.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(1.0));
        sprites.push((CE_PARTICLE, 7, 0), false, false, at(6.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(9.0));
        sprites.push((CE_LIGHTNING, 0, 0), false, true, at(9.0));
        let groups: Vec<CeGroup> = sprites.sorted_by(&view).collect();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].z_sort, -4.0, "the particles' mean");
        assert_eq!(groups[1].z_sort, -5.0, "the bolt's middle, not its mean");
    }
}
