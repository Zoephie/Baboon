//! The setup card: its header controls, its body and the scenario BSP list.

use super::*;

/// The Model Setup card's header actions: the variant actions, then Refresh
/// Model.
pub(in crate::app::model_preview) fn draw_setup_header_actions(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
    requests: &mut SetupRequests,
) {
    // An object shows its `.model`'s variants but
    // holds none, so there is nothing here to write.
    if panel.entry.group_tag == u32::from_be_bytes(*b"hlmt")
        && draw_variant_header_actions(ui, panel.data, state, edit)
    {
        requests.mutation = true;
    }
    if icon_text_button(ui, ButtonIcon::Refresh, "Refresh Model", true).clicked() {
        state.invalidate_load();
        requests.reload = true;
    }
}

/// The Model Setup card's body: a scenario's BSPs, the variants and regions,
/// and the object's functions.
pub(in crate::app::model_preview) fn draw_setup_body(ui: &mut Ui, data: &ModelPreviewData, state: &mut ModelPreviewState) {
    draw_scenario_setup(ui, data, state);
    draw_variant_controls(ui, data, state);
    draw_object_functions(ui, data, state);
}

/// A scenario's per-BSP toggle list. Region toggles cannot serve here: a
/// region only exists once its BSP is loaded, and the point of this list is
/// choosing what to load in the first place.
pub(in crate::app::model_preview) fn draw_scenario_setup(ui: &mut Ui, data: &ModelPreviewData, state: &mut ModelPreviewState) {
    if !data.scenario_bsps.is_empty() {
        ui.label(RichText::new("Structure BSPs").strong().color(text_dark()));
        ui.horizontal_wrapped(|ui| {
            for (index, reference) in data.scenario_bsps.iter().enumerate() {
                let Some(reference) = reference else {
                    continue;
                };
                let mut checked = state.scenario_bsp_selection.contains(&index);
                if ui
                    .baboon_checkbox(&mut checked, bsp_display_name(reference))
                    .on_hover_text(reference.as_str())
                    .changed()
                {
                    if checked {
                        state.scenario_bsp_selection.insert(index);
                    } else {
                        state.scenario_bsp_selection.remove(&index);
                    }
                }
            }
        });
        if state.scenario_bsp_selection.is_empty() {
            ui.label(
                RichText::new("Check a structure BSP to load its geometry.")
                    .color(subtle_dark()),
            );
        }
        ui.separator();
    }
}

pub(in crate::app::model_preview) fn model_setup_extra_header_height(width: f32) -> f32 {
    if width >= MODEL_SETUP_INLINE_HEADER_MIN_WIDTH {
        0.0
    } else if width >= MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT
    } else {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT * 2.0
    }
}

pub(in crate::app::model_preview) fn draw_model_setup_header_controls(
    ui: &mut Ui,
    section_width: f32,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    mut draw_actions: impl FnMut(&mut Ui, &mut ModelPreviewState),
) {
    let mut draw_action_group = |ui: &mut Ui, state: &mut ModelPreviewState| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            draw_actions(ui, state);
        });
    };
    if section_width < MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
        // Three intact groups: title above, then variant navigation, then
        // Save/Delete/Refresh. Do not let individual buttons escape the card.
        ui.vertical(|ui| {
            draw_variant_selector(ui, data, state);
            draw_action_group(ui, state);
        });
    } else {
        // The title is either inline or on the row above; the two control
        // groups fit together at this width.
        draw_variant_selector(ui, data, state);
        draw_action_group(ui, state);
    }
}
