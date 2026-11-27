//! A collection's instances built into a model: each a quad of its sprite,
//! standing on its bottom edge, as the detail object vertex shaders build it
//! from their corner table (`c15` to `c18`: across −½ to ½, up 0 to 1) and
//! their type's size (`c19 + type`: the sprite's texels times `size`).

use super::super::super::common::geometry::{Overflow, VertexBase, expand_preview_bounds_local};
use super::super::super::{
    RenderModelPreview, RenderModelPreviewBatch, RenderModelPreviewMaterial,
    RenderModelPreviewRegion, RenderModelPreviewVertex,
};
use super::super::bake::{PERMUTATION, type_name};
use super::HaloCeCollection;

/// The corners, across then up, and the triangles of each face.
const CORNERS: [[f32; 2]; 4] = [[-0.5, 0.0], [0.5, 0.0], [0.5, 1.0], [-0.5, 1.0]];
const FACES: [[u32; 6]; 2] = [[0, 1, 2, 0, 2, 3], [0, 2, 1, 0, 3, 2]];

/// `collection`'s instances as a model, drawn with `material` (its plate): a
/// region and batch per type. Each quad draws both faces, as it turns to face
/// the eye in game.
pub(in crate::app::model_preview) fn bake_halo_ce(
    collection: &HaloCeCollection,
    material: RenderModelPreviewMaterial,
) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        materials: vec![material],
        ..Default::default()
    };
    for (type_index, kind) in collection.types.iter().enumerate() {
        let name = type_name(type_index, &kind.name);
        preview.regions.push(RenderModelPreviewRegion {
            name: name.clone(),
            permutations: vec![PERMUTATION.to_owned()],
        });
        let Some(sprites) = collection
            .plate
            .sequences
            .get(kind.sequence)
            .filter(|sprites| !sprites.is_empty())
        else {
            continue;
        };
        let index_start = preview.indices.len() as u32;
        for instance in collection
            .instances
            .iter()
            .filter(|instance| instance.type_index == type_index)
        {
            let sprite = sprites[instance.frame % sprites.len()];
            let [left, right, top, bottom] = sprite.bounds;
            let texels = collection
                .plate
                .images
                .get(sprite.image)
                .copied()
                .unwrap_or([0.0; 2]);
            let size = [
                (right - left) * texels[0] * kind.size,
                (bottom - top) * texels[1] * kind.size,
            ];
            let Some(base) = VertexBase::of(
                preview.vertices.len(),
                CORNERS.len(),
                Overflow::RefusesAppend,
            ) else {
                break;
            };
            for [across, up] in CORNERS {
                let [x, y, z] = instance.position;
                let position = [x + across * size[0], y, z + up * size[1]];
                expand_preview_bounds_local(
                    &mut preview.bounds_min,
                    &mut preview.bounds_max,
                    position,
                );
                preview.vertices.push(RenderModelPreviewVertex {
                    position,
                    normal: [0.0, -1.0, 0.0],
                    texcoord: [
                        if across < 0.0 { left } else { right },
                        if up > 0.0 { top } else { bottom },
                    ],
                    ..Default::default()
                });
            }
            for face in FACES {
                preview
                    .indices
                    .extend(face.map(|corner| base.offset(corner)));
            }
        }
        let index_count = preview.indices.len() as u32 - index_start;
        if index_count > 0 {
            preview.batches.push(RenderModelPreviewBatch {
                region_name: name,
                permutation_name: PERMUTATION.to_owned(),
                material_index: 0,
                index_start,
                index_count,
                ..Default::default()
            });
        }
    }
    preview
}
