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
//! camera, turning the chain of nodes above it, solved by FABRIK. Dragging
//! the root moves the whole skeleton; Alt-dragging a joint twists it about
//! its bone. A pinned joint stays where it was pinned, its own chain turned
//! to hold it there whatever else moves.
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
use std::collections::{BTreeMap, BTreeSet};

/// The hand pose, and the posing under way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PoseEdit {
    /// Whether a drag on a joint poses it rather than orbiting the camera.
    pub on: bool,
    /// Each turned node's turn, in its parent's frame.
    pub turns: BTreeMap<usize, RealQuaternion>,
    /// Each scaled node's scale, over the scale beneath: one factor for all
    /// three axes, as a Halo node transform (and its animations) carries.
    /// It scales the node and everything below it.
    pub scales: BTreeMap<usize, f32>,
    /// The joint last picked, the active one, which carries the gizmo.
    pub selected: Option<usize>,
    /// The other joints selected with it (Shift+click), which the gizmo's
    /// drags take along. See [`PoseEdit::joints`].
    pub selection: BTreeSet<usize>,
    /// The drag under way: the joint dragged.
    pub dragging: Option<PoseDrag>,
    /// How many nodes above a dragged joint turn: its parent alone at 1.
    pub chain: usize,
    /// Whether the physics model's joint limits hold.
    pub limits: bool,
    /// What the selected joint's gizmo does.
    pub gizmo: GizmoMode,
    /// Whether the gizmo runs along the active joint's own axes, as it is
    /// posed, rather than the model's.
    pub local: bool,
    /// Each pinned joint and where it was pinned, in model space.
    pub pins: BTreeMap<usize, [f32; 3]>,
    /// How far the skeleton's roots are moved, in model space.
    pub offset: [f32; 3],
    /// The geometry the turns are for (`ModelPreviewData::geometry_id`);
    /// another model's nodes start unposed.
    pub geometry: u64,
}

/// A joint being dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PoseDrag {
    pub joint: usize,
    /// Twisting it about its bone, rather than moving it.
    pub twist: bool,
    /// The gizmo handle it was taken by: none drags it across the plane
    /// facing the camera.
    pub lock: Option<Lock>,
    /// Where the joint stood when the drag began, which a lock's line or
    /// plane runs through.
    pub anchor: [f32; 3],
    /// Where a locked drag last brought the joint, on its line or plane.
    pub held: [f32; 3],
    /// The rotate gizmo's ring it was taken by, which turns it instead.
    pub ring: Option<RingGrab>,
    /// The scale gizmo's circle it was taken by, which scales it instead.
    pub scale: Option<ScaleGrab>,
    /// The gizmo's axes as the drag began, which a lock keeps to while the
    /// joint moves under them.
    pub axes: Axes,
    /// How far from the joint, along a lock's line or plane, the press took
    /// the handle: the joint keeps that far behind the pointer, rather than
    /// jumping to it.
    pub grabbed: [f32; 3],
}

/// The scale gizmo's circle taken by a drag: the joint scales by the
/// pointer's distance from the joint on screen over its distance there at
/// the press, outward growing, inward shrinking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScaleGrab {
    /// The joint on screen at the press.
    pub centre: [f32; 2],
    /// The pointer's distance from it at the press, in points.
    pub press: f32,
    /// The joint's scale at the press.
    pub from: f32,
}

impl ScaleGrab {
    /// The scale the drag asks for with the pointer at `pointer`, held
    /// within [`SCALE_RANGE`].
    pub(crate) fn scale(&self, pointer: [f32; 2]) -> f32 {
        let distance = (pointer[0] - self.centre[0]).hypot(pointer[1] - self.centre[1]);
        (self.from * distance / self.press.max(1.0)).clamp(SCALE_RANGE[0], SCALE_RANGE[1])
    }
}

/// The least and most a joint scales by hand: small enough to all but hide
/// a part, never to nothing, whose inverse the skinning can't take.
pub(crate) const SCALE_RANGE: [f32; 2] = [0.05, 20.0];

/// A rotate gizmo ring taken by a drag: the joint turns about its axis as
/// the pointer runs along the ring from where it was taken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RingGrab {
    /// The model axis it turns about: 0 x, 1 y, 2 z.
    pub axis: usize,
    /// Where the pointer took the ring, on screen.
    pub press: [f32; 2],
    /// The ring's way round on screen at that point, unit length: the
    /// pointer's run along it, over the ring's radius, is the turn.
    pub tangent: [f32; 2],
    /// The ring's radius on screen, in points.
    pub radius: f32,
    /// How far the drag has turned the joint so far, in radians.
    pub turned: f32,
}

impl RingGrab {
    /// The turn the drag asks for with the pointer at `pointer`, all told.
    pub(crate) fn angle(&self, pointer: [f32; 2]) -> f32 {
        let run = (pointer[0] - self.press[0]) * self.tangent[0]
            + (pointer[1] - self.press[1]) * self.tangent[1];
        run / self.radius.max(1.0)
    }
}

/// What the selected joint's gizmo does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GizmoMode {
    /// Arrows and squares that move it.
    #[default]
    Move,
    /// Rings that turn it.
    Rotate,
    /// A circle that scales it, and what hangs below it.
    Scale,
}

/// The gizmo's three axes, x, y and z, each a unit vector in model space.
pub(crate) type Axes = [[f32; 3]; 3];

/// The model's own axes: x forward, y left, z up.
pub(crate) const GLOBAL_AXES: Axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A node's own axes as it is posed: its world transform's x, y and z,
/// as each node's frame runs forward, left, up.
pub(crate) fn axes_of(matrix: &Matrix4) -> Axes {
    std::array::from_fn(|axis| {
        let column: [f32; 3] = std::array::from_fn(|row| matrix.m[row][axis]);
        let length = norm(column);
        if length < 1e-9 {
            GLOBAL_AXES[axis]
        } else {
            column.map(|value| value / length)
        }
    })
}

/// What a gizmo handle holds a dragged joint to, in model space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lock {
    /// Along one of the gizmo's axes: 0 x, 1 y, 2 z.
    Axis(usize),
    /// Within the plane square to one of them: 2 the xy plane, 1 xz, 0 yz.
    Plane(usize),
}

impl Lock {
    /// The point the pointer's line of sight (`origin`, `direction`) picks
    /// under this lock in `axes`, for a joint that began at `anchor`: the
    /// nearest point of the axis through it, or where the line meets the
    /// plane through it. None where the line runs along the axis or the
    /// plane.
    pub(crate) fn target(
        self,
        anchor: [f32; 3],
        origin: [f32; 3],
        direction: [f32; 3],
        axes: &Axes,
    ) -> Option<[f32; 3]> {
        let dot = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).map(|(a, b)| a * b).sum::<f32>();
        let unit = |axis: usize| axes[axis];
        let from_origin = sub(anchor, origin);
        match self {
            Lock::Axis(axis) => {
                // The nearest points of two lines: the axis and the sight.
                let along = unit(axis);
                let b = dot(along, direction);
                let denominator = 1.0 - b * b;
                if denominator < 1e-6 {
                    return None;
                }
                let t = (b * dot(direction, from_origin) - dot(along, from_origin)) / denominator;
                Some(std::array::from_fn(|index| {
                    anchor[index] + along[index] * t
                }))
            }
            Lock::Plane(normal) => {
                let normal = unit(normal);
                let facing = dot(direction, normal);
                if facing.abs() < 1e-6 {
                    return None;
                }
                let t = dot(from_origin, normal) / facing;
                Some(std::array::from_fn(|index| {
                    origin[index] + direction[index] * t
                }))
            }
        }
    }
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
            scales: BTreeMap::new(),
            selected: None,
            selection: BTreeSet::new(),
            dragging: None,
            chain: DEFAULT_CHAIN,
            limits: true,
            gizmo: GizmoMode::Move,
            local: false,
            pins: BTreeMap::new(),
            offset: [0.0; 3],
            geometry: 0,
        }
    }
}

impl PoseEdit {
    /// Whether any node is turned or scaled, or the skeleton moved, from
    /// the pose beneath.
    pub(crate) fn is_posed(&self) -> bool {
        !self.turns.is_empty() || !self.scales.is_empty() || self.offset != [0.0; 3]
    }

    /// `joint`'s scale over the scale beneath: 1 unscaled.
    pub(crate) fn scale_of(&self, joint: usize) -> f32 {
        self.scales.get(&joint).copied().unwrap_or(1.0)
    }

    /// Scale `joint`, and what hangs below it, to `scale` over the scale
    /// beneath, held within [`SCALE_RANGE`]; back to 1, it is unscaled.
    pub(crate) fn set_scale(&mut self, joint: usize, scale: f32) {
        let scale = scale.clamp(SCALE_RANGE[0], SCALE_RANGE[1]);
        if (scale - 1.0).abs() < 1e-4 {
            self.scales.remove(&joint);
        } else {
            self.scales.insert(joint, scale);
        }
    }

    /// Unposed, for `geometry`'s nodes, if the turns are another model's.
    pub(crate) fn follow(&mut self, geometry: u64) {
        if self.geometry != geometry {
            *self = Self {
                on: self.on,
                chain: self.chain,
                limits: self.limits,
                gizmo: self.gizmo,
                local: self.local,
                geometry,
                ..Self::default()
            };
        }
    }

    /// Every turn, scale, move and pin undone.
    pub(crate) fn reset(&mut self) {
        self.turns.clear();
        self.scales.clear();
        self.pins.clear();
        self.offset = [0.0; 3];
        self.dragging = None;
    }

    /// Every selected joint, the active one among them, in node order.
    pub(crate) fn joints(&self) -> Vec<usize> {
        let mut all = self.selection.clone();
        all.extend(self.selected);
        all.into_iter().collect()
    }

    /// Whether `joint` is selected, active or not.
    pub(crate) fn is_selected(&self, joint: usize) -> bool {
        self.selected == Some(joint) || self.selection.contains(&joint)
    }

    /// `joint` made the active joint, the selection otherwise kept.
    pub(crate) fn activate(&mut self, joint: usize) {
        self.selection.extend(self.selected);
        self.selection.remove(&joint);
        self.selected = Some(joint);
    }

    /// `joint` alone selected, or nothing.
    pub(crate) fn select_only(&mut self, joint: Option<usize>) {
        self.selection.clear();
        self.selected = joint;
    }

    /// Shift+click: `joint` added to the selection and made active, or, if
    /// it was selected, taken out of it; the active joint then falls to the
    /// last of the rest.
    pub(crate) fn toggle_selected(&mut self, joint: usize) {
        if self.is_selected(joint) {
            self.selection.remove(&joint);
            if self.selected == Some(joint) {
                self.selected = self.selection.pop_last();
            }
        } else {
            self.selection.extend(self.selected);
            self.selected = Some(joint);
        }
    }

    /// Shift+double-click: `joint` and every node below it added to the
    /// selection, `joint` active.
    pub(crate) fn select_below(&mut self, nodes: &[RenderModelPreviewNode], joint: usize) {
        self.selection.extend(self.selected);
        for index in 0..nodes.len() {
            if descends(nodes, index, joint) {
                self.selection.insert(index);
            }
        }
        self.selection.remove(&joint);
        self.selected = Some(joint);
    }

    /// Pin `joint` where it stands, or unpin it.
    pub(crate) fn toggle_pin(&mut self, joint: usize, at: [f32; 3]) {
        if self.pins.remove(&joint).is_none() {
            self.pins.insert(joint, at);
        }
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

/// Whether `node` is `ancestor` or hangs anywhere below it.
pub(crate) fn descends(nodes: &[RenderModelPreviewNode], node: usize, ancestor: usize) -> bool {
    let mut at = node;
    // A malformed parent list can't loop: a node's parent comes before it.
    for _ in 0..=nodes.len() {
        if at == ancestor {
            return true;
        }
        match nodes.get(at).and_then(|node| usize::try_from(node.parent).ok()).filter(|&parent| parent < at) {
            Some(parent) => at = parent,
            None => return false,
        }
    }
    false
}

/// The selected joints a move of `active` takes along by the same amount:
/// each not `active`, not above or below it, and not below another of
/// them, which rides along with that one. Moving a joint and one below it
/// both would set their chains fighting over the same bones.
pub(crate) fn moved_with(nodes: &[RenderModelPreviewNode], joints: &[usize], active: usize) -> Vec<usize> {
    joints
        .iter()
        .copied()
        .filter(|&joint| joint != active && !descends(nodes, joint, active) && !descends(nodes, active, joint))
        .filter(|&joint| !joints.iter().any(|&other| other != joint && descends(nodes, joint, other)))
        .collect()
}

/// `matrices`, each node's world transform as the preview poses it, with
/// `edit`'s turns and scales laid over: each node's transform relative to
/// its parent kept, but for its turn and scale, and its parent's posed
/// transform carried down.
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
        // Turned, then scaled about its own origin: its children's offsets
        // scale with it, as an animation's node scale carries down.
        let local = match (edit.turns.get(&index), edit.scales.get(&index)) {
            (None, None) => local,
            (turn, grow) => {
                let (translation, rotation, scale) = local.decompose();
                let rotation = turn.map_or(rotation, |turn| (*turn * rotation).normalized());
                Matrix4::from_loc_rot_scale(translation, rotation, scale * grow.copied().unwrap_or(1.0))
            }
        };
        matrices[index] = match parent {
            Some(parent) => matrices[parent] * local,
            None => moved(local, edit.offset),
        };
    }
}

/// `matrix` moved by `offset` in model space.
fn moved(matrix: Matrix4, offset: [f32; 3]) -> Matrix4 {
    if offset == [0.0; 3] {
        return matrix;
    }
    let (translation, rotation, scale) = matrix.decompose();
    let translation = blam_tags::math::RealPoint3d {
        x: translation.x + offset[0],
        y: translation.y + offset[1],
        z: translation.z + offset[2],
    };
    Matrix4::from_loc_rot_scale(translation, rotation, scale)
}

/// Twist `joint` by `angle` radians about its bone: the line to its first
/// child, or from its parent where it has none. Its limit, if any, holds.
pub(crate) fn twist(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    angle: f32,
    limits: &[ResolvedLimit],
) {
    let mut matrices = base.to_vec();
    apply(nodes, &mut matrices, edit);
    let Some(at) = matrices.get(joint).map(Matrix4::translation) else {
        return;
    };
    let child = nodes
        .iter()
        .position(|node| usize::try_from(node.parent).ok() == Some(joint));
    let parent = usize::try_from(nodes[joint].parent).ok();
    let bone = match (child, parent) {
        (Some(child), _) => sub(matrices[child].translation(), at),
        (None, Some(parent)) => sub(at, matrices[parent].translation()),
        (None, None) => return,
    };
    let length = norm(bone);
    if length < 1e-6 {
        return;
    }
    let axis = RealVector3d {
        i: bone[0] / length,
        j: bone[1] / length,
        k: bone[2] / length,
    };
    edit.turn_by(
        joint,
        parent.and_then(|parent| matrices.get(parent)),
        about(axis, angle),
    );
    for limit in limits.iter().filter(|limit| limit.b == joint) {
        hold(nodes, base, edit, limit);
    }
}

/// Turn `joint` by `angle` radians about `axis`, a unit vector in model
/// space, carrying what hangs below it: the rotate gizmo's ring. Its limit,
/// if any, holds.
pub(crate) fn rotate(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    axis: [f32; 3],
    angle: f32,
    limits: &[ResolvedLimit],
) {
    let mut matrices = base.to_vec();
    apply(nodes, &mut matrices, edit);
    if matrices.get(joint).is_none() {
        return;
    }
    let along = RealVector3d { i: axis[0], j: axis[1], k: axis[2] };
    let parent = usize::try_from(nodes[joint].parent).ok();
    edit.turn_by(
        joint,
        parent.and_then(|parent| matrices.get(parent)),
        about(along, angle),
    );
    for limit in limits.iter().filter(|limit| limit.b == joint) {
        hold(nodes, base, edit, limit);
    }
}

/// Each pin but `except` (the joint being dragged) held where it was pinned:
/// its own chain of [`DEFAULT_CHAIN`] nodes turned to bring it back.
pub(crate) fn hold_pins(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    except: Option<usize>,
    limits: &[ResolvedLimit],
) {
    let pins: Vec<(usize, [f32; 3])> = edit
        .pins
        .iter()
        .filter(|(joint, _)| Some(**joint) != except)
        .map(|(joint, at)| (*joint, *at))
        .collect();
    for (joint, at) in pins {
        let chain = chain_above(nodes, joint, DEFAULT_CHAIN);
        reach(nodes, base, edit, joint, &chain, at, limits);
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
    for limit in &held {
        seed_hinge(nodes, base, edit, limit);
    }
    for _ in 0..rounds {
        place(nodes, base, edit, joint, chain, target);
        for limit in &held {
            hold(nodes, base, edit, limit);
        }
    }
    if !held.is_empty() {
        refine(nodes, base, edit, joint, chain, target, &held);
    }
}

/// Passes of [`refine`] each drag event takes, at most.
const REFINE_PASSES: usize = 8;

/// Finish a limited chain's reach turning each node only as it can turn,
/// nearest first (cyclic coordinate descent): a hinge about its own axis by
/// the angle that best brings `joint` to `target`, any other node by the
/// least swing that points it there, each then held to its limit. FABRIK
/// bends a chain in the plane of its joints, which a hinge's plane need not
/// be; this lands the joint where the hinge lets it.
fn refine(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    chain: &[usize],
    target: [f32; 3],
    held: &[&ResolvedLimit],
) {
    let posed = |edit: &PoseEdit| {
        let mut matrices = base.to_vec();
        apply(nodes, &mut matrices, edit);
        matrices
    };
    let reach_length: f32 = {
        let matrices = posed(edit);
        chain
            .iter()
            .scan(joint, |below, &node| {
                let length = distance(matrices[*below].translation(), matrices[node].translation());
                *below = node;
                Some(length)
            })
            .sum()
    };
    for _ in 0..REFINE_PASSES {
        for &node in chain {
            let matrices = posed(edit);
            let (at, effector) = (matrices[node].translation(), matrices[joint].translation());
            if distance(effector, target) < 1e-5 * reach_length {
                return;
            }
            let limit = held.iter().find(|limit| limit.b == node);
            let turn = match limit.map(|limit| (limit, limit.kind)) {
                Some((limit, LimitKind::Hinge { .. })) => {
                    let axis = (matrices[limit.a].decompose().1 * limit.frame_a) * UP;
                    let axis = [axis.i, axis.j, axis.k];
                    let flat = |point: [f32; 3]| {
                        let offset = sub(point, at);
                        let along = offset.iter().zip(&axis).map(|(a, b)| a * b).sum::<f32>();
                        std::array::from_fn::<f32, 3, _>(|index| {
                            offset[index] - axis[index] * along
                        })
                    };
                    let (from, to) = (flat(effector), flat(target));
                    if norm(from) < 1e-6 || norm(to) < 1e-6 {
                        continue;
                    }
                    let cross = [
                        from[1] * to[2] - from[2] * to[1],
                        from[2] * to[0] - from[0] * to[2],
                        from[0] * to[1] - from[1] * to[0],
                    ];
                    let sine = cross.iter().zip(&axis).map(|(a, b)| a * b).sum::<f32>();
                    let cosine = from.iter().zip(&to).map(|(a, b)| a * b).sum::<f32>();
                    let axis = RealVector3d {
                        i: axis[0],
                        j: axis[1],
                        k: axis[2],
                    };
                    Some(about(axis, sine.atan2(cosine)))
                }
                _ => swing(at, effector, target),
            };
            let Some(turn) = turn else {
                continue;
            };
            let parent = usize::try_from(nodes[node].parent)
                .ok()
                .and_then(|parent| matrices.get(parent).copied());
            edit.turn_by(node, parent.as_ref(), turn);
            if let Some(limit) = limit {
                hold(nodes, base, edit, limit);
            }
        }
    }
}

/// Trials of [`reach_held`]'s search along the way to a target out of
/// reach.
const HELD_TRIALS: usize = 12;

/// [`reach`] for a joint held to a gizmo's axis or plane: `from` (where
/// the joint was last brought, on the line) and `target` both lie on it, and
/// the joint goes no further from `from` toward `target` than the chain (and
/// its limits) can take it exactly. Where [`reach`] would point the chain at
/// a target out of reach, which carries the joint off the line, this stops it
/// at the furthest point toward the target it lands on. The point it was
/// brought to, for the next step to search from.
pub(crate) fn reach_held(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    joint: usize,
    chain: &[usize],
    (from, target): ([f32; 3], [f32; 3]),
    limits: &[ResolvedLimit],
) -> [f32; 3] {
    let at = |edit: &PoseEdit| {
        let mut matrices = base.to_vec();
        apply(nodes, &mut matrices, edit);
        matrices
    };
    let matrices = at(edit);
    if matrices.get(joint).is_none() {
        return from;
    }
    let reach_length: f32 = chain
        .iter()
        .scan(joint, |below, &node| {
            let length = distance(matrices[*below].translation(), matrices[node].translation());
            *below = node;
            Some(length)
        })
        .sum();
    let tolerance = 2e-3 * reach_length.max(1e-3);
    let lands = |goal: [f32; 3]| {
        let mut trial = edit.clone();
        reach(nodes, base, &mut trial, joint, chain, goal, limits);
        let landed = at(&trial)[joint].translation();
        (distance(landed, goal) <= tolerance).then_some(trial)
    };
    if let Some(trial) = lands(target) {
        *edit = trial;
        return target;
    }
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    let mut best = None;
    for _ in 0..HELD_TRIALS {
        let middle = 0.5 * (low + high);
        let goal = std::array::from_fn(|axis| from[axis] + (target[axis] - from[axis]) * middle);
        match lands(goal) {
            Some(trial) => {
                best = Some((trial, goal));
                low = middle;
            }
            None => high = middle,
        }
    }
    match best {
        Some((trial, goal)) => {
            *edit = trial;
            goal
        }
        None => from,
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
    // Off the plane square to left. Havok measures the angle away from left,
    // the other way to the angle toward it, as with the hinge: in its terms
    // the game's negated range lets each of the Chief's legs splay outward
    // but not cross the other, and each arm lift away from his body.
    let [x, y, z] = [direction.i, direction.j, direction.k];
    let elevation = -(-y.clamp(-1.0, 1.0).asin()).clamp(plane[0], plane[1]);
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

/// How far into its larger side a limited hinge is bent before a chain
/// through it is placed.
const HINGE_SEED: f32 = 5.0 * std::f32::consts::PI / 180.0;

/// A limited hinge bent a little into its larger side (the way the joint
/// bends) if it stands straight or bent the other way: FABRIK bends a chain
/// the way it is already bent, so a knee a hair back in its bind pose would
/// otherwise fold backward against its limit.
fn seed_hinge(
    nodes: &[RenderModelPreviewNode],
    base: &[Matrix4],
    edit: &mut PoseEdit,
    limit: &ResolvedLimit,
) {
    let LimitKind::Hinge { angle } = limit.kind else {
        return;
    };
    let side = if angle[1].abs() >= angle[0].abs() {
        angle[1].signum()
    } else {
        angle[0].signum()
    };
    let mut matrices = base.to_vec();
    apply(nodes, &mut matrices, edit);
    let (Some(a), Some(b)) = (matrices.get(limit.a), matrices.get(limit.b)) else {
        return;
    };
    let turn_a = a.decompose().1 * limit.frame_a;
    let frame_b = b.decompose().1 * limit.frame_b;
    let relative = (turn_a.conjugate() * frame_b).normalized();
    if hinge_angle(relative) * side >= HINGE_SEED {
        return;
    }
    let wanted = (turn_a * about(UP, -side * HINGE_SEED)).normalized();
    let turn = (wanted * frame_b.conjugate()).normalized();
    let parent = usize::try_from(nodes[limit.b].parent)
        .ok()
        .and_then(|parent| matrices.get(parent).copied());
    edit.turn_by(limit.b, parent.as_ref(), turn);
}

/// A frame relative to its other side's, kept to a limited hinge: only its
/// turn about up, measured from left, within the range.
fn hold_hinge(relative: RealQuaternion, angle: [f32; 2]) -> RealQuaternion {
    about(UP, -hinge_angle(relative).clamp(angle[0], angle[1]))
}

/// A limited hinge's angle as Havok measures it: the turn of B's left from
/// A's, the other way about up from the right hand. The game negates the
/// tag's range for it, so a bend the tag's range allows is allowed here:
/// the Chief's elbow and knee bend to their tags' 75° and 60°, the way they
/// bend, and 15° the other.
fn hinge_angle(relative: RealQuaternion) -> f32 {
    let left = relative * LEFT;
    let turned = left.j.atan2(left.i) - LEFT.j.atan2(LEFT.i);
    -((turned + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI)
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

    /// A node turned a quarter about up has its x along the model's y and
    /// its y along the model's −x; an axis lock in its axes runs along them.
    #[test]
    fn local_axes_are_the_nodes_own_and_a_lock_keeps_to_them() {
        let quarter = RealQuaternion { i: 0.0, j: 0.0, k: std::f32::consts::FRAC_1_SQRT_2, w: std::f32::consts::FRAC_1_SQRT_2 };
        let matrix = Matrix4::from_loc_rot_scale(point([1.0, 2.0, 3.0]), quarter, 2.0);
        let axes = axes_of(&matrix);
        assert!(near(axes[0], [0.0, 1.0, 0.0]) && near(axes[1], [-1.0, 0.0, 0.0]) && near(axes[2], [0.0, 0.0, 1.0]), "{axes:?}");
        // Looking down from above at a point off to the side: the lock
        // along local x picks a point on the model's y line through it.
        let target = Lock::Axis(0).target([0.0; 3], [0.3, 0.5, 5.0], [0.0, 0.0, -1.0], &axes).unwrap();
        assert!(near(target, [0.0, 0.5, 0.0]), "{target:?}");
    }

    /// Shift+click adds a joint and makes it active, and again takes it
    /// out, the active falling to another; Shift+double-click takes a
    /// joint and all below it; a plain click selects one alone.
    #[test]
    fn the_selection_adds_takes_out_and_takes_all_below() {
        let (nodes, _) = chain();
        let mut edit = PoseEdit::default();
        edit.select_only(Some(0));
        edit.toggle_selected(2);
        assert_eq!((edit.selected, edit.joints()), (Some(2), vec![0, 2]));
        edit.toggle_selected(2);
        assert_eq!((edit.selected, edit.joints()), (Some(0), vec![0]));
        edit.select_only(None);
        edit.select_below(&nodes, 1);
        assert_eq!((edit.selected, edit.joints()), (Some(1), vec![1, 2]));
        edit.activate(2);
        assert_eq!((edit.selected, edit.joints()), (Some(2), vec![1, 2]));
        edit.select_only(Some(0));
        assert_eq!(edit.joints(), vec![0]);
    }

    /// A move of one joint takes along the others selected, but none above
    /// or below it, and of two on one limb only the upper.
    #[test]
    fn a_move_takes_along_the_others_but_none_on_its_own_limb() {
        // 0 the root; 1 → 2 one limb, 3 → 4 another.
        let parents = [-1, 0, 1, 0, 3];
        let nodes: Vec<RenderModelPreviewNode> = parents
            .iter()
            .enumerate()
            .map(|(index, &parent)| RenderModelPreviewNode { name: format!("node {index}"), parent, ..Default::default() })
            .collect();
        assert!(descends(&nodes, 2, 1) && descends(&nodes, 2, 0) && !descends(&nodes, 2, 3));
        assert_eq!(moved_with(&nodes, &[2, 4], 2), vec![4]);
        assert_eq!(moved_with(&nodes, &[1, 2, 4], 2), vec![4], "not the joint above it");
        assert_eq!(moved_with(&nodes, &[2, 3, 4], 2), vec![3], "the upper of the other limb's two");
    }

    /// Turning a joint and the one below it both by a quarter about up adds
    /// up: the tip swings a half turn round from where it pointed.
    #[test]
    fn nested_turns_add_up() {
        let (nodes, base) = chain();
        let mut edit = PoseEdit::default();
        for joint in [1, 2] {
            rotate(&nodes, &base, &mut edit, joint, GLOBAL_AXES[2], std::f32::consts::FRAC_PI_2, &[]);
        }
        let mut matrices = base.clone();
        apply(&nodes, &mut matrices, &edit);
        assert!(near(matrices[1].translation(), [1.0, 0.0, 0.0]));
        assert!(near(matrices[2].translation(), [1.0, 1.0, 0.0]), "{:?}", matrices[2].translation());
    }

    /// Scaling the middle node scales it and carries down: it stays where
    /// it stands, the bone below it doubles and the tip takes the scale.
    /// Back to 1, nothing is scaled; Reset undoes a scale.
    #[test]
    fn a_scale_carries_down_and_leaves_the_joint_where_it_stands() {
        let (nodes, base) = chain();
        let mut edit = PoseEdit::default();
        edit.set_scale(1, 2.0);
        assert!(edit.is_posed());
        let mut matrices = base.clone();
        apply(&nodes, &mut matrices, &edit);
        assert!(near(matrices[0].translation(), [0.0; 3]));
        assert!(near(matrices[1].translation(), [1.0, 0.0, 0.0]));
        assert!(near(matrices[2].translation(), [3.0, 0.0, 0.0]), "{:?}", matrices[2].translation());
        assert!((matrices[2].decompose().2 - 2.0).abs() < 1e-4);
        edit.set_scale(1, 1.0);
        assert!(!edit.is_posed());
        edit.set_scale(1, 0.0);
        assert_eq!(edit.scale_of(1), SCALE_RANGE[0], "never to nothing");
        edit.reset();
        assert!(edit.scales.is_empty());
    }

    /// A chain below a scaled node reaches with its bones as long as they
    /// now are: a target only the longer bone can reach is reached.
    #[test]
    fn a_scaled_chain_reaches_with_its_scaled_lengths() {
        let (nodes, base) = chain();
        let mut edit = PoseEdit::default();
        edit.set_scale(1, 2.0);
        // 2.6 from the root: past the unscaled reach of 2, in the scaled 3.
        let target = [1.8, 1.8, 0.4];
        reach(&nodes, &base, &mut edit, 2, &[1, 0], target, &[]);
        let mut matrices = base.clone();
        apply(&nodes, &mut matrices, &edit);
        assert!(distance(matrices[2].translation(), target) < 1e-3, "{:?}", matrices[2].translation());
        assert!((distance(matrices[1].translation(), matrices[2].translation()) - 2.0).abs() < 1e-3);
    }

    /// The scale circle's drag: twice as far from the joint as the press
    /// is twice the scale it began with; in to the joint stops at the least.
    #[test]
    fn the_scale_grab_scales_by_the_pointers_distance() {
        let grab = ScaleGrab { centre: [10.0, 10.0], press: 50.0, from: 1.5 };
        assert!((grab.scale([110.0, 10.0]) - 3.0).abs() < 1e-5);
        assert!((grab.scale([10.0, 35.0]) - 0.75).abs() < 1e-5);
        assert_eq!(grab.scale([10.0, 10.0]), SCALE_RANGE[0]);
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
        // Havok's angle runs the other way about up: a right-handed 100°
        // turn is -100°, past this range's -15° end.
        let range = [degrees(-15.0), degrees(75.0)];
        let past = about(UP, degrees(100.0));
        let held = hold_hinge(past, range);
        assert!((held.w - about(UP, degrees(15.0)).w).abs() < 1e-5 && held.k > 0.0);
        let bent = about(UP, degrees(-50.0));
        assert!(
            hold_hinge(bent, range).dot(bent).abs() > 1.0 - 1e-5,
            "within range, untouched"
        );
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
        let bent = hinge_angle(relative);
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

    /// Moving the root carries every node with it.
    #[test]
    fn moving_the_root_moves_the_skeleton() {
        let (nodes, base) = line(3);
        let edit = PoseEdit {
            offset: [0.0, 0.0, 0.5],
            ..Default::default()
        };
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        for index in 0..3 {
            assert!(near(posed[index].translation(), [index as f32, 0.0, 0.5]));
        }
    }

    /// A twist spins a joint about its bone: the child on the bone stays,
    /// what lies off it goes round.
    #[test]
    fn a_twist_spins_only_what_lies_off_the_bone() {
        let nodes: Vec<RenderModelPreviewNode> = [-1, 0, 1]
            .iter()
            .enumerate()
            .map(|(index, &parent)| RenderModelPreviewNode {
                name: format!("node {index}"),
                parent,
                ..Default::default()
            })
            .collect();
        // A bend at the middle: the tip off the line from the middle's parent.
        let base: Vec<Matrix4> = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]
            .iter()
            .map(|&at| Matrix4::from_loc_rot_scale(point(at), RealQuaternion::IDENTITY, 1.0))
            .collect();
        let mut edit = PoseEdit::default();
        // The root's bone runs to the middle, along x: the tip swings round x.
        twist(
            &nodes,
            &base,
            &mut edit,
            0,
            std::f32::consts::FRAC_PI_2,
            &[],
        );
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        assert!(
            near(posed[1].translation(), [1.0, 0.0, 0.0]),
            "the child on the bone stays"
        );
        assert!(
            near(posed[2].translation(), [1.0, 0.0, 1.0]),
            "{:?}",
            posed[2].translation()
        );
    }

    /// A pinned joint stays where it was pinned when the root moves.
    #[test]
    fn a_pin_holds_when_the_root_moves() {
        let nodes: Vec<RenderModelPreviewNode> = (0..4)
            .map(|index| RenderModelPreviewNode {
                name: format!("node {index}"),
                parent: index as i16 - 1,
                ..Default::default()
            })
            .collect();
        // Bent, so the chain can take up a move toward the pin.
        let base: Vec<Matrix4> = [
            [0.0, 0.0, 2.0],
            [0.0, 0.5, 1.0],
            [0.0, 0.0, 0.0],
            [0.3, 0.0, 0.0],
        ]
        .iter()
        .map(|&at| Matrix4::from_loc_rot_scale(point(at), RealQuaternion::IDENTITY, 1.0))
        .collect();
        let mut edit = PoseEdit::default();
        edit.toggle_pin(2, [0.0, 0.0, 0.0]);
        edit.offset = [0.0, 0.0, -0.3];
        hold_pins(&nodes, &base, &mut edit, None, &[]);
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        assert!(
            near(posed[0].translation(), [0.0, 0.0, 1.7]),
            "the root moved"
        );
        assert!(
            distance(posed[2].translation(), [0.0, 0.0, 0.0]) < 1e-3,
            "{:?}",
            posed[2].translation()
        );
        edit.toggle_pin(2, [0.0; 3]);
        assert!(edit.pins.is_empty(), "a second toggle unpins");
    }

    /// With `BLAM_TEST_H3EK`: the Chief, his feet pinned, lowered by his
    /// pelvis: his feet stay put, his knees bend within their hinges.
    #[test]
    fn the_chief_crouches_on_pinned_feet() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root, kit_source};
        use crate::app::model_preview::load::TagReader;
        use crate::app::model_preview::{ModelPreviewState, armature_node_matrices, pose_beneath};
        use crate::core::game::GameId;
        let (Some(root), Some(source)) = (kit_root(GameId::Halo3), kit_source(GameId::Halo3))
        else {
            return;
        };
        let physics = TagReader::new(&source)
            .load(
                "objects\\characters\\masterchief\\masterchief",
                "physics_model",
                b"phmo",
            )
            .unwrap();
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let nodes = &data.preview.nodes;
        let limits = resolve_limits(nodes, &read_joint_limits(&physics));
        let find = |name: &str| nodes.iter().position(|node| node.name == name).unwrap();
        let mut state = ModelPreviewState::default();
        state.pose.follow(data.geometry_id);
        let base = pose_beneath(&data, &state);
        let feet = [find("l_foot"), find("r_foot")];
        for foot in feet {
            state.pose.toggle_pin(foot, base[foot].translation());
        }
        state.pose.offset = [0.0, 0.0, -0.05];
        hold_pins(nodes, &base, &mut state.pose, None, &limits);
        let posed = armature_node_matrices(&data, &state);
        for foot in feet {
            assert!(
                distance(posed[foot].translation(), base[foot].translation()) < 2e-3,
                "{} slid",
                nodes[foot].name
            );
        }
        assert!(
            posed[find("pelvis")].translation()[2] < base[find("pelvis")].translation()[2] - 0.04
        );
        for knee in [find("l_calf"), find("r_calf")] {
            let limit = limits.iter().find(|limit| limit.b == knee).unwrap();
            let LimitKind::Hinge { angle } = limit.kind else {
                panic!("a knee is a hinge")
            };
            let frame = |node: usize, frame: RealQuaternion| posed[node].decompose().1 * frame;
            let relative = (frame(limit.a, limit.frame_a).conjugate() * frame(knee, limit.frame_b))
                .normalized();
            let bent = hinge_angle(relative);
            assert!(
                bent >= angle[0] - 1e-3 && bent <= angle[1] + 1e-3,
                "{} at {}",
                nodes[knee].name,
                bent.to_degrees()
            );
        }
    }

    /// An axis lock keeps the target on the axis through the anchor, at the
    /// point nearest the line of sight; a plane lock where the line meets
    /// the plane.
    #[test]
    fn a_lock_holds_the_target_to_its_axis_or_plane() {
        let anchor = [1.0, 2.0, 3.0];
        // Looking down -y from above the anchor, offset in x and z.
        let (origin, direction) = ([1.5, 10.0, 3.25], [0.0, -1.0, 0.0]);
        assert!(near(
            Lock::Axis(0).target(anchor, origin, direction, &GLOBAL_AXES).unwrap(),
            [1.5, 2.0, 3.0]
        ));
        assert!(near(
            Lock::Axis(2).target(anchor, origin, direction, &GLOBAL_AXES).unwrap(),
            [1.0, 2.0, 3.25]
        ));
        assert!(
            Lock::Axis(1).target(anchor, origin, direction, &GLOBAL_AXES).is_none(),
            "along the sight"
        );
        assert!(near(
            Lock::Plane(1).target(anchor, origin, direction, &GLOBAL_AXES).unwrap(),
            [1.5, 2.0, 3.25]
        ));
        assert!(
            Lock::Plane(2).target(anchor, origin, direction, &GLOBAL_AXES).is_none(),
            "edge on"
        );
        let slanted = [0.0, -0.6, -0.8];
        let hit = Lock::Plane(2)
            .target(anchor, [0.0, 6.0, 11.0], slanted, &GLOBAL_AXES)
            .unwrap();
        assert!(
            (hit[2] - 3.0).abs() < 1e-5,
            "on the xy plane through the anchor: {hit:?}"
        );
    }

    /// With `BLAM_TEST_H3EK`: the Chief's left hand held to the up axis and
    /// pulled far above his head stays on the line through where it began,
    /// rising as high as his arm and its limits let it.
    #[test]
    fn a_hand_held_to_an_axis_stays_on_it_past_its_reach() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root, kit_source};
        use crate::app::model_preview::load::TagReader;
        use crate::app::model_preview::{ModelPreviewState, armature_node_matrices, pose_beneath};
        use crate::core::game::GameId;
        let (Some(root), Some(source)) = (kit_root(GameId::Halo3), kit_source(GameId::Halo3))
        else {
            return;
        };
        let physics = TagReader::new(&source)
            .load(
                "objects\\characters\\masterchief\\masterchief",
                "physics_model",
                b"phmo",
            )
            .unwrap();
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let nodes = &data.preview.nodes;
        let limits = resolve_limits(nodes, &read_joint_limits(&physics));
        let hand = nodes.iter().position(|node| node.name == "l_hand").unwrap();
        for limited in [false, true] {
            let mut state = ModelPreviewState::default();
            state.pose.follow(data.geometry_id);
            let base = pose_beneath(&data, &state);
            let start = base[hand].translation();
            let chain = chain_above(nodes, hand, DEFAULT_CHAIN);
            let used: &[ResolvedLimit] = if limited { &limits } else { &[] };
            // Dragged up a step at a time, far past the arm's reach.
            let mut held = start;
            for step in 1..=20 {
                let target = [start[0], start[1], start[2] + 0.05 * step as f32];
                held = reach_held(
                    nodes,
                    &base,
                    &mut state.pose,
                    hand,
                    &chain,
                    (held, target),
                    used,
                );
            }
            let moved = armature_node_matrices(&data, &state)[hand].translation();
            let off = ((moved[0] - start[0]).powi(2) + (moved[1] - start[1]).powi(2)).sqrt();
            assert!(off < 2e-3, "limits {limited}: off the axis by {off}");
            assert!(
                moved[2] > start[2] + 0.05,
                "limits {limited}: it rose: {}",
                moved[2] - start[2]
            );
        }
    }

    /// With `BLAM_TEST_H3EK`: the plane limit's sign, by the Chief's left
    /// leg. His hips' plane ranges are one-sided: swung out to his left it
    /// keeps the swing, swung in across his right leg it is held back to
    /// where it stood. Ragdolls are built so the legs splay but don't cross.
    #[test]
    fn a_leg_splays_outward_but_does_not_cross() {
        use crate::app::model_preview::harness::kit::{kit_preview, kit_root, kit_source};
        use crate::app::model_preview::load::TagReader;
        use crate::app::model_preview::{ModelPreviewState, armature_node_matrices, pose_beneath};
        use crate::core::game::GameId;
        let (Some(root), Some(source)) = (kit_root(GameId::Halo3), kit_source(GameId::Halo3))
        else {
            return;
        };
        let physics = TagReader::new(&source)
            .load(
                "objects\\characters\\masterchief\\masterchief",
                "physics_model",
                b"phmo",
            )
            .unwrap();
        let data = kit_preview(
            &root,
            GameId::Halo3,
            "objects\\characters\\masterchief\\masterchief",
            "render_model",
            b"mode",
        );
        let nodes = &data.preview.nodes;
        let limits = resolve_limits(nodes, &read_joint_limits(&physics));
        let find = |name: &str| nodes.iter().position(|node| node.name == name).unwrap();
        let (thigh, foot) = (find("l_thigh"), find("l_foot"));
        let hip = limits
            .iter()
            .find(|limit| limit.b == thigh)
            .unwrap()
            .clone();
        // The foot's sideways place before and after the thigh is swung
        // `angle` (out to the left for a positive one) and held.
        let swung = |angle: f32| {
            let mut state = ModelPreviewState::default();
            state.pose.follow(data.geometry_id);
            let base = pose_beneath(&data, &state);
            let parent = base[nodes[thigh].parent as usize];
            // About +x: a leg hanging down swings toward +y, the left.
            state
                .pose
                .turn_by(thigh, Some(&parent), about(FORWARD, angle));
            hold(nodes, &base, &mut state.pose, &hip);
            (
                base[foot].translation()[1],
                armature_node_matrices(&data, &state)[foot].translation()[1],
            )
        };
        let (start, out) = swung(degrees(20.0));
        assert!(
            out > start + 0.02,
            "out to the left it keeps its swing: {start} → {out}"
        );
        let (start, inward) = swung(degrees(-20.0));
        assert!(
            inward > start - 0.01,
            "in across the other leg it is held back: {start} → {inward}"
        );
    }

    /// A ring turns its joint about the ring's axis, carrying what hangs
    /// below it, the joint itself staying put.
    #[test]
    fn a_ring_turns_its_joint_about_its_axis() {
        let (nodes, base) = line(3);
        let mut edit = PoseEdit::default();
        rotate(
            &nodes,
            &base,
            &mut edit,
            1,
            GLOBAL_AXES[2],
            std::f32::consts::FRAC_PI_2,
            &[],
        );
        let mut posed = base.clone();
        apply(&nodes, &mut posed, &edit);
        assert!(
            near(posed[1].translation(), [1.0, 0.0, 0.0]),
            "the joint stays"
        );
        assert!(
            near(posed[2].translation(), [1.0, 1.0, 0.0]),
            "{:?}",
            posed[2].translation()
        );
        let grab = RingGrab {
            axis: 2,
            press: [10.0, 10.0],
            tangent: [0.0, -1.0],
            radius: 50.0,
            turned: 0.0,
        };
        assert!(
            (grab.angle([30.0, -15.0]) - 0.5).abs() < 1e-6,
            "a run of half the radius along it is half a radian"
        );
    }
}
