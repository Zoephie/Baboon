//! Function popup coordination and shared function-editor entry points.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

mod graph;
mod h2;
mod h3;
mod interaction;
mod history;

pub(in crate::app) use graph::*;
pub(in crate::app) use h2::*;
pub(in crate::app) use h3::*;
use interaction::*;
pub(in crate::app) use interaction::set_function_ranged;
pub(in crate::app) use history::InlineFunctionGesture;
use history::FunctionHistory;

/// Diff a view's current values against the last-applied snapshot and
/// build `PendingFieldEdit`s for the fields that changed. The blob is
/// hex-encoded into the string edit channel; wrapper fields use their
/// normal text representations.
pub(in crate::app) fn push_function_edit(
    paths: &FunctionEditPaths,
    prev: &FunctionSnapshot,
    view: &FunctionView,
) -> FunctionEditBatch {
    let mut edits = Vec::new();
    let mut data_ops = Vec::new();
    let data = view.data_bytes();
    if data != prev.data {
        match &paths.data {
            FunctionDataStorage::DataField(path) if !path.is_empty() => {
                edits.push(PendingFieldEdit {
                    path: path.clone(),
                    input: encode_hex(&data),
                });
            }
            FunctionDataStorage::Halo2ByteBlock(block_path) if !block_path.is_empty() => {
                data_ops.push(FunctionDataOp {
                    block_path: block_path.clone(),
                    data,
                });
            }
            _ => {}
        }
    }
    if view.output_index != prev.output_index && !paths.parameter_type.is_empty() {
        if let Some(index) = view.output_index {
            // Write the schema name (resolved by parse_enum_value) rather than
            // a raw integer, so the edit doesn't depend on wire-value order.
            let input = OUTPUT_TYPE_OPTIONS
                .iter()
                .find(|(value, _)| *value == index)
                .map(|(_, name)| (*name).to_owned())
                .unwrap_or_else(|| index.to_string());
            edits.push(PendingFieldEdit {
                path: paths.parameter_type.clone(),
                input,
            });
        }
    }
    if view.input_name != prev.input_name && !paths.input_name.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.input_name.clone(),
            input: if view.input_name.is_empty() {
                "none".to_owned()
            } else {
                view.input_name.clone()
            },
        });
    }
    if view.range_name != prev.range_name && !paths.range_name.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.range_name.clone(),
            input: if view.range_name.is_empty() {
                "none".to_owned()
            } else {
                view.range_name.clone()
            },
        });
    }
    if view.time_period_in_seconds != prev.time_period && !paths.time_period.is_empty() {
        edits.push(PendingFieldEdit {
            path: paths.time_period.clone(),
            input: view.time_period_in_seconds.to_string(),
        });
    }
    FunctionEditBatch {
        tag_key: String::new(),
        edits,
        data_ops,
    }
}

pub(in crate::app) fn draw_function_popup(
    ctx: &egui::Context,
    function_popup: &mut Option<FunctionPopup>,
    color_popup: &mut Option<MaterialColorPopup>,
) -> Option<FunctionEditBatch> {
    let popup = function_popup.as_mut()?;
    let mut close = false;
    let mut commit = false;
    let editable = popup.editable;
    let window_title = format!("Function Editor - {}", popup.title);
    egui::Window::new(&window_title)
        .constrain_to(window_work_area(ctx))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .default_size(window_size(ctx, Vec2::new(623.0, 440.0), false))
        .max_width(window_width(ctx, 623.0))
        .show(ctx, |ui| {
            let (header_rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 28.0), Sense::hover(),
            );
            ui.separator();
            if !editable {
                ui.label(
                    RichText::new("read-only (function has no writable path on this tag)")
                        .color(subtle_dark())
                        .small(),
                );
            }
            draw_function_editor(
                ui,
                &mut popup.view,
                editable,
                &mut popup.selected_graph,
                &mut popup.selected_point,
                color_popup,
            );
            popup.history.observe(&popup.view, ctx.input(|input| input.pointer.primary_down()));
            ui.add_space(4.0);
            ui.separator();
            ui.horizontal(|ui| {
                if icon_text_button(ui, ButtonIcon::Undo, "Undo", popup.can_step_history(false)).clicked() {
                    popup.step_history(false);
                }
                if icon_text_button(ui, ButtonIcon::Redo, "Redo", popup.can_step_history(true)).clicked() {
                    popup.step_history(true);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                        close = true;
                    }
                    if icon_text_button(ui, ButtonIcon::Confirm, "OK", true).clicked() {
                        commit = true;
                        close = true;
                    }
                });
            });
            // The editor can expand the content width. Paint the header only
            // after that width is known, using the frame's inner corner radius.
            let header_rect = egui::Rect::from_min_max(
                header_rect.min,
                egui::pos2(ui.min_rect().right(), header_rect.bottom()),
            );
            let mut header_ui = ui.new_child(egui::UiBuilder::new()
                .max_rect(header_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)));
            let border = ui.visuals().window_stroke.width.ceil() as u8;
            let rounding = &mut header_ui.visuals_mut().window_corner_radius;
            rounding.nw = rounding.nw.saturating_sub(border);
            rounding.ne = rounding.ne.saturating_sub(border);
            crate::app::search::draw_icon_window_header_without_close(
                &mut header_ui, &window_title, ButtonIcon::Function,
            );
        });

    // Commit edits only when OK is pressed. Live-writing while a modal is
    // open can invalidate classic H2 wrapper fields underneath combo boxes.
    let mut batch = None;
    if editable && commit {
        if let Some(paths) = popup.view.edit.clone() {
            let mut edits = push_function_edit(&paths, &popup.last_applied, &popup.view);
            if !edits.edits.is_empty() || !edits.data_ops.is_empty() {
                popup.last_applied = FunctionSnapshot::from_view(&popup.view);
                edits.tag_key = popup.tag_key.clone();
                batch = Some(FunctionEditBatch {
                    tag_key: edits.tag_key,
                    edits: edits.edits,
                    data_ops: edits.data_ops,
                });
            }
        }
    }

    if close {
        *function_popup = None;
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_window_header_and_footer_follow_the_final_content_bounds() {
        let ctx = egui::Context::default();
        let mut editor = TagFunctionEditor::from_function(
            h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap(),
        );
        editor.set_function_type(FunctionType::Spline).unwrap();
        let mut popup = Some(FunctionPopup::new(
            "test".to_owned(), "test_field".to_owned(),
            FunctionView::from_function(editor.into_function()), true,
        ));
        let mut frame = || crate::app::run_ui_test(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 1000.0))),
            ..Default::default()
        }, |_| { draw_function_popup(&ctx, &mut popup, &mut None); });
        frame();
        frame();
        let output = frame();
        fn flatten<'a>(shape: &'a egui::Shape, shapes: &mut Vec<&'a egui::Shape>) {
            if let egui::Shape::Vec(children) = shape {
                for child in children { flatten(child, shapes); }
            } else { shapes.push(shape); }
        }
        let mut shapes = Vec::new();
        for shape in &output.shapes { flatten(&shape.shape, &mut shapes); }
        let rectangles = shapes.iter().filter_map(|shape| match shape {
            egui::Shape::Rect(rect) => Some(rect), _ => None,
        }).collect::<Vec<_>>();
        let window = rectangles.iter().filter(|rect| rect.stroke.width > 0.0 && rect.rect.width() > 500.0)
            .max_by(|a, b| a.rect.area().total_cmp(&b.rect.area())).unwrap();
        let header = rectangles.iter().find(|rect| rect.rect.width() > 400.0
            && rect.rect.height() < 60.0 && (rect.rect.top() - window.rect.top()).abs() <= 2.0)
            .expect("header is painted at the top of the window");
        assert!((window.rect.right() - header.rect.right()).abs() <= 2.0);
        assert_eq!(header.corner_radius.nw, window.corner_radius.nw.saturating_sub(window.stroke.width.ceil() as u8));
        let content_bottom = rectangles.iter().filter(|rect|
            ((rect.rect.width() - 465.0).abs() < 1.0 && rect.rect.height() < 50.0)
            || ((rect.rect.width() - 150.0).abs() < 1.0 && rect.rect.height() < 160.0 && rect.rect.height() > 60.0)
            || ((rect.rect.width() - 623.0).abs() < 1.0 && rect.rect.height() > 30.0 && rect.rect.height() < 200.0)
        ).map(|rect| rect.rect.bottom()).max_by(f32::total_cmp).unwrap();
        let footer_separator = shapes.iter().filter_map(|shape| match shape {
            egui::Shape::LineSegment { points, .. } if points[1].x - points[0].x > 500.0 => Some(points[0].y),
            _ => None,
        }).max_by(f32::total_cmp).unwrap();
        assert!((0.0..=12.0).contains(&(footer_separator - content_bottom)), "footer {footer_separator} should follow content {content_bottom}; frames {:?}", rectangles.iter().map(|rect| rect.rect).collect::<Vec<_>>());
    }
}
