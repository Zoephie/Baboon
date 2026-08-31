//! Preview geometry derived from non-render tags: collision models, physics
//! models, structure BSPs, and scenario BSP composites. It owns the conversion
//! of blam-tags' JMS/ASS scenes and physics primitives into
//! [`RenderModelPreview`]s; render_model decoding, the GL renderer, and the
//! panel presentation belong elsewhere.

use super::*;
use blam_tags::geometry::{CompressionBounds, read_compression_bounds_at};
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
const COLLISION_COLOR: [u8; 3] = [216, 130, 74];
const PHYSICS_COLOR: [u8; 3] = [104, 150, 216];
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
    preview.materials.push(RenderModelPreviewMaterial::default());
    preview.batches.push(RenderModelPreviewBatch {
        region_name: region_name.to_owned(),
        permutation_name: "default".to_owned(),
        material_index,
        index_start,
        index_count: triples.len() as u32,
        flat_color: color,
    });
    ensure_preview_region(preview, region_name);
}

fn ensure_preview_region(preview: &mut RenderModelPreview, region_name: &str) {
    if !preview.regions.iter().any(|region| region.name == region_name) {
        preview.regions.push(RenderModelPreviewRegion {
            name: region_name.to_owned(),
            permutations: vec!["default".to_owned()],
        });
    }
}

fn empty_preview() -> RenderModelPreview {
    RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    }
}

fn finish_preview(mut preview: RenderModelPreview, what: &str) -> Result<RenderModelPreview, String> {
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
    append_jms_triangles(
        &mut preview,
        &jms,
        COLLISION_REGION,
        Some(COLLISION_COLOR),
        false,
    );
    finish_preview(preview, "collision model")
}

/// A physics tag (`phmo`) tessellated shape by shape. The tag stores
/// parametric primitives — spheres, boxes, pills, convex polyhedra — with no
/// mesh anywhere, so the mesh is generated here.
pub(super) fn build_physics_preview(
    tag: &TagFile,
    skeleton: Option<&[blam_tags::JmsNode]>,
) -> Result<RenderModelPreview, String> {
    let jms = physics_jms_for_game(tag, skeleton).map_err(|error| error.to_string())?;
    let mut triples: Vec<([f32; 3], [f32; 3])> = Vec::new();
    for sphere in &jms.spheres {
        push_sphere(
            &mut triples,
            &sphere.rotation,
            point(&sphere.translation),
            sphere.radius,
        );
    }
    for shape in &jms.boxes {
        push_box(&mut triples, shape);
    }
    for capsule in &jms.capsules {
        push_capsule(&mut triples, capsule);
    }
    for convex in &jms.convex_shapes {
        push_convex(&mut triples, convex);
    }
    let mut preview = empty_preview();
    push_derived_batch(&mut preview, PHYSICS_REGION, Some(PHYSICS_COLOR), &triples);
    finish_preview(preview, "physics model")
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
        [[-1., 1., -1.], [1., 1., -1.], [1., -1., -1.], [-1., -1., -1.]], // -z
        [[1., -1., -1.], [1., 1., -1.], [1., 1., 1.], [1., -1., 1.]], // +x
        [[-1., -1., 1.], [-1., 1., 1.], [-1., 1., -1.], [-1., -1., -1.]], // -x
        [[-1., 1., -1.], [-1., 1., 1.], [1., 1., 1.], [1., 1., -1.]], // +y
        [[1., -1., -1.], [1., -1., 1.], [-1., -1., 1.], [-1., -1., -1.]], // -y
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
        let local = [ring_radius * phi.cos(), ring_radius * phi.sin(), z + z_offset];
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
        preview.materials.push(RenderModelPreviewMaterial::default());
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
    let preview = match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => {
            let mut preview = empty_preview();
            match JmsFile::from_scenario_structure_bsp_ce(tag) {
                Ok(jms) => append_jms_triangles(&mut preview, &jms, "render", None, true),
                Err(error) => return Err(error.to_string()),
            }
            if !render_only
                && let Ok(jms) = JmsFile::from_scenario_structure_bsp_ce_collision(tag)
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
        preview.materials.push(RenderModelPreviewMaterial::default());
    }

    let clusters = root
        .field_path("clusters")
        .and_then(|field| field.as_block())
        .ok_or("This structure BSP has no clusters block.")?;
    let defs = root
        .field_path("resource interface/raw_resources[0]/raw_items/instanced geometries definitions")
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
            let Some(def) = usize::try_from(def_index).ok().and_then(|i| {
                (i < defs.len()).then(|| defs.element(i).unwrap())
            }) else {
                continue;
            };
            let mesh_index = def.read_int_any("mesh index").unwrap_or(-1);
            let Some(mesh) = usize::try_from(mesh_index).ok().and_then(|i| meshes.get(i)) else {
                continue;
            };
            let flip = mesh_compression
                .get(&(mesh_index as usize))
                .map(|&compression| bounds_axis_flip(&read_compression_bounds_at(&root, compression)))
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
    let Some(block) = root.field_path("materials").and_then(|field| field.as_block()) else {
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
            self.position[0] + s * (self.forward[0] * p[0] + self.left[0] * p[1] + self.up[0] * p[2]),
            self.position[1] + s * (self.forward[1] * p[0] + self.left[1] * p[1] + self.up[1] * p[2]),
            self.position[2] + s * (self.forward[2] * p[0] + self.left[2] * p[1] + self.up[2] * p[2]),
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
fn append_collision_bsp_triangles(
    bsp: &TagStruct<'_>,
    triples: &mut Vec<([f32; 3], [f32; 3])>,
) {
    let (Some(surfaces), Some(edges), Some(vertices)) = (
        bsp.field_path("surfaces").and_then(|field| field.as_block()),
        bsp.field_path("edges").and_then(|field| field.as_block()),
        bsp.field_path("vertices").and_then(|field| field.as_block()),
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
        dst.indices
            .extend(src.indices[start..end].iter().map(|index| index + vertex_base));
        dst.batches.push(RenderModelPreviewBatch {
            region_name: batch.region_name.clone(),
            permutation_name: batch.permutation_name.clone(),
            material_index: (batch.material_index as usize + material_base)
                .min(u16::MAX as usize) as u16,
            index_start,
            index_count: (end - start) as u32,
            flat_color: batch.flat_color,
        });
    }
    for region in &src.regions {
        if !dst.regions.iter().any(|existing| existing.name == region.name) {
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
#[path = "../tests/derived_preview.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/bsp_texture_probe.rs"]
mod bsp_texture_probe;
