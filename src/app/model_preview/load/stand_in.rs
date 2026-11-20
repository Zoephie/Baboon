//! A model's stand-in: the simpler model a game swaps in at a distance, as
//! Halo 3's and ODST's `lod_render_model`. It is drawn in the render model's
//! place, on its own layer, while the Other menu's LOD Model is ticked.

use super::super::{ModelPreviewLayer, RenderModelPreview, TagFile};
use super::render_model::build_render_preview;
use super::{TagReader, merge_preview_append};

/// A stand-in a `.model` names: its path, and why it could not be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StandIn {
    pub path: String,
    /// What went wrong reading it (most often a file that is not there);
    /// `None` once its geometry is in the preview.
    pub missing: Option<String>,
    /// Its vertices and triangles, which the model's own counts leave out.
    pub vertices: usize,
    pub triangles: usize,
}

/// Append the LOD render model `model_tag` names to `preview` on the LOD
/// Model layer, its nodes matched to the model's by name so it takes the
/// model's pose. `None` when the game's `.model` has no such field; one
/// that names none is `missing`, so its choice shows, greyed out.
pub(super) fn append_lod_model(
    model_tag: &TagFile,
    reader: Option<&TagReader>,
    preview: &mut RenderModelPreview,
) -> Option<StandIn> {
    model_tag.root().field("lod_render_model")?;
    let path = model_tag
        .root()
        .read_tag_ref_with_group("lod_render_model")
        .map(|(_, path)| path.trim().to_owned())
        .unwrap_or_default();
    if path.is_empty() {
        return Some(StandIn {
            path,
            missing: Some("This model names no LOD model.".to_owned()),
            vertices: 0,
            triangles: 0,
        });
    }
    let mut found = StandIn {
        path,
        missing: None,
        vertices: 0,
        triangles: 0,
    };
    match read_stand_in(reader, &found.path) {
        Ok(stand_in) => {
            found.vertices = stand_in.vertices.len();
            found.triangles = stand_in.indices.len() / 3;
            append_stand_in(preview, stand_in, ModelPreviewLayer::LodModel);
        }
        Err(error) => found.missing = Some(error),
    }
    Some(found)
}

fn read_stand_in(reader: Option<&TagReader>, path: &str) -> Result<RenderModelPreview, String> {
    let reader = reader.ok_or("The LOD model needs a loaded editing kit.")?;
    let tag = reader
        .load(path, "render_model", b"mode")
        .map_err(|error| format!("Could not load {path}.render_model: {error}"))?;
    let stand_in = build_render_preview(&tag)?;
    if stand_in.batches.is_empty() {
        return Err(format!("{path}.render_model has no previewable draw batches."));
    }
    Ok(stand_in)
}

/// Add `stand_in`'s geometry to `preview` on `layer`. Its vertices bind to
/// the model's nodes of the same names, so it moves with the model's
/// animation; a node the model lacks binds to the root.
pub(super) fn append_stand_in(
    preview: &mut RenderModelPreview,
    mut stand_in: RenderModelPreview,
    layer: ModelPreviewLayer,
) {
    let node_of: Vec<f32> = stand_in
        .nodes
        .iter()
        .map(|node| {
            preview
                .nodes
                .iter()
                .position(|own| own.name.eq_ignore_ascii_case(&node.name))
                .unwrap_or(0) as f32
        })
        .collect();
    for vertex in &mut stand_in.vertices {
        for index in &mut vertex.node_indices {
            *index = node_of.get(*index as usize).copied().unwrap_or(0.0);
        }
    }
    for batch in &mut stand_in.batches {
        batch.layer = layer;
    }
    // Its import errors show with it, not with the model.
    for error in &mut stand_in.errors {
        error.layer = layer;
    }
    merge_preview_append(preview, &stand_in);
}

#[cfg(test)]
mod tests {
    use crate::app::model_preview::ModelPreviewLayer;
    use crate::app::model_preview::harness::kit::{kit_preview, kit_root};
    use crate::core::game::GameId;

    /// Halo 3's Covenant cruiser names an LOD render model: it loads on its
    /// own layer, bound to the cruiser's nodes, and counts apart.
    #[test]
    fn a_shipped_model_carries_its_lod_model() {
        let Some(root) = kit_root(GameId::Halo3) else {
            return;
        };
        let path = r"objects\vehicles\cov_cruiser\cov_cruiser";
        let data = kit_preview(&root, GameId::Halo3, path, "model", b"hlmt");
        let preview = &data.preview;
        let lod = preview.lod_model.as_ref().expect("an LOD model");
        assert!(lod.path.ends_with("cov_cruiser_lod"), "{lod:?}");
        assert_eq!(lod.missing, None);
        assert!(lod.triangles > 0 && lod.vertices > 0, "{lod:?}");
        let on_layer = |layer| preview.batches.iter().filter(move |batch| batch.layer == layer);
        assert!(on_layer(ModelPreviewLayer::LodModel).count() > 0);
        assert!(on_layer(ModelPreviewLayer::Render).count() > 0);
        let nodes = preview.nodes.len() as f32;
        assert!(
            preview.vertices[preview.vertices.len() - lod.vertices..]
                .iter()
                .all(|vertex| vertex.node_indices.iter().all(|&node| node < nodes.max(1.0))),
            "an LOD vertex binds past the cruiser's nodes"
        );
    }
}
