//! The collision and physics overlays a model or object names.

use super::super::{RenderModelPreview, TagFile, model_skeleton, tag_ref_path};
use super::{TagReader, build_collision_preview, build_physics_preview};

/// The `.model`'s collision layer, posed on its own skeleton, ready to merge
/// over the render preview. `None` when the reference is absent or unreadable
/// — a missing overlay degrades to nothing rather than failing the preview.
pub(in crate::app::model_preview) fn hlmt_collision_overlay(
    model_tag: &TagFile,
    reader: &TagReader,
) -> Overlay {
    let Some(reference) = tag_ref_path(&model_tag.root(), "collision model") else {
        return Ok(None);
    };
    let collision = reader
        .load(&reference, "collision_model", b"coll")
        .map_err(|error| format!("Could not load {reference}.collision_model: {error}"))?;
    let skeleton = model_skeleton(&reader.source, model_tag);
    let built = build_collision_preview(&collision, skeleton.as_ref().map(|s| s.nodes()));
    built
        .map(Some)
        .map_err(|error| format!("{reference}.collision_model: {error}"))
}

/// An overlay built off a model: none when the model names nothing to draw,
/// and why when it names something that would not build.
pub(in crate::app::model_preview) type Overlay = Result<Option<RenderModelPreview>, String>;

/// The `.model`'s physics layer, likewise. Reads both spellings the H2-era
/// definitions used (`physics_model` and `physics model`); the legacy H2
/// `physics` (`phys`) reference is deliberately not resolved — it is not a
/// `phmo` and has no shapes to draw.
pub(in crate::app::model_preview) fn hlmt_physics_overlay(
    model_tag: &TagFile,
    reader: &TagReader,
) -> Overlay {
    let root = model_tag.root();
    let Some(reference) =
        tag_ref_path(&root, "physics_model").or_else(|| tag_ref_path(&root, "physics model"))
    else {
        return Ok(None);
    };
    let physics = reader
        .load(&reference, "physics_model", b"phmo")
        .map_err(|error| format!("Could not load {reference}.physics_model: {error}"))?;
    let skeleton = model_skeleton(&reader.source, model_tag);
    let built = build_physics_preview(&physics, skeleton.as_ref().map(|s| s.nodes()));
    built
        .map(Some)
        .map_err(|error| format!("{reference}.physics_model: {error}"))
}
