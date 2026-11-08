//! What any game's held or carried object needs: a weapon row, an arm that
//! reaches for a marker, and folding a second model into a preview.

use blam_tags::math::Matrix4;

use super::*;
use crate::app::model_preview::common::math::{axis_of, cross, dot, matrix_of, minus, place_of, plus, scaled, unit_by_reciprocal};
use super::common::geometry::{Overflow, VertexBase};
use super::common::marker::{MarkerFrame, attach_point, attach_vector};

/// One row of a weapon dropdown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WeaponOption {
    pub(crate) path: String,
    pub(crate) name: String,
    /// The class its row sits under.
    pub(crate) class: String,
}

/// `inverse_kinematics_adjust_matrices` keeps the arm this much short of straight.
pub(super) const LONGEST_REACH: f32 = 0.98;

/// `inverse_kinematics_adjust_matrices`: bend at the elbow, on the side it
/// already bends, so the hand reaches `desired`.
pub(super) fn reach_with_arm(desired: Matrix4, shoulder: &mut Matrix4, elbow: &mut Matrix4, hand: &mut Matrix4) {
    let start = place_of(shoulder);
    let joint = place_of(elbow);
    let upper = dot(minus(joint, start), minus(joint, start)).sqrt();
    let fore = dot(minus(place_of(hand), joint), minus(place_of(hand), joint)).sqrt();
    let to_goal = minus(place_of(&desired), start);
    let mut reach = dot(to_goal, to_goal).sqrt();
    if reach < 0.0001 {
        return;
    }
    let toward = scaled(to_goal, 1.0 / reach);
    let side = unit_by_reciprocal(cross(toward, minus(joint, start)));
    let bend = cross(side, toward);
    let mut goal = desired;
    let longest = (upper + fore) * LONGEST_REACH;
    // An arm with no length has nothing to bend.
    if longest < 0.0001 {
        return;
    }
    if longest < reach {
        reach = longest;
        let place = plus(start, scaled(toward, reach));
        (0..3).for_each(|row| goal.m[row][3] = place[row]);
    }
    let along = (reach * reach + upper * upper - fore * fore) / (reach * 2.0);
    let out = (upper * upper - along * along).max(0.0).sqrt();
    // A bone's first axis runs down it, and its second stays as near as it can.
    let aimed = |x: [f32; 3], up: [f32; 3], place: [f32; 3]| {
        let z = unit_by_reciprocal(cross(x, up));
        matrix_of([x, cross(z, x), z], place)
    };
    let upper_axis = unit_by_reciprocal(plus(scaled(toward, along), scaled(bend, out)));
    let fore_axis = unit_by_reciprocal(plus(scaled(toward, reach - along), scaled(bend, -out)));
    let bent = plus(start, scaled(upper_axis, upper));
    *elbow = aimed(fore_axis, axis_of(elbow, 1), bent);
    *shoulder = aimed(upper_axis, axis_of(shoulder, 1), start);
    *hand = goal;
}

/// Add `other` to `preview`, moved onto `parent` and owned by object `owner`.
pub(super) fn merge_attached(
    preview: &mut RenderModelPreview,
    mut other: RenderModelPreview,
    parent: MarkerFrame,
    child: MarkerFrame,
    owner: u8,
    name: &str,
) {
    let Some(vertex_base) = VertexBase::of(preview.vertices.len(), other.vertices.len(), Overflow::RefusesAppend) else {
        return;
    };
    let index_base = preview.indices.len() as u32;
    let material_base = preview.materials.len() as u16;
    let bound = [f32::from(parent.node), 0.0, 0.0, 0.0];
    for vertex in &mut other.vertices {
        vertex.position = attach_point(&parent, &child, vertex.position);
        vertex.normal = attach_vector(&parent, &child, vertex.normal);
        vertex.tangent = attach_vector(&parent, &child, vertex.tangent);
        vertex.binormal = attach_vector(&parent, &child, vertex.binormal);
        vertex.node_indices = bound;
        vertex.node_weights = [1.0, 0.0, 0.0, 0.0];
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(vertex.position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(vertex.position[axis]);
        }
    }
    preview.vertices.append(&mut other.vertices);
    preview.indices.extend(other.indices.iter().map(|&index| vertex_base.offset(index)));
    for material in &mut other.materials {
        material.object = owner;
    }
    preview.materials.append(&mut other.materials);
    let label = |region: &str| format!("{name}: {region}");
    let node = usize::try_from(parent.node).unwrap_or(0);
    for mut batch in other.batches {
        batch.index_start += index_base;
        batch.material_index += material_base;
        batch.region_name = label(&batch.region_name);
        // Moved and bound as its vertices are.
        batch.sort_position = batch.sort_position.map(|(point, _)| (attach_point(&parent, &child, point), node));
        preview.batches.push(batch);
    }
    for mut region in other.regions {
        region.name = label(&region.name);
        preview.regions.push(region);
    }
}
