use super::*;

#[test]
fn editing_kit_inputs_match_button_height() {
    let ctx = egui::Context::default();
    ctx.set_global_style(foundation_style());
    let _ = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut value = String::from("Editing kit");
            for interactive in [true, false] {
                let top = ui.next_widget_position().y;
                let response =
                    ui.add(editing_kit_text_input(&mut value, 200.0).interactive(interactive));
                // egui 0.36: TextEdit's response rect includes its frame margins.
                assert_eq!(response.rect.height(), 24.0);
                assert_eq!(
                    ui.next_widget_position().y - top - ui.spacing().item_spacing.y,
                    24.0
                );
            }
            let engine = egui::ComboBox::from_id_salt("height_test_engine")
                .selected_text("Halo 2")
                .show_ui(ui, |_| {});
            assert_eq!(engine.response.rect.height(), 24.0);
        });
    });
}

#[test]
fn editing_kit_read_only_policy_tracks_profiles_and_excludes_campaign_evolved() {
    let mut profile = CustomEditingKitProfile {
        read_only: true,
        git_tracked: false,
        id: "read-only-kit".to_owned(),
        name: "Protected kit".to_owned(),
        game: "halo2_mcc".to_owned(),
        root: PathBuf::from("C:/Kits/Protected"),
        icon: None,
        tags_folder: None,
        data_folder: None,
    };
    let identity = EditingKitProfileIdentity {
        id: profile.id.clone(),
        name: profile.name.clone(),
    };
    assert!(profile.is_read_only_for(Some(&identity), None));
    assert!(profile.is_read_only_for(None, Some(&profile.root.join("tags"))));
    assert!(profile.is_read_only_for(
        None,
        Some(&profile.root.join("tags/objects/example.weapon"))
    ));
    assert!(!profile.is_read_only_for(None, Some(Path::new("C:/Kits/Other"))));
    // A Halo 2 kit can share its root with another kit using another tags
    // folder; this kit's read-only setting doesn't reach that one.
    assert!(!profile.is_read_only_for(None, Some(&profile.root.join("tags_moda"))));
    let mut halo3 = profile.clone();
    halo3.game = "halo3_mcc".to_owned();
    assert!(halo3.is_read_only_for(None, Some(&halo3.root)));
    assert!(halo3.is_read_only_for(None, Some(&halo3.root.join("tags_moda"))));
    assert!(CustomEditingKitDraft::from_profile(&profile).read_only);
    profile.git_tracked = true;
    assert!(CustomEditingKitDraft::from_profile(&profile).git_tracked);
    profile.read_only = false;
    assert!(!profile.is_read_only_for(Some(&identity), None));
    profile.read_only = true;
    profile.game = "haloce_evolved".to_owned();
    assert!(!profile.is_read_only_for(Some(&identity), Some(&profile.root)));

    let ctx = egui::Context::default();
    ctx.set_global_style(foundation_style());
    for game in ["halo2_mcc", "haloce_mcc", "halo3_mcc", "haloce_evolved"] {
        let mut draft = CustomEditingKitDraft::new();
        draft.game = game.to_owned();
        let output = crate::app::run_ui_test(&ctx, Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_editing_kit_form(ui, &mut draft, None);
            });
        });
        let checkbox_visible = output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Read-Only")
        });
        assert_eq!(checkbox_visible, game != "haloce_evolved");
        let git_checkbox_visible = output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Tracked in Git")
        });
        assert_eq!(git_checkbox_visible, game != "haloce_evolved");
        // Only kits whose tools take -tags_dir/-data_dir offer the folders.
        let folders_visible = output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Tags Folder")
        });
        assert_eq!(
            folders_visible,
            matches!(game, "haloce_mcc" | "halo2_mcc"),
            "{game}"
        );
    }
}

/// Choosing a root fills the folders the user hasn't chosen with the root's
/// own `tags` and `data`; a folder the user picked survives a root change.
#[test]
fn choosing_a_root_fills_only_the_folders_still_on_auto() {
    let outer = crate::test_kits::unique_temp_dir("kit-folder-autofill");
    let first = outer.join("H2EK");
    let second = outer.join("H2EK-copy");
    for root in [&first, &second] {
        for folder in ["tags", "Data", "tags_moda"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
    }
    let mut draft = CustomEditingKitDraft::new();
    draft.game = "halo2_mcc".to_owned();
    draft.root_input = first.display().to_string();
    refill_kit_folders(&mut draft);
    let filled = (
        draft.tags_folder_input.clone(),
        draft.data_folder_input.clone(),
    );

    draft.tags_folder_input = "tags_moda".to_owned();
    draft.tags_folder_auto = false;
    draft.root_input = second.display().to_string();
    refill_kit_folders(&mut draft);
    let after_user_pick = draft.tags_folder_input.clone();

    let mut halo3 = CustomEditingKitDraft::new();
    halo3.game = "halo3_mcc".to_owned();
    halo3.root_input = first.display().to_string();
    refill_kit_folders(&mut halo3);
    let _ = std::fs::remove_dir_all(&outer);

    assert_eq!(filled, ("tags".to_owned(), "Data".to_owned()));
    assert_eq!(after_user_pick, "tags_moda");
    assert!(halo3.tags_folder_input.is_empty() && halo3.data_folder_input.is_empty());
}

#[test]
fn editing_kit_form_preview_tracks_draft_name_and_keeps_fields_inside_dialog() {
    let ctx = egui::Context::default();
    ctx.set_global_style(foundation_style());
    egui_extras::install_image_loaders(&ctx);
    let mut draft = CustomEditingKitDraft::new();
    draft.root_input = r"C:\Program Files (x86)\Steam\steamapps\common\H2EK".to_owned();
    for name in ["Halo 2: Rebalance", "Renamed kit"] {
        draft.name = name.to_owned();
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    // Tall enough for the Halo 2 form's folder rows; the
                    // dialog scrolls when a screen is shorter.
                    Vec2::new(600.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let right = ui.max_rect().right();
                    let actions = draw_editing_kit_form(ui, &mut draft, None);
                    assert!(!actions.save && !actions.cancel && !actions.remove);
                    assert!(ui.min_rect().right() <= right + 1.0, "form fields overflow");
                    assert!(
                        ui.next_widget_position().y < 880.0,
                        "form unexpectedly fills height"
                    );
                });
            },
        );
        assert!(
            output.shapes.iter().any(|shape| matches!(
                &shape.shape, egui::Shape::Text(text) if text.galley.text() == name
                    && text.galley.job.sections.iter().all(|section| section.format.font_id.size == 14.0)
            )),
            "live preview did not display changed name"
        );
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Rect(rect) if rect.fill == foundation_documentation_bg()
        )), "preview is missing its translucent card fill");
    }
    assert!(
        draft_editing_kit_icon_texture(&ctx, &CustomEditingKitIconDraft::Default).is_none()
    );
    let icon = CustomEditingKitIconDraft::Selected(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/Game Icons/h2.png"),
    );
    let texture =
        draft_editing_kit_icon_texture(&ctx, &icon).expect("selected PNG has no preview");
    let cached = draft_editing_kit_icon_texture(&ctx, &icon).unwrap();
    assert_eq!(texture.id(), cached.id());
    assert!(
        draft_editing_kit_icon_texture(&ctx, &CustomEditingKitIconDraft::Default).is_none()
    );
}

#[test]
fn editing_kit_action_columns_match_shared_button_sizes_at_high_dpi() {
    for scale in [1.0, 2.0, 3.0] {
        let ctx = egui::Context::default();
        ctx.set_global_style(foundation_style());
        ctx.set_pixels_per_point(scale);
        egui_extras::install_image_loaders(&ctx);
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(400.0, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    for (icon, label) in
                        [(ButtonIcon::Open, "Open"), (ButtonIcon::Edit, "Edit")]
                    {
                        let reserved = editing_kit_action_width(ui, label);
                        let button = icon_text_button(ui, icon, label, true);
                        assert!((button.rect.width() - reserved).abs() <= 1.0);
                        assert_eq!(button.rect.height(), BUTTON_HEIGHT);
                        assert!(
                            button.rect.width() < 80.0,
                            "button still has oversized fixed width"
                        );
                    }
                });
            },
        );
    }
}

fn settings_frame(
    ctx: &egui::Context,
    tab: SettingsTab,
    events: Vec<egui::Event>,
) -> egui::Rect {
    let mut rect = egui::Rect::NOTHING;
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1200.0, 900.0),
            )),
            events,
            ..Default::default()
        },
        |_| {
            rect = egui::Window::new("Settings")
                .id(egui::Id::new("settings_resize_test"))
                .title_bar(false)
                .collapsible(false)
                .resizable(true)
                .default_pos(egui::pos2(100.0, 100.0))
                .default_size(window_size(ctx, Vec2::new(760.0, 400.0), false))
                .show(ctx, |ui| {
                    let mut open = true;
                    let mut selected = tab;
                    settings_window_body(ui, &mut open, &mut selected, |ui, _| {
                        ui.label("Settings content");
                    });
                })
                .unwrap()
                .response
                .rect;
        },
    );
    rect
}

#[test]
fn settings_horizontal_edge_resize_does_not_grow_height() {
    for tab in [
        SettingsTab::Startup,
        SettingsTab::Browser,
        SettingsTab::EditingKits,
        SettingsTab::Appearance,
        SettingsTab::Tools,
    ] {
        for right_edge in [false, true] {
            let ctx = egui::Context::default();
            let mut rect = settings_frame(&ctx, tab, vec![]);
            for _ in 0..4 {
                rect = settings_frame(&ctx, tab, vec![]);
            }
            let initial = rect;
            assert!(
                initial.height() < 500.0,
                "settings unexpectedly expanded to full height"
            );
            let start = egui::pos2(
                if right_edge {
                    rect.right() - 1.0
                } else {
                    rect.left() + 1.0
                },
                rect.center().y,
            );
            settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(start)]);
            settings_frame(
                &ctx,
                tab,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let mut end = start;
            for step in 1..=8 {
                end =
                    start + Vec2::new(if right_edge { -10.0 } else { 10.0 } * step as f32, 0.0);
                rect = settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(end)]);
                assert!(
                    (rect.height() - initial.height()).abs() <= 1.0,
                    "horizontal resize changed height: {} -> {}",
                    initial.height(),
                    rect.height()
                );
            }
            settings_frame(
                &ctx,
                tab,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            rect = settings_frame(&ctx, tab, vec![]);
            assert!(
                (rect.width() - initial.width()).abs() > 20.0,
                "edge drag did not resize"
            );
            assert!((rect.height() - initial.height()).abs() <= 1.0);
        }
    }
}

#[test]
fn settings_window_can_shrink_vertically() {
    for tab in [
        SettingsTab::Startup,
        SettingsTab::Browser,
        SettingsTab::EditingKits,
        SettingsTab::Appearance,
        SettingsTab::Tools,
    ] {
        let ctx = egui::Context::default();
        let mut rect = settings_frame(&ctx, tab, vec![]);
        for _ in 0..4 {
            rect = settings_frame(&ctx, tab, vec![]);
        }
        let initial = rect;
        assert!(
            initial.height() < 500.0,
            "settings unexpectedly expanded to full height"
        );
        let start = egui::pos2(rect.center().x, rect.bottom() - 1.0);
        settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(start)]);
        settings_frame(
            &ctx,
            tab,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let end = start - Vec2::new(0.0, 100.0);
        settings_frame(&ctx, tab, vec![egui::Event::PointerMoved(end)]);
        settings_frame(
            &ctx,
            tab,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        rect = settings_frame(&ctx, tab, vec![]);
        assert!(
            rect.height() < initial.height() - 80.0,
            "vertical resize is locked: {} -> {}",
            initial.height(),
            rect.height()
        );
    }
}

#[test]
fn editing_kit_reordering_moves_entries_before_and_after_without_changing_identity() {
    let mut profiles: Vec<_> = ["one", "two", "three"]
        .into_iter()
        .map(|id| CustomEditingKitProfile {
            read_only: false,
            git_tracked: false,
            id: id.to_owned(),
            name: id.to_owned(),
            game: "halo2_mcc".to_owned(),
            root: PathBuf::from(format!("C:/Kits/{id}")),
            icon: None,
            tags_folder: None,
            data_folder: None,
        })
        .collect();
    let original = profiles.clone();
    assert!(reorder_editing_kit_profiles(
        &mut profiles,
        &EditingKitReorderRequest {
            source: "one".to_owned(),
            target: "three".to_owned(),
            after: true,
        }
    ));
    assert_eq!(
        profiles,
        vec![
            original[1].clone(),
            original[2].clone(),
            original[0].clone()
        ]
    );
    assert!(reorder_editing_kit_profiles(
        &mut profiles,
        &EditingKitReorderRequest {
            source: "one".to_owned(),
            target: "two".to_owned(),
            after: false,
        }
    ));
    assert_eq!(profiles, original);
    assert!(!reorder_editing_kit_profiles(
        &mut profiles,
        &EditingKitReorderRequest {
            source: "one".to_owned(),
            target: "two".to_owned(),
            after: false,
        }
    ));
    let validation = EditingKitValidationCache::new(&HashMap::new(), &profiles);
    let entries = visible_editing_kit_menu_entries(&profiles, &validation);
    assert_eq!(entries.len(), 3);
    for (entry, profile) in entries.iter().zip(&profiles) {
        assert!(matches!(entry, EditingKitMenuEntry::Custom(entry) if entry.id == profile.id));
    }
}

#[test]
fn editing_kit_grabber_drag_produces_reorder_request() {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    let mut positions = [egui::Pos2::ZERO; 2];
    let mut frame = |events| {
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(360.0, 240.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    for (index, id) in ["one", "two"].into_iter().enumerate() {
                        let top = ui.next_widget_position();
                        positions[index] = top + Vec2::new(9.0, 22.0);
                        ui.push_id(id, |ui| {
                            editing_kit_card(
                                ui,
                                id,
                                Path::new("C:/Kit"),
                                None,
                                None,
                                None,
                                Some(id),
                            );
                        });
                    }
                });
            },
        );
        positions
    };
    let positions = frame(vec![]);
    let start = positions[0];
    frame(vec![egui::Event::PointerMoved(start)]);
    frame(vec![egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    frame(vec![egui::Event::PointerMoved(
        start + Vec2::new(0.0, 10.0),
    )]);
    let end = positions[1] + Vec2::new(80.0, 12.0);
    frame(vec![egui::Event::PointerMoved(end)]);
    frame(vec![egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let request = ctx
        .data(|data| {
            data.get_temp::<EditingKitReorderRequest>(egui::Id::new(
                "editing_kit_reorder_request",
            ))
        })
        .expect("grabber drag did not produce a reorder request");
    assert_eq!(request.source, "one");
    assert_eq!(request.target, "two");
    assert!(request.after);
}

#[test]
fn editing_kit_cards_fit_settings_widths_with_long_paths() {
    for width in [360.0, 760.0] {
        for error in [None, Some("Folder not found")] {
            let ctx = egui::Context::default();
            egui_extras::install_image_loaders(&ctx);
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let right = ui.max_rect().right();
                        let first_top = ui.next_widget_position().y;
                        assert_eq!(
                            editing_kit_card(
                                ui,
                                "My Halo 2 editing kit",
                                Path::new(
                                    r"C:\Program Files (x86)\Steam\steamapps\common\H2EK"
                                ),
                                None,
                                error,
                                None,
                                Some("first-kit"),
                            ),
                            (false, false, false)
                        );
                        let second_top = ui.next_widget_position().y;
                        editing_kit_card(
                            ui,
                            "Second kit",
                            Path::new(r"C:\H2EK"),
                            None,
                            error,
                            Some("Custom image unavailable"),
                            Some("second-kit"),
                        );
                        let third_top = ui.next_widget_position().y;
                        let first_height = second_top - first_top;
                        let second_height = third_top - second_top;
                        assert!(
                            first_height <= 60.0,
                            "first row is too tall: {first_height}"
                        );
                        assert!(
                            (first_height - second_height).abs() <= 1.0,
                            "row heights differ: {first_height} vs {second_height}"
                        );
                        assert!(
                            ui.min_rect().right() <= right + 1.0,
                            "editing kit card overflows at width {width}"
                        );
                    });
                },
            );
            for label in ["Open", "Edit"] {
                let positions: Vec<f32> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            (text.galley.text() == label).then_some(text.pos.x)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(positions.len(), 2, "missing {label} buttons");
                assert!(
                    (positions[0] - positions[1]).abs() <= 1.0,
                    "{label} buttons are not column-aligned"
                );
            }
        }
    }
}
