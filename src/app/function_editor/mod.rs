//! Function popup coordination and shared function-editor entry points.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

mod graph;
mod h2;
mod h3;

pub(super) use graph::*;
pub(super) use h2::*;
pub(super) use h3::*;

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
    let mut open = true;
    let mut close = false;
    let mut commit = false;
    let editable = popup.editable;
    egui::Window::new(popup.title.clone())
        .collapsible(false)
        .resizable(false)
        .default_size(Vec2::new(700.0, 440.0))
        .open(&mut open)
        .show(ctx, |ui| {
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
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("OK").clicked() {
                        commit = true;
                        close = true;
                    }
                });
            });
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

    if close || !open {
        *function_popup = None;
    }
    batch
}
