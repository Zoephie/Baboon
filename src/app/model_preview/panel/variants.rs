//! The panel's variant controls: the picker, the selector and the header
//! actions that write a new variant to the tag.

use super::super::super::super::core::document::ops::{BlockOpKind, ModelVariantOp};
use super::super::super::ui_kit::button_icons::{
    ButtonIcon, close_menu, icon_button, icon_text_button, icon_text_dropdown_button,
};
use super::super::super::ui_kit::style::{ICON_BUTTON_SIZE, subtle_dark, text_dark};
use super::rows::{CardRows, name_cell, name_column_width};
use super::super::{
    BlockConfirm, FieldEditContext, ModelPreviewData, ModelPreviewState, ModelRegionSelection,
    ModelVariantPreview, combo_scroll_next_index, default_permutation, detect_active_variant,
    dropdown_wheel_delta, is_model_physics_overlay_region, normalized_new_variant_name,
    picker_button, picker_popup_width, picker_results, picker_search_field,
    reset_model_preview_selection, selected_variant_regions,
};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui;
use eframe::egui::{RichText, Ui};

/// The names of the weapons the preview was read holding: their regions
/// are named `"{weapon}: {region}"`, and show in the Weapons card.
pub(in crate::app::model_preview) fn held_weapon_names(state: &ModelPreviewState) -> Vec<&str> {
    [state.loaded_weapon.as_deref(), state.loaded_secondary_weapon.as_deref()]
        .into_iter()
        .flatten()
        .map(crate::core::tag_path::name)
        .collect()
}

/// What `region` is called on the weapon `weapon` brought, if it is one of
/// that weapon's.
pub(in crate::app::model_preview) fn weapon_region<'a>(region: &'a str, weapon: &str) -> Option<&'a str> {
    region.strip_prefix(weapon)?.strip_prefix(": ")
}

/// A row per region of the model's own, held weapons' left to the Weapons
/// card: whether it draws, its name, and its permutations.
pub(in crate::app::model_preview) fn draw_variant_controls(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    rows: &mut CardRows,
) {
    let weapons: Vec<String> = held_weapon_names(state).into_iter().map(str::to_owned).collect();
    let regions: Vec<(usize, &str)> = data
        .preview
        .regions
        .iter()
        .enumerate()
        .filter(|(_, region)| !is_model_physics_overlay_region(data, region))
        .filter(|(_, region)| !weapons.iter().any(|weapon| weapon_region(&region.name, weapon).is_some()))
        .map(|(index, region)| (index, region.name.as_str()))
        .collect();
    draw_region_rows(ui, data, state, rows, &regions);
}

/// A row for each of `regions` (an index into the preview's, and the name
/// to show it by): its checkbox, its name in a shared column, and its
/// permutations between arrows that step through them.
pub(in crate::app::model_preview) fn draw_region_rows(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    rows: &mut CardRows,
    regions: &[(usize, &str)],
) {
    let row_width = ui.available_width().max(1.0);
    // Past the checkbox: the names share a column, which gives way first in
    // a narrow card, leaving the checkbox, the arrows and some of the picker.
    let fixed = 2.0 * ICON_BUTTON_SIZE.x + 20.0 + 4.0 * 6.0 + 60.0;
    let name_width = name_column_width(ui, regions.iter().map(|(_, name)| *name), row_width)
        .min((row_width - fixed).max(30.0));
    for &(index, label) in regions {
        let region = &data.preview.regions[index];
        let selection = state
            .region_selections
            .entry(region.name.clone())
            .or_insert_with(|| ModelRegionSelection {
                enabled: !region.permutations.is_empty(),
                permutation: default_permutation(region),
            });
        let current = region
            .permutations
            .iter()
            .position(|permutation| *permutation == selection.permutation);
        let count = region.permutations.len();
        rows.row(ui, |ui| {
            ui.baboon_checkbox(&mut selection.enabled, "");
            name_cell(ui, label, name_width);
            let mut stepped = None;
            if icon_button(
                ui,
                ButtonIcon::Left,
                "Previous permutation",
                current.is_some_and(|at| at > 0),
                text_dark(),
            )
            .clicked()
            {
                stepped = current.map(|at| at - 1);
            }
            let arrow = ICON_BUTTON_SIZE.x + ui.spacing().item_spacing.x;
            let width = (ui.available_width() - arrow).max(40.0);
            let salt = (data.source_key.as_str(), region.name.as_str());
            let (choice, wheel_delta) = draw_permutation_picker(
                ui,
                salt,
                &selection.permutation,
                &region.permutations,
                width,
            );
            if icon_button(
                ui,
                ButtonIcon::Right,
                "Next permutation",
                current.map_or(count > 0, |at| at + 1 < count),
                text_dark(),
            )
            .clicked()
            {
                stepped = Some(current.map_or(0, |at| at + 1));
            }
            let wheeled = wheel_delta
                .and_then(|delta| combo_scroll_next_index(current.unwrap_or(0), count, delta));
            let stepped = stepped.or(wheeled).map(|next| region.permutations[next].clone());
            if let Some(permutation) = choice.or(stepped) {
                selection.permutation = permutation;
                selection.enabled = true;
            }
        });
    }
}

/// A region's permutations as the app's searchable dropdown: the chosen
/// permutation, if one was picked, and a wheel turn over the closed button.
pub(in crate::app::model_preview) fn draw_permutation_picker(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    selected: &str,
    permutations: &[String],
    width: f32,
) -> (Option<String>, Option<i32>) {
    let popup_id = ui.make_persistent_id(("model_preview_permutation", salt));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = picker_button(
        ui,
        popup_id,
        selected,
        Some(permutations.len()),
        width,
        text_dark(),
        !permutations.is_empty(),
    );
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut choice = None;
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_id = popup_id.with("entry_search");
            let mut filter = ui
                .data(|data| data.get_temp::<String>(filter_id))
                .unwrap_or_default();
            let changed = picker_search_field(ui, &mut filter, "search permutations…", just_opened);
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            ui.separator();
            let query = filter.trim().to_lowercase();
            picker_results(ui, 300.0, changed, |ui| {
                let mut shown = 0;
                for permutation in permutations {
                    if !query.is_empty() && !permutation.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    let current = permutation == selected;
                    let row = ui.selectable_label(current, permutation);
                    if just_opened && query.is_empty() && current {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        choice = Some(permutation.clone());
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No permutations match.").color(subtle_dark()));
                }
            });
        });
    (choice, dropdown_wheel_delta(ui, &response, popup_open))
}

/// Search variants without losing the `<None>` reset or wheel navigation.
fn draw_variant_picker(
    ui: &mut Ui,
    source_key: &str,
    selected_label: &str,
    variants: &[ModelVariantPreview],
    selected: Option<usize>,
    width: f32,
) -> (Option<Option<usize>>, Option<i32>) {
    let popup_id = ui.make_persistent_id(("model_preview_variant", source_key));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = picker_button(
        ui,
        popup_id,
        selected_label,
        Some(variants.len()),
        width,
        text_dark(),
        true,
    );
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut choice = None;
    // Open while its id is open in memory, as the button above toggles it;
    // left alone, a popup built from a response is always open.
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_id = popup_id.with("entry_search");
            let mut filter = ui
                .data(|data| data.get_temp::<String>(filter_id))
                .unwrap_or_default();
            let changed = picker_search_field(ui, &mut filter, "search variants…", just_opened);
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            ui.separator();
            let query = filter.trim().to_lowercase();
            picker_results(ui, 300.0, changed, |ui| {
                let mut shown = 0;
                if query.is_empty() || "<none>".contains(&query) {
                    shown += 1;
                    if ui.selectable_label(selected.is_none(), "<None>").clicked() {
                        choice = Some(None);
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                for (index, variant) in variants.iter().enumerate() {
                    let label = format!("{index}. {}", variant.name);
                    if !query.is_empty() && !label.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    let row = ui.selectable_label(selected == Some(index), label);
                    if just_opened && query.is_empty() && selected == Some(index) {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        choice = Some(Some(index));
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No variants match.").color(subtle_dark()));
                }
            });
        });
    (choice, dropdown_wheel_delta(ui, &response, popup_open))
}

/// The Permutations card's first row: the variant between arrows that step
/// through them, stretched to what `actions` (Save and Delete, drawn right
/// to left from the row's right end) leave.
pub(in crate::app::model_preview) fn draw_variant_selector(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    rows: &mut CardRows,
    actions: &mut dyn FnMut(&mut Ui, &mut ModelPreviewState),
) {
    // Reverse-sync: reflect manual region/permutation tweaks in the combo —
    // show the matching variant, or "(custom)" when the live selection matches
    // none. When it lands exactly on a variant, adopt it so Update/Drop target
    // the shown variant.
    let active_variant = detect_active_variant(data, state);
    if let Some(choice) = active_variant {
        state.selected_variant = choice;
    }
    rows.row(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            actions(ui, state);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                draw_variant_stepper(ui, data, state, active_variant);
            });
        });
    });
}

/// The variant picker between its arrows, filling the width it is given.
fn draw_variant_stepper(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    mut active_variant: Option<Option<usize>>,
) {
    {
        if icon_button(
            ui,
            ButtonIcon::Left,
            "Previous variant",
            state.selected_variant.is_some_and(|index| index > 0),
            text_dark(),
        )
        .clicked()
        {
            let previous = state.selected_variant.expect("previous variant is enabled") - 1;
            reset_model_preview_selection(state, data, Some(previous));
            active_variant = Some(Some(previous));
        }
        let selected = match active_variant {
            Some(None) => "<None>".to_owned(),
            Some(Some(idx)) => data
                .variants
                .get(idx)
                .map(|variant| format!("{idx}. {}", variant.name))
                .unwrap_or_else(|| "<None>".to_owned()),
            None => "(custom)".to_owned(),
        };
        let arrow = ICON_BUTTON_SIZE.x + ui.spacing().item_spacing.x;
        let width = (ui.available_width() - arrow).max(100.0);
        let (choice, wheel_delta) = draw_variant_picker(
            ui,
            &data.source_key,
            &selected,
            &data.variants,
            state.selected_variant,
            width,
        );
        if let Some(choice) = choice {
            reset_model_preview_selection(state, data, choice);
        }
        if let Some(delta) = wheel_delta {
            let current = state
                .selected_variant
                .map(|index| index as i32 + 1)
                .unwrap_or(0);
            if let Some(next) =
                combo_scroll_next_index(current as usize, data.variants.len() + 1, delta)
            {
                let selected_variant = if next == 0 { None } else { Some(next - 1) };
                reset_model_preview_selection(state, data, selected_variant);
            }
        }
        let next = match state.selected_variant {
            Some(index) if index + 1 < data.variants.len() => Some(index + 1),
            None if !data.variants.is_empty() => Some(0),
            _ => None,
        };
        if icon_button(
            ui,
            ButtonIcon::Right,
            "Next variant",
            next.is_some(),
            text_dark(),
        )
        .clicked()
        {
            reset_model_preview_selection(state, data, next);
        }
    }
}

pub(in crate::app::model_preview) fn draw_variant_header_actions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    let mut mutation_requested = false;
    // Drawn right to left from the row's end: Delete last, Save before it.
    let can_delete = edit.editable && state.selected_variant.is_some();
    // Icon-only, so the variant picker keeps its room in a narrow sidebar.
    if icon_button(ui, ButtonIcon::Garbage, "Delete selected variant", can_delete, text_dark())
        .clicked()
    {
        let variant_index = state
            .selected_variant
            .expect("button enabled only when a variant is selected");
        *edit.block_confirm = Some(BlockConfirm {
            opened_at: None,
            kit: None,
            tag_key: edit.tag_key.to_owned(),
            path: "variants".to_owned(),
            kind: BlockOpKind::Delete(variant_index),
            message: format!(
                "Delete element {variant_index} of {} from this block?",
                data.variants.len()
            ),
            confirm_label: "Delete".to_owned(),
        });
    }
    // Where the row is too narrow for Save's label beside a readable
    // variant picker, Save is its icon and arrow alone.
    let compact = ui.available_width() < COMPACT_SAVE_BELOW;
    let save = icon_text_dropdown_button(ui, ButtonIcon::Save, if compact { "" } else { "Save" }, |ui| {
        ui.set_min_width(240.0);
        ui.label("New Variant Name");
        ui.add_enabled(
            edit.editable,
            crate::app::ui_kit::singleline_text_edit(&mut state.new_variant_name)
                .desired_width(220.0),
        );
        let chosen_regions = selected_variant_regions(data, state);
        let create_name = normalized_new_variant_name(data, state);
        let can_create = edit.editable && create_name.is_some() && !chosen_regions.is_empty();
        if icon_text_button(ui, ButtonIcon::Add, "Save as New Variant", can_create)
            .on_hover_text("Create a .model variant using the visible region selections.")
            .clicked()
        {
            let name = create_name.expect("button enabled only when name is valid");
            state.variant_after_load = Some(name.clone());
            edit.model_variant_ops.push(ModelVariantOp::Create {
                name,
                regions: chosen_regions.clone(),
            });
            state.new_variant_name.clear();
            mutation_requested = true;
            close_menu(ui);
        }
        let can_update =
            edit.editable && state.selected_variant.is_some() && !chosen_regions.is_empty();
        if ui
            .add_enabled(can_update, egui::Button::new("Update Selected Variant"))
            .on_hover_text("Replace the selected variant's region permutations.")
            .clicked()
        {
            edit.model_variant_ops.push(ModelVariantOp::Update {
                variant_index: state
                    .selected_variant
                    .expect("button enabled only when a variant is selected"),
                regions: chosen_regions,
            });
            mutation_requested = true;
            close_menu(ui);
        }
    });
    if compact {
        save.response.on_hover_text("Save variant");
    }
    mutation_requested
}

/// The room left of Delete below which Save drops its label: Save's own
/// width, the variant arrows, and some 170 points of picker.
const COMPACT_SAVE_BELOW: f32 = 320.0;

#[cfg(test)]
mod tests {
    use super::super::super::super::ui_kit::style::{foundation_fonts, foundation_style};
    use super::super::super::ModelVariantPreview;
    use super::*;
    use eframe::egui;
    use eframe::egui::Vec2;
    use std::collections::HashMap;

    #[test]
    fn variant_picker_filters_and_preserves_none_and_original_indices() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let variants = ["minor", "major", "special veteran"]
            .into_iter()
            .map(|name| ModelVariantPreview {
                name: name.to_owned(),
                regions: HashMap::new(),
                hidden_regions: Default::default(),
                muted_nodes: Vec::new(),
            })
            .collect::<Vec<_>>();
        let frame = |events| {
            let mut result = None;
            let mut popup_id = None;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        popup_id = Some(ui.make_persistent_id(("model_preview_variant", "test")));
                        result = Some(draw_variant_picker(
                            ui,
                            "test",
                            "0. minor",
                            &variants,
                            Some(0),
                            150.0,
                        ));
                    });
                },
            );
            (result.unwrap().0, output, popup_id.unwrap())
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(Vec::new());
        let pos = egui::pos2(70.0, 18.0);
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        frame(vec![pointer(pos, false)]);
        frame(Vec::new());
        let (_, output, popup_id) = frame(vec![egui::Event::Text("VETERAN".to_owned())]);
        let row_pos = |output: &egui::FullOutput, text: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == text => {
                        Some(shape.pos + shape.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("picker row")
        };
        let pos = row_pos(&output, "2. special veteran");
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        assert_eq!(frame(vec![pointer(pos, false)]).0, Some(Some(2)));
        ctx.data_mut(|data| data.insert_temp(popup_id.with("entry_search"), String::new()));
        egui::Popup::open_id(&ctx, popup_id);
        frame(Vec::new());
        let (_, output, _) = frame(Vec::new());
        let pos = row_pos(&output, "<None>");
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        assert_eq!(frame(vec![pointer(pos, false)]).0, Some(None));
    }
}
