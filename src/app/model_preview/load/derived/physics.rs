//! Physics models as preview geometry: each shape tessellated and placed by its parent bone.

use super::super::super::{
    BatchOutline, MAX_PREVIEW_BONES, ModelPreviewLayer, RenderModelPreview, RenderModelPreviewBatch,
    RenderModelPreviewMaterial, RenderModelPreviewVertex, TagFile, physics_jms_for_game,
};
use super::super::errors::append_model_errors;
use super::{
    JMS_SCALE, Overflow, PHYSICS_COLOR, PHYSICS_REGION, VertexBase, empty_preview,
    ensure_preview_region, face_normal, finish_preview, point, rotate,
};
use crate::app::model_preview::common::geometry::expand_preview_bounds_local;
use blam_tags::JmsFile;
use blam_tags::math::RealQuaternion;

/// A physics tag (`phmo`) tessellated shape by shape. The tag stores
/// parametric primitives — spheres, boxes, pills, convex polyhedra — with no
/// mesh anywhere, so the mesh is generated here.
pub(in crate::app::model_preview) fn build_physics_preview(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> Result<RenderModelPreview, String> {
    let jms = physics_jms_for_game(tag, skeleton).map_err(|error| error.to_string())?;
    // Each shape's faces, and its outline as the game's debug view draws
    // it: rings and arcs round a sphere or pill, a box's or hull's corners.
    let mut vertices = Vec::new();
    let mut lines = Vec::new();
    let mut place = |triples: &[_], outline: &[_], parent| {
        append_physics_shape(&mut vertices, triples, parent, &jms, skeleton);
        append_physics_shape(&mut lines, outline, parent, &jms, skeleton);
    };
    for sphere in &jms.spheres {
        let (mut triples, mut outline) = (Vec::new(), Vec::new());
        let center = point(&sphere.translation);
        push_sphere(&mut triples, &sphere.rotation, center, sphere.radius);
        push_sphere_outline(&mut outline, &sphere.rotation, center, sphere.radius);
        place(&triples, &outline, sphere.parent);
    }
    for shape in &jms.boxes {
        let mut triples = Vec::new();
        push_box(&mut triples, shape);
        place(&triples, &crease_lines(&triples), shape.parent);
    }
    for capsule in &jms.capsules {
        let (mut triples, mut outline) = (Vec::new(), Vec::new());
        push_capsule(&mut triples, capsule);
        push_capsule_outline(&mut outline, capsule);
        place(&triples, &outline, capsule.parent);
    }
    for convex in &jms.convex_shapes {
        let mut triples = Vec::new();
        push_convex(&mut triples, convex);
        place(&triples, &crease_lines(&triples), convex.parent);
    }
    let mut preview = empty_preview();
    push_physics_batch(&mut preview, vertices, lines);
    append_model_errors(tag, &mut preview, ModelPreviewLayer::Physics);
    finish_preview(preview, "physics model")
}

pub(super) fn append_physics_shape(
    output: &mut Vec<RenderModelPreviewVertex>,
    triples: &[([f32; 3], [f32; 3])],
    parent: i32,
    jms: &JmsFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) {
    let source_node = usize::try_from(parent)
        .ok()
        .and_then(|index| jms.nodes.get(index));
    let target_node = source_node.and_then(|source| {
        skeleton.and_then(|nodes| nodes.iter().position(|node| node.name == source.name))
    });
    for &(mut position, mut normal) in triples {
        if let Some(node) = source_node {
            let rotated = rotate(&node.rotation, position);
            position = [
                rotated[0] + node.translation.x / JMS_SCALE,
                rotated[1] + node.translation.y / JMS_SCALE,
                rotated[2] + node.translation.z / JMS_SCALE,
            ];
            normal = rotate(&node.rotation, normal);
        }
        let (node_indices, node_weights) = target_node
            .filter(|&index| index < MAX_PREVIEW_BONES)
            .map(|index| ([index as f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]))
            .unwrap_or_default();
        output.push(RenderModelPreviewVertex {
            position,
            normal,
            node_indices,
            node_weights,
            ..Default::default()
        });
    }
}

/// The shapes' `faces` and the `lines` (point pairs) that outline them, as
/// two batches sharing one material.
fn push_physics_batch(
    preview: &mut RenderModelPreview,
    faces: Vec<RenderModelPreviewVertex>,
    lines: Vec<RenderModelPreviewVertex>,
) {
    let material_index = preview.materials.len().min(u16::MAX as usize) as u16;
    let faces = push_physics_part(preview, faces, material_index, BatchOutline::Hidden);
    let lines = push_physics_part(preview, lines, material_index, BatchOutline::Lines);
    if faces || lines {
        preview
            .materials
            .push(RenderModelPreviewMaterial::default());
        ensure_preview_region(preview, PHYSICS_REGION);
    }
}

/// One physics batch of `vertices`, each used once; false when there are
/// none.
fn push_physics_part(
    preview: &mut RenderModelPreview,
    vertices: Vec<RenderModelPreviewVertex>,
    material_index: u16,
    outline: BatchOutline,
) -> bool {
    if vertices.is_empty() {
        return false;
    }
    let Some(vertex_base) = VertexBase::of(
        preview.vertices.len(),
        vertices.len(),
        Overflow::RefusesAppend,
    ) else {
        return false;
    };
    let index_start = preview.indices.len() as u32;
    for vertex in &vertices {
        expand_preview_bounds_local(
            &mut preview.bounds_min,
            &mut preview.bounds_max,
            vertex.position,
        );
    }
    preview
        .indices
        .extend((0..vertices.len() as u32).map(|offset| vertex_base.offset(offset)));
    preview.vertices.extend(vertices);
    preview.batches.push(RenderModelPreviewBatch {
        region_name: PHYSICS_REGION.to_owned(),
        permutation_name: "default".to_owned(),
        material_index,
        index_start,
        index_count: preview.indices.len() as u32 - index_start,
        flat_color: Some(PHYSICS_COLOR),
        layer: ModelPreviewLayer::Physics,
        outline,
        sort_position: None,
    });
    true
}

/// Points round each outline circle or arc's full turn.
const OUTLINE_SEGMENTS: usize = 32;

/// Line pairs along `points`, a point after point, scaled into world units.
fn push_polyline(lines: &mut Vec<([f32; 3], [f32; 3])>, points: &[[f32; 3]]) {
    let scale = |p: [f32; 3]| [p[0] / JMS_SCALE, p[1] / JMS_SCALE, p[2] / JMS_SCALE];
    for pair in points.windows(2) {
        lines.push((scale(pair[0]), [0.0, 0.0, 1.0]));
        lines.push((scale(pair[1]), [0.0, 0.0, 1.0]));
    }
}

/// `fraction` of a turn round an arc from `start` radians, `point` placing
/// each angle.
fn arc(start: f32, fraction: f32, point: impl Fn(f32) -> [f32; 3]) -> Vec<[f32; 3]> {
    let steps = ((OUTLINE_SEGMENTS as f32 * fraction).ceil() as usize).max(1);
    (0..=steps)
        .map(|step| point(start + std::f32::consts::TAU * fraction * step as f32 / steps as f32))
        .collect()
}

/// A sphere's outline: its three great circles on its own axes, as the
/// object's bounding sphere is drawn.
pub(super) fn push_sphere_outline(
    lines: &mut Vec<([f32; 3], [f32; 3])>,
    rotation: &RealQuaternion,
    center: [f32; 3],
    radius: f32,
) {
    if !(radius.is_finite() && radius > 0.0) {
        return;
    }
    let place = |local: [f32; 3]| {
        let r = rotate(rotation, local);
        [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
    };
    let (s, c) = (|a: f32| radius * a.sin(), |a: f32| radius * a.cos());
    push_polyline(lines, &arc(0.0, 1.0, |a| place([c(a), s(a), 0.0])));
    push_polyline(lines, &arc(0.0, 1.0, |a| place([c(a), 0.0, s(a)])));
    push_polyline(lines, &arc(0.0, 1.0, |a| place([0.0, c(a), s(a)])));
}

/// A pill's outline: a ring round each end of its body, four lines down
/// its sides, and over each end a cross of half circles.
pub(super) fn push_capsule_outline(
    lines: &mut Vec<([f32; 3], [f32; 3])>,
    capsule: &blam_tags::JmsCapsule,
) {
    let radius = capsule.radius;
    if !(radius.is_finite() && radius > 0.0 && capsule.height.is_finite()) {
        return;
    }
    let center = point(&capsule.translation);
    let height = capsule.height.max(0.0);
    // Local +Z is the pill's axis, its body from 0 to `height`, as
    // `push_capsule` lays it.
    let place = |local: [f32; 3]| {
        let r = rotate(&capsule.rotation, local);
        [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
    };
    let (s, c) = (|a: f32| radius * a.sin(), |a: f32| radius * a.cos());
    for z in [0.0, height] {
        push_polyline(lines, &arc(0.0, 1.0, |a| place([c(a), s(a), z])));
    }
    for quarter in 0..4 {
        let a = std::f32::consts::FRAC_PI_2 * quarter as f32;
        push_polyline(lines, &[place([c(a), s(a), 0.0]), place([c(a), s(a), height])]);
    }
    // Half circles from one side of the ring, over the end, to the other.
    for (z, over) in [(height, 1.0), (0.0, -1.0)] {
        push_polyline(lines, &arc(0.0, 0.5, |a| place([c(a), 0.0, z + over * s(a)])));
        push_polyline(lines, &arc(0.0, 0.5, |a| place([0.0, c(a), z + over * s(a)])));
    }
}

/// The edges of `triples` (flat-shaded triangles) where their faces meet at
/// an angle, as line pairs: a box's twelve, not the diagonals that split its
/// sides.
pub(super) fn crease_lines(triples: &[([f32; 3], [f32; 3])]) -> Vec<([f32; 3], [f32; 3])> {
    use std::collections::HashMap;
    // Positions to a micrometre, so shared corners meet as one key.
    let key = |p: [f32; 3]| p.map(|v| (v * 1.0e6).round() as i64);
    let mut faces: HashMap<([i64; 3], [i64; 3]), Vec<[f32; 3]>> = HashMap::new();
    let mut order = Vec::new();
    for triangle in triples.chunks_exact(3) {
        let normal = triangle[0].1;
        for (from, to) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (triangle[from].0, triangle[to].0);
            let (ka, kb) = (key(a), key(b));
            let edge = if ka <= kb { (ka, kb) } else { (kb, ka) };
            let normals = faces.entry(edge).or_default();
            if normals.is_empty() {
                order.push((edge, a, b));
            }
            normals.push(normal);
        }
    }
    let mut lines = Vec::new();
    for (edge, a, b) in order {
        let normals = &faces[&edge];
        let flat = normals.len() == 2 && {
            let (n, m) = (normals[0], normals[1]);
            n[0] * m[0] + n[1] * m[1] + n[2] * m[2] > 0.999
        };
        if !flat {
            lines.push((a, [0.0, 0.0, 1.0]));
            lines.push((b, [0.0, 0.0, 1.0]));
        }
    }
    lines
}

/// Emit one triangle with its flat face normal, ÷100 into world units.
fn emit_cm_triangle(
    triples: &mut Vec<([f32; 3], [f32; 3])>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) {
    let scale = |p: [f32; 3]| [p[0] / JMS_SCALE, p[1] / JMS_SCALE, p[2] / JMS_SCALE];
    let (a, b, c) = (scale(a), scale(b), scale(c));
    let normal = face_normal(a, b, c);
    triples.push((a, normal));
    triples.push((b, normal));
    triples.push((c, normal));
}

const SPHERE_RINGS: usize = 8;

const SPHERE_SEGMENTS: usize = 14;

pub(super) fn push_sphere(
    triples: &mut Vec<([f32; 3], [f32; 3])>,
    rotation: &RealQuaternion,
    center: [f32; 3],
    radius: f32,
) {
    if !(radius.is_finite() && radius > 0.0) {
        return;
    }
    let place = |ring: usize, segment: usize| -> [f32; 3] {
        let theta = std::f32::consts::PI * ring as f32 / SPHERE_RINGS as f32;
        let phi = std::f32::consts::TAU * segment as f32 / SPHERE_SEGMENTS as f32;
        let local = [
            radius * theta.sin() * phi.cos(),
            radius * theta.sin() * phi.sin(),
            radius * theta.cos(),
        ];
        let r = rotate(rotation, local);
        [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
    };
    for ring in 0..SPHERE_RINGS {
        for segment in 0..SPHERE_SEGMENTS {
            let (a, b) = (place(ring, segment), place(ring, segment + 1));
            let (c, d) = (place(ring + 1, segment), place(ring + 1, segment + 1));
            if ring > 0 {
                emit_cm_triangle(triples, a, b, c);
            }
            if ring + 1 < SPHERE_RINGS {
                emit_cm_triangle(triples, b, d, c);
            }
        }
    }
}

pub(super) fn push_box(triples: &mut Vec<([f32; 3], [f32; 3])>, shape: &blam_tags::JmsBox) {
    let half = [
        (shape.width * 0.5).abs(),
        (shape.length * 0.5).abs(),
        (shape.height * 0.5).abs(),
    ];
    if !half.iter().all(|extent| extent.is_finite()) {
        return;
    }
    let center = point(&shape.translation);
    let corner = |x: f32, y: f32, z: f32| -> [f32; 3] {
        let local = [x * half[0], y * half[1], z * half[2]];
        let r = rotate(&shape.rotation, local);
        [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
    };
    // Six faces, two triangles each, wound outward.
    let faces: [[[f32; 3]; 4]; 6] = [
        [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]], // +z
        [
            [-1., 1., -1.],
            [1., 1., -1.],
            [1., -1., -1.],
            [-1., -1., -1.],
        ], // -z
        [[1., -1., -1.], [1., 1., -1.], [1., 1., 1.], [1., -1., 1.]], // +x
        [
            [-1., -1., 1.],
            [-1., 1., 1.],
            [-1., 1., -1.],
            [-1., -1., -1.],
        ], // -x
        [[-1., 1., -1.], [-1., 1., 1.], [1., 1., 1.], [1., 1., -1.]], // +y
        [
            [1., -1., -1.],
            [1., -1., 1.],
            [-1., -1., 1.],
            [-1., -1., -1.],
        ], // -y
    ];
    for face in faces {
        let quad = face.map(|[x, y, z]| corner(x, y, z));
        emit_cm_triangle(triples, quad[0], quad[1], quad[2]);
        emit_cm_triangle(triples, quad[0], quad[2], quad[3]);
    }
}

pub(super) fn push_capsule(
    triples: &mut Vec<([f32; 3], [f32; 3])>,
    capsule: &blam_tags::JmsCapsule,
) {
    if !(capsule.radius.is_finite() && capsule.radius > 0.0 && capsule.height.is_finite()) {
        return;
    }
    let center = point(&capsule.translation);
    // Local +Z is the pill axis; the capsule is anchored at the bottom-cap
    // center, so the cylinder spans z ∈ [0, height] with hemispheres beyond.
    let place = |z: f32, ring_radius: f32, segment: usize, z_offset: f32| -> [f32; 3] {
        let phi = std::f32::consts::TAU * segment as f32 / SPHERE_SEGMENTS as f32;
        let local = [
            ring_radius * phi.cos(),
            ring_radius * phi.sin(),
            z + z_offset,
        ];
        let r = rotate(&capsule.rotation, local);
        [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
    };
    let height = capsule.height.max(0.0);
    // Cylinder wall.
    for segment in 0..SPHERE_SEGMENTS {
        let a = place(0.0, capsule.radius, segment, 0.0);
        let b = place(0.0, capsule.radius, segment + 1, 0.0);
        let c = place(height, capsule.radius, segment, 0.0);
        let d = place(height, capsule.radius, segment + 1, 0.0);
        emit_cm_triangle(triples, a, b, d);
        emit_cm_triangle(triples, a, d, c);
    }
    // Hemisphere caps: quarter-rings from the equator to each pole.
    let cap_rings = SPHERE_RINGS / 2;
    for (pole_z, direction) in [(height, 1.0f32), (0.0, -1.0f32)] {
        for ring in 0..cap_rings {
            let theta0 = std::f32::consts::FRAC_PI_2 * ring as f32 / cap_rings as f32;
            let theta1 = std::f32::consts::FRAC_PI_2 * (ring + 1) as f32 / cap_rings as f32;
            let (r0, z0) = (capsule.radius * theta0.cos(), capsule.radius * theta0.sin());
            let (r1, z1) = (capsule.radius * theta1.cos(), capsule.radius * theta1.sin());
            for segment in 0..SPHERE_SEGMENTS {
                let a = place(pole_z, r0, segment, z0 * direction);
                let b = place(pole_z, r0, segment + 1, z0 * direction);
                let c = place(pole_z, r1, segment, z1 * direction);
                let d = place(pole_z, r1, segment + 1, z1 * direction);
                if direction > 0.0 {
                    emit_cm_triangle(triples, a, b, d);
                    emit_cm_triangle(triples, a, d, c);
                } else {
                    emit_cm_triangle(triples, a, d, b);
                    emit_cm_triangle(triples, a, c, d);
                }
            }
        }
    }
}

/// Largest convex shape the brute-force hull below will attempt. Halo
/// polyhedra are small (usually well under 32 vertices); anything bigger is
/// malformed data not worth O(n⁴) over.
const MAX_CONVEX_VERTICES: usize = 96;

pub(super) fn push_convex(triples: &mut Vec<([f32; 3], [f32; 3])>, convex: &blam_tags::JmsConvex) {
    let count = convex.vertices.len();
    if !(4..=MAX_CONVEX_VERTICES).contains(&count) {
        return;
    }
    let center = point(&convex.translation);
    let points: Vec<[f32; 3]> = convex
        .vertices
        .iter()
        .map(|vertex| {
            let r = rotate(&convex.rotation, point(vertex));
            [r[0] + center[0], r[1] + center[1], r[2] + center[2]]
        })
        .collect();
    if points.iter().flatten().any(|value| !value.is_finite()) {
        return;
    }
    // Brute-force hull: a triple is a hull face when every other point lies on
    // one side of its plane. Coplanar sets emit overlapping triangles, which
    // draw identically; the shapes are far too small for the O(n⁴) to matter.
    let extent = points
        .iter()
        .flatten()
        .fold(0.0f32, |all, value| all.max(value.abs()));
    let eps = (extent * 1e-4).max(1e-6);
    for i in 0..count {
        for j in (i + 1)..count {
            for k in (j + 1)..count {
                let normal = {
                    let (a, b, c) = (points[i], points[j], points[k]);
                    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    [
                        e1[1] * e2[2] - e1[2] * e2[1],
                        e1[2] * e2[0] - e1[0] * e2[2],
                        e1[0] * e2[1] - e1[1] * e2[0],
                    ]
                };
                let length =
                    (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                if length <= f32::EPSILON {
                    continue;
                }
                let side = |p: [f32; 3]| -> f32 {
                    (p[0] - points[i][0]) * normal[0]
                        + (p[1] - points[i][1]) * normal[1]
                        + (p[2] - points[i][2]) * normal[2]
                };
                let mut above = false;
                let mut below = false;
                for (index, p) in points.iter().enumerate() {
                    if index == i || index == j || index == k {
                        continue;
                    }
                    let s = side(*p);
                    above |= s > eps * length;
                    below |= s < -eps * length;
                    if above && below {
                        break;
                    }
                }
                if above && below {
                    continue;
                }
                if !above {
                    // All other points below the plane: normal faces outward.
                    emit_cm_triangle(triples, points[i], points[j], points[k]);
                } else {
                    emit_cm_triangle(triples, points[i], points[k], points[j]);
                }
            }
        }
    }
}
