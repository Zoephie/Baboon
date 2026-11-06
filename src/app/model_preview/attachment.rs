//! What any game's held or carried object needs: rigid transforms, an arm that
//! reaches for a marker, and folding a second model into a preview.

use blam_tags::math::Matrix4;

use super::*;
use crate::app::model_preview::common::math::{cross, dot, scaled, unit_by_reciprocal};

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

pub(super) const IDENTITY: Matrix4 = Matrix4 {
    m: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};



/// `perpendicular3d`: a vector square to `a`, from its two largest parts.
pub(super) fn perpendicular(a: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = a.map(f32::abs);
    if x > y || x > z {
        if y > z { [a[1], -a[0], 0.0] } else { [-a[2], 0.0, a[0]] }
    } else {
        [0.0, a[2], -a[1]]
    }
}


pub(super) fn added(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] + b[axis])
}

pub(super) fn apart(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}


pub(super) fn axis_of(matrix: &Matrix4, axis: usize) -> [f32; 3] {
    std::array::from_fn(|row| matrix.m[row][axis])
}

pub(super) fn place_of(matrix: &Matrix4) -> [f32; 3] {
    axis_of(matrix, 3)
}

pub(super) fn matrix_of(axes: [[f32; 3]; 3], position: [f32; 3]) -> Matrix4 {
    let mut matrix = IDENTITY;
    for row in 0..3 {
        for axis in 0..3 {
            matrix.m[row][axis] = axes[axis][row];
        }
        matrix.m[row][3] = position[row];
    }
    matrix
}

pub(super) fn rigid_inverse(matrix: &Matrix4) -> Matrix4 {
    let axes = [axis_of(matrix, 0), axis_of(matrix, 1), axis_of(matrix, 2)];
    let place = place_of(matrix);
    let mut inverse = IDENTITY;
    for row in 0..3 {
        inverse.m[row][0] = axes[row][0];
        inverse.m[row][1] = axes[row][1];
        inverse.m[row][2] = axes[row][2];
        inverse.m[row][3] = -dot(axes[row], place);
    }
    inverse
}

/// `inverse_kinematics_adjust_matrices`: bend at the elbow, on the side it
/// already bends, so the hand reaches `desired`.
pub(super) fn reach_with_arm(desired: Matrix4, shoulder: &mut Matrix4, elbow: &mut Matrix4, hand: &mut Matrix4) {
    let start = place_of(shoulder);
    let joint = place_of(elbow);
    let upper = dot(apart(joint, start), apart(joint, start)).sqrt();
    let fore = dot(apart(place_of(hand), joint), apart(place_of(hand), joint)).sqrt();
    let to_goal = apart(place_of(&desired), start);
    let mut reach = dot(to_goal, to_goal).sqrt();
    if reach < 0.0001 {
        return;
    }
    let toward = scaled(to_goal, 1.0 / reach);
    let side = unit_by_reciprocal(cross(toward, apart(joint, start)));
    let bend = cross(side, toward);
    let mut goal = desired;
    let longest = (upper + fore) * LONGEST_REACH;
    // An arm with no length has nothing to bend.
    if longest < 0.0001 {
        return;
    }
    if longest < reach {
        reach = longest;
        let place = added(start, scaled(toward, reach));
        (0..3).for_each(|row| goal.m[row][3] = place[row]);
    }
    let along = (reach * reach + upper * upper - fore * fore) / (reach * 2.0);
    let out = (upper * upper - along * along).max(0.0).sqrt();
    // A bone's first axis runs down it, and its second stays as near as it can.
    let aimed = |x: [f32; 3], up: [f32; 3], place: [f32; 3]| {
        let z = unit_by_reciprocal(cross(x, up));
        matrix_of([x, cross(z, x), z], place)
    };
    let upper_axis = unit_by_reciprocal(added(scaled(toward, along), scaled(bend, out)));
    let fore_axis = unit_by_reciprocal(added(scaled(toward, reach - along), scaled(bend, -out)));
    let bent = added(start, scaled(upper_axis, upper));
    *elbow = aimed(fore_axis, axis_of(elbow, 1), bent);
    *shoulder = aimed(upper_axis, axis_of(shoulder, 1), start);
    *hand = goal;
}

/// A marker as a rigid transform: its three axes, then its position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MarkerFrame {
    pub(super) axes: [[f32; 3]; 3],
    pub(super) position: [f32; 3],
    pub(super) node: i16,
}

impl MarkerFrame {
    pub(super) const ORIGIN: Self = Self {
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        position: [0.0; 3],
        node: 0,
    };

    /// `object_get_marker_by_name`: no name, or none found, is the origin.
    pub(super) fn find(preview: &RenderModelPreview, name: &str) -> Self {
        preview
            .markers
            .iter()
            .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name))
            .map_or(Self::ORIGIN, |marker| Self {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index.max(0),
            })
    }

    /// A direction in this frame's axes, in the space the frame sits in.
    fn rotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            (0..3).map(|index| self.axes[index][axis] * value[index]).sum()
        })
    }

    /// `rotate`'s inverse: axes are unit and square to each other.
    fn unrotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|index| (0..3).map(|axis| self.axes[index][axis] * value[axis]).sum())
    }
}

/// Where a point of the child goes when its `child` marker meets `parent`.
pub(super) fn attach_point(parent: &MarkerFrame, child: &MarkerFrame, point: [f32; 3]) -> [f32; 3] {
    let local = child.unrotate(std::array::from_fn(|axis| point[axis] - child.position[axis]));
    let turned = parent.rotate(local);
    std::array::from_fn(|axis| turned[axis] + parent.position[axis])
}

pub(super) fn attach_vector(parent: &MarkerFrame, child: &MarkerFrame, vector: [f32; 3]) -> [f32; 3] {
    parent.rotate(child.unrotate(vector))
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
    let vertex_base = preview.vertices.len() as u32;
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
    preview.indices.extend(other.indices.iter().map(|index| index + vertex_base));
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
