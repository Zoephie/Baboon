//! Preview geometry derived from non-render tags: collision models, physics
//! models, structure BSPs, and scenario BSP composites. It owns the conversion
//! of blam-tags' JMS/ASS scenes and physics primitives into
//! [`RenderModelPreview`]s; render_model decoding, the GL renderer, and the
//! panel presentation belong elsewhere.

use super::super::common::geometry::{Overflow, VertexBase};
use super::super::{
    ModelPreviewData, ModelPreviewLayer, RenderModelPreview, RenderModelPreviewBatch,
    RenderModelPreviewMaterial, RenderModelPreviewRegion, RenderModelPreviewVertex, TagEntry,
    TagFile, model_preview_data, owning_model_skeleton,
};
use super::{TagReader, expand_preview_bounds_local};
use blam_tags::JmsFile;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};

mod collision;
pub(in crate::app::model_preview) use collision::*;
mod physics;
pub(in crate::app::model_preview) use physics::*;
mod ass;
mod sbsp;
pub(in crate::app::model_preview) use sbsp::*;

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
    let Some(vertex_base) = VertexBase::of(
        preview.vertices.len(),
        triples.len(),
        Overflow::RefusesAppend,
    ) else {
        return;
    };
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
        .extend((0..triples.len() as u32).map(|offset| vertex_base.offset(offset)));
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
        sort_position: None,
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
    match preview
        .regions
        .iter_mut()
        .find(|region| region.name == region_name)
    {
        Some(region) => {
            if !region
                .permutations
                .iter()
                .any(|name| name == permutation_name)
            {
                region.permutations.push(permutation_name.to_owned());
            }
        }
        None => preview.regions.push(RenderModelPreviewRegion {
            name: region_name.to_owned(),
            permutations: vec![permutation_name.to_owned()],
        }),
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

/// Collision and physics tags carry no render geometry of their own —
/// their preview is derived: collision BSPs walked into triangles, physics
/// primitives tessellated. Posed on the owning `.model`'s skeleton when
/// one is beside them, unposed otherwise.
pub(super) fn load_collision_or_physics(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
) -> Result<ModelPreviewData, String> {
    let group = model_tag.header.group_tag.to_be_bytes();
    let skeleton = reader.and_then(|reader| owning_model_skeleton(&reader.source, entry));
    let nodes = skeleton.as_ref().map(|skeleton| skeleton.nodes());
    let preview = if &group == b"coll" {
        build_collision_preview(model_tag, nodes)?
    } else {
        build_physics_preview(model_tag, nodes)?
    };
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests;
