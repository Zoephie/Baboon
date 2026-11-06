//! Reach's NEW blend screens: the aim, look and other pose overlays a
//! data-driven graph lays over an action, each a yaw and pitch turned into up
//! to three weighted frames of an overlay. The preview holds a unit at rest,
//! aiming and looking dead ahead. See `docs/reach-preview-research/blend_screens.md`.

use blam_tags::animation::{Animation, AnimationGroup};
use blam_tags::math::{RealPoint3d, RealQuaternion};
use blam_tags::{NodeTransform, OverlayDelta, Pose, Skeleton, TagFile, TagStruct};

use super::super::common::fields::elements;
use super::super::dx9::passes::tag_function_at;

/// The weights' `EPS`.
const EPS: f32 = 0.0001;
/// `private flags`: the graph mixes on its data-driven board.
const DATA_DRIVEN: i128 = 1 << 3;
/// A screen's `flags`.
const ACTIVE_ONLY_WHEN_WEAPON_DOWN: i128 = 1;
const ATTEMPT_PIECEWISE: i128 = 1 << 1;
/// A blend screen item's `flags`.
const ITEM_DISABLED: i128 = 1;
/// `blend_screen_weight_sources`.
const WEIGHT_NONE: i128 = 0;
const WEIGHT_OBJECT_FUNCTION: i128 = 1;

/// A screen's layout of frames over yaw and pitch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BlendScreenData {
    kind: u8,
    /// Yaw and pitch, radians; point i is frame i.
    points: Vec<[f32; 2]>,
    /// Segments' two vertices and their links, or triangles' three and theirs.
    elements: Vec<[u8; 6]>,
}

impl BlendScreenData {
    pub(crate) fn parse(section: &[u8]) -> Option<Self> {
        let byte = |at: usize| section.get(at).copied();
        let word = |at: usize| Some(u32::from_le_bytes(section.get(at..at + 4)?.try_into().ok()?) as usize);
        let float = |at: usize| Some(f32::from_le_bytes(section.get(at..at + 4)?.try_into().ok()?));
        let (kind, points, count) = (byte(0)?, byte(1)? as usize, byte(2)? as usize);
        let (points_at, elements_at) = (word(4)?, word(8)?);
        let points = (0..points)
            .map(|index| Some([float(points_at + 8 * index)?, float(points_at + 8 * index + 4)?]))
            .collect::<Option<Vec<_>>>()?;
        let stride = if kind == 0 { 4 } else { 8 };
        let elements = (0..count)
            .map(|index| {
                let at = elements_at + stride * index;
                Some(if kind == 0 {
                    [byte(at)?, byte(at + 1)?, 0, byte(at + 2)?, byte(at + 3)?, 0]
                } else {
                    [byte(at)?, byte(at + 1)?, byte(at + 2)?, byte(at + 3)?, byte(at + 4)?, byte(at + 5)?]
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { kind, points, elements })
    }

    /// `set_euler_angles`' bounds: a sphere's all the way round, the
    /// others the reach of their points.
    fn clamped(&self, yaw: f32, pitch: f32) -> (f32, f32) {
        if self.kind == 2 {
            let pi = std::f32::consts::PI;
            return (yaw.clamp(-pi, pi), pitch.clamp(-pi, pi));
        }
        let bound = |axis: usize| {
            let values = self.points.iter().map(|point| point[axis]);
            (values.clone().fold(f32::INFINITY, f32::min), values.fold(f32::NEG_INFINITY, f32::max))
        };
        let ((low_yaw, high_yaw), (low_pitch, high_pitch)) = (bound(0), bound(1));
        if low_yaw > high_yaw {
            return (yaw, pitch);
        }
        (yaw.clamp(low_yaw, high_yaw), pitch.clamp(low_pitch, high_pitch))
    }

    /// `compute_frame_weights_from_blend_screen_data`: interpolation weights
    /// and their frames, and the index the second blend breaks at.
    pub(crate) fn weights(&self, yaw: f32, pitch: f32) -> Option<([f32; 3], [usize; 3], Option<usize>)> {
        let (yaw, pitch) = self.clamped(yaw, pitch);
        match self.kind {
            0 => self.segments(yaw, pitch).map(|(weights, frames)| (weights, frames, None)),
            1 => self.triangulation(yaw, pitch).map(|(weights, frames)| (weights, frames, None)),
            2 => self.sphere(yaw, pitch),
            _ => None,
        }
    }

    fn point(&self, index: u8) -> [f32; 2] {
        self.points.get(index as usize).copied().unwrap_or([0.0; 2])
    }

    /// `compute_frame_weights_from_linear_segments`.
    fn segments(&self, yaw: f32, pitch: f32) -> Option<([f32; 3], [usize; 3])> {
        if self.points.len() == 1 {
            return Some(([1.0, 0.0, 0.0], [0; 3]));
        }
        let (first, last) = (self.elements.first()?, self.elements.last()?);
        let snap = |value: f32| if value.abs() < EPS { 0.0 } else { value };
        let p = [snap(yaw), snap(pitch)];
        let start = self.point(first[0]);
        let end = self.point(last[1]);
        let (q, _) = closest_on_line2(p, start, sub2(end, start));
        let reached = length2_squared(sub2(q, start));
        for segment in &self.elements {
            let (a, b) = (self.point(segment[0]), self.point(segment[1]));
            if length2_squared(sub2(a, start)) <= reached && reached <= length2_squared(sub2(b, start)) {
                let length = length2_squared(sub2(b, a)).sqrt();
                let t = if length > EPS { length2_squared(sub2(p, a)).sqrt() / length } else { 0.0 };
                return Some(([1.0 - t, t, 0.0], [segment[0] as usize, segment[1] as usize, 0]));
            }
        }
        None
    }

    /// `compute_weights_from_raw_triangulation`: the first triangle holding
    /// the point, or the nearest edge of the hull.
    fn triangulation(&self, yaw: f32, pitch: f32) -> Option<([f32; 3], [usize; 3])> {
        let snap = |value: f32| if value.abs() < EPS { 0.0 } else { value };
        let p = [snap(yaw), snap(pitch)];
        let frames = |triangle: &[u8; 6]| [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
        for triangle in &self.elements {
            let [a, b, c] = [0, 1, 2].map(|corner| self.point(triangle[corner]));
            if point_in_triangle2(p, a, b, c) {
                let (u, v, w) = barycentric2(p, a, b, c);
                return Some((to_interpolation(u, v, w), frames(triangle)));
            }
        }
        let mut best: Option<(f32, &[u8; 6], usize)> = None;
        for triangle in &self.elements {
            for edge in 0..3 {
                if triangle[3 + edge] != 0xFF {
                    continue;
                }
                let (a, b) = (self.point(triangle[edge]), self.point(triangle[(edge + 1) % 3]));
                let (q, _) = closest_on_line2(p, a, sub2(b, a));
                let distance = length2_squared(sub2(p, q));
                if best.is_none_or(|(known, _, _)| distance < known) {
                    best = Some((distance, triangle, edge));
                }
            }
        }
        let (_, triangle, edge) = best?;
        let (a, b) = (self.point(triangle[edge]), self.point(triangle[(edge + 1) % 3]));
        let (_, s) = closest_on_line2(p, a, sub2(b, a));
        let mut weights = [0.0; 3];
        weights[(edge + 1) % 3] = s;
        weights[edge] = 1.0 - s;
        Some((weights, frames(triangle)))
    }

    /// `compute_frame_weights_from_sample_sphere_triangulation`, its stale
    /// pitch-only vertex as both builds keep it.
    fn sphere(&self, yaw: f32, pitch: f32) -> Option<([f32; 3], [usize; 3], Option<usize>)> {
        if self.points.len() < 4 {
            return None;
        }
        let ray = direction(yaw, pitch);
        let mut pitch_only: Option<usize> = None;
        let mut best: Option<(f32, [f32; 3], [usize; 3], [[f32; 3]; 3])> = None;
        let mut breaks = None;
        for triangle in &self.elements {
            breaks = None;
            let mut corners = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
            let original = corners;
            let mut slot = None;
            for (index, corner) in corners.iter().enumerate() {
                let [y, p] = self.points.get(*corner).copied().unwrap_or([0.0; 2]);
                if y.abs() < EPS && p.abs() > EPS {
                    if slot.is_none() {
                        slot = Some(index);
                        pitch_only = Some(*corner);
                    } else {
                        slot = None;
                        pitch_only = None;
                    }
                }
            }
            if let Some(slot) = slot {
                corners.swap(slot, 2);
                breaks = Some(2);
            }
            let on_sphere = |corner: usize| {
                let [y, p] = self.points.get(corner).copied().unwrap_or([0.0; 2]);
                direction(y, p)
            };
            let q = corners.map(on_sphere);
            let normal = cross3(sub3(q[1], q[0]), sub3(q[2], q[0]));
            let length = dot3(normal, normal).sqrt();
            let (normal, distance) = if length < EPS { ([0.0; 3], 0.0) } else {
                let normal = normal.map(|value| value / length);
                (normal, dot3(normal, q[0]))
            };
            let along = dot3(normal, ray);
            if along.abs() <= EPS {
                continue;
            }
            let t = distance / along;
            if t <= 0.0 {
                continue;
            }
            let hit = ray.map(|value| value * t);
            let (a, b, c) = barycentric3(hit, q[0], q[1], q[2]);
            if a >= -EPS && b >= -EPS && c >= -EPS {
                let mut weights = to_interpolation(a, b, c);
                if let Some(only) = pitch_only {
                    let only_pitch = self.points.get(only).map_or(0.0, |point| point[1]);
                    if only_pitch.abs() >= EPS {
                        weights[2] = (pitch / only_pitch).clamp(0.0, 1.0);
                    }
                }
                return Some((weights, corners, breaks));
            }
            if best.is_none_or(|(known, ..)| t < known) {
                best = Some((t, [a, b, c], original, original.map(on_sphere)));
            }
        }
        let (t, kept, corners, q) = best?;
        let hit = ray.map(|value| value * t);
        let mut order = [0usize, 1, 2];
        order.sort_by(|left, right| kept[*right].partial_cmp(&kept[*left]).unwrap_or(std::cmp::Ordering::Equal));
        let (_, s) = closest_on_line3(hit, q[order[0]], sub3(q[order[1]], q[order[0]]));
        Some(([1.0 - s, s, 0.0], order.map(|index| corners[index]), breaks))
    }
}

fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn length2_squared(a: [f32; 2]) -> f32 {
    a[0] * a[0] + a[1] * a[1]
}

fn cross2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

/// `closest_point_on_line`: the point and how far along, pinned to the segment.
fn closest_on_line2(p: [f32; 2], a: [f32; 2], d: [f32; 2]) -> ([f32; 2], f32) {
    let length = length2_squared(d).sqrt();
    if length <= EPS {
        return (a, 0.0);
    }
    let t = ((sub2(p, a)[0] * d[0] + sub2(p, a)[1] * d[1]) / length / length).clamp(0.0, 1.0);
    ([a[0] + d[0] * t, a[1] + d[1] * t], t)
}

/// `point_in_triangle2d`, for counter-clockwise triangles.
fn point_in_triangle2(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    if cross2(sub2(b, a), sub2(c, a)).abs() < EPS {
        return false;
    }
    [(a, b), (b, c), (c, a)].iter().all(|(u, v)| {
        let w = sub2(p, *u);
        let length = length2_squared(w).sqrt();
        let s = if length >= EPS { cross2(sub2(*v, *u), w) / length } else { 0.0 };
        s > -EPS
    })
}

fn barycentric2(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> (f32, f32, f32) {
    let area = |x: [f32; 2], y: [f32; 2], z: [f32; 2]| 0.5 * cross2(sub2(x, z), sub2(y, z));
    let whole = area(a, b, c);
    if whole.abs() < EPS {
        return (0.0, 0.0, 0.0);
    }
    let v = area(a, p, c) / whole;
    let w = area(a, b, p) / whole;
    (1.0 - v - w, v, w)
}

/// `barycentric_coordinates_to_interpolation_weights`.
fn to_interpolation(a: f32, b: f32, c: f32) -> [f32; 3] {
    let s = a + b;
    if s.abs() > EPS { [a / s, b / s, c] } else { [a, b, c] }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `vector3d_from_euler_angles2d`.
fn direction(yaw: f32, pitch: f32) -> [f32; 3] {
    [yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()]
}

fn closest_on_line3(p: [f32; 3], a: [f32; 3], d: [f32; 3]) -> ([f32; 3], f32) {
    let length = dot3(d, d).sqrt();
    if length <= EPS {
        return (a, 0.0);
    }
    let t = (dot3(sub3(p, a), d) / length / length).clamp(0.0, 1.0);
    ([a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t], t)
}

fn barycentric3(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> (f32, f32, f32) {
    let across = cross3(sub3(a, c), sub3(b, c));
    let length = dot3(across, across).sqrt();
    if length < EPS {
        return (0.0, 0.0, 0.0);
    }
    let normal = across.map(|value| value / length);
    let whole = 0.5 * dot3(across, normal);
    if whole.abs() < EPS {
        return (0.0, 0.0, 0.0);
    }
    let v = 0.5 * dot3(cross3(sub3(a, c), sub3(p, c)), normal) / whole;
    let w = 0.5 * dot3(cross3(sub3(a, p), sub3(b, p)), normal) / whole;
    (1.0 - v - w, v, w)
}

/// One blend screen as the preview runs it.
struct Screen {
    animation: usize,
    piecewise: bool,
    weight: f32,
    /// The skeleton nodes it may write.
    mask: Vec<bool>,
}

/// The overlay group the state of `clip` names, most specific first: its
/// action, then its set, type, class and mode
/// (`find_hierarchical_property_index_and_graph`).
fn overlay_group(graph: &TagFile, clip: usize) -> Option<usize> {
    let content = graph.root().descend("content")?;
    let group = |holder: &TagStruct<'_>| holder.read_int_any("overlay group").and_then(|index| usize::try_from(index).ok());
    let names = |entry: &TagStruct<'_>| {
        let animation = entry.descend("animation")?;
        let own = animation.read_int_any("graph index").is_none_or(|graph| graph < 0);
        own.then(|| animation.read_block_index("animation")).and_then(|index| usize::try_from(index).ok())
    };
    for mode in elements(content, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                for set in elements(kind, "sets") {
                    for action in elements(set, "actions") {
                        if names(&action) == Some(clip) {
                            return group(&action)
                                .or_else(|| group(&set))
                                .or_else(|| group(&kind))
                                .or_else(|| group(&class))
                                .or_else(|| group(&mode));
                        }
                    }
                }
            }
        }
    }
    None
}

/// A node mask's nodes: every node but those it lists, as the `look` mask's
/// postprocessed flags have it.
fn node_mask(definitions: TagStruct<'_>, index: Option<usize>, nodes: usize) -> Vec<bool> {
    let mut mask = vec![true; nodes];
    let Some(entry) = index.and_then(|index| elements(definitions, "node masks").get(index).copied()) else {
        return mask;
    };
    for listed in elements(entry, "nodes") {
        if let Some(node) = usize::try_from(listed.read_block_index("node")).ok().and_then(|node| mask.get_mut(node)) {
            *node = false;
        }
    }
    mask
}

/// The screens of `clip`'s overlay group a unit standing at rest runs:
/// weapon up, so `active only when weapon down` is off, no damage, no
/// motion, and the object functions a screen weighs by at their rest of 1.
fn screens(graph: &TagFile, clip: usize, nodes: usize) -> Vec<Screen> {
    let Some(definitions) = graph.root().descend("definitions") else {
        return Vec::new();
    };
    let Some(group) = overlay_group(graph, clip).and_then(|group| elements(definitions, "overlay groups").get(group).copied()) else {
        return Vec::new();
    };
    let all = elements(definitions, "NEW blend screens");
    let functions = elements(definitions, "functions");
    let mut out = Vec::new();
    for item in elements(group, "blend screens") {
        if item.read_int_any("flags").is_some_and(|flags| flags & ITEM_DISABLED != 0) {
            continue;
        }
        let Some(screen) = usize::try_from(item.read_block_index("blend screen")).ok().and_then(|index| all.get(index)) else {
            continue;
        };
        let flags = screen.read_int_any("flags").unwrap_or(0);
        if flags & ACTIVE_ONLY_WHEN_WEAPON_DOWN != 0 {
            continue;
        }
        let input = match screen.read_int_any("weight source") {
            Some(WEIGHT_NONE) | Some(WEIGHT_OBJECT_FUNCTION) => 1.0,
            _ => 0.0,
        };
        let function = usize::try_from(screen.read_block_index("weight function"))
            .ok()
            .and_then(|index| functions.get(index))
            .and_then(|function| tag_function_at(&function.descend("function")?, "function"));
        let weight = function.map_or(input, |function| function.evaluate(input, 0.0)).clamp(0.0, 1.0);
        if weight <= 0.0 {
            continue;
        }
        let Some(animation) = screen.descend("animation") else {
            continue;
        };
        if animation.read_int_any("graph index").is_some_and(|graph| graph >= 0) {
            continue;
        }
        let Some(animation) = usize::try_from(animation.read_block_index("animation")).ok() else {
            continue;
        };
        let mask = usize::try_from(item.read_block_index("node mask")).ok();
        out.push(Screen {
            animation,
            piecewise: flags & ATTEMPT_PIECEWISE != 0,
            weight,
            mask: node_mask(definitions, mask, nodes),
        });
    }
    out
}

/// One whole overlay frame, or none.
fn frame_of(deltas: &[Vec<OverlayDelta>], frame: usize) -> Option<&Vec<OverlayDelta>> {
    deltas.get(frame.min(deltas.len().checked_sub(1)?))
}

/// `remap_and_blend_unpacked_data_to_target<1,0>`: each animated component
/// of `into` moved toward `from`'s by `weight`.
fn blend_into(into: &mut [OverlayDelta], from: &[OverlayDelta], weight: f32) {
    for (into, from) in into.iter_mut().zip(from) {
        if let (Some(a), Some(b)) = (into.rotation, from.rotation) {
            into.rotation = Some(a.nlerp(b, weight));
        }
        if let (Some(a), Some(b)) = (into.translation, from.translation) {
            into.translation = Some(RealPoint3d { x: a.x + (b.x - a.x) * weight, y: a.y + (b.y - a.y) * weight, z: a.z + (b.z - a.z) * weight });
        }
        if let (Some(a), Some(b)) = (into.scale, from.scale) {
            into.scale = Some(a + (b - a) * weight);
        }
    }
}

/// `internal_compute_node_orientations_new` for an overlay: the base frame
/// with the others blended in turn, laid over `pose` on `mask`'s nodes, whole
/// at full weight and scaled down below it.
fn decode_and_apply(
    deltas: &[Vec<OverlayDelta>],
    base: usize,
    extras: &[(usize, f32)],
    weight: f32,
    mask: &[bool],
    pose: &mut Pose,
) {
    if weight <= EPS {
        return;
    }
    let Some(mut delta) = frame_of(deltas, base).cloned() else {
        return;
    };
    for (frame, share) in extras {
        if let Some(other) = frame_of(deltas, *frame) {
            blend_into(&mut delta, other, *share);
        }
    }
    let full = weight > 1.0 - EPS;
    for frame in &mut pose.frames {
        for (node, (transform, delta)) in frame.iter_mut().zip(&delta).enumerate() {
            if !mask.get(node).copied().unwrap_or(false) {
                continue;
            }
            lay(transform, delta, if full { 1.0 } else { weight }, full);
        }
    }
}

/// `rotation_blender`, `translation_blender` and `scale_blender`: modes 2
/// (whole) and 3 (by `weight`, the turn left unnormalized as the game does).
fn lay(transform: &mut NodeTransform, delta: &OverlayDelta, weight: f32, full: bool) {
    if let Some(turn) = delta.rotation {
        let turn = if full {
            turn
        } else {
            let sign = if turn.w < 0.0 { -1.0 } else { 1.0 };
            RealQuaternion { i: turn.i * weight, j: turn.j * weight, k: turn.k * weight, w: sign + (turn.w - sign) * weight }
        };
        transform.rotation = transform.rotation * turn;
    }
    if let Some(moved) = delta.translation {
        let t = transform.translation;
        transform.translation = RealPoint3d { x: t.x + moved.x * weight, y: t.y + moved.y * weight, z: t.z + moved.z * weight };
    }
    if let Some(scale) = delta.scale {
        transform.scale *= 1.0 + weight * (scale - 1.0);
    }
}

/// Where the preview aims and looks, yaw then pitch in radians: dead ahead.
/// `H2_SHOT_AIM=yaw,pitch` (degrees) turns a test shot's.
fn aim() -> (f32, f32) {
    #[cfg(test)]
    if let Ok(aim) = std::env::var("H2_SHOT_AIM") {
        let mut parts = aim.split(',').map(|part| part.trim().parse::<f32>().unwrap_or(0.0).to_radians());
        return (parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0));
    }
    (0.0, 0.0)
}

/// Lay the blend screens of `clip`'s state over its `pose`, aiming and
/// looking as [`aim`] has it, if `graph` mixes on the data-driven board.
pub(crate) fn lay_blend_screens(graph: &TagFile, animation: &Animation<'_>, skeleton: &Skeleton, clip: usize, pose: &mut Pose) {
    let data_driven = graph
        .root()
        .descend("definitions")
        .and_then(|definitions| definitions.read_int_any("private flags"))
        .is_some_and(|flags| flags & DATA_DRIVEN != 0);
    if !data_driven {
        return;
    }
    for screen in screens(graph, clip, skeleton.nodes.len()) {
        let Some(group) = animation.get(screen.animation) else {
            continue;
        };
        if group.animation_type.as_deref() != Some("overlay") {
            continue;
        }
        let Some(data) = group.data_section("blend_screen_data").and_then(BlendScreenData::parse) else {
            continue;
        };
        // Every source this preview runs is aim or look, or reads nothing.
        let (yaw, pitch) = aim();
        let Some((weights, frames, breaks)) = data.weights(yaw, pitch) else {
            continue;
        };
        let Some(deltas) = overlay_deltas(group, skeleton) else {
            continue;
        };
        compose(&deltas, weights, frames, breaks, screen.piecewise, screen.weight, &screen.mask, pose);
    }
}

fn overlay_deltas(group: &AnimationGroup<'_>, skeleton: &Skeleton) -> Option<Vec<Vec<OverlayDelta>>> {
    Some(group.decode().ok()?.overlay_deltas(skeleton))
}

/// `compute_node_orientations_from_blend_screen_with_object_space_override`:
/// the first weighted frame is the base, the next two blend into it, and a
/// sphere's pitch-only frame past the break is laid on its own.
#[allow(clippy::too_many_arguments)]
fn compose(
    deltas: &[Vec<OverlayDelta>],
    weights: [f32; 3],
    frames: [usize; 3],
    breaks: Option<usize>,
    piecewise: bool,
    weight: f32,
    mask: &[bool],
    pose: &mut Pose,
) {
    let mut base = None;
    let mut extras = Vec::new();
    for index in 0..3 {
        if Some(index) == breaks {
            break;
        }
        if weights[index] > EPS {
            if base.is_none() {
                base = Some(frames[index]);
            } else {
                extras.push((frames[index], weights[index]));
            }
        }
    }
    let base = base.unwrap_or(0);
    match breaks {
        None => decode_and_apply(deltas, base, &extras, weight, mask, pose),
        Some(_) => {
            let secondary = if weights[2] > EPS { weights[2] } else { 0.0 };
            let first = if piecewise { weight } else { weight * (1.0 - secondary) };
            decode_and_apply(deltas, base, &extras, first, mask, pose);
            if secondary > EPS {
                decode_and_apply(deltas, frames[2], &[], weight * secondary, mask, pose);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `combat:rifle:aim_still_up`'s layout: yaw 0, −55° and +73.5° by pitch
    /// ±90, ±45 and 0, and the spec's sixteen triangles.
    fn aim_still_up() -> BlendScreenData {
        let degrees = |value: f32| value.to_radians();
        let mut points = Vec::new();
        for yaw in [0.0, -55.0, 73.5] {
            for pitch in [90.0, 45.0, 0.0, -45.0, -90.0] {
                points.push([degrees(yaw), degrees(pitch)]);
            }
        }
        let none = 0xFF;
        let triangles = [
            [0, 1, 11, 6, 12, 4], [2, 1, 7, 2, 9, 3], [1, 2, 12, 1, 5, 12], [3, 2, 7, 5, 1, 10],
            [11, 10, 0, none, none, 0], [3, 12, 2, 13, 2, 3], [1, 0, 6, 0, 8, 9], [4, 3, 9, 14, 11, none],
            [5, 6, 0, none, 6, none], [6, 7, 1, none, 1, 6], [7, 8, 3, none, 11, 3], [8, 9, 3, none, 7, 10],
            [12, 11, 1, none, 0, 2], [12, 3, 13, 5, 14, none], [4, 13, 3, 15, 13, 7], [4, 14, 13, none, none, 14],
        ];
        BlendScreenData { kind: 1, points, elements: triangles.to_vec() }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn dead_ahead_is_the_middle_frame_whole() {
        let (weights, frames, breaks) = aim_still_up().weights(0.0, 0.0).unwrap();
        assert_eq!((weights, frames, breaks), ([1.0, 0.0, 0.0], [2, 1, 7], None));
    }

    #[test]
    fn the_spec_vectors_come_back() {
        let screen = aim_still_up();
        let at = |yaw: f32, pitch: f32| screen.weights(yaw.to_radians(), pitch.to_radians()).unwrap();
        let (weights, frames, _) = at(30.0, 20.0);
        assert!(close(weights, [0.75096, 0.24904, 0.40816]) && frames == [1, 2, 12], "{weights:?} {frames:?}");
        let (weights, frames, _) = at(-30.0, -20.0);
        assert!(close(weights, [0.97778, 0.02222, 0.54545]) && frames == [3, 2, 7], "{weights:?} {frames:?}");
        let (weights, frames, _) = at(100.0, 0.0);
        assert!(close(weights, [0.0, 0.0, 1.0]) && frames == [1, 2, 12], "clamped to 73.5: {weights:?} {frames:?}");
        let (weights, frames, _) = at(60.0, 80.0);
        assert!(close(weights, [0.27222, 0.72778, 0.18367]) && frames == [11, 10, 0], "{weights:?} {frames:?}");
    }

    #[test]
    fn a_line_of_poses_splits_between_its_neighbours() {
        let degrees = |pitch: f32| [0.0, pitch.to_radians()];
        let points = [52.4, 33.1, 0.0, -30.2, -43.8].map(degrees).to_vec();
        let segments = [[4, 3, 0, 0xFF, 1, 0], [3, 2, 0, 0, 2, 0], [2, 1, 0, 1, 3, 0], [1, 0, 0, 2, 0xFF, 0]];
        let line = BlendScreenData { kind: 0, points, elements: segments.to_vec() };
        let (weights, frames, _) = line.weights(0.0, 0.0).unwrap();
        assert!(close(weights, [0.0, 1.0, 0.0]) && frames[..2] == [3, 2], "{weights:?} {frames:?}");
        // The spec gives these pitches to a tenth of a degree, so the
        // weights here agree with its exact ones to a few thousandths.
        let (weights, frames, _) = line.weights(0.0, 20f32.to_radians()).unwrap();
        assert!((weights[0] - 0.39518).abs() < 2e-3 && frames[..2] == [2, 1], "{weights:?} {frames:?}");
    }

    #[test]
    fn a_sphere_blends_pitch_on_its_own() {
        let degrees = |yaw: f32, pitch: f32| [yaw.to_radians(), pitch.to_radians()];
        let points = vec![degrees(-2.0, 0.0), degrees(180.0, 0.0), degrees(90.0, 0.0), degrees(-90.0, 0.0), degrees(0.0, -90.0), degrees(0.0, 90.0)];
        // `banshee_d:acc`'s first triangle holds dead ahead, with its pitch-only
        // vertex 5; the rest of its triangles are not needed to reach it.
        let triangles = vec![[0, 2, 5, 0xFF, 0xFF, 0xFF], [3, 0, 4, 0xFF, 0xFF, 0xFF], [0, 3, 5, 0xFF, 0xFF, 0xFF], [2, 0, 4, 0xFF, 0xFF, 0xFF]];
        let sphere = BlendScreenData { kind: 2, points, elements: triangles };
        let (weights, frames, breaks) = sphere.weights(0.0, 30f32.to_radians()).unwrap();
        assert_eq!(breaks, Some(2));
        assert_eq!(frames[2], 5, "the pitch-only vertex last");
        assert!((weights[2] - 1.0 / 3.0).abs() < 1e-4, "30 of its 90: {weights:?}");
    }

    #[test]
    fn a_partial_overlay_turns_part_of_the_way() {
        let mut transform = NodeTransform::IDENTITY;
        let half_turn = RealQuaternion { i: 0.0, j: 0.0, k: 1.0, w: 0.0 };
        let delta = OverlayDelta { rotation: Some(half_turn), translation: Some(RealPoint3d { x: 2.0, y: 0.0, z: 0.0 }), scale: Some(3.0) };
        lay(&mut transform, &delta, 0.5, false);
        assert!((transform.translation.x - 1.0).abs() < 1e-6);
        assert!((transform.scale - 2.0).abs() < 1e-6);
        assert!((transform.rotation.k - 0.5).abs() < 1e-6 && (transform.rotation.w - 0.5).abs() < 1e-6, "unnormalized");
        let mut whole = NodeTransform::IDENTITY;
        lay(&mut whole, &delta, 1.0, true);
        assert_eq!(whole.rotation, half_turn);
    }

    /// With `BLAM_TEST_HREK`: the Spartan's rifle idle names overlay group 57,
    /// whose aim and look screens run at rest and its weapon-down and pain
    /// screens do not; the aim screen leaves out the `look` mask's nodes.
    #[test]
    fn the_spartans_rifle_idle_runs_its_aim_and_look_screens() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            return;
        };
        let path = std::path::Path::new(&root).join("objects/characters/spartans/spartans.model_animation_graph");
        let graph = TagFile::read(path).unwrap();
        let animation = Animation::new(&graph).unwrap();
        let idle = animation.find("combat:rifle:idle").unwrap().index;
        assert_eq!(overlay_group(&graph, idle), Some(57));
        let skeleton = Skeleton::from_tag(&graph);
        let running = screens(&graph, idle, skeleton.nodes.len());
        let names: Vec<_> = running.iter().filter_map(|screen| animation.get(screen.animation)?.name.clone()).collect();
        assert_eq!(names, ["combat:rifle:aim_still_up", "combat:rifle:look_still_up"]);
        let left_out: Vec<_> = (0..skeleton.nodes.len()).filter(|node| !running[0].mask[*node]).collect();
        assert_eq!(left_out, [27, 30, 38, 42]);
        assert!(running[1].mask.iter().all(|node| *node), "no mask is every node");
    }
}
