//! Reach's IK chains (`apply_ik_chains`): after a data-driven graph's
//! animation, each chain's hand or foot is solved to a marker, mostly the held
//! weapon's, at the offset and weight its animation carries for it. See
//! `docs/reach-preview-research/ik.md`.

use blam_tags::animation::{decode_extended_block, AnimationGroup, ExtendedChannel};
use blam_tags::math::{Matrix4, RealPoint3d, RealQuaternion};
use blam_tags::{Skeleton, TagFile};

use super::super::attachment::{added, apart, axis_of, matrix_of, place_of, rigid_inverse};
use super::super::common::fields::elements;
use super::super::RenderModelPreviewNode;
use crate::app::model_preview::common::math::{cross, dot, scaled, unit_by_reciprocal};

/// `e_animation_ik_chain_event_usage`: whose markers a target names.
const USAGE_SELF: i128 = 2;
const USAGE_PRIMARY_WEAPON: i128 = 4;
const USAGE_SECONDARY_WEAPON: i128 = 5;
/// `disable ik chains`, of an animation's `user flags`.
const DISABLE_IK_CHAINS: i128 = 1 << 11;
/// `apply_ik_chains` solves ranks 0 to 3.
const MOST_RANKS: usize = 4;
const SMALLEST_WEIGHT: f32 = 1e-4;

/// Whose marker a target is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReachIkHolder {
    /// The unit's own.
    Unit,
    /// The object the unit rides in (Halo 4's IK sets); the preview places
    /// none.
    Parent,
    /// Its primary weapon's (owner 1) or its secondary's (owner 2).
    Weapon(u8),
}

/// One chain's target through one clip, a value per frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReachIkTrack {
    /// The graph's chain order by start node: the order chains solve in.
    pub(crate) rank: usize,
    /// The effector node, by name.
    pub(crate) effector: String,
    pub(crate) holder: ReachIkHolder,
    pub(crate) marker: String,
    /// The effector, in the marker's space.
    pub(crate) effector_frames: Vec<(RealQuaternion, [f32; 3])>,
    pub(crate) weights: Vec<f32>,
    /// A point the elbow or knee bends toward, in the root node's space.
    pub(crate) poles: Option<Vec<[f32; 3]>>,
}

/// One graph `ik chains` entry, its nodes by name.
struct GraphChain {
    name: String,
    kind: i128,
    start: String,
    effector: String,
    rank: usize,
}

fn node_name(skeleton: &Skeleton, index: i128) -> String {
    usize::try_from(index).ok().and_then(|index| skeleton.nodes.get(index)).map_or_else(String::new, |node| node.name.clone())
}

/// The chains of the first graph down `graphs` (the clip's own, then its
/// parents) that has any, `inherit_ik_chains_from_parent_graph` resolving
/// their nodes by name. Ranked by start node, as `post_process_ik_chains`.
fn graph_chains(graphs: &[&TagFile]) -> Vec<GraphChain> {
    for graph in graphs {
        let Some(definitions) = graph.root().descend("definitions") else {
            continue;
        };
        let skeleton = Skeleton::from_tag(graph);
        let chains = elements(definitions, "ik chains");
        if chains.is_empty() {
            continue;
        }
        let mut found: Vec<(i128, GraphChain)> = chains
            .iter()
            .map(|chain| {
                let start = chain.read_int_any("start node").unwrap_or(-1);
                let chain = GraphChain {
                    name: chain.read_string_id("name").unwrap_or_default(),
                    kind: chain.read_int_any("type").unwrap_or(0),
                    start: node_name(&skeleton, start),
                    effector: node_name(&skeleton, chain.read_int_any("effector node").unwrap_or(-1)),
                    rank: 0,
                };
                (start, chain)
            })
            .collect();
        found.sort_by_key(|(start, _)| *start);
        return found
            .into_iter()
            .enumerate()
            .map(|(rank, (_, chain))| GraphChain { rank, ..chain })
            .collect();
    }
    Vec::new()
}

/// What each entry of a section holds, frame by frame.
fn channels(group: &AnimationGroup<'_>, name: &str) -> Vec<ExtendedChannel> {
    group.data_section(name).and_then(|section| decode_extended_block(section).ok()).unwrap_or_default()
}

/// The chain targets the clip `group` of `graphs[0]` carries
/// (`setup_ik_chain_targets_for_animation`): none if the clip turns its
/// chains off. Targets through proxies are left out; no kit unit has one.
pub(crate) fn read_reach_ik_tracks(graphs: &[&TagFile], group: &AnimationGroup<'_>) -> Vec<ReachIkTrack> {
    let Some(graph) = graphs.first() else {
        return Vec::new();
    };
    let Some(clip) = graph
        .root()
        .descend("definitions")
        .and_then(|definitions| elements(definitions, "animations").get(group.index).copied())
    else {
        return Vec::new();
    };
    if clip.read_int_any("user flags").is_some_and(|flags| flags & DISABLE_IK_CHAINS != 0) {
        return Vec::new();
    }
    let Some(shared) = elements(clip, "shared animation data").first().copied() else {
        return Vec::new();
    };
    let events = elements(shared, "ik chain events");
    if events.is_empty() {
        return Vec::new();
    }
    let chains = graph_chains(graphs);
    let skeleton = Skeleton::from_tag(graph);
    let transforms = channels(group, "ik_chain_event_data");
    let weights = channels(group, "ik_chain_control_data");
    let poles = channels(group, "ik_chain_pole_vector_data");
    // Reach's indices are signed, −1 for none; Halo 4's are unsigned bytes,
    // and its sampler takes 0xFF as none (`sample_extended_block_data`,
    // X360 0x83464668).
    let entry = |index: i128| usize::try_from(index).ok().filter(|at| *at != 0xFF);
    let mut tracks = Vec::new();
    for event in events {
        if event.read_int_any("proxy id").is_some_and(|id| id != -1) {
            continue;
        }
        let name = event.read_string_id("chain name").unwrap_or_default();
        let kind = event.read_int_any("chain type").unwrap_or(0);
        let start = node_name(&skeleton, event.read_int_any("chain start node").unwrap_or(-1));
        let effector = node_name(&skeleton, event.read_int_any("chain effector node").unwrap_or(-1));
        // An event naming no chain of the graph is deleted at load.
        let Some(chain) = chains
            .iter()
            .find(|chain| chain.name == name && chain.kind == kind && chain.start == start && chain.effector == effector)
        else {
            continue;
        };
        if chain.rank >= MOST_RANKS {
            continue;
        }
        let holder = match event.read_int_any("chain usage") {
            Some(USAGE_SELF) => ReachIkHolder::Unit,
            Some(USAGE_PRIMARY_WEAPON) => ReachIkHolder::Weapon(1),
            Some(USAGE_SECONDARY_WEAPON) => ReachIkHolder::Weapon(2),
            // World, parent and assassination targets have nothing to find
            // in a preview.
            _ => continue,
        };
        let transform = entry(event.read_int_any("effector transform data index").unwrap_or(-1));
        let effector_frames = match transform.and_then(|at| transforms.get(at..at + 2)) {
            Some([ExtendedChannel::Rotation(rotations), ExtendedChannel::Translation(translations)]) => rotations
                .iter()
                .zip(translations)
                .map(|(rotation, translation)| (*rotation, [translation.x, translation.y, translation.z]))
                .collect(),
            _ => continue,
        };
        // With no weight a target stays at the zero it was cleared to.
        let weight = entry(event.read_int_any("effector weight data index").unwrap_or(-1));
        let Some(ExtendedChannel::Scalar(weights)) = weight.and_then(|at| weights.get(at)) else {
            continue;
        };
        let pole = entry(event.read_int_any("pole point data index").unwrap_or(-1));
        let poles = match pole.and_then(|at| poles.get(at)) {
            Some(ExtendedChannel::Translation(points)) => Some(points.iter().map(|point| [point.x, point.y, point.z]).collect()),
            _ => None,
        };
        tracks.push(ReachIkTrack {
            rank: chain.rank,
            effector: chain.effector.clone(),
            holder,
            marker: event.read_string_id("proxy marker").unwrap_or_default(),
            effector_frames,
            weights: weights.clone(),
            poles,
        });
    }
    tracks.sort_by_key(|track| track.rank);
    tracks
}

/// A marker an IK target can name: the node it rides and where it sits on
/// that node in bind space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReachIkMarker {
    pub(crate) holder: ReachIkHolder,
    pub(crate) name: String,
    pub(crate) node: usize,
    pub(crate) bind: Matrix4,
}

/// Each marker of `model` by its first of a name, as `object_get_markers_by_string_id`
/// takes the first, moved by `place` into the unit's bind space onto `node`
/// (none: the marker's own node).
pub(crate) fn reach_ik_markers(
    model: &super::super::RenderModelPreview,
    holder: ReachIkHolder,
    place: impl Fn(&super::super::attachment::MarkerFrame) -> ([[f32; 3]; 3], [f32; 3]),
    node: Option<usize>,
) -> Vec<ReachIkMarker> {
    let mut out: Vec<ReachIkMarker> = Vec::new();
    for marker in &model.markers {
        if out.iter().any(|known| known.name.eq_ignore_ascii_case(&marker.name)) {
            continue;
        }
        let frame = super::super::attachment::MarkerFrame::find(model, &marker.name);
        let (axes, position) = place(&frame);
        out.push(ReachIkMarker {
            holder,
            name: marker.name.clone(),
            node: node.unwrap_or(frame.node.max(0) as usize),
            bind: matrix_of(axes, position),
        });
    }
    out
}

/// A value of `values` between frames `a` and `b`.
fn sampled<T: Copy>(values: &[T], a: usize, b: usize) -> Option<(T, T)> {
    let last = values.len().checked_sub(1)?;
    Some((values[a.min(last)], values[b.min(last)]))
}

/// Solve every chain of `tracks` on the posed `worlds`, between frames `a`
/// and `b` at `blend`. A chain whose marker is missing is skipped, where the
/// game would solve to the offset alone.
pub(crate) fn apply_reach_ik(
    tracks: &[ReachIkTrack],
    markers: &[ReachIkMarker],
    nodes: &[RenderModelPreviewNode],
    inverse_binds: &[Matrix4],
    worlds: &mut [Matrix4],
    (a, b, blend): (usize, usize, f32),
) {
    let mut solved: Vec<usize> = Vec::new();
    for track in tracks {
        let Some((first, second)) = sampled(&track.weights, a, b) else {
            continue;
        };
        let weight = first + (second - first) * blend;
        if weight.abs() < SMALLEST_WEIGHT {
            continue;
        }
        let Some(marker) = markers.iter().find(|marker| marker.holder == track.holder && marker.name == track.marker) else {
            continue;
        };
        let (Some(holder), Some(inverse)) = (worlds.get(marker.node), inverse_binds.get(marker.node)) else {
            continue;
        };
        let on = *holder * *inverse * marker.bind;
        let Some(((qa, ta), (qb, tb))) = sampled(&track.effector_frames, a, b) else {
            continue;
        };
        let offset = (0..3).map(|axis| ta[axis] + (tb[axis] - ta[axis]) * blend).collect::<Vec<_>>();
        let point = RealPoint3d { x: offset[0], y: offset[1], z: offset[2] };
        let desired = unscaled(&on) * Matrix4::from_loc_rot_scale(point, qa.nlerp(qb, blend), 1.0);
        let Some(effector) = nodes.iter().position(|node| node.name == track.effector) else {
            continue;
        };
        if solved.contains(&effector) {
            continue;
        }
        solved.push(effector);
        let pole = track.poles.as_ref().and_then(|poles| sampled(poles, a, b)).and_then(|(pa, pb)| {
            let local = [0, 1, 2].map(|axis| pa[axis] + (pb[axis] - pa[axis]) * blend);
            let root = worlds.first()?;
            Some(transform_point(root, local))
        });
        two_bone_ik(nodes, worlds, effector, desired, weight, pole);
    }
}

fn transform_point(matrix: &Matrix4, point: [f32; 3]) -> [f32; 3] {
    let m = &matrix.m;
    [0, 1, 2].map(|row| m[row][0] * point[0] + m[row][1] * point[1] + m[row][2] * point[2] + m[row][3])
}

/// A matrix with its axes made unit length: `matrix4x3_to_orientation` with
/// the scale forced to 1.
fn unscaled(matrix: &Matrix4) -> Matrix4 {
    matrix_of([0, 1, 2].map(|axis| unit_by_reciprocal(axis_of(matrix, axis))), place_of(matrix))
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
            if usize::try_from(child.parent).ok() == Some(at) && !out.contains(&index) && index != node {
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
    let (Some(&e_old), Some(&j_old), Some(&a_old)) = (worlds.get(effector), worlds.get(joint), worlds.get(base)) else {
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
        desired = Matrix4::from_loc_rot_scale(RealPoint3d { x: place[0], y: place[1], z: place[2] }, turn, 1.0);
    }
    let (mut a, mut j, mut e) = (a_old, j_old, e_old);
    let (a_at, j_at, e_at) = (place_of(&a), place_of(&j), place_of(&e));
    let upper_bone = apart(j_at, a_at);
    let lower_bone = apart(e_at, j_at);
    let toward_pole = unit_by_reciprocal(pole.map_or(upper_bone, |pole| apart(pole, a_at)));
    let align_a = ik_alignment(&a, upper_bone);
    a = a * align_a;
    let align_j = ik_alignment(&j, lower_bone);
    j = j * align_j;
    let (upper, lower) = (dot(upper_bone, upper_bone).sqrt(), dot(lower_bone, lower_bone).sqrt());
    if upper > 1e-4 && lower > 1e-4 {
        let reach = unit_by_reciprocal(apart(e_at, a_at));
        let to_goal = apart(place_of(&desired), a_at);
        let length = dot(to_goal, to_goal).sqrt();
        let toward = if length > 0.0 { scaled(to_goal, 1.0 / length) } else { reach };
        let longest = 0.98 * (upper + lower);
        let shortest = longest.min(1.02 * (upper - lower).abs());
        let distance = length.clamp(shortest, longest);
        let hand_at = added(a_at, scaled(toward, distance));
        let bend_up = up_of_forward_left(reach, toward_pole);
        let [forward, left, _] = frame_forward_up(toward, bend_up);
        let cosine = (lower * lower - distance * distance - upper * upper) / (-2.0 * distance * upper);
        let sine = (1.0 - cosine * cosine).max(1e-8).sqrt();
        let elbow = added(a_at, scaled(added(scaled(forward, cosine), scaled(left, sine)), upper));
        let joint_axes = frame_forward_up(apart(hand_at, elbow), axis_of(&j, 2));
        let base_axes = frame_forward_up(apart(elbow, a_at), axis_of(&a, 2));
        a = matrix_of(base_axes, a_at);
        j = matrix_of(joint_axes, elbow);
        e = matrix_of([0, 1, 2].map(|axis| unit_by_reciprocal(axis_of(&desired, axis))), hand_at);
    }
    a = a * rigid_inverse(&align_a);
    j = j * rigid_inverse(&align_j);
    // The root never moves: its solve runs on a copy.
    let root = nodes.get(base).is_some_and(|node| node.parent < 0);
    let moved = |new: &Matrix4, old: &Matrix4| *new * rigid_inverse(old);
    let (d_e, d_j, d_a) = (moved(&e, &e_old), moved(&j, &j_old), if root { Matrix4::IDENTITY } else { moved(&a, &a_old) });
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
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::core::game::GameId;

    fn node(name: &str, parent: i16) -> RenderModelPreviewNode {
        RenderModelPreviewNode { name: name.to_owned(), parent, ..Default::default() }
    }

    fn at(place: [f32; 3]) -> Matrix4 {
        matrix_of([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], place)
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        let gap = apart(a, b);
        dot(gap, gap).sqrt()
    }

    /// A root, a shoulder, an upper arm and forearm of 1 each along x, a
    /// hand, and a finger off the hand.
    fn arm() -> (Vec<RenderModelPreviewNode>, Vec<Matrix4>) {
        let nodes = vec![node("root", -1), node("upper", 0), node("fore", 1), node("hand", 2), node("finger", 3)];
        let worlds = vec![at([0.0; 3]), at([0.0; 3]), at([1.0, 0.0, 0.0]), at([2.0, 0.0, 0.0]), at([2.5, 0.0, 0.0])];
        (nodes, worlds)
    }

    #[test]
    fn the_hand_lands_on_a_target_in_reach_with_its_turn() {
        let (nodes, mut worlds) = arm();
        let turned = Matrix4::from_loc_rot_scale(RealPoint3d { x: 1.2, y: 0.8, z: 0.3 }, RealQuaternion::IDENTITY, 1.0);
        let turned = turned * matrix_of([[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], [0.0; 3]);
        two_bone_ik(&nodes, &mut worlds, 3, turned, 1.0, None);
        assert!(distance(place_of(&worlds[3]), [1.2, 0.8, 0.3]) < 1e-4, "{:?}", place_of(&worlds[3]));
        assert!(distance(axis_of(&worlds[3], 0), [0.0, 1.0, 0.0]) < 1e-4, "the hand takes the target's turn");
        assert!((distance(place_of(&worlds[2]), place_of(&worlds[1])) - 1.0).abs() < 1e-4, "the upper arm keeps its length");
        assert!((distance(place_of(&worlds[3]), place_of(&worlds[2])) - 1.0).abs() < 1e-4, "and the forearm its");
        assert!((distance(place_of(&worlds[4]), place_of(&worlds[3])) - 0.5).abs() < 1e-4, "the finger goes with the hand");
        assert_eq!(place_of(&worlds[0]), [0.0; 3], "the root stays");
    }

    #[test]
    fn a_target_out_of_reach_takes_the_arm_to_its_limit() {
        let (nodes, mut worlds) = arm();
        two_bone_ik(&nodes, &mut worlds, 3, at([5.0, 0.0, 0.0]), 1.0, None);
        assert!((place_of(&worlds[3])[0] - 1.96).abs() < 1e-4, "0.98 of the arm: {:?}", place_of(&worlds[3]));
    }

    #[test]
    fn half_weight_reaches_half_way() {
        let (nodes, mut worlds) = arm();
        let target = [1.0, 1.0, 0.0];
        two_bone_ik(&nodes, &mut worlds, 3, at(target), 0.5, None);
        assert!(distance(place_of(&worlds[3]), [1.5, 0.5, 0.0]) < 1e-4, "{:?}", place_of(&worlds[3]));
    }

    /// With `BLAM_TEST_HREK`: the Spartan's rifle idle holds both hands to
    /// its primary weapon, the left arm first by rank, at full weight.
    #[test]
    fn the_spartans_rifle_idle_reaches_for_its_primary_weapon() {
        let Some(root) = kit_root(GameId::HaloReach) else { return };
        let path = std::path::Path::new(&root).join("objects/characters/spartans/spartans.model_animation_graph");
        let graph = TagFile::read(path).unwrap();
        let animation = blam_tags::Animation::new(&graph).unwrap();
        let idle = animation.find("combat:rifle:idle").unwrap();
        let tracks = read_reach_ik_tracks(&[&graph], idle);
        let summary: Vec<_> = tracks.iter().map(|track| (track.rank, track.effector.as_str(), track.marker.as_str(), track.holder)).collect();
        assert_eq!(
            summary,
            [
                (2, "b_l_hand", "left_hand_spartan", ReachIkHolder::Weapon(1)),
                (3, "b_r_hand", "right_hand_spartan", ReachIkHolder::Weapon(1)),
            ]
        );
        assert!(tracks.iter().all(|track| track.weights.iter().all(|weight| (weight - 1.0).abs() < 1e-3)));
        assert_eq!(tracks[0].effector_frames.len(), 156);
    }
}
