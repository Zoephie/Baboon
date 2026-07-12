//! Preview tag loading and render-model resolution.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;

pub(super) fn ensure_model_preview_loaded(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    state: &mut ModelPreviewState,
) {
    if state.loaded_key.as_deref() == Some(entry.key.as_str()) && state.data.is_some() {
        return;
    }
    state.loaded_key = Some(entry.key.clone());
    state.data = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load_model_preview(model_tag, entry, names, source)
        }))
        .map_err(|_| "Render model preview crashed while parsing this tag.".to_owned())
        .and_then(|result| result)
        .map(|data| {
            state.render_model_path = Some(data.render_model_path.clone());
            // Auto-select the canonical variant (named `default`, else the first)
            // so the preview opens showing a complete configured model.
            let default_variant = default_variant_index(&data.variants);
            reset_model_preview_selection(state, &data, default_variant);
            data
        }),
    );
}

pub(super) fn load_model_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
) -> Result<ModelPreviewData, String> {
    // Halo CE `gbxmodel` (mod2) and a bare `render_model` (mode) ARE the
    // render geometry — there is no `.model` (hlmt) wrapper carrying a
    // "render model" reference, so preview the tag itself.
    let group = model_tag.header.group_tag.to_be_bytes();
    if matches!(&group, b"mode" | b"mod2") {
        let preview = build_render_preview(model_tag)?;
        if preview.batches.is_empty() {
            return Err("This render tag has no previewable draw batches.".to_owned());
        }
        let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
        let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
        return Ok(ModelPreviewData {
            source_key: entry.key.clone(),
            render_model_path: entry.display_path.clone(),
            preview,
            draw_triangles,
            variants: Vec::new(),
        });
    }

    let Some((group_tag, rel_path)) = model_tag.root().read_tag_ref_with_group("render model")
    else {
        return Err("This model tag has no render model reference.".to_owned());
    };
    if rel_path.trim().is_empty() {
        return Err("This model tag has an empty render model reference.".to_owned());
    }
    let Some(TagSource::LooseFolder { root, .. }) = source else {
        return Err("Render model preview requires a loaded loose-folder editing kit.".to_owned());
    };
    let extension = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
        .unwrap_or("render_model");
    let mut normalized = rel_path.replace('/', "\\");
    if let Some(stripped) = normalized.strip_suffix(&format!(".{extension}")) {
        normalized = stripped.to_owned();
    }
    let path = resolve_tag_path(root, &normalized, extension);
    if !path.exists() {
        return Err(format!(
            "Referenced render_model was not found: {}",
            path.display()
        ));
    }
    let render_entry = TagEntry {
        key: format!("file:{}", path.display()),
        display_path: format!("{}.{}", normalized.replace('\\', "/"), extension),
        group_tag,
        group_name: names.name_for(group_tag).map(str::to_owned),
        location: TagEntryLocation::LooseFile(path),
    };
    let render_tag =
        read_entry(source.unwrap(), &render_entry).map_err(|error| error.to_string())?;
    let preview = build_render_preview(&render_tag)?;
    if preview.batches.is_empty() {
        return Err("Referenced render_model has no previewable draw batches.".to_owned());
    }
    let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
    let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
    Ok(ModelPreviewData {
        source_key: render_entry.key,
        render_model_path: normalized,
        preview,
        draw_triangles,
        variants: read_model_variants(model_tag),
    })
}

/// Build preview geometry from a render-geometry tag — a `render_model`
/// (`mode`), a Halo CE `gbxmodel` (`mod2`), or a Halo 2 `render_model`.
///
/// One tag→geometry path for every engine: blam-tags' `RenderModel::from_tag`
/// / `derive_render_meshes` game-dispatch (H3 reads `render geometry`, H2 the
/// `sections`, Halo CE the gbxmodel `geometries`), so batches carry the render
/// model's own region/permutation names and stay in sync with the variant
/// selection. JMS is export-only — never used for rendering.
pub(super) fn build_render_preview(render_tag: &TagFile) -> Result<RenderModelPreview, String> {
    let render_model = RenderModel::from_tag(render_tag).map_err(|error| error.to_string())?;
    let render_meshes =
        RenderModel::derive_render_meshes(render_tag).map_err(|error| error.to_string())?;
    Ok(render_model_to_preview(&render_model, &render_meshes))
}

pub(super) fn expand_preview_bounds_local(min: &mut [f32; 3], max: &mut [f32; 3], point: [f32; 3]) {
    for axis in 0..3 {
        min[axis] = min[axis].min(point[axis]);
        max[axis] = max[axis].max(point[axis]);
    }
}

pub(super) struct RawVariantRegion {
    pub(super) perm: Option<String>,
    pub(super) parent: i128,
}
pub(super) struct RawVariant {
    pub(super) name: String,
    pub(super) regions: Vec<(String, RawVariantRegion)>,
}
