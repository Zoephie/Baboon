use super::super::super::ui_kit::button_icons::{
    ButtonIcon, icon_text_button, icon_text_dropdown_button, selectable_icon_text_button,
};
use super::super::super::ui_kit::style::{
    BUTTON_HEIGHT, ICON_BUTTON_SIZE, foundation_fonts, foundation_style,
};
use super::super::super::{Baboon, TagSource, locate_definitions_root};
use super::super::{
    ANIMATION_FRAME_RATE, MIN_MODEL_SETUP_WIDTH, ModelPreviewData, ModelPreviewLayer,
    ModelPreviewState, ModelRegionSelection, ModelRenderMode, PHYSICS_REGION,
    PreviewAnimationEntry, PreviewAnimationPlayback, PreviewAnimationPose, RenderModelPreview,
    RenderModelPreviewBatch, RenderModelPreviewRegion, build_render_preview, frame,
    is_model_physics_overlay_region, load_referenced_tag_from_source, merge_preview_append,
    model_preview_data, reset_model_preview_selection, selected_variant_regions,
};
use super::animation::{
    advance_playback_clock, animation_header_group, draw_animation_combo, frame_tick_xs,
};
use super::model_preview_supports_textures;
use super::setup::{draw_model_setup_header_controls, model_setup_extra_header_height};
use super::variants::draw_variant_controls;
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
use blam_tags::TagFile;
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
                    offset = scroll.state.offset.y;
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

#[test]
fn model_setup_header_wraps_only_when_controls_need_room() {
    assert_eq!(model_setup_extra_header_height(399.0), 64.0);
    assert_eq!(model_setup_extra_header_height(400.0), 32.0);
    assert_eq!(
        model_setup_extra_header_height(559.0),
        MODEL_SETUP_EXTRA_HEADER_HEIGHT
    );
    assert_eq!(model_setup_extra_header_height(560.0), 0.0);
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
fn model_setup_header_groups_stay_inside_their_card() {
    let data = model_preview_data(
        String::new(),
        String::new(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    for width in [240.0, 300.0, 399.0, 400.0, 559.0, 560.0, 900.0] {
        let context = egui::Context::default();
        context.set_fonts(foundation_fonts());
        context.set_global_style(foundation_style());
        let mut state = ModelPreviewState::default();
        let _ = crate::app::run_ui_test(
            &context,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(width + 16.0, 400.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.set_width(width);
                    let mut actions_bounds = egui::Rect::NOTHING;
                    let mut actions_area = egui::Rect::NOTHING;
                    let card = draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                        if part == ModelPreviewSectionPart::Header {
                            actions_area = ui.max_rect();
                            draw_model_setup_header_controls(
                                ui,
                                width,
                                &data,
                                &mut state,
                                |ui, _| {
                                    icon_text_dropdown_button(ui, ButtonIcon::Save, "Save", |_| {});
                                    icon_text_button(ui, ButtonIcon::Garbage, "Delete", true);
                                },
                            );
                            actions_bounds = ui.min_rect();
                        }
                    });
                    assert!(
                        actions_bounds.right() <= actions_area.right() + 1.0,
                        "width {width}: controls {actions_bounds:?}, area {actions_area:?}"
                    );
                    assert!(
                        actions_bounds.bottom() <= actions_area.bottom() + 1.0,
                        "width {width}: controls {actions_bounds:?}, area {actions_area:?}"
                    );
                    assert!(card.width() <= width + 1.0);
                });
            },
        );
    }
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
        &state,
        &data.preview.regions[1]
    ));
    assert!(state.region_selections[PHYSICS_REGION].enabled);
    assert_eq!(selected_variant_regions(&data, &state).len(), 1);
    assert_eq!(
        selected_variant_regions(&data, &state)[0].region_name,
        "body"
    );

    state.overlays_loaded = false;
    assert!(!is_model_physics_overlay_region(
        &data,
        &state,
        &data.preview.regions[1]
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
fn variant_buttons_wrap_without_widening_a_narrow_setup_card() {
    let preview = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "helmet".into(),
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
    for width in [280.0, 360.0, 520.0] {
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
                            draw_variant_controls(ui, &data, &mut state);
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
                draw_variant_controls(ui, &data, &mut state);
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
        clamp_setup_width(f32::NAN, page.width()),
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
        let mut drawn = [egui::Rect::NOTHING; 3];
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
                    let setup_body = setup_body_height(rects.setup);
                    drawn[1] = in_rect(ui, rects.setup, "setup", |ui| {
                        draw_model_preview_section(
                            ui,
                            "Model Setup",
                            Some(setup_body),
                            |ui, part| {
                                if part == ModelPreviewSectionPart::Body {
                                    crate::app::ScrollArea::vertical()
                                        .auto_shrink([false, false])
                                        .max_height(setup_body)
                                        .show(ui, |ui| {
                                            ui.set_min_height(setup_body * 2.0);
                                        });
                                }
                            },
                        )
                    });
                    drawn[2] = in_rect(ui, rects.animation, "animation", |ui| {
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
        for (card, rect) in drawn
            .iter()
            .zip([rects.preview, rects.setup, rects.animation])
        {
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

/// A shading mode is picked from inside the View menu. It was a combo box
/// there, whose list is a popup of its own; egui 0.36 keeps one popup open at
/// a time, so opening the list closed the menu and nothing could be picked.
#[test]
fn a_shading_mode_is_picked_inside_the_view_menu() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::foundation_fonts());
    ctx.set_global_style(crate::app::foundation_style());
    let tag = TagFile::new(locate_definitions_root().join("halo3_mcc/render_model.json")).unwrap();
    let data = model_preview_data(
        "test".to_owned(),
        "test".to_owned(),
        RenderModelPreview::default(),
        Vec::new(),
    );
    let mut state = ModelPreviewState::default();
    let target = ModelRenderMode::ALL
        .into_iter()
        .find(|mode| *mode != state.render_mode)
        .expect("more than one mode");
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
                        &tag,
                        &data,
                        state,
                        true,
                        None,
                        Some(GameId::Halo3),
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
        .find(|(text, _)| text.starts_with("View:"))
        .unwrap_or_else(|| panic!("the View button: {shown:?}"))
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
    assert_eq!(state.render_mode, target, "the mode was picked");
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
                    None,
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
                            None,
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

/// The sidebar body, 360 points wide, drawn frame after frame.
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
                        super::functions::draw_setup_body(ui, data, state)
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
        egui::pos2(Self::WIDTH - ICON_BUTTON_SIZE.x / 2.0, row.center().y)
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
        equippable: vec![
            crate::app::model_preview::ce::weapons::CeEquippable::for_test("weapons\\rifle\\rifle"),
        ],
        hidden_clips: Default::default(),
    });
    data.ce.functions = Some(Default::default());
    data
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
    let mut state = ModelPreviewState::default();
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
        state.weapon_carry.is_some(),
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
