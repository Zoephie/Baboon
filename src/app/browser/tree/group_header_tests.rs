use super::*;

#[test]
fn group_tree_label_splits_friendly_name_and_fourcc() {
    assert_eq!(group_tree_label_parts("control cntl"), ("control", "cntl"));
    assert_eq!(group_tree_label_parts("bloc"), ("", "bloc"));
}

#[test]
fn folder_header_hover_target_spans_the_available_row() {
    let ctx = egui::Context::default();
    let mut expected = egui::Rect::NOTHING;
    let mut actual = egui::Rect::NOTHING;
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(360.0, 100.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                expected = ui.available_rect_before_wrap();
                actual = show_folder_tree_header(
                    ui,
                    "characters",
                    "characters",
                    text_dark(),
                    false,
                    false,
                    |_| {},
                )
                .rect;
            });
        },
    );

    assert_eq!(actual.left(), expected.left());
    assert_eq!(actual.right(), expected.right());
}

#[test]
fn favorites_header_hover_target_spans_the_available_row() {
    let ctx = egui::Context::default();
    let mut expected = egui::Rect::NOTHING;
    let mut actual = egui::Rect::NOTHING;
    let _ = crate::app::run_ui_test(
        &ctx,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(360.0, 100.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                expected = ui.available_rect_before_wrap();
                actual = show_favorites_section(ui, |_| {}).rect;
            });
        },
    );

    assert_eq!(actual.left(), expected.left());
    assert_eq!(actual.right(), expected.right());
}

#[test]
fn nested_folder_headers_keep_their_own_guides_enabled() {
    let ctx = egui::Context::default();
    let mut nested_guide_enabled = false;
    let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            begin_folder_chevron_collection(ui);
            show_folder_tree_header(ui, "outer", "outer", text_dark(), true, true, |ui| {
                nested_guide_enabled = ui.visuals().indent_has_left_vline;
                show_folder_tree_header(ui, "inner", "inner", text_dark(), true, true, |_| {});
            });
        });
    });

    assert!(nested_guide_enabled);
}

#[test]
fn folder_guide_is_split_instead_of_painted_over() {
    let shapes = guide_segments_around_cutouts(
        10.0,
        0.0,
        40.0,
        &[egui::Rect::from_min_max(
            egui::pos2(5.0, 14.0),
            egui::pos2(15.0, 26.0),
        )],
        Stroke::new(1.0_f32, Color32::WHITE),
    );

    assert_eq!(shapes.len(), 2);
}

#[test]
fn nested_chevron_center_matches_parent_icon_guide() {
    let ctx = egui::Context::default();
    let mut delta = f32::INFINITY;
    let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let parent_icon_center = ui.spacing().indent
                + ui.spacing().item_spacing.x
                + BROWSER_TREE_ICON_SIZE * 0.5;
            let nested_chevron_center = ui.spacing().indent
                + ui.spacing().indent * 0.5
                + browser_chevron_center_offset(ui);
            delta = nested_chevron_center - (parent_icon_center + BROWSER_GUIDE_ICON_OFFSET);
        });
    });

    assert!(delta.abs() < f32::EPSILON);
}
