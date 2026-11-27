//! A decorator set opened on its own: its types stood on a patch of ground
//! (`model_preview::decorators`).

use super::super::decorators::{
    DecoratorSet, DecoratorType, Layout, bake_halo_2, bake_halo_ce, bake_placements,
    halo_2_type_name, read_decorator_set, read_halo_2_set, read_halo_ce_collection, scatter,
    scatter_halo_2, scatter_halo_ce,
};
use super::super::{
    ModelPreviewData, RenderModelPreviewMaterial, TagEntry, TagFile, TagNameIndex,
    model_preview_data,
};
use super::TagReader;
use std::sync::Arc;

/// The group a decorator set is drawn as its own material by: its `texture`
/// is the only one it has.
pub(in crate::app::model_preview) const DECORATOR_SET_GROUP: [u8; 4] = *b"dctr";

/// Halo CE's detail object collection, drawn as its own material by: its
/// sprite plate is the only one it has.
pub(in crate::app::model_preview) const DETAIL_OBJECT_COLLECTION_GROUP: [u8; 4] = *b"dobc";

/// Halo 2's decorator set, whose permutations draw through `shad`s of its own.
pub(in crate::app::model_preview) const HALO_2_DECORATOR_SET_GROUP: [u8; 4] = *b"DECR";

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
    set.layout = layout;
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

/// A Halo 2 `.decorator_set`, laid out by `layout`.
pub(super) fn load_halo_2_decorator_set(
    tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    layout: Layout,
) -> Result<ModelPreviewData, String> {
    let mut notes = Vec::new();
    let mut halo2 = read_halo_2_set(tag, reader, &mut notes)?;
    halo2.records = scatter_halo_2(&halo2, layout);
    let types = halo2
        .types()
        .map(|(class, permutation)| DecoratorType {
            mesh_name: halo_2_type_name(&halo2, class, permutation),
            placement_weight: Some(
                halo2.classes[class].permutations[permutation].distribution_weight,
            ),
            ..Default::default()
        })
        .collect();
    let mut preview = bake_halo_2(&halo2);
    if preview.batches.is_empty() {
        return Err("This decorator set's permutations have nothing to draw.".to_owned());
    }
    preview.decorators = Some(Arc::new(DecoratorSet {
        types,
        layout,
        halo2: Some(Arc::new(halo2)),
        ..Default::default()
    }));
    let mut data = model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    );
    data.notes = notes;
    Ok(data)
}

/// A Halo CE `.detail_object_collection`, laid out by `layout`.
pub(super) fn load_detail_object_collection(
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    reader: Option<&TagReader>,
    layout: Layout,
) -> Result<ModelPreviewData, String> {
    let mut collection = read_halo_ce_collection(tag, reader)?;
    collection.instances = scatter_halo_ce(&collection, layout);
    let material = RenderModelPreviewMaterial {
        shader_path: crate::app::references::dependency_entry_reference_path(entry, names)
            .unwrap_or_default(),
        shader_group: u32::from_be_bytes(DETAIL_OBJECT_COLLECTION_GROUP),
        ..Default::default()
    };
    let types = collection
        .types
        .iter()
        .map(|kind| DecoratorType {
            mesh_name: kind.name.clone(),
            ..Default::default()
        })
        .collect();
    let mut preview = bake_halo_ce(&collection, material);
    if preview.batches.is_empty() {
        return Err("This detail object collection's types have no sprites to draw.".to_owned());
    }
    preview.decorators = Some(Arc::new(DecoratorSet {
        types,
        layout,
        halo_ce: Some(Arc::new(collection)),
        ..Default::default()
    }));
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}
