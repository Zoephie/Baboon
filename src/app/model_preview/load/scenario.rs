//! Scenarios and structure BSPs: the BSP list, each BSP rebranded as one toggle, and their labels.

use super::*;

/// The scenario's `structure bsps` block as backslash reference paths, in tag
/// order. Elements whose reference is empty stay in the list as `None` so the
/// panel's indices line up with the tag's.
pub(in crate::app::model_preview) fn scenario_bsp_paths(scenario: &TagFile) -> Vec<Option<String>> {
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

/// Rename every region and batch of a preview to one region — the shape the
/// scenario composite wants, where each BSP is a single toggle.
pub(in crate::app::model_preview) fn rebrand_preview_region(preview: &mut RenderModelPreview, region_name: &str) {
    for batch in &mut preview.batches {
        batch.region_name = region_name.to_owned();
        batch.permutation_name = "default".to_owned();
    }
    preview.regions = vec![RenderModelPreviewRegion {
        name: region_name.to_owned(),
        permutations: vec!["default".to_owned()],
    }];
}

/// The leaf name a BSP toggle shows: the last path segment of its reference.
pub(in crate::app::model_preview) fn bsp_display_name(reference: &str) -> String {
    Some(tag_path::name(reference))
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or(reference)
        .to_owned()
}

/// A structure BSP previews its own geometry, layered into `render` /
/// `collision` / `portals` / `weather` regions so each is a toggle.
pub(super) fn load_structure_bsp(model_tag: &TagFile, entry: &TagEntry) -> Result<ModelPreviewData, String> {
    let preview = build_sbsp_preview(model_tag, false)?;
    Ok(model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    ))
}

/// A scenario previews a composite of the structure BSPs the user has
/// checked — each selected BSP loads render-only and lands as one region,
/// so the region list doubles as the per-BSP toggle. Nothing loads until
/// something is checked: a campaign scenario's full BSP set is far too
/// much to parse unasked.
pub(super) fn load_scenario(
    model_tag: &TagFile,
    entry: &TagEntry,
    reader: Option<&TagReader>,
    settings: &PreviewLoadSettings,
) -> Result<ModelPreviewData, String> {
    let bsps = scenario_bsp_paths(model_tag);
    if bsps.is_empty() {
        return Err("This scenario lists no structure BSPs.".to_owned());
    }
    let mut preview = RenderModelPreview::default();
    let mut notes = Vec::new();
    if !settings.scenario_selection.is_empty() {
        let Some(reader) = reader else {
            return Err("Scenario preview requires a loaded source.".to_owned());
        };
        preview.bounds_min = [f32::INFINITY; 3];
        preview.bounds_max = [f32::NEG_INFINITY; 3];
        for &index in &settings.scenario_selection {
            let Some(Some(reference)) = bsps.get(index) else {
                continue;
            };
            // One BSP that will not read leaves the rest showing.
            let read = || {
                let tag = reader.load(reference, "scenario_structure_bsp", b"sbsp")
                    .map_err(|error| format!("Could not load {reference}: {error}"))?;
                build_sbsp_preview(&tag, true).map_err(|error| format!("{reference}: {error}"))
            };
            match feature(&mut notes, reference, || Some(read())) {
                Some(Ok(mut bsp_preview)) => {
                    rebrand_preview_region(&mut bsp_preview, &bsp_display_name(reference));
                    merge_preview_append(&mut preview, &bsp_preview);
                }
                Some(Err(error)) => notes.push(error),
                None => {}
            }
        }
        if !preview.bounds_min.iter().all(|bound| bound.is_finite()) {
            preview.bounds_min = [0.0; 3];
            preview.bounds_max = [0.0; 3];
        }
    }
    let mut data = model_preview_data(
        entry.key.clone(),
        entry.display_path.clone(),
        preview,
        Vec::new(),
    );
    data.scenario_bsps = bsps;
    data.notes = notes;
    Ok(data)
}
