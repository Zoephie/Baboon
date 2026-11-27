use super::super::super::ui_kit::button_icons::{
    ButtonIcon, icon_button, icon_text_button, icon_text_dropdown_button, selectable_icon_text_button,
};
use super::super::super::ui_kit::style::{
    BUTTON_HEIGHT, ICON_BUTTON_SIZE, foundation_fonts, foundation_style,
};
use super::super::super::{Baboon, TagSource};
use super::super::{
    ANIMATION_FRAME_RATE, MIN_MODEL_SETUP_WIDTH, ModelPreviewData, ModelPreviewLayer,
    ModelPreviewState, ModelRegionSelection, ModelShading, PHYSICS_REGION,
    PreviewAnimationEntry, PreviewAnimationPlayback, PreviewAnimationPose, RenderModelPreview,
    RenderModelPreviewBatch, RenderModelPreviewRegion, build_render_preview, frame,
    is_model_physics_overlay_region, load_referenced_tag_from_source, merge_preview_append,
    model_preview_data, reset_model_preview_selection, selected_variant_regions,
};
use super::animation::{
    advance_playback_clock, animation_header_group, draw_animation_combo, frame_tick_xs,
};
use super::model_preview_supports_textures;
use super::variants::{draw_variant_controls, draw_variant_selector};
use super::view_menu::{
    ModelViewCheckboxIcon, draw_marker_filter_field, draw_model_view_settings_menu,
    model_view_icon_checkbox, overlay_layer_available,
};
use super::weapons::{weapon_matches, weapon_picker_scrolls};
use super::*;
use crate::app::model_preview::harness::kit::kit_root;
use crate::app::ui_kit::checkbox::CheckboxUi as _;
use crate::app::ui_kit::radio::RadioUi as _;
use crate::core::game::GameId;
use eframe::egui;
use eframe::egui::{Sense, Vec2};

#[test]
fn middle_drag_pans_model_without_starting_page_autoscroll() {
    let context = egui::Context::default();
    crate::app::ui_kit::autoscroll::install(&context);
    let data = model_preview_data(
        "test".into(),
        "test".into(),
        RenderModelPreview::default(),
        vec![],
    );
    let mut state = ModelPreviewState::default();
    let initial_focus = state.focus;
    let mut time = 0.0;
    let mut frame = |events| {
        time += 1.0 / 60.0;
        let mut offset = 0.0;
        let mut rect = egui::Rect::NOTHING;
        let output = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 350.0),
                )),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let scroll = crate::app::ScrollArea::vertical().show(ui, |ui| {
                        rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, 180.0));
                        crate::app::model_preview::viewport::draw_model_viewport(
                            ui,
                            &data,
                            &mut state,
                            rect.size(),
                        );
                        ui.add_space(900.0);
                    });
                    offset = scroll.state.clamped_offset().y;
                });
                crate::app::ui_kit::autoscroll::draw_blocker(ui);
            },
        );
        let anchor = output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Circle(circle) if (circle.radius - 13.0).abs() < 0.01)
        });
        (rect, offset, anchor)
    };
    frame(vec![]);
    let (rect, _, _) = frame(vec![]);
    let origin = rect.center();
    frame(vec![egui::Event::PointerMoved(origin)]);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Middle,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (_, _, anchor) = frame(vec![button(origin, true)]);
    assert!(!anchor, "model panning must own the initial middle press");
    frame(vec![egui::Event::PointerMoved(
        origin + Vec2::new(30.0, 45.0),
    )]);
    for _ in 0..8 {
        let (_, offset, anchor) = frame(vec![]);
        assert_eq!(
            offset, 0.0,
            "panning a model must not move its containing page"
        );
        assert!(!anchor);
    }
    frame(vec![button(origin + Vec2::new(30.0, 45.0), false)]);
    // A click without a drag must not latch page autoscroll either.
    frame(vec![button(origin, true)]);
    frame(vec![button(origin, false)]);
    assert!(!frame(vec![]).2);
    drop(frame);
    assert_ne!(
        state.focus, initial_focus,
        "middle-drag must still pan the camera"
    );
}

fn bitmap_header_control_rows(screen_width: f32, wrap_width: f32) -> (f32, f32) {
    let context = egui::Context::default();
    let mut left_y = 0.0;
    let mut right_y = 0.0;
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(screen_width, 160.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                draw_model_preview_section_with_header_wrap(
                    ui,
                    "Bitmap Preview",
                    Some(1.0),
                    Some(wrap_width),
                    |ui, part| {
                        if part == ModelPreviewSectionPart::Header {
                            left_y = ui.button("Selector").rect.center().y;
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    right_y = ui.button("Actions").rect.center().y;
                                },
                            );
                        }
                    },
                );
            });
        },
    );
    (left_y, right_y)
}

#[test]
fn bitmap_header_moves_actions_to_a_second_row_below_its_wrap_width() {
    let (wide_left, wide_right) = bitmap_header_control_rows(640.0, 400.0);
    assert!((wide_left - wide_right).abs() < 1.0);

    let (narrow_left, narrow_right) = bitmap_header_control_rows(320.0, 400.0);
    assert!(narrow_right > narrow_left + BUTTON_HEIGHT);
}

/// The Permutations card's first row holds the variant picker between its
/// arrows and, at the row's right end, Save then Delete, all on one line
/// and inside the card from the narrowest sidebar up.
#[test]
fn variant_row_keeps_save_and_delete_beside_the_picker() {
    let data = model_preview_data(
        String::new(),
        String::new(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    for width in [MIN_MODEL_SETUP_WIDTH, 360.0, 520.0] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        context.set_global_style(foundation_style());
        let mut state = ModelPreviewState::default();
        let (mut row, mut save, mut delete) =
            (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
        let _ = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 200.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let top = ui.cursor().top();
                    let mut rows = super::rows::CardRows::default();
                    draw_variant_selector(ui, &data, &mut state, &mut rows, &mut |ui, _| {
                        delete = icon_button(
                            ui,
                            ButtonIcon::Garbage,
                            "Delete selected variant",
                            true,
                            egui::Color32::BLACK,
                        )
                        .rect;
                        save = icon_text_dropdown_button(ui, ButtonIcon::Save, "Save", |_| {})
                            .response
                            .rect;
                    });
                    row = egui::Rect::from_min_max(
                        egui::pos2(0.0, top),
                        egui::pos2(width, ui.cursor().top()),
                    );
                });
            },
        );
        assert!(save.right() <= delete.left(), "width {width}: Save {save:?}, Delete {delete:?}");
        assert!(
            (save.center().y - delete.center().y).abs() < 1.0,
            "width {width}: Save {save:?}, Delete {delete:?}"
        );
        assert!(delete.right() <= width, "width {width}: Delete {delete:?} past the pane");
        assert!(row.height() < 2.0 * BUTTON_HEIGHT, "width {width}: row {row:?} wrapped");
    }
}

/// A sidebar card's header keeps its Reset at the right end of the title
/// row; header controls (a picker and actions) follow the title where they fit and
/// take a row of their own under it where they do not, inside the card at
/// every width.
#[test]
fn setup_card_header_wraps_its_controls_under_the_title_when_narrow() {
    use crate::app::editor::fields::{BoxPart, draw_foundation_box};
    let data = model_preview_data(
        String::new(),
        String::new(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    let mut wrapped_at = Vec::new();
    for width in [MIN_MODEL_SETUP_WIDTH, 300.0, 360.0, 520.0, 900.0] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        context.set_global_style(foundation_style());
        let mut state = ModelPreviewState::default();
        let (mut card, mut controls, mut reset) = (
            egui::Rect::NOTHING,
            egui::Rect::NOTHING,
            egui::Rect::NOTHING,
        );
        // The header sizes its rows from the widths its controls drew at
        // the frame before.
        for _ in 0..3 {
            let _ = crate::app::run_ui_test(
                &context,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 400.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(context, |ui| {
                            let top = ui.cursor().top();
                            draw_foundation_box(ui, "Permutations".into(), "card", |ui, part| {
                                match part {
                                    BoxPart::Controls => {
                                        // A picker between arrows, as a
                                        // header's controls might be.
                                        let _ = (&data, &mut state);
                                        ui.add_sized(
                                            Vec2::new(24.0, BUTTON_HEIGHT),
                                            egui::Button::new("<"),
                                        );
                                        ui.add_sized(
                                            Vec2::new(150.0, BUTTON_HEIGHT),
                                            egui::Button::new("0. default"),
                                        );
                                        ui.add_sized(
                                            Vec2::new(24.0, BUTTON_HEIGHT),
                                            egui::Button::new(">"),
                                        );
                                        icon_text_dropdown_button(
                                            ui,
                                            ButtonIcon::Save,
                                            "Save",
                                            |_| {},
                                        );
                                        icon_text_button(ui, ButtonIcon::Garbage, "Delete", true);
                                        controls = ui.min_rect();
                                    }
                                    BoxPart::Trailing => {
                                        reset = ui.button("R").rect;
                                    }
                                    BoxPart::Body => {}
                                }
                            });
                            card = egui::Rect::from_min_max(
                                egui::pos2(0.0, top),
                                egui::pos2(width, ui.min_rect().bottom()),
                            );
                        });
                },
            );
        }
        let title_row = reset.center().y;
        assert!(
            reset.right() <= card.right() - 7.0,
            "width {width}: Reset {reset:?} in {card:?}"
        );
        assert!(
            controls.right() <= card.right() + 1.0,
            "width {width}: controls {controls:?} overflow {card:?}"
        );
        assert!(
            controls.right() <= reset.left() || controls.top() > reset.bottom(),
            "width {width}: controls {controls:?} under Reset {reset:?}"
        );
        if controls.center().y > title_row + 8.0 {
            wrapped_at.push(width);
        } else {
            assert!(
                (controls.center().y - title_row).abs() < 2.0,
                "width {width}: {controls:?}"
            );
        }
    }
    assert!(
        wrapped_at.contains(&MIN_MODEL_SETUP_WIDTH) && !wrapped_at.contains(&900.0),
        "wrapped at {wrapped_at:?}"
    );
}
#[test]
fn marker_filter_field_matches_button_height() {
    let context = egui::Context::default();
    context.set_fonts(foundation_fonts());
    let mut filter = String::new();
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(320.0, 100.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let available = ui.available_width();
                let response = draw_marker_filter_field(ui, &mut filter);
                assert_eq!(response.rect.height(), BUTTON_HEIGHT);
                assert_eq!(response.rect.width(), available);
            });
        },
    );
}

#[test]
fn physics_overlay_is_not_a_model_setup_variant_region() {
    let preview = RenderModelPreview {
        regions: vec![
            RenderModelPreviewRegion {
                name: "body".into(),
                permutations: vec!["default".into()],
            },
            RenderModelPreviewRegion {
                name: PHYSICS_REGION.into(),
                permutations: vec!["default".into()],
            },
        ],
        batches: vec![
            RenderModelPreviewBatch {
                region_name: "body".into(),
                layer: ModelPreviewLayer::Render,
                sort_position: None,
                ..Default::default()
            },
            RenderModelPreviewBatch {
                region_name: PHYSICS_REGION.into(),
                layer: ModelPreviewLayer::Physics,
                sort_position: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    let mut state = ModelPreviewState {
        overlays_loaded: true,
        ..Default::default()
    };
    reset_model_preview_selection(&mut state, &data, None);

    assert!(is_model_physics_overlay_region(
        &data,
        &data.preview.regions[1]
    ));
    assert!(state.region_selections[PHYSICS_REGION].enabled);
    assert_eq!(selected_variant_regions(&data, &state).len(), 1);
    assert_eq!(
        selected_variant_regions(&data, &state)[0].region_name,
        "body"
    );

    // Its overlays loading again, the model drawn still has them.
    state.overlays_loaded = false;
    assert!(is_model_physics_overlay_region(
        &data,
        &data.preview.regions[1]
    ));
    // A physics tag opened on its own lists its region.
    let mut alone = (*data.preview).clone();
    alone.batches.retain(|batch| batch.layer == ModelPreviewLayer::Physics);
    let alone = model_preview_data(String::new(), String::new(), alone, Vec::new());
    assert!(!is_model_physics_overlay_region(
        &alone,
        &alone.preview.regions[1]
    ));
}

fn layer_batch(
    region: &str,
    permutation: &str,
    layer: ModelPreviewLayer,
) -> RenderModelPreviewBatch {
    RenderModelPreviewBatch {
        region_name: region.into(),
        permutation_name: permutation.into(),
        layer,
        ..Default::default()
    }
}

fn select(state: &mut ModelPreviewState, region: &str, permutation: &str) {
    state.region_selections.insert(
        region.into(),
        ModelRegionSelection {
            enabled: true,
            permutation: permutation.into(),
        },
    );
}

/// An overlay that shares a region name but none of its permutation
/// names falls back to its own first permutation; one that does share
/// the name follows the selection, and render batches never fall back.
#[test]
fn overlay_permutations_follow_the_selection_or_fall_back() {
    use ModelPreviewLayer::{Collision, Render};
    let preview = RenderModelPreview {
        batches: vec![
            layer_batch("__unnamed", "monitor", Render),
            layer_batch("__unnamed", "lightning-100", Render),
            layer_batch("__unnamed", "__base", Collision),
            layer_batch("__unnamed", "damaged", Collision),
            layer_batch("hull", "base", Render),
            layer_batch("hull", "base", Collision),
            layer_batch("hull", "broken", Collision),
        ],
        ..Default::default()
    };
    let mut state = ModelPreviewState {
        overlays_loaded: true,
        show_render: true,
        show_collision: true,
        ..Default::default()
    };
    select(&mut state, "__unnamed", "monitor");
    select(&mut state, "hull", "base");
    assert_eq!(frame::visible_batch_indices(&preview, &state), [0, 2, 4, 5]);

    // A matching collision permutation wins over the fallback.
    select(&mut state, "hull", "broken");
    assert_eq!(frame::visible_batch_indices(&preview, &state), [0, 2, 6]);

    // A disabled region hides its overlay too.
    state
        .region_selections
        .get_mut("__unnamed")
        .unwrap()
        .enabled = false;
    assert_eq!(frame::visible_batch_indices(&preview, &state), [6]);

    // A standalone tag's batches are the primary preview: exact match only.
    state.overlays_loaded = false;
    select(&mut state, "__unnamed", "monitor");
    assert_eq!(frame::visible_batch_indices(&preview, &state), [0, 6]);
}

/// The monitor biped against `BLAM_TEST_HCEEK` (the kit's `tags`): its
/// gbxmodel and collision model share the `__unnamed` region but no
/// permutation name, which hid the collision layer entirely.
#[test]
fn halo_ce_monitor_collision_overlay_is_drawn() {
    let Some(tags) = kit_root(GameId::HaloCe) else {
        return;
    };
    let biped = tags.join("characters/monitor/monitor.biped");
    let definitions = crate::core::bundled::locate_definitions_root();
    let source = TagSource::LooseFolder {
        root: tags,
        game: Some(GameId::HaloCe),
        definitions_root: definitions.clone(),
    };
    let object = crate::core::source::read_tag_from_bytes(
        &std::fs::read(&biped).unwrap(),
        Some(GameId::HaloCe),
        Some(definitions.as_path()),
        u32::from_be_bytes(*b"bipd"),
    )
    .unwrap();
    let render = load_referenced_tag_from_source(
        &source,
        r"characters\monitor\monitor",
        "gbxmodel",
        b"mod2",
    )
    .unwrap();
    let mut preview = build_render_preview(&render).unwrap();
    let collision = super::super::ce::overlays::halo1_object_collision_overlay(
        &object,
        &crate::app::model_preview::load::TagReader::new(&source),
    )
    .unwrap()
    .expect("collision overlay");
    merge_preview_append(&mut preview, &collision);
    let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    let mut state = ModelPreviewState {
        overlays_loaded: true,
        show_render: true,
        show_collision: true,
        ..Default::default()
    };
    reset_model_preview_selection(&mut state, &data, None);

    let visible = frame::visible_batch_indices(&data.preview, &state);
    let layers = |layer| {
        visible
            .iter()
            .filter(|&&index| data.preview.batches[index].layer == layer)
            .count()
    };
    assert!(layers(ModelPreviewLayer::Render) > 0);
    assert!(
        layers(ModelPreviewLayer::Collision) > 0,
        "collision batches hidden"
    );
}

/// The collision toggle disables only once the overlays have landed
/// without a collision layer.
#[test]
fn collision_toggle_is_available_only_with_collision_geometry() {
    let render_only = RenderModelPreview {
        batches: vec![layer_batch("body", "base", ModelPreviewLayer::Render)],
        ..Default::default()
    };
    let mut with_collision = render_only.clone();
    with_collision
        .batches
        .push(layer_batch("body", "base", ModelPreviewLayer::Collision));
    let render_only = model_preview_data(String::new(), String::new(), render_only, Vec::new());
    let with_collision =
        model_preview_data(String::new(), String::new(), with_collision, Vec::new());
    let mut state = ModelPreviewState::default();
    let collision = ModelPreviewLayer::Collision;

    assert!(
        overlay_layer_available(&render_only, &state, collision),
        "unknown yet"
    );
    state.overlays_loaded = true;
    assert!(!overlay_layer_available(&render_only, &state, collision));
    assert!(overlay_layer_available(&with_collision, &state, collision));
}

#[test]
fn permutation_rows_fit_a_narrow_setup_card() {
    let preview = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "helmet_with_a_name_longer_than_any_sidebar_column".into(),
            permutations: vec![
                "minor".into(),
                "chiefweapon".into(),
                "jump_pack".into(),
                "stalker".into(),
            ],
        }],
        ..Default::default()
    };
    let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    for width in [240.0, 280.0, 360.0, 520.0] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        let mut state = ModelPreviewState::default();
        let mut card_rect = egui::Rect::NOTHING;
        let _ = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 600.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let available = ui.available_width();
                    card_rect = draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                        if part == ModelPreviewSectionPart::Body {
                            let mut rows = super::rows::CardRows::default();
                            draw_variant_controls(ui, &data, &mut state, &mut rows);
                        }
                    });
                    assert!(
                        card_rect.width() <= available + 1.0,
                        "pane {width}: card {}, available {available}",
                        card_rect.width()
                    );
                });
            },
        );
    }
}

#[test]
fn variant_rows_are_not_capped_shorter_than_the_setup_card() {
    let preview = RenderModelPreview {
        regions: (0..20)
            .map(|index| RenderModelPreviewRegion {
                name: format!("region_{index}"),
                permutations: vec!["default".into()],
            })
            .collect(),
        ..Default::default()
    };
    let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    let context = egui::Context::default();
    context.set_fonts(foundation_fonts());
    let mut state = ModelPreviewState::default();
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(500.0, 900.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let top = ui.next_widget_position().y;
                draw_variant_controls(ui, &data, &mut state, &mut super::rows::CardRows::default());
                assert!(ui.min_rect().bottom() - top > 500.0);
            });
        },
    );
}

#[test]
fn animation_groups_and_scrubber_fit_narrow_cards() {
    for width in [280.0, 360.0, 520.0, 800.0] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        let _ = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width, 600.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let available = ui.available_width();
                    let rect =
                        draw_model_preview_section(ui, "Animation Player", None, |ui, part| {
                            if part == ModelPreviewSectionPart::Header {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 16.0;
                                    animation_header_group(ui, 240.0, |ui| {
                                        ui.add_sized(
                                            Vec2::new(240.0, BUTTON_HEIGHT),
                                            egui::Button::new("Animation"),
                                        );
                                    });
                                    animation_header_group(ui, 112.0, |ui| {
                                        for _ in 0..4 {
                                            ui.add_sized(ICON_BUTTON_SIZE, egui::Button::new(""));
                                        }
                                    });
                                    animation_header_group(ui, 90.0, |ui| {
                                        ui.label("Speed");
                                        ui.add_sized(
                                            Vec2::new(48.0, BUTTON_HEIGHT),
                                            egui::DragValue::new(&mut 1.0_f32),
                                        );
                                    });
                                    ui.label("2 / 9");
                                });
                            } else {
                                let mut frame = 2.0;
                                ui.spacing_mut().slider_width =
                                    (ui.available_width() - 2.0).max(1.0);
                                ui.add(egui::Slider::new(&mut frame, 0.0..=9.0).show_value(false));
                            }
                        });
                    assert!(
                        rect.width() <= available + 1.0,
                        "pane {width}: card {}, available {available}",
                        rect.width()
                    );
                });
            },
        );
    }
}

/// The cards split the page with nothing over: the animation player along
/// the bottom, the sidebar as wide as asked within its clamp, and the
/// viewport taking the rest.
#[test]
fn panel_rects_split_the_page_and_clamp_the_sidebar() {
    let page = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::new(1_400.0, 900.0));
    let rects = panel_rects(page, 360.0);
    assert_eq!(rects.setup.width(), 360.0);
    assert_eq!(rects.animation.bottom(), page.bottom());
    assert_eq!(rects.animation.width(), page.width());
    assert_eq!(
        rects.animation.height(),
        super::animation::animation_card_height(page.width())
    );
    assert_eq!(rects.preview.left(), page.left());
    assert_eq!(rects.setup.right(), page.right());
    assert_eq!(rects.preview.right(), rects.handle.left());
    assert_eq!(rects.handle.right(), rects.setup.left());
    assert_eq!(
        rects.preview.bottom() + MODEL_PREVIEW_SECTION_GAP,
        rects.animation.top()
    );
    assert_eq!(
        rects.preview.width() + rects.handle.width() + rects.setup.width(),
        page.width()
    );
    // Too narrow: the sidebar keeps its least width.
    assert_eq!(panel_rects(page, 10.0).setup.width(), MIN_MODEL_SETUP_WIDTH);
    // Too wide: the viewport keeps its own.
    let wide = panel_rects(page, 5_000.0);
    assert_eq!(wide.preview.width(), MIN_MODEL_VIEWPORT_WIDTH);
    assert_eq!(
        panel_rects(page, f32::NAN).setup.width(),
        MIN_MODEL_SETUP_WIDTH
    );
}

/// Each card drawn into its rect fills it exactly, at the sidebar header's
/// breakpoints and the animation header's, so nothing pushes the page into
/// scrolling.
#[test]
fn cards_fill_their_rects_at_the_header_breakpoints() {
    for (page_width, setup_width) in [(1_600.0, 360.0), (1_600.0, 800.0), (700.0, 300.0)] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        context.set_global_style(foundation_style());
        let page = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(page_width, 800.0));
        let rects = panel_rects(page, setup_width);
        let mut drawn = [egui::Rect::NOTHING; 2];
        let _ = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(page_width, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let preview_body = rects.preview.height() - PREVIEW_HEADER_HEIGHT;
                    drawn[0] = in_rect(ui, rects.preview, "preview", |ui| {
                        draw_model_preview_section(
                            ui,
                            "Model Preview",
                            Some(preview_body),
                            |ui, part| {
                                if part == ModelPreviewSectionPart::Body {
                                    let viewport = Vec2::new(
                                        ui.available_width(),
                                        preview_body - MODEL_PREVIEW_STATS_FOOTER_HEIGHT,
                                    );
                                    ui.allocate_exact_size(viewport, Sense::hover());
                                    ui.allocate_exact_size(
                                        Vec2::new(viewport.x, MODEL_PREVIEW_STATS_FOOTER_HEIGHT),
                                        Sense::hover(),
                                    );
                                }
                            },
                        )
                    });
                    drawn[1] = in_rect(ui, rects.animation, "animation", |ui| {
                        draw_model_preview_section(
                            ui,
                            "Animation Player",
                            Some(super::animation::ANIMATION_BODY_HEIGHT),
                            |_, _| {},
                        )
                    });
                });
            },
        );
        for (card, rect) in drawn.iter().zip([rects.preview, rects.animation]) {
            assert!(
                (card.height() - rect.height()).abs() < 1.0
                    && (card.top() - rect.top()).abs() < 1.0,
                "page {page_width}, setup {setup_width}: card {card:?} in {rect:?}"
            );
        }
    }
}

#[test]
fn section_body_stays_below_header_inside_a_horizontal_row() {
    let context = egui::Context::default();
    context.set_fonts(foundation_fonts());
    let mut header_rect = None;
    let mut body_rect = None;
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(800.0, 600.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(360.0, 0.0), |ui| {
                        draw_model_preview_section(
                            ui,
                            "Model Preview",
                            None,
                            |ui, part| match part {
                                ModelPreviewSectionPart::Header => {
                                    header_rect = Some(ui.max_rect())
                                }
                                ModelPreviewSectionPart::Body => body_rect = Some(ui.max_rect()),
                            },
                        );
                    });
                });
            });
        },
    );

    let header_rect = header_rect.expect("header was drawn");
    let body_rect = body_rect.expect("body was drawn");
    assert!(body_rect.min.y >= header_rect.max.y);
    // The preview viewport now reaches the card edge; the title retains
    // its 8-point header inset.
    assert!((header_rect.min.x - body_rect.min.x - 8.0).abs() < 1.0);
}

#[test]
fn model_geometry_uses_the_model_preview_tab_name() {
    assert_eq!(
        preview_panel_title(u32::from_be_bytes(*b"hlmt")),
        "Model Preview"
    );
    assert_eq!(
        preview_panel_title(u32::from_be_bytes(*b"mode")),
        "Model Preview"
    );
    assert_eq!(
        preview_panel_title(u32::from_be_bytes(*b"coll")),
        "Collision Model"
    );
}

/// Clearing the picker's search gives the popup back its full height.
#[test]
fn animation_picker_expands_after_clearing_search() {
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    ctx.set_global_style(foundation_style());
    let animations = (0..40)
        .map(|index| PreviewAnimationEntry {
            name: format!("animation {index}"),
            frame_count: 30,
            kind: "jma",
            playable: true,
        })
        .collect::<Vec<_>>();
    let mut playback = PreviewAnimationPlayback::default();
    let frame = |playback: &mut PreviewAnimationPlayback| {
        let mut popup_id = None;
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let id =
                        ui.make_persistent_id(("model_animation_popup", "test_animation_picker"));
                    egui::Popup::open_id(ui.ctx(), id);
                    popup_id = Some(id);
                    draw_animation_combo(
                        ui,
                        "test_animation_picker",
                        &animations,
                        &Default::default(),
                        playback,
                        240.0,
                    );
                });
            },
        );
        ctx.memory(|memory| memory.area_rect(popup_id.unwrap()).unwrap().height())
    };
    for _ in 0..5 {
        frame(&mut playback);
    }
    let full_height = frame(&mut playback);
    playback.filter = "animation 39".to_owned();
    for _ in 0..5 {
        frame(&mut playback);
    }
    let filtered_height = frame(&mut playback);
    assert!(full_height - filtered_height > 150.0);
    playback.filter.clear();
    for _ in 0..5 {
        frame(&mut playback);
    }
    assert!((frame(&mut playback) - full_height).abs() < 1.0);
}

/// Two panes on the same tag draw the same playback state in one pass; the
/// clock moves once. Each pane used to move it, so playback ran at 2x.
#[test]
fn two_panes_advance_the_clock_once_per_pass() {
    let mut playback = PreviewAnimationPlayback {
        playing: true,
        ..Default::default()
    };
    advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
    advance_playback_clock(&mut playback, 7, 0.1, 10.0, 300.0);
    assert!((playback.time - 0.1).abs() < 1e-6, "{}", playback.time);
    advance_playback_clock(&mut playback, 8, 0.1, 10.0, 300.0);
    assert!((playback.time - 0.2).abs() < 1e-6, "{}", playback.time);
}

/// The header's model toggles take the bitmap channels' shortcuts: a click
/// flips one, Alt shows only it, Shift hides only it, and a toggle that
/// can't be used (no collision model) is left as it is.
#[test]
fn model_toggles_take_alt_and_shift_clicks() {
    use super::view_menu::{LayerAction, ModelLayer, apply_layer_actions};
    let mut state = ModelPreviewState::default();
    let shown = |state: &ModelPreviewState| {
        (state.show_render, state.show_collision, state.show_physics, state.show_armature)
    };
    let drawn = |clicked: ModelLayer, action: LayerAction| {
        [ModelLayer::Render, ModelLayer::Collision, ModelLayer::Physics, ModelLayer::Armature]
            .map(|layer| {
                let enabled = layer != ModelLayer::Collision;
                (layer, enabled, if layer == clicked { action } else { LayerAction::None })
            })
    };
    state.show_render = true;
    apply_layer_actions(&mut state, &drawn(ModelLayer::Armature, LayerAction::Toggle));
    assert_eq!(shown(&state), (true, false, false, true));
    apply_layer_actions(&mut state, &drawn(ModelLayer::Physics, LayerAction::Solo));
    assert_eq!(shown(&state), (false, false, true, false));
    apply_layer_actions(&mut state, &drawn(ModelLayer::Physics, LayerAction::Exclude));
    // Collision has nothing to show, so it stays off.
    assert_eq!(shown(&state), (true, false, false, true));
}

/// A clip carried through a reload (Game Shaders on or off) comes back as it
/// was, paused, stopped or playing, at its time; one picked from the list
/// plays from the top.
#[test]
fn a_reloaded_clip_resumes_as_it_was() {
    use super::super::animation::resume_or_start;
    for (playing, stopped) in [(false, false), (false, true), (true, false)] {
        let mut playback = PreviewAnimationPlayback {
            playing,
            stopped,
            time: 1.5,
            resume_on_decode: true,
            ..Default::default()
        };
        resume_or_start(&mut playback);
        assert_eq!((playback.playing, playback.stopped), (playing, stopped));
        assert_eq!(playback.time, 1.5);
        assert!(!playback.resume_on_decode, "resumed once, not every decode");
        // The next clip decoded is a pick: it plays from the top.
        resume_or_start(&mut playback);
        assert_eq!((playback.playing, playback.stopped, playback.time), (true, false, 0.0));
    }
}

/// A looping clock wraps to the animation's loop frame, not to 0, so the
/// slider and the pose agree on where playback is.
#[test]
fn a_looping_clock_wraps_to_the_loop_frame() {
    let frames = vec![Vec::new(); 10];
    let mut playback = PreviewAnimationPlayback {
        playing: true,
        pose: Some(std::sync::Arc::new(PreviewAnimationPose {
            loop_frame: 4,
            ..PreviewAnimationPose::new(0, frames)
        })),
        ..Default::default()
    };
    playback.time = 9.5 / ANIMATION_FRAME_RATE;
    advance_playback_clock(
        &mut playback,
        1,
        1.0 / ANIMATION_FRAME_RATE,
        10.0 / ANIMATION_FRAME_RATE,
        9.0,
    );
    assert!(
        (playback.time * ANIMATION_FRAME_RATE - 4.5).abs() < 1e-3,
        "{}",
        playback.time * ANIMATION_FRAME_RATE
    );
}

#[test]
fn textured_shading_is_limited_to_supported_editing_kits() {
    assert!(model_preview_supports_textures(Some(GameId::Halo3)));
    assert!(model_preview_supports_textures(Some(GameId::Halo3Odst)));
    assert!(model_preview_supports_textures(Some(GameId::HaloReach)));
    assert!(model_preview_supports_textures(Some(GameId::Halo4)));
    assert!(model_preview_supports_textures(Some(GameId::Halo2)));
    assert!(model_preview_supports_textures(Some(GameId::HaloCe)));
    for game in [None, Some(GameId::Halo2Amp), Some(GameId::CampaignEvolved)] {
        assert!(!model_preview_supports_textures(game), "{game:?}");
    }
}

/// Each frame's tick sits exactly where the timeline's handle lands for that
/// frame: the handle travels a range inset from the slider's ends, and ticks
/// spread across the whole slider would drift off it towards each end.
#[test]
fn frame_ticks_sit_where_the_handle_lands() {
    let frames = 12usize;
    let last = (frames - 1) as f32;
    for frame in [0usize, 3, 7, 11] {
        let ctx = egui::Context::default();
        let mut slider_rect = egui::Rect::NOTHING;
        let mut handle_shape = egui::style::HandleShape::Circle;
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.spacing_mut().slider_width = 400.0;
                let mut value = frame as f32;
                let response = ui.add(
                    egui::Slider::new(&mut value, 0.0..=last)
                        .show_value(false)
                        .step_by(1.0),
                );
                slider_rect = response.rect;
                handle_shape = ui.style().visuals.handle_shape;
            });
        });
        // The handle is the one narrow shape inside the slider: not the
        // panel's background, and not the rail, which spans the slider.
        fn handle_x(shape: &egui::Shape, slider: egui::Rect) -> Option<f32> {
            let rect = match shape {
                egui::Shape::Rect(rect) => rect.rect,
                egui::Shape::Circle(circle) => egui::Rect::from_center_size(
                    circle.center,
                    egui::Vec2::splat(circle.radius * 2.0),
                ),
                egui::Shape::Vec(shapes) => {
                    return shapes.iter().find_map(|shape| handle_x(shape, slider));
                }
                _ => return None,
            };
            (slider.contains(rect.center()) && rect.width() < slider.height() * 2.0)
                .then(|| rect.center().x)
        }
        let handle_x = output
            .shapes
            .iter()
            .find_map(|clipped| handle_x(&clipped.shape, slider_rect))
            .expect("the slider paints its handle");
        let ticks = frame_tick_xs(slider_rect, handle_shape, last, frames);
        assert_eq!(ticks.len(), frames, "a tick per frame at this width");
        assert!(
            (ticks[frame] - handle_x).abs() < 0.5,
            "frame {frame}: tick at {} but the handle at {handle_x}",
            ticks[frame]
        );
    }
}

/// A long animation is ticked sparsely rather than drawn as a solid bar.
#[test]
fn a_long_animation_is_ticked_every_few_frames() {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 18.0));
    let ticks = frame_tick_xs(rect, egui::style::HandleShape::Circle, 999.0, 1000);
    assert!(
        ticks.len() < 100 && ticks.len() > 10,
        "{} ticks",
        ticks.len()
    );
    assert!(ticks.windows(2).all(|pair| pair[1] - pair[0] >= 3.0));
}

/// A shading choice is picked from inside the Shading menu. It was a combo
/// box in the View menu, whose list is a popup of its own; egui 0.36 keeps one
/// popup open at a time, so opening the list closed the menu and nothing could
/// be picked. Game Shaders is one of the choices, and picking it turns the
/// game's shaders on.
#[test]
fn a_shading_choice_is_picked_inside_the_shading_menu() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::foundation_fonts());
    ctx.set_global_style(crate::app::foundation_style());
    let data = model_preview_data(
        "test".to_owned(),
        "test".to_owned(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    let mut state = ModelPreviewState::default();
    let target = ModelShading::GameShaders;
    assert_ne!(state.shading(), target);
    let mut time = 0.0;
    type Painted = Vec<(String, egui::Rect)>;
    let mut frame = |events: Vec<egui::Event>, state: &mut ModelPreviewState| -> Painted {
        time += 0.1;
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 900.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_model_view_settings_menu(
                        ui,
                        &data,
                        state,
                        true,
                        None,
                        false,
                    );
                });
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().trim().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    };
    fn click(
        frame: &mut impl FnMut(Vec<egui::Event>, &mut ModelPreviewState) -> Vec<(String, egui::Rect)>,
        at: egui::Pos2,
        state: &mut ModelPreviewState,
    ) -> Vec<(String, egui::Rect)> {
        for step in 1..=3 {
            frame(
                vec![egui::Event::PointerMoved(
                    at - egui::vec2(0.0, 3.0 - step as f32),
                )],
                state,
            );
        }
        for pressed in [true, false] {
            frame(
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                state,
            );
        }
        frame(Vec::new(), state)
    }
    frame(Vec::new(), &mut state);
    let shown = frame(Vec::new(), &mut state);
    let menu = shown
        .iter()
        .find(|(text, _)| text == "Textured")
        .unwrap_or_else(|| panic!("the Shading button: {shown:?}"))
        .1
        .center();
    let shown = click(&mut frame, menu, &mut state);
    let option = shown
        .iter()
        .find(|(text, _)| text == target.label())
        .unwrap_or_else(|| panic!("the menu lists {:?}: {shown:?}", target.label()))
        .1
        .center();
    click(&mut frame, option, &mut state);
    assert_eq!(state.shading(), target, "the choice was picked");
    assert!(state.game_shaders, "Game Shaders is on");
}

#[test]
fn the_weapon_picker_filters_only_a_list_that_scrolls() {
    let weapon = |class: &str, name: &str| super::super::weapons::WeaponOption {
        path: name.to_owned(),
        name: name.to_owned(),
        class: class.to_owned(),
    };
    let few = [weapon("pistol", "pistol"), weapon("rifle", "needler")];
    assert!(!weapon_picker_scrolls(&few, 20.0));
    let many = vec![weapon("rifle", "shotgun"); 15];
    assert!(weapon_picker_scrolls(&many, 20.0));

    assert!(weapon_matches(&few[0], ""));
    assert!(weapon_matches(&few[1], "rifle"));
    assert!(!weapon_matches(&few[1], "pistol"));
}

#[test]
fn icon_checkbox_label_has_no_text_cursor() {
    let ctx = egui::Context::default();
    Baboon::configure_context(&ctx);
    let mut checked = false;
    let mut frame = |events, time| {
        crate::app::run_ui_test(
            &ctx,
            crate::app::loose_fixture::screen(events, time),
            |ui| {
                model_view_icon_checkbox(
                    ui,
                    &mut checked,
                    ModelViewCheckboxIcon::Markers,
                    "Markers",
                );
            },
        )
    };
    let first = frame(vec![], 0.0);
    let at = first
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Markers" => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
            }
            _ => None,
        })
        .expect("checkbox label");
    let hover = frame(vec![egui::Event::PointerMoved(at)], 0.1);
    assert_ne!(hover.platform_output.cursor_icon, egui::CursorIcon::Text);
    for (time, pressed) in [(0.2, true), (0.3, false)] {
        frame(
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }],
            time,
        );
    }
    assert!(checked, "clicking the label still toggles its checkbox");
}

#[test]
fn menu_checkbox_glyphs_align_with_option_icons() {
    let ctx = egui::Context::default();
    Baboon::configure_context(&ctx);
    let mut checked = false;
    let mut output = None;
    for time in [0.0, 0.1] {
        output = Some(crate::app::run_ui_test(
            &ctx,
            crate::app::loose_fixture::screen(vec![], time),
            |ui| {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .ui_stack_info(egui::UiStackInfo::new(egui::UiKind::Menu)),
                    |ui| {
                        crate::app::browser::style_list_menu(ui);
                        selectable_icon_text_button(ui, ButtonIcon::FolderOpen, "Folders", true);
                        model_view_icon_checkbox(
                            ui,
                            &mut checked,
                            ModelViewCheckboxIcon::Markers,
                            "Markers",
                        );
                        ui.baboon_checkbox(&mut checked, "Show prefixes");
                        ui.baboon_radio_value(&mut checked, true, "Textured");
                    },
                );
            },
        ));
    }
    let output = output.unwrap();
    let images: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
            egui::Shape::Circle(circle) => Some(egui::Rect::from_center_size(
                circle.center,
                Vec2::splat(circle.radius * 2.0),
            )),
            egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect.rect),
            _ => None,
        })
        .filter(|rect| {
            (14.0..=16.1).contains(&rect.width()) && (rect.height() - rect.width()).abs() < 0.1
        })
        .collect();
    let glyph_x = |label| {
        let y = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos.y + text.galley.size().y * 0.5)
                }
                _ => None,
            })
            .expect("row label");
        images
            .iter()
            .filter(|rect| (rect.center().y - y).abs() < 1.0)
            .map(|rect| rect.left())
            .min_by(f32::total_cmp)
            .expect("row glyph")
    };
    assert_eq!(
        glyph_x("Markers"),
        glyph_x("Folders"),
        "icon-checkbox row inset"
    );
    assert_eq!(
        glyph_x("Show prefixes"),
        glyph_x("Folders"),
        "plain checkbox inset"
    );
    assert_eq!(glyph_x("Textured"), glyph_x("Folders"), "radio row inset");
}

/// The sidebar's cards, 360 points wide, drawn frame after frame.
struct SetupBody {
    context: egui::Context,
    time: f64,
}

impl SetupBody {
    const WIDTH: f32 = 360.0;

    fn new() -> Self {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        context.set_global_style(foundation_style());
        Self { context, time: 0.0 }
    }

    /// One frame with `events`: the texts it painted, and where.
    fn frame(
        &mut self,
        data: &ModelPreviewData,
        state: &mut ModelPreviewState,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        self.time += 1.0 / 60.0;
        let output = crate::app::run_ui_test(
            &self.context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(Self::WIDTH, 900.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(context, |ui| {
                        super::functions::draw_setup_body(ui, data, state, true, &mut |_, _| {})
                    });
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    /// Where `title`'s section header puts its Reset: at the row's right.
    fn reset_of(texts: &[(String, egui::Rect)], title: &str) -> egui::Pos2 {
        let row = texts
            .iter()
            .find(|(text, _)| text == title)
            .unwrap_or_else(|| panic!("no {title} section: {texts:?}"))
            .1;
        // Past the card's 8-point header padding.
        egui::pos2(Self::WIDTH - 8.0 - ICON_BUTTON_SIZE.x / 2.0, row.center().y)
    }

    /// Whether the widget at `at` in the last frame was enabled. A pass
    /// ends by swapping its widgets into `prev_pass`.
    fn enabled_at(&self, at: egui::Pos2) -> Option<bool> {
        self.context.viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|widget| widget.rect.contains(at) && widget.sense.senses_click())
                .last()
                .map(|widget| widget.enabled)
        })
    }

    fn click(&mut self, data: &ModelPreviewData, state: &mut ModelPreviewState, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(data, state, vec![egui::Event::PointerMoved(at)]);
        self.frame(data, state, vec![button(true)]);
        self.frame(data, state, vec![button(false)]);
    }
}

/// A Halo CE biped with a scenario BSP, two permutations of one region, a
/// weapon it can hold and functions, but no effects.
fn sectioned_object() -> ModelPreviewData {
    let preview = RenderModelPreview {
        game: Some(GameId::HaloCe),
        regions: vec![RenderModelPreviewRegion {
            name: "body".into(),
            permutations: vec!["base".into(), "alt".into()],
        }],
        ..RenderModelPreview::default()
    };
    let mut data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    data.scenario_bsps = vec![Some("levels\\a\\a.scenario_structure_bsp".into())];
    data.ce.weapons = Some(crate::app::model_preview::ce::weapons::CeBipedWeapons {
        equippable: Some(vec![
            crate::app::model_preview::ce::weapons::CeEquippable::for_test("weapons\\rifle\\rifle"),
        ]),
        hidden_clips: Default::default(),
    });
    data.ce.functions = Some(Default::default());
    data
}

/// A biped's weapons are listed once its picker opens, not by the load: until
/// then the picker counts none and opening it asks for them. A listed one
/// never asks.
#[test]
fn the_weapon_picker_asks_for_its_weapons_when_first_opened() {
    let opened = |listed: bool| {
        let mut data = sectioned_object();
        if !listed && let Some(weapons) = data.ce.weapons.as_mut() {
            weapons.equippable = None;
        }
        let mut state = ModelPreviewState::default();
        reset_model_preview_selection(&mut state, &data, None);
        let mut body = SetupBody::new();
        body.frame(&data, &mut state, Vec::new());
        let texts = body.frame(&data, &mut state, Vec::new());
        let count = if listed { "(1)" } else { "(…)" };
        let button = texts
            .iter()
            .find(|(text, _)| text == count)
            .unwrap_or_else(|| panic!("no {count} on the picker: {texts:?}"))
            .1
            .center();
        assert!(!state.list_weapons, "nothing asks before it opens");
        body.click(&data, &mut state, button);
        let texts = body.frame(&data, &mut state, Vec::new());
        let listing = texts.iter().any(|(text, _)| text == "Listing weapons…");
        (state.list_weapons, listing)
    };
    assert_eq!(opened(false), (true, true));
    assert_eq!(opened(true), (false, false));
}

/// Without Game Shaders the object's functions get no card, though it has
/// some: only Game Shaders read them.
#[test]
fn the_functions_card_shows_with_game_shaders_alone() {
    let data = sectioned_object();
    let shown = |game_shaders| {
        let mut state = ModelPreviewState {
            game_shaders,
            ..Default::default()
        };
        reset_model_preview_selection(&mut state, &data, None);
        let mut body = SetupBody::new();
        body.frame(&data, &mut state, Vec::new());
        let texts = body.frame(&data, &mut state, Vec::new());
        texts.iter().any(|(text, _)| text == "Object Functions")
    };
    assert!(shown(true));
    assert!(!shown(false));
}

/// Each section shows only with something in it, and its Reset is enabled
/// only once something in it is changed, and puts that back.
#[test]
fn setup_sections_reset_what_they_hold_and_only_once_it_changed() {
    const TITLES: [&str; 4] = [
        "Structure BSPs",
        "Permutations",
        "Weapons",
        "Object Functions",
    ];
    let data = sectioned_object();
    // The Effects and Object Functions cards show with Game Shaders alone.
    let mut state = ModelPreviewState {
        game_shaders: true,
        ..Default::default()
    };
    reset_model_preview_selection(&mut state, &data, None);
    let mut body = SetupBody::new();
    body.frame(&data, &mut state, Vec::new());
    let texts = body.frame(&data, &mut state, Vec::new());
    assert!(
        !texts.iter().any(|(text, _)| text == "Effects"),
        "an Effects section with no effects: {texts:?}"
    );
    for title in TITLES {
        let at = SetupBody::reset_of(&texts, title);
        assert_eq!(
            body.enabled_at(at),
            Some(false),
            "{title}'s Reset with nothing changed"
        );
    }

    state.scenario_bsp_selection.insert(0);
    state.region_selections.get_mut("body").unwrap().permutation = "alt".into();
    state.weapon = Some("weapons\\rifle\\rifle".into());
    state
        .ce
        .function_inputs
        .insert((data.geometry_id, 0), [0.5; 4]);
    let texts = body.frame(&data, &mut state, Vec::new());
    for title in TITLES {
        let at = SetupBody::reset_of(&texts, title);
        assert_eq!(
            body.enabled_at(at),
            Some(true),
            "{title}'s Reset with it changed"
        );
    }

    for title in TITLES {
        let texts = body.frame(&data, &mut state, Vec::new());
        body.click(&data, &mut state, SetupBody::reset_of(&texts, title));
    }
    assert!(state.scenario_bsp_selection.is_empty());
    assert_eq!(state.region_selections["body"].permutation, "base");
    assert_eq!(state.weapon, None);
    assert!(
        state.reload_carry.is_some(),
        "the selection rides the weapon's reload"
    );
    assert!(state.ce.function_inputs.is_empty());
}

/// Halo CE's Effects reset puts its scales back and forgets its plays and
/// shots, of this object only.
#[test]
fn halo_ce_effects_reset_forgets_this_objects_plays() {
    let controls = crate::app::model_preview::ce::panel::CONTROLS.effects;
    let data = sectioned_object();
    let mut state = ModelPreviewState::default();
    assert!(!(controls.changed)(&data, &state));
    state.ce.event_scales = [0.5, 0.5];
    assert!((controls.changed)(&data, &state));
    (controls.reset)(&data, &mut state);
    assert!(!(controls.changed)(&data, &state));
    state.ce.event_counts.insert((data.geometry_id, 0), 2);
    state.ce.event_counts.insert((data.geometry_id + 1, 0), 3);
    assert!((controls.changed)(&data, &state));
    (controls.reset)(&data, &mut state);
    assert!(!(controls.changed)(&data, &state));
    assert_eq!(
        state.ce.event_counts.len(),
        1,
        "another object's plays went too"
    );
}

/// The shared Effects and Object Functions sections split the object's
/// inputs between them: each resets its own and leaves the other's.
#[test]
fn effects_and_functions_reset_their_own_inputs() {
    use super::functions::{held_inputs, release_inputs};
    let mut state = ModelPreviewState::default();
    let events = ["fire"];
    state.function_inputs.insert((7, "fire".into()), 1.0);
    state.function_inputs.insert((7, "speed".into()), 0.3);
    state.function_inputs.insert((8, "fire".into()), 1.0);
    assert!(held_inputs(&state, 7, &events, true));
    assert!(held_inputs(&state, 7, &events, false));
    release_inputs(&mut state, 7, &events, true);
    assert!(!held_inputs(&state, 7, &events, true));
    assert!(
        held_inputs(&state, 7, &events, false),
        "the slider went with the effect"
    );
    assert!(state.function_inputs.contains_key(&(8, "fire".into())));
    release_inputs(&mut state, 7, &events, false);
    assert!(!held_inputs(&state, 7, &events, false));
    assert_eq!(state.function_inputs.len(), 1);
}

/// A region's permutations are a dropdown: it opens on a search field that
/// narrows the list, and picking one sets and turns on the region.
#[test]
fn permutation_dropdown_filters_and_picks() {
    let data = sectioned_object();
    let mut state = ModelPreviewState::default();
    reset_model_preview_selection(&mut state, &data, None);
    state.region_selections.get_mut("body").unwrap().enabled = false;
    let mut body = SetupBody::new();
    body.frame(&data, &mut state, Vec::new());
    let texts = body.frame(&data, &mut state, Vec::new());
    let count = |texts: &[(String, egui::Rect)], needle: &str| {
        texts.iter().filter(|(text, _)| text == needle).count()
    };
    assert_eq!(
        count(&texts, "alt"),
        0,
        "a closed dropdown lists nothing: {texts:?}"
    );
    let button = texts
        .iter()
        .find(|(text, _)| text == "base")
        .expect("the dropdown")
        .1;
    body.click(&data, &mut state, button.center());
    let texts = body.frame(&data, &mut state, Vec::new());
    assert_eq!(count(&texts, "alt"), 1, "the open list: {texts:?}");
    assert_eq!(
        count(&texts, "base"),
        2,
        "the button and the list: {texts:?}"
    );
    body.frame(&data, &mut state, vec![egui::Event::Text("al".into())]);
    let texts = body.frame(&data, &mut state, Vec::new());
    assert_eq!(count(&texts, "base"), 1, "the filter kept base: {texts:?}");
    let alt = texts
        .iter()
        .find(|(text, _)| text == "alt")
        .expect("alt listed")
        .1;
    body.click(&data, &mut state, alt.center());
    let selection = &state.region_selections["body"];
    assert_eq!(selection.permutation, "alt");
    assert!(
        selection.enabled,
        "picking a permutation turns its region on"
    );
}

/// An object's bounding sphere is its bounding radius about its bounding
/// offset, found under the parent groups' fields; none until the radius is
/// set, and none for a tag that is no object.
#[test]
fn an_objects_bounding_sphere_reads_its_radius_and_offset() {
    let definitions = crate::core::bundled::locate_definitions_root();
    let mut biped = blam_tags::TagFile::new(definitions.join("halo3_mcc/biped.json")).unwrap();
    let game = Some(GameId::Halo3);
    assert_eq!(super::view_menu::object_bounding_sphere(&biped, game), None);
    // The parent groups' fields sit in a struct named after each.
    let (radius, offset) = ("unit/object/bounding radius", "unit/object/bounding offset");
    let mut root = biped.root_mut();
    root.field_path_mut(radius)
        .unwrap_or_else(|| panic!("{radius}"))
        .set(blam_tags::TagFieldData::Real(0.75))
        .unwrap();
    root.field_path_mut(offset)
        .unwrap_or_else(|| panic!("{offset}"))
        .set(blam_tags::TagFieldData::RealPoint3d(blam_tags::math::RealPoint3d {
            x: 0.0,
            y: 0.0,
            z: 0.5,
        }))
        .unwrap();
    let sphere = super::view_menu::object_bounding_sphere(&biped, game).expect("a sphere");
    assert_eq!(sphere.radius, 0.75);
    assert_eq!(sphere.center, [0.0, 0.0, 0.5]);

    let render = blam_tags::TagFile::new(definitions.join("halo3_mcc/render_model.json")).unwrap();
    assert_eq!(super::view_menu::object_bounding_sphere(&render, game), None);
}

/// With the H3 and Halo CE kits: a shipped object's bounding sphere sits
/// about its root node, as the game puts it, so it holds the middle of what
/// it bounds. From the object's origin, a biped's sat at its feet.
#[test]
fn a_shipped_objects_bounding_sphere_holds_its_geometry() {
    use crate::app::model_preview::harness::kit::kit_preview;
    let cases: &[(GameId, &str, &str, &[u8; 4])] = &[
        (GameId::Halo3, r"objects\characters\masterchief\masterchief", "biped", b"bipd"),
        (GameId::Halo3, r"objects\weapons\rifle\assault_rifle\assault_rifle", "weapon", b"weap"),
        (GameId::Halo3, r"objects\vehicles\warthog\warthog", "vehicle", b"vehi"),
        (GameId::HaloCe, r"characters\cyborg\cyborg", "biped", b"bipd"),
        (GameId::HaloCe, r"vehicles\warthog\mp_warthog", "vehicle", b"vehi"),
    ];
    for (game, path, extension, group) in cases {
        let Some(root) = kit_root(*game) else { continue };
        let source = crate::app::model_preview::harness::kit::kit_source(*game).unwrap();
        let tag = crate::app::export::load_referenced_tag_from_source(&source, path, extension, group)
            .unwrap_or_else(|error| panic!("{game:?} {path}: {error}"));
        let sphere = super::view_menu::object_bounding_sphere(&tag, Some(*game));
        let data = kit_preview(&root, *game, path, extension, group);
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for vertex in &data.preview.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
        let mid: Vec<f32> = (0..3).map(|a| (min[a] + max[a]) * 0.5).collect();
        let state = ModelPreviewState::default();
        let root = crate::app::model_preview::armature_node_matrices(&data, &state);
        let placed = sphere
            .expect("a bounding sphere")
            .about_root(root.first());
        let distance = (0..3)
            .map(|axis| (placed.center[axis] - mid[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            distance < placed.radius * 0.5,
            "{game:?} {path}: {placed:?} about the middle {mid:?}"
        );
    }
}

/// Pose mode's gizmo, driven through the viewport: a click on a joint
/// selects it; a drag that starts on its x arrow moves it along x alone,
/// though the pointer strays across the arrow too; a click on empty space
/// deselects it.
#[test]
fn a_drag_on_the_gizmos_x_arrow_moves_the_joint_along_x_alone() {
    use crate::app::model_preview::{PreviewCamera, RenderModelPreviewNode, armature_node_positions};
    // A bent arm: a root, an elbow up and out, a hand back down level.
    let worlds = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.3], [0.8, 0.0, 0.0]];
    let locals = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.3], [0.4, 0.0, -0.3]];
    let nodes = (0..3)
        .map(|index| RenderModelPreviewNode {
            name: format!("node {index}"),
            parent: index as i16 - 1,
            bind_rotation: [0.0, 0.0, 0.0, 1.0],
            bind_translation: locals[index],
            inverse_bind: [
                [1.0, 0.0, 0.0, -worlds[index][0]],
                [0.0, 1.0, 0.0, -worlds[index][1]],
                [0.0, 0.0, 1.0, -worlds[index][2]],
            ],
        })
        .collect();
    let preview = RenderModelPreview {
        nodes,
        bounds_min: [-1.0; 3],
        bounds_max: [1.0; 3],
        ..Default::default()
    };
    let data = model_preview_data("test".into(), "test".into(), preview, vec![]);
    let mut state = ModelPreviewState::default();
    state.pose.on = true;
    let context = egui::Context::default();
    let mut time = 0.0;
    let mut rect = egui::Rect::NOTHING;
    let mut frame = |state: &mut ModelPreviewState, events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 400.0))),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(400.0, 300.0));
                    crate::app::model_preview::viewport::draw_model_viewport(ui, &data, state, rect.size());
                });
            },
        );
        rect
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // Slid onto a point over a few frames, as a pointer comes.
    let slide = |frame: &mut dyn FnMut(&mut ModelPreviewState, Vec<egui::Event>) -> egui::Rect,
                 state: &mut ModelPreviewState,
                 from: egui::Pos2,
                 to: egui::Pos2| {
        for step in 1..=4 {
            frame(state, vec![egui::Event::PointerMoved(from + (to - from) * (step as f32 / 4.0))]);
        }
    };
    frame(&mut state, vec![]);
    let rect = frame(&mut state, vec![]);
    let camera = PreviewCamera::new(&data, &state, rect);
    let hand = armature_node_positions(&data, &state)[2];
    let at = camera.project(hand).pos;
    slide(&mut frame, &mut state, rect.center(), at);
    frame(&mut state, vec![button(at, true)]);
    frame(&mut state, vec![button(at, false)]);
    assert_eq!(state.pose.selected, Some(2), "a click selects the joint");
    // Along the x arrow, most of the way out.
    let length = 70.0 / camera.points_per_unit(hand);
    let arrow = camera.project([hand[0] + length * 0.7, hand[1], hand[2]]).pos;
    let along = (arrow - at).normalized();
    let across = Vec2::new(-along.y, along.x);
    slide(&mut frame, &mut state, at, arrow);
    frame(&mut state, vec![button(arrow, true)]);
    let to = arrow + along * 25.0 + across * 20.0;
    slide(&mut frame, &mut state, arrow, to);
    frame(&mut state, vec![button(to, false)]);
    let moved = armature_node_positions(&data, &state)[2];
    assert!((moved[0] - hand[0]).abs() > 0.01, "moved along x: {moved:?}");
    assert!((moved[1] - hand[1]).abs() < 1e-3 && (moved[2] - hand[2]).abs() < 1e-3, "y and z held: {hand:?} → {moved:?}");
    // A click on empty space lets the joint go.
    let empty = rect.min + Vec2::new(12.0, 12.0);
    slide(&mut frame, &mut state, to, empty);
    frame(&mut state, vec![button(empty, true)]);
    frame(&mut state, vec![button(empty, false)]);
    assert_eq!(state.pose.selected, None, "a click on empty space deselects");
}

/// Pose mode's rotate gizmo, driven through the viewport: with the elbow
/// selected, a drag along its z ring the way the ring runs turns the hand
/// about z that way round, the elbow staying put and the hand's height kept.
#[test]
fn a_drag_round_the_gizmos_z_ring_turns_the_joint_about_z() {
    use crate::app::model_preview::pose::GizmoMode;
    use crate::app::model_preview::{PreviewCamera, RenderModelPreviewNode, armature_node_positions};
    let worlds = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.3], [0.8, 0.0, 0.0]];
    let locals = [[0.0, 0.0, 0.0], [0.4, 0.0, 0.3], [0.4, 0.0, -0.3]];
    let nodes = (0..3)
        .map(|index| RenderModelPreviewNode {
            name: format!("node {index}"),
            parent: index as i16 - 1,
            bind_rotation: [0.0, 0.0, 0.0, 1.0],
            bind_translation: locals[index],
            inverse_bind: [
                [1.0, 0.0, 0.0, -worlds[index][0]],
                [0.0, 1.0, 0.0, -worlds[index][1]],
                [0.0, 0.0, 1.0, -worlds[index][2]],
            ],
        })
        .collect();
    let preview = RenderModelPreview {
        nodes,
        bounds_min: [-1.0; 3],
        bounds_max: [1.0; 3],
        ..Default::default()
    };
    let data = model_preview_data("test".into(), "test".into(), preview, vec![]);
    let mut state = ModelPreviewState::default();
    state.pose.on = true;
    state.pose.gizmo = GizmoMode::Rotate;
    // Looking down on it from above, so the z ring is a full circle.
    state.pitch = 1.2;
    // The pose belongs to this model before the elbow is picked.
    state.pose.follow(data.geometry_id);
    state.pose.selected = Some(1);
    let context = egui::Context::default();
    let mut time = 0.0;
    let mut rect = egui::Rect::NOTHING;
    let mut frame = |state: &mut ModelPreviewState, events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(500.0, 400.0))),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(400.0, 300.0));
                    crate::app::model_preview::viewport::draw_model_viewport(ui, &data, state, rect.size());
                });
            },
        );
        rect
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut state, vec![]);
    let rect = frame(&mut state, vec![]);
    let camera = PreviewCamera::new(&data, &state, rect);
    let before = armature_node_positions(&data, &state);
    let elbow = before[1];
    // A point on the z ring, at 0.6 of a turn round, and the way it runs.
    let radius = 60.0 / camera.points_per_unit(elbow);
    let on_ring = |t: f32| camera.project([elbow[0] + radius * t.cos(), elbow[1] + radius * t.sin(), elbow[2]]).pos;
    let (t, step) = (3.8_f32, 0.05_f32);
    let start = on_ring(t);
    let way = (on_ring(t + step) - on_ring(t - step)).normalized();
    let to = start + way * 30.0;
    for index in 1..=4 {
        let at = rect.center() + (start - rect.center()) * (index as f32 / 4.0);
        frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    }
    frame(&mut state, vec![button(start, true)]);
    for index in 1..=4 {
        frame(&mut state, vec![egui::Event::PointerMoved(start + (to - start) * (index as f32 / 4.0))]);
    }
    frame(&mut state, vec![button(to, false)]);
    let after = armature_node_positions(&data, &state);
    let around = |hand: [f32; 3]| (hand[1] - elbow[1]).atan2(hand[0] - elbow[0]);
    assert!((0..3).all(|axis| (after[1][axis] - elbow[axis]).abs() < 1e-4), "the elbow stays");
    assert!((after[2][2] - before[2][2]).abs() < 1e-4, "the hand keeps its height");
    let turned = around(after[2]) - around(before[2]);
    assert!(turned > 0.1, "turned about z the way the ring runs: {turned}");
}

/// With `BLAM_TEST_H3EK`: the Model Preview card's header fits its width,
/// from wide down to narrow: no two of its labels or icons overlap, and
/// none leaves the card. Posing, the header carries its most.
#[test]
fn the_preview_header_never_overlaps_itself() {
    use crate::app::model_preview::load::TagReader;
    use crate::app::{TagEntry, TagEntryLocation};
    let Some(root) = kit_root(GameId::Halo3) else {
        return;
    };
    let path = "objects\\characters\\masterchief\\masterchief";
    let data = crate::app::model_preview::harness::kit::kit_preview(&root, GameId::Halo3, path, "biped", b"bipd");
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    let tag = TagReader::new(&source).load(path, "biped", b"bipd").unwrap();
    let entry = TagEntry {
        key: format!("{path}.biped"),
        display_path: format!("{path}.biped"),
        group_tag: u32::from_be_bytes(*b"bipd"),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::LooseFile(root.join("objects/characters/masterchief/masterchief.biped")),
    };
    let panel = super::layout::PreviewPanel {
        tag: &tag,
        entry: &entry,
        source_game: Some(GameId::Halo3),
        supports_textures: true,
        own_view: None,
        data: &data,
        reloading: false,
    };
    for width in [1400.0_f32, 1000.0, 800.0, 640.0, 520.0, 440.0, 380.0, 340.0] {
        let mut state = ModelPreviewState::default();
        state.pose.on = true;
        state.pose.follow(data.geometry_id);
        state.pose.turns.insert(0, blam_tags::math::RealQuaternion::IDENTITY);
        // Untextured, so the viewport draws rather than waiting on textures.
        state.set_shading(crate::app::model_preview::ModelShading::Solid);
        let context = egui::Context::default();
        let mut card = egui::Rect::NOTHING;
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let output = crate::app::run_ui_test(
                &context,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width + 40.0, 500.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut requests = super::layout::SetupRequests::default();
                        ui.allocate_ui(Vec2::new(width, 480.0), |ui| {
                            ui.set_width(width);
                            card = ui.max_rect();
                            super::layout::draw_preview_card(ui, &panel, &mut state, &mut requests, 460.0);
                        });
                    });
                },
            );
            shapes = output.shapes;
        }
        // What the header paints: each label's and each icon's bounds.
        let header = egui::Rect::from_min_size(card.min, Vec2::new(card.width(), super::layout::PREVIEW_HEADER_HEIGHT));
        let mut pieces: Vec<egui::Rect> = Vec::new();
        let mut labels = String::new();
        for clipped in &shapes {
            if let egui::Shape::Text(text) = &clipped.shape
                && header.contains(text.visual_bounding_rect().center())
            {
                labels.push_str(text.galley.text());
                labels.push('|');
            }
            let rect = match &clipped.shape {
                egui::Shape::Text(text) => text.visual_bounding_rect(),
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => mesh.calc_bounds(),
                _ => continue,
            };
            // A hair's overlap at an edge is anti-aliasing, not a clash.
            let rect = rect.shrink(1.0);
            if rect.is_positive() && header.contains(rect.center()) {
                pieces.push(rect);
            }
        }
        assert!(pieces.len() > 3, "{width}: the header drew its controls");
        if width >= 1000.0 {
            assert!(labels.contains("Refresh Model") && labels.contains("Pose"), "{width}: {labels}");
        }
        // Pose's tools ride in the viewport, not the header.
        assert!(!labels.contains("Rotate"), "{width}: {labels}");
        let strip = shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == "Rotate" => Some(text.visual_bounding_rect()),
            _ => None,
        });
        let strip = strip.unwrap_or_else(|| panic!("{width}: no Rotate in the viewport"));
        assert!(strip.top() > header.bottom() && strip.left() < card.left() + 200.0, "{width}: {strip:?}");
        for (index, a) in pieces.iter().enumerate() {
            assert!(a.min.x >= card.min.x - 1.0 && a.max.x <= card.max.x + 1.0, "{width}: {a:?} leaves the card {card:?}");
            for b in &pieces[index + 1..] {
                assert!(!a.intersects(*b), "{width}: {a:?} overlaps {b:?}");
            }
        }
    }
}
