//! The view menu: shading, layers, markers, errors, game shaders and the camera.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::app::model_preview) fn draw_model_view_settings_menu(
    ui: &mut Ui,
    tag: &TagFile,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    supports_textures: bool,
    is_campaign_evolved: bool,
    source_game: Option<GameId>,
    has_armature: bool,
) {
    preview_header_menu(
        ui,
        ButtonIcon::View,
        &format!("View: {}", state.render_mode.label()),
        |ui| {
            const VIEW_SETTINGS_WIDTH: f32 = 280.0;
            // Fix both bounds: a menu's sizing pass can otherwise let the
            // full-width marker filter grow wider than the shading choices.
            ui.set_width(VIEW_SETTINGS_WIDTH);
            // Shading is picked right here. It was a combo box, whose list is
            // a popup of its own; egui 0.36 keeps one popup open at a time, so
            // opening it closed this menu and nothing could be picked.
            ui.label(RichText::new("Shading").color(subtle_dark()).small());
            for mode in ModelRenderMode::ALL {
                if supports_textures || !mode.uses_textures() {
                    ui.baboon_radio_value(&mut state.render_mode, mode, mode.label());
                }
            }
            if is_campaign_evolved {
                ui.baboon_checkbox(&mut state.high_detail, "High Detail")
                    .on_hover_text(
                        "Decode full-resolution Nanite geometry instead of Unreal's coarse fallback.",
                    );
            }
            ui.separator();

            if !is_campaign_evolved
                && (tag.header.group_tag.to_be_bytes() == *b"hlmt"
                    || is_object_family_group(tag.header.group_tag, source_game))
            {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_render,
                    ModelViewCheckboxIcon::Tag(*b"mode"),
                    "Render Model",
                    source_game,
                );
                let has_collision =
                    overlay_layer_available(data, state, ModelPreviewLayer::Collision);
                ui.add_enabled_ui(has_collision, |ui| {
                    model_view_icon_checkbox(
                        ui,
                        &mut state.show_collision,
                        ModelViewCheckboxIcon::Tag(*b"coll"),
                        "Collision Model",
                        source_game,
                    )
                    .on_disabled_hover_text("This model has no collision model, or it has no drawable geometry.");
                });
                if tag.header.group_tag.to_be_bytes() == *b"hlmt" {
                    model_view_icon_checkbox(
                        ui,
                        &mut state.show_physics,
                        ModelViewCheckboxIcon::Tag(*b"phmo"),
                        "Physics Model",
                        source_game,
                    );
                }
                if state.overlays_pending && (state.show_collision || state.show_physics) {
                    ui.spinner();
                }
            }
            ui.add_enabled_ui(has_armature, |ui| {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_armature,
                    ModelViewCheckboxIcon::Tag(*b"jmad"),
                    "Armature",
                    source_game,
                )
                .on_hover_text("Draw the model skeleton; hover a joint to see its name.");
            });

            ui.separator();
            model_view_icon_checkbox(
                ui,
                &mut state.show_markers,
                ModelViewCheckboxIcon::Markers,
                "Show Markers",
                source_game,
            );
            draw_marker_filter_field(ui, &mut state.marker_filter);

            let error_count = data.preview.errors.len();
            ui.add_enabled_ui(error_count > 0, |ui| {
                model_view_icon_checkbox(
                    ui,
                    &mut state.show_errors,
                    ModelViewCheckboxIcon::Errors,
                    "Show Errors",
                    source_game,
                )
                .on_hover_text(format!(
                    "Highlight {error_count} error/warning report primitive(s); hover one to see its report."
                ));
                if state.show_errors {
                    ui.indent("model_error_filters", |ui| {
                        ui.baboon_checkbox(
                            &mut state.show_non_critical_errors,
                            "Show Non-Critical",
                        )
                        .on_hover_text("Include error and warning reports marked non-critical.");
                    });
                }
            });

            ui.separator();
            ui.baboon_checkbox(&mut state.show_grid, "Show Grid")
                .on_hover_text(
                    "Ground-reference grid on the z = 0 plane, spaced to the model's size.",
                );
            ui.baboon_checkbox(&mut state.show_backfaces, "Render Backfaces");
            ui.baboon_checkbox(&mut state.game_shaders, "Game Shaders")
                .on_hover_text(
                    "Draw Halo CE, Halo 2 and Halo 3 materials through the game's own \
                     shaders, lit by its default lighting.",
                );
            // Default Lights, PRT and Self Shadow change nothing a Halo CE
            // model draws, and its renderer has no bloom, so its menu leaves
            // them out.
            let halo_ce = data.preview.game == Some(GameId::HaloCe);
            if !halo_ce {
                ui.add_enabled(
                    state.game_shaders,
                    ThemedCheckbox::new(&mut state.default_lights, "Default Lights"),
                )
                .on_hover_text(
                    "Light Halo 2 models as the game does in a level with no lightmaps: \
                     fixed lighting plus two sphere lights. Off, they are lit as an \
                     object in a level with lightmaps.",
                );
                ui.add_enabled(state.game_shaders, ThemedCheckbox::new(&mut state.prt, "PRT"))
                    .on_hover_text(
                        "Use the occlusion Halo 2 and Halo 3 models bake into their vertices. \
                         Off, they light as though nothing shades them.",
                    );
            }
            ui.add_enabled(
                state.game_shaders && (halo_ce || !state.default_lights),
                ThemedCheckbox::new(&mut state.light_follows_camera, "Light Follows Camera"),
            )
            .on_hover_text(
                "Keep the main light of Halo 2 and Halo 3 models beside the viewer as you \
                 orbit. This is the preview's own. Off, it stays fixed to the model.",
            );
            ui.add_enabled(
                state.game_shaders,
                ThemedCheckbox::new(&mut state.active_camo, "Active Camo"),
            )
            .on_hover_text(
                "Draw a Halo 2 or Halo 3 model through its active camouflage, for \
                 the models that have it.",
            );
            if !halo_ce {
                ui.add_enabled(
                    state.game_shaders,
                    ThemedCheckbox::new(&mut state.bloom, "Bloom"),
                )
                .on_hover_text(
                    "Add the glow of Halo 2 and Halo 3 materials with a bloom pass. The \
                     blur is Halo 2's, and its strength is this preview's own: the \
                     game's follows the exposure of the whole scene.",
                );
            }
            if !halo_ce {
                ui.add_enabled(
                    state.game_shaders,
                    ThemedCheckbox::new(&mut state.self_shadow, "Self Shadow"),
                )
                .on_hover_text(
                    "Shadow Halo 2 and Halo 3 models from their main light. Halo 3 uses the \
                     game's own shadow pass; Halo 2's is the preview's, as the game draws none here.",
                );
            }
        },
    );
    let least = min_preview_scale(&data.preview);
    preview_header_menu(ui, ButtonIcon::Find, "Camera", |ui| {
        ui.set_min_width(280.0);
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
        ui.horizontal(|ui| {
            ui.label("Preview Size");
            ui.add(
                egui::Slider::new(
                    model_preview_size,
                    MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                )
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
            );
            draw_model_viewport_size_input(ui, model_preview_size);
        });
        if ui.button("Reset View").clicked() {
            state.yaw = -0.45;
            state.pitch = 0.25;
            state.focus = [0.0; 3];
            state.scale = 1.0;
        }
    });
}

#[derive(Clone, Copy)]
pub(in crate::app::model_preview) enum ModelViewCheckboxIcon {
    Tag([u8; 4]),
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
    !state.overlays_loaded || data.preview.batches.iter().any(|batch| batch.layer == layer)
}

pub(in crate::app::model_preview) fn model_view_icon_checkbox(
    ui: &mut Ui,
    checked: &mut bool,
    icon: ModelViewCheckboxIcon,
    label: &str,
    game: Option<GameId>,
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
                    ModelViewCheckboxIcon::Tag(group) => {
                        paint_tag_icon_at(ui, Some(u32::from_be_bytes(group)), game, icon_rect);
                    }
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

pub(in crate::app::model_preview) fn draw_marker_filter_field(ui: &mut Ui, filter: &mut String) -> egui::Response {
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
    ui.painter().rect_stroke(rect, rounding, stroke, egui::StrokeKind::Middle);
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
