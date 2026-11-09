//! Inline function rows and edit-path construction.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

fn foundation_function_row(
    ui: &mut Ui, meta: &FieldDisplayMeta, depth: usize, contents: impl FnOnce(&mut Ui),
) {
    if stacked_field_row(ui) {
        foundation_field_row(ui, meta, depth, contents);
    } else {
        ui.horizontal_top(|ui| {
            foundation_label_cell(ui, &meta.label, meta.help.as_deref());
            contents(ui);
        });
    }
}

pub(in crate::app) fn draw_foundation_function_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    function: &TagFunction,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let key = function_row_key(ui, function, &meta.label, depth, edit.editable);
    draw_function_row_unless_offscreen(ui, ("function_row", path), key, |ui| {
        draw_foundation_function_row_contents(ui, meta, function, depth, path, edit);
    });
}

fn draw_foundation_function_row_contents(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    function: &TagFunction,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    foundation_function_row(ui, meta, depth, |ui| {
        crate::app::ScrollArea::horizontal().id_salt(("function_surface", path))
            .auto_shrink([false, true]).show(ui, |ui| {
        Frame::NONE
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0_f32, foundation_group_edge()))
            // A 6-point inset: egui counts the stroke as padding.
            .inner_margin(egui::Margin::same(5))
            .show(ui, |ui| {
                // `Frame::show` inherits the parent layout, and this row is
                // built inside a `horizontal_top`. Force a vertical layout so
                // the function editor stacks its controls / graph / time-period
                // top-to-bottom (Guerilla-style) instead of sprawling to the
                // right.
                ui.vertical(|ui| {
                    let inline = is_inline_field_function(path, function.encoding(), edit.game, edit.group_tag);
                    if inline {
                        ui.set_width(623.0);
                    } else {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        foundation_input_cell(ui, &shader_function_grid_text(function), 520.0);
                        let can_edit = edit.can_edit(meta);
                        let function_button = foundation_header_button_clicked_hint(
                            ui,
                            "f()",
                            can_edit,
                            Some("Function is read-only"),
                        );
                        if function_button {
                            *edit.function_request = Some(FunctionPopup::new(
                                edit.tag_key.to_owned(),
                                canonical_field_path(path),
                                FunctionView::from_function(function.clone()).with_edit(
                                    foundation_function_edit_paths(path, function.encoding()),
                                ),
                                true,
                            ));
                        }
                    });
                    ui.add_space(4.0);
                    }
                    #[cfg(test)]
                    FUNCTION_PREVIEWS_BUILT.with(|count| count.set(count.get() + 1));
                    ui.push_id(("function", path), |ui| {
                        if inline {
                            let view = FunctionView::from_function(function.clone()).with_edit(
                                foundation_function_edit_paths(path, function.encoding()),
                            );
                            draw_editable_inline_field_function(ui, view, edit.can_edit(meta), edit);
                            return;
                        }
                        // Inline preview is always read-only; the editable
                        // editor lives in the f() popup.
                        let mut view = FunctionView::from_function(function.clone());
                        let (mut graph, mut point, mut no_popup) = (0usize, 0usize, None);
                        draw_function_editor(
                            ui,
                            &mut view,
                            false,
                            &mut graph,
                            &mut point,
                            &mut no_popup,
                        );
                    });
                });
            });
        });
    });
}

pub(in crate::app) fn draw_foundation_inline_function_row(
    ui: &mut Ui,
    label: String,
    mut view: FunctionView,
    depth: usize,
    data_path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let encoding = view.function.encoding();
    view = view.with_edit(foundation_function_edit_paths(data_path, encoding));

    // Object functions and Halo 2 HUD functions edit directly in this row;
    // others use their preview and the f() button for the full editor.
    draw_foundation_wrapped_function_row(ui, label, view, depth, edit);
}

fn draw_foundation_wrapped_function_row(
    ui: &mut Ui,
    label: String,
    view: FunctionView,
    depth: usize,
    edit: &mut FieldEditContext<'_>,
) {
    let key = function_row_key(ui, &view.function, &label, depth, edit.editable);
    let id_source = ("wrapped_function_row", data_path_id(&view).to_owned());
    draw_function_row_unless_offscreen(ui, id_source, key, |ui| {
        draw_foundation_wrapped_function_row_contents(ui, label, view, depth, edit);
    });
}

fn draw_foundation_wrapped_function_row_contents(
    ui: &mut Ui,
    label: String,
    view: FunctionView,
    depth: usize,
    edit: &mut FieldEditContext<'_>,
) {
    let meta = field_display_meta(&label);
    foundation_function_row(ui, &meta, depth, |ui| {
        crate::app::ScrollArea::horizontal().id_salt(("wrapped_function_surface", data_path_id(&view)))
            .auto_shrink([false, true]).show(ui, |ui| {
        Frame::NONE
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0_f32, foundation_group_edge()))
            // A 6-point inset: egui counts the stroke as padding.
            .inner_margin(egui::Margin::same(5))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    let inline = is_inline_field_function(data_path_id(&view), view.function.encoding(), edit.game, edit.group_tag);
                    if inline {
                        ui.set_width(623.0);
                    } else {
                    ui.set_min_width(640.0);
                    ui.horizontal(|ui| {
                        foundation_input_cell(
                            ui,
                            &shader_function_grid_text(&view.function),
                            520.0,
                        );
                        let function_button = foundation_header_button_clicked_hint(
                            ui,
                            "f()",
                            edit.editable,
                            Some("Function is read-only"),
                        );
                        if function_button {
                            *edit.function_request = Some(FunctionPopup::new(
                                edit.tag_key.to_owned(),
                                label.clone(),
                                view.clone(),
                                true,
                            ));
                        }
                    });
                    ui.add_space(4.0);
                    }
                    #[cfg(test)]
                    FUNCTION_PREVIEWS_BUILT.with(|count| count.set(count.get() + 1));
                    ui.push_id(("wrapped_function", data_path_id(&view)), |ui| {
                        if inline {
                            draw_editable_inline_field_function(ui, view.clone(), edit.editable, edit);
                            return;
                        }
                        let mut preview = view.clone();
                        let (mut graph, mut point, mut no_popup) = (0usize, 0usize, None);
                        draw_function_editor(
                            ui,
                            &mut preview,
                            false,
                            &mut graph,
                            &mut point,
                            &mut no_popup,
                        );
                    });
                });
            });
        });
    });
}

#[cfg(test)]
thread_local! {
    /// Inline function editors and previews this thread built.
    pub(in crate::app) static FUNCTION_PREVIEWS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Off draws every function row, as the editor did before it culled any.
    pub(in crate::app) static FUNCTION_ROWS_CULLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
fn function_rows_culled() -> bool {
    FUNCTION_ROWS_CULLED.with(|culled| culled.get())
}

#[cfg(not(test))]
fn function_rows_culled() -> bool {
    true
}

/// Everything a function row's height depends on: the function itself, what
/// the row shows around it, and the space it is laid out in.
fn function_row_key(
    ui: &Ui,
    function: &TagFunction,
    label: &str,
    depth: usize,
    editable: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    function.to_bytes().hash(&mut hasher);
    (function.encoding() == FunctionEncoding::H2).hash(&mut hasher);
    label.hash(&mut hasher);
    depth.hash(&mut hasher);
    editable.hash(&mut hasher);
    // The viewport's width, not `available_width`: a scroll area's content
    // grows to the widest row laid out so far, so that one depends on which
    // rows above were culled.
    ui.clip_rect().width().to_bits().hash(&mut hasher);
    ui.ctx().pixels_per_point().to_bits().hash(&mut hasher);
    hasher.finish()
}

/// Draw a function row, unless the height it had when last drawn puts all of
/// it outside the clip rect: then reserve that height instead. Each row
/// builds a full function editor, graphs sampled and all, so a tag
/// with many functions paid for every one of them each frame, on screen or
/// not.
///
/// The height is reused only under the same `key`, so a function edited
/// while its row is off screen (the f() popup outlives the row) is measured
/// again.
fn draw_function_row_unless_offscreen(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    key: u64,
    draw: impl FnOnce(&mut Ui),
) {
    let top_down = ui.layout().main_dir() == egui::Direction::TopDown;
    if !top_down {
        draw(ui);
        return;
    }
    let id = ui.make_persistent_id(id_source);
    let gesture_active = ui.data(|data| data.get_temp::<bool>(id.with("gesture_active"))).unwrap_or(false);
    let spacing = ui.spacing().item_spacing.y;
    if function_rows_culled()
        && !gesture_active
        && let Some((cached_key, height)) = ui.data(|data| data.get_temp::<(u64, f32)>(id))
        && cached_key == key
        && height > spacing
    {
        let rect =
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
        if !ui.is_rect_visible(rect) {
            ui.allocate_space(egui::vec2(0.0, height - spacing));
            return;
        }
    }
    let top = ui.cursor().top();
    let current_row = egui::Id::new("drawing_function_row");
    ui.data_mut(|data| data.insert_temp(current_row, id));
    draw(ui);
    ui.data_mut(|data| data.remove::<egui::Id>(current_row));
    let height = ui.cursor().top() - top;
    ui.data_mut(|data| data.insert_temp(id, (key, height)));
}

fn data_path_id(view: &FunctionView) -> &str {
    view.edit
        .as_ref()
        .map(|paths| match &paths.data {
            FunctionDataStorage::DataField(path) | FunctionDataStorage::Halo2ByteBlock(path) => {
                path.as_str()
            }
        })
        .unwrap_or("function")
}

fn is_inline_h2_object_function(path: &str, encoding: FunctionEncoding) -> bool {
    encoding == FunctionEncoding::H2 && is_object_function_path(path)
}

fn is_object_function_path(path: &str) -> bool {
    path
            .split('/')
            .any(|segment| segment.contains('[')
                && segment.split(['#', '[']).next() == Some("functions"))
}

fn is_inline_object_function(path: &str, encoding: FunctionEncoding, game: Option<GameId>) -> bool {
    is_inline_h2_object_function(path, encoding)
        || (encoding == FunctionEncoding::Blob
            && matches!(game, Some(GameId::Halo3 | GameId::Halo3Odst))
            && is_object_function_path(path))
}

pub(super) fn is_h2_hud_function_tag(game: Option<GameId>, group_tag: u32) -> bool {
    game == Some(GameId::Halo2) && group_tag == u32::from_be_bytes(*b"nhdt")
}

fn is_inline_field_function(path: &str, encoding: FunctionEncoding, game: Option<GameId>, group_tag: u32) -> bool {
    is_inline_object_function(path, encoding, game)
        || (encoding == FunctionEncoding::H2 && is_h2_hud_function_tag(game, group_tag))
}

fn draw_editable_inline_field_function(
    ui: &mut Ui,
    mut view: FunctionView,
    editable: bool,
    edit: &mut FieldEditContext<'_>,
) {
    // Match the graph (465), gap (8), and right-hand controls (150), rather
    // than expanding the parameter fields across the tag pane.
    ui.set_width(623.0);
    let id = edit.widget_id(("inline_function_selection", data_path_id(&view)));
    let (mut graph, mut point) = ui
        .data(|data| data.get_temp::<(usize, usize)>(id))
        .unwrap_or_default();
    let previous = FunctionSnapshot::from_view(&view);
    let gesture_id = id.with("function_gesture");
    let mut gesture = ui.data_mut(|data| {
        let gesture = data.get_temp::<InlineFunctionGesture>(gesture_id);
        data.remove::<InlineFunctionGesture>(gesture_id);
        gesture
    })
        .filter(|gesture| gesture.matches_document(&view))
        .unwrap_or_else(|| InlineFunctionGesture::new(&view));
    view = gesture.view.clone();
    if is_h2_hud_function_tag(edit.game, edit.group_tag) && view.function.encoding() == FunctionEncoding::H2 {
        view.color_types = ColorTypeChoices::ScalarOnly;
    }
    let mut color = None;
    draw_inline_mapping_function_editor(
        ui, &mut view, editable, &mut graph, &mut point, &mut color,
    );
    let dragging = ui.input(|input| input.pointer.primary_down());
    if let Some(row) = ui.data(|data| data.get_temp::<egui::Id>(egui::Id::new("drawing_function_row"))) {
        ui.data_mut(|data| data.insert_temp(row.with("gesture_active"), dragging));
    }
    if dragging {
        gesture.view = view.clone();
        ui.data_mut(|data| data.insert_temp(gesture_id, gesture));
        ui.ctx().request_repaint();
    }
    if !dragging && FunctionSnapshot::from_view(&view) != previous
        && let Some(paths) = view.edit.as_ref()
    {
        let batch = push_function_edit(paths, &previous, &view);
        edit.pending.extend(batch.edits);
        edit.h2_shader_param_ops
            .extend(
                batch
                    .data_ops
                    .into_iter()
                    .map(|op| H2ShaderParamOp::EditFunctionData {
                        block_path: op.block_path,
                        data: op.data,
                    }),
            );
    }
    ui.data_mut(|data| data.insert_temp(id, (graph, point)));
    if let Some(color) = color
        && let Some(paths) = view.edit.as_ref()
    {
        *edit.color_request = Some(color.with_inline_function_target(
            edit.tag_key,
            paths.data.clone(),
            view.function.clone(),
        ));
    }
}

/// Write targets for a function at `data_path`. Where its bytes go follows
/// from the encoding: a Halo 2 function lives in a byte-block, an H3+ blob in a
/// data field.
pub(in crate::app) fn foundation_function_edit_paths(
    data_path: &str,
    encoding: FunctionEncoding,
) -> FunctionEditPaths {
    FunctionEditPaths {
        data: match encoding {
            FunctionEncoding::H2 => FunctionDataStorage::Halo2ByteBlock(data_path.to_owned()),
            FunctionEncoding::Blob => FunctionDataStorage::DataField(data_path.to_owned()),
        },
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    }
}

/// First-pass editable function types — others stay read-only (graph +
/// controls disabled) but still round-trip on save.
pub(in crate::app) fn draw_foundation_enum_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    options: &[&str],
    current: Option<i64>,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let mut selected = current.unwrap_or(-1);
    let breakpoint = foundation_field_breakpoint(ui, depth, 180.0, "");
    foundation_field_row_at(ui, meta, depth, breakpoint, |ui| {

        ui.add_enabled_ui(edit.can_edit(meta), |ui| {
            let selected_label = enum_option_label(options, selected);
            let selected_text = highlighted_widget_text(
                ui,
                &selected_label,
                TextStyle::Button,
                text_dark(),
                FindTargetKind::Value,
            )
            .unwrap_or_else(|| selected_label.clone().into());
            // Focusing the trigger or its search disables offscreen-row
            // culling. Anchor the popup to the field, not the auto IDs of the
            // surrounding layout, which change when those rows are restored.
            let enum_id = edit.widget_id((path, "enum_popup"));
            let (_, wheel_delta) = ui.scope_builder(egui::UiBuilder::new().id(enum_id), |ui| combo_box_with_scroll(
                ui,
                crate::app::ui_kit::ValueComboBox::from_id_salt((edit.view_scope, edit.tag_key, path, "enum"))
                    .width(240.0_f32.min(ui.available_size_before_wrap().x.max(1.0)))
                    .selected_text(selected_text),
                |ui| {
                    // `!` options are listed only in expert mode, as Foundation does.
                    for (index, option) in options.iter().enumerate() {
                        let option = field_display_meta(option);
                        if option.advanced && !edit.expert_mode {
                            continue;
                        }
                        let response = ui.selectable_value(&mut selected, index as i64, option.label);
                        if let Some(help) = option.help {
                            response.on_hover_text(help);
                        }
                    }
                },
            )).inner;
            if let Some(delta) = wheel_delta
                && let Some(next) =
                    combo_scroll_next_i64(selected, 0, options.len() as i64 - 1, delta)
            {
                selected = next;
            }
        });
        if Some(selected) != current && selected >= 0 {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: selected.to_string(),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::{FunctionType, H2Function, TagFunction};
    use crate::app::editor::FieldDisplayMeta;
    use crate::app::editor::fields::with_test_edit_context;
    use eframe::egui;
    use super::{FUNCTION_PREVIEWS_BUILT, FUNCTION_ROWS_CULLED, draw_foundation_function_row};

    fn constant_view() -> FunctionView {
        let bytes = decode_hex(&constant_function_hex(0.0)).expect("constant function bytes");
        FunctionView::from_function(TagFunction::parse(&bytes).expect("constant function"))
    }

    #[test]
    fn every_function_reads_in_its_games_encoding() {
        assert_eq!(constant_view().function.encoding(), FunctionEncoding::Blob);
        let mut raw = vec![0; 28];
        raw[0] = FunctionType::Constant as u8;
        raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        let view = FunctionView::from_function(h2_tag_function(&raw).expect("an H2 block"));
        assert_eq!(view.function.encoding(), FunctionEncoding::H2);
    }

    #[test]
    fn inline_object_function_controls_fit_the_graph_grid_without_summary() {
        for kind in [FunctionType::Transition, FunctionType::Periodic] {
            let ctx = egui::Context::default();
            let mut editor = TagFunctionEditor::from_function(h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap());
            editor.set_function_type(kind).unwrap();
            let view = FunctionView::from_function(editor.into_function())
                .with_edit(foundation_function_edit_paths("object#0/functions#21[0]/default function#5/data", FunctionEncoding::H2));
            let summary = shader_function_grid_text(&view.function);
            let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1800.0, 1000.0))),
                ..Default::default()
            }, |ui| with_test_edit_context(|edit| {
                draw_foundation_wrapped_function_row_contents(ui, "default function".to_owned(), view.clone(), 0, edit);
            }));
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == summary)));
            let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == function_plot_bg() && (rect.rect.width() - 445.0).abs() < 1.0 => Some(rect.rect),
                _ => None,
            }).unwrap();
            let right = plot.left() - 10.0 + 623.0;
            for shape in &output.shapes {
                if let egui::Shape::Rect(rect) = &shape.shape
                    && rect.rect.top() < plot.top() && rect.rect.height() <= 36.0
                    && rect.rect.left() >= plot.left() - 10.0
                {
                    assert!(rect.rect.right() <= right + 1.0, "{kind:?}: a control extends beyond the graph grid");
                }
            }
        }
    }

        fn functions_path(st: TagStruct<'_>, prefix: &str) -> Option<String> {
            for field in st.fields() {
                let path = append_field_path_for(prefix, &field);
                if field.clean_name() == "functions" && field.as_block().is_some() {
                    return Some(path);
                }
                if let Some(nested) = field.as_struct()
                    && let Some(path) = functions_path(nested, &path)
                {
                    return Some(path);
                }
            }
            None
        }

    #[test]
    fn inherited_h2_object_functions_use_the_inline_editor_with_positional_paths() {
        for kind in ["biped", "vehicle", "weapon", "scenery", "equipment"] {
            let mut tag = TagFile::new(test_definition_path(&format!("halo2_mcc/{kind}.json"))).unwrap();
            let path = functions_path(tag.root(), "").unwrap_or_else(|| panic!("{kind} inherits Object functions"));
            assert!(path.contains('#'), "test real positional field addresses");
            apply_one_block_op(&mut tag, &BlockOp { path: path.clone(), kind: BlockOpKind::Add }).unwrap();
            let element_path = format!("{path}[0]");
            let element = tag.root().descend(&element_path).unwrap();
            let field = element.fields().find(|field| field.clean_name() == "default function" && field.as_struct().is_some()).unwrap();
            let struct_path = append_field_path_for(&element_path, &field);
            let (view, data_path) = inline_mapping_function_from_struct(field.as_struct().unwrap(), &struct_path).unwrap();
            assert!(is_inline_h2_object_function(&data_path, view.function.encoding()), "{kind}: {data_path}");
        }
        assert!(!is_inline_h2_object_function("functions#21[0]/default function#5/data", FunctionEncoding::Blob));
        assert!(!is_inline_h2_object_function("old functions#20[0]/data", FunctionEncoding::H2));
    }

    #[test]
    fn halo3_and_odst_object_functions_edit_inline_without_changing_reach_or_halo4() {
        for (schema, game) in [("halo3_mcc", GameId::Halo3), ("halo3odst_mcc", GameId::Halo3Odst)] {
            for kind in ["biped", "vehicle", "weapon"] {
                let mut tag = TagFile::new(test_definition_path(&format!("{schema}/{kind}.json"))).unwrap();
                let block_path = functions_path(tag.root(), "").unwrap();
                apply_one_block_op(&mut tag, &BlockOp { path: block_path.clone(), kind: BlockOpKind::Add }).unwrap();
                let element_path = format!("{block_path}[0]");
                let element = tag.root().descend(&element_path).unwrap();
                let field = element.fields().find(|field| field.clean_name() == "default function" && field.as_struct().is_some()).unwrap();
                let struct_path = append_field_path_for(&element_path, &field);
                let (mut view, data_path) = inline_mapping_function_from_struct(field.as_struct().unwrap(), &struct_path).unwrap();
                assert!(is_inline_object_function(&data_path, view.function.encoding(), Some(game)));
                let mut editor = TagFunctionEditor::from_function(view.function.clone());
                // Fresh Object functions are Identity (also presented as
                // Curve), so explicitly author a control-point curve here.
                editor.set_master_type(EngineMasterType::Basic).unwrap();
                editor.set_master_type(EngineMasterType::Curve).unwrap();
                view.function = editor.into_function();
                view = view.with_edit(foundation_function_edit_paths(&data_path, FunctionEncoding::Blob));
                let ctx = egui::Context::default();
                let mut pending = Vec::new();
                let mut frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput {
                    events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1100.0, 1000.0))), ..Default::default()
                }, |ui| with_test_edit_context(|edit| {
                    edit.game = Some(game);
                    draw_foundation_wrapped_function_row_contents(ui, "default function".to_owned(), view.clone(), 0, edit);
                    assert!(edit.function_request.is_none());
                    assert!(edit.h2_shader_param_ops.is_empty());
                    pending.extend(edit.pending.iter().cloned());
                    if ui.input(|input| input.pointer.primary_down()) {
                        assert!(edit.pending.is_empty(), "the graph draft is not committed during a drag");
                    }
                }));
                frame(Vec::new());
                let output = frame(Vec::new());
                for label in ["Input", "Range", "Time Period"] {
                    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)));
                }
                let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == function_plot_bg() && (rect.rect.width() - 445.0).abs() < 1.0 => Some(rect.rect), _ => None,
                }).unwrap();
                let point = TagFunctionEditor::from_function(view.function.clone()).curve_control_point(0, 0).unwrap();
                let start = egui::pos2(egui::lerp(plot.left()..=plot.right(), point.0), egui::lerp(plot.bottom()..=plot.top(), point.1));
                let end = egui::pos2(start.x, plot.center().y);
                frame(vec![egui::Event::PointerMoved(start)]);
                frame(vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
                frame(vec![egui::Event::PointerMoved(end)]);
                frame(vec![egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
                assert_eq!(pending.len(), 1, "one blob edit per completed drag");
                let edit = pending.last().expect("inline graph emits a blob field edit");
                assert_eq!(edit.path, data_path);
                apply_field_edit(&mut tag, &edit.path, &edit.input).unwrap();
                let bytes = tag.root().field_path(&data_path).unwrap().as_data().unwrap().to_vec();
                let edited = TagFunctionEditor::from_function(TagFunction::parse(&bytes).unwrap());
                assert!((edited.curve_control_point(0, 0).unwrap().1 - 0.5).abs() < 0.01);
            }
        }
        for game in [GameId::HaloReach, GameId::Halo4] {
            assert!(!is_inline_object_function("object#0/functions#21[0]/default function#5/data", FunctionEncoding::Blob, Some(game)));
        }
    }

    fn new_h2_hud_effect(block: &str) -> (TagFile, String) {
        let mut tag = TagFile::new_classic(
            test_definition_path("halo2_mcc/new_hud_definition.json"),
            blam_tags::classic::ClassicEngine::Halo2V4,
        ).unwrap();
        apply_one_block_op(&mut tag, &BlockOp { path: block.to_owned(), kind: BlockOpKind::Add }).unwrap();
        let path = format!("{block}[0]/effect");
        apply_one_block_op(&mut tag, &BlockOp { path, kind: BlockOpKind::Add }).unwrap();
        (tag, format!("{block}[0]/effect[0]"))
    }

    #[test]
    fn h2_hud_functions_keep_their_fields_without_wrapper_headers_or_popouts() {
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(
            &locate_definitions_root(), GameId::Halo2, "new_hud_definition",
        )));
        for block in ["bitmap widgets", "text widgets"] {
            let (tag, path) = new_h2_hud_effect(block);
            for width in [1100.0, 1800.0] {
                let ctx = egui::Context::default();
                ctx.set_fonts(crate::app::foundation_fonts());
                let mut output = None;
                for _ in 0..2 {
                    output = Some(crate::app::run_ui_test(&ctx, egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 7000.0))),
                        ..Default::default()
                    }, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            with_test_edit_context(|edit| {
                                edit.game = Some(GameId::Halo2);
                                edit.group_tag = u32::from_be_bytes(*b"nhdt");
                                edit.docs = Some(docs);
                                draw_struct_fields_inline(ui, tag.root().descend(&path).unwrap(),
                                    &TagNameIndex::default(), 0, false, &path, edit);
                                assert!(edit.function_request.is_none());
                                assert!(edit.pending.is_empty() && edit.h2_shader_param_ops.is_empty(),
                                    "drawing a fresh HUD effect must not modify it");
                            });
                        });
                    }));
                }
                let output = output.unwrap();
                let texts = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()), _ => None,
                }).collect::<Vec<_>>();
                assert!(!texts.iter().any(|text| text.to_lowercase().contains("your mom")), "{texts:?}");
                assert!(!texts.contains(&"f()"), "HUD functions edit directly");
                assert!(!texts.contains(&"Function"), "no collapsible function heading");
                assert_eq!(texts.iter().filter(|text| **text == "function").count(), 5,
                    "one static label per function, without an extra wrapper heading");
                for section in ["Scale", "Horizontal Scale", "Vertical Scale", "Theta", "Offset", "Horizontal Offset", "Vertical Offset"] {
                    assert!(texts.contains(&section), "keep meaningful effect sections: {section}");
                }
                for label in ["input name", "range name", "time period in seconds", "Function Type"] {
                    assert_eq!(texts.iter().filter(|text| **text == label).count(), 5, "{label}: {texts:?}");
                }
                let panels = output.shapes.iter().filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == foundation_group_bg()
                        && rect.stroke.color == foundation_group_edge() && rect.rect.height() > 150.0 => Some(rect.rect),
                    _ => None,
                }).collect::<Vec<_>>();
                assert_eq!(panels.len(), 5, "one panel per HUD function: {panels:?}");
                assert!(panels.iter().all(|rect| rect.width() <= 640.0), "HUD editors must stay compact: {panels:?}");
            }
        }
    }

    #[test]
    fn h2_hud_unnamed_structs_keep_useful_sections_without_blank_headers() {
        let (tag, _) = new_h2_hud_effect("bitmap widgets");
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(
            &locate_definitions_root(), GameId::Halo2, "new_hud_definition",
        )));
        let mut output = None;
        for _ in 0..2 {
            output = Some(crate::app::run_ui_test(&ctx, egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 10000.0))),
                ..Default::default()
            }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    with_test_edit_context(|edit| {
                        edit.game = Some(GameId::Halo2);
                        edit.group_tag = u32::from_be_bytes(*b"nhdt");
                        edit.docs = Some(docs);
                        draw_struct_fields_inline(ui, tag.root().descend("bitmap widgets[0]").unwrap(),
                            &TagNameIndex::default(), 0, false, "bitmap widgets[0]", edit);
                    });
                });
            }));
        }
        let output = output.unwrap();
        let titles = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().trim().chars().count() > 1 =>
                Some((text.galley.text(), text.galley.rect.translate(text.pos.to_vec2()))),
            _ => None,
        }).collect::<Vec<_>>();
        for title in ["widget inputs", "Widget States"] {
            assert!(titles.iter().any(|(text, _)| *text == title), "keep the definition's section: {title}");
        }
        for shape in &output.shapes {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.fill == foundation_section_bar() && rect.rect.width() > 100.0
                && (32.0..=40.0).contains(&rect.rect.height()) {
                assert!(titles.iter().any(|(_, title)| rect.rect.intersects(*title)),
                    "a section header must have a meaningful title: {:?}", rect.rect);
            }
        }
    }

    #[test]
    fn h2_hud_graph_edits_commit_inline_and_preserve_wrapper_fields() {
        let (mut tag, effect_path) = new_h2_hud_effect("bitmap widgets");
        let paths = tag.root().descend(&effect_path).unwrap().fields().filter_map(|field| {
            let wrapper = field.as_struct()?;
            if wrapper.definition().name() != "hud_widget_effect_function_struct_2" { return None; }
            let wrapper_path = append_field_path_for(&effect_path, &field);
            let scalar = wrapper.fields().find(|field| field.as_struct().is_some()).unwrap();
            let scalar_path = append_field_path_for(&wrapper_path, &scalar);
            let mapping = scalar.as_struct().unwrap().fields().find(|field| field.as_struct().is_some()).unwrap();
            Some((wrapper_path, append_field_path_for(&scalar_path, &mapping)))
        }).collect::<Vec<_>>();
        assert_eq!(paths.len(), 5);
        let (wrapper_path, mapping_path) = &paths[1];
        for (field, value) in [("input name", "hud_input"), ("range name", "hud_range"), ("time period in seconds", "2.5")] {
            crate::core::document::apply::apply_field_edit(&mut tag, &format!("{wrapper_path}/{field}"), value).unwrap();
        }
        let mut function = TagFunctionEditor::from_function(TagFunction::H2(H2Function::new(FunctionType::Identity)));
        function.set_function_type(FunctionType::Spline).unwrap();
        let data_path = append_field_path(mapping_path, "data");
        replace_halo2_function_byte_block(&mut tag, &data_path, &function.into_function().to_bytes()).unwrap();
        let (view, _) = inline_mapping_function_from_struct(tag.root().descend(mapping_path).unwrap(), mapping_path).unwrap();
        let initial = TagFunctionEditor::from_function(view.function.clone()).curve_control_point(0, 0).unwrap();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let mut pending = Vec::new();
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(
            &locate_definitions_root(), GameId::Halo2, "new_hud_definition",
        )));
        let mut frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput {
            events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 7000.0))),
            ..Default::default()
        }, |ui| {
            with_test_edit_context(|edit| {
                edit.game = Some(GameId::Halo2);
                edit.group_tag = u32::from_be_bytes(*b"nhdt");
                edit.docs = Some(docs);
                draw_struct_fields_inline(ui, tag.root().descend(&effect_path).unwrap(),
                    &TagNameIndex::default(), 0, false, &effect_path, edit);
                assert!(edit.function_request.is_none());
                pending.extend(edit.h2_shader_param_ops.iter().cloned());
            });
        });
        frame(vec![]);
        let output = frame(vec![]);
        let plot = output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == function_plot_bg() && (rect.rect.width() - 445.0).abs() < 1.0 => Some(rect.rect),
            _ => None,
        }).nth(1).unwrap();
        let start = egui::pos2(egui::lerp(plot.left()..=plot.right(), initial.0), egui::lerp(plot.bottom()..=plot.top(), initial.1));
        let end = egui::pos2(start.x, plot.center().y);
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
        drop(frame);
        assert_eq!(pending.len(), 1, "one inline graph edit on release");
        crate::core::document::apply::apply_one_h2_shader_param_op(&mut tag, &pending[0]).unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let reloaded = blam_tags::classic::read_classic_tag_file(&bytes,
            blam_tags::layout::TagLayout::from_json(test_definition_path("halo2_mcc/new_hud_definition.json")).unwrap()).unwrap();
        let function_bytes = halo2_function_bytes_from_struct(reloaded.root().descend(mapping_path).unwrap()).unwrap();
        let edited = TagFunctionEditor::from_function(h2_tag_function(&function_bytes).unwrap());
        assert!((edited.curve_control_point(0, 0).unwrap().1 - 0.5).abs() < 0.01);
        assert!(halo2_function_bytes_from_struct(reloaded.root().descend(&paths[0].1).unwrap()).unwrap().is_empty(),
            "duplicate placeholder names must not target the neighboring function");
        for (field, value) in [("input name", "hud_input"), ("range name", "hud_range")] {
            assert!(matches!(reloaded.root().field_path(&format!("{wrapper_path}/{field}")).unwrap().value(),
                Some(TagFieldData::StringId(id)) if id.string == value));
        }
        assert!(matches!(reloaded.root().field_path(&format!("{wrapper_path}/time period in seconds")).unwrap().value(),
            Some(TagFieldData::Real(value)) if value == 2.5));
    }

    fn hud_group_frame(
        ctx: &egui::Context, tag: &TagFile, path: &str,
        docs: &'static crate::app::help::DefDocs, events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        crate::app::run_ui_test(ctx, egui::RawInput {
            events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 8000.0))),
            ..Default::default()
        }, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                with_test_edit_context(|edit| {
                    edit.game = Some(GameId::Halo2);
                    edit.group_tag = u32::from_be_bytes(*b"nhdt");
                    edit.docs = Some(docs);
                    draw_struct_fields_inline(ui, tag.root().descend(path).unwrap(),
                        &TagNameIndex::default(), 0, false, path, edit);
                    assert!(edit.pending.is_empty() && edit.h2_shader_param_ops.is_empty(), "layout controls must not edit tag data");
                });
            });
        })
    }

    fn hud_texts(output: &egui::FullOutput) -> Vec<&str> {
        output.shapes.iter().filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text()), _ => None,
        }).collect()
    }

    fn hud_click_text(
        ctx: &egui::Context, tag: &TagFile, path: &str, docs: &'static crate::app::help::DefDocs, title: &str,
    ) -> egui::FullOutput {
        let output = hud_group_frame(ctx, tag, path, docs, vec![]);
        let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == title => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        }).unwrap_or_else(|| panic!("missing clickable label: {title}"));
        hud_group_frame(ctx, tag, path, docs, vec![egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
        hud_group_frame(ctx, tag, path, docs, vec![])
    }

    #[test]
    fn h2_hud_lookup_opens_without_focus_after_scrolling_past_cached_rows() {
        let (tag, path) = new_h2_hud_effect("bitmap widgets");
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(
            &locate_definitions_root(), GameId::Halo2, "new_hud_definition")));
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let heights = Box::leak(Box::<RowHeights>::default());
        with_test_edit_context(|edit| {
            edit.game = Some(GameId::Halo2);
            edit.group_tag = u32::from_be_bytes(*b"nhdt");
            edit.docs = Some(docs);
            edit.row_heights = Some(heights);
            let mut time = 0.0;
            let mut frame = |events| {
                time += 0.1;
                let editing = ctx.memory(|memory| memory.focused().is_some());
                edit.row_heights.as_deref_mut().unwrap().begin(RowHeightsBasis {
                    layout: (0, 0), width: 1200.0, pixels_per_point: 1.0,
                    expert_mode: false, filtering: false,
                }, !editing);
                crate::app::run_ui_test(&ctx, egui::RawInput {
                    events, time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 600.0))),
                    ..Default::default()
                }, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        crate::app::ScrollArea::vertical().auto_shrink([false, false]).vertical_scroll_offset(750.0).show(ui, |ui| {
                            draw_struct_fields_inline(ui, tag.root().descend(&path).unwrap(), &TagNameIndex::default(), 0, false, &path, edit);
                        });
                    });
                })
            };
            for _ in 0..4 { frame(vec![]); }
            let output = frame(vec![]);
            let input = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if (rect.rect.width() - 180.0).abs() < 1.0
                    && (rect.rect.height() - 24.0).abs() < 1.0 && shape.clip_rect.contains(rect.rect.center())
                    && rect.rect.center().y > 100.0 => Some(rect.rect), _ => None,
            }).expect("a visible effect input after scrolling");
            assert!(ctx.memory(|memory| memory.focused().is_none()));
            let pos = input.right_center() + egui::vec2(ctx.global_style().spacing.item_spacing.x + 24.0, 0.0);
            frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]);
            }
            for _ in 0..6 {
                let output = frame(vec![]);
                assert!(hud_texts(&output).contains(&"Search Functions"), "the lookup must stay open with cached offscreen rows");
            }
        });
    }

    #[test]
    fn h2_hud_input_and_range_lookup_commit_exact_ids_to_the_selected_wrapper() {
        for block in ["bitmap widgets", "text widgets"] {
            for name in ["input name", "range name"] {
                let (mut tag, effect_path) = new_h2_hud_effect(block);
                let wrapper_field = tag.root().descend(&effect_path).unwrap().fields()
                    .filter(|field| field.as_struct().is_some_and(|st| st.definition().name() == "hud_widget_effect_function_struct_2"))
                    .nth(1).unwrap();
                let wrapper_path = append_field_path_for(&effect_path, &wrapper_field);
                let wrapper = wrapper_field.as_struct().unwrap();
                let field = wrapper.field(name).unwrap();
                let path = append_field_path_for(&wrapper_path, &field);
                let ctx = egui::Context::default();
                ctx.set_fonts(crate::app::foundation_fonts());
                let mut pending = Vec::new();
                let mut frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput {
                    events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                    ..Default::default()
                }, |ui| {
                    with_test_edit_context(|edit| {
                        edit.game = Some(GameId::Halo2);
                        edit.group_tag = u32::from_be_bytes(*b"nhdt");
                        draw_field(ui, field, path.clone(), wrapper.raw(), &TagNameIndex::default(),
                            0, false, &wrapper_path, edit, None, None, 300.0);
                        pending.extend(edit.pending.iter().cloned());
                    });
                });
                let click = |frame: &mut dyn FnMut(Vec<egui::Event>) -> egui::FullOutput, pos| {
                    frame(vec![egui::Event::PointerMoved(pos)]);
                    for pressed in [true, false] {
                        frame(vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }]);
                    }
                    frame(vec![])
                };
                frame(vec![]);
                let output = frame(vec![]);
                let input = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if (rect.rect.width() - 180.0).abs() < 1.0
                        && (rect.rect.height() - 24.0).abs() < 1.0 => Some(rect.rect), _ => None,
                }).expect("native string-id input");
                assert!(ctx.memory(|memory| memory.focused().is_none()), "open the picker without an active field");
                click(&mut frame, input.right_center() + egui::vec2(ctx.global_style().spacing.item_spacing.x + 24.0, 0.0));
                let output = frame(vec![]);
                assert!(hud_texts(&output).contains(&"Search Functions"));
                let texts = hud_texts(&output);
                for input in ["hud_input1", "hud_input2", "hud_input3", "hud_input4"] {
                    assert_eq!(texts.iter().filter(|text| **text == input).count(), 1, "{input} must appear directly in the popup: {texts:?}");
                }
                for other in ["hud", "vehicle", "weapon", "time", "engine_power", "body_vitality"] {
                    assert!(!texts.contains(&other), "HUD Effect choices must not include other function groups: {texts:?}");
                }
                frame(vec![egui::Event::Text("HUD_INPUT4".into())]);
                let output = frame(vec![]);
                let target = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "hud_input4" => Some(text.pos + text.galley.size() * 0.5), _ => None,
                }).expect("HUD functions are searchable");
                click(&mut frame, target);
                frame(vec![]);
                drop(frame);
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].path, path);
                assert_eq!(pending[0].input, "hud_input4");
                crate::core::document::apply::apply_field_edit(&mut tag, &path, &pending[0].input).unwrap();
                let reloaded = blam_tags::classic::read_classic_tag_file(&tag.write_to_bytes().unwrap(),
                    blam_tags::layout::TagLayout::from_json(test_definition_path("halo2_mcc/new_hud_definition.json")).unwrap()).unwrap();
                assert_eq!(reloaded.root().descend(&wrapper_path).unwrap().read_string_id(name).as_deref(), Some("hud_input4"));
            }
        }
    }

    #[test]
    fn h2_hud_state_section_collapses_flags_and_documentation_but_not_cutoffs() {
        let (tag, _) = new_h2_hud_effect("bitmap widgets");
        let widget_path = "bitmap widgets[0]";
        let field = tag.root().descend(widget_path).unwrap().fields().find(|field|
            field.as_struct().is_some_and(|st| st.definition().name() == "hud_widget_state_definition_struct_2")).unwrap();
        let path = append_field_path_for(widget_path, &field);
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(&locate_definitions_root(), GameId::Halo2, "new_hud_definition")));
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        let before = tag.write_to_bytes().unwrap();
        let open = hud_group_frame(&ctx, &tag, &path, docs, vec![]);
        assert_eq!(hud_texts(&open).iter().filter(|text| text.starts_with("[Y]") || text.starts_with("[N]")).count(), 8);
        assert!(hud_texts(&open).iter().any(|text| text.contains("this section is split up")));
        let closed = hud_click_text(&ctx, &tag, &path, docs, "Widget States");
        assert!(!hud_texts(&closed).iter().any(|text| text.starts_with("[Y]") || text.starts_with("[N]") || text.contains("this section is split up")));
        for cutoff in ["age cutoff", "clip cutoff", "total cutoff"] { assert!(hud_texts(&closed).contains(&cutoff)); }
        let reopened = hud_click_text(&ctx, &tag, &path, docs, "Widget States");
        assert_eq!(hud_texts(&reopened).iter().filter(|text| text.starts_with("[Y]") || text.starts_with("[N]")).count(), 8);
        assert_eq!(tag.write_to_bytes().unwrap(), before);
    }

    #[test]
    fn h2_hud_effect_sections_collapse_independently_with_static_axis_headers() {
        let (tag, path) = new_h2_hud_effect("bitmap widgets");
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(&locate_definitions_root(), GameId::Halo2, "new_hud_definition")));
        for (section, remaining, hidden) in [
            ("Scale", 3, &["Horizontal Scale", "Vertical Scale"][..]),
            ("Theta", 4, &[][..]),
            ("Offset", 3, &["Horizontal Offset", "Vertical Offset"][..]),
        ] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::foundation_fonts());
            ctx.all_styles_mut(|style| style.animation_time = 0.0);
            hud_group_frame(&ctx, &tag, &path, docs, vec![]);
            let closed = hud_click_text(&ctx, &tag, &path, docs, section);
            assert_eq!(hud_texts(&closed).iter().filter(|text| **text == "Function Type").count(), remaining, "{section}");
            for title in hidden { assert!(!hud_texts(&closed).contains(title)); }
            let reopened = hud_click_text(&ctx, &tag, &path, docs, section);
            assert_eq!(hud_texts(&reopened).iter().filter(|text| **text == "Function Type").count(), 5);
            let static_header = hud_click_text(&ctx, &tag, &path, docs, "Horizontal Scale");
            assert_eq!(hud_texts(&static_header).iter().filter(|text| **text == "Function Type").count(), 5);
        }
    }

    #[test]
    fn h2_hud_effect_output_picker_offers_only_scalar_intensity() {
        let (tag, path) = new_h2_hud_effect("bitmap widgets");
        let docs = Box::leak(Box::new(crate::app::help::build_def_docs(&locate_definitions_root(), GameId::Halo2, "new_hud_definition")));
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        hud_group_frame(&ctx, &tag, &path, docs, vec![]);
        let output = hud_click_text(&ctx, &tag, &path, docs, "scalar (intensity)");
        let texts = hud_texts(&output);
        assert!(texts.iter().filter(|text| **text == "scalar (intensity)").count() > 5, "the output picker opens: {texts:?}");
        for color in ["constant", "2-color", "3-color", "4-color"] { assert!(!texts.contains(&color), "HUD output must be scalar: {texts:?}"); }
    }

    #[test]
    fn h2_object_functions_edit_inline_without_wrapper_fields() {
        let ctx = egui::Context::default();
        let mut editor = TagFunctionEditor::from_function(h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap());
        editor.set_function_type(FunctionType::Spline).unwrap();
        let path = "functions[0]/default function/data";
        let view = FunctionView::from_function(editor.into_function()).with_edit(foundation_function_edit_paths(path, FunctionEncoding::H2));
        assert!(is_inline_h2_object_function(path, FunctionEncoding::H2));
        assert!(!is_inline_h2_object_function("parameters[0]/animation properties[0]/function/data", FunctionEncoding::H2));
        let initial = TagFunctionEditor::from_function(view.function.clone()).curve_control_point(0, 0).unwrap();
        let mut pending = Vec::new();
        let mut selection = None;
        let mut frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 1000.0))),
            ..Default::default()
        }, |ui| {
            ui.set_width(623.0);
            with_test_edit_context(|edit| {
                let id = edit.widget_id(("inline_function_selection", path));
                draw_editable_inline_field_function(ui, view.clone(), true, edit);
                if ui.input(|input| input.pointer.primary_down()) {
                    assert!(edit.h2_shader_param_ops.is_empty(), "dragging only changes the live draft");
                }
                pending.extend(edit.h2_shader_param_ops.iter().cloned());
                selection = ui.data(|data| data.get_temp::<(usize, usize)>(id));
            });
        });
        frame(Vec::new());
        let output = frame(Vec::new());
        for label in ["Input", "Range", "Time Period"] {
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)));
        }
        let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == function_plot_bg() && (rect.rect.width() - 445.0).abs() < 1.0 => Some(rect.rect),
            _ => None,
        }).unwrap();
        let start = egui::pos2(egui::lerp(plot.left()..=plot.right(), initial.0), egui::lerp(plot.bottom()..=plot.top(), initial.1));
        let end = egui::pos2(start.x, plot.center().y);
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]);
        assert_eq!(pending.len(), 1, "the whole drag commits once on release");
        let op = pending.last().expect("dragging inline queues a tag edit");
        let H2ShaderParamOp::EditFunctionData { block_path, data } = op else { panic!("a byte-block edit"); };
        assert_eq!(block_path, path);
        let edited = TagFunctionEditor::from_function(h2_tag_function(data).unwrap());
        assert!((edited.curve_control_point(0, 0).unwrap().1 - 0.5).abs() < 0.01);
        assert_eq!(selection, Some((0, 0)));
        assert_ne!(data_path_id(&view), data_path_id(&view.clone().with_edit(foundation_function_edit_paths("functions[1]/default function/data", FunctionEncoding::H2))));
    }

    /// Adding a block element that contains a `mapping_function` used to leave the
    /// function at `data [0 bytes]`, which fell past
    /// `inline_mapping_function_from_struct` and drew a raw byte row with no
    /// function editor. The reported case: `equipment`'s hologram block and its
    /// `shimmer to camo function`.
    #[test]
    fn a_new_block_elements_function_is_recognized_by_the_editor() {
        let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
            .expect("the Reach equipment schema loads");
        {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("hologram")
                .expect("hologram field resolves");
            let mut hologram = field.as_block_mut().expect("hologram is a block");
            hologram.add_element();
        }

        // The row the UI actually draws is the inner `mapping_function`, not the
        // named wrapper around it — the wrapper has no `data` field of its own, so
        // the field tree descends into it first. That is the nesting in the report:
        // "shimmer to camo function" > "function" > "data".
        // `scalar_function_named_struct` has two fields called "function" — the
        // `fned` editor marker and the `mapping_function` struct — so pick it the
        // way the field tree does, by walking fields, rather than by path ordinal.
        let wrapper = tag
            .root()
            .field_path("hologram[0]/shimmer to camo function")
            .and_then(|field| field.as_struct())
            .expect("the new element exposes the shimmer wrapper struct");
        let shimmer = wrapper
            .fields_all()
            .find_map(|field| field.as_struct())
            .expect("the wrapper holds the mapping_function struct");
        let (view, data_path) = inline_mapping_function_from_struct(
            shimmer,
            "hologram[0]/shimmer to camo function/function",
        )
        .expect("a fresh function must reach the function editor, not a raw data row");

        assert_eq!(
            data_path,
            "hologram[0]/shimmer to camo function/function/data"
        );
        assert_eq!(
            view.function.encoding(),
            FunctionEncoding::Blob,
            "a Reach function is an H3+ blob, not a Halo 2 byte-block"
        );
    }

    /// The seeded function has to be the engine's, not merely non-empty: Identity,
    /// CLAMPED | GPU, clamping to 0..1. Asserting only that the editor opens would
    /// pass on any 32 bytes that happen to parse.
    #[test]
    fn a_new_functions_bytes_match_what_the_engine_writes() {
        let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
            .expect("the Reach equipment schema loads");
        {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("hologram")
                .expect("hologram field resolves");
            let mut hologram = field.as_block_mut().expect("hologram is a block");
            hologram.add_element();
        }

        let bytes = tag
            .root()
            .field_path("hologram[0]/shimmer to camo function/function/data")
            .and_then(|field| field.as_data().map(|d| d.to_vec()))
            .expect("the function's data field");

        assert_eq!(
            bytes,
            blam_tags::default_function_definition_bytes(blam_tags::io::Endian::Le),
            "a fresh function must be the blob c_function_definition::tag_placement_new writes"
        );
        let function = TagFunction::parse(&bytes).expect("it parses");
        assert_eq!(function.function_type(), FunctionType::Identity);
        let function = function.as_blob().expect("a blob");
        assert!(function.flags().is_clamped(), "CLAMPED");
        assert!(function.flags().is_gpu(), "GPU");
        assert!(
            !function.flags().is_optimized(),
            "postprocess clears OPTIMIZED"
        );
    }

    /// A new Halo 2 element's function is an empty `data` byte-block. It must get
    /// the function editor (opened as the identity the engine grows an empty block
    /// into) and must never be seeded with an H3+ blob, which the H2 engine would
    /// misread.
    #[test]
    fn a_new_halo2_function_opens_as_h2_and_gets_no_h3_blob() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json"))
            .expect("the Halo 2 shader schema loads");
        {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("parameters")
                .expect("parameters field resolves");
            let mut params = field.as_block_mut().expect("parameters is a block");
            params.add_element();
        }
        {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("parameters[0]/animation properties")
                .expect("animation properties field resolves");
            let mut anim = field
                .as_block_mut()
                .expect("animation properties is a block");
            anim.add_element();
        }

        // Nothing seeded: the byte-block is still empty and there is no data field.
        let function_struct = tag
            .root()
            .field_path("parameters[0]/animation properties[0]/function")
            .and_then(|field| field.as_struct())
            .expect("the H2 function struct");
        let seeded: Vec<_> = function_struct
            .fields_all()
            .filter(|field| field.field_type() == TagFieldType::Data)
            .filter_map(|field| field.as_data().map(|d| d.len()))
            .filter(|len| *len > 0)
            .collect();
        assert!(
            seeded.is_empty(),
            "Halo 2 should carry no seeded function blob, found {seeded:?}"
        );
        assert_eq!(
            halo2_function_bytes_from_struct(function_struct),
            Some(Vec::new())
        );

        let (view, data_path) = inline_mapping_function_from_struct(
            function_struct,
            "parameters[0]/animation properties[0]/function",
        )
        .expect("an empty H2 function still reaches the function editor");
        assert_eq!(view.function.encoding(), FunctionEncoding::H2);
        assert_eq!(view.function.function_type(), FunctionType::Identity);
        assert_eq!(
            data_path,
            "parameters[0]/animation properties[0]/function/data"
        );
    }

    /// Seeding is worthless if the bytes do not persist. A fresh element's function
    /// has to come back byte-identical after a write/read cycle — a `data` sub-chunk
    /// that the writer drops would look correct in memory and be empty again on
    /// reopen, which is the same symptom the fix was for.
    #[test]
    fn a_seeded_function_survives_a_save_and_reload() {
        let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
            .expect("the Reach equipment schema loads");
        {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("hologram")
                .expect("hologram field resolves");
            let mut hologram = field.as_block_mut().expect("hologram is a block");
            hologram.add_element();
        }
        let before = tag
            .root()
            .field_path("hologram[0]/shimmer to camo function/function/data")
            .and_then(|field| field.as_data().map(|d| d.to_vec()))
            .expect("the seeded function");

        let bytes = tag.write_to_bytes().expect("the tag serializes");
        let reloaded = TagFile::read_from_bytes(&bytes).expect("and reads back");
        let after = reloaded
            .root()
            .field_path("hologram[0]/shimmer to camo function/function/data")
            .and_then(|field| field.as_data().map(|d| d.to_vec()))
            .expect("the function survives the round trip");

        assert_eq!(after, before, "the seeded function changed across a save");
        assert_eq!(after.len(), 32, "and it is still a whole function");
    }

    /// Every `mapping_function` in a struct tree, with its resolvable path.
    fn collect_h2_mapping_functions(st: TagStruct<'_>, path: &str, out: &mut Vec<String>) {
        if halo2_function_bytes_from_struct(st).is_some_and(|bytes| !bytes.is_empty()) {
            out.push(path.to_owned());
        }
        for field in st.fields_all() {
            let field_path = append_field_path_for(path, &field);
            if let Some(nested) = field.as_struct() {
                collect_h2_mapping_functions(nested, &field_path, out);
            } else if let Some(block) = field.as_block() {
                for (index, element) in block.iter().enumerate() {
                    collect_h2_mapping_functions(element, &format!("{field_path}[{index}]"), out);
                }
            }
        }
    }

    /// Luna's report went through the field tree: a shipped Halo 2 effect's
    /// functions drew with the wrong editor and their edits never landed. Every
    /// function in the tag now derives the H2 encoding and byte-block storage from
    /// the real path, and an edit written through the byte-block writer reads back
    /// as exactly that edit.
    #[test]
    fn shipped_h2_effect_functions_derive_h2_and_write_back() {
        let tag_path =
            crate::core::test_kits::tag_path("halo2_mcc", "effects/cinematics/03/iac_engine_fire.effect");
        let def = test_definition_path("halo2_mcc/effect.json");
        if !std::path::Path::new(tag_path).exists() || !def.exists() {
            eprintln!("skipping: H2 effect/definition not present");
            return;
        }
        let bytes = std::fs::read(tag_path).unwrap();
        let layout = blam_tags::layout::TagLayout::from_json(&def).unwrap();
        let mut tag = blam_tags::classic::read_classic_tag_file(&bytes, layout).unwrap();

        let mut paths = Vec::new();
        collect_h2_mapping_functions(tag.root(), "", &mut paths);
        assert!(
            paths.len() > 5,
            "the effect has particle functions ({} found)",
            paths.len()
        );
        eprintln!("checked {} H2 functions", paths.len());

        let mut target = None;
        for path in &paths {
            let root = tag.root();
            let st = root.descend(path).expect("the collected path resolves");
            let original = halo2_function_bytes_from_struct(st).unwrap();
            let (view, data_path) =
                inline_mapping_function_from_struct(st, path).expect("the editor finds the function");
            assert_eq!(view.function.encoding(), FunctionEncoding::H2, "{path}");
            assert_eq!(
                view.data_bytes(),
                original,
                "{path}: reading never rewrites"
            );
            let edit_paths = foundation_function_edit_paths(&data_path, view.function.encoding());
            assert!(
                matches!(edit_paths.data, FunctionDataStorage::Halo2ByteBlock(_)),
                "{path}"
            );
            if target.is_none() && view.function.color_count() == 0 {
                target = Some((view, edit_paths));
            }
        }

        // Edit one scalar function's output range and write it back.
        let (mut view, edit_paths) = target.expect("a scalar function to edit");
        let before = view.data_bytes();
        let previous = FunctionSnapshot::from_view(&view);
        view.function
            .as_h2_mut()
            .unwrap()
            .set_clamp_range(0.25, 4.0)
            .unwrap();
        let batch = push_function_edit(&edit_paths, &previous, &view);
        assert!(
            batch.edits.is_empty(),
            "no hex string edit for a byte-block"
        );
        assert_eq!(batch.data_ops.len(), 1);
        let op = &batch.data_ops[0];
        replace_halo2_function_byte_block(&mut tag, &op.block_path, &op.data)
            .expect("the writer accepts it");

        let struct_path = op
            .block_path
            .strip_suffix("/data")
            .unwrap_or(&op.block_path);
        let written =
            halo2_function_bytes_from_struct(tag.root().descend(struct_path).unwrap()).unwrap();
        assert_eq!(written, op.data);
        let reread = h2_tag_function(&written).unwrap();
        let f = reread.as_h2().unwrap();
        assert_eq!((f.clamp_range_min(), f.clamp_range_max()), (0.25, 4.0));
        assert_eq!(&written[..4], &before[..4], "header untouched");
        assert_eq!(&written[12..], &before[12..], "graph data untouched");
    }

    // Function rows off screen are reserved, not built.
    //
    // Every function row carries a full read-only function editor as its
    // preview. The tests draw a column of them in a scroll area and compare what
    // the viewport shows with culling on against the same frames with it off.

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
            slider: None,
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
            let _ = crate::app::run_ui_test(&self.ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let output = crate::app::ScrollArea::vertical()
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
                    // The content's clip rect, which is the viewport itself:
                    // egui no longer widens it by `clip_rect_margin`.
                    viewport = output.inner_rect;
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
}
