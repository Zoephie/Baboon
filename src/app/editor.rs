//! Tag presentation, mutation, parsing, bitmap preview, and structural diffing.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

mod sound;
pub(super) use sound::*;
mod mutations;
pub(super) use mutations::*;
mod value_parser;
pub(super) use value_parser::*;
mod bitmap;
pub(super) use bitmap::*;
mod model;
pub(super) use model::*;
mod diff;
pub(super) use diff::*;

use super::sound_extract::{
    ExtractItem, ExtractRequest, ExtractSource, reimport_base_dir_lang, sanitize_component,
};

pub(super) fn draw_entry_header(ui: &mut Ui, entry: &TagEntry, names: &TagNameIndex) {
    ui.heading(RichText::new(&entry.display_path).color(text_dark()));
    ui.horizontal(|ui| {
        ui.label(RichText::new("Group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, entry.group_tag)).color(text_dark()));
        if let Some(name) = &entry.group_name {
            ui.label(RichText::new(name).color(subtle_dark()));
        }
    });
    ui.separator();
}

pub(super) fn draw_tag(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    model_preview: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let is_object_family = is_object_family_group(entry.group_tag);
    let is_shaderish =
        is_material_tag(entry) || is_material_shader_tag(entry) || is_shader_tag(entry);
    let is_model = is_model_group(entry.group_tag, names);

    draw_tag_metadata(ui, tag, names);
    if !is_object_family {
        draw_object_model_summary(ui, tag, entry, names, edit);
    }
    if is_sound_classes_group(entry.group_tag) {
        draw_sound_classes_summary(ui, tag);
    }
    if is_sound_group(entry.group_tag) {
        draw_sound_player(ui, tag, edit);
    }
    if is_dialogue_group(entry.group_tag) {
        draw_dialogue_summary(ui, tag, edit);
    }
    if is_sound_looping_group(entry.group_tag) {
        draw_sound_looping_player(ui, tag, edit);
    }
    if is_material_effects_group(entry.group_tag) {
        draw_material_effects_summary(ui, tag, edit);
    }

    if is_model {
        draw_model_tag_panel_tabs(ui, model_preview);
    }
    ui.add_space(6.0);

    if is_model && model_preview.active_tab == ModelTagPanelTab::RenderModel {
        draw_model_preview_panel(
            ui,
            tag,
            entry,
            names,
            source,
            model_preview,
            model_preview_size,
            edit,
        );
        return;
    }

    draw_tag_fields_scroll(
        ui,
        tag,
        entry,
        names,
        source,
        rmdf_cache,
        rmop_cache,
        color_popup,
        function_popup,
        expert_mode,
        edit,
        is_object_family,
        is_shaderish,
    );
}

fn draw_model_tag_panel_tabs(ui: &mut Ui, model_preview: &mut ModelPreviewState) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::Fields,
            "Fields",
        );
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::RenderModel,
            "Render model",
        );
    });
}

fn draw_tag_fields_scroll(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    is_object_family: bool,
    is_shaderish: bool,
) {
    let scroll_height = ui.available_height().max(0.0);
    if is_shaderish {
        // The Guerilla-style shader grid is the single editing surface — no
        // separate field tab. The grid's bitmap/scalar/int/function/category
        // cells are editable inline; when the grid can't be built it falls
        // back to the standard editable field tree (inside draw_material_tag).
        ScrollArea::both()
            .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
            .max_height(scroll_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                draw_material_tag(
                    ui,
                    tag,
                    entry,
                    names,
                    source,
                    rmdf_cache,
                    rmop_cache,
                    color_popup,
                    function_popup,
                    expert_mode,
                    edit,
                );
            });
        return;
    }

    ScrollArea::both()
        .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
            if is_object_family {
                draw_inherited_object_fields(ui, tag.root(), names, expert_mode, edit);
            } else {
                draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
            }
        });
}

const TAG_FIELD_SCROLL_MIN_WIDTH: f32 = 980.0;

#[cfg(test)]
#[path = "editor/tests.rs"]
mod extracted_tests;
