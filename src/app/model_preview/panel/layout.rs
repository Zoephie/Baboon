//! The panel entry: the preview card and the setup card, side by side or stacked, and their headers.

use super::super::super::ui_kit::style::subtle_dark;
use super::super::super::{GameId, TagEntry, TagNameIndex};
use super::super::{
    FieldEditContext, ModelPreviewData, ModelPreviewState, is_previewable_geometry_group_for_game,
};
use super::animation::draw_animation_player;
use super::section::{
    MODEL_PREVIEW_SECTION_GAP, MODEL_PREVIEW_STATS_FOOTER_HEIGHT, ModelPreviewSectionPart,
    WIDE_MODEL_SETUP_MIN_WIDTH, draw_model_preview_loading_shells, draw_model_preview_section,
    wide_model_preview_section_width,
};
use super::setup::{
    draw_model_setup_header_controls, draw_setup_body, draw_setup_header_actions,
    model_setup_extra_header_height,
};
use super::view_menu::draw_model_view_settings_menu;
use super::viewport_size::{draw_model_viewport_with_stats, model_viewport_size};
use blam_tags::TagFile;
use eframe::egui::{Color32, RichText, Ui, Vec2};

// The material resolver understands the render-method shaders validated for
// H3EK, H3ODSTEK and HREK, Halo 2's template-and-parameters `shader`, and Halo CE's
// model and environment shaders. Other kits may share a tag container
// generation, but that alone does not make their texture path supported.
pub(in crate::app::model_preview) fn model_preview_supports_textures(game: Option<GameId>) -> bool {
    matches!(
        game,
        Some(
            GameId::Halo3
                | GameId::Halo3Odst
                | GameId::HaloReach
                | GameId::Halo4
                | GameId::Halo2
                | GameId::HaloCe
        )
    )
}

pub(in crate::app) fn draw_model_preview_panel(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source_game: Option<GameId>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
) {
    let is_model = is_previewable_geometry_group_for_game(entry.group_tag, names, source_game);
    if !is_model {
        return;
    }

    let supports_textures = model_preview_supports_textures(source_game);
    if !supports_textures {
        state.render_mode = state.render_mode.without_textures();
    }

    ui.scope(|ui| {
        let needs_load = state.needs_preview_load(&entry.key);
        if needs_load {
            draw_model_preview_loading_shells(ui, *model_preview_size);
            return;
        }

        let is_campaign_evolved = is_campaign_evolved_model(tag);

        let Some(data_result) = state.data.take() else {
            ui.label(RichText::new("No preview loaded").color(subtle_dark()));
            return;
        };
        let mut restore_data = Some(data_result);
        let data = match restore_data.as_ref().expect("preview data just set") {
            Ok(data) => data,
            Err(error) => {
                ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                state.data = restore_data.take();
                return;
            }
        };
        let panel = PreviewPanel {
            tag,
            entry,
            source_game,
            supports_textures,
            is_campaign_evolved,
            data,
        };

        let mut requests = SetupRequests::default();
        let page_width = ui.available_width();
        let can_place_controls_beside = page_width >= 780.0;
        if can_place_controls_beside {
            draw_preview_beside_setup(ui, &panel, state, model_preview_size, edit, &mut requests);
        } else {
            draw_preview_above_setup(ui, &panel, state, model_preview_size, edit, &mut requests);
        }
        draw_animation_player(ui, &panel, state);
        if requests.mutation {
            state.invalidate_load();
        } else if !requests.reload {
            state.data = restore_data.take();
        }
    });
    ui.add_space(8.0);
}

/// What every section of a loaded preview's panel draws from.
pub(in crate::app::model_preview) struct PreviewPanel<'a> {
    pub(in crate::app::model_preview) tag: &'a TagFile,
    pub(in crate::app::model_preview) entry: &'a TagEntry,
    pub(in crate::app::model_preview) source_game: Option<GameId>,
    pub(in crate::app::model_preview) supports_textures: bool,
    pub(in crate::app::model_preview) is_campaign_evolved: bool,
    pub(in crate::app::model_preview) data: &'a ModelPreviewData,
}

/// What the Model Setup header's actions asked for this pass.
#[derive(Default)]
pub(in crate::app::model_preview) struct SetupRequests {
    /// A variant was written, so the preview loads again.
    pub(in crate::app::model_preview) mutation: bool,
    /// Refresh Model was pressed.
    pub(in crate::app::model_preview) reload: bool,
}

/// A Campaign Evolved `.model`: one that names a skeleton model.
pub(in crate::app::model_preview) fn is_campaign_evolved_model(tag: &TagFile) -> bool {
    tag.header.group_tag.to_be_bytes() == *b"hlmt"
        && tag
            .root()
            .read_tag_ref_with_group("skeleton model")
            .map(|(_, reference)| !reference.trim().is_empty())
            .unwrap_or(false)
}

/// The wide layout: the Model Preview card and the Model Setup card side by
/// side, their outer heights matched.
pub(in crate::app::model_preview) fn draw_preview_beside_setup(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
    requests: &mut SetupRequests,
) {
    let page_width = ui.available_width();
    let gap = MODEL_PREVIEW_SECTION_GAP;
    // At wide sizes the persisted preview scale controls the entire
    // preview card. Reserve enough room for Model Setup, then let the
    // card grow around its 470×300 viewport until it reaches that
    // limit.
    let preview_width = wide_model_preview_section_width(page_width, *model_preview_size);
    let setup_width = (page_width - preview_width - gap).max(WIDE_MODEL_SETUP_MIN_WIDTH);
    let preview_viewport_size = model_viewport_size(preview_width, *model_preview_size);
    let shared_body_height = preview_viewport_size.y + MODEL_PREVIEW_STATS_FOOTER_HEIGHT;
    // Setup's body has 8-point top/bottom margins; Preview's body is
    // edge-to-edge. Match their *outer* card heights, not just content.
    let setup_body_height =
        (shared_body_height - model_setup_extra_header_height(setup_width) - 16.0).max(1.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui(Vec2::new(preview_width, 0.0), |ui| {
            ui.set_min_width(preview_width);
            ui.set_max_width(preview_width);
            draw_model_preview_section(
                ui,
                "Model Preview",
                Some(shared_body_height),
                |ui, part| match part {
                    ModelPreviewSectionPart::Header => {
                        draw_preview_header(ui, panel, state, model_preview_size)
                    }
                    ModelPreviewSectionPart::Body => {
                        draw_model_viewport_with_stats(
                            ui,
                            panel.data,
                            state,
                            preview_viewport_size,
                        );
                    }
                },
            );
        });
        ui.allocate_ui(Vec2::new(setup_width, 0.0), |ui| {
            ui.set_min_width(setup_width);
            ui.set_max_width(setup_width);
            draw_model_preview_section(ui, "Model Setup", Some(setup_body_height), |ui, part| {
                if part == ModelPreviewSectionPart::Header {
                    draw_model_setup_header_controls(
                        ui,
                        setup_width,
                        panel.data,
                        state,
                        |ui, state| draw_setup_header_actions(ui, panel, state, edit, requests),
                    );
                } else {
                    crate::app::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(setup_body_height)
                        .show(ui, |ui| {
                            draw_setup_body(ui, panel.data, state);
                        });
                }
            });
        });
    });
}

/// The narrow layout: the Model Preview card, then the Model Setup card
/// under it.
pub(in crate::app::model_preview) fn draw_preview_above_setup(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
    requests: &mut SetupRequests,
) {
    draw_model_preview_section(ui, "Model Preview", None, |ui, part| match part {
        ModelPreviewSectionPart::Header => {
            draw_preview_header(ui, panel, state, model_preview_size)
        }
        ModelPreviewSectionPart::Body => {
            let size = model_viewport_size(ui.available_width(), *model_preview_size);
            draw_model_viewport_with_stats(ui, panel.data, state, size);
        }
    });
    ui.add_space(8.0);
    let setup_width = ui.available_width();
    draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
        if part == ModelPreviewSectionPart::Header {
            draw_model_setup_header_controls(ui, setup_width, panel.data, state, |ui, state| {
                draw_setup_header_actions(ui, panel, state, edit, requests)
            });
        } else {
            draw_setup_body(ui, panel.data, state);
        }
    });
}

/// The Model Preview card's header: the view settings menu.
pub(in crate::app::model_preview) fn draw_preview_header(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
) {
    draw_model_view_settings_menu(
        ui,
        panel.tag,
        panel.data,
        state,
        model_preview_size,
        panel.supports_textures,
        panel.is_campaign_evolved,
        panel.source_game,
        !panel.data.preview.nodes.is_empty(),
    )
}

/// What the preview panel and its tab call themselves, by tag group. Only
/// `.model`-family tags actually show a *render model*; the derived previews
/// name what they draw.
pub(in crate::app) fn preview_panel_title(group_tag: u32) -> &'static str {
    match &group_tag.to_be_bytes() {
        b"coll" => "Collision Model",
        b"phmo" => "Physics Model",
        b"sbsp" => "Structure BSP",
        b"scnr" => "Scenario Geometry",
        _ => "Model Preview",
    }
}
