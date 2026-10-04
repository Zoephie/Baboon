//! The tag editor: the tag pane, the generic field editor (`fields`), the
//! panels for particular groups (shader, material, function, sound, bitmap,
//! model), the edits they collect and how the app applies them.

use super::*;

mod sound;
pub(super) use sound::*;
mod apply_doc;
pub(in crate::app) mod actions;
#[cfg(test)]
mod campaign_evolved_field_paths_tests;
mod field_meta;
pub(super) use crate::core::document::apply::*;
pub(super) use crate::core::document::value::*;
pub(super) use field_meta::*;
mod bitmap;
pub(super) use bitmap::*;
mod model;
pub(super) use model::*;

use super::sound_extract::{
    ExtractItem, ExtractRequest, ExtractSource, reimport_base_dir_lang, sanitize_component,
};

pub(super) fn draw_tag(
    ui: &mut Ui,
    tag: &TagFile,
    // `(document id, dirty revision, kit generation)`: changes whenever `tag`
    // or what it is read against may have.
    document_revision: (u64, u64, u64, u64),
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    source_game: Option<GameId>,
    rmdf_cache: &mut HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    rmop_cache: &mut HashMap<String, Option<Arc<RenderMethodOption>>>,
    h2_templates: &mut H2TemplateCache,
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
    let is_model = is_previewable_geometry_group_for_game(entry.group_tag, names, source_game);

    if expert_mode {
        draw_tag_metadata(ui, tag, entry, names);
    }
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
        draw_model_tag_panel_tabs(ui, model_preview, entry.group_tag);
    }
    ui.add_space(6.0);

    if is_model && model_preview.active_tab == ModelTagPanelTab::ModelPreview {
        draw_model_preview_panel(
            ui,
            tag,
            entry,
            names,
            source_game,
            model_preview,
            model_preview_size,
            edit,
        );
        return;
    }

    draw_tag_fields_scroll(
        ui,
        tag,
        document_revision,
        entry,
        names,
        source,
        rmdf_cache,
        rmop_cache,
        h2_templates,
        color_popup,
        function_popup,
        expert_mode,
        edit,
        is_object_family,
        is_shaderish,
    );
}

fn draw_model_tag_panel_tabs(ui: &mut Ui, model_preview: &mut ModelPreviewState, group_tag: u32) {
    draw_preview_panel_toggle(
        ui,
        &mut model_preview.active_tab,
        ModelTagPanelTab::Fields,
        ModelTagPanelTab::ModelPreview,
        preview_panel_title(group_tag),
        ButtonIcon::RenderModel,
    );
}

pub(in crate::app) fn draw_preview_panel_toggle<T: Copy + PartialEq>(
    ui: &mut Ui,
    active: &mut T,
    fields: T,
    preview: T,
    preview_label: &str,
    preview_icon: ButtonIcon,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if view_tab_button(ui, ButtonIcon::DefaultTag, "Tag Fields", *active == fields).clicked() {
            *active = fields;
        }
        if view_tab_button(ui, preview_icon, preview_label, *active == preview).clicked() {
            *active = preview;
        }
    });
}

pub(in crate::app) fn view_tab_button(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: &str,
    selected: bool,
) -> egui::Response {
    view_tab_button_optional_icon(ui, Some(icon), label, selected)
}

pub(in crate::app) fn view_text_tab_button(
    ui: &mut Ui,
    label: &str,
    selected: bool,
) -> egui::Response {
    view_tab_button_optional_icon(ui, None, label, selected)
}

fn view_tab_button_optional_icon(
    ui: &mut Ui,
    icon: Option<ButtonIcon>,
    label: &str,
    selected: bool,
) -> egui::Response {
    const PADDING_X: f32 = 20.0;
    const PADDING_Y: f32 = 10.0;
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font_id, text_dark());
    let content_height = BUTTON_ICON_SIZE.max(galley.size().y);
    let icon_width = if icon.is_some() {
        BUTTON_ICON_SIZE + BUTTON_ICON_TEXT_GAP
    } else {
        0.0
    };
    let size = Vec2::new(
        PADDING_X * 2.0 + icon_width + galley.size().x,
        PADDING_Y * 2.0 + content_height,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let emphasized = selected || response.hovered();
    let color = if emphasized {
        text_dark()
    } else {
        text_dark().gamma_multiply(0.75)
    };

    if response.hovered() {
        let hover = ui.visuals().widgets.hovered.bg_fill;
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.hovered.corner_radius,
            Color32::from_rgba_unmultiplied(hover.r(), hover.g(), hover.b(), 72),
        );
    }
    if selected {
        let stroke = Stroke::new(2.0_f32, ui.visuals().selection.stroke.color);
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - stroke.width / 2.0, stroke);
    }

    let content_width = icon_width + galley.size().x;
    let content_rect = egui::Align2::CENTER_CENTER
        .align_size_within_rect(Vec2::new(content_width, content_height), rect);
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(
            content_rect.left(),
            content_rect.center().y - BUTTON_ICON_SIZE / 2.0,
        ),
        Vec2::splat(BUTTON_ICON_SIZE),
    );
    if let Some(icon) = icon {
        paint_button_icon_at(ui, icon, icon_rect, color);
    }
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(content_rect.left() + icon_width, content_rect.top()),
        content_rect.right_bottom(),
    );
    let text_pos = egui::Align2::LEFT_CENTER
        .align_size_within_rect(galley.size(), text_rect)
        .min;
    ui.painter().galley(text_pos, galley, color);
    response
}

fn draw_tag_fields_scroll(
    ui: &mut Ui,
    tag: &TagFile,
    document_revision: (u64, u64, u64, u64),
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    rmop_cache: &mut HashMap<String, Option<Arc<RenderMethodOption>>>,
    h2_templates: &mut H2TemplateCache,
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
                    document_revision,
                    entry,
                    names,
                    source,
                    rmdf_cache,
                    rmop_cache,
                    h2_templates,
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
mod extracted_tests;
pub(in crate::app) mod fields;
pub(in crate::app) use fields::*;
pub(in crate::app) mod shader;
pub(in crate::app) use shader::*;
pub(in crate::app) mod material;
pub(in crate::app) use material::*;
pub(in crate::app) mod function_editor;
pub(in crate::app) use function_editor::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;
pub(in crate::app) mod pane;
pub(in crate::app) mod tsv_paste_window;

/// The tag editor's windows and requests: the colour and function popups, the
/// reference picker, TSV paste, block confirmation and clipboard, a deferred
/// file action and a Campaign Evolved sound reference.
pub(in crate::app) struct EditorFeature {
    pub(in crate::app) color_popup: Option<MaterialColorPopup>,
    /// Kits owning the editing popups below. Each outlives the frame that
    /// opened it and applies an edit addressed by tag key — and a tag key is
    /// only unique within a kit, so applying against whichever kit happens to
    /// be active when the user confirms could edit another game's document, or
    /// silently drop the edit when no such key exists there.
    pub(in crate::app) color_popup_kit: Option<KitId>,
    pub(in crate::app) function_popup_kit: Option<KitId>,
    pub(in crate::app) tag_reference_picker_kit: Option<KitId>,
    /// Function editor snapshot and write targets captured when the popup opens.
    pub(in crate::app) function_popup: Option<FunctionPopup>,
    pub(in crate::app) tsv_paste: Option<TsvPasteState>,
    /// Pending destructive block op (delete / delete all) awaiting confirm.
    pub(in crate::app) block_confirm: Option<BlockConfirm>,
    /// Clipboard for copy/paste of a block element between identical tags.
    pub(in crate::app) block_clipboard: Option<BlockClipboard>,
    /// Movable Campaign Evolved tag-reference picker, when one is open.
    pub(in crate::app) tag_reference_picker: Option<TagReferencePickerState>,
    /// Pending play/extract of a `.sound` a container-source tag only refers to,
    /// stamped with the kit that raised it. Resolved after rendering, since the
    /// referenced tag's audio has to be walked out to Wwise first.
    /// A referenced sound to resolve, with the kit whose containers resolve it
    /// and the tab whose player asked (which owns the playback).
    pub(in crate::app) pending_ce_sound_ref: Option<(KitId, String, CeSoundRefRequest)>,
    /// One cross-frame edit popup at a time; its embedded tag/path identity
    /// prevents applying a delayed confirmation to the newly selected tag.
    /// File-menu actions run after the editor has rendered, so an edit being
    /// committed by focus loss is applied before its save/export snapshot.
    pub(in crate::app) deferred_file_action: Option<DeferredFileAction>,
}

/// What the editor derives from this kit's documents and keeps between frames:
/// bitmap and model previews, render-method definitions and options with the
/// epoch that invalidates the shader grid, Halo 2 templates, and Campaign
/// Evolved sound bindings.
#[derive(Default)]
pub(in crate::app) struct EditorCaches {
    pub(in crate::app) bitmap_previews: HashMap<String, BitmapPreviewState>,
    pub(in crate::app) model_previews: HashMap<String, ModelPreviewState>,
    /// Source-local render-method definition cache; `None` is a cached miss.
    pub(in crate::app) rmdf_cache: HashMap<String, Option<Arc<RenderMethodDefinition>>>,
    /// Source-local render-method option cache; `None` is a cached miss.
    pub(in crate::app) rmop_cache: HashMap<String, Option<Arc<RenderMethodOption>>>,
    /// Moves on whenever `rmdf_cache` and `rmop_cache` are cleared, so the
    /// shader grid, which memoises its model per document revision, rebuilds
    /// from the definitions as they are now.
    pub(in crate::app) render_method_epoch: u64,
    pub(in crate::app) h2_templates: H2TemplateCache,
    /// Campaign Evolved Wwise bindings, cached per tag key because resolving
    /// one walks several packages.
    pub(in crate::app) ce_sound_bindings: HashMap<String, Arc<crate::core::source::ce_audio::CeSoundBinding>>,
}
