//! The panel entry: the page the preview tab fills, as Blender lays out its
//! editors. The Animation Player is docked along the bottom at a fixed
//! height; above it the viewport takes every point the Model Setup sidebar
//! leaves, and the sidebar is as wide as its edge was last dragged.

use super::super::super::ui_kit::button_icons::{ButtonIcon, icon_text_button};
use super::super::super::ui_kit::dock::{
    SidebarLimits, SidebarSplit, draw_sidebar_resize_handle, split_sidebar,
};
use super::super::super::ui_kit::style::{foundation_group_edge, subtle_dark};
use super::super::super::{GameId, TagEntry, TagNameIndex};
use super::super::{
    FieldEditContext, MAX_MODEL_SETUP_WIDTH, MIN_MODEL_SETUP_WIDTH, ModelPreviewData,
    ModelPreviewState, PreviewAnimationPlayback, is_previewable_geometry_group_for_game,
};
use super::super::state::ReloadingPreview;
use super::animation::{animation_card_height, draw_animation_player};
use super::functions::draw_setup_body;
use super::section::{
    MODEL_PREVIEW_SECTION_GAP, MODEL_PREVIEW_STATS_FOOTER_HEIGHT, ModelPreviewSectionPart,
    MODEL_PREVIEW_TITLE, draw_model_preview_loading_shells, draw_model_preview_section,
    section_title,
};
use super::setup::draw_setup_header_actions;
use super::view_menu::{
    draw_model_layer_toggles, draw_model_other_menu, draw_model_view_settings_menu,
    object_bounding_sphere,
};
use super::viewport_size::draw_model_viewport_with_stats;
use blam_tags::TagFile;
use eframe::egui;
use eframe::egui::{Rect, RichText, Sense, Ui, Vec2};

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
    let rects = panel_rects(page, setup_width_for(state, *model_setup_width));

    let own_view = super::super::load::own_view_settings(tag);
    if state.needs_preview_load(&entry.key) {
        // The same model with new settings (Game Shaders, a weapon) keeps its
        // page: drawn from the model as it was, held aside once the load is
        // asked for, or still in place the pass before it is.
        let same_model = state.loaded_key.as_deref() == Some(entry.key.as_str());
        let shown = match (state.reloading.take(), state.data.take()) {
            (Some(reloading), data) => {
                state.data = data;
                Some((reloading, false))
            }
            (None, Some(Ok(data))) if same_model => {
                let animation = state.animation.display_copy();
                Some((ReloadingPreview { data, animation }, true))
            }
            (None, data) => {
                state.data = data;
                None
            }
        };
        let Some((shown, in_place)) = shown else {
            draw_model_preview_loading_shells(ui, &rects);
            return;
        };
        let panel = PreviewPanel {
            tag,
            entry,
            source_game,
            supports_textures,
            own_view,
            data: &shown.data,
            reloading: true,
        };
        let page_width = page.width();
        let refreshed = draw_reloading_page(
            ui,
            &rects,
            &panel,
            state,
            shown.animation.display_copy(),
            page_width,
            model_setup_width,
            edit,
        );
        // Refresh Model starts over from the loading shells.
        if !refreshed {
            if in_place {
                state.data = Some(Ok(shown.data));
            } else {
                state.reloading = Some(shown);
            }
        }
        return;
    }

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
                ui.colored_label(crate::app::ui_kit::panel_error_text(), error);
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
        reloading: false,
    };

    let mut requests = SetupRequests::default();
    draw_setup_resize_handle(ui, &rects, page.width(), model_setup_width);
    in_rect(ui, rects.preview, "model_preview_viewport_card", |ui| {
        draw_preview_card(ui, &panel, state, &mut requests, rects.preview.height());
    });
    // After a reload, the page before it stays beside the new viewport
    // until what follows the load has arrived.
    let now = ui.input(|input| input.time);
    let settling = state.settling.take().and_then(|mut settling| {
        settling.since.get_or_insert(now);
        (!state.page_settled(&settling, data, now)).then_some(settling)
    });
    if let Some(settling) = &settling {
        let held = PreviewPanel {
            data: &settling.page.data,
            ..panel
        };
        draw_still_page(ui, &rects, &held, state, settling.page.animation.display_copy(), edit);
        ui.ctx().request_repaint();
    } else {
        in_rect(ui, rects.setup, "model_preview_setup_cards", |ui| {
            draw_setup_cards(ui, &panel, state, edit, &mut requests);
        });
        in_rect(ui, rects.animation, "model_preview_animation_card", |ui| {
            draw_animation_player(ui, &panel, state);
        });
    }
    state.settling = settling;
    if requests.mutation {
        state.invalidate_load();
    } else if !requests.reload {
        state.data = restore_data.take();
        if let Some(Ok(data)) = state.data.as_mut() {
            super::super::decorators::relayout(data, state.decorator_layout);
        }
    }
}

/// The page as it was while the same model loads again with new settings
/// (Game Shaders, a weapon): the header works as ever, the viewport waits on
/// the load, and the sidebar and player show what they did, still, until the
/// new model arrives.
#[allow(clippy::too_many_arguments)]
fn draw_reloading_page(
    ui: &mut Ui,
    rects: &PanelRects,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    shown_animation: PreviewAnimationPlayback,
    page_width: f32,
    model_setup_width: &mut f32,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    // Header actions write nothing to the tag; a click on Refresh Model, or
    // a shading change, starts its own load next pass. Says whether Refresh
    // Model was clicked.
    let mut requests = SetupRequests::default();
    draw_setup_resize_handle(ui, rects, page_width, model_setup_width);
    in_rect(ui, rects.preview, "model_preview_viewport_card", |ui| {
        draw_preview_card(ui, panel, state, &mut requests, rects.preview.height());
    });
    if requests.reload {
        state.invalidate_load();
    }
    draw_still_page(ui, rects, panel, state, shown_animation, edit);
    requests.reload
}

/// The sidebar and player of `panel`, with its player as `shown_animation`,
/// drawn as they look but taking no clicks.
fn draw_still_page(
    ui: &mut Ui,
    rects: &PanelRects,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    shown_animation: PreviewAnimationPlayback,
    edit: &mut FieldEditContext<'_>,
) {
    // Shown, not worked: what is picked here would be lost to the load's
    // own carry, and the player's clip indexes the old model's list.
    let live = std::mem::replace(&mut state.animation, shown_animation);
    let mut discard = SetupRequests::default();
    still(ui, rects.setup, "model_preview_setup_cards", |ui| {
        draw_setup_cards(ui, panel, state, edit, &mut discard);
    });
    still(ui, rects.animation, "model_preview_animation_card", |ui| {
        draw_animation_player(ui, panel, state);
    });
    state.animation = live;
}

/// `add_contents` in `rect`, drawn as it looks but taking no clicks.
fn still(ui: &mut Ui, rect: Rect, id: &str, add_contents: impl FnOnce(&mut Ui)) {
    in_rect(ui, rect, id, |ui| {
        ui.visuals_mut().disabled_alpha = 1.0;
        ui.add_enabled_ui(false, add_contents);
    });
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

/// The sidebar's width: as it was left, but wide enough for a shader's grid
/// while the Materials card shows one, as the page allows. What was left is
/// kept, for when it shows none.
fn setup_width_for(state: &ModelPreviewState, setup_width: f32) -> f32 {
    let editing = state.material_editor.as_ref().is_some_and(|editor| editor.selected.is_some());
    if editing { setup_width.max(MATERIALS_SETUP_WIDTH) } else { setup_width }
}

/// The sidebar's width for a shader's grid: the grid's least, with the
/// sidebar's and the card's padding.
const MATERIALS_SETUP_WIDTH: f32 = crate::app::editor::SHADER_GRID_MIN_WIDTH + 4.0 * SETUP_PANEL_PADDING + 24.0;

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
    /// Drawn from the model as it was while another loads: the viewport
    /// waits. Said here, not asked of the state, whose data the page holds
    /// out while it draws.
    pub(in crate::app::model_preview) reloading: bool,
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
        MODEL_PREVIEW_TITLE,
        Some(body_height),
        |ui, part| match part {
            ModelPreviewSectionPart::Header => draw_preview_header(ui, panel, state, requests),
            ModelPreviewSectionPart::Body => {
                let viewport = Vec2::new(
                    ui.available_width().max(1.0),
                    (body_height - MODEL_PREVIEW_STATS_FOOTER_HEIGHT).max(1.0),
                );
                draw_model_viewport_with_stats(ui, panel.data, state, viewport, panel.reloading);
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
    // One bordered panel holds the cards, so they read as one sidebar, with
    // the same padding all round. The scroll area reaches the border at the
    // right, so its scroll bar, which floats and shows only while used,
    // rides in that padding rather than taking room of its own.
    let padding = SETUP_PANEL_PADDING as i8;
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, foundation_group_edge()))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin {
            left: padding,
            right: 0,
            top: padding,
            bottom: padding,
        })
        .show(ui, |ui| {
    crate::app::ScrollArea::vertical()
        .id_salt("model_setup_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    right: padding,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    // A card leaves 8 points above it for what came before;
                    // the first has nothing before it, and starts level with
                    // the viewport's card.
                    ui.add_space(-(8.0 - ui.spacing().item_spacing.y).max(0.0));
                    let materials = panel.supports_textures;
                    draw_setup_body(ui, panel.data, state, materials, &mut |ui, state| {
                        draw_setup_header_actions(ui, panel, state, edit, requests)
                    })
                });
        });
        });
}

/// The room between the setup panel's border and its cards.
pub(in crate::app) const SETUP_PANEL_PADDING: f32 = 8.0;

/// The Model Preview card's header. Right to left: the View and Shading
/// menus, then Refresh Model. At the left, the model and armature
/// toggles, or the card's title for a tag with none, then the Other menu.
pub(in crate::app::model_preview) fn draw_preview_header(
    ui: &mut Ui,
    panel: &PreviewPanel<'_>,
    state: &mut ModelPreviewState,
    requests: &mut SetupRequests,
) {
    // Right of the View menu, as the header lays out from the right.
    super::snapshot::draw_snapshot_controls(ui, state);
    draw_model_view_settings_menu(
        ui,
        panel.data,
        state,
        panel.supports_textures,
        panel.own_view,
    );
    // Right of Refresh Model, as the header lays out from the right.
    if icon_text_button(ui, ButtonIcon::Refresh, "Refresh Model", true)
        .on_hover_text("Load the model again from its tags.")
        .clicked()
    {
        state.invalidate_load();
        requests.reload = true;
    }
    state.bounding_sphere = object_bounding_sphere(panel.tag, panel.source_game);
    // What the menus leave, from the header's left edge.
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let toggles = draw_model_layer_toggles(
            ui,
            panel.tag,
            panel.data,
            state,
            panel.own_view,
            panel.source_game,
            !panel.data.preview.nodes.is_empty(),
        );
        if !toggles {
            section_title(ui, MODEL_PREVIEW_TITLE);
        }
        draw_model_other_menu(ui, panel.data, state);
    });
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
        b"dctr" | b"DECR" => "Decorator Set",
        b"dobc" => "Detail Objects",
        _ => "Model Preview",
    }
}
