//! The panel entry: the page the preview tab fills, as Blender lays out its
//! editors. The Animation Player is docked along the bottom at a fixed
//! height; above it the viewport takes every point the Model Setup sidebar
//! leaves, and the sidebar is as wide as its edge was last dragged.

use super::super::super::ui_kit::button_icons::{ButtonIcon, icon_text_button};
use super::super::super::ui_kit::dock::{
    SidebarLimits, SidebarSplit, draw_sidebar_resize_handle,
    sidebar_scroll_gutter, split_sidebar,
};
use super::super::super::ui_kit::style::subtle_dark;
use super::super::super::{GameId, TagEntry, TagNameIndex};
use super::super::{
    FieldEditContext, MAX_MODEL_SETUP_WIDTH, MIN_MODEL_SETUP_WIDTH, ModelPreviewData,
    ModelPreviewState, is_previewable_geometry_group_for_game,
};
use super::animation::{animation_card_height, draw_animation_player};
use super::functions::draw_setup_body;
use super::section::{
    MODEL_PREVIEW_SECTION_GAP, MODEL_PREVIEW_STATS_FOOTER_HEIGHT, ModelPreviewSectionPart,
    draw_model_preview_loading_shells, draw_model_preview_section,
};
use super::setup::draw_setup_header_actions;
use super::view_menu::draw_model_view_settings_menu;
use super::viewport_size::draw_model_viewport_with_stats;
use blam_tags::TagFile;
use eframe::egui;
use eframe::egui::{Color32, Rect, RichText, Sense, Ui, Vec2};

/// The narrowest the viewport is squeezed to by a wide sidebar.
pub(in crate::app::model_preview) const MIN_MODEL_VIEWPORT_WIDTH: f32 = 320.0;

/// The least height the panel takes, so a short pane scrolls to it rather
/// than crushing the viewport.
pub(in crate::app::model_preview) const MIN_MODEL_PREVIEW_PANEL_HEIGHT: f32 = 360.0;

/// Whether the preview resolves `game`'s materials to textures, by its
/// rules.
pub(in crate::app::model_preview) fn model_preview_supports_textures(game: Option<GameId>) -> bool {
    super::super::load::rules(game).textures
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_model_preview_panel(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source_game: Option<GameId>,
    state: &mut ModelPreviewState,
    model_setup_width: &mut f32,
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

    // The rest of the page, so the tab never scrolls: the same height the
    // Tag Fields tab's own scroll area fills.
    let size = Vec2::new(
        ui.available_width().max(1.0),
        ui.available_height().max(MIN_MODEL_PREVIEW_PANEL_HEIGHT),
    );
    let (page, _) = ui.allocate_exact_size(size, Sense::hover());
    let rects = panel_rects(page, *model_setup_width);

    if state.needs_preview_load(&entry.key) {
        draw_model_preview_loading_shells(ui, &rects);
        return;
    }

    let own_view = super::super::load::own_view_settings(tag);

    let Some(data_result) = state.data.take() else {
        in_rect(ui, page, "model_preview_empty", |ui| {
            ui.label(RichText::new("No preview loaded").color(subtle_dark()));
        });
        return;
    };
    let mut restore_data = Some(data_result);
    let data = match restore_data.as_ref().expect("preview data just set") {
        Ok(data) => data,
        Err(error) => {
            in_rect(ui, page, "model_preview_error", |ui| {
                ui.colored_label(Color32::from_rgb(150, 56, 44), error);
            });
            state.data = restore_data.take();
            return;
        }
    };
    let panel = PreviewPanel {
        tag,
        entry,
        source_game,
        supports_textures,
        own_view,
        data,
    };

    let mut requests = SetupRequests::default();
    draw_setup_resize_handle(ui, &rects, page.width(), model_setup_width);
    in_rect(ui, rects.preview, "model_preview_viewport_card", |ui| {
        draw_preview_card(ui, &panel, state, &mut requests, rects.preview.height());
    });
    in_rect(ui, rects.setup, "model_preview_setup_cards", |ui| {
        draw_setup_cards(ui, &panel, state, edit, &mut requests);
    });
    in_rect(ui, rects.animation, "model_preview_animation_card", |ui| {
        draw_animation_player(ui, &panel, state);
    });
    if requests.mutation {
        state.invalidate_load();
    } else if !requests.reload {
        state.data = restore_data.take();
    }
}

pub(in crate::app::model_preview) use crate::app::ui_kit::dock::in_rect;

/// Where each card of the panel goes in `page`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app::model_preview) struct PanelRects {
    /// The Model Preview card: the viewport and its stats footer.
    pub(in crate::app::model_preview) preview: Rect,
    /// The gap between the viewport and the sidebar, which drags the
    /// sidebar's width.
    pub(in crate::app::model_preview) handle: Rect,
    /// The Model Setup sidebar.
    pub(in crate::app::model_preview) setup: Rect,
    /// The Animation Player, along the bottom.
    pub(in crate::app::model_preview) animation: Rect,
}

/// The Model Setup sidebar's limits.
const SETUP_LIMITS: SidebarLimits = SidebarLimits {
    min: MIN_MODEL_SETUP_WIDTH,
    max: MAX_MODEL_SETUP_WIDTH,
    min_main: MIN_MODEL_VIEWPORT_WIDTH,
    gap: MODEL_PREVIEW_SECTION_GAP,
};

/// Splits `page` into the panel's cards for a sidebar `setup_width` wide.
pub(in crate::app::model_preview) fn panel_rects(page: Rect, setup_width: f32) -> PanelRects {
    let gap = MODEL_PREVIEW_SECTION_GAP;
    let animation_height = animation_card_height(page.width());
    let animation = Rect::from_min_max(
        egui::pos2(page.left(), page.bottom() - animation_height),
        page.max,
    );
    let top_bottom = (animation.top() - gap).max(page.top());
    let top = Rect::from_min_max(page.min, egui::pos2(page.right(), top_bottom));
    let split = split_sidebar(top, setup_width, SETUP_LIMITS);
    PanelRects {
        preview: split.main,
        handle: split.handle,
        setup: split.sidebar,
        animation,
    }
}

/// The sidebar's left edge: dragging it widens or narrows the sidebar, and
/// the viewport takes what is left.
fn draw_setup_resize_handle(
    ui: &mut Ui,
    rects: &PanelRects,
    panel_width: f32,
    setup_width: &mut f32,
) {
    let split = SidebarSplit {
        main: rects.preview,
        handle: rects.handle,
        sidebar: rects.setup,
    };
    draw_sidebar_resize_handle(ui, &split, panel_width, setup_width, SETUP_LIMITS, "model_setup_resize");
}

/// What every section of a loaded preview's panel draws from.
pub(in crate::app::model_preview) struct PreviewPanel<'a> {
    pub(in crate::app::model_preview) tag: &'a TagFile,
    pub(in crate::app::model_preview) entry: &'a TagEntry,
    pub(in crate::app::model_preview) source_game: Option<GameId>,
    pub(in crate::app::model_preview) supports_textures: bool,
    /// The view menu's own settings for the tag's model, if its game has any.
    pub(in crate::app::model_preview) own_view: Option<super::super::load::OwnViewSettings>,
    pub(in crate::app::model_preview) data: &'a ModelPreviewData,
}

/// What the panel's header actions asked for this pass.
#[derive(Default)]
pub(in crate::app::model_preview) struct SetupRequests {
    /// A variant was written, so the preview loads again.
    pub(in crate::app::model_preview) mutation: bool,
    /// Refresh Model was pressed.
    pub(in crate::app::model_preview) reload: bool,
}

/// The Model Preview card, `height` tall: the viewport over its stats
/// footer.
pub(in crate::app::model_preview) fn draw_preview_card(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    requests: &mut SetupRequests,
    height: f32,
) {
    let body_height = (height - PREVIEW_HEADER_HEIGHT).max(1.0);
    draw_model_preview_section(
        ui,
        "Model Preview",
        Some(body_height),
        |ui, part| match part {
            ModelPreviewSectionPart::Header => draw_preview_header(ui, panel, state, requests),
            ModelPreviewSectionPart::Body => {
                let viewport = Vec2::new(
                    ui.available_width().max(1.0),
                    (body_height - MODEL_PREVIEW_STATS_FOOTER_HEIGHT).max(1.0),
                );
                draw_model_viewport_with_stats(ui, panel.data, state, viewport);
            }
        },
    );
}

/// The Model Preview card's header height: one row.
pub(in crate::app::model_preview) const PREVIEW_HEADER_HEIGHT: f32 = 40.0;

/// The Model Setup sidebar: its cards, scrolling together, clear of the
/// scroll bar that floats over its right edge.
pub(in crate::app::model_preview) fn draw_setup_cards(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
    requests: &mut SetupRequests,
) {
    let gutter = setup_scroll_gutter(ui);
    crate::app::ScrollArea::vertical()
        .id_salt("model_setup_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    right: gutter as i8,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    // A card leaves 8 points above it for what came before;
                    // the first has nothing before it, and starts level with
                    // the viewport's card.
                    ui.add_space(-(8.0 - ui.spacing().item_spacing.y).max(0.0));
                    draw_setup_body(ui, panel.data, state, &mut |ui, state| {
                        draw_setup_header_actions(ui, panel, state, edit, requests)
                    })
                });
        });
}

/// The room the sidebar's cards leave at their right: the scroll bar at its
/// widest, and its margin from the cards.
pub(in crate::app::model_preview) fn setup_scroll_gutter(ui: &Ui) -> f32 {
    sidebar_scroll_gutter(ui)
}

/// The Model Preview card's header, right to left: the view settings menu,
/// then Refresh Model to the left of the Camera menu.
pub(in crate::app::model_preview) fn draw_preview_header(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    requests: &mut SetupRequests,
) {
    draw_model_view_settings_menu(
        ui,
        panel.tag,
        panel.data,
        state,
        panel.supports_textures,
        panel.own_view,
        panel.source_game,
        !panel.data.preview.nodes.is_empty(),
    );
    if icon_text_button(ui, ButtonIcon::Refresh, "Refresh Model", true)
        .on_hover_text("Load the model again from its tags.")
        .clicked()
    {
        state.invalidate_load();
        requests.reload = true;
    }
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
