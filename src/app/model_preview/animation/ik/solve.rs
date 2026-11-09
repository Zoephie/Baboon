//! What every game's IK poses with: the markers a target names, the
//! two-bone solve that brings an effector to one, and the solvers a clip
//! carries, which pose after its sampled frames.

use std::sync::Arc;

use blam_tags::TagFile;
use blam_tags::animation::AnimationGroup;
use blam_tags::math::{Matrix4, RealPoint3d};

use crate::app::model_preview::RenderModelPreviewNode;
use crate::app::model_preview::common::math::{axis_of, matrix_of, place_of, rigid_inverse};
use crate::app::model_preview::common::math::{
    cross, dot, minus, plus, scaled, unit_by_reciprocal,
};

/// What a clip solves after its sampled pose and the held weapon's IK, read
/// from its graph by a game's reader: Halo 4's IK sets, Reach's IK chains.
pub(crate) trait PoseSolver: std::fmt::Debug + Send + Sync + std::any::Any {
    /// Solve on the posed `worlds`, between frames `a` and `b` at `blend`.
    fn solve(
        &self,
        markers: &[IkMarker],
        nodes: &[RenderModelPreviewNode],
        inverse_binds: &[Matrix4],
        worlds: &mut [Matrix4],
        frames: (usize, usize, f32),
    );

    /// The solver as a pose held on frame `at` alone reads it.
    #[cfg(test)]
    fn held_at(&self, at: usize) -> Arc<dyn PoseSolver>;

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
}

/// A reader of a [`PoseSolver`] from the clip `group` of `graphs[0]` (its
/// parents after it): none where the clip carries nothing it solves.
pub(in crate::app::model_preview) type PoseSolverReader =
    fn(&[&TagFile], &AnimationGroup<'_>) -> Option<Arc<dyn PoseSolver>>;

/// Whose marker a target is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IkHolder {
    /// The unit's own.
    Unit,
    /// The object the unit rides in (Halo 4's IK sets); the preview places
    /// none.
    Parent,
    /// Its primary weapon's (owner 1) or its secondary's (owner 2).
    Weapon(u8),
}

/// A marker an IK target can name: the node it rides and where it sits on
/// that node in bind space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IkMarker {
    pub(crate) holder: IkHolder,
    pub(crate) name: String,
    pub(crate) node: usize,
    pub(crate) bind: Matrix4,
}

/// Each marker of `model` by its first of a name, as `object_get_markers_by_string_id`
/// takes the first, moved by `place` into the unit's bind space onto `node`
/// (none: the marker's own node).
pub(crate) fn ik_markers(
    model: &super::super::super::RenderModelPreview,
    holder: IkHolder,
    place: impl Fn(&crate::app::model_preview::common::marker::MarkerFrame) -> ([[f32; 3]; 3], [f32; 3]),
    node: Option<usize>,
) -> Vec<IkMarker> {
    let mut out: Vec<IkMarker> = Vec::new();
    for marker in &model.markers {
        if out
            .iter()
            .any(|known| known.name.eq_ignore_ascii_case(&marker.name))
        {
            continue;
        }
        let frame =
            crate::app::model_preview::common::marker::MarkerFrame::find(model, &marker.name);
        let (axes, position) = place(&frame);
        out.push(IkMarker {
            holder,
            name: marker.name.clone(),
            node: node.unwrap_or(frame.node.max(0) as usize),
            bind: matrix_of(axes, position),
        });
    }
    out
}

pub(in crate::app::model_preview) fn transform_point(
    matrix: &Matrix4,
    point: [f32; 3],
) -> [f32; 3] {
    let m = &matrix.m;
    [0, 1, 2]
        .map(|row| m[row][0] * point[0] + m[row][1] * point[1] + m[row][2] * point[2] + m[row][3])
}

/// A matrix with its axes made unit length: `matrix4x3_to_orientation` with
/// the scale forced to 1.
pub(in crate::app::model_preview) fn unscaled(matrix: &Matrix4) -> Matrix4 {
    matrix_of(
        [0, 1, 2].map(|axis| unit_by_reciprocal(axis_of(matrix, axis))),
        place_of(matrix),
    )
}

/// Only the turn of `matrix`.
fn turn_of(matrix: &Matrix4) -> Matrix4 {
    matrix_of([0, 1, 2].map(|axis| axis_of(matrix, axis)), [0.0; 3])
}

/// `a × b`, unit length; where they lie along each other, `a` crossed with
/// the global up, or failing that the global left, as the frame builders
/// fall back on global axes.
fn square(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    for other in [b, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]] {
        let crossed = cross(a, other);
        if dot(crossed, crossed) > 1e-10 {
            return unit_by_reciprocal(crossed);
        }
    }
    [0.0, 0.0, 1.0]
}

/// `frame_forward_up`'s axes: forward along `forward`, up as near `up` as
/// it can be, left square to both.
fn frame_forward_up(forward: [f32; 3], up: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = unit_by_reciprocal(forward);
    let left = square(up, forward);
    [forward, left, cross(forward, left)]
}

/// `frame_forward_left`'s up: square to `forward` and `left`.
fn up_of_forward_left(forward: [f32; 3], left: [f32; 3]) -> [f32; 3] {
    square(unit_by_reciprocal(forward), unit_by_reciprocal(left))
}

/// `ik_alignment`: the turn that takes `matrix`'s forward exactly along
/// `bone`, as a turn to apply after it.
fn ik_alignment(matrix: &Matrix4, bone: [f32; 3]) -> Matrix4 {
    let forward = unit_by_reciprocal(bone);
    if dot(forward, unit_by_reciprocal(axis_of(matrix, 0))) >= 1.0 - 1e-4 {
        return Matrix4::IDENTITY;
    }
    let mut second = [0.0, 0.0, 1.0];
    if dot(forward, second).abs() >= 1.0 - 1e-4 {
        second = [0.0, 1.0, 0.0];
    }
    let up = unit_by_reciprocal(cross(forward, cross(forward, second)));
    let left = unit_by_reciprocal(cross(up, forward));
    let aligned = matrix_of([forward, left, cross(forward, left)], [0.0; 3]);
    rigid_inverse(&turn_of(matrix)) * aligned
}

/// Every node below `node`, by the preview's own hierarchy.
fn below(nodes: &[RenderModelPreviewNode], node: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut queue = vec![node];
    while let Some(at) = queue.pop() {
        for (index, child) in nodes.iter().enumerate() {
            if usize::try_from(child.parent).ok() == Some(at)
                && !out.contains(&index)
                && index != node
            {
                out.push(index);
                queue.push(index);
            }
        }
    }
    out
}

/// `render_model_apply_two_bone_ik` with `g_enable_new_ik_method`: the
/// effector's parent and grandparent turned so it reaches `desired`, taking
/// its turn, then what hangs off each moved along with it.
pub(crate) fn two_bone_ik(
    nodes: &[RenderModelPreviewNode],
    worlds: &mut [Matrix4],
    effector: usize,
    mut desired: Matrix4,
    weight: f32,
    pole: Option<[f32; 3]>,
) {
    let parent = |node: usize| usize::try_from(nodes.get(node)?.parent).ok();
    let Some(joint) = parent(effector) else {
        return;
    };
    let Some(base) = parent(joint) else {
        return;
    };
    let (Some(&e_old), Some(&j_old), Some(&a_old)) =
        (worlds.get(effector), worlds.get(joint), worlds.get(base))
    else {
        return;
    };
    if (weight - 1.0).abs() >= 1e-4 {
        let (_, from, _) = e_old.decompose();
        let (_, to, _) = desired.decompose();
        let place = [0, 1, 2].map(|axis| {
            let (from, to) = (place_of(&e_old)[axis], place_of(&desired)[axis]);
            from + (to - from) * weight
        });
        let turn = from.nlerp(to, weight);
        desired = Matrix4::from_loc_rot_scale(
            RealPoint3d {
                x: place[0],
                y: place[1],
                z: place[2],
            },
            turn,
            1.0,
        );
    }
    let (mut a, mut j, mut e) = (a_old, j_old, e_old);
    let (a_at, j_at, e_at) = (place_of(&a), place_of(&j), place_of(&e));
    let upper_bone = minus(j_at, a_at);
    let lower_bone = minus(e_at, j_at);
    let toward_pole = unit_by_reciprocal(pole.map_or(upper_bone, |pole| minus(pole, a_at)));
    let align_a = ik_alignment(&a, upper_bone);
    a = a * align_a;
    let align_j = ik_alignment(&j, lower_bone);
    j = j * align_j;
    let (upper, lower) = (
        dot(upper_bone, upper_bone).sqrt(),
        dot(lower_bone, lower_bone).sqrt(),
    );
    if upper > 1e-4 && lower > 1e-4 {
        let reach = unit_by_reciprocal(minus(e_at, a_at));
        let to_goal = minus(place_of(&desired), a_at);
        let length = dot(to_goal, to_goal).sqrt();
        let toward = if length > 0.0 {
            scaled(to_goal, 1.0 / length)
        } else {
            reach
        };
        let longest = 0.98 * (upper + lower);
        let shortest = longest.min(1.02 * (upper - lower).abs());
        let distance = length.clamp(shortest, longest);
        let hand_at = plus(a_at, scaled(toward, distance));
        let bend_up = up_of_forward_left(reach, toward_pole);
        let [forward, left, _] = frame_forward_up(toward, bend_up);
        let cosine =
            (lower * lower - distance * distance - upper * upper) / (-2.0 * distance * upper);
        let sine = (1.0 - cosine * cosine).max(1e-8).sqrt();
        let elbow = plus(
            a_at,
            scaled(plus(scaled(forward, cosine), scaled(left, sine)), upper),
        );
        let joint_axes = frame_forward_up(minus(hand_at, elbow), axis_of(&j, 2));
        let base_axes = frame_forward_up(minus(elbow, a_at), axis_of(&a, 2));
        a = matrix_of(base_axes, a_at);
        j = matrix_of(joint_axes, elbow);
        e = matrix_of(
            [0, 1, 2].map(|axis| unit_by_reciprocal(axis_of(&desired, axis))),
            hand_at,
        );
    }
    a = a * rigid_inverse(&align_a);
    j = j * rigid_inverse(&align_j);
    // The root never moves: its solve runs on a copy.
    let root = nodes.get(base).is_some_and(|node| node.parent < 0);
    let moved = |new: &Matrix4, old: &Matrix4| *new * rigid_inverse(old);
    let (d_e, d_j, d_a) = (
        moved(&e, &e_old),
        moved(&j, &j_old),
        if root {
            Matrix4::IDENTITY
        } else {
            moved(&a, &a_old)
        },
    );
    if !root {
        worlds[base] = a;
    }
    worlds[joint] = j;
    worlds[effector] = e;
    // `model_adjust_node_matrices_children_recursive`, then the child fixup.
    for node in below(nodes, effector) {
        worlds[node] = d_e * worlds[node];
    }
    let others = |of: usize, skip: usize| -> Vec<usize> {
        let mut out = Vec::new();
        for (index, child) in nodes.iter().enumerate() {
            if usize::try_from(child.parent).ok() == Some(of) && index != skip {
                out.push(index);
                out.extend(below(nodes, index));
            }
        }
        out
    };
    for node in others(joint, effector) {
        worlds[node] = d_j * worlds[node];
    }
    for node in others(base, joint) {
        worlds[node] = d_a * worlds[node];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::math::RealQuaternion;

    fn node(name: &str, parent: i16) -> RenderModelPreviewNode {
        RenderModelPreviewNode {
            name: name.to_owned(),
            parent,
            ..Default::default()
        }
    }

    fn at(place: [f32; 3]) -> Matrix4 {
        matrix_of([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], place)
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        let gap = minus(a, b);
        dot(gap, gap).sqrt()
    }

    /// A root, a shoulder, an upper arm and forearm of 1 each along x, a
    /// hand, and a finger off the hand.
    fn arm() -> (Vec<RenderModelPreviewNode>, Vec<Matrix4>) {
        let nodes = vec![
            node("root", -1),
            node("upper", 0),
            node("fore", 1),
            node("hand", 2),
            node("finger", 3),
        ];
        let worlds = vec![
            at([0.0; 3]),
            at([0.0; 3]),
            at([1.0, 0.0, 0.0]),
            at([2.0, 0.0, 0.0]),
            at([2.5, 0.0, 0.0]),
        ];
        (nodes, worlds)
    }

    #[test]
    fn the_hand_lands_on_a_target_in_reach_with_its_turn() {
        let (nodes, mut worlds) = arm();
        let turned = Matrix4::from_loc_rot_scale(
            RealPoint3d {
                x: 1.2,
                y: 0.8,
                z: 0.3,
            },
            RealQuaternion::IDENTITY,
            1.0,
        );
        let turned = turned
            * matrix_of(
                [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                [0.0; 3],
            );
        two_bone_ik(&nodes, &mut worlds, 3, turned, 1.0, None);
        assert!(
            distance(place_of(&worlds[3]), [1.2, 0.8, 0.3]) < 1e-4,
            "{:?}",
            place_of(&worlds[3])
        );
        assert!(
            distance(axis_of(&worlds[3], 0), [0.0, 1.0, 0.0]) < 1e-4,
            "the hand takes the target's turn"
        );
        assert!(
            (distance(place_of(&worlds[2]), place_of(&worlds[1])) - 1.0).abs() < 1e-4,
            "the upper arm keeps its length"
        );
        assert!(
            (distance(place_of(&worlds[3]), place_of(&worlds[2])) - 1.0).abs() < 1e-4,
            "and the forearm its"
        );
        assert!(
            (distance(place_of(&worlds[4]), place_of(&worlds[3])) - 0.5).abs() < 1e-4,
            "the finger goes with the hand"
        );
        assert_eq!(place_of(&worlds[0]), [0.0; 3], "the root stays");
    }

    #[test]
    fn a_target_out_of_reach_takes_the_arm_to_its_limit() {
        let (nodes, mut worlds) = arm();
        two_bone_ik(&nodes, &mut worlds, 3, at([5.0, 0.0, 0.0]), 1.0, None);
        assert!(
            (place_of(&worlds[3])[0] - 1.96).abs() < 1e-4,
            "0.98 of the arm: {:?}",
            place_of(&worlds[3])
        );
    }

    #[test]
    fn half_weight_reaches_half_way() {
        let (nodes, mut worlds) = arm();
        let target = [1.0, 1.0, 0.0];
        two_bone_ik(&nodes, &mut worlds, 3, at(target), 0.5, None);
        assert!(
            distance(place_of(&worlds[3]), [1.5, 0.5, 0.0]) < 1e-4,
            "{:?}",
            place_of(&worlds[3])
        );
    }
}
