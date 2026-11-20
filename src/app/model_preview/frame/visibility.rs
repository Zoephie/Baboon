//! Which batches, instances, markers and layers a frame draws.

use super::super::{
    MaterialTextures, ModelPreviewLayer, ModelPreviewState, ModelRegionSelection,
    RenderModelPreview,
};
use std::collections::HashMap;

/// Leave out what only a shield draws, once the materials are known.
pub(in crate::app::model_preview) fn without_shield_parts(
    visible: &mut Vec<usize>,
    preview: &RenderModelPreview,
    textures: Option<&[MaterialTextures]>,
) {
    let Some(textures) = textures else {
        return;
    };
    visible.retain(|&batch| {
        let material = preview
            .batches
            .get(batch)
            .map_or(0, |batch| batch.material_index as usize);
        !textures
            .get(material)
            .is_some_and(|texture| texture.shield_only)
    });
}

pub(in crate::app::model_preview) fn visible_batch_indices(
    preview: &RenderModelPreview,
    state: &ModelPreviewState,
) -> Vec<usize> {
    let overlay_has_selected = |layer: ModelPreviewLayer, region: &str, permutation: &str| {
        preview.batches.iter().any(|batch| {
            batch.layer == layer
                && batch.region_name == region
                && batch.permutation_name == permutation
        })
    };
    let overlay_first_permutation = |layer: ModelPreviewLayer, region: &str| {
        preview
            .batches
            .iter()
            .find(|batch| batch.layer == layer && batch.region_name == region)
            .map(|batch| batch.permutation_name.as_str())
    };
    let instances = visible_instances(preview, state);
    preview
        .batches
        .iter()
        .enumerate()
        .filter_map(|(index, batch)| {
            if !model_layer_visible(state, batch.layer) {
                return None;
            }
            if let Some(instance) = preview.instances.batches.get(&index) {
                // A mask holds 128 placements; one past them is never chosen.
                let chosen = u32::try_from(*instance)
                    .ok()
                    .and_then(|shift| instances.checked_shr(shift));
                return (chosen.unwrap_or(0) & 1 != 0).then_some(index);
            }
            let selection = state.region_selections.get(&batch.region_name)?;
            if !selection.enabled {
                return None;
            }
            if selection.permutation == batch.permutation_name {
                return Some(index);
            }
            // An overlay or stand-in without the permutation picked draws
            // its first.
            let is_overlay = matches!(
                batch.layer,
                ModelPreviewLayer::LodModel | ModelPreviewLayer::Imposter
            ) || (state.overlays_loaded && batch.layer != ModelPreviewLayer::Render);
            (is_overlay
                && !overlay_has_selected(batch.layer, &batch.region_name, &selection.permutation)
                && overlay_first_permutation(batch.layer, &batch.region_name)
                    == Some(batch.permutation_name.as_str()))
            .then_some(index)
        })
        .collect()
}

/// `model_compute_instance_bitfield_visible`: what the selected permutations
/// allow of what the variant's instance group chose.
fn visible_instances(preview: &RenderModelPreview, state: &ModelPreviewState) -> u128 {
    let instances = &preview.instances;
    let chosen = state
        .selected_variant
        .and_then(|variant| instances.variants.get(variant));
    let allowed = state
        .region_selections
        .iter()
        .filter(|(_, selection)| selection.enabled);
    let allowed = allowed.filter_map(|(region, selection)| {
        instances
            .masks
            .get(&(region.clone(), selection.permutation.clone()))
    });
    chosen.copied().unwrap_or(0) & allowed.fold(0, |mask, permutation| mask | permutation)
}

pub(in crate::app::model_preview) fn marker_active(
    only: &(String, String),
    selections: &HashMap<String, ModelRegionSelection>,
) -> bool {
    let selection = selections.get(&only.0);
    selection.is_some_and(|selection| selection.enabled && selection.permutation == only.1)
}

pub(in crate::app::model_preview) fn model_layer_visible(
    state: &ModelPreviewState,
    layer: ModelPreviewLayer,
) -> bool {
    // The LOD model is drawn in the render model's place, and only while
    // the render model is shown.
    let lod_swapped = state.show_lod_model && state.show_render;
    if layer == ModelPreviewLayer::LodModel {
        return lod_swapped;
    }
    if layer == ModelPreviewLayer::Render && lod_swapped {
        return false;
    }
    if layer == ModelPreviewLayer::Imposter {
        return state.show_imposter;
    }
    if !state.overlays_loaded {
        // A bare render/collision/physics tag is the primary preview, not an
        // optional overlay. `show_render` remains true for that path.
        return state.show_render;
    }
    match layer {
        ModelPreviewLayer::Render => state.show_render,
        ModelPreviewLayer::Collision => state.show_collision,
        ModelPreviewLayer::Physics => state.show_physics,
        ModelPreviewLayer::LodModel => lod_swapped,
        ModelPreviewLayer::Imposter => state.show_imposter,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{
        MaterialTextures, ModelPreviewLayer, ModelPreviewState, ModelRegionSelection,
        RenderModelPreview, RenderModelPreviewBatch,
    };
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_shields_parts_leave_the_ordinary_passes_once_materials_are_known() {
        let batch = |material_index| RenderModelPreviewBatch {
            material_index,
            ..Default::default()
        };
        let preview = RenderModelPreview {
            batches: vec![batch(0), batch(1), batch(0)],
            ..Default::default()
        };
        let textures = [
            MaterialTextures::default(),
            MaterialTextures {
                shield_only: true,
                ..Default::default()
            },
        ];
        let mut visible = vec![0, 1, 2];
        without_shield_parts(&mut visible, &preview, None);
        assert_eq!(visible, [0, 1, 2], "nothing is known yet");
        without_shield_parts(&mut visible, &preview, Some(&textures));
        assert_eq!(visible, [0, 2]);
    }

    #[test]
    fn a_marker_of_one_permutation_goes_with_that_permutation() {
        let selections = HashMap::from([(
            "f_hull".to_owned(),
            ModelRegionSelection {
                enabled: true,
                permutation: "base".to_owned(),
            },
        )]);
        let of = |region: &str, permutation: &str| (region.to_owned(), permutation.to_owned());
        assert!(marker_active(&of("f_hull", "base"), &selections));
        assert!(!marker_active(&of("f_hull", "minor"), &selections));
        assert!(!marker_active(&of("hull", "base"), &selections));
        let mut off = selections.clone();
        off.get_mut("f_hull").unwrap().enabled = false;
        assert!(!marker_active(&of("f_hull", "base"), &off));
    }

    #[test]
    fn error_visibility_follows_its_owning_model_layer() {
        let mut state = ModelPreviewState {
            overlays_loaded: true,
            show_render: true,
            show_collision: false,
            show_physics: true,
            ..Default::default()
        };
        assert!(model_layer_visible(&state, ModelPreviewLayer::Render));
        assert!(!model_layer_visible(&state, ModelPreviewLayer::Collision));
        assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));

        state.show_render = false;
        assert!(!model_layer_visible(&state, ModelPreviewLayer::Render));
        assert!(model_layer_visible(&state, ModelPreviewLayer::Physics));
    }

    #[test]
    fn the_lod_model_takes_the_render_models_place_while_it_is_shown() {
        let mut state = ModelPreviewState {
            show_render: true,
            ..Default::default()
        };
        let visible = |state: &ModelPreviewState| {
            [ModelPreviewLayer::Render, ModelPreviewLayer::LodModel]
                .map(|layer| model_layer_visible(state, layer))
        };
        assert_eq!(visible(&state), [true, false]);
        state.show_lod_model = true;
        assert_eq!(visible(&state), [false, true]);
        state.show_render = false;
        assert_eq!(visible(&state), [false, false], "hiding the render model hides both");
    }
}
