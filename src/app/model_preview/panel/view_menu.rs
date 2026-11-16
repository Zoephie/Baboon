//! The preview header's controls: its model and armature toggles, and the
//! View, Shading and Camera menus.

use super::super::super::GameId;
use super::super::super::ui_kit::button_icons::{
    ButtonIcon, fixed_width_icon_text_menu_button, icon_dropdown_button, icon_text_dropdown_button,
    paint_button_icon_at, paint_checkbox_row_hover, selectable_text_button,
};
use super::super::super::ui_kit::checkbox::ThemedCheckbox;
use super::super::super::ui_kit::style::{
    BUTTON_HEIGHT, foundation_input_edge, menu_control_margin, placeholder_text, subtle_dark,
    text_dark,
};
use super::super::super::ui_kit::tag_icons::{
    paint_tag_icon_at, tag_icon, tag_icon_accent, tag_icon_image_in,
};
use super::super::load::{DetailLevels, OwnViewSettings};
use super::carry_through_reload;
use super::super::{
    BoundingSphere, MAX_PREVIEW_SCALE, ModelPreviewBackground, ModelPreviewData,
    ModelPreviewLayer, ModelPreviewState, ModelShading, is_object_family_group,
    min_preview_scale,
};
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use blam_tags::TagFile;
use eframe::egui;
use eframe::egui::{Frame, RichText, Sense, Stroke, Ui, Vec2};

/// The preview header's menus, right to left: View, then Shading.
pub(in crate::app::model_preview) fn draw_model_view_settings_menu(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    supports_textures: bool,
    own_view: Option<OwnViewSettings>,
) {
    draw_model_view_menu(ui, data, state, own_view);
    draw_model_shading_menu(ui, data, state, supports_textures);
}

const MENU_WIDTH: f32 = 280.0;

/// The Shading button's width, which fits its longest choice, Game Shaders,
/// so picking another never shifts the buttons beside it.
pub(in crate::app::model_preview) const SHADING_BUTTON_WIDTH: f32 = 140.0;

/// One of a menu's mutually exclusive choices: a full-width row, highlighted
/// while it is the one in effect.
fn menu_choice(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
        selectable_text_button(ui, label, selected)
    })
    .inner
}

/// The Shading menu: how the surfaces are drawn, then what is drawn with
/// them and how the game's shaders light them.
pub(in crate::app::model_preview) fn draw_model_shading_menu(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    supports_textures: bool,
) {
    let current = state.shading();
    fixed_width_icon_text_menu_button(
        ui,
        ButtonIcon::Shading,
        current.label(),
        SHADING_BUTTON_WIDTH,
        |ui| {
            // Fix both bounds: a menu's sizing pass can otherwise let its rows
            // grow wider than the menu.
            ui.set_width(MENU_WIDTH);
            // Shading is picked right here. It was a combo box, whose list is
            // a popup of its own; egui 0.36 keeps one popup open at a time, so
            // opening it closed this menu and nothing could be picked.
            for shading in ModelShading::ALL {
                if !supports_textures && shading.uses_textures() {
                    continue;
                }
                let response = menu_choice(ui, shading.label(), current == shading);
                let response = if shading == ModelShading::GameShaders {
                    response.on_hover_text(
                        "Draw Halo CE, Halo 2 and Halo 3 materials through the game's own \
                         shaders, lit by its default lighting, with the object's effects \
                         and functions. The other choices read none of those.",
                    )
                } else {
                    response
                };
                // What Game Shaders reads changes, so the preview reads again.
                if response.clicked() && state.set_shading(shading) {
                    super::weapons::carry_through_reload(data, state);
                }
            }
            ui.separator();

            let mut overlay = state.wireframe_overlay();
            if ui
                .add_enabled(
                    current != ModelShading::Wireframe,
                    ThemedCheckbox::new(&mut overlay, "Wireframe"),
                )
                .on_hover_text("Draw the wireframe over the shaded model.")
                .changed()
            {
                state.set_wireframe_overlay(overlay);
            }
            ui.baboon_checkbox(&mut state.show_backfaces, "Render Backfaces");

            const NEEDS_GAME_SHADERS: &str = "Needs Game Shaders shading.";
            let game_shaders = state.game_shaders;
            // What a game's models draw no differently for, its rules leave out.
            let lit_options = super::super::load::rules(data.preview.game).lit_options;
            if lit_options {
                ui.add_enabled(
                    game_shaders,
                    ThemedCheckbox::new(&mut state.default_lights, "Default Lights"),
                )
                .on_hover_text(
                    "Light Halo 2 models as the game does in a level with no lightmaps: \
                     fixed lighting plus two sphere lights. Off, they are lit as an \
                     object in a level with lightmaps.",
                )
                .on_disabled_hover_text(NEEDS_GAME_SHADERS);
                ui.add_enabled(game_shaders, ThemedCheckbox::new(&mut state.prt, "PRT"))
                    .on_hover_text(
                        "Use the occlusion Halo 2 and Halo 3 models bake into their vertices. \
                         Off, they light as though nothing shades them.",
                    )
                    .on_disabled_hover_text(NEEDS_GAME_SHADERS);
            }
            ui.add_enabled(
                game_shaders && (!lit_options || !state.default_lights),
                ThemedCheckbox::new(&mut state.light_follows_camera, "Light Follows Camera"),
            )
            .on_hover_text(
                "Keep the main light of Halo 2 and Halo 3 models beside the viewer as you \
                 orbit. This is the preview's own. Off, it stays fixed to the model.",
            )
            .on_disabled_hover_text(if game_shaders {
                "Default Lights places the lights itself."
            } else {
                NEEDS_GAME_SHADERS
            });
            ui.add_enabled(
                game_shaders,
                ThemedCheckbox::new(&mut state.active_camo, "Active Camo"),
            )
            .on_hover_text(
                "Draw a Halo 2 or Halo 3 model through its active camouflage, for \
                 the models that have it.",
            )
            .on_disabled_hover_text(NEEDS_GAME_SHADERS);
            if lit_options {
                ui.add_enabled(game_shaders, ThemedCheckbox::new(&mut state.bloom, "Bloom"))
                    .on_hover_text(
                        "Add the glow of Halo 2 and Halo 3 materials with a bloom pass. The \
                         blur is Halo 2's, and its strength is this preview's own: the \
                         game's follows the exposure of the whole scene.",
                    )
                    .on_disabled_hover_text(NEEDS_GAME_SHADERS);
                ui.add_enabled(
                    game_shaders,
                    ThemedCheckbox::new(&mut state.self_shadow, "Self Shadow"),
                )
                .on_hover_text(
                    "Shadow Halo 2 and Halo 3 models from their main light. Halo 3 uses the \
                     game's own shadow pass; Halo 2's is the preview's, as the game draws none here.",
                )
                .on_disabled_hover_text(NEEDS_GAME_SHADERS);
            }
        },
    );
}

/// The View menu: the game's own view settings, the grid, the background and
/// the camera.
pub(in crate::app::model_preview) fn draw_model_view_menu(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    own_view: Option<OwnViewSettings>,
) {
    preview_header_menu(ui, ButtonIcon::View, "View", |ui| {
        ui.set_width(MENU_WIDTH);
        if let Some(own) = own_view {
            (own.draw)(ui, state);
            ui.separator();
        }
        ui.baboon_checkbox(&mut state.show_grid, "Show Grid")
            .on_hover_text("Ground-reference grid on the z = 0 plane, spaced to the model's size.");

        ui.separator();
        ui.label(RichText::new("Background Color").color(subtle_dark()).small());
        for background in ModelPreviewBackground::ALL {
            let response = menu_choice(ui, background.label(), state.background == background);
            let response = if background == ModelPreviewBackground::Theme {
                response.on_hover_text("The page's own background, dark or light with the theme.")
            } else {
                response
            };
            if response.clicked() {
                state.background = background;
            }
        }

        ui.separator();
        draw_camera_settings(ui, data, state);
    });
}

/// An object's bounding sphere: its `bounding radius` about its `bounding
/// offset`. `None` for a tag that is no object, or one whose radius is not
/// above 0.
pub(in crate::app::model_preview) fn object_bounding_sphere(
    tag: &TagFile,
    game: Option<GameId>,
) -> Option<BoundingSphere> {
    if !is_object_family_group(tag.header.group_tag, game) {
        return None;
    }
    let holder = super::super::common::fields::object_block(tag, "bounding radius")?;
    let radius = holder.read_real("bounding radius")?;
    let offset = holder.read_point3d("bounding offset");
    (radius > 0.0).then_some(BoundingSphere {
        center: [offset.x, offset.y, offset.z],
        radius,
    })
}

/// The Other menu, beside the header's model toggles: the markers, with
/// their filter, the error reports and, for an object, its bounding sphere.
pub(in crate::app::model_preview) fn draw_model_other_menu(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    icon_dropdown_button(ui, ButtonIcon::Other, "Other: markers, errors and level of detail", |ui| {
        // Fix both bounds: a menu's sizing pass can otherwise let the
        // full-width marker filter grow wider than the rest.
        ui.set_width(MENU_WIDTH);
        model_view_icon_checkbox(
            ui,
            &mut state.show_markers,
            ModelViewCheckboxIcon::Markers,
            "Show Markers",
        );
        draw_marker_filter_field(ui, &mut state.marker_filter);

        let error_count = data.preview.errors.len();
        ui.add_enabled_ui(error_count > 0, |ui| {
            model_view_icon_checkbox(
                ui,
                &mut state.show_errors,
                ModelViewCheckboxIcon::Errors,
                "Show Errors",
            )
            .on_hover_text(format!(
                "Highlight {error_count} error/warning report primitive(s); hover one to see its report."
            ));
            if state.show_errors {
                ui.indent("model_error_filters", |ui| {
                    ui.baboon_checkbox(&mut state.show_non_critical_errors, "Show Non-Critical")
                        .on_hover_text("Include error and warning reports marked non-critical.");
                });
            }
        });

        if state.bounding_sphere.is_some() {
            ui.separator();
            ui.baboon_checkbox(&mut state.show_bounding_sphere, "Show Bounding Sphere")
                .on_hover_text(
                    "Outline the object's bounding radius about its bounding offset.",
                );
        }

        if let Some(levels) = &data.preview.detail_levels {
            ui.separator();
            draw_detail_levels(ui, data, state, levels);
        }
    });
}

/// Halo CE's and Halo 2's levels of detail as choices, the highest first and
/// the one drawn picked.
/// A level no permutation has is greyed out. Picking one reloads the model,
/// keeping the regions picked.
fn draw_detail_levels(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    levels: &DetailLevels,
) {
    ui.label(RichText::new("Level of Detail").color(subtle_dark()).small());
    for (level, label) in levels.labels.iter().enumerate().rev() {
        let available = levels.available.get(level).copied().unwrap_or(false);
        let response = ui
            .add_enabled_ui(available, |ui| menu_choice(ui, label, levels.shown == level))
            .inner
            .on_disabled_hover_text("No permutation of this model has geometry at this level.");
        if response.clicked() && levels.shown != level {
            carry_through_reload(data, state);
            state.detail_level = Some(level);
        }
    }
}

/// The View menu's camera settings: zoom, projection and a reset.
fn draw_camera_settings(ui: &mut Ui, data: &ModelPreviewData, state: &mut ModelPreviewState) {
    let least = min_preview_scale(&data.preview);
    ui.label(RichText::new("Camera").color(subtle_dark()).small());
    ui.horizontal(|ui| {
        ui.label("Zoom");
        ui.add(
            egui::Slider::new(&mut state.scale, least..=MAX_PREVIEW_SCALE)
                .logarithmic(true)
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        let drag_speed = (state.scale * 0.05) as f64;
        ui.add(
            egui::DragValue::new(&mut state.scale)
                .range(least..=MAX_PREVIEW_SCALE)
                .speed(drag_speed)
                .max_decimals(5)
                .suffix("×"),
        );
    });
    ui.baboon_checkbox(&mut state.perspective, "Perspective Projection")
        .on_hover_text(
            "Perspective projection with a 60° lens; zooming moves the camera closer. Clear \
             it for the flat orthographic view. The framing at the orbit point stays \
             identical, so toggling never jumps.",
        );
    if ui.button("Reset View").clicked() {
        state.yaw = -0.45;
        state.pitch = 0.25;
        state.focus = [0.0; 3];
        state.scale = 1.0;
    }
}

/// The preview header's icon toggles, left to right: the render, collision
/// and physics models and the armature, each where the tag has one. Returns
/// whether any was drawn, so the header shows its title instead when none is.
pub(in crate::app::model_preview) fn draw_model_layer_toggles(
    ui: &mut Ui,
    tag: &TagFile,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    own_view: Option<OwnViewSettings>,
    source_game: Option<GameId>,
    has_armature: bool,
) -> bool {
    let group = tag.header.group_tag.to_be_bytes();
    let overlays = !own_view.is_some_and(|own| own.hides_overlays)
        && (group == *b"hlmt" || is_object_family_group(tag.header.group_tag, source_game));
    if !overlays && !has_armature {
        return false;
    }
    ui.spacing_mut().item_spacing.x = 4.0;
    let has_collision = overlay_layer_available(data, state, ModelPreviewLayer::Collision);
    let has_physics = overlay_layer_available(data, state, ModelPreviewLayer::Physics);
    // Each toggle drawn, whether it can be used, and what a click asked.
    let mut drawn = Vec::new();
    let mut toggle = |ui: &mut Ui, layer: ModelLayer, enabled: bool, disabled_tip: Option<&str>| {
        let on = layer.shown(state);
        let (response, action) = layer_toggle(ui, on, layer, source_game, enabled);
        if let Some(tip) = disabled_tip {
            response.on_disabled_hover_text(format!("{}\n\n{tip}", layer.label()));
        }
        drawn.push((layer, enabled, action));
    };
    toggle(ui, ModelLayer::Render, true, None);
    if overlays {
        toggle(
            ui,
            ModelLayer::Collision,
            has_collision,
            Some("This model has no collision model, or it has no drawable geometry."),
        );
        // An object's physics model comes through the `.model` it names, as
        // its collision model does; Halo CE has no physics models.
        if source_game != Some(GameId::HaloCe) {
            toggle(
                ui,
                ModelLayer::Physics,
                has_physics,
                Some("This model has no physics model, or it has no drawable geometry."),
            );
        }
    }
    if let Some(lod_model) = &data.preview.lod_model {
        toggle(ui, ModelLayer::LodModel, lod_model.missing.is_none(), lod_model.missing.as_deref());
    }
    if let Some(imposter) = &data.preview.imposter {
        toggle(ui, ModelLayer::Imposter, imposter.missing.is_none(), imposter.missing.as_deref());
    }
    if has_armature {
        toggle(ui, ModelLayer::Armature, true, None);
    }
    apply_layer_actions(state, &drawn);
    if state.overlays_pending && (state.show_collision || state.show_physics) {
        ui.spinner();
    }
    true
}

/// One of the header's model toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::model_preview) enum ModelLayer {
    Render,
    Collision,
    Physics,
    /// Halo 3's and ODST's LOD render model, where the model is.
    LodModel,
    /// Reach's and Halo 2 Anniversary's imposter, where the model is.
    Imposter,
    Armature,
}

impl ModelLayer {
    pub(in crate::app::model_preview) fn label(self) -> &'static str {
        match self {
            Self::Render => "Render Model",
            Self::Collision => "Collision Model",
            Self::Physics => "Physics Model",
            Self::LodModel => "LOD Model",
            Self::Imposter => "Imposter",
            Self::Armature => "Armature",
        }
    }

    /// The tag group whose icon the toggle shows.
    fn group(self) -> [u8; 4] {
        match self {
            Self::Render => *b"mode",
            Self::Collision => *b"coll",
            Self::Physics => *b"phmo",
            Self::LodModel => *b"mode",
            Self::Imposter => *b"impo",
            Self::Armature => *b"jmad",
        }
    }

    fn flag(self, state: &mut ModelPreviewState) -> &mut bool {
        match self {
            Self::Render => &mut state.show_render,
            Self::Collision => &mut state.show_collision,
            Self::Physics => &mut state.show_physics,
            Self::LodModel => &mut state.show_lod_model,
            Self::Imposter => &mut state.show_imposter,
            Self::Armature => &mut state.show_armature,
        }
    }

    fn shown(self, state: &mut ModelPreviewState) -> bool {
        *self.flag(state)
    }
}

/// What a click on a model toggle asked for, by its modifier, as the bitmap
/// preview's channel buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::model_preview) enum LayerAction {
    None,
    /// A plain click: flip it.
    Toggle,
    /// Alt-click: show only this one.
    Solo,
    /// Shift-click: hide only this one.
    Exclude,
}

impl LayerAction {
    fn of_click(modifiers: egui::Modifiers) -> Self {
        if modifiers.alt {
            Self::Solo
        } else if modifiers.shift {
            Self::Exclude
        } else {
            Self::Toggle
        }
    }
}

/// Apply this pass's clicks to the toggles `drawn` (each with whether it
/// could be used): Alt and Shift set every usable toggle drawn, the clicked
/// one one way and the rest the other.
pub(in crate::app::model_preview) fn apply_layer_actions(
    state: &mut ModelPreviewState,
    drawn: &[(ModelLayer, bool, LayerAction)],
) {
    for &(clicked, _, action) in drawn {
        let others = match action {
            LayerAction::None => continue,
            LayerAction::Toggle => {
                let flag = clicked.flag(state);
                *flag = !*flag;
                continue;
            }
            LayerAction::Solo => false,
            LayerAction::Exclude => true,
        };
        for &(layer, enabled, _) in drawn {
            if enabled {
                *layer.flag(state) = if layer == clicked { !others } else { others };
            }
        }
    }
}

/// An icon-only toggle showing `layer`'s tag icon: an ordinary button while
/// off, filled with the icon's own colour and the icon drawn white while on.
fn layer_toggle(
    ui: &mut Ui,
    on: bool,
    layer: ModelLayer,
    game: Option<GameId>,
    enabled: bool,
) -> (egui::Response, LayerAction) {
    let group = layer.group();
    let icon = tag_icon(Some(u32::from_be_bytes(group)), game);
    let accent = tag_icon_accent(icon).filter(|_| on && enabled);
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            if let Some(accent) = accent {
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = accent;
                widgets.inactive.bg_stroke = Stroke::NONE;
                widgets.hovered.weak_bg_fill = accent.gamma_multiply(0.85);
                widgets.hovered.bg_stroke = Stroke::NONE;
                widgets.active.weak_bg_fill = accent.gamma_multiply(0.7);
                widgets.active.bg_stroke = Stroke::NONE;
            }
            let response = ui.add(egui::Button::new("").min_size(Vec2::splat(BUTTON_HEIGHT)));
            let icon_rect =
                egui::Rect::from_center_size(response.rect.center(), Vec2::splat(16.0));
            match accent {
                Some(_) => tag_icon_image_in(ui.ctx(), icon, 16.0, egui::Color32::WHITE)
                    .fit_to_exact_size(icon_rect.size())
                    .paint_at(ui, icon_rect),
                None => paint_tag_icon_at(ui, Some(u32::from_be_bytes(group)), game, icon_rect),
            }
            response
        })
        .inner;
    let action = if response.clicked() {
        LayerAction::of_click(ui.input(|input| input.modifiers))
    } else {
        LayerAction::None
    };
    let shown = if on { "shown" } else { "hidden" };
    let extra = match layer {
        ModelLayer::Armature => "\nHover a joint to see its name.",
        ModelLayer::LodModel => "\nThe simpler model the game draws at a distance.",
        ModelLayer::Imposter => {
            "\nThe untextured stand-in the game draws far away, coloured as baked."
        }
        _ => "",
    };
    let tip = format!(
        "{} ({shown})\nAlt-click: show only this one\nShift-click: hide only this one{extra}",
        layer.label()
    );
    (response.on_hover_text(tip), action)
}

#[derive(Clone, Copy)]
pub(in crate::app::model_preview) enum ModelViewCheckboxIcon {
    Markers,
    Errors,
}

/// Whether an overlay toggle has anything to show. Until the overlay worker
/// reports back the answer is unknown, so the toggle stays enabled; after
/// it, a layer with no batches means the model names no such tag, or the
/// reference did not resolve to anything drawable.
pub(in crate::app::model_preview) fn overlay_layer_available(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    layer: ModelPreviewLayer,
) -> bool {
    !state.overlays_loaded
        || data
            .preview
            .batches
            .iter()
            .any(|batch| batch.layer == layer)
}

pub(in crate::app::model_preview) fn model_view_icon_checkbox(
    ui: &mut Ui,
    checked: &mut bool,
    icon: ModelViewCheckboxIcon,
    label: &str,
) -> egui::Response {
    let row = Frame::NONE
        .inner_margin(menu_control_margin(ui))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let checkbox_response = ui.baboon_checkbox(checked, "");
                // Native checkbox text starts before the end of an empty checkbox's
                // 24-point hitbox. Put the icon at that same text start, without
                // overlapping the two clickable hitboxes.
                let icon_left = checkbox_response.rect.left()
                    + ui.spacing().icon_width
                    + ui.spacing().icon_spacing;
                let interaction_left = icon_left.max(checkbox_response.rect.right());
                ui.spacing_mut().item_spacing.x = interaction_left - checkbox_response.rect.right();
                let interaction_width = (icon_left + 16.0 - interaction_left).max(1.0);
                let (icon_hit_rect, icon_response) = ui.allocate_exact_size(
                    Vec2::new(interaction_width, BUTTON_HEIGHT),
                    Sense::click(),
                );
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(icon_left, icon_hit_rect.center().y - 8.0),
                    Vec2::splat(16.0),
                );
                match icon {
                    ModelViewCheckboxIcon::Markers => {
                        paint_button_icon_at(ui, ButtonIcon::Markers, icon_rect, text_dark());
                    }
                    ModelViewCheckboxIcon::Errors => {
                        paint_button_icon_at(ui, ButtonIcon::Errors, icon_rect, text_dark());
                    }
                }
                ui.spacing_mut().item_spacing.x = 4.0;
                let label_response = ui.add(
                    egui::Label::new(label)
                        .selectable(false)
                        .sense(Sense::click()),
                );
                if icon_response.clicked() || label_response.clicked() {
                    *checked = !*checked;
                }
                checkbox_response
            })
        })
        .inner;
    if ui.rect_contains_pointer(row.response.rect) && !row.inner.hovered() {
        paint_checkbox_row_hover(ui, row.inner.rect, *checked);
    }
    row.response
}

pub(in crate::app::model_preview) fn draw_marker_filter_field(
    ui: &mut Ui,
    filter: &mut String,
) -> egui::Response {
    const HEIGHT: f32 = 24.0;
    const ICON_SIZE: f32 = 16.0;
    let width = ui.available_width().max(HEIGHT);
    let (rect, background_response) =
        ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    let rounding = ui.visuals().widgets.inactive.corner_radius;
    ui.painter()
        .rect_filled(rect, rounding, ui.visuals().extreme_bg_color);

    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 4.0, rect.center().y - ICON_SIZE * 0.5),
        Vec2::splat(ICON_SIZE),
    );
    paint_button_icon_at(ui, ButtonIcon::Filter, icon_rect, text_dark());
    let icon_response = ui.interact(
        icon_rect,
        background_response.id.with("marker_filter_icon"),
        Sense::click(),
    );
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(icon_rect.right() + 8.0, rect.top()),
        egui::pos2(rect.right() - 8.0, rect.bottom()),
    );
    let edit_response = ui.put(
        edit_rect,
        crate::app::ui_kit::singleline_text_edit(filter)
            .hint_text(placeholder_text("Filter Markers…"))
            .text_color(text_dark())
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::same(0))
            .vertical_align(egui::Align::Center)
            .min_size(edit_rect.size()),
    );
    if icon_response.clicked() {
        edit_response.request_focus();
    }
    let response = background_response
        .union(icon_response)
        .union(edit_response.clone());
    let stroke = if edit_response.has_focus() {
        ui.visuals().selection.stroke
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        Stroke::new(1.0_f32, foundation_input_edge())
    };
    ui.painter()
        .rect_stroke(rect, rounding, stroke, egui::StrokeKind::Middle);
    response
}

pub(in crate::app::model_preview) fn preview_header_menu(
    ui: &mut Ui,
    icon: ButtonIcon,
    label: &str,
    add_contents: impl FnOnce(&mut Ui),
) {
    icon_text_dropdown_button(ui, icon, label, add_contents);
}
