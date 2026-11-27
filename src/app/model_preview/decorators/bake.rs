//! Every placement's mesh copied into one model: what Solid and Textured
//! draw, which have no instancing (their program falls back as far as GLSL
//! 1.20). Game Shaders draws the placements instanced instead.

use super::super::common::geometry::{Overflow, VertexBase, expand_preview_bounds_local};
use super::super::{
    RenderModelPreview, RenderModelPreviewBatch, RenderModelPreviewMaterial,
    RenderModelPreviewRegion, RenderModelPreviewVertex,
};
use super::DecoratorSet;
use super::layout::rotate;

/// Each type's one permutation: a type is a region of its own, so every type
/// shows at once, as a model shows a permutation of each region.
pub(crate) const PERMUTATION: &str = "default";

/// `set`'s placements as a model: a region and batch per type, drawn with the set's
/// texture (`material`). A two-sided set also draws each triangle's back,
/// facing the other way, since the preview culls by its view settings
/// rather than by material.
pub(in crate::app::model_preview) fn bake_placements(
    set: &DecoratorSet,
    material: RenderModelPreviewMaterial,
) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        materials: vec![material],
        ..Default::default()
    };
    for (type_index, (kind, mesh)) in set.types.iter().zip(&set.meshes).enumerate() {
        let name = type_name(type_index, &kind.mesh_name);
        preview.regions.push(RenderModelPreviewRegion {
            name: name.clone(),
            permutations: vec![PERMUTATION.to_owned()],
        });
        let index_start = preview.indices.len() as u32;
        for placement in set
            .placements
            .iter()
            .filter(|placement| placement.type_index == type_index)
        {
            let Some(base) = VertexBase::of(
                preview.vertices.len(),
                mesh.positions.len() * 2,
                Overflow::RefusesAppend,
            ) else {
                break;
            };
            let sides: &[f32] = if set.two_sided { &[1.0, -1.0] } else { &[1.0] };
            for (side, facing) in sides.iter().enumerate() {
                let first = (side * mesh.positions.len()) as u32;
                for ((position, normal), texcoord) in mesh
                    .positions
                    .iter()
                    .zip(&mesh.normals)
                    .zip(&mesh.texcoords)
                {
                    let turned = rotate(
                        placement.rotation,
                        position.map(|value| value * placement.scale),
                    );
                    let position: [f32; 3] =
                        std::array::from_fn(|axis| placement.position[axis] + turned[axis]);
                    expand_preview_bounds_local(
                        &mut preview.bounds_min,
                        &mut preview.bounds_max,
                        position,
                    );
                    preview.vertices.push(RenderModelPreviewVertex {
                        position,
                        normal: rotate(placement.rotation, *normal).map(|value| value * facing),
                        texcoord: *texcoord,
                        ..Default::default()
                    });
                }
                for triangle in mesh.indices.chunks_exact(3) {
                    let corners = triangle.iter().map(|&index| base.offset(first + index));
                    if *facing > 0.0 {
                        preview.indices.extend(corners);
                    } else {
                        preview.indices.extend(corners.rev());
                    }
                }
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

/// A type as the sidebar lists it: its mesh's name, numbered, as two types
/// may draw the same mesh.
pub(crate) fn type_name(type_index: usize, mesh_name: &str) -> String {
    if mesh_name.is_empty() {
        format!("{type_index}")
    } else {
        format!("{type_index}: {mesh_name}")
    }
}
