//! Reach's and Halo 2 Anniversary's imposters: the low, untextured model the
//! game swaps in far away, its look baked into each vertex. Drawn on its own
//! tab, and where its `.model` is under the Imposter toggle.

use super::super::common::geometry::{Overflow, VertexBase, expand_preview_bounds_local};
use super::super::{
    ModelPreviewData, ModelPreviewLayer, RenderModelPreview, RenderModelPreviewBatch,
    RenderModelPreviewRegion, RenderModelPreviewVertex, TagEntry, TagFile, model_preview_data,
};
use super::render_model::{point3_to_array, vector3_to_array};
use super::stand_in::StandIn;
use super::{TagReader, merge_preview_append};
use blam_tags::TagFieldData;
use blam_tags::render_model::{RenderMesh, extract_render_geometry_meshes_at};
use std::collections::BTreeMap;

/// Where an imposter keeps its render geometry.
const GEOMETRY: &str = "geometry";

/// Levels each colour channel is rounded to, so triangles of near colours
/// share a batch.
const COLOR_LEVELS: f32 = 24.0;

/// An `.imposter_model` opened on its own.
pub(super) fn load_imposter_tag(
    tag: &TagFile,
    entry: &TagEntry,
) -> Result<ModelPreviewData, String> {
    let preview = build_imposter_preview(tag)?;
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}

/// Append the imposter `model_tag` names to `preview` on the Imposter layer.
/// `None` when the `.model` names none.
pub(super) fn append_imposter(
    model_tag: &TagFile,
    reader: Option<&TagReader>,
    preview: &mut RenderModelPreview,
) -> Option<StandIn> {
    let (_, path) = model_tag.root().read_tag_ref_with_group("imposter model")?;
    let path = path.trim().to_owned();
    if path.is_empty() {
        return None;
    }
    let mut found = StandIn {
        path,
        missing: None,
        vertices: 0,
        triangles: 0,
    };
    let read = reader
        .ok_or_else(|| "The imposter needs a loaded editing kit.".to_owned())
        .and_then(|reader| {
            reader
                .load(&found.path, "imposter_model", b"impo")
                .map_err(|error| format!("Could not load {}.imposter_model: {error}", found.path))
        })
        .and_then(|tag| build_imposter_preview(&tag));
    match read {
        Ok(mut imposter) => {
            found.vertices = imposter.vertices.len();
            found.triangles = imposter.indices.len() / 3;
            for batch in &mut imposter.batches {
                batch.layer = ModelPreviewLayer::Imposter;
            }
            merge_preview_append(preview, &imposter);
        }
        Err(error) => found.missing = Some(error),
    }
    Some(found)
}

/// An imposter's geometry, each triangle drawn in the colour baked into its
/// vertices. Its regions are its own (`region_00`…), not its model's, and it
/// is not posed: its vertices draw where the tag puts them.
pub(in crate::app::model_preview) fn build_imposter_preview(
    tag: &TagFile,
) -> Result<RenderModelPreview, String> {
    let root = tag.root();
    let meshes =
        extract_render_geometry_meshes_at(&root, GEOMETRY).map_err(|error| error.to_string())?;
    let colors = baked_colors(tag, &meshes);
    let mut preview = RenderModelPreview {
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };
    let regions = root.field("regions").and_then(|field| field.as_block());
    for region_index in 0..regions.as_ref().map_or(0, |regions| regions.len()) {
        let Some(region) = regions.as_ref().and_then(|regions| regions.element(region_index))
        else {
            continue;
        };
        let region_name = region.read_string_id("name").unwrap_or_default();
        let mut entry = RenderModelPreviewRegion {
            name: region_name.clone(),
            permutations: Vec::new(),
        };
        let permutations = region.field("permutations").and_then(|field| field.as_block());
        for permutation_index in 0..permutations.as_ref().map_or(0, |block| block.len()) {
            let Some(permutation) = permutations
                .as_ref()
                .and_then(|block| block.element(permutation_index))
            else {
                continue;
            };
            let permutation_name = permutation.read_string_id("name").unwrap_or_default();
            entry.permutations.push(permutation_name.clone());
            let first = permutation.read_int_any("mesh index").unwrap_or(-1);
            let count = permutation.read_int_any("mesh count").unwrap_or(1).max(1);
            let Ok(first) = usize::try_from(first) else {
                continue;
            };
            for mesh_index in first..first.saturating_add(count as usize) {
                if let Some(mesh) = meshes.get(mesh_index) {
                    append_mesh(
                        &mut preview,
                        mesh,
                        colors.get(mesh_index).map_or(&[][..], Vec::as_slice),
                        &region_name,
                        &permutation_name,
                    );
                }
            }
        }
        preview.regions.push(entry);
    }
    if preview.batches.is_empty() {
        return Err("This imposter has no previewable geometry.".to_owned());
    }
    Ok(preview)
}

/// Each mesh's vertex colours, from its `raw imposter data` diffuse, in
/// display colour. The game lights them in HDR, so they are scaled for the
/// brightest to sit near the middle, as lit albedo would. Empty for a mesh
/// whose colours do not line up with its vertices.
fn baked_colors(tag: &TagFile, meshes: &[RenderMesh]) -> Vec<Vec<[f32; 3]>> {
    let root = tag.root();
    let raw = root
        .field_path(&format!("{GEOMETRY}/per mesh temporary"))
        .and_then(|field| field.as_block());
    let mut colors: Vec<Vec<[f32; 3]>> = meshes
        .iter()
        .enumerate()
        .map(|(mesh_index, mesh)| {
            let Some(brdf) = raw
                .as_ref()
                .and_then(|raw| raw.element(mesh_index))
                .and_then(|mesh| mesh.field("raw imposter data")?.as_block())
            else {
                return Vec::new();
            };
            if brdf.len() != mesh.vertices.len() {
                return Vec::new();
            }
            (0..brdf.len())
                .map(|vertex| {
                    match brdf
                        .element(vertex)
                        .and_then(|vertex| vertex.field("diffuse")?.value())
                    {
                        Some(TagFieldData::RealRgbColor(color)) => {
                            [color.red, color.green, color.blue].map(|channel| channel.max(0.0))
                        }
                        _ => [0.0; 3],
                    }
                })
                .collect()
        })
        .collect();
    // The brightest vertex but for a few: one hot vertex should not darken
    // the rest.
    let mut brightness: Vec<f32> = colors
        .iter()
        .flatten()
        .map(|color| color.iter().copied().fold(0.0, f32::max))
        .collect();
    brightness.sort_by(f32::total_cmp);
    let bright = brightness
        .get(brightness.len().saturating_sub(1) * 95 / 100)
        .copied()
        .filter(|bright| *bright > 0.0)
        .unwrap_or(1.0);
    for color in colors.iter_mut().flatten() {
        *color = color.map(|channel| (channel / bright * 0.55).clamp(0.0, 1.0));
    }
    colors
}

/// Add `mesh` to `preview`, its triangles batched by baked colour.
fn append_mesh(
    preview: &mut RenderModelPreview,
    mesh: &RenderMesh,
    colors: &[[f32; 3]],
    region_name: &str,
    permutation_name: &str,
) {
    let Some(base) = VertexBase::of(
        preview.vertices.len(),
        mesh.vertices.len(),
        Overflow::DropsTriangle,
    ) else {
        return;
    };
    for vertex in &mesh.vertices {
        let position = point3_to_array(vertex.position);
        expand_preview_bounds_local(&mut preview.bounds_min, &mut preview.bounds_max, position);
        preview.vertices.push(RenderModelPreviewVertex {
            position,
            normal: vector3_to_array(vertex.normal),
            texcoord: [vertex.texcoord.x, vertex.texcoord.y],
            tangent: vector3_to_array(vertex.tangent),
            binormal: vector3_to_array(vertex.binormal),
            // Not posed: no weight draws it at its bind position.
            node_indices: [0.0; 4],
            node_weights: [0.0; 4],
        });
    }
    let mut by_color: BTreeMap<[u8; 3], Vec<u32>> = BTreeMap::new();
    for part in &mesh.parts {
        let start = part.index_start as usize;
        let end = start
            .saturating_add(part.index_count as usize)
            .min(mesh.indices.len());
        for triangle in mesh.indices.get(start..end).unwrap_or_default().chunks_exact(3) {
            let corners = [triangle[0], triangle[1], triangle[2]];
            let offsets = corners.map(|index| {
                ((index as usize) < mesh.vertices.len())
                    .then(|| base.checked_offset(index))
                    .flatten()
            });
            let [Some(a), Some(b), Some(c)] = offsets else {
                continue;
            };
            by_color
                .entry(triangle_color(colors, corners))
                .or_default()
                .extend([a, b, c]);
        }
    }
    for (color, indices) in by_color {
        let Ok(index_start) = u32::try_from(preview.indices.len()) else {
            return;
        };
        preview.indices.extend_from_slice(&indices);
        preview.batches.push(RenderModelPreviewBatch {
            region_name: region_name.to_owned(),
            permutation_name: permutation_name.to_owned(),
            material_index: 0,
            index_start,
            index_count: indices.len() as u32,
            flat_color: Some(color),
            layer: ModelPreviewLayer::Render,
            sort_position: None,
        });
    }
}

/// A triangle's colour, its corners' averaged and rounded; mid grey when the
/// mesh has none.
fn triangle_color(colors: &[[f32; 3]], corners: [u32; 3]) -> [u8; 3] {
    let mut sum = [0.0_f32; 3];
    for corner in corners {
        let Some(color) = colors.get(corner as usize) else {
            return [128; 3];
        };
        for channel in 0..3 {
            sum[channel] += color[channel] / 3.0;
        }
    }
    sum.map(|channel| {
        let rounded = (channel * COLOR_LEVELS).round() / COLOR_LEVELS;
        (rounded * 255.0).round().clamp(0.0, 255.0) as u8
    })
}

#[cfg(test)]
mod tests {
    use super::triangle_color;
    use crate::app::model_preview::ModelPreviewLayer;
    use crate::app::model_preview::harness::kit::{kit_preview, kit_root};
    use crate::core::game::GameId;

    #[test]
    fn a_triangle_takes_its_corners_rounded_colour() {
        let colors = [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let [red, green, blue] = triangle_color(&colors, [0, 1, 2]);
        assert!(red > 160 && red < 180, "{red}");
        assert_eq!(green, 0);
        assert!(blue > 75 && blue < 95, "{blue}");
        assert_eq!(triangle_color(&[], [0, 1, 2]), [128; 3]);
    }

    /// Reach's banshee: its imposter opens on its own tab, coloured from its
    /// baked vertices, and its `.model` carries it on the Imposter layer.
    #[test]
    fn a_shipped_imposter_draws_alone_and_with_its_model() {
        let Some(root) = kit_root(GameId::HaloReach) else {
            return;
        };
        let path = r"objects\vehicles\covenant\banshee\banshee";
        let alone = kit_preview(&root, GameId::HaloReach, path, "imposter_model", b"impo");
        let preview = &alone.preview;
        assert!(preview.indices.len() / 3 > 100, "{} triangles", preview.indices.len() / 3);
        assert!(preview.regions.iter().any(|region| region.name == "region_00"));
        let colors: std::collections::HashSet<_> =
            preview.batches.iter().filter_map(|batch| batch.flat_color).collect();
        assert!(colors.len() > 4, "{colors:?}");

        let model = kit_preview(&root, GameId::HaloReach, path, "model", b"hlmt");
        let imposter = model.preview.imposter.as_ref().expect("an imposter");
        assert_eq!(imposter.missing, None);
        assert_eq!(imposter.triangles, preview.indices.len() / 3);
        let on_layer = |layer| model.preview.batches.iter().filter(move |batch| batch.layer == layer);
        assert!(on_layer(ModelPreviewLayer::Imposter).count() > 0);
        assert!(on_layer(ModelPreviewLayer::Render).count() > 0);
    }
}
