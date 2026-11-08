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
