//! Inline function rows and edit-path construction.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

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
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
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
                    #[cfg(test)]
                    FUNCTION_PREVIEWS_BUILT.with(|count| count.set(count.get() + 1));
                    ui.push_id(("function", path), |ui| {
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
        draw_field_help(ui, meta);
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

    // Every function, whatever its game, shows the same row: its summary, the
    // f() button that opens the editor, and the editor itself as a read-only
    // preview.
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
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &label, None);
        Frame::NONE
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0_f32, foundation_group_edge()))
            // A 6-point inset: egui counts the stroke as padding.
            .inner_margin(egui::Margin::same(5))
            .show(ui, |ui| {
                ui.vertical(|ui| {
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
                    #[cfg(test)]
                    FUNCTION_PREVIEWS_BUILT.with(|count| count.set(count.get() + 1));
                    ui.push_id(("wrapped_function", data_path_id(&view)), |ui| {
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
}

#[cfg(test)]
thread_local! {
    /// Read-only function previews this thread built.
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
/// builds a full read-only function editor, graphs sampled and all, so a tag
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
    let spacing = ui.spacing().item_spacing.y;
    if function_rows_culled()
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
    draw(ui);
    let height = ui.cursor().top() - top;
    ui.data_mut(|data| data.insert_temp(id, (key, height)));
}

fn data_path_id(view: &FunctionView) -> &str {
    view.edit
        .as_ref()
        .and_then(|paths| paths.data.data_field_path())
        .unwrap_or("function")
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
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
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
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt((edit.view_scope, edit.tag_key, path, "enum"))
                    .width(240.0)
                    .selected_text(selected_text),
                |ui| {
                    for (index, option) in options.iter().enumerate() {
                        ui.selectable_value(&mut selected, index as i64, *option);
                    }
                },
            );
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
        draw_field_help(ui, meta);
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
