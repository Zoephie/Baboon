//! Preview geometry derived from non-render tags: collision models, physics
//! models, structure BSPs, and scenario BSP composites. It owns the conversion
//! of blam-tags' JMS/ASS scenes and physics primitives into
//! [`RenderModelPreview`]s; render_model decoding, the GL renderer, and the
//! panel presentation belong elsewhere.

use super::*;
use blam_tags::geometry::{CompressionBounds, read_compression_bounds_at};
use blam_tags::jms_split::MaterialLabel;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::extract_sbsp_render_geometry_meshes;
use blam_tags::{AssFile, AssObjectPayload, AssTriangle, JmsFile};
use std::collections::HashMap;

/// JMS and ASS positions are world units × 100 (centimetres); render_model
/// previews are world units, and an overlay merged into one must agree.
const JMS_SCALE: f32 = 100.0;

pub(in crate::app) const COLLISION_REGION: &str = "collision";
pub(in crate::app) const PHYSICS_REGION: &str = "physics";

/// Fixed overlay colors, chosen apart from the render palette so a collision
/// or physics layer reads at a glance no matter what it overlaps.
const COLLISION_COLOR: [u8; 3] = [0xED, 0x5E, 0xBE];
const PHYSICS_COLOR: [u8; 3] = [0xFF, 0x56, 0x56];
const PORTAL_COLOR: [u8; 3] = [120, 196, 176];
const WEATHER_COLOR: [u8; 3] = [150, 168, 200];

fn point(p: &RealPoint3d) -> [f32; 3] {
    [p.x, p.y, p.z]
}

fn rotate(q: &RealQuaternion, v: [f32; 3]) -> [f32; 3] {
    let r = q.rotate(RealVector3d {
        i: v[0],
        j: v[1],
        k: v[2],
    });
    [r.i, r.j, r.k]
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [n[0] / length, n[1] / length, n[2] / length]
    }
}

/// Append `(position, normal)` triples — three per triangle, world units —
/// as one region with one batch in `color`. The region gets a single
/// `default` permutation, which is what makes the existing region list its
/// on/off toggle.
fn push_derived_batch(
    preview: &mut RenderModelPreview,
    region_name: &str,
    color: Option<[u8; 3]>,
    triples: &[([f32; 3], [f32; 3])],
) {
    if triples.is_empty() {
        return;
    }
    let Ok(vertex_base) = u32::try_from(preview.vertices.len()) else {
        return;
    };
    if triples.len().checked_add(preview.vertices.len()).is_none()
        || triples.len() + preview.vertices.len() > u32::MAX as usize
    {
        return;
    }
    for (position, normal) in triples {
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, *position);
        preview.vertices.push(RenderModelPreviewVertex {
            position: *position,
            normal: *normal,
            ..Default::default()
        });
    }
    let index_start = preview.indices.len() as u32;
    preview
        .indices
        .extend((0..triples.len() as u32).map(|offset| vertex_base + offset));
    let material_index = preview.materials.len().min(u16::MAX as usize) as u16;
    preview
        .materials
        .push(RenderModelPreviewMaterial::default());
    preview.batches.push(RenderModelPreviewBatch {
        region_name: region_name.to_owned(),
        permutation_name: "default".to_owned(),
        material_index,
        index_start,
        index_count: triples.len() as u32,
        flat_color: color,
        layer: match region_name {
            COLLISION_REGION => ModelPreviewLayer::Collision,
            PHYSICS_REGION => ModelPreviewLayer::Physics,
            _ => ModelPreviewLayer::Render,
        },
    });
    ensure_preview_region(preview, region_name);
}

fn ensure_preview_region(preview: &mut RenderModelPreview, region_name: &str) {
    ensure_preview_region_permutation(preview, region_name, "default");
}

fn ensure_preview_region_permutation(
    preview: &mut RenderModelPreview,
    region_name: &str,
    permutation_name: &str,
) {
    if let Some(region) = preview
        .regions
        .iter_mut()
        .find(|region| region.name == region_name)
    {
        if !region
            .permutations
            .iter()
            .any(|name| name == permutation_name)
        {
            region.permutations.push(permutation_name.to_owned());
        }
        return;
    }
    if !preview
        .regions
        .iter()
        .any(|region| region.name == region_name)
    {
        preview.regions.push(RenderModelPreviewRegion {
            name: region_name.to_owned(),
            permutations: vec![permutation_name.to_owned()],
        });
    }
}

/// Preserve a collision JMS's actual region/permutation cells and bone
/// influences. The generic derived-mesh path intentionally flattens both,
/// which is right for BSP helper layers but made model collision static and
/// immune to the variant controls.
fn append_collision_jms(
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
        let vertex_base = preview.vertices.len() as u32;
        let index_start = preview.indices.len() as u32;
        preview
            .indices
            .extend((0..vertices.len() as u32).map(|offset| vertex_base + offset));
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
        });
        ensure_preview_region_permutation(preview, &region_name, &permutation_name);
    }
}

fn empty_preview() -> RenderModelPreview {
    RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    }
}

fn finish_preview(
    mut preview: RenderModelPreview,
    what: &str,
) -> Result<RenderModelPreview, String> {
    if preview.vertices.is_empty() {
        return Err(format!("This {what} has no previewable geometry."));
    }
    if !preview.bounds_min.iter().all(|b| b.is_finite()) {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }
    Ok(preview)
}

/// Append a JMS triangle mesh (÷100 into world units) as one region.
///
/// `authored_normals` keeps the vertex normals the source carried (H1 BSP
/// render geometry has real ones); off recomputes flat face normals, which is
/// what collision meshes need — their JMS writer emits a constant `(0,0,1)`.
fn append_jms_triangles(
    preview: &mut RenderModelPreview,
    jms: &JmsFile,
    region_name: &str,
    color: Option<[u8; 3]>,
    authored_normals: bool,
) {
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(jms.triangles.len() * 3);
    for triangle in &jms.triangles {
        let corners = [
            jms.vertices.get(triangle.v[0] as usize),
            jms.vertices.get(triangle.v[1] as usize),
            jms.vertices.get(triangle.v[2] as usize),
        ];
        let (Some(a), Some(b), Some(c)) = (corners[0], corners[1], corners[2]) else {
            continue;
        };
        let positions = [a, b, c].map(|vertex| {
            let p = point(&vertex.position);
            [p[0] / JMS_SCALE, p[1] / JMS_SCALE, p[2] / JMS_SCALE]
        });
        let flat = face_normal(positions[0], positions[1], positions[2]);
        for (vertex, position) in [a, b, c].into_iter().zip(positions) {
            let normal = if authored_normals {
                let n = [vertex.normal.i, vertex.normal.j, vertex.normal.k];
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if length > 0.0001 {
                    [n[0] / length, n[1] / length, n[2] / length]
                } else {
                    flat
                }
            } else {
                flat
            };
            triples.push((position, normal));
        }
    }
    push_derived_batch(preview, region_name, color, &triples);
}

/// A collision tag (`coll`, or H1 `model_collision_geometry`) as solid flat
/// geometry, posed by `skeleton` when the owning model supplies one.
pub(super) fn build_collision_preview(
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
fn pose_collision_errors(
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

/// A physics tag (`phmo`) tessellated shape by shape. The tag stores
/// parametric primitives — spheres, boxes, pills, convex polyhedra — with no
/// mesh anywhere, so the mesh is generated here.
pub(super) fn build_physics_preview(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> Result<RenderModelPreview, String> {
    let jms = physics_jms_for_game(tag, skeleton).map_err(|error| error.to_string())?;
    let mut vertices = Vec::new();
    for sphere in &jms.spheres {
        let mut triples = Vec::new();
        push_sphere(
            &mut triples,
            &sphere.rotation,
            point(&sphere.translation),
            sphere.radius,
        );
        append_physics_shape(&mut vertices, &triples, sphere.parent, &jms, skeleton);
    }
    for shape in &jms.boxes {
        let mut triples = Vec::new();
        push_box(&mut triples, shape);
        append_physics_shape(&mut vertices, &triples, shape.parent, &jms, skeleton);
    }
    for capsule in &jms.capsules {
        let mut triples = Vec::new();
        push_capsule(&mut triples, capsule);
        append_physics_shape(&mut vertices, &triples, capsule.parent, &jms, skeleton);
    }
    for convex in &jms.convex_shapes {
        let mut triples = Vec::new();
        push_convex(&mut triples, convex);
        append_physics_shape(&mut vertices, &triples, convex.parent, &jms, skeleton);
    }
    let mut preview = empty_preview();
    push_physics_batch(&mut preview, vertices);
    append_model_errors(tag, &mut preview, ModelPreviewLayer::Physics);
    finish_preview(preview, "physics model")
}

fn append_physics_shape(
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

fn push_physics_batch(preview: &mut RenderModelPreview, vertices: Vec<RenderModelPreviewVertex>) {
    if vertices.is_empty() {
        return;
    }
    let vertex_base = preview.vertices.len() as u32;
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
        .extend((0..vertices.len() as u32).map(|offset| vertex_base + offset));
    preview.vertices.extend(vertices);
    let material_index = preview.materials.len().min(u16::MAX as usize) as u16;
    preview
        .materials
        .push(RenderModelPreviewMaterial::default());
    preview.batches.push(RenderModelPreviewBatch {
        region_name: PHYSICS_REGION.to_owned(),
        permutation_name: "default".to_owned(),
        material_index,
        index_start,
        index_count: preview.indices.len() as u32 - index_start,
        flat_color: Some(PHYSICS_COLOR),
        layer: ModelPreviewLayer::Physics,
    });
    ensure_preview_region(preview, PHYSICS_REGION);
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

fn push_sphere(
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

fn push_box(triples: &mut Vec<([f32; 3], [f32; 3])>, shape: &blam_tags::JmsBox) {
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

fn push_capsule(triples: &mut Vec<([f32; 3], [f32; 3])>, capsule: &blam_tags::JmsCapsule) {
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

fn push_convex(triples: &mut Vec<([f32; 3], [f32; 3])>, convex: &blam_tags::JmsConvex) {
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

/// Which preview region an ASS object belongs to, read off its materials.
/// The ASS builders name recompile-marker layers by convention: `+portal` for
/// cluster portals, `@collision_only` for the sealed collision BSP,
/// `+weather` for weather polyhedra.
fn ass_object_region(ass: &AssFile, triangles: &[AssTriangle]) -> &'static str {
    let mut region = None;
    for triangle in triangles {
        let name = ass
            .materials
            .get(triangle.material.max(0) as usize)
            .map(|material| material.name.as_str())
            .unwrap_or("");
        let layer = if name.starts_with("+portal") {
            "portals"
        } else if name.starts_with("@collision_only") {
            COLLISION_REGION
        } else if name.starts_with("+weather") {
            "weather"
        } else {
            "render"
        };
        match region {
            None => region = Some(layer),
            Some(existing) if existing != layer => return "render",
            Some(_) => {}
        }
    }
    region.unwrap_or("render")
}

fn layer_color(region: &str) -> Option<[u8; 3]> {
    match region {
        r if r == COLLISION_REGION => Some(COLLISION_COLOR),
        "portals" => Some(PORTAL_COLOR),
        "weather" => Some(WEATHER_COLOR),
        _ => None,
    }
}

/// Convert an in-memory ASS scene (the sbsp exporters' output) into preview
/// geometry: every placed mesh instance transformed into world units, batched
/// per material, and grouped into `render` / `collision` / `portals` /
/// `weather` regions so each layer gets its own toggle.
fn ass_to_preview(ass: &AssFile, render_only: bool) -> RenderModelPreview {
    let mut preview = empty_preview();
    // One preview material per ASS material, so the flat palette still varies
    // across a BSP's shaders. Untextured on purpose: the ASS keeps shader
    // basenames, not resolvable tag paths.
    preview.materials = ass
        .materials
        .iter()
        .map(|_| RenderModelPreviewMaterial::default())
        .collect();
    if preview.materials.is_empty() {
        preview
            .materials
            .push(RenderModelPreviewMaterial::default());
    }

    for instance in &ass.instances {
        let Some(object) = usize::try_from(instance.object_index)
            .ok()
            .and_then(|index| ass.objects.get(index))
        else {
            continue;
        };
        let AssObjectPayload::Mesh {
            vertices,
            triangles,
        } = &object.payload
        else {
            continue;
        };
        if vertices.is_empty() || triangles.is_empty() {
            continue;
        }
        let region = ass_object_region(ass, triangles);
        if render_only && region != "render" {
            continue;
        }
        let color = layer_color(region);

        let Ok(vertex_base) = u32::try_from(preview.vertices.len()) else {
            break;
        };
        if vertices.len() + preview.vertices.len() > u32::MAX as usize {
            break;
        }
        for vertex in vertices {
            let p = point(&vertex.position);
            let scaled = [
                p[0] * instance.local_scale,
                p[1] * instance.local_scale,
                p[2] * instance.local_scale,
            ];
            let rotated = rotate(&instance.local_rotation, scaled);
            let translation = point(&instance.local_translation);
            let position = [
                (rotated[0] + translation[0]) / JMS_SCALE,
                (rotated[1] + translation[1]) / JMS_SCALE,
                (rotated[2] + translation[2]) / JMS_SCALE,
            ];
            let normal = rotate(
                &instance.local_rotation,
                [vertex.normal.i, vertex.normal.j, vertex.normal.k],
            );
            expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
            preview.vertices.push(RenderModelPreviewVertex {
                position,
                normal,
                ..Default::default()
            });
        }

        // Batch per material run, so per-shader palette colors survive.
        let mut by_material: Vec<(i32, Vec<u32>)> = Vec::new();
        for triangle in triangles {
            if triangle.v.iter().any(|&v| v as usize >= vertices.len()) {
                continue;
            }
            let position = by_material
                .iter()
                .position(|(material, _)| *material == triangle.material)
                .unwrap_or_else(|| {
                    by_material.push((triangle.material, Vec::new()));
                    by_material.len() - 1
                });
            let slot = &mut by_material[position].1;
            slot.extend(triangle.v.iter().map(|&v| vertex_base + v));
        }
        for (material, indices) in by_material {
            if indices.is_empty() {
                continue;
            }
            let index_start = preview.indices.len() as u32;
            let index_count = indices.len() as u32;
            preview.indices.extend(indices);
            preview.batches.push(RenderModelPreviewBatch {
                region_name: region.to_owned(),
                permutation_name: "default".to_owned(),
                material_index: material.clamp(0, preview.materials.len() as i32 - 1) as u16,
                index_start,
                index_count,
                flat_color: color,
                layer: ModelPreviewLayer::Render,
            });
        }
        ensure_preview_region(&mut preview, region);
    }
    preview
}

/// A structure BSP's geometry, per engine: H3-family and H2 through the ASS
/// scene builders, H1 through its JMS render/collision extractors.
/// `render_only` drops the collision/portal/weather layers — the scenario
/// composite wants just the visible world.
pub(super) fn build_sbsp_preview(
    tag: &TagFile,
    render_only: bool,
) -> Result<RenderModelPreview, String> {
    let mut preview = match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => {
            let mut preview = empty_preview();
            match JmsFile::from_scenario_structure_bsp_ce(tag) {
                Ok(jms) => append_jms_triangles(&mut preview, &jms, "render", None, true),
                Err(error) => return Err(error.to_string()),
            }
            if !render_only && let Ok(jms) = JmsFile::from_scenario_structure_bsp_ce_collision(tag)
            {
                append_jms_triangles(
                    &mut preview,
                    &jms,
                    COLLISION_REGION,
                    Some(COLLISION_COLOR),
                    false,
                );
            }
            preview
        }
        blam_tags::game::Game::Halo2 => ass_to_preview(
            &AssFile::from_scenario_structure_bsp_h2(tag).map_err(|error| error.to_string())?,
            render_only,
        ),
        blam_tags::game::Game::Halo3 => {
            // Campaign Evolved's Blam/Unreal hybrid BSPs ship no render
            // geometry at all (`per mesh temporary` is empty; Unreal owns
            // everything rendered) — the ASS builder already knows to source
            // their content from collision, so those stay on that path.
            let has_render_geometry = tag
                .root()
                .field_path("render geometry/per mesh temporary")
                .and_then(|field| field.as_block())
                .is_some_and(|block| !block.is_empty());
            if has_render_geometry {
                build_sbsp_preview_h3(tag, render_only)?
            } else {
                ass_to_preview(
                    &AssFile::from_scenario_structure_bsp(tag)
                        .map_err(|error| error.to_string())?,
                    render_only,
                )
            }
        }
    };
    append_model_errors(tag, &mut preview, ModelPreviewLayer::Render);
    finish_preview(preview, "structure BSP")
}

/// The H3-family structure BSP, decoded natively so the render layer keeps
/// what the ASS text format throws away: real UVs, the authored tangent
/// frames, and the `materials` block's shader references. That is what lets
/// the existing shader→texture pipeline shade a BSP exactly like a model —
/// diffuse, normal maps, and alpha-test cutouts included.
fn build_sbsp_preview_h3(tag: &TagFile, render_only: bool) -> Result<RenderModelPreview, String> {
    let root = tag.root();
    let mut preview = empty_preview();
    preview.materials = sbsp_preview_materials(&root);
    if preview.materials.is_empty() {
        preview
            .materials
            .push(RenderModelPreviewMaterial::default());
    }

    let clusters = root
        .field_path("clusters")
        .and_then(|field| field.as_block())
        .ok_or("This structure BSP has no clusters block.")?;
    let defs = root
        .field_path(
            "resource interface/raw_resources[0]/raw_items/instanced geometries definitions",
        )
        .and_then(|field| field.as_block());
    let instances = root
        .field_path("instanced geometry instances")
        .and_then(|field| field.as_block());

    // Which compression bounds decode which mesh. Cluster meshes are stored
    // in world units (identity); each instanced-geometry definition names its
    // own `compression index`. An odd number of negative-span axes flips the
    // unpacker's winding, which the append below undoes per triangle.
    let mut mesh_compression: HashMap<usize, usize> = HashMap::new();
    if let Some(defs) = &defs {
        for index in 0..defs.len() {
            let def = defs.element(index).unwrap();
            let mesh_index = def.read_int_any("mesh index").unwrap_or(-1);
            let compression = def.read_int_any("compression index").unwrap_or(0).max(0) as usize;
            if mesh_index >= 0 {
                mesh_compression.insert(mesh_index as usize, compression);
            }
        }
    }
    let meshes = extract_sbsp_render_geometry_meshes(&root, |mesh_index| {
        match mesh_compression.get(&mesh_index) {
            Some(&compression) => read_compression_bounds_at(&root, compression),
            None => CompressionBounds::identity(),
        }
    })
    .map_err(|error| error.to_string())?;

    // Everything lands in one `render` region, batched per material at the
    // end: a level is thousands of cluster/instance parts, and one draw call
    // per part would be the slow way to spend a frame.
    let mut render_indices: Vec<Vec<u32>> = vec![Vec::new(); preview.materials.len()];
    let identity = InstancePlacement::identity();
    for index in 0..clusters.len() {
        let cluster = clusters.element(index).unwrap();
        let mesh_index = cluster.read_int_any("mesh index").unwrap_or(-1);
        let Some(mesh) = usize::try_from(mesh_index).ok().and_then(|i| meshes.get(i)) else {
            continue;
        };
        append_sbsp_mesh(&mut preview, &mut render_indices, mesh, &identity, false);
    }
    if let (Some(defs), Some(instances)) = (&defs, &instances) {
        for index in 0..instances.len() {
            let instance = instances.element(index).unwrap();
            let def_index = instance.read_int_any("instance definition").unwrap_or(-1);
            let Some(def) = usize::try_from(def_index)
                .ok()
                .and_then(|i| (i < defs.len()).then(|| defs.element(i).unwrap()))
            else {
                continue;
            };
            let mesh_index = def.read_int_any("mesh index").unwrap_or(-1);
            let Some(mesh) = usize::try_from(mesh_index).ok().and_then(|i| meshes.get(i)) else {
                continue;
            };
            let flip = mesh_compression
                .get(&(mesh_index as usize))
                .map(|&compression| {
                    bounds_axis_flip(&read_compression_bounds_at(&root, compression))
                })
                .unwrap_or(false);
            let placement = InstancePlacement {
                forward: vector(&instance.read_vec3("forward")),
                left: vector(&instance.read_vec3("left")),
                up: vector(&instance.read_vec3("up")),
                position: point(&instance.read_point3d("position")),
                scale: instance.read_real("scale").unwrap_or(1.0),
            };
            append_sbsp_mesh(&mut preview, &mut render_indices, mesh, &placement, flip);
        }
    }
    for (material_index, indices) in render_indices.into_iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        let index_start = preview.indices.len() as u32;
        let index_count = indices.len() as u32;
        preview.indices.extend(indices);
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "render".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: material_index.min(u16::MAX as usize) as u16,
            index_start,
            index_count,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
        });
    }
    if !preview.batches.is_empty() {
        ensure_preview_region(&mut preview, "render");
    }

    if !render_only {
        append_sbsp_portals(&root, &mut preview);
        append_sbsp_collision(&root, &mut preview);
    }
    Ok(preview)
}

/// The sbsp `materials` block's shader references, index-aligned with the
/// mesh parts' `material_index` — the same contract the render_model preview
/// keeps, so `resolve_model_textures` needs nothing new.
fn sbsp_preview_materials(root: &TagStruct<'_>) -> Vec<RenderModelPreviewMaterial> {
    let Some(block) = root
        .field_path("materials")
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    (0..block.len())
        .map(|index| {
            block
                .element(index)
                .and_then(|element| element.read_tag_ref_with_group("render method"))
                .filter(|(_, path)| !path.trim().is_empty())
                .map(|(shader_group, path)| RenderModelPreviewMaterial {
                    shader_path: path.replace('/', "\\"),
                    shader_group,
                })
                .unwrap_or_default()
        })
        .collect()
}

/// One instanced-geometry placement: basis columns, position, uniform scale.
struct InstancePlacement {
    forward: [f32; 3],
    left: [f32; 3],
    up: [f32; 3],
    position: [f32; 3],
    scale: f32,
}

impl InstancePlacement {
    fn identity() -> Self {
        Self {
            forward: [1.0, 0.0, 0.0],
            left: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            position: [0.0; 3],
            scale: 1.0,
        }
    }

    fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let s = if self.scale.is_finite() && self.scale != 0.0 {
            self.scale
        } else {
            1.0
        };
        [
            self.position[0]
                + s * (self.forward[0] * p[0] + self.left[0] * p[1] + self.up[0] * p[2]),
            self.position[1]
                + s * (self.forward[1] * p[0] + self.left[1] * p[1] + self.up[1] * p[2]),
            self.position[2]
                + s * (self.forward[2] * p[0] + self.left[2] * p[1] + self.up[2] * p[2]),
        ]
    }

    fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        [
            self.forward[0] * v[0] + self.left[0] * v[1] + self.up[0] * v[2],
            self.forward[1] * v[0] + self.left[1] * v[1] + self.up[1] * v[2],
            self.forward[2] * v[0] + self.left[2] * v[1] + self.up[2] * v[2],
        ]
    }
}

fn vector(v: &RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}

/// Whether decompressing through these bounds mirrors an odd number of axes,
/// inverting triangle winding against the stored normals.
fn bounds_axis_flip(bounds: &CompressionBounds) -> bool {
    if !bounds.pos_compressed {
        return false;
    }
    [
        bounds.px_max < bounds.px_min,
        bounds.py_max < bounds.py_min,
        bounds.pz_max < bounds.pz_min,
    ]
    .into_iter()
    .filter(|flipped| *flipped)
    .count()
        % 2
        == 1
}

/// Append one decoded render mesh under `placement`, keeping UVs and rotating
/// the full tangent frame, and file its triangles into the per-material index
/// lists. `flip` swaps winding for meshes whose compression bounds mirror.
fn append_sbsp_mesh(
    preview: &mut RenderModelPreview,
    render_indices: &mut [Vec<u32>],
    mesh: &blam_tags::render_model::RenderMesh,
    placement: &InstancePlacement,
    flip: bool,
) {
    let Ok(vertex_base) = u32::try_from(preview.vertices.len()) else {
        return;
    };
    if mesh.vertices.len() + preview.vertices.len() > u32::MAX as usize {
        return;
    }
    preview.vertices.reserve(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let position = placement.apply(point(&vertex.position));
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: placement.rotate(vector(&vertex.normal)),
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: placement.rotate(vector(&vertex.tangent)),
            binormal: placement.rotate(vector(&vertex.binormal)),
            // BSPs are static; zero weights keep the skinning path inert.
            ..Default::default()
        });
    }
    for part in &mesh.parts {
        let start = part.index_start as usize;
        let end = start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        if start >= end {
            continue;
        }
        let slot = (part.material_index as usize).min(render_indices.len().saturating_sub(1));
        let Some(indices) = render_indices.get_mut(slot) else {
            continue;
        };
        for triangle in mesh.indices[start..end].chunks_exact(3) {
            if triangle.iter().any(|&i| i as usize >= mesh.vertices.len()) {
                continue;
            }
            let (b, c) = if flip { (2, 1) } else { (1, 2) };
            indices.push(vertex_base + triangle[0]);
            indices.push(vertex_base + triangle[b]);
            indices.push(vertex_base + triangle[c]);
        }
    }
}

/// Cluster portals as a fan-triangulated `portals` layer. Portal points are
/// stored in world units, so no rescale.
fn append_sbsp_portals(root: &TagStruct<'_>, preview: &mut RenderModelPreview) {
    let Some(portals) = root
        .field_path("cluster portals")
        .and_then(|field| field.as_block())
    else {
        return;
    };
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::new();
    for index in 0..portals.len() {
        let portal = portals.element(index).unwrap();
        let Some(vertices) = portal.field("vertices").and_then(|field| field.as_block()) else {
            continue;
        };
        let ring: Vec<[f32; 3]> = (0..vertices.len())
            .filter_map(|vi| vertices.element(vi))
            .map(|element| point(&element.read_point3d("point")))
            .collect();
        for k in 1..ring.len().saturating_sub(1) {
            let normal = face_normal(ring[0], ring[k], ring[k + 1]);
            triples.push((ring[0], normal));
            triples.push((ring[k], normal));
            triples.push((ring[k + 1], normal));
        }
    }
    push_derived_batch(preview, "portals", Some(PORTAL_COLOR), &triples);
}

/// The sealed-world collision BSP as a `collision` layer, walked straight off
/// the winged-edge blocks. H3/ODST keep it in `collision bsp`; Reach-era
/// tags moved it to `large collision bsp` — read whichever are present.
fn append_sbsp_collision(root: &TagStruct<'_>, preview: &mut RenderModelPreview) {
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::new();
    for name in ["collision bsp", "large collision bsp"] {
        let Some(block) = root
            .field_path(&format!(
                "resource interface/raw_resources[0]/raw_items/{name}"
            ))
            .and_then(|field| field.as_block())
        else {
            continue;
        };
        for index in 0..block.len() {
            let bsp = block.element(index).unwrap();
            append_collision_bsp_triangles(&bsp, &mut triples);
        }
    }
    push_derived_batch(preview, COLLISION_REGION, Some(COLLISION_COLOR), &triples);
}

/// Walk one collision BSP's surfaces: each surface rings its edges (an edge
/// belongs to two surfaces; which side it is on decides start-vs-end vertex
/// and forward-vs-reverse continuation), then fan-triangulates the ring.
fn append_collision_bsp_triangles(bsp: &TagStruct<'_>, triples: &mut Vec<([f32; 3], [f32; 3])>) {
    let (Some(surfaces), Some(edges), Some(vertices)) = (
        bsp.field_path("surfaces")
            .and_then(|field| field.as_block()),
        bsp.field_path("edges").and_then(|field| field.as_block()),
        bsp.field_path("vertices")
            .and_then(|field| field.as_block()),
    ) else {
        return;
    };
    type EdgeRow = (i128, i128, i128, i128, i128, i128);
    let read_edge = |index: i128| -> Option<EdgeRow> {
        let edge = edges.element(usize::try_from(index).ok()?)?;
        Some((
            edge.read_int_any("start vertex").unwrap_or(-1),
            edge.read_int_any("end vertex").unwrap_or(-1),
            edge.read_int_any("forward edge").unwrap_or(-1),
            edge.read_int_any("reverse edge").unwrap_or(-1),
            edge.read_int_any("left surface").unwrap_or(-1),
            edge.read_int_any("right surface").unwrap_or(-1),
        ))
    };
    for surface_index in 0..surfaces.len() {
        let surface = surfaces.element(surface_index).unwrap();
        let first_edge = surface.read_int_any("first edge").unwrap_or(-1);
        if first_edge < 0 {
            continue;
        }
        let si = surface_index as i128;
        let mut ring: Vec<[f32; 3]> = Vec::new();
        let mut edge_index = first_edge;
        // Malformed rings never terminate; the step bound is the bail-out.
        let max_steps = edges.len() * 2 + 8;
        for _ in 0..max_steps {
            let Some((start, end, forward, reverse, left, right)) = read_edge(edge_index) else {
                ring.clear();
                break;
            };
            let (vertex_index, next) = if left == si {
                (start, forward)
            } else if right == si {
                (end, reverse)
            } else {
                ring.clear();
                break;
            };
            let Some(vertex) = usize::try_from(vertex_index)
                .ok()
                .and_then(|vi| vertices.element(vi))
            else {
                ring.clear();
                break;
            };
            ring.push(point(&vertex.read_point3d("point")));
            if next == first_edge {
                break;
            }
            edge_index = next;
        }
        for k in 1..ring.len().saturating_sub(1) {
            let normal = face_normal(ring[0], ring[k], ring[k + 1]);
            triples.push((ring[0], normal));
            triples.push((ring[k], normal));
            triples.push((ring[k + 1], normal));
        }
    }
}

/// The scenario's `structure bsps` block as backslash reference paths, in tag
/// order. Elements whose reference is empty stay in the list as `None` so the
/// panel's indices line up with the tag's.
pub(super) fn scenario_bsp_paths(scenario: &TagFile) -> Vec<Option<String>> {
    let root = scenario.root();
    let Some(block) = root
        .field_path("structure bsps")
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    (0..block.len())
        .map(|index| {
            block
                .element(index)
                .and_then(|element| tag_ref_path(&element, "structure bsp"))
        })
        .collect()
}

/// Append every region of `src` into `dst`, offsetting vertex, index, and
/// material references. Regions with a name `dst` already lists are merged
/// into the existing entry.
pub(super) fn merge_preview_append(dst: &mut RenderModelPreview, src: &RenderModelPreview) {
    let Ok(vertex_base) = u32::try_from(dst.vertices.len()) else {
        return;
    };
    if src.vertices.len() + dst.vertices.len() > u32::MAX as usize {
        return;
    }
    let material_base = dst.materials.len();
    dst.materials.extend(src.materials.iter().cloned());
    dst.errors.extend(src.errors.iter().cloned());
    for vertex in &src.vertices {
        expand_preview_bounds_local(&mut dst.bounds_min, &mut dst.bounds_max, vertex.position);
        dst.vertices.push(*vertex);
    }
    for batch in &src.batches {
        let start = batch.index_start as usize;
        let end = start
            .saturating_add(batch.index_count as usize)
            .min(src.indices.len());
        if start >= end {
            continue;
        }
        let index_start = dst.indices.len() as u32;
        dst.indices.extend(
            src.indices[start..end]
                .iter()
                .map(|index| index + vertex_base),
        );
        dst.batches.push(RenderModelPreviewBatch {
            region_name: batch.region_name.clone(),
            permutation_name: batch.permutation_name.clone(),
            material_index: (batch.material_index as usize + material_base).min(u16::MAX as usize)
                as u16,
            index_start,
            index_count: (end - start) as u32,
            flat_color: batch.flat_color,
            layer: batch.layer,
        });
    }
    for region in &src.regions {
        if let Some(existing) = dst
            .regions
            .iter_mut()
            .find(|existing| existing.name == region.name)
        {
            for permutation in &region.permutations {
                if !existing.permutations.contains(permutation) {
                    existing.permutations.push(permutation.clone());
                }
            }
        } else {
            dst.regions.push(region.clone());
        }
    }
}

/// Rename every region and batch of a preview to one region — the shape the
/// scenario composite wants, where each BSP is a single toggle.
pub(super) fn rebrand_preview_region(preview: &mut RenderModelPreview, region_name: &str) {
    for batch in &mut preview.batches {
        batch.region_name = region_name.to_owned();
        batch.permutation_name = "default".to_owned();
    }
    preview.regions = vec![RenderModelPreviewRegion {
        name: region_name.to_owned(),
        permutations: vec!["default".to_owned()],
    }];
}

/// The `.model`'s collision layer, posed on its own skeleton, ready to merge
/// over the render preview. `None` when the reference is absent or unreadable
/// — a missing overlay degrades to nothing rather than failing the preview.
pub(super) fn hlmt_collision_overlay(
    model_tag: &TagFile,
    source: &TagSource,
) -> Option<RenderModelPreview> {
    let reference = tag_ref_path(&model_tag.root(), "collision model")?;
    let collision =
        load_referenced_tag_from_source(source, &reference, "collision_model", b"coll").ok()?;
    let skeleton = model_skeleton(source, model_tag);
    build_collision_preview(&collision, skeleton.as_ref().map(|s| s.nodes())).ok()
}

/// Halo CE's object tag is its model wrapper. Resolve the gbxmodel bind pose
/// with CE's corrected quaternion convention and use it to place the direct
/// collision-model reference.
pub(super) fn halo1_object_collision_overlay(
    object_tag: &TagFile,
    source: &TagSource,
) -> Option<RenderModelPreview> {
    let model_reference = halo1_object_reference(object_tag, "model")?;
    let collision_reference = halo1_object_reference(object_tag, "collision model")?;
    let render =
        load_referenced_tag_from_source(source, &model_reference, "gbxmodel", b"mod2").ok()?;
    let collision = load_referenced_tag_from_source(
        source,
        &collision_reference,
        "model_collision_geometry",
        b"coll",
    )
    .ok()?;
    let nodes = render_model_skeleton(&render).ok()?;
    build_collision_preview(&collision, Some(&nodes)).ok()
}

/// The `.model`'s physics layer, likewise. Reads both spellings the H2-era
/// definitions used (`physics_model` and `physics model`); the legacy H2
/// `physics` (`phys`) reference is deliberately not resolved — it is not a
/// `phmo` and has no shapes to draw.
pub(super) fn hlmt_physics_overlay(
    model_tag: &TagFile,
    source: &TagSource,
) -> Option<RenderModelPreview> {
    let root = model_tag.root();
    let reference =
        tag_ref_path(&root, "physics_model").or_else(|| tag_ref_path(&root, "physics model"))?;
    let physics =
        load_referenced_tag_from_source(source, &reference, "physics_model", b"phmo").ok()?;
    let skeleton = model_skeleton(source, model_tag);
    build_physics_preview(&physics, skeleton.as_ref().map(|s| s.nodes())).ok()
}

impl Baboon {
    /// Start building a loaded `.model` preview's collision/physics overlays
    /// on a worker, once per load.
    ///
    /// Building them inline froze the frame: every toggle re-read the
    /// collision and physics tags, re-walked the collision BSP, and
    /// re-tessellated the shapes — on the UI thread, in both directions.
    /// Built here instead, the merged geometry sits in memory for the
    /// document's lifetime and the toggles become draw-time filters.
    /// Idempotent and cheap to call every frame, like the texture request
    /// beside it.
    pub(in crate::app) fn maybe_request_model_overlays(
        &mut self,
        kit_index: usize,
        key: &str,
        ctx: &egui::Context,
    ) -> bool {
        let kit = &self.model.kits[kit_index];
        let view = &self.views[kit.id];
        let Some(state) = view.caches.model_previews.get(key) else {
            return false;
        };
        if state.overlays_loaded || state.overlays_pending {
            return false;
        }
        let Some(Ok(data)) = state.data.as_ref() else {
            return false;
        };
        let geometry_id = data.geometry_id;
        let Some(entry) = kit.entry_for_key(key).cloned() else {
            return false;
        };
        if entry.group_tag != u32::from_be_bytes(*b"hlmt")
            && !is_object_family_group(entry.group_tag)
        {
            return false;
        }
        let Some(source) = kit.source.as_ref().map(|source| source.source.clone()) else {
            return false;
        };
        let stamp = KitStamp {
            kit: kit.id,
            generation: kit.generation,
        };
        if let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(key) {
            state.overlays_pending = true;
        }

        // Through `spawn_worker` like every preview worker: a panicking tag
        // must still send its message or the pending flag sticks and the
        // overlays never arrive.
        let (key, panic_key) = (key.to_owned(), key.to_owned());
        let build = move || -> Option<_> {
                let model = crate::core::source::read_entry(&source, &entry).ok()?;
                if blam_tags::game::Game::of(&model) == blam_tags::game::Game::Halo1
                    && is_object_family_group(model.header.group_tag)
                {
                    return Some((halo1_object_collision_overlay(&model, &source), None));
                }
                // Classic models only: a Campaign Evolved hlmt previews
                // through Unreal geometry and hides the overlay toggles.
                model.root().read_tag_ref_with_group("render model")?;
                Some((
                    hlmt_collision_overlay(&model, &source),
                    hlmt_physics_overlay(&model, &source),
                ))
        };
        spawn_worker(
            &self.tx,
            ctx,
            move || {
                let (collision, physics) = build().unwrap_or((None, None));
                WorkerMessage::ModelOverlaysBuilt { stamp, key, geometry_id, collision, physics }
            },
            move |_| WorkerMessage::ModelOverlaysBuilt {
                stamp,
                key: panic_key,
                geometry_id,
                collision: None,
                physics: None,
            },
        );
        true
    }

    /// Merge a worker's overlay geometry into the preview it was built for.
    pub(in crate::app) fn handle_model_overlays_built(
        &mut self,
        stamp: KitStamp,
        key: String,
        geometry_id: u64,
        collision: Option<RenderModelPreview>,
        physics: Option<RenderModelPreview>,
    ) -> bool {
        let Some(kit_index) = self.model.resolve_kit(stamp.kit) else {
            return true;
        };
        let stale = self.model.resolve_stamp(stamp).is_none();
        let Some(state) = self.views[self.model.kits[kit_index].id].caches.model_previews.get_mut(&key) else {
            return true;
        };
        // The in-flight marker is cleared before the staleness check: a result
        // dropped for a generation bump used to leave it set, and the preview
        // then waited for it for good (the overlays never arrived).
        state.overlays_pending = false;
        if stale {
            return true;
        }
        {
            let Some(Ok(data)) = state.data.as_mut() else {
                return true;
            };
            if data.geometry_id != geometry_id {
                // The preview reloaded while this built; the reload re-armed
                // the request, so a fresh build is already on its way.
                return true;
            }
            state.overlays_loaded = true;
            if collision.is_none() && physics.is_none() {
                return false;
            }
            let mut merged = (*data.preview).clone();
            if let Some(overlay) = &collision {
                merge_preview_append(&mut merged, overlay);
            }
            if let Some(overlay) = &physics {
                merge_preview_append(&mut merged, overlay);
            }
            data.preview = Arc::new(merged);
            data.geometry_id = NEXT_MODEL_GEOMETRY_ID.fetch_add(1, Ordering::Relaxed);
        }
        // The layer filter also consults region selection, so collision-only
        // regions need entries. Shared names retain the render model's active
        // permutation and therefore follow the same variant selection.
        let regions = state
            .data
            .as_ref()
            .and_then(|data| data.as_ref().ok())
            .map(|data| data.preview.regions.clone())
            .unwrap_or_default();
        for region in regions {
            state
                .region_selections
                .entry(region.name)
                .or_insert(ModelRegionSelection {
                    enabled: true,
                    permutation: region
                        .permutations
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "default".to_owned()),
                });
        }
        // Textures are keyed by `textures_id`, which the merge leaves alone:
        // a resolve in flight still lands, and resolved ones stay uploaded.
        false
    }
}

/// The leaf name a BSP toggle shows: the last path segment of its reference.
pub(super) fn bsp_display_name(reference: &str) -> String {
    reference
        .rsplit(['\\', '/'])
        .next()
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or(reference)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::math::RealRgbColor;
    use blam_tags::{
        AssInstance, AssMaterial, AssObject, AssVertex, JmsMaterial, JmsNode, JmsTriangle, JmsVertex,
    };

    // The derived-preview builders' non-drawing halves: JMS/ASS scenes into
    // preview geometry, physics primitives into meshes, and the merge/rename
    // plumbing the hlmt overlays and the scenario composite stand on.
    //
    // Everything here is pure geometry — no tag files, no GL — because that is
    // the half that can be wrong quietly: a missed ÷100 draws a BSP a hundred
    // times too big, a bad winding turns a hull inside out, and both look like
    // "the preview is broken" with no error anywhere.

    #[test]
    fn overlay_colors_match_their_tag_icons() {
        assert_eq!(COLLISION_COLOR, [0xED, 0x5E, 0xBE]);
        assert_eq!(PHYSICS_COLOR, [0xFF, 0x56, 0x56]);
    }

    fn jms_vertex(x: f32, y: f32, z: f32) -> JmsVertex {
        JmsVertex {
            position: RealPoint3d { x, y, z },
            normal: RealVector3d {
                i: 0.0,
                j: 0.0,
                k: 1.0,
            },
            tangent: None,
            binormal: None,
            node_sets: Vec::new(),
            uvs: Vec::new(),
            color: None,
        }
    }

    /// A JMS mesh lands ÷100 in world units, in its named region, with the flat
    /// overlay color — and with a *recomputed* face normal, because the collision
    /// JMS writer emits a constant `(0,0,1)` that would shade every wall flat.
    #[test]
    fn a_jms_collision_mesh_scales_down_and_recomputes_its_normals() {
        let jms = JmsFile {
            // A wall in the XZ plane: its face normal is ±Y, nothing like the
            // constant +Z the vertices claim.
            vertices: vec![
                jms_vertex(0.0, 0.0, 0.0),
                jms_vertex(100.0, 0.0, 0.0),
                jms_vertex(0.0, 0.0, 100.0),
            ],
            triangles: vec![JmsTriangle {
                material: 0,
                v: [0, 1, 2],
                region: 0,
            }],
            ..Default::default()
        };
        let mut preview = empty_preview();
        append_jms_triangles(
            &mut preview,
            &jms,
            COLLISION_REGION,
            Some(COLLISION_COLOR),
            false,
        );

        assert_eq!(preview.vertices.len(), 3);
        assert_eq!(preview.vertices[1].position, [1.0, 0.0, 0.0], "÷100");
        let normal = preview.vertices[0].normal;
        assert!(
            normal[1].abs() > 0.99 && normal[0].abs() < 0.01 && normal[2].abs() < 0.01,
            "the face normal must be recomputed, not the writer's constant +Z: {normal:?}"
        );
        assert_eq!(preview.batches.len(), 1);
        assert_eq!(preview.batches[0].region_name, COLLISION_REGION);
        assert_eq!(preview.batches[0].flat_color, Some(COLLISION_COLOR));
        assert_eq!(preview.regions.len(), 1, "the region doubles as the toggle");
    }

    #[test]
    fn collision_cells_keep_variant_names_and_remap_bones_by_name() {
        let mut vertices = vec![
            jms_vertex(0.0, 0.0, 0.0),
            jms_vertex(100.0, 0.0, 0.0),
            jms_vertex(0.0, 100.0, 0.0),
        ];
        for vertex in &mut vertices {
            vertex.node_sets = vec![(0, 1.0)];
        }
        let collision_node = JmsNode {
            name: "spine".to_owned(),
            parent: -1,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d::ZERO,
        };
        let target_skeleton = vec![
            JmsNode {
                name: "pelvis".to_owned(),
                ..collision_node.clone()
            },
            collision_node.clone(),
        ];
        let jms = JmsFile {
            nodes: vec![collision_node],
            // CE JMS keeps its region table for file output as well as carrying
            // the permutation/region cell in the material label. The cell label
            // must win or every triangle becomes the first region + `default`.
            regions: vec!["wrong fallback".to_owned()],
            materials: vec![JmsMaterial {
                name: "metal".to_owned(),
                material_name: "(1) major armor".to_owned(),
            }],
            vertices,
            triangles: vec![JmsTriangle {
                material: 0,
                v: [0, 1, 2],
                region: 0,
            }],
            ..Default::default()
        };
        let mut preview = empty_preview();
        append_collision_jms(&mut preview, &jms, Some(&target_skeleton));

        assert_eq!(preview.batches[0].region_name, "armor");
        assert_eq!(preview.batches[0].permutation_name, "major");
        assert_eq!(preview.batches[0].layer, ModelPreviewLayer::Collision);
        assert_eq!(preview.vertices[0].node_indices[0], 1.0);
        assert_eq!(preview.vertices[0].node_weights[0], 1.0);
    }

    #[test]
    fn collision_error_points_follow_the_same_bind_pose_as_the_mesh() {
        let half_turn = std::f32::consts::FRAC_1_SQRT_2;
        let collision = JmsFile {
            nodes: vec![JmsNode {
                name: "spine".to_owned(),
                parent: -1,
                rotation: RealQuaternion::IDENTITY,
                translation: RealPoint3d::ZERO,
            }],
            ..Default::default()
        };
        let skeleton = vec![
            JmsNode {
                name: "pelvis".to_owned(),
                parent: -1,
                rotation: RealQuaternion::IDENTITY,
                translation: RealPoint3d::ZERO,
            },
            JmsNode {
                name: "spine".to_owned(),
                parent: 0,
                rotation: RealQuaternion {
                    i: half_turn,
                    j: 0.0,
                    k: 0.0,
                    w: half_turn,
                },
                translation: RealPoint3d {
                    x: 100.0,
                    y: 200.0,
                    z: 300.0,
                },
            },
        ];
        let point = ModelErrorPoint {
            position: [0.0, 1.0, 0.0],
            node_indices: [0, -1, -1, -1],
            node_weights: [1.0, 0.0, 0.0, 0.0],
        };
        let mut errors = vec![ModelErrorPrimitive {
            label: "open edge".to_owned(),
            non_critical: false,
            color: MODEL_ERROR_FALLBACK_COLOR,
            layer: ModelPreviewLayer::Collision,
            shape: ModelErrorShape::Vector {
                point,
                normal: [0.0, 1.0, 0.0],
                length: 1.0,
            },
        }];

        pose_collision_errors(&mut errors, &collision, &skeleton);

        let ModelErrorShape::Vector { point, normal, .. } = &errors[0].shape else {
            panic!("expected vector error");
        };
        assert_eq!(point.node_indices[0], 1, "bone remapped by name");
        assert!((point.position[0] - 1.0).abs() < 0.001);
        assert!((point.position[1] - 2.0).abs() < 0.001);
        assert!((point.position[2] - 4.0).abs() < 0.001);
        assert!(normal[1].abs() < 0.001 && (normal[2] - 1.0).abs() < 0.001);
    }

    #[test]
    fn physics_shapes_are_placed_and_weighted_by_their_parent_bone() {
        let source_node = JmsNode {
            name: "spine".to_owned(),
            parent: -1,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let target_skeleton = vec![
            JmsNode {
                name: "pelvis".to_owned(),
                ..source_node.clone()
            },
            source_node.clone(),
        ];
        let jms = JmsFile {
            nodes: vec![source_node],
            ..Default::default()
        };
        let mut vertices = Vec::new();
        append_physics_shape(
            &mut vertices,
            &[([0.25, 0.0, 0.0], [0.0, 0.0, 1.0])],
            0,
            &jms,
            Some(&target_skeleton),
        );

        assert_eq!(vertices[0].position, [1.25, 0.0, 0.0]);
        assert_eq!(vertices[0].node_indices[0], 1.0);
        assert_eq!(vertices[0].node_weights[0], 1.0);
    }

    /// Authored normals survive when asked for — H1 BSP render geometry carries
    /// real ones worth keeping.
    #[test]
    fn authored_normals_are_kept_when_the_source_has_real_ones() {
        let jms = JmsFile {
            vertices: vec![
                jms_vertex(0.0, 0.0, 0.0),
                jms_vertex(100.0, 0.0, 0.0),
                jms_vertex(0.0, 0.0, 100.0),
            ],
            triangles: vec![JmsTriangle {
                material: 0,
                v: [0, 1, 2],
                region: 0,
            }],
            ..Default::default()
        };
        let mut preview = empty_preview();
        append_jms_triangles(&mut preview, &jms, "render", None, true);
        assert_eq!(preview.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(preview.batches[0].flat_color, None);
    }

    #[test]
    fn a_sphere_tessellates_onto_its_own_surface() {
        let mut triples = Vec::new();
        push_sphere(
            &mut triples,
            &RealQuaternion::IDENTITY,
            [100.0, 0.0, 0.0],
            50.0,
        );
        assert!(!triples.is_empty());
        for (position, normal) in &triples {
            // Centered at 1.0 world units, radius 0.5.
            let d = ((position[0] - 1.0).powi(2) + position[1].powi(2) + position[2].powi(2)).sqrt();
            assert!(
                (d - 0.5).abs() < 0.01,
                "vertex off the sphere: {position:?}"
            );
            let len = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt();
            assert!((len - 1.0).abs() < 0.01, "non-unit normal");
        }
    }

    #[test]
    fn a_box_tessellates_to_its_full_extents() {
        let shape = blam_tags::JmsBox {
            name: String::new(),
            parent: -1,
            material: 0,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            width: 200.0,
            length: 100.0,
            height: 50.0,
        };
        let mut triples = Vec::new();
        push_box(&mut triples, &shape);
        assert_eq!(triples.len(), 36, "6 faces × 2 triangles × 3 vertices");
        let max = |axis: usize| {
            triples
                .iter()
                .map(|(p, _)| p[axis].abs())
                .fold(0.0f32, f32::max)
        };
        // Full extents 200/100/50 cm → half extents 1.0/0.5/0.25 world units.
        assert!((max(0) - 1.0).abs() < 1e-4);
        assert!((max(1) - 0.5).abs() < 1e-4);
        assert!((max(2) - 0.25).abs() < 1e-4);
    }

    #[test]
    fn a_capsule_spans_bottom_cap_to_top_cap() {
        let capsule = blam_tags::JmsCapsule {
            name: String::new(),
            parent: -1,
            material: 0,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            height: 100.0,
            radius: 25.0,
        };
        let mut triples = Vec::new();
        push_capsule(&mut triples, &capsule);
        assert!(!triples.is_empty());
        let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);
        for (position, _) in &triples {
            min_z = min_z.min(position[2]);
            max_z = max_z.max(position[2]);
        }
        // Anchored at the bottom-cap center: caps extend a radius past each end.
        assert!((min_z - -0.25).abs() < 0.01, "bottom cap at {min_z}");
        assert!((max_z - 1.25).abs() < 0.01, "top cap at {max_z}");
    }

    /// Every hull face must point away from the shape's centroid — an inside-out
    /// hull culls to nothing and reads as "physics preview is empty".
    #[test]
    fn a_convex_hull_winds_every_face_outward() {
        let corners = [
            [-100.0, -100.0, -100.0],
            [100.0, -100.0, -100.0],
            [-100.0, 100.0, -100.0],
            [100.0, 100.0, -100.0],
            [-100.0, -100.0, 100.0],
            [100.0, -100.0, 100.0],
            [-100.0, 100.0, 100.0],
            [100.0, 100.0, 100.0],
        ];
        let convex = blam_tags::JmsConvex {
            name: String::new(),
            parent: -1,
            material: 0,
            rotation: RealQuaternion::IDENTITY,
            translation: RealPoint3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            vertices: corners
                .iter()
                .map(|[x, y, z]| RealPoint3d {
                    x: *x,
                    y: *y,
                    z: *z,
                })
                .collect(),
        };
        let mut triples = Vec::new();
        push_convex(&mut triples, &convex);
        assert!(!triples.is_empty(), "a cube's corners must produce a hull");
        for face in triples.chunks_exact(3) {
            let (a, b, c) = (face[0].0, face[1].0, face[2].0);
            let normal = face[0].1;
            let center = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            // The hull is centered on the origin, so outward == away from origin.
            let outward = center[0] * normal[0] + center[1] * normal[1] + center[2] * normal[2];
            assert!(outward > 0.0, "face wound inward: {center:?} {normal:?}");
        }
    }

    fn ass_vertex(x: f32, y: f32, z: f32) -> AssVertex {
        AssVertex {
            position: RealPoint3d { x, y, z },
            normal: RealVector3d {
                i: 0.0,
                j: 0.0,
                k: 1.0,
            },
            color: RealRgbColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
            },
            node_set: Vec::new(),
            uvs: Vec::new(),
        }
    }

    fn ass_mesh(material: i32) -> AssObject {
        AssObject {
            xref_filepath: String::new(),
            xref_objectname: String::new(),
            payload: AssObjectPayload::Mesh {
                vertices: vec![
                    ass_vertex(0.0, 0.0, 0.0),
                    ass_vertex(100.0, 0.0, 0.0),
                    ass_vertex(0.0, 100.0, 0.0),
                ],
                triangles: vec![AssTriangle {
                    material,
                    v: [0, 1, 2],
                }],
            },
        }
    }

    fn ass_material(name: &str) -> AssMaterial {
        AssMaterial {
            name: name.to_owned(),
            lightmap_variant: String::new(),
            bm_strings: Vec::new(),
        }
    }

    /// The special ASS layers land in their own regions with their own colors;
    /// ordinary shaders land in `render`, scaled and placed by their instance.
    #[test]
    fn ass_scenes_split_into_layer_regions_and_apply_instance_transforms() {
        let ass = AssFile {
            materials: vec![ass_material("some_shader"), ass_material("+portal")],
            objects: vec![ass_mesh(0), ass_mesh(1)],
            instances: vec![
                AssInstance {
                    object_index: 0,
                    local_translation: RealPoint3d {
                        x: 100.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    ..Default::default()
                },
                AssInstance {
                    object_index: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let preview = ass_to_preview(&ass, false);

        let regions: Vec<&str> = preview
            .regions
            .iter()
            .map(|region| region.name.as_str())
            .collect();
        assert_eq!(regions, ["render", "portals"]);
        // The render instance sits 100 cm along X: its first vertex lands ÷100 at
        // exactly one world unit.
        assert_eq!(preview.vertices[0].position, [1.0, 0.0, 0.0]);
        let portal_batch = preview
            .batches
            .iter()
            .find(|batch| batch.region_name == "portals")
            .expect("portal layer batch");
        assert!(
            portal_batch.flat_color.is_some(),
            "layers keep fixed colors"
        );
        let render_batch = preview
            .batches
            .iter()
            .find(|batch| batch.region_name == "render")
            .expect("render batch");
        assert_eq!(render_batch.flat_color, None, "shaders use the palette");

        // The scenario composite asks for render only.
        let render_only = ass_to_preview(&ass, true);
        assert!(
            render_only
                .batches
                .iter()
                .all(|batch| batch.region_name == "render"),
            "render_only must drop the marker layers"
        );
    }

    /// The @collision_only layer is what the standalone sbsp preview shows as its
    /// collision toggle.
    #[test]
    fn the_ass_collision_layer_becomes_the_collision_region() {
        let ass = AssFile {
            materials: vec![ass_material("@collision_only")],
            objects: vec![ass_mesh(0)],
            instances: vec![AssInstance::default()],
            ..Default::default()
        };
        let preview = ass_to_preview(&ass, false);
        assert_eq!(preview.regions.len(), 1);
        assert_eq!(preview.regions[0].name, COLLISION_REGION);
    }

    /// Merging offsets vertices, indices, and material references — an overlay
    /// whose indices still pointed at its own vertex 0 would draw garbage out of
    /// the render model's buffer.
    #[test]
    fn merging_an_overlay_offsets_every_reference() {
        let mut dst = empty_preview();
        append_jms_triangles(
            &mut dst,
            &JmsFile {
                vertices: vec![
                    jms_vertex(0.0, 0.0, 0.0),
                    jms_vertex(100.0, 0.0, 0.0),
                    jms_vertex(0.0, 100.0, 0.0),
                ],
                triangles: vec![JmsTriangle {
                    material: 0,
                    v: [0, 1, 2],
                    region: 0,
                }],
                ..Default::default()
            },
            "render",
            None,
            false,
        );
        let mut src = empty_preview();
        append_jms_triangles(
            &mut src,
            &JmsFile {
                vertices: vec![
                    jms_vertex(0.0, 0.0, 200.0),
                    jms_vertex(100.0, 0.0, 200.0),
                    jms_vertex(0.0, 100.0, 200.0),
                ],
                triangles: vec![JmsTriangle {
                    material: 0,
                    v: [0, 1, 2],
                    region: 0,
                }],
                ..Default::default()
            },
            COLLISION_REGION,
            Some(COLLISION_COLOR),
            false,
        );

        merge_preview_append(&mut dst, &src);

        assert_eq!(dst.vertices.len(), 6);
        assert_eq!(dst.batches.len(), 2);
        let merged = &dst.batches[1];
        assert_eq!(merged.region_name, COLLISION_REGION);
        assert_eq!(merged.material_index, 1, "materials must offset");
        let first_index = dst.indices[merged.index_start as usize];
        assert_eq!(first_index, 3, "indices must offset past dst's vertices");
        assert_eq!(
            dst.regions.len(),
            2,
            "the overlay's region joins the region list as its toggle"
        );
        assert!(dst.bounds_max[2] >= 2.0 - 1e-4, "bounds must expand");
    }

    #[test]
    fn rebranding_collapses_a_bsp_into_one_toggle() {
        let ass = AssFile {
            materials: vec![ass_material("shader"), ass_material("+portal")],
            objects: vec![ass_mesh(0), ass_mesh(1)],
            instances: vec![
                AssInstance {
                    object_index: 0,
                    ..Default::default()
                },
                AssInstance {
                    object_index: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut preview = ass_to_preview(&ass, false);
        rebrand_preview_region(&mut preview, "010_jungle");
        assert_eq!(preview.regions.len(), 1);
        assert_eq!(preview.regions[0].name, "010_jungle");
        assert!(
            preview
                .batches
                .iter()
                .all(|batch| batch.region_name == "010_jungle")
        );
    }

    /// Instanced geometry lands where its basis puts it — a swapped basis or a
    /// missed scale scatters a level's crates and railings into the wrong rooms.
    #[test]
    fn an_instance_placement_applies_basis_scale_and_position() {
        let placement = InstancePlacement {
            // A quarter-turn: local X maps to world Y, local Y to world -X.
            forward: [0.0, 1.0, 0.0],
            left: [-1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            position: [10.0, 20.0, 30.0],
            scale: 2.0,
        };
        assert_eq!(placement.apply([1.0, 0.0, 0.0]), [10.0, 22.0, 30.0]);
        assert_eq!(placement.apply([0.0, 1.0, 0.0]), [8.0, 20.0, 30.0]);
        // Normals rotate but never scale or translate.
        assert_eq!(placement.rotate([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);

        // A zero or garbage scale falls back to 1 rather than collapsing the
        // instance into a point.
        let degenerate = InstancePlacement {
            scale: 0.0,
            ..InstancePlacement::identity()
        };
        assert_eq!(degenerate.apply([3.0, 0.0, 0.0]), [3.0, 0.0, 0.0]);
    }

    /// Winding flips only when an ODD number of compression axes mirror.
    #[test]
    fn compression_mirroring_flips_winding_only_on_odd_axes() {
        let mut bounds = CompressionBounds::identity();
        assert!(!bounds_axis_flip(&bounds), "identity never flips");

        bounds.pos_compressed = true;
        assert!(!bounds_axis_flip(&bounds), "no mirrored axis");
        (bounds.px_min, bounds.px_max) = (1.0, -1.0);
        assert!(bounds_axis_flip(&bounds), "one mirrored axis flips");
        (bounds.py_min, bounds.py_max) = (1.0, -1.0);
        assert!(!bounds_axis_flip(&bounds), "two mirrors cancel");
    }

    #[test]
    fn a_bsp_toggle_shows_the_reference_leaf() {
        assert_eq!(
            bsp_display_name("levels\\solo\\010_jungle\\010_jungle"),
            "010_jungle"
        );
        assert_eq!(bsp_display_name("010_jungle"), "010_jungle");
    }

    /// The scenario BSP selection invalidates the cached preview; the overlay
    /// toggles must NOT — they are draw-time filters over geometry a worker
    /// merged in once, and re-parsing collision and physics tags on the UI
    /// thread for every tick is exactly the freeze this design removed.
    #[test]
    fn overlay_toggles_never_invalidate_but_the_bsp_selection_does() {
        let mut state = ModelPreviewState::default();
        state.loaded_key = Some("file:a.model".to_owned());
        state.data = Some(Err("placeholder".to_owned()));
        state.loaded_high_detail = state.high_detail;
        assert!(!state.needs_preview_load("file:a.model"));

        state.show_collision = true;
        state.show_physics = true;
        state.show_render = false;
        assert!(
            !state.needs_preview_load("file:a.model"),
            "overlay toggles are frame-level filters, never rebuilds"
        );

        state.scenario_bsp_selection.insert(2);
        assert!(state.needs_preview_load("file:a.model"));
        state.loaded_scenario_selection.insert(2);
        assert!(!state.needs_preview_load("file:a.model"));

        assert!(state.needs_preview_load("file:b.model"), "a different tag");
    }

    /// Point this at an editing kit's `tags` folder to run the derived builders
    /// against real collision, physics, and BSP tags. Absent, this self-skips.
    const KIT_TAGS_ENV: &str = "BABOON_MODEL_KIT";

    #[test]
    fn real_kit_collision_physics_and_bsp_tags_build_previews() {
        let Some(tags_root) = std::env::var_os(KIT_TAGS_ENV).map(std::path::PathBuf::from) else {
            eprintln!("skipping: set {KIT_TAGS_ENV} to an editing kit's tags folder");
            return;
        };
        if !tags_root.is_dir() {
            eprintln!("skipping: {} is not a folder", tags_root.display());
            return;
        }

        let mut built = 0usize;
        let collect = |extension: &str, group: &[u8; 4], take: usize| -> Vec<std::path::PathBuf> {
            let _ = group;
            walkdir::WalkDir::new(&tags_root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|found| {
                    found.file_type().is_file()
                        && found
                            .path()
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
                })
                .map(|found| found.path().to_path_buf())
                .take(take)
                .collect()
        };

        for path in collect("collision_model", b"coll", 10) {
            let Ok(tag) =
                crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"coll"))
            else {
                continue;
            };
            if let Ok(preview) = build_collision_preview(&tag, None) {
                assert!(!preview.vertices.is_empty(), "{}", path.display());
                built += 1;
            }
        }
        for path in collect("physics_model", b"phmo", 10) {
            let Ok(tag) =
                crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"phmo"))
            else {
                continue;
            };
            if let Ok(preview) = build_physics_preview(&tag, None) {
                assert!(!preview.vertices.is_empty(), "{}", path.display());
                built += 1;
            }
        }
        for path in collect("scenario_structure_bsp", b"sbsp", 2) {
            let Ok(tag) =
                crate::core::source::read_tag_at_path(&path, None, None, u32::from_be_bytes(*b"sbsp"))
            else {
                continue;
            };
            if let Ok(preview) = build_sbsp_preview(&tag, false) {
                assert!(!preview.vertices.is_empty(), "{}", path.display());
                assert!(
                    preview.regions.iter().any(|region| region.name == "render"),
                    "{} produced no render layer",
                    path.display()
                );
                built += 1;
            }
        }
        assert!(
            built > 0,
            "nothing under {} built a preview",
            tags_root.display()
        );
        eprintln!("built {built} derived previews from the real kit");
    }

    // The textured BSP contract against a real kit: the native H3 decode must
    // keep UVs, tangent frames, and the materials block's shader paths — the
    // three things the ASS text path lost, and the three things diffuse, normal
    // mapping, and alpha-test each depend on.

    /// Point `BABOON_MODEL_KIT` at an H3-family kit's `tags` folder to run this
    /// against real BSPs; absent, it self-skips like the other fixture tests.
    ///
    /// Several candidates rather than the first found: a working kit accumulates
    /// converted and experimental BSPs whose materials are legitimately empty, and
    /// the contract only claims that a *shipped* BSP keeps its shading inputs — so
    /// one candidate passing the full chain is the assertion.
    #[test]
    fn a_real_bsps_render_layer_keeps_uvs_tangents_and_shader_paths() {
        let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from) else {
            eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
            return;
        };
        let candidates: Vec<std::path::PathBuf> = walkdir::WalkDir::new(&tags_root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|found| {
                found.file_type().is_file()
                    && found.path().extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("scenario_structure_bsp")
                    })
            })
            .map(|found| found.path().to_path_buf())
            .take(8)
            .collect();
        if candidates.is_empty() {
            eprintln!(
                "skipping: no .scenario_structure_bsp under {}",
                tags_root.display()
            );
            return;
        }

        let source = TagSource::LooseFolder {
            root: tags_root.clone(),
            game: Some(GameId::Halo3),
            definitions_root: std::path::PathBuf::new(),
        };
        for path in &candidates {
            let Ok(tag) =
                crate::core::source::read_tag_at_path(path, None, None, u32::from_be_bytes(*b"sbsp"))
            else {
                continue;
            };
            let Ok(preview) = build_sbsp_preview(&tag, false) else {
                continue;
            };
            let with_shader = preview
                .materials
                .iter()
                .filter(|material| !material.shader_path.is_empty())
                .count();
            eprintln!(
                "{}: {} vertices, {} batches, {with_shader}/{} materials with shaders",
                path.display(),
                preview.vertices.len(),
                preview.batches.len(),
                preview.materials.len(),
            );
            if with_shader == 0 {
                continue;
            }

            // UVs must not all be zero, or every texture would smear one texel.
            let nonzero_uv = preview
                .vertices
                .iter()
                .filter(|vertex| vertex.texcoord[0].abs() > 0.001 || vertex.texcoord[1].abs() > 0.001)
                .count();
            assert!(
                nonzero_uv > preview.vertices.len() / 4,
                "{}: UVs did not survive the decode",
                path.display()
            );
            // Tangent frames must survive for normal mapping to perturb.
            let with_tangent = preview
                .vertices
                .iter()
                .filter(|vertex| {
                    let t = vertex.tangent;
                    (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt() > 0.5
                })
                .count();
            assert!(
                with_tangent > preview.vertices.len() / 4,
                "{}: tangent frames did not survive the decode",
                path.display()
            );

            let sample: Vec<RenderModelPreviewMaterial> =
                preview.materials.iter().take(12).cloned().collect();
            let resolved = resolve_model_textures(&source, &sample);
            let base = resolved
                .iter()
                .filter(|material| material.get(TextureSlot::Base).is_some())
                .count();
            let bump = resolved
                .iter()
                .filter(|material| material.get(TextureSlot::Bump).is_some())
                .count();
            eprintln!(
                "  of {} sampled materials: {base} diffuse, {bump} normal maps resolved",
                sample.len()
            );
            assert!(base > 0, "{}: no diffuse maps resolved", path.display());
            assert!(bump > 0, "{}: no normal maps resolved", path.display());
            return;
        }
        panic!(
            "none of {} candidate BSPs under {} carried shader materials",
            candidates.len(),
            tags_root.display()
        );
    }

    /// Collision and physics overlays land while the model's textures are
    /// still resolving. The merge appends flat-coloured materials after the
    /// model's own, so the resolve in flight is still the right answer and
    /// has to be kept, not dropped and run again.
    #[test]
    fn an_overlay_merge_keeps_the_texture_resolve_in_flight() {
        let mut app = Baboon::for_test();
        let stamp = app.model.kit_stamp();
        let key = "file:a.model".to_owned();
        let preview = RenderModelPreview {
            materials: vec![Default::default()],
            ..Default::default()
        };
        let data = super::super::model_preview_data(key.clone(), key.clone(), preview, Vec::new());
        let (geometry_id, textures_id) = (data.geometry_id, data.textures_id);
        let state = app.views[app.model.kits[0].id].caches.model_previews.entry(key.clone()).or_default();
        state.data = Some(Ok(data));
        state.textures_pending = true;

        let overlay = RenderModelPreview {
            materials: vec![Default::default()],
            ..Default::default()
        };
        app.handle_model_overlays_built(stamp, key.clone(), geometry_id, Some(overlay), None);
        let state = &app.views[app.model.kits[0].id].caches.model_previews[&key];
        assert!(
            state.textures_pending,
            "the resolve in flight is still awaited"
        );
        let Some(Ok(data)) = state.data.as_ref() else {
            panic!("preview data");
        };
        assert_ne!(data.geometry_id, geometry_id, "the geometry did change");

        app.handle_model_textures_resolved(
            stamp,
            key.clone(),
            textures_id,
            vec![Default::default()],
        );
        let Some(Ok(data)) = app.views[app.model.kits[0].id].caches.model_previews[&key].data.as_ref() else {
            panic!("preview data");
        };
        assert!(data.textures.is_some(), "and its result is kept");
    }
}
