//! The variant, region and permutation selection: which variant and
//! permutations a preview starts in, and which regions it draws.

use super::super::ModelVariantRegionChoice;
use super::{
    ModelPreviewData, ModelPreviewLayer, ModelPreviewState, ModelRegionSelection, PHYSICS_REGION,
    RenderModelPreviewRegion,
};
use std::collections::HashMap;

/// The variant to select on load: the one an object names, else one literally
/// named `default`, else the first (the canonical entry — `chief`, `minor`,
/// `minor_scl`). `None` for models with no variants (CE gbxmodels), which fall
/// back to base permutations.
pub(super) fn default_variant_index(data: &ModelPreviewData) -> Option<usize> {
    let named = |name: &str| {
        data.variants
            .iter()
            .position(|v| v.name.eq_ignore_ascii_case(name))
    };
    data.default_variant
        .as_deref()
        .and_then(named)
        .or_else(|| named("default"))
        .or((!data.variants.is_empty()).then_some(0))
}

pub(super) fn reset_model_preview_selection(
    state: &mut ModelPreviewState,
    data: &ModelPreviewData,
    variant: Option<usize>,
) {
    state.selected_variant = variant;
    state.region_selections = compute_variant_selection(data, variant);
}

/// The region selection (enabled + permutation per region) that selecting
/// `variant` (`None` = base/`<None>`) produces. Pure — used both to apply a
/// variant and to reverse-detect which variant the live selection matches.
pub(super) fn compute_variant_selection(
    data: &ModelPreviewData,
    variant: Option<usize>,
) -> HashMap<String, ModelRegionSelection> {
    let mut selections = HashMap::new();
    let selected_variant = variant.and_then(|idx| data.variants.get(idx));
    for region in &data.preview.regions {
        let named = selected_variant.and_then(|variant| variant.regions.get(&region.name));
        let hidden =
            selected_variant.is_some_and(|variant| variant.hidden_regions.contains(&region.name));
        let (enabled, permutation) = match named {
            Some(name) if region.permutations.contains(name) => (true, name.clone()),
            // The game resolves a name the render model lacks to no permutation.
            Some(_) => (false, default_permutation(region)),
            None => {
                let empty = data.preview.instances.empty_regions.contains(&region.name);
                (
                    !hidden && !empty && !region.permutations.is_empty(),
                    default_permutation(region),
                )
            }
        };
        selections.insert(
            region.name.clone(),
            ModelRegionSelection {
                enabled,
                permutation,
            },
        );
    }
    selections
}

/// What `model_select_initial_permutation` falls back on: an ordinary
/// permutation, as a name starting `~` keeps one out of its choosing.
pub(super) fn default_permutation(region: &RenderModelPreviewRegion) -> String {
    region
        .permutations
        .iter()
        .find(|permutation| !permutation.starts_with('~'))
        .or(region.permutations.first())
        .cloned()
        .unwrap_or_default()
}

/// Reverse-sync: which variant choice (`None` = `<None>`/base, `Some(idx)` =
/// a named variant) the live region selection currently matches, or `None` if
/// it matches no known variant ("(custom)"). Checks the active selection first
/// so an exact match stays put.
pub(super) fn detect_active_variant(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<Option<usize>> {
    let matches = |choice: Option<usize>| {
        let computed = compute_variant_selection(data, choice);
        computed.len() == state.region_selections.len()
            && computed
                .iter()
                .all(|(name, sel)| state.region_selections.get(name) == Some(sel))
    };
    let current = state.selected_variant;
    if matches(current) {
        return Some(current);
    }
    if current.is_some() && matches(None) {
        return Some(None);
    }
    for idx in 0..data.variants.len() {
        let choice = Some(idx);
        if choice != current && matches(choice) {
            return Some(choice);
        }
    }
    None // matches no known variant → "(custom)"
}

pub(super) fn selected_variant_regions(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<ModelVariantRegionChoice> {
    data.preview
        .regions
        .iter()
        .filter_map(|region| {
            // Physics is an overlay toggle, not a .model variant region.
            if is_model_physics_overlay_region(data, state, region) {
                return None;
            }
            let selection = state.region_selections.get(&region.name)?;
            if !selection.enabled
                || selection.permutation.is_empty()
                || !region
                    .permutations
                    .iter()
                    .any(|p| p == &selection.permutation)
            {
                return None;
            }
            Some(ModelVariantRegionChoice {
                region_name: region.name.clone(),
                permutation_name: selection.permutation.clone(),
            })
        })
        .collect()
}

/// The physics overlay uses a synthetic `physics/default` draw batch. Hide it
/// from Model Setup while preserving its internal selection for draw filtering.
/// A genuine render region named `physics` (or a standalone physics tag) must
/// still appear normally.
pub(super) fn is_model_physics_overlay_region(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    region: &RenderModelPreviewRegion,
) -> bool {
    state.overlays_loaded
        && region.name == PHYSICS_REGION
        && data.preview.batches.iter().any(|batch| {
            batch.layer == ModelPreviewLayer::Physics && batch.region_name == region.name
        })
        && !data.preview.batches.iter().any(|batch| {
            batch.layer == ModelPreviewLayer::Render && batch.region_name == region.name
        })
}

pub(super) fn normalized_new_variant_name(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<String> {
    let name = state.new_variant_name.trim();
    if name.is_empty() {
        return None;
    }
    if data
        .variants
        .iter()
        .any(|variant| variant.name.eq_ignore_ascii_case(name))
    {
        return None;
    }
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::super::{
        ModelVariantPreview, RenderModelPreview, RenderModelPreviewRegion, model_preview_data,
    };
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn an_object_s_default_variant_comes_first() {
        let variant = |name: &str| ModelVariantPreview {
            name: name.to_owned(),
            regions: HashMap::new(),
            hidden_regions: Default::default(),
            muted_nodes: Vec::new(),
        };
        let mut data = model_preview_data(
            String::new(),
            String::new(),
            RenderModelPreview::default(),
            vec![variant("first"), variant("default"), variant("chosen")],
        );
        assert_eq!(default_variant_index(&data), Some(1));
        data.default_variant = Some("chosen".to_owned());
        assert_eq!(default_variant_index(&data), Some(2));
        data.default_variant = Some("gone".to_owned());
        assert_eq!(default_variant_index(&data), Some(1));
        data.variants.remove(1);
        assert_eq!(default_variant_index(&data), Some(0));
        data.variants.clear();
        assert_eq!(default_variant_index(&data), None);
    }

    #[test]
    fn a_tilde_permutation_is_never_the_default() {
        let region = |permutations: &[&str]| RenderModelPreviewRegion {
            name: String::new(),
            permutations: permutations.iter().map(|name| (*name).to_owned()).collect(),
        };
        assert_eq!(default_permutation(&region(&["~hidden", "base"])), "base");
        assert_eq!(default_permutation(&region(&["~only"])), "~only");
    }
}
