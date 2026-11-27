//! A decorator set opened on its own: its types stood on a patch of ground
//! (`model_preview::decorators`).

use super::super::decorators::{Layout, bake_placements, read_decorator_set, scatter};
use super::super::{
    ModelPreviewData, RenderModelPreviewMaterial, TagEntry, TagFile, TagNameIndex,
    model_preview_data,
};
use super::TagReader;
use std::sync::Arc;

/// The group a decorator set is drawn as its own material by: its `texture`
/// is the only one it has.
pub(in crate::app::model_preview) const DECORATOR_SET_GROUP: [u8; 4] = *b"dctr";

/// A `.decorator_set`, laid out by `layout`.
pub(super) fn load_decorator_set(
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
    layout: Layout,
) -> Result<ModelPreviewData, String> {
    let mut notes = Vec::new();
    let mut set = read_decorator_set(tag, reader, &mut notes)?;
    set.placements = scatter(&set, layout);
    let material = RenderModelPreviewMaterial {
        shader_path: crate::app::references::dependency_entry_reference_path(entry, names)
            .unwrap_or_default(),
        shader_group: u32::from_be_bytes(DECORATOR_SET_GROUP),
        ..Default::default()
    };
    let mut preview = bake_placements(&set, material);
    if preview.batches.is_empty() {
        return Err("This decorator set's types have no geometry to draw.".to_owned());
    }
    preview.decorators = Some(Arc::new(set));
    let mut data = model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    );
    data.notes = notes;
    Ok(data)
}
