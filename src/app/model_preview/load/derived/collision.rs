//! Collision models as preview geometry: the JMS scene, posed by the bind pose, and its error points.

use super::super::super::{
    MAX_PREVIEW_BONES, ModelErrorPoint, ModelErrorPrimitive, ModelErrorShape, ModelPreviewLayer,
    RenderModelPreview, RenderModelPreviewBatch, RenderModelPreviewMaterial,
    RenderModelPreviewVertex, TagFile, collision_jms_for_game,
};
use super::super::errors::append_model_errors;
use super::{
    COLLISION_COLOR, JMS_SCALE, Overflow, VertexBase, empty_preview,
    ensure_preview_region_permutation, face_normal, finish_preview, point, rotate,
};
use crate::app::model_preview::common::geometry::expand_preview_bounds_local;
use blam_tags::JmsFile;
use blam_tags::jms_split::MaterialLabel;

/// Preserve a collision JMS's actual region/permutation cells and bone
/// influences. The generic derived-mesh path intentionally flattens both,
/// which is right for BSP helper layers but made model collision static and
/// immune to the variant controls.
pub(super) fn append_collision_jms(
    preview: &mut RenderModelPreview,
    jms: &JmsFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) {
    let node_map = skeleton.map(|target| {
        jms.nodes
            .iter()
            .map(|source| target.iter().position(|node| node.name == source.name))
            .collect::<Vec<_>>()
    });
    let mut cells: Vec<(String, String, Vec<RenderModelPreviewVertex>)> = Vec::new();

    for triangle in &jms.triangles {
        let label = jms
            .materials
            .get(triangle.material.max(0) as usize)
            .map(|material| MaterialLabel::parse(&material.material_name));
        let (region_name, permutation_name) = if let Some(label) = label
            .as_ref()
            .filter(|label| label.region != "default" || label.permutation != "default")
        {
            (label.region.clone(), label.permutation.clone())
        } else if !jms.regions.is_empty() {
            (
                jms.regions
                    .get(triangle.region.max(0) as usize)
                    .cloned()
                    .unwrap_or_else(|| "default".to_owned()),
                "default".to_owned(),
            )
        } else {
            let label = label.unwrap_or_else(|| MaterialLabel::parse("default default"));
            (label.region, label.permutation)
        };
        let corners = triangle.v.map(|index| jms.vertices.get(index as usize));
        let [Some(a), Some(b), Some(c)] = corners else {
            continue;
        };
        let source_vertices = [a, b, c];
        let positions = source_vertices.map(|vertex| {
            let p = point(&vertex.position);
            [p[0] / JMS_SCALE, p[1] / JMS_SCALE, p[2] / JMS_SCALE]
        });
        let normal = face_normal(positions[0], positions[1], positions[2]);
        let cell = if let Some(index) = cells.iter().position(|(region, permutation, _)| {
            region == &region_name && permutation == &permutation_name
        }) {
            &mut cells[index].2
        } else {
            cells.push((region_name.clone(), permutation_name.clone(), Vec::new()));
            &mut cells.last_mut().expect("just pushed collision cell").2
        };
        for (source, position) in source_vertices.into_iter().zip(positions) {
            let mut indices = [0.0; 4];
            let mut weights = [0.0; 4];
            let mut count = 0;
            for &(source_index, weight) in &source.node_sets {
                if count == 4 || source_index < 0 || weight <= 0.0 {
                    continue;
                }
                let target_index = if let Some(map) = &node_map {
                    let Some(target_index) = map.get(source_index as usize).copied().flatten()
                    else {
                        continue;
                    };
                    target_index
                } else {
                    source_index as usize
                };
                if target_index >= MAX_PREVIEW_BONES {
                    continue;
                }
                indices[count] = target_index as f32;
                weights[count] = weight;
                count += 1;
            }
            let weight_sum: f32 = weights.iter().sum();
            if weight_sum > f32::EPSILON {
                for weight in &mut weights {
                    *weight /= weight_sum;
                }
            }
            expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
            cell.push(RenderModelPreviewVertex {
                position,
                normal,
                node_indices: indices,
                node_weights: weights,
                ..Default::default()
            });
        }
    }

    for (region_name, permutation_name, vertices) in cells {
        if vertices.is_empty() {
            continue;
        }
        let Some(vertex_base) = VertexBase::of(
            preview.vertices.len(),
            vertices.len(),
            Overflow::RefusesAppend,
        ) else {
            continue;
        };
        let index_start = preview.indices.len() as u32;
        preview
            .indices
            .extend((0..vertices.len() as u32).map(|offset| vertex_base.offset(offset)));
        preview.vertices.extend(vertices);
        let material_index = preview.materials.len().min(u16::MAX as usize) as u16;
        preview
            .materials
            .push(RenderModelPreviewMaterial::default());
        let index_count = preview.indices.len() as u32 - index_start;
        preview.batches.push(RenderModelPreviewBatch {
            region_name: region_name.clone(),
            permutation_name: permutation_name.clone(),
            material_index,
            index_start,
            index_count,
            flat_color: Some(COLLISION_COLOR),
            layer: ModelPreviewLayer::Collision,
            sort_position: None,
        });
        ensure_preview_region_permutation(preview, &region_name, &permutation_name);
    }
}

/// A collision tag (`coll`, or H1 `model_collision_geometry`) as solid flat
/// geometry, posed by `skeleton` when the owning model supplies one.
pub(in crate::app::model_preview) fn build_collision_preview(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> Result<RenderModelPreview, String> {
    let jms = collision_jms_for_game(tag, skeleton).map_err(|error| error.to_string())?;
    let mut preview = empty_preview();
    append_collision_jms(&mut preview, &jms, skeleton);
    let error_start = preview.errors.len();
    append_model_errors(tag, &mut preview, ModelPreviewLayer::Collision);
    if let Some(skeleton) = skeleton {
        pose_collision_errors(&mut preview.errors[error_start..], &jms, skeleton);
    }
    finish_preview(preview, "collision model")
}

/// Collision BSP vertices are converted from bone-local coordinates into the
/// render skeleton's bind pose by `JmsFile::from_collision_model_with_skeleton`.
/// Tool-report points use the same local coordinates and influences, so apply
/// the identical bind transform before they share the preview with that mesh.
pub(super) fn pose_collision_errors(
    errors: &mut [ModelErrorPrimitive],
    collision: &JmsFile,
    skeleton: &[blam_tags::JmsNode],
) {
    let node_map = collision
        .nodes
        .iter()
        .map(|source| skeleton.iter().position(|node| node.name == source.name))
        .collect::<Vec<_>>();

    let pose_point = |point: &mut ModelErrorPoint| {
        let posed = pose_collision_value(point.position, point, &node_map, skeleton, true);
        if posed.total_weight > f32::EPSILON {
            point.position = posed.value.map(|component| component / posed.total_weight);
            point.node_indices = posed.node_indices;
            point.node_weights = posed.node_weights.map(|weight| weight / posed.total_weight);
        }
    };
    let pose_direction = |point: &ModelErrorPoint, direction: &mut [f32; 3]| {
        let posed = pose_collision_value(*direction, point, &node_map, skeleton, false);
        let length = posed
            .value
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if posed.total_weight > f32::EPSILON && length > f32::EPSILON {
            *direction = posed.value.map(|component| component / length);
        }
    };

    for error in errors {
        match &mut error.shape {
            ModelErrorShape::Point(point) => pose_point(point),
            ModelErrorShape::Vector { point, normal, .. } => {
                pose_direction(point, normal);
                pose_point(point);
            }
            ModelErrorShape::Polyline(points) | ModelErrorShape::Face(points) => {
                for point in points {
                    pose_point(point);
                }
            }
        }
    }
}

struct PosedCollisionValue {
    value: [f32; 3],
    total_weight: f32,
    node_indices: [i16; 4],
    node_weights: [f32; 4],
}

fn pose_collision_value(
    value: [f32; 3],
    influences: &ModelErrorPoint,
    node_map: &[Option<usize>],
    skeleton: &[blam_tags::JmsNode],
    include_translation: bool,
) -> PosedCollisionValue {
    let mut posed = PosedCollisionValue {
        value: [0.0; 3],
        total_weight: 0.0,
        node_indices: [-1; 4],
        node_weights: [0.0; 4],
    };
    for slot in 0..4 {
        let source_index = influences.node_indices[slot];
        let weight = influences.node_weights[slot];
        if source_index < 0 || weight <= 0.0 {
            continue;
        }
        let Some(target_index) = node_map.get(source_index as usize).copied().flatten() else {
            continue;
        };
        let Some(node) = skeleton.get(target_index) else {
            continue;
        };
        let mut transformed = rotate(&node.rotation, value);
        if include_translation {
            transformed[0] += node.translation.x / JMS_SCALE;
            transformed[1] += node.translation.y / JMS_SCALE;
            transformed[2] += node.translation.z / JMS_SCALE;
        }
        for axis in 0..3 {
            posed.value[axis] += weight * transformed[axis];
        }
        posed.node_indices[slot] = target_index as i16;
        posed.node_weights[slot] = weight;
        posed.total_weight += weight;
    }
    posed
}
