use super::*;
use crate::core::game::GameId;
use super::model_preview_supports_textures;

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
    assert_eq!(model_setup_extra_header_height(559.0), 64.0);
    assert_eq!(model_setup_extra_header_height(560.0), 32.0);
    assert_eq!(
        model_setup_extra_header_height(679.0),
        MODEL_SETUP_EXTRA_HEADER_HEIGHT
    );
    assert_eq!(model_setup_extra_header_height(680.0), 0.0);
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
    for width in [400.0, 559.0, 560.0, 679.0, 680.0, 900.0] {
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
                    let card =
                        draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
                            if part == ModelPreviewSectionPart::Header {
                                actions_area = ui.max_rect();
                                draw_model_setup_header_controls(
                                    ui,
                                    width,
                                    &data,
                                    &mut state,
                                    |ui, _| {
                                        icon_text_dropdown_button(
                                            ui,
                                            ButtonIcon::Save,
                                            "Save",
                                            |_| {},
                                        );
                                        icon_text_button(
                                            ui,
                                            ButtonIcon::Garbage,
                                            "Delete",
                                            true,
                                        );
                                        icon_text_button(
                                            ui,
                                            ButtonIcon::Refresh,
                                            "Refresh Model",
                                            true,
                                        );
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
                ..Default::default()
            },
            RenderModelPreviewBatch {
                region_name: PHYSICS_REGION.into(),
                layer: ModelPreviewLayer::Physics,
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
    assert_eq!(
        renderer::visible_batch_indices(&preview, &state),
        [0, 2, 4, 5]
    );

    // A matching collision permutation wins over the fallback.
    select(&mut state, "hull", "broken");
    assert_eq!(renderer::visible_batch_indices(&preview, &state), [0, 2, 6]);

    // A disabled region hides its overlay too.
    state
        .region_selections
        .get_mut("__unnamed")
        .unwrap()
        .enabled = false;
    assert_eq!(renderer::visible_batch_indices(&preview, &state), [6]);

    // A standalone tag's batches are the primary preview: exact match only.
    state.overlays_loaded = false;
    select(&mut state, "__unnamed", "monitor");
    assert_eq!(renderer::visible_batch_indices(&preview, &state), [0, 6]);
}

/// The monitor biped against `BLAM_TEST_HCEEK` (the kit's `tags`): its
/// gbxmodel and collision model share the `__unnamed` region but no
/// permutation name, which hid the collision layer entirely.
#[test]
fn halo_ce_monitor_collision_overlay_is_drawn() {
    let tags = std::path::PathBuf::from(crate::core::test_kits::tag_path("haloce_mcc", ""));
    let biped = tags.join("characters/monitor/monitor.biped");
    if !biped.is_file() {
        eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
        return;
    }
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
    let collision =
        halo1_object_collision_overlay(&object, &source).expect("collision overlay");
    merge_preview_append(&mut preview, &collision);
    let data = model_preview_data(String::new(), String::new(), preview, Vec::new());
    let mut state = ModelPreviewState {
        overlays_loaded: true,
        show_render: true,
        show_collision: true,
        ..Default::default()
    };
    reset_model_preview_selection(&mut state, &data, None);

    let visible = renderer::visible_batch_indices(&data.preview, &state);
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
    let render_only =
        model_preview_data(String::new(), String::new(), render_only, Vec::new());
    let with_collision =
        model_preview_data(String::new(), String::new(), with_collision, Vec::new());
    let mut state = ModelPreviewState::default();
    let collision = ModelPreviewLayer::Collision;

    assert!(overlay_layer_available(&render_only, &state, collision), "unknown yet");
    state.overlays_loaded = true;
    assert!(!overlay_layer_available(&render_only, &state, collision));
    assert!(overlay_layer_available(&with_collision, &state, collision));
}

#[test]
fn viewport_percentage_conversion_clamps_to_persisted_range() {
    assert_eq!(model_preview_size_percent(1.25), 125.0);
    assert_eq!(model_preview_size_from_percent(125.0), 1.25);
    assert_eq!(
        model_preview_size_from_percent(20.0),
        MIN_MODEL_PREVIEW_SIZE
    );
    assert_eq!(
        model_preview_size_from_percent(400.0),
        MAX_MODEL_PREVIEW_SIZE
    );
}

#[test]
fn viewport_shrinks_without_changing_aspect_ratio() {
    let size = model_viewport_size(200.0, 1.0);
    assert_eq!(size.x, 200.0);
    assert!((size.x / size.y - 470.0 / 300.0).abs() < 0.000_001);
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
                    card_rect =
                        draw_model_preview_section(ui, "Model Setup", None, |ui, part| {
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
                                            ui.add_sized(
                                                ICON_BUTTON_SIZE,
                                                egui::Button::new(""),
                                            );
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
                                ui.add(
                                    egui::Slider::new(&mut frame, 0.0..=9.0).show_value(false),
                                );
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

#[test]
fn preview_scale_resizes_the_wide_section_until_setup_reaches_its_minimum() {
    assert_eq!(wide_model_preview_section_width(1_600.0, 1.0), 470.0);
    assert_eq!(wide_model_preview_section_width(1_600.0, 1.5), 705.0);
    assert_eq!(
        wide_model_preview_section_width(1_000.0, 2.0),
        1_000.0 - MODEL_PREVIEW_SECTION_GAP - WIDE_MODEL_SETUP_MIN_WIDTH
    );
}

#[test]
fn preview_and_setup_cards_match_outer_height_at_both_header_breakpoints() {
    for setup_width in [360.0, 800.0] {
        for scale in [1.0, 1.5] {
            let context = egui::Context::default();
            context.set_fonts(foundation_fonts());
            let viewport = model_viewport_size(470.0 * scale, scale);
            let shared_body_height = viewport.y + MODEL_PREVIEW_STATS_FOOTER_HEIGHT;
            let setup_body_height =
                shared_body_height - model_setup_extra_header_height(setup_width) - 16.0;
            let mut preview_rect = egui::Rect::NOTHING;
            let mut setup_rect = egui::Rect::NOTHING;
            let _ = crate::app::run_ui_test(
                &context,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(2_000.0, 1_200.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui(Vec2::new(viewport.x, 0.0), |ui| {
                                ui.set_width(viewport.x);
                                preview_rect = draw_model_preview_section(
                                    ui,
                                    "Model Preview",
                                    Some(shared_body_height),
                                    |ui, part| {
                                        if part == ModelPreviewSectionPart::Body {
                                            ui.allocate_exact_size(viewport, Sense::hover());
                                            ui.allocate_exact_size(
                                                Vec2::new(
                                                    viewport.x,
                                                    MODEL_PREVIEW_STATS_FOOTER_HEIGHT,
                                                ),
                                                Sense::hover(),
                                            );
                                        }
                                    },
                                );
                            });
                            ui.allocate_ui(Vec2::new(setup_width, 0.0), |ui| {
                                ui.set_width(setup_width);
                                setup_rect = draw_model_preview_section(
                                    ui,
                                    "Model Setup",
                                    Some(setup_body_height),
                                    |ui, part| {
                                        if part == ModelPreviewSectionPart::Body {
                                            egui::ScrollArea::vertical()
                                                .auto_shrink([false, false])
                                                .max_height(setup_body_height)
                                                .show(ui, |ui| {
                                                    ui.set_min_height(setup_body_height * 2.0);
                                                });
                                        }
                                    },
                                );
                            });
                        });
                    });
                },
            );
            assert!(
                (preview_rect.height() - setup_rect.height()).abs() < 1.0,
                "scale {scale}, setup width {setup_width}: preview={}, setup={}",
                preview_rect.height(),
                setup_rect.height()
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
                        draw_model_preview_section(ui, "Model Preview", None, |ui, part| {
                            match part {
                                ModelPreviewSectionPart::Header => {
                                    header_rect = Some(ui.max_rect())
                                }
                                ModelPreviewSectionPart::Body => {
                                    body_rect = Some(ui.max_rect())
                                }
                            }
                        });
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
fn campaign_evolved_preview_defaults_to_full_detail() {
    assert!(ModelPreviewState::default().high_detail);
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
        let _ = crate::app::run_ui_test(&ctx, 
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let id = ui
                        .make_persistent_id(("model_animation_popup", "test_animation_picker"));
                    egui::Popup::open_id(ui.ctx(), id);
                    popup_id = Some(id);
                    draw_animation_combo(
                        ui,
                        "test_animation_picker",
                        &animations,
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

#[test]
fn textured_shading_is_limited_to_supported_editing_kits() {
    assert!(model_preview_supports_textures(Some(GameId::Halo3)));
    assert!(model_preview_supports_textures(Some(GameId::HaloReach)));
    assert!(model_preview_supports_textures(Some(GameId::Halo2)));
    assert!(model_preview_supports_textures(Some(GameId::HaloCe)));
    for game in [
        None,
        Some(GameId::Halo3Odst),
        Some(GameId::Halo4),
        Some(GameId::Halo2Amp),
        Some(GameId::CampaignEvolved),
    ] {
        assert!(!model_preview_supports_textures(game), "{game:?}");
    }
}
