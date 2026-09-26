//! Function rows off screen are reserved, not built.
//!
//! Every function row carries a full read-only function editor as its
//! preview. The tests draw a column of them in a scroll area and compare what
//! the viewport shows with culling on against the same frames with it off.

use eframe::egui;

use super::{FUNCTION_PREVIEWS_BUILT, FUNCTION_ROWS_CULLED, draw_foundation_function_row};
use crate::app::FieldDisplayMeta;
use crate::app::foundation::extracted_tests::tests::with_test_edit_context;
use blam_tags::{FunctionType, H2Function, TagFunction};

fn function(kind: FunctionType) -> TagFunction {
    TagFunction::H2(H2Function::new(kind))
}

fn meta(label: String) -> FieldDisplayMeta {
    FieldDisplayMeta {
        label,
        unit: None,
        range: None,
        help: None,
        tag_reference_allowed: Vec::new(),
        read_only: false,
        advanced: false,
    }
}

struct Frame {
    /// (row index, top) of each row the clip rect reaches.
    visible: Vec<(usize, f32)>,
    previews: usize,
    content_height: f32,
}

struct Column {
    ctx: egui::Context,
    functions: Vec<TagFunction>,
    culled: bool,
}

impl Column {
    fn new(functions: Vec<TagFunction>, culled: bool) -> Self {
        Self {
            ctx: egui::Context::default(),
            functions,
            culled,
        }
    }

    fn frame(&self, offset: f32) -> Frame {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            ..Default::default()
        };
        FUNCTION_ROWS_CULLED.with(|culled| culled.set(self.culled));
        FUNCTION_PREVIEWS_BUILT.with(|count| count.set(0));
        let mut tops = Vec::new();
        let mut viewport = egui::Rect::NOTHING;
        let mut content_height = 0.0;
        let _ = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let output = egui::ScrollArea::vertical()
                    .vertical_scroll_offset(offset)
                    .show(ui, |ui| {
                        with_test_edit_context(|edit| {
                            for (index, function) in self.functions.iter().enumerate() {
                                tops.push((index, ui.cursor().top()));
                                let path = format!("functions[{index}]/function");
                                let meta = meta(format!("function {index}"));
                                draw_foundation_function_row(ui, &meta, function, 0, &path, edit);
                            }
                        });
                    });
                // The content's clip rect, which egui widens past the
                // viewport by `clip_rect_margin`.
                let margin = ui.visuals().clip_rect_margin;
                viewport = output.inner_rect.expand(margin);
                content_height = output.content_size.y;
            });
        });
        FUNCTION_ROWS_CULLED.with(|culled| culled.set(true));
        let visible = tops
            .windows(2)
            .map(|pair| (pair[0], pair[1].1))
            .chain(tops.last().map(|&last| (last, last.1 + 400.0)))
            .filter(|((_, top), bottom)| *bottom > viewport.top() && *top < viewport.bottom())
            .map(|(row, _)| row)
            .collect();
        Frame {
            visible,
            previews: FUNCTION_PREVIEWS_BUILT.with(|count| count.get()),
            content_height,
        }
    }
}

fn assert_same_view(culled: &Frame, full: &Frame, context: &str) {
    assert!(
        !full.visible.is_empty(),
        "{context}: the viewport showed nothing"
    );
    assert_eq!(
        culled.content_height, full.content_height,
        "{context}: culling changed the column's height"
    );
    assert_eq!(
        culled.visible, full.visible,
        "{context}: culling moved a visible row"
    );
}

fn mixed_functions() -> Vec<TagFunction> {
    (0..60)
        .map(|index| {
            function(if index % 3 == 0 {
                FunctionType::Constant
            } else {
                FunctionType::MultiLinearKey
            })
        })
        .collect()
}

#[test]
fn off_screen_function_rows_are_not_built() {
    let culled = Column::new(mixed_functions(), true);
    let full = Column::new(mixed_functions(), false);
    let height = full.frame(0.0).content_height;
    for offset in [0.0, 2_500.0, height / 2.0, height - 800.0, 0.0] {
        for pass in 0..2 {
            let context = format!("offset {offset}, frame {pass}");
            let (culled, full) = (culled.frame(offset), full.frame(offset));
            assert_same_view(&culled, &full, &context);
            assert_eq!(
                full.previews, 60,
                "{context}: the reference built every preview"
            );
            if pass == 1 {
                assert!(
                    culled.previews <= culled.visible.len(),
                    "{context}: built {} previews to show {} rows",
                    culled.previews,
                    culled.visible.len()
                );
            }
        }
    }
}

/// The f() popup edits a function while its row may be off screen. The
/// height cached for the old function must not be reserved for the new one.
#[test]
fn a_function_changed_off_screen_is_measured_again() {
    let mut culled = Column::new(mixed_functions(), true);
    let mut full = Column::new(mixed_functions(), false);
    let height = full.frame(0.0).content_height;
    culled.frame(0.0);
    culled.frame(height - 800.0);
    let before = full.frame(0.0).content_height;
    for column in [&mut culled, &mut full] {
        for function in column.functions.iter_mut().take(10) {
            *function = self::function(FunctionType::Constant);
        }
    }
    full.frame(height - 800.0);
    assert_ne!(
        full.frame(height - 800.0).content_height,
        before,
        "the change does not alter any row's height, so it tests nothing"
    );
    for pass in 0..2 {
        let offset = full.frame(0.0).content_height - 800.0;
        assert_same_view(
            &culled.frame(offset),
            &full.frame(offset),
            &format!("after the change, frame {pass}"),
        );
    }
}
