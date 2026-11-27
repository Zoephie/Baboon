//! Posing a skeleton by hand, for viewing: joints dragged in the viewport,
//! over the bind pose or over whatever an animation poses.
//!
//! A pose is a turn per node, each about the node's own origin in its
//! parent's frame, laid over the preview's pose after the animation and the
//! game's IK. Only turns: a bone is a node's offset from its parent, which
//! no turn changes, so nothing stretches. The mesh follows through the
//! game's own skin weights, as it does for an animation.
//!
//! Dragging a joint carries it across the plane through it that faces the
//! camera, turning the chain of nodes above it, solved by FABRIK.
//!
//! Where the physics model limits a joint, the pose keeps to its limits as
//! the game's ragdolls do (`c_havok_component::create_constraint`, Halo 3
//! 0x1803206F0): a ragdoll joint's twist about its frame's forward, its cone
//! about forward and its angle off the plane square to left; a limited
//! hinge's turn about up, measured from left.

use super::RenderModelPreviewNode;
use super::common::fields::elements;
use blam_tags::math::{Matrix4, RealQuaternion, RealVector3d};
use blam_tags::{TagFieldData, TagFile, TagStruct};
use std::collections::BTreeMap;

/// The hand pose, and the posing under way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PoseEdit {
    /// Whether a drag on a joint poses it rather than orbiting the camera.
    pub on: bool,
    /// Each turned node's turn, in its parent's frame.
    pub turns: BTreeMap<usize, RealQuaternion>,
    /// The joint last picked, which the armature marks.
    pub selected: Option<usize>,
    /// The drag under way: the joint dragged.
    pub dragging: Option<PoseDrag>,
    /// How many nodes above a dragged joint turn: its parent alone at 1.
    pub chain: usize,
    /// Whether the physics model's joint limits hold.
    pub limits: bool,
    /// The geometry the turns are for (`ModelPreviewData::geometry_id`);
    /// another model's nodes start unposed.
    pub geometry: u64,
}

/// A joint being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PoseDrag {
    pub joint: usize,
}

/// A dragged joint's chain to begin with: its parent and grandparent, as
/// Reach's and Halo 4's graphs run each limb's IK chain from an effector to
/// its grandparent.
pub(crate) const DEFAULT_CHAIN: usize = 2;

impl Default for PoseEdit {
    fn default() -> Self {
        Self {
            on: false,
            turns: BTreeMap::new(),
            selected: None,
            dragging: None,
            chain: DEFAULT_CHAIN,
            limits: true,
            geometry: 0,
        }
    }
}

impl PoseEdit {
    /// Whether any node is turned from the pose beneath.
    pub(crate) fn is_posed(&self) -> bool {
        !self.turns.is_empty()
    }

    /// Unposed, for `geometry`'s nodes, if the turns are another model's.
    pub(crate) fn follow(&mut self, geometry: u64) {
        if self.geometry != geometry {
            *self = Self {
                on: self.on,
                chain: self.chain,
                limits: self.limits,
                geometry,
                ..Self::default()
            };
        }
    }

    /// Every turn undone.
    pub(crate) fn reset(&mut self) {
        self.turns.clear();
        self.dragging = None;
    }

    /// Turn `node` further by `turn`, a turn in model space about the node's
    /// origin, given the node's parent's world transform now (none for a
    /// root).
    pub(crate) fn turn_by(&mut self, node: usize, parent: Option<&Matrix4>, turn: RealQuaternion) {
        let parent = parent.map_or(RealQuaternion::IDENTITY, |parent| parent.decompose().1);
        // World `P · T · q` turned by `D` is `P · (P⁻¹ D P · T) · q`.
        let local = (parent.conjugate() * turn * parent).normalized();
        let before = self
            .turns
            .get(&node)
            .copied()
            .unwrap_or(RealQuaternion::IDENTITY);
        self.turns.insert(node, (local * before).normalized());
    }
}

/// `matrices`, each node's world transform as the preview poses it, with
/// `edit`'s turns laid over: each node's transform relative to its parent
/// kept, but for its turn, and its parent's posed transform carried down.
pub(in crate::app::model_preview) fn apply(
    nodes: &[RenderModelPreviewNode],
    matrices: &mut [Matrix4],
    edit: &PoseEdit,
) {
    if !edit.is_posed() {
        return;
    }
    let before = matrices.to_vec();
    for (index, node) in nodes.iter().enumerate().take(matrices.len()) {
        let parent = usize::try_from(node.parent)
            .ok()
            .filter(|&parent| parent < index);
        // Its transform relative to its parent, as posed beneath.
        let local = match parent {
            Some(parent) => before[parent].inverse() * before[index],
            None => before[index],
        };
        let local = match edit.turns.get(&index) {
            Some(turn) => {
                let (translation, rotation, scale) = local.decompose();
                Matrix4::from_loc_rot_scale(translation, (*turn * rotation).normalized(), scale)
            }
            None => local,
        };
        matrices[index] = match parent {
            Some(parent) => matrices[parent] * local,
            None => local,
        };
    }
}

/// The nodes a drag of `joint` turns, nearest first: up to `links` of its
/// ancestors.
pub(crate) fn chain_above(
    nodes: &[RenderModelPreviewNode],
    joint: usize,
    links: usize,
) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut at = joint;
    while chain.len() < links {
        let Some(parent) = nodes
            .get(at)
            .and_then(|node| usize::try_from(node.parent).ok())
            .filter(|&parent| parent < nodes.len())
        else {
            break;
        };
        chain.push(parent);
        at = parent;
    }
    chain
}

/// Passes of the solve each drag event takes, at most.
const ITERATIONS: usize = 12;

/// Turn `chain` (nearest first) so `joint` reaches toward `target`, its
/// turns laid over `base` (each node's world transform beneath the hand
/// pose). FABRIK (Aristidou and Lasenby) places the chain's joints, each
/// bone its own length: a target out of reach straightens the chain toward
/// it; one in reach takes passes back from the target and forward from the
/// fixed root until the joint lands on it. Each node then takes the least
/// swing that carries its child onto its new place, root first, which adds
/// no twist.
pub(crate) fn reach(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    chain: &[usize],
    target: [f32; 3],
    limits: &[ResolvedLimit],
) {
    // The chain's joints that a limit holds, each against its other side.
    let held: Vec<&ResolvedLimit> = limits
        .iter()
        .filter(|limit| chain.contains(&limit.b))
        .collect();
    // Placed, then held to its limits, which may leave the joint short of
    // the target: placed again from there, a few times over.
    let rounds = if held.is_empty() { 1 } else { LIMIT_ROUNDS };
    for _ in 0..rounds {
        place(nodes, base, edit, joint, chain, target);
        for limit in &held {
            hold(nodes, base, edit, limit);
        }
    }
}

/// Rounds of placing and holding to limits each drag event takes.
const LIMIT_ROUNDS: usize = 4;

/// FABRIK's placing of `chain` so `joint` lands on `target`, and each
/// node's swing onto it.
fn place(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    chain: &[usize],
    target: [f32; 3],
) {
    if chain.is_empty() {
        return;
    }
    let posed = |edit: &PoseEdit| {
        let mut matrices = base.to_vec();
        apply(nodes, &mut matrices, edit);
        matrices
    };
    // The chain root first, the dragged joint last.
    let order: Vec<usize> = chain.iter().rev().copied().chain([joint]).collect();
    let matrices = posed(edit);
    let mut places: Vec<[f32; 3]> = order
        .iter()
        .map(|&node| matrices[node].translation())
        .collect();
    let lengths: Vec<f32> = places
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .collect();
    let reach_length: f32 = lengths.iter().sum();
    if reach_length <= 1e-6 {
        return;
    }
    let toward = |from: [f32; 3], to: [f32; 3], length: f32| {
        let span = sub(to, from);
        let norm = norm(span);
        if norm < 1e-9 {
            from
        } else {
            std::array::from_fn(|axis| from[axis] + span[axis] * length / norm)
        }
    };
    let root = places[0];
    if distance(root, target) >= reach_length {
        for index in 0..lengths.len() {
            places[index + 1] = toward(places[index], target, lengths[index]);
        }
    } else {
        for _ in 0..ITERATIONS {
            if distance(places[places.len() - 1], target) < 1e-5 * reach_length {
                break;
            }
            let last = places.len() - 1;
            places[last] = target;
            for index in (0..last).rev() {
                places[index] = toward(places[index + 1], places[index], lengths[index]);
            }
            places[0] = root;
            for index in 0..last {
                places[index + 1] = toward(places[index], places[index + 1], lengths[index]);
            }
        }
    }
    // Each node, root first, swung so its child lands on its place.
    for index in 0..lengths.len() {
        let matrices = posed(edit);
        let (node, child) = (order[index], order[index + 1]);
        let Some(turn) = swing(
            matrices[node].translation(),
            matrices[child].translation(),
            places[index + 1],
        ) else {
            continue;
        };
        let parent = usize::try_from(nodes[node].parent)
            .ok()
            .and_then(|parent| matrices.get(parent).copied());
        edit.turn_by(node, parent.as_ref(), turn);
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn norm(v: [f32; 3]) -> f32 {
    v.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    norm(sub(a, b))
}

/// A physics model's joint limit, its nodes by name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JointLimit {
    pub name: String,
    /// `node a`'s and `node b`'s names: the side held still and the side
    /// limited against it.
    pub a: String,
    pub b: String,
    /// Each side's frame in its node's space, its `forward`, `left` and
    /// `up` the columns.
    pub frame_a: RealQuaternion,
    pub frame_b: RealQuaternion,
    pub kind: LimitKind,
}

/// What a joint limit allows, in radians, as the game hands it to Havok.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LimitKind {
    /// `hkRagdollConstraintData`: the twist about forward, the cone's half
    /// angle about forward, and the angle off the plane square to left.
    Ragdoll {
        twist: [f32; 2],
        cone: f32,
        plane: [f32; 2],
    },
    /// `hkLimitedHingeConstraintData`: the turn about up, from left.
    Hinge { angle: [f32; 2] },
}

/// A joint limit with its nodes found in the skeleton.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedLimit {
    pub a: usize,
    pub b: usize,
    pub frame_a: RealQuaternion,
    pub frame_b: RealQuaternion,
    pub kind: LimitKind,
}

/// `limits` whose nodes `nodes` has, found by name as the game's model node
/// maps match a physics model's nodes to its render model's.
pub(crate) fn resolve_limits(
    nodes: &[RenderModelPreviewNode],
    limits: &[JointLimit],
) -> Vec<ResolvedLimit> {
    let find = |name: &str| {
        nodes
            .iter()
            .position(|node| node.name.eq_ignore_ascii_case(name))
    };
    let mut resolved: Vec<ResolvedLimit> = limits
        .iter()
        .filter_map(|limit| {
            Some(ResolvedLimit {
                a: find(&limit.a)?,
                b: find(&limit.b)?,
                frame_a: limit.frame_a,
                frame_b: limit.frame_b,
                kind: limit.kind,
            })
        })
        .collect();
    // Parents first, so a joint is held before what hangs below it.
    resolved.sort_by_key(|limit| limit.b);
    resolved
}

/// The physics model's ragdoll and limited hinge constraints, read by name.
/// The tag holds degrees; `create_constraint` hands Havok the twist as it
/// is, the cone's larger end, and the plane's and hinge's ranges negated.
pub(in crate::app::model_preview) fn read_joint_limits(physics: &TagFile) -> Vec<JointLimit> {
    let root = physics.root();
    let names: Vec<String> = elements(root, "nodes")
        .iter()
        .map(|node| node.read_string_id("name").unwrap_or_default())
        .collect();
    let real =
        |holder: &TagStruct<'_>, name: &str| holder.read_real(name).unwrap_or(0.0).to_radians();
    let ordered = |low: f32, high: f32| [low.min(high), low.max(high)];
    let negated = |low: f32, high: f32| ordered(-low, -high);
    let mut limits = Vec::new();
    for constraint in elements(root, "ragdoll constraints") {
        let kind = LimitKind::Ragdoll {
            twist: ordered(
                real(&constraint, "min twist"),
                real(&constraint, "max twist"),
            ),
            cone: real(&constraint, "min cone").max(real(&constraint, "max cone")),
            plane: negated(
                real(&constraint, "min plane"),
                real(&constraint, "max plane"),
            ),
        };
        limits.extend(read_bodies(&constraint, &names, kind));
    }
    for constraint in elements(root, "limited hinge constraints") {
        let kind = LimitKind::Hinge {
            angle: negated(
                real(&constraint, "limit min angle"),
                real(&constraint, "limit max angle"),
            ),
        };
        limits.extend(read_bodies(&constraint, &names, kind));
    }
    limits
}

/// A constraint's `constraint bodies`: its two nodes and their frames.
fn read_bodies(
    constraint: &TagStruct<'_>,
    names: &[String],
    kind: LimitKind,
) -> Option<JointLimit> {
    let bodies = constraint.field("constraint bodies")?.as_struct()?;
    let node = |field: &str| {
        let index = usize::try_from(bodies.read_int_any(field)?).ok()?;
        names.get(index).filter(|name| !name.is_empty()).cloned()
    };
    let vector = |field: &str| match bodies.field(field).and_then(|field| field.value()) {
        Some(TagFieldData::RealVector3d(vector)) => Some(vector),
        _ => None,
    };
    let frame = |side: &str| {
        let [forward, left, up] =
            ["forward", "left", "up"].map(|axis| vector(&format!("{side} {axis}")));
        Some(RealQuaternion::from_basis_columns(forward?, left?, up?).normalized())
    };
    Some(JointLimit {
        name: bodies.read_string_id("name").unwrap_or_default(),
        a: node("node a")?,
        b: node("node b")?,
        frame_a: frame("a")?,
        frame_b: frame("b")?,
        kind,
    })
}

/// `limit`'s node b turned so its frame keeps to the limit against node a's.
fn hold(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    limit: &ResolvedLimit,
) {
    let mut matrices = base.to_vec();
    apply(nodes, &mut matrices, edit);
    let (Some(a), Some(b)) = (matrices.get(limit.a), matrices.get(limit.b)) else {
        return;
    };
    let turn_a = a.decompose().1 * limit.frame_a;
    let turn_b = b.decompose().1;
    let frame_b = turn_b * limit.frame_b;
    // B's frame in A's.
    let relative = (turn_a.conjugate() * frame_b).normalized();
    let held = match limit.kind {
        LimitKind::Ragdoll { twist, cone, plane } => hold_ragdoll(relative, twist, cone, plane),
        LimitKind::Hinge { angle } => hold_hinge(relative, angle),
    };
    // The world turn that carries B's frame onto the held one.
    let wanted = (turn_a * held).normalized();
    let turn = (wanted * frame_b.conjugate()).normalized();
    if (turn.w.abs() - 1.0).abs() < 1e-7 {
        return;
    }
    let parent = usize::try_from(nodes[limit.b].parent)
        .ok()
        .and_then(|parent| matrices.get(parent).copied());
    edit.turn_by(limit.b, parent.as_ref(), turn);
}

const FORWARD: RealVector3d = RealVector3d {
    i: 1.0,
    j: 0.0,
    k: 0.0,
};
const LEFT: RealVector3d = RealVector3d {
    i: 0.0,
    j: 1.0,
    k: 0.0,
};
const UP: RealVector3d = RealVector3d {
    i: 0.0,
    j: 0.0,
    k: 1.0,
};

/// A frame relative to its other side's, kept to a ragdoll joint's limits:
/// its forward (the twist axis) brought within the plane range off the
/// plane square to left, then within the cone about forward; its twist about
/// forward clamped.
fn hold_ragdoll(
    relative: RealQuaternion,
    twist: [f32; 2],
    cone: f32,
    plane: [f32; 2],
) -> RealQuaternion {
    let direction = relative * FORWARD;
    let swing = RealQuaternion::shortest_arc(FORWARD, direction);
    let twisted = (swing.conjugate() * relative).normalized();
    let twist_angle = (2.0 * twisted.i.atan2(twisted.w) + std::f32::consts::PI)
        .rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    let twist_angle = twist_angle.clamp(twist[0], twist[1]);
    // Off the plane square to left: the angle toward left.
    let [x, y, z] = [direction.i, direction.j, direction.k];
    let elevation = y.clamp(-1.0, 1.0).asin().clamp(plane[0], plane[1]);
    let flat = (x * x + z * z).sqrt();
    let (fx, fz) = if flat > 1e-6 {
        (x / flat, z / flat)
    } else {
        (1.0, 0.0)
    };
    let mut held = [fx * elevation.cos(), elevation.sin(), fz * elevation.cos()];
    // Within the cone about forward.
    let off = held[0].clamp(-1.0, 1.0).acos();
    if off > cone {
        let side = [0.0, held[1], held[2]];
        let side_length = norm(side);
        held = if side_length > 1e-6 {
            [
                cone.cos(),
                side[1] / side_length * cone.sin(),
                side[2] / side_length * cone.sin(),
            ]
        } else {
            [1.0, 0.0, 0.0]
        };
    }
    let held = RealVector3d {
        i: held[0],
        j: held[1],
        k: held[2],
    };
    let swing = RealQuaternion::shortest_arc(FORWARD, held.normalized());
    (swing * about(FORWARD, twist_angle)).normalized()
}

/// A frame relative to its other side's, kept to a limited hinge: only its
/// turn about up, measured from left, within the range.
fn hold_hinge(relative: RealQuaternion, angle: [f32; 2]) -> RealQuaternion {
    let left = relative * LEFT;
    let turned = left.j.atan2(left.i) - LEFT.j.atan2(LEFT.i);
    let turned =
        (turned + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    about(UP, turned.clamp(angle[0], angle[1]))
}

/// The turn by `angle` radians about the unit `axis`.
fn about(axis: RealVector3d, angle: f32) -> RealQuaternion {
    let (sin, cos) = (angle * 0.5).sin_cos();
    RealQuaternion {
        i: axis.i * sin,
        j: axis.j * sin,
        k: axis.k * sin,
        w: cos,
    }
}

/// The turn about `pivot` that carries `from` toward `to` across the
/// sphere about it: what a joint dragged from `from` to `to` turns its
/// parent at `pivot` by. None where either lies on the pivot.
pub(crate) fn swing(pivot: [f32; 3], from: [f32; 3], to: [f32; 3]) -> Option<RealQuaternion> {
    let vector = |point: [f32; 3]| RealVector3d {
        i: point[0] - pivot[0],
        j: point[1] - pivot[1],
        k: point[2] - pivot[2],
    };
    let (from, to) = (vector(from), vector(to));
    if from.length() < 1e-6 || to.length() < 1e-6 {
        return None;
    }
    Some(RealQuaternion::shortest_arc(
        from.normalized(),
        to.normalized(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::math::RealPoint3d;

    /// A point as the math types take it.
    fn point(position: [f32; 3]) -> RealPoint3d {
        RealPoint3d {
            x: position[0],
            y: position[1],
            z: position[2],
        }
    }

    /// A chain of three nodes a unit apart along x, as bind matrices.
    fn chain() -> (Vec<RenderModelPreviewNode>, Vec<Matrix4>) {
        let nodes: Vec<RenderModelPreviewNode> = (0..3)
            .map(|index| RenderModelPreviewNode {
                name: format!("node {index}"),
                parent: index as i16 - 1,
                ..Default::default()
            })
            .collect();
        let matrices = (0..3)
            .map(|index| {
                Matrix4::from_loc_rot_scale(
                    point([index as f32, 0.0, 0.0]),
                    RealQuaternion::IDENTITY,
                    1.0,
                )
            })
            .collect();
        (nodes, matrices)
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-4)
    }

    /// Turning the middle node a quarter about up swings the tip round it
    /// and keeps every bone's length; the root stays.
    #[test]
    fn a_turn_swings_what_hangs_below_and_keeps_lengths() {
        let (nodes, mut matrices) = chain();
        let mut edit = PoseEdit::default();
        let quarter = swing([1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]).unwrap();
        edit.turn_by(1, Some(&matrices[0].clone()), quarter);
        apply(&nodes, &mut matrices, &edit);
        assert!(near(matrices[0].translation(), [0.0; 3]));
        assert!(near(matrices[1].translation(), [1.0, 0.0, 0.0]));
        assert!(
            near(matrices[2].translation(), [1.0, 1.0, 0.0]),
            "{:?}",
            matrices[2].translation()
        );
    }

    /// A turn is kept in the parent's frame, so it rides along when the
    /// parent turns beneath it, as over an animation.
    #[test]
    fn a_turn_rides_its_parent() {
        let (nodes, bind) = chain();
        let mut edit = PoseEdit::default();
        let quarter = swing([1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]).unwrap();
        edit.turn_by(1, Some(&bind[0].clone()), quarter);
        // Beneath, the root turns a quarter about up too.
        let root_turn = swing([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let mut matrices: Vec<Matrix4> = bind.clone();
        let mut animated = PoseEdit::default();
        animated.turn_by(0, None, root_turn);
        apply(&nodes, &mut matrices, &animated);
        apply(&nodes, &mut matrices, &edit);
        assert!(near(matrices[1].translation(), [0.0, 1.0, 0.0]));
        assert!(
            near(matrices[2].translation(), [-1.0, 1.0, 0.0]),
            "{:?}",
            matrices[2].translation()
        );
    }

    /// Another model's nodes start unposed; the same one keeps its pose.
    #[test]
    fn a_pose_belongs_to_its_geometry() {
        let mut edit = PoseEdit {
            on: true,
            geometry: 1,
            ..Default::default()
        };
        edit.turns.insert(0, RealQuaternion::IDENTITY);
        edit.follow(1);
        assert!(edit.is_posed());
        edit.follow(2);
        assert!(!edit.is_posed() && edit.on);
    }

    /// With `BLAM_TEST_H3EK`: the Chief's arm dragged up, unanimated. Every
    /// bone keeps its length, the hand moves, what lies above the turned
    /// node stays, and the mesh is skinned to the pose.
    #[test]
    fn a_dragged_arm_keeps_every_bone_and_skins_the_mesh() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root};
        use crate::app::model_preview::{
            ModelPreviewState, animation_skinning_rows, armature_node_matrices,
        };
        use crate::core::game::GameId;
        let Some(root) = kit_root(GameId::Halo3) else {
            return;
        };
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let mut state = ModelPreviewState::default();
        state.pose.follow(data.geometry_id);
        let nodes = &data.preview.nodes;
        let find = |name: &str| nodes.iter().position(|node| node.name == name).unwrap();
        let (upperarm, hand) = (find("r_upperarm"), find("r_hand"));
        let bind = armature_node_matrices(&data, &state);
        assert!(
            animation_skinning_rows(&data, &state).is_none(),
            "unposed, the bind pose"
        );
        let parent = usize::try_from(nodes[upperarm].parent)
            .ok()
            .map(|parent| bind[parent].clone());
        let pivot = bind[upperarm].translation();
        let elbow = bind[find("r_forearm")].translation();
        let raised = [elbow[0], elbow[1], elbow[2] + 0.2];
        state.pose.turn_by(
            upperarm,
            parent.as_ref(),
            swing(pivot, elbow, raised).unwrap(),
        );
        let posed = armature_node_matrices(&data, &state);
        let length = |matrices: &[Matrix4], index: usize| {
            let parent = nodes[index].parent as usize;
            let (a, b) = (
                matrices[index].translation(),
                matrices[parent].translation(),
            );
            (0..3)
                .map(|axis| (a[axis] - b[axis]).powi(2))
                .sum::<f32>()
                .sqrt()
        };
        for index in (0..nodes.len()).filter(|&index| nodes[index].parent >= 0) {
            assert!(
                (length(&bind, index) - length(&posed, index)).abs() < 1e-4,
                "{} stretched",
                nodes[index].name
            );
        }
        assert!(
            !near(bind[hand].translation(), posed[hand].translation()),
            "the hand moved"
        );
        assert!(near(
            bind[find("pelvis")].translation(),
            posed[find("pelvis")].translation()
        ));
        assert!(
            animation_skinning_rows(&data, &state).is_some(),
            "the mesh is skinned to the pose"
        );
    }

    /// A straight chain of `count` nodes a unit apart along x.
    fn line(count: usize) -> (Vec<RenderModelPreviewNode>, Vec<Matrix4>) {
        let nodes = (0..count)
            .map(|index| RenderModelPreviewNode {
                name: format!("node {index}"),
                parent: index as i16 - 1,
                ..Default::default()
            })
            .collect();
        let matrices = (0..count)
            .map(|index| {
                Matrix4::from_loc_rot_scale(
                    point([index as f32, 0.0, 0.0]),
                    RealQuaternion::IDENTITY,
                    1.0,
                )
            })
            .collect();
        (nodes, matrices)
    }

    fn lengths(nodes: &[RenderModelPreviewNode], matrices: &[Matrix4]) -> Vec<f32> {
        (1..nodes.len())
            .map(|index| {
                distance(
                    matrices[index].translation(),
                    matrices[index - 1].translation(),
                )
            })
            .collect()
    }

    /// One drag event takes a chain's tip to a target within its reach, and
    /// every bone keeps its length.
    #[test]
    fn a_chain_reaches_a_target_in_reach_and_keeps_its_bones() {
        let (nodes, base) = line(4);
        let mut edit = PoseEdit::default();
        let chain = chain_above(&nodes, 3, 3);
        assert_eq!(chain, [2, 1, 0]);
        let target = [1.5, 1.6, 0.4];
        reach(&nodes, &base, &mut edit, 3, &chain, target, &[]);
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        assert!(
            distance(posed[3].translation(), target) < 1e-3,
            "{:?}",
            posed[3].translation()
        );
        for length in lengths(&nodes, &posed) {
            assert!((length - 1.0).abs() < 1e-4, "{length}");
        }
        assert!(
            near(posed[0].translation(), [0.0; 3]),
            "the chain's root stays"
        );
    }

    /// A target past the chain's reach straightens it toward the target at
    /// once.
    #[test]
    fn a_target_out_of_reach_leaves_the_chain_pointing_at_it() {
        let (nodes, base) = line(3);
        let mut edit = PoseEdit::default();
        let chain = chain_above(&nodes, 2, 2);
        let target = [0.0, 10.0, 0.0];
        reach(&nodes, &base, &mut edit, 2, &chain, target, &[]);
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        let tip = posed[2].translation();
        assert!(tip.iter().all(|value| value.is_finite()));
        assert!(near(tip, [0.0, 2.0, 0.0]), "{tip:?}");
    }

    /// A chain asked longer than the joint's ancestors stops at the root.
    #[test]
    fn a_chain_stops_at_the_root() {
        let (nodes, _) = line(3);
        assert_eq!(chain_above(&nodes, 2, 9), [1, 0]);
        assert!(chain_above(&nodes, 0, 2).is_empty());
    }

    /// With `BLAM_TEST_H3EK`: the Chief's right hand dragged forward with
    /// its default chain reaches the spot, the upper arm and forearm
    /// bending, nothing stretching, the pelvis where it was.
    #[test]
    fn the_chiefs_hand_reaches_with_its_arm() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root};
        use crate::app::model_preview::{ModelPreviewState, armature_node_matrices, pose_beneath};
        use crate::core::game::GameId;
        let Some(root) = kit_root(GameId::Halo3) else {
            return;
        };
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let mut state = ModelPreviewState::default();
        state.pose.follow(data.geometry_id);
        let nodes = &data.preview.nodes;
        let find = |name: &str| nodes.iter().position(|node| node.name == name).unwrap();
        let hand = find("r_hand");
        let base = pose_beneath(&data, &state);
        let chain = chain_above(nodes, hand, DEFAULT_CHAIN);
        assert_eq!(chain, [find("r_forearm"), find("r_upperarm")]);
        let start = base[hand].translation();
        let shoulder = base[find("r_upperarm")].translation();
        // Halfway in toward the shoulder and up: in reach, the elbow bending.
        let target = [
            0.5 * (start[0] + shoulder[0]),
            0.5 * (start[1] + shoulder[1]),
            shoulder[2],
        ];
        reach(nodes, &base, &mut state.pose, hand, &chain, target, &[]);
        let posed = armature_node_matrices(&data, &state);
        assert!(distance(posed[hand].translation(), target) < 1e-3);
        let length = |matrices: &[Matrix4], index: usize| {
            distance(
                matrices[index].translation(),
                matrices[nodes[index].parent as usize].translation(),
            )
        };
        for index in (0..nodes.len()).filter(|&index| nodes[index].parent >= 0) {
            assert!(
                (length(&base, index) - length(&posed, index)).abs() < 1e-4,
                "{} stretched",
                nodes[index].name
            );
        }
        assert!(near(
            base[find("pelvis")].translation(),
            posed[find("pelvis")].translation()
        ));
    }

    fn degrees(value: f32) -> f32 {
        value.to_radians()
    }

    /// A hinge keeps only its turn about up, within its range: a turn past
    /// the range stops at its end, and a turn about another axis is taken
    /// out.
    #[test]
    fn a_hinge_turns_only_about_up_and_within_range() {
        let range = [degrees(-15.0), degrees(75.0)];
        let past = about(UP, degrees(100.0));
        let held = hold_hinge(past, range);
        assert!((held.w - about(UP, degrees(75.0)).w).abs() < 1e-5 && held.k > 0.0);
        let sideways = (about(FORWARD, degrees(30.0)) * about(UP, degrees(20.0))).normalized();
        let held = hold_hinge(sideways, range);
        let up = held * UP;
        assert!(
            (up.k - 1.0).abs() < 1e-5,
            "the hinge's axis stays put: {up:?}"
        );
    }

    /// A ragdoll joint's twist axis stays within its cone, and its twist
    /// within range; a turn inside both is left alone.
    #[test]
    fn a_ragdoll_joint_keeps_to_its_cone_and_twist() {
        let (twist, cone, plane) = (
            [degrees(-15.0), degrees(15.0)],
            degrees(50.0),
            [degrees(-40.0), degrees(40.0)],
        );
        let inside = (about(UP, degrees(20.0)) * about(FORWARD, degrees(5.0))).normalized();
        let held = hold_ragdoll(inside, twist, cone, plane);
        assert!(
            held.dot(inside).abs() > 1.0 - 1e-5,
            "inside its limits, untouched"
        );
        // Swung about left, square to the plane range: the cone stops it.
        let swung = about(LEFT, degrees(80.0));
        let direction = hold_ragdoll(swung, twist, cone, plane) * FORWARD;
        assert!(
            (direction.i.acos() - cone).abs() < 1e-3,
            "swung to the cone's edge"
        );
        let twisted = about(FORWARD, degrees(60.0));
        let held = hold_ragdoll(twisted, twist, cone, plane);
        assert!(
            (2.0 * held.i.atan2(held.w) - degrees(15.0)).abs() < 1e-3,
            "twist stops at its end"
        );
        let toward_left = RealQuaternion::shortest_arc(
            FORWARD,
            RealVector3d {
                i: 0.5,
                j: 0.8,
                k: 0.0,
            }
            .normalized(),
        );
        let direction = hold_ragdoll(toward_left, twist, degrees(80.0), plane) * FORWARD;
        assert!(
            (direction.j.asin() - degrees(40.0)).abs() < 1e-3,
            "off the plane no further than its range"
        );
    }

    /// With `BLAM_TEST_H3EK`: the Chief's physics model limits his head,
    /// shoulders and hips as ragdoll joints and his elbows and knees as
    /// hinges, every node found in his render model; dragged far round, his
    /// right elbow bends no further than its hinge allows.
    #[test]
    fn the_chiefs_elbow_keeps_to_its_hinge() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root, kit_source};
        use crate::app::model_preview::load::TagReader;
        use crate::app::model_preview::{ModelPreviewState, armature_node_matrices, pose_beneath};
        use crate::core::game::GameId;
        let (Some(root), Some(source)) = (kit_root(GameId::Halo3), kit_source(GameId::Halo3))
        else {
            return;
        };
        let reader = TagReader::new(&source);
        let physics = reader
            .load(
                "objects\\characters\\masterchief\\masterchief",
                "physics_model",
                b"phmo",
            )
            .unwrap();
        let limits = read_joint_limits(&physics);
        let count = |hinge: bool| {
            limits
                .iter()
                .filter(|limit| matches!(limit.kind, LimitKind::Hinge { .. }) == hinge)
                .count()
        };
        assert_eq!((count(false), count(true)), (5, 4));
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let nodes = &data.preview.nodes;
        let resolved = resolve_limits(nodes, &limits);
        assert_eq!(
            resolved.len(),
            9,
            "every limit's nodes are the render model's"
        );
        let find = |name: &str| nodes.iter().position(|node| node.name == name).unwrap();
        let (hand, forearm) = (find("r_hand"), find("r_forearm"));
        let elbow = resolved
            .iter()
            .find(|limit| limit.b == forearm)
            .unwrap()
            .clone();
        let LimitKind::Hinge { angle } = elbow.kind else {
            panic!("the elbow is a hinge")
        };
        let mut state = ModelPreviewState::default();
        state.pose.follow(data.geometry_id);
        let base = pose_beneath(&data, &state);
        let chain = chain_above(nodes, hand, DEFAULT_CHAIN);
        let shoulder = base[find("r_upperarm")].translation();
        // Right into the shoulder: an unlimited arm folds the hand onto it.
        for _ in 0..4 {
            reach(
                nodes,
                &base,
                &mut state.pose,
                hand,
                &chain,
                shoulder,
                &resolved,
            );
        }
        let posed = armature_node_matrices(&data, &state);
        let frame = |node: usize, frame: RealQuaternion| posed[node].decompose().1 * frame;
        let relative = (frame(elbow.a, elbow.frame_a).conjugate() * frame(forearm, elbow.frame_b))
            .normalized();
        let left = relative * LEFT;
        let bent = left.j.atan2(left.i) - std::f32::consts::FRAC_PI_2;
        assert!(
            bent >= angle[0] - 1e-3 && bent <= angle[1] + 1e-3,
            "{} outside {angle:?}",
            bent.to_degrees()
        );
        assert!(
            distance(posed[hand].translation(), shoulder) > 0.01,
            "the hand stops short of folding onto the shoulder"
        );
    }
}
