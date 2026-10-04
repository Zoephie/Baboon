use super::*;

#[test]
fn welcome_left_background_reaches_taller_column_bottom() {
    for taller_left in [false, true] {
        let ctx = egui::Context::default();
        let mut bottom = 0.0;
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(820.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    draw_welcome_columns(ui, |columns| {
                        for (index, column) in columns.iter_mut().enumerate() {
                            let height = if (index == 0) == taller_left {
                                400.0
                            } else {
                                120.0
                            };
                            column.allocate_exact_size(
                                Vec2::new(column.available_width(), height),
                                Sense::hover(),
                            );
                        }
                        bottom = columns
                            .iter()
                            .map(|column| column.min_rect().bottom())
                            .fold(0.0, f32::max);
                    });
                });
            },
        );
        let background = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == Color32::from_black_alpha(51) => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .expect("missing left pane background");
        assert_eq!(background.bottom(), bottom);
    }
}

/// egui 0.36 aligns a button's icon and label by the enclosing layout,
/// and a horizontal row centres them; welcome rows keep them at the left.
#[test]
fn welcome_rows_keep_their_icon_and_label_at_the_left() {
    for horizontal in [false, true] {
        let ctx = egui::Context::default();
        let mut button = egui::Rect::NOTHING;
        let output = crate::app::run_ui_test(
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
                    let row = |ui: &mut Ui| {
                        ui.set_width(300.0);
                        button = welcome_icon_button(
                            ui,
                            ButtonIcon::FolderClosed,
                            "halo3_mcc",
                            text_dark(),
                        )
                        .rect;
                    };
                    if horizontal {
                        ui.horizontal(row);
                    } else {
                        ui.vertical(row);
                    }
                });
            },
        );
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "halo3_mcc" => {
                    Some(text.visual_bounding_rect())
                }
                _ => None,
            })
            .expect("missing row label");
        assert!(
            label.left() - button.left() < 48.0,
            "label starts {} px into a {} px row (horizontal: {horizontal})",
            label.left() - button.left(),
            button.width(),
        );
        assert!(button.y_range().contains(label.center().y));
    }
}
