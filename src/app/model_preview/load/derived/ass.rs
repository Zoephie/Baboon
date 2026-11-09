//! ASS scenes as preview geometry: one region per layer, instances placed.

use super::*;
use crate::app::model_preview::common::geometry::expand_preview_bounds_local;

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
pub(super) fn ass_to_preview(ass: &AssFile, render_only: bool) -> RenderModelPreview {
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

        let Some(vertex_base) = VertexBase::of(
            preview.vertices.len(),
            vertices.len(),
            Overflow::RefusesAppend,
        ) else {
            break;
        };
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
            slot.extend(triangle.v.iter().map(|&v| vertex_base.offset(v)));
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
                sort_position: None,
            });
        }
        ensure_preview_region(&mut preview, region);
    }
    preview
}
