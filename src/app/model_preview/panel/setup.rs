//! The setup card: its header controls, its body and the scenario BSP list.

use super::super::super::ui_kit::style::subtle_dark;
use super::super::{FieldEditContext, ModelPreviewData, ModelPreviewState, bsp_display_name};
use super::layout::{PreviewPanel, SetupRequests};
use super::section::{
    MODEL_SETUP_EXTRA_HEADER_HEIGHT, MODEL_SETUP_INLINE_HEADER_MIN_WIDTH,
    MODEL_SETUP_INLINE_SELECTOR_MIN_WIDTH, MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH,
};
use super::variants::{draw_variant_header_actions, draw_variant_selector};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui::{RichText, Ui};

/// The Model Setup card's header actions: the variant actions.
pub(in crate::app::model_preview) fn draw_setup_header_actions(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
    requests: &mut SetupRequests,
) {
    if setup_has_actions(panel.entry.group_tag)
        && draw_variant_header_actions(ui, panel.data, state, edit)
    {
        requests.mutation = true;
    }
}

/// A scenario's per-BSP toggle list. Region toggles cannot serve here: a
/// region only exists once its BSP is loaded, and the point of this list is
/// choosing what to load in the first place.
pub(in crate::app::model_preview) fn draw_scenario_setup(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    if !data.scenario_bsps.is_empty() {
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
                RichText::new("Check a structure BSP to load its geometry.").color(subtle_dark()),
            );
        }
    }
}

/// Whether the Model Setup header shows the variant actions: only a
/// `.model` does, as an object shows its `.model`'s variants but holds none,
/// so there is nothing for it to write.
pub(in crate::app::model_preview) fn setup_has_actions(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"hlmt")
}

/// The Model Setup header's rows below its first at `width`, for a header
/// with the variant actions (`has_actions`) or with the variant picker alone.
pub(in crate::app::model_preview) fn model_setup_extra_header_height(
    width: f32,
    has_actions: bool,
) -> f32 {
    if !has_actions {
        if width >= MODEL_SETUP_INLINE_SELECTOR_MIN_WIDTH {
            0.0
        } else {
            MODEL_SETUP_EXTRA_HEADER_HEIGHT
        }
    } else if width >= MODEL_SETUP_INLINE_HEADER_MIN_WIDTH {
        0.0
    } else if width >= MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT
    } else {
        MODEL_SETUP_EXTRA_HEADER_HEIGHT * 2.0
    }
}

/// The Model Setup header's controls: the variant picker, then the variant
/// actions when `has_actions`.
pub(in crate::app::model_preview) fn draw_model_setup_header_controls(
    ui: &mut Ui,
    section_width: f32,
    has_actions: bool,
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
    if !has_actions {
        draw_variant_selector(ui, data, state);
    } else if section_width < MODEL_SETUP_SHARED_CONTROLS_ROW_MIN_WIDTH {
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
