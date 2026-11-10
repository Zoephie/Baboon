//! The Permutations card's variant actions and the scenario BSP list.

use super::super::super::ui_kit::style::subtle_dark;
use super::super::{FieldEditContext, ModelPreviewData, ModelPreviewState, bsp_display_name};
use super::layout::{PreviewPanel, SetupRequests};
use super::variants::draw_variant_header_actions;
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui::{RichText, Ui};

/// The variant actions in the Permutations card's header.
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

/// Whether the Permutations header shows the variant actions: only a
/// `.model` does, as an object shows its `.model`'s variants but holds none,
/// so there is nothing for it to write.
pub(in crate::app::model_preview) fn setup_has_actions(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"hlmt")
}
