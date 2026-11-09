//! Halo CE's collision overlay: its object tag is its model wrapper, naming
//! the gbxmodel and the collision geometry directly.

use super::super::load::overlays::Overlay;
use super::super::load::{TagReader, build_collision_preview};
use super::super::{TagFile, object_reference, render_model_skeleton};

/// Halo CE's object tag is its model wrapper. Resolve the gbxmodel bind pose
/// with CE's corrected quaternion convention and use it to place the direct
/// collision-model reference.
pub(in crate::app::model_preview) fn halo1_object_collision_overlay(
    object_tag: &TagFile,
    reader: &TagReader,
) -> Overlay {
    let (Some(model_reference), Some(collision_reference)) = (
        object_reference(object_tag, "model"),
        object_reference(object_tag, "collision model"),
    ) else {
        return Ok(None);
    };
    let render = reader
        .load(&model_reference, "gbxmodel", b"mod2")
        .map_err(|error| format!("Could not load {model_reference}.gbxmodel: {error}"))?;
    let collision = reader
        .load(&collision_reference, "model_collision_geometry", b"coll")
        .map_err(|error| {
            format!("Could not load {collision_reference}.model_collision_geometry: {error}")
        })?;
    let nodes = render_model_skeleton(&render)
        .map_err(|error| format!("{model_reference}.gbxmodel: {error}"))?;
    let built = build_collision_preview(&collision, Some(&nodes));
    built
        .map(Some)
        .map_err(|error| format!("{collision_reference}.model_collision_geometry: {error}"))
}
