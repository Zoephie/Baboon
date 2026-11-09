//! The panel's variant controls: the picker, the selector and the header
//! actions that write a new variant to the tag.

use super::super::super::super::core::document::ops::{BlockOpKind, ModelVariantOp};
use super::super::super::ui_kit::button_icons::{
    ButtonIcon, close_menu, icon_button, icon_text_button, icon_text_dropdown_button,
    selectable_text_button,
};
use super::super::super::ui_kit::style::{BUTTON_HEIGHT, bold_font, subtle_dark, text_dark};
use super::super::{
    BlockConfirm, FieldEditContext, ModelPreviewData, ModelPreviewState, ModelRegionSelection,
    ModelVariantPreview, combo_scroll_next_index, default_permutation, detect_active_variant,
    dropdown_wheel_delta, is_model_physics_overlay_region, normalized_new_variant_name,
    picker_button, picker_popup_width, picker_results, picker_search_field,
    reset_model_preview_selection, selected_variant_regions,
};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use eframe::egui;
use eframe::egui::{RichText, Ui, Vec2};

pub(in crate::app::model_preview) fn draw_variant_controls(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let region_name_width = data
        .preview
        .regions
        .iter()
        .filter(|region| !is_model_physics_overlay_region(data, state, region))
        .map(|region| {
            ui.painter()
                .layout_no_wrap(region.name.clone(), bold_font(13.0), text_dark())
                .size()
                .x
        })
        .fold(90.0_f32, f32::max)
        .min(180.0);
    let row_width = ui.available_width().max(1.0);
    ui.set_max_width(row_width);
    let mut visible_rows = 0;
    for region in &data.preview.regions {
        if is_model_physics_overlay_region(data, state, region) {
            continue;
        }
        if visible_rows > 0 {
            ui.separator();
        }
        visible_rows += 1;
        let selection = state
            .region_selections
            .entry(region.name.clone())
            .or_insert_with(|| ModelRegionSelection {
                enabled: !region.permutations.is_empty(),
                permutation: default_permutation(region),
            });
        ui.horizontal_wrapped(|ui| {
            ui.baboon_checkbox(&mut selection.enabled, "");
            ui.add_sized(
                Vec2::new(region_name_width, BUTTON_HEIGHT),
                egui::Label::new(
                    RichText::new(&region.name)
                        .color(text_dark())
                        .font(bold_font(13.0)),
                ),
            );
            for permutation in &region.permutations {
                let selected = selection.permutation == *permutation;
                let response = selectable_text_button(ui, permutation, selected);
                if response.clicked() {
                    selection.permutation = permutation.clone();
                    selection.enabled = true;
                }
            }
        });
    }
}

/// Search variants without losing the `<None>` reset or wheel navigation.
fn draw_variant_picker(
    ui: &mut Ui,
    source_key: &str,
    selected_label: &str,
    variants: &[ModelVariantPreview],
    selected: Option<usize>,
) -> (Option<Option<usize>>, Option<i32>) {
    let popup_id = ui.make_persistent_id(("model_preview_variant", source_key));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = picker_button(
        ui,
        popup_id,
        selected_label,
        variants.len(),
        150.0,
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

pub(in crate::app::model_preview) fn draw_variant_selector(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    // Reverse-sync: reflect manual region/permutation tweaks in the combo —
    // show the matching variant, or "(custom)" when the live selection matches
    // none. When it lands exactly on a variant, adopt it so Update/Drop target
    // the shown variant.
    let mut active_variant = detect_active_variant(data, state);
    if let Some(choice) = active_variant {
        state.selected_variant = choice;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
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
        let (choice, wheel_delta) = draw_variant_picker(
            ui,
            &data.source_key,
            &selected,
            &data.variants,
            state.selected_variant,
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
    });
}

pub(in crate::app::model_preview) fn draw_variant_header_actions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    let mut mutation_requested = false;
    icon_text_dropdown_button(ui, ButtonIcon::Save, "Save", |ui| {
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
    let can_delete = edit.editable && state.selected_variant.is_some();
    if icon_text_button(ui, ButtonIcon::Garbage, "Delete", can_delete)
        .on_hover_text("Delete selected variant")
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
    mutation_requested
}

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
