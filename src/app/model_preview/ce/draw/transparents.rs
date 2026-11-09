//! Halo CE's sorted transparents: the model's transparent parts and its
//! effects' sprite groups, far to near, as `render_transparent_geometry` has them.

use super::super::super::common::sprite_gl::link_sprite_gl;
use super::super::super::common::view::camera_view_frame;
use super::super::super::dx9::passes::Dx9Layer;
use super::super::super::{ModelGpuFrame, RenderCore, common};
use super::effect_gl::CeEffectGl;
use super::sprites::{CeGroup, ce_groups, ce_has_groups, ce_z_sort, draw_ce_group};
use crate::app::model_preview::ce;
use eframe::glow::{self, HasContext as _};

/// Halo CE's transparents as `rasterizer_transparent_geometry_draw` draws
/// them: the model's transparent parts and the effects' groups in one sort
/// by `z_sort`, far to near, each part from its centroid on its node.
pub(in crate::app::model_preview) unsafe fn draw_ce_transparents(
    core: &mut RenderCore,
    gl: &glow::Context,
    frame: &ModelGpuFrame,
    slot: usize,
    ce: &mut ce::slot::CeSlot,
    effect_gl: &mut Option<Option<CeEffectGl>>,
) {
    enum Transparent {
        Part(usize),
        Group(CeGroup),
    }
    unsafe {
        let view = camera_view_frame(&frame.camera);
        let bones = frame.bones.as_ref().map(|bones| bones.as_slice());
        let mut groups = Vec::new();
        if core.draws_object_sprites(frame) && ce_has_groups(&frame.preview) {
            core.sprite_gl
                .get_or_insert_with(|| link_sprite_gl(gl, core.program));
            core.slots[slot].sync_sprite_textures(gl, frame);
            groups = ce_groups(gl, ce, frame, bones, &view);
        }
        let points = &core.slots[slot].sort_points;
        let parts = frame.visible_batches.iter().map(|&index| {
            let place = frame
                .preview
                .batches
                .get(index)
                .and_then(|batch| batch.sort_position);
            let point = match place {
                Some((point, node)) => common::view::posed_on(node, point, [0.0; 3], bones).0,
                None => points.get(index).copied().unwrap_or(frame.camera.center),
            };
            (ce_z_sort(&view, point, false), Transparent::Part(index))
        });
        let groups = groups
            .into_iter()
            .map(|group| (group.z_sort, Transparent::Group(group)));
        let mut sorted: Vec<_> = parts.chain(groups).collect();
        sorted.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        let vertex_array = core.slots[slot].vertex_array;
        for (_, transparent) in sorted {
            match transparent {
                Transparent::Part(index) => {
                    // What the passes take as theirs: the model's vertices
                    // and the frame's culling.
                    gl.bind_vertex_array(Some(vertex_array));
                    if frame.show_backfaces {
                        gl.disable(glow::CULL_FACE);
                    } else {
                        gl.enable(glow::CULL_FACE);
                    }
                    core.draw_h2_passes(gl, frame, slot, &[index], Dx9Layer::Transparent, None);
                }
                Transparent::Group(group) => {
                    let Some(flares) = core.sprite_gl.as_ref().and_then(Option::as_ref) else {
                        continue;
                    };
                    draw_ce_group(
                        gl,
                        effect_gl,
                        core.program,
                        flares,
                        ce,
                        &frame.camera,
                        group,
                    );
                }
            }
        }
        // As the passes leave it.
        gl.use_program(Some(core.program));
        gl.bind_vertex_array(Some(vertex_array));
        gl.disable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LESS);
        gl.depth_mask(true);
        gl.color_mask(true, true, true, true);
        gl.active_texture(glow::TEXTURE0);
        if frame.show_backfaces {
            gl.disable(glow::CULL_FACE);
        } else {
            gl.enable(glow::CULL_FACE);
        }
    }
}
