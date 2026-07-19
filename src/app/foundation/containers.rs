//! Struct, block, array, inheritance, and container presentation.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn draw_struct_fields(
    ui: &mut Ui,
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let title = if depth == 0 {
        let cleaned = clean_field_name(tag_struct.name());
        if clean_field_key(&cleaned) == "model" {
            cleaned.to_ascii_uppercase()
        } else {
            format!("Group {}", cleaned.to_ascii_uppercase())
        }
    } else {
        clean_field_name(tag_struct.name())
    };
    let open_override = edit.resolve_open(path_prefix, depth <= 1);
    draw_foundation_group(
        ui,
        title,
        // Index-stripped so the struct's open state survives paging through a
        // parent block/array's element indices (see `strip_node_indices`).
        ("struct", strip_node_indices(path_prefix), depth),
        depth,
        depth <= 1,
        open_override,
        |ui| {
            draw_fields_with_docs(
                ui,
                &tag_struct,
                names,
                depth,
                expert_mode,
                path_prefix,
                edit,
                None,
            );
        },
    );
}

pub(in crate::app) fn draw_inherited_object_fields(
    ui: &mut Ui,
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let chain = inherited_struct_chain(tag_struct);
    if chain.len() <= 1 {
        draw_struct_fields(ui, tag_struct, names, 0, expert_mode, "", edit);
        return;
    }

    for (struct_value, path_prefix) in chain.iter().rev() {
        let title = clean_field_name(struct_value.name()).to_ascii_uppercase();
        let open_override = edit.resolve_open(path_prefix, true);
        draw_foundation_group(
            ui,
            title,
            ("inherited_struct", strip_node_indices(path_prefix)),
            0,
            true,
            open_override,
            |ui| {
                let parent_field = inherited_parent_field_name(*struct_value);
                draw_fields_with_docs(
                    ui,
                    struct_value,
                    names,
                    0,
                    expert_mode,
                    path_prefix,
                    edit,
                    parent_field,
                );
            },
        );
    }
}

pub(in crate::app) fn inherited_struct_chain(
    tag_struct: TagStruct<'_>,
) -> Vec<(TagStruct<'_>, String)> {
    let mut chain = vec![(tag_struct, String::new())];
    let mut current = tag_struct;
    let mut path_prefix = String::new();
    while let Some(parent_field) = inherited_parent_field(current) {
        let Some(parent_struct) = parent_field.as_struct() else {
            break;
        };
        path_prefix = append_field_path(&path_prefix, parent_field.clean_name().as_ref());
        chain.push((parent_struct, path_prefix.clone()));
        current = parent_struct;
    }
    chain
}

pub(in crate::app) fn inherited_parent_field_name(tag_struct: TagStruct<'_>) -> Option<&str> {
    inherited_parent_field(tag_struct).map(|field| field.name())
}

pub(in crate::app) fn inherited_parent_field(tag_struct: TagStruct<'_>) -> Option<TagField<'_>> {
    tag_struct
        .fields()
        .find(|field| field.as_struct().is_some() && is_inherited_parent_name(field.name()))
}

pub(in crate::app) fn is_inherited_parent_name(name: &str) -> bool {
    matches!(
        clean_field_key(name).as_str(),
        "object"
            | "unit"
            | "item"
            | "device"
            | "device machine"
            | "device control"
            | "device light fixture"
    )
}

/// Render a struct's fields, overlaying the JSON-definition docs: inject
/// explanation rows at their authored positions and attach each field's
/// help/units (recovered from the definition, since shipped tags strip them).
/// `skip_field` omits one field by name (used to hide an inherited parent).
pub(in crate::app) fn draw_fields_with_docs(
    ui: &mut Ui,
    tag_struct: &TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
    skip_field: Option<&str>,
) {
    let guid = tag_struct.definition().guid();
    let entries: &[DefEntry] = edit.docs.map(|docs| docs.entries_for(&guid)).unwrap_or(&[]);
    let parent_raw = tag_struct.raw();
    let reference_value_width = shared_tag_reference_value_width(ui, depth);
    // In active filter mode, injected explanation/section headers are suppressed
    // so a filtered view shows only matches and their containers (no orphans).
    let show_explanations = !edit.is_active_filter();
    let mut cursor = 0usize;
    for field in tag_struct.fields_all() {
        if skip_field == Some(field.name()) {
            continue;
        }
        // Find this field's matching definition entry (clean names line up with
        // the engine-stripped tag name); emit any explanations that precede it.

        let mut meta_override = None;
        if !entries.is_empty() {
            let name = field.name();
            if let Some(match_idx) = (cursor..entries.len()).find(|&i| {
                matches!(&entries[i], DefEntry::Field { clean_name, .. } if clean_name == name)
            }) {
                for (offset, entry) in entries[cursor..match_idx].iter().enumerate() {
                    if let DefEntry::Explanation { title, body } = entry {
                        if show_explanations {
                            draw_foundation_explanation_row(
                                ui,
                                title,
                                Some(body),
                                depth,
                                (path_prefix, cursor + offset),
                            );
                        }
                    }
                }
                if let DefEntry::Field {
                    help,
                    unit,
                    range,
                    tag_reference_allowed,
                    ..
                } = &entries[match_idx]
                {
                    // The engine strips everything after `:` from the field name,
                    // so unit/range/help are recovered from the definition here.
                    let mut meta = field_display_meta(name);
                    meta.help = help.clone();
                    meta.unit = unit.clone();
                    meta.range = range.clone();
                    meta.tag_reference_allowed = tag_reference_allowed.clone();
                    meta_override = Some(meta);
                }
                cursor = match_idx + 1;
            }
        }
        // Resolve a block-index field's target block (sibling or ancestor) for
        // the element dropdown; `None` falls back to the numeric editor.
        let root = edit.root;
        let block_index = block_index_target_options(tag_struct, &field, names, root, path_prefix);
        let semantic_short_index = semantic_short_index_target_options(
            ui,
            edit,
            tag_struct,
            &field,
            names,
            root,
            path_prefix,
        );
        draw_field(
            ui,
            field,
            parent_raw,
            names,
            depth,
            expert_mode,
            path_prefix,
            edit,
            meta_override,
            block_index,
            semantic_short_index,
            reference_value_width,
        );
    }
    // Any explanations after the last matched field.
    for (offset, entry) in entries[cursor..].iter().enumerate() {
        if let DefEntry::Explanation { title, body } = entry {
            if show_explanations {
                draw_foundation_explanation_row(
                    ui,
                    title,
                    Some(body),
                    depth,
                    (path_prefix, cursor + offset),
                );
            }
        }
    }
}

pub(in crate::app) fn draw_field(
    ui: &mut Ui,
    field: TagField<'_>,
    parent_raw: &[u8],
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
    meta_override: Option<FieldDisplayMeta>,
    block_index: Option<(Vec<String>, String)>,
    semantic_short_index: Option<(Vec<String>, String)>,
    tag_reference_value_width: f32,
) {
    let field_path = append_field_path_for(path_prefix, &field);
    ui.data_mut(|data| {
        data.insert_temp(
            find_render_cell_id(),
            FindRenderCell {
                tag_key: edit.tag_key.to_owned(),
                field_path: field_path.clone(),
            },
        )
    });
    // Active (filter) field-search: hide everything that isn't a match, an
    // ancestor container of one, or inside a name-matched container.
    if !edit.field_visible(&field_path) {
        return;
    }
    // `meta_override` carries help/units recovered from the JSON definition
    // (shipped tags strip them); fall back to parsing the tag's own field name.
    let meta = meta_override.unwrap_or_else(|| field_display_meta(field.name()));
    if meta.advanced && !expert_mode {
        return;
    }
    if is_internal_schema_marker_name(field.name()) {
        return;
    }
    // Field navigation is shared by reference jumps and Find. Consume the
    // one-shot target before dispatching by field type so functions, explanation
    // rows, and container headers scroll just as scalar value rows do.
    let scroll_here = edit.field_nav.is_some()
        && ui
            .data(|d| d.get_temp::<String>(field_jump_target_id()))
            .as_deref()
            == Some(field_path.as_str());
    if scroll_here {
        let target = egui::Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(ui.available_width().max(1.0), 24.0),
        );
        ui.scroll_to_rect(target, Some(egui::Align::Center));
        ui.data_mut(|d| {
            d.remove::<String>(field_jump_target_id());
            if d.get_temp::<String>(jump_target_id()).as_deref() == Some(field_path.as_str()) {
                d.remove::<String>(jump_target_id());
            }
        });
        ui.ctx().request_repaint();
    }
    match field.field_type() {
        TagFieldType::Terminator
        | TagFieldType::Pad
        | TagFieldType::UselessPad
        | TagFieldType::Skip
        | TagFieldType::Unknown => {
            return;
        }
        TagFieldType::Explanation => {
            // Note: shipped tags strip explanation fields from their layout, so
            // this rarely fires — explanations are normally injected from the
            // definition docs in `draw_fields_with_docs`.
            draw_foundation_explanation_row(
                ui,
                field.name(),
                field.explanation(),
                depth,
                &field_path,
            );
            return;
        }
        _ => {}
    }
    // Reference-jump glow/scroll: pulse and scroll to the field a "References to"
    // jump landed on. The input clock and temp-data are only touched while a nav
    // is actually in flight.
    let glow = edit
        .field_nav
        .is_some_and(|_| edit.field_nav_glow(&field_path, ui.input(|i| i.time)));
    let glow_fill = egui::Color32::from_rgba_unmultiplied(255, 214, 0, 38);
    if let Some(function) = field.as_function() {
        if glow {
            egui::Frame::none().fill(glow_fill).show(ui, |ui| {
                draw_foundation_function_row(ui, &meta, &function, depth, &field_path, edit);
            });
        } else {
            draw_foundation_function_row(ui, &meta, &function, depth, &field_path, edit);
        }
        return;
    }
    if let Some(value) = field_value_with_legacy_inline_old_string_id(field, parent_raw) {
        if is_hidden_non_expert_value(&value, expert_mode) {
            return;
        }
        if glow || scroll_here {
            let fill = if glow {
                glow_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            let framed = egui::Frame::none().fill(fill).show(ui, |ui| {
                draw_foundation_value_row(
                    ui,
                    field,
                    &meta,
                    field.type_name(),
                    &value,
                    names,
                    depth,
                    &field_path,
                    edit,
                    block_index.as_ref(),
                    semantic_short_index.as_ref(),
                    tag_reference_value_width,
                );
            });
            if scroll_here {
                ui.scroll_to_rect(framed.response.rect, Some(egui::Align::Center));
                ui.data_mut(|d| d.remove::<String>(field_jump_target_id()));
                ui.ctx().request_repaint();
            }
        } else {
            draw_foundation_value_row(
                ui,
                field,
                &meta,
                field.type_name(),
                &value,
                names,
                depth,
                &field_path,
                edit,
                block_index.as_ref(),
                semantic_short_index.as_ref(),
                tag_reference_value_width,
            );
        }
        return;
    }

    if let Some(nested) = field.as_struct() {
        if let Some((function_view, data_path)) =
            inline_mapping_function_from_struct(nested, &field_path)
        {
            draw_foundation_inline_function_row(
                ui,
                inline_function_label(field.name(), path_prefix),
                function_view,
                depth,
                &data_path,
                edit,
            );
            return;
        }
        // A struct is a single fixed sub-structure (not a paginated collection
        // like a block/array), so show it expanded by default — matching
        // Foundation/Guerilla. The user can still collapse it, and that choice
        // persists (collapse state is keyed index-free; see `strip_node_indices`).

        let nested_default_open = true;
        let open_override = edit.resolve_open(&field_path, nested_default_open);
        draw_foundation_group(
            ui,
            visible_container_title(field.name(), path_prefix),
            ("field_struct", strip_node_indices(&field_path)),
            depth + 1,
            nested_default_open,
            open_override,
            |ui| {
                draw_struct_fields_inline(
                    ui,
                    nested,
                    names,
                    depth + 1,
                    expert_mode,
                    &field_path,
                    edit,
                )
            },
        );
    } else if let Some(block) = field.as_block() {
        draw_foundation_block(
            ui,
            field.name(),
            block,
            names,
            depth,
            expert_mode,
            &field_path,
            edit,
        );
    } else if let Some(array) = field.as_array() {
        draw_foundation_array(
            ui,
            field.name(),
            array,
            names,
            depth,
            expert_mode,
            &field_path,
            edit,
        );
    } else if let Some(resource) = field.as_resource() {
        draw_resource(
            ui,
            field.name(),
            resource,
            names,
            depth,
            expert_mode,
            &field_path,
            edit,
        );
    } else {
        draw_foundation_text_row(ui, field.name(), "unavailable", field.type_name(), depth);
    }
}

pub(in crate::app) fn draw_struct_fields_inline(
    ui: &mut Ui,
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    draw_fields_with_docs(
        ui,
        &tag_struct,
        names,
        depth,
        expert_mode,
        path_prefix,
        edit,
        None,
    );
}

pub(in crate::app) fn draw_foundation_explanation_row(
    ui: &mut Ui,
    name: &str,
    body: Option<&str>,
    depth: usize,
    id_salt: impl std::hash::Hash,
) {
    // `name` is the explanation's title (often a section header like
    // "$$$ WEAPON $$$", sometimes empty); `body` is its text, read straight
    // from the loaded layout (the schema `definition`), with a hardcoded
    // fallback for the few explanations whose text isn't in the definition.
    //
    // Rendered like Foundation's explanation panel: a collapsible (default-open)
    // bold header bar with a wrapped monospace body — not the previous tiny text.
    let title = clean_field_name(name);
    let body = body
        .map(str::to_owned)
        .or_else(|| known_explanation_text(name))
        .unwrap_or_default();
    let has_body = !body.trim().is_empty();
    if title.is_empty() && !has_body {
        return;
    }
    let header = if title.is_empty() {
        "(explanation)".to_owned()
    } else {
        title
    };

    ui.scope(|ui| {
        ui.add_space(2.0);
        // Full-width header bar (see draw_foundation_group), matching Foundation.
        draw_foundation_collapsing_header(
            ui,
            header,
            ("foundation_explanation", id_salt),
            depth,
            true,
            None,
            foundation_section_bar(),
            |ui| {
                if has_body {
                    Frame::none()
                        .fill(foundation_group_bg())
                        .stroke(Stroke::new(1.0, foundation_group_edge()))
                        .inner_margin(egui::Margin {
                            left: 8.0 + depth as f32 * 4.0,
                            right: 8.0,
                            top: 6.0,
                            bottom: 6.0,
                        })
                        .show(ui, |ui| {
                            // The box spans the full parent width (Foundation's
                            // border is Width=Auto in a stretch StackPanel); only the
                            // text itself is capped (~650px) and left-aligned.
                            ui.set_min_width(ui.available_width());
                            let text_width = ui.available_width().min(650.0);
                            ui.scope(|ui| {
                                ui.set_max_width(text_width);
                                ui.label(
                                    RichText::new(body.trim_end())
                                        .color(text_dark())
                                        .monospace()
                                        .size(12.0),
                                );
                            });
                        });
                }
            },
        );
    });
}

pub(super) fn known_explanation_text(name: &str) -> Option<String> {
    (clean_field_key(name) == "screen flash").then(|| {
        "There are seven screen flash types:\n\nNONE: DST'= DST\nLIGHTEN: DST'= DST(1 - A) + C\nDARKEN: DST'= DST(1 - A) - C\nMAX: DST'= MAX[DST(1 - C), (C - A)(1-DST)]\nMIN: DST'= MIN[DST(1 - C), (C + A)(1-DST)]\nTINT: DST'= DST(1 - C) + (A*PIN[2C - 1, 0, 1] + A)(1-DST)\nINVERT: DST'= DST(1 - C) + A)\n\nIn the above equations C and A represent the color and alpha of the screen flash, DST represents the color in the framebuffer before the screen flash is applied, and DST' represents the color after the screen flash is applied.".to_owned()
    })
}

pub(super) fn visible_container_title(name: &str, path_prefix: &str) -> String {
    if is_internal_placeholder_name(name) {
        path_prefix
            .rsplit('/')
            .next()
            .map(strip_index_suffix)
            .filter(|parent| !parent.is_empty())
            .map(clean_field_name)
            .unwrap_or_else(|| "function".to_owned())
    } else {
        clean_field_name(name)
    }
}

pub(super) fn inline_function_label(name: &str, path_prefix: &str) -> String {
    if is_internal_placeholder_name(name) {
        "function".to_owned()
    } else {
        visible_container_title(name, path_prefix)
    }
}

fn is_internal_placeholder_name(name: &str) -> bool {
    matches!(
        internal_marker_key(name).as_str(),
        "dirty whore" | "whore function" | "hide group id" | "end hide group id"
    )
}

pub(super) fn is_internal_schema_marker_name(name: &str) -> bool {
    matches!(
        internal_marker_key(name).as_str(),
        "hide group id" | "end hide group id" | "whore function"
    )
}

fn internal_marker_key(name: &str) -> String {
    clean_field_key(name).replace('_', " ")
}

fn strip_index_suffix(segment: &str) -> &str {
    segment.split_once('[').map_or(segment, |(name, _)| name)
}

fn inline_mapping_function_from_struct(
    tag_struct: TagStruct<'_>,
    struct_path: &str,
) -> Option<(FunctionView, String)> {
    match halo2_function_bytes_from_struct(tag_struct) {
        Some(bytes) if !bytes.is_empty() => {
            let data_path = append_field_path(struct_path, "data");
            if let Some(view) = legacy_mapping_function_view_for_path(&bytes, struct_path) {
                return Some((view, data_path));
            }
            if let Ok(function) = TagFunction::parse(&bytes) {
                return Some((FunctionView::from_function(function), data_path));
            }
        }
        _ => {}
    }

    for field in tag_struct.fields_all() {
        if field.field_type() != TagFieldType::Data {
            continue;
        }
        let data_path = append_field_path(struct_path, field.name());
        if let Some(function) = field.as_function() {
            return Some((FunctionView::from_function(function), data_path));
        }
        let bytes = field.as_data()?.to_vec();
        if bytes.is_empty() {
            continue;
        }
        if let Some(view) = legacy_mapping_function_view(&bytes) {
            return Some((view, data_path));
        }
    }
    None
}

pub(super) fn is_vibration_function_path(path: &str) -> bool {
    let path = internal_marker_key(path);
    (path.contains("low frequency rumble")
        || path.contains("high frequency rumble")
        || path.contains("low frequency vibration")
        || path.contains("high frequency vibration"))
        && (path.contains("dirty whore") || path.contains("function"))
}

fn legacy_mapping_function_view_for_path(bytes: &[u8], path: &str) -> Option<FunctionView> {
    if is_vibration_function_path(path) {
        if let Some(view) = damage_effect_vibration_function_view(bytes) {
            return Some(view);
        }
    }
    legacy_mapping_function_view(bytes)
}

fn damage_effect_vibration_function_view(bytes: &[u8]) -> Option<FunctionView> {
    let h2_legacy = H2LegacyFunctionView::parse_damage_effect_vibration(bytes.to_vec())?;
    let function = decode_hex(&constant_function_hex(0.0))
        .ok()
        .and_then(|data| TagFunction::parse(&data).ok())?;
    Some(FunctionView::from_function(function).with_h2_legacy(h2_legacy))
}

pub(super) fn legacy_mapping_function_view(bytes: &[u8]) -> Option<FunctionView> {
    let h2_legacy = H2LegacyFunctionView::parse(bytes.to_vec())?;
    let function = decode_hex(&constant_function_hex(0.0))
        .ok()
        .and_then(|data| TagFunction::parse(&data).ok())?;
    Some(FunctionView::from_function(function).with_h2_legacy(h2_legacy))
}

pub(in crate::app) fn draw_foundation_group(
    ui: &mut Ui,
    title: String,
    id_salt: impl std::hash::Hash,
    depth: usize,
    default_open: bool,
    // `Some(open)` forces the open-state this frame (Search-fields filter);
    // `None` leaves the node's stored / default state untouched.
    open_override: Option<bool>,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.scope(|ui| {
        ui.add_space(2.0);
        draw_foundation_collapsing_header(
            ui,
            title,
            id_salt,
            depth,
            default_open,
            open_override,
            foundation_section_bar(),
            |ui| {
                Frame::none()
                    .fill(foundation_group_bg())
                    .stroke(Stroke::new(1.0, foundation_group_edge()))
                    .inner_margin(egui::Margin {
                        left: 8.0 + depth as f32 * 4.0,
                        right: 8.0,

                        top: 6.0,
                        bottom: 6.0,
                    })
                    .show(ui, add_contents);
            },
        );
    });
}

/// Draw a full-width modern collapsing header with the same rounded chevron
/// control used by block headers. Keeping this in one helper ensures groups,
/// explanations, and section bars share the same affordance.
#[allow(clippy::too_many_arguments)]
fn draw_foundation_collapsing_header(
    ui: &mut Ui,
    title: String,
    id_salt: impl std::hash::Hash,
    depth: usize,
    default_open: bool,
    open_override: Option<bool>,
    bar_fill: Color32,
    add_contents: impl FnOnce(&mut Ui),
) -> bool {
    let id = ui.make_persistent_id(("foundation_collapsing_header", id_salt));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    if let Some(open) = open_override {
        state.set_open(open);
    }

    let row_width = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(row_width, 28.0), Sense::hover());
    ui.painter().rect_filled(row_rect, 5.0, bar_fill);
    ui.painter()
        .rect_stroke(row_rect, 5.0, Stroke::new(1.0, foundation_block_edge()));
    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(row_rect.shrink2(Vec2::new(6.0, 3.0))),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
            ui.horizontal_centered(|ui| {
                ui.add_space(depth as f32 * 4.0);
                let toggle = foundation_header_toggle_cell(ui, state.is_open(), true);
                if toggle.clicked() {
                    state.toggle(ui);
                }
                let label = if findable_text_has_match(ui, &title, FindTargetKind::Label) {
                    let (label_rect, label) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width().max(80.0), 20.0),
                        Sense::click(),
                    );
                    paint_findable_text(
                        ui,
                        label_rect.left_center(),
                        Align2::LEFT_CENTER,
                        &title,
                        bold_font(12.5),
                        foundation_block_text(),
                        FindTargetKind::Label,
                    );
                    label
                } else {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&title)
                                .color(foundation_block_text())
                                .font(bold_font(12.5)),
                        )
                        .sense(Sense::click()),
                    )
                };
                if label.clicked() {
                    state.toggle(ui);
                }
            });
        },
    );
    state.store(ui.ctx());
    let open = state.is_open();
    if open {
        state.show_body_unindented(ui, add_contents);
        ui.add_space(3.0);
    }
    open
}

pub(in crate::app) fn draw_foundation_block(
    ui: &mut Ui,
    name: &str,
    block: TagBlock<'_>,

    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let count = block.len();
    let sel = block_selected_index(ui, edit, path_prefix, count);
    let selected_label = if count == 0 {
        "NONE".to_owned()
    } else {
        block_element_dropdown_label(block.element(sel), names, sel)
    };

    let block_default_open = depth == 0 || is_priority_section(name);
    let open_override = edit.resolve_open(path_prefix, block_default_open);
    // A clipboard is compatible when it came from the same group + block schema
    // position AND holds elements of the same on-disk size. Element subscripts
    // are stripped so which parent block element is selected doesn't matter —
    // the block's shape is identical across siblings. A size mismatch means a
    // different struct version; surfacing it as `VersionMismatch` keeps a
    // cross-version paste out of the menu (the engine would reject it after the
    // click anyway) and prevents version corruption until upgrade/downgrade.
    let paste_gate = match edit.block_clipboard {
        Some(clip)
            if edit.editable
                && clip.group_tag == edit.group_tag
                && strip_element_indices(&clip.block_path) == strip_element_indices(path_prefix) =>
        {
            if clip.element_size == Some(block.element_size()) {
                PasteGate::Ready(clip.elements.len())
            } else {
                PasteGate::VersionMismatch
            }
        }
        _ => PasteGate::Empty,
    };
    let block_size_label = edit
        .show_block_sizes
        .then(|| format_block_size_label(count, block.element_size()));
    let actions = draw_foundation_block_control(
        ui,
        name,
        &selected_label,
        sel,
        count,
        Some(block.definition().max_count()),
        edit.editable,
        true, // is a real block — add/delete allowed
        edit.view_scope,
        edit.tag_key,
        path_prefix,
        depth,
        block_default_open,
        open_override,
        paste_gate,
        block_size_label.as_deref(),
        |i| block_element_dropdown_label(block.element(i), names, i),
        |ui| {
            if count == 0 {
                ui.label(
                    RichText::new("NONE / empty block")
                        .italics()
                        .color(subtle_dark()),
                );
                return;
            }
            if let Some(element) = block.element(sel) {
                let element_path = format!("{path_prefix}[{sel}]");
                draw_struct_fields_inline(
                    ui,
                    element,
                    names,
                    depth + 1,
                    expert_mode,
                    &element_path,
                    edit,
                );
            }
        },
    );

    handle_block_actions(ui, edit, path_prefix, sel, count, expert_mode, &actions);

    // Copy the selected element, or the whole block, onto the clipboard.
    let copy_indices: Option<Vec<usize>> = if actions.copy {
        Some(vec![sel])
    } else if actions.copy_block {
        Some((0..count).collect())
    } else {
        None
    };
    if let Some(indices) = copy_indices {
        let elements: Vec<_> = indices
            .iter()
            .filter_map(|&i| block.element_snapshot(i))
            .collect();
        if !elements.is_empty() {
            *edit.block_clip_request = Some(BlockClipboard {
                group_tag: edit.group_tag,
                block_path: path_prefix.to_owned(),
                label: clean_field_name(name),
                element_size: Some(block.element_size()),
                elements,
            });
        }
    }

    // Copy the whole block as TSV (plaintext, Excel-friendly).
    if actions.copy_block_tsv && count > 0 {
        let tsv = block_to_tsv(&block, names);
        if !tsv.is_empty() {
            ui.output_mut(|output| output.copied_text = tsv);
        }
    }

    // Request the TSV-import window for this block.
    if actions.paste_tsv && count > 0 {
        *edit.tsv_paste_request = Some(TsvPasteRequest {
            block_path: path_prefix.to_owned(),
            block_label: clean_field_name(name),
            element_count: count,
        });
    }

    // Paste / replace from the clipboard.
    let clip_elements = edit.block_clipboard.map(|clip| clip.elements.clone());
    if let Some(elements) = clip_elements {
        if actions.paste {
            let at = if count == 0 { 0 } else { sel + 1 };
            edit.block_ops.push(BlockOp {
                path: path_prefix.to_owned(),
                kind: BlockOpKind::Paste {
                    at,
                    elements: elements.clone(),
                },
            });
            set_block_selected_index(ui, edit, path_prefix, at);
        }
        if actions.replace_element && count > 0 {
            edit.block_ops.push(BlockOp {
                path: path_prefix.to_owned(),
                kind: BlockOpKind::ReplaceElement {
                    at: sel,
                    elements: elements.clone(),
                },
            });
            set_block_selected_index(ui, edit, path_prefix, sel);
        }
        if actions.replace_block {
            // Destructive (clears the block) — route through the confirm modal.
            *edit.block_confirm = Some(BlockConfirm {
                tag_key: edit.tag_key.to_owned(),
                path: path_prefix.to_owned(),
                kind: BlockOpKind::ReplaceBlock { elements },
                message: format!(
                    "Replace ALL {count} element(s) in this block with {} clipboard element(s)?",
                    edit.block_clipboard.map_or(0, |c| c.elements.len())
                ),
                confirm_label: "Replace".to_owned(),
            });
        }
    }
}

pub(in crate::app) fn format_block_size_label(count: usize, element_size: usize) -> String {
    let total = count.saturating_mul(element_size);
    format!(
        "{} x {} B = {}",
        count,
        element_size,
        format_byte_count(total)
    )
}

pub(in crate::app) fn format_byte_count(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f32 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f32 / (1024.0 * 1024.0))
    }
}

/// Translate header button clicks into block selection changes / deferred ops.
pub(in crate::app) fn handle_block_actions(
    ui: &Ui,
    edit: &mut FieldEditContext<'_>,
    path: &str,
    sel: usize,
    count: usize,
    expert_mode: bool,
    actions: &BlockHeaderActions,
) {
    if let Some(new_sel) = actions.new_selection {
        set_block_selected_index(ui, edit, path, new_sel);
    }
    if actions.add {
        edit.block_ops.push(BlockOp {
            path: path.to_owned(),
            kind: BlockOpKind::Add,
        });
        // Select the new (appended) element next frame.
        set_block_selected_index(ui, edit, path, count);
    }
    if actions.insert {
        edit.block_ops.push(BlockOp {
            path: path.to_owned(),
            kind: BlockOpKind::Insert(sel),
        });
        set_block_selected_index(ui, edit, path, sel);
    }
    if actions.duplicate {
        edit.block_ops.push(BlockOp {
            path: path.to_owned(),
            kind: BlockOpKind::Duplicate(sel),
        });
        set_block_selected_index(ui, edit, path, sel + 1);
    }
    if actions.delete && count > 0 {
        if expert_mode {
            edit.block_ops.push(BlockOp {
                path: path.to_owned(),
                kind: BlockOpKind::Delete(sel),
            });
            set_block_selected_index(ui, edit, path, sel.saturating_sub(1));
        } else {
            *edit.block_confirm = Some(BlockConfirm {
                tag_key: edit.tag_key.to_owned(),
                path: path.to_owned(),
                kind: BlockOpKind::Delete(sel),
                message: format!("Delete element {sel} of {count} from this block?"),
                confirm_label: "Delete".to_owned(),
            });
        }
    }
    if actions.delete_all && count > 0 {
        *edit.block_confirm = Some(BlockConfirm {
            tag_key: edit.tag_key.to_owned(),

            path: path.to_owned(),
            kind: BlockOpKind::DeleteAll,
            message: format!("Delete ALL {count} elements from this block?"),
            confirm_label: "Delete".to_owned(),
        });
    }
}

pub(in crate::app) fn block_element_dropdown_label(
    element: Option<TagStruct<'_>>,
    names: &TagNameIndex,
    index: usize,
) -> String {
    let Some(element) = element else {
        return format!("{index}.");
    };
    block_element_content_label(element, names)
        .map(|label| format!("{index}. {label}"))
        .unwrap_or_else(|| format!("{index}. {}", element.name()))
}

pub(in crate::app) fn block_element_content_label(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    first_named_string_label(element)
        .or_else(|| first_tag_reference_label(element, names))
        .or_else(|| first_string_label(element))
        .or_else(|| first_scalar_label(element, names))
}

pub(in crate::app) fn first_tag_reference_label(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        match field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            Some(TagFieldData::TagReference(reference))
                if reference.group_tag_and_name.is_some() =>
            {
                let label =
                    format_foundation_scalar_value(names, &TagFieldData::TagReference(reference));
                if !label.trim().is_empty() && label != "NONE" {
                    return Some(label);
                }
            }
            _ => {}
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_tag_reference_label(nested, names) {
                return Some(label);
            }
        }
    }
    None
}

pub(in crate::app) fn first_named_string_label(element: TagStruct<'_>) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        match field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            Some(value) if is_name_like_field(field.name()) => {
                if let Some(label) = stringish_label(&value) {
                    return Some(label);
                }
            }
            _ => {}
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_named_string_label(nested) {
                return Some(label);
            }
        }
    }

    None
}

pub(in crate::app) fn first_string_label(element: TagStruct<'_>) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        let local_value = field_value_with_legacy_inline_old_string_id(field, parent_raw);
        if let Some(label) = local_value.as_ref().and_then(stringish_label) {
            return Some(label);
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_string_label(nested) {
                return Some(label);
            }
        }
    }
    None
}

pub(in crate::app) fn first_scalar_label(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        match field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            Some(value) if scalar_is_useful_for_block_label(&value) => {
                let value = format_foundation_scalar_value(names, &value);
                if label_has_content(&value) {
                    return Some(format!("{}: {value}", clean_field_name(field.name())));
                }
            }
            _ => {}
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_scalar_label(nested, names) {
                return Some(label);
            }
        }
    }
    None
}

pub(in crate::app) fn field_value_with_legacy_inline_old_string_id(
    field: TagField<'_>,
    parent_raw: &[u8],
) -> Option<TagFieldData> {
    if let Some(value) = field.value() {
        return Some(value);
    }
    legacy_inline_old_string_id(field, parent_raw)
        .map(|string| TagFieldData::OldStringId(StringIdData { string }))
}

pub(in crate::app) fn legacy_inline_old_string_id(
    field: TagField<'_>,
    parent_raw: &[u8],
) -> Option<String> {
    if field.field_type() != TagFieldType::OldStringId {
        return None;
    }
    let offset = field.definition().offset() as usize;
    let bytes = parent_raw.get(offset..offset + 32)?;
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    let value = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    if value.is_empty() {
        return None;
    }
    if !value.bytes().all(|byte| matches!(byte, 0x20..=0x7e)) {
        return None;
    }
    Some(value.to_owned())
}

pub(in crate::app) fn is_name_like_field(name: &str) -> bool {
    let clean = clean_field_name(name).to_ascii_lowercase();
    clean == "name"
        || clean.ends_with(" name")
        || clean.contains("material")
        || clean.contains("permutation")
        || clean.contains("region")
        || clean.contains("variant")
        || clean.contains("marker")
        || clean.contains("node")
}

pub(in crate::app) fn stringish_label(value: &TagFieldData) -> Option<String> {
    let raw = match value {
        TagFieldData::String(text) | TagFieldData::LongString(text) => text.as_str(),
        TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => id.string.as_str(),
        _ => return None,
    };
    let label = trim_formatted_value(raw);
    label_has_content(&label).then_some(label)
}

pub(in crate::app) fn scalar_is_useful_for_block_label(value: &TagFieldData) -> bool {
    matches!(
        value,
        TagFieldData::CharEnum { name: Some(_), .. }
            | TagFieldData::ShortEnum { name: Some(_), .. }
            | TagFieldData::LongEnum { name: Some(_), .. }
            | TagFieldData::CharBlockIndex(_)
            | TagFieldData::CustomCharBlockIndex(_)
            | TagFieldData::ShortBlockIndex(_)
            | TagFieldData::CustomShortBlockIndex(_)
            | TagFieldData::LongBlockIndex(_)
            | TagFieldData::CustomLongBlockIndex(_)
            | TagFieldData::CharInteger(_)
            | TagFieldData::ShortInteger(_)
            | TagFieldData::LongInteger(_)
            | TagFieldData::ByteInteger(_)
            | TagFieldData::WordInteger(_)
            | TagFieldData::DwordInteger(_)
    )
}

pub(in crate::app) fn label_has_content(label: &str) -> bool {
    let trimmed = label.trim();
    !trimmed.is_empty() && trimmed != "NONE"
}

pub(in crate::app) fn draw_foundation_array(
    ui: &mut Ui,
    name: &str,
    array: blam_tags::TagArray<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let count = array.len();
    let sel = block_selected_index(ui, edit, path_prefix, count);
    let selected_label = if count == 0 {
        "NONE".to_owned()
    } else {
        block_element_dropdown_label(array.element(sel), names, sel)
    };
    let open_override = edit.resolve_open(path_prefix, depth == 0);
    // A clipboard is compatible when it came from this same array schema
    // position. Element subscripts are stripped so which parent block element is
    // selected doesn't matter — the array's shape is identical across siblings.
    // Arrays are fixed-count inline structs, not FieldSet-versioned, so there's
    // no version dimension to gate on (only group + schema position).
    let paste_gate = match edit.block_clipboard {
        Some(clip)
            if edit.editable
                && clip.group_tag == edit.group_tag
                && strip_element_indices(&clip.block_path) == strip_element_indices(path_prefix) =>
        {
            PasteGate::Ready(clip.elements.len())
        }
        _ => PasteGate::Empty,
    };
    let actions = draw_foundation_block_control(
        ui,
        name,
        &selected_label,
        sel,
        count,
        None, // arrays are fixed-size — capacity gate not applicable
        edit.editable,
        false, // arrays are fixed-size — no add/delete
        edit.view_scope,
        edit.tag_key,
        path_prefix,
        depth,
        depth == 0,
        open_override,
        paste_gate,
        None,
        |i| block_element_dropdown_label(array.element(i), names, i),
        |ui| {
            if count == 0 {
                ui.label(
                    RichText::new("NONE / empty array")
                        .italics()
                        .color(subtle_dark()),
                );
                return;
            }
            if let Some(element) = array.element(sel) {
                let element_path = format!("{path_prefix}[{sel}]");
                draw_struct_fields_inline(
                    ui,
                    element,
                    names,
                    depth + 1,
                    expert_mode,
                    &element_path,
                    edit,
                );
            }
        },
    );
    // Arrays support selection, read-only copy/TSV, and in-place replace of an
    // element (their fixed count rules out insert/delete).
    if let Some(new_sel) = actions.new_selection {
        set_block_selected_index(ui, edit, path_prefix, new_sel);
    }
    let copy_indices: Option<Vec<usize>> = if actions.copy {
        Some(vec![sel])
    } else if actions.copy_block {
        Some((0..count).collect())
    } else {
        None
    };
    if let Some(indices) = copy_indices {
        let elements: Vec<_> = indices
            .iter()
            .filter_map(|&i| array.element_snapshot(i))
            .collect();
        if !elements.is_empty() {
            *edit.block_clip_request = Some(BlockClipboard {
                group_tag: edit.group_tag,
                block_path: path_prefix.to_owned(),
                label: clean_field_name(name),
                // Inline arrays aren't FieldSet-versioned and expose no element-
                // size accessor — no version dimension to gate on.
                element_size: None,
                elements,
            });
        }
    }
    if actions.replace_element && count > 0 {
        if let Some(elements) = edit.block_clipboard.map(|clip| clip.elements.clone()) {
            edit.block_ops.push(BlockOp {
                path: path_prefix.to_owned(),
                kind: BlockOpKind::ReplaceElement { at: sel, elements },
            });
            set_block_selected_index(ui, edit, path_prefix, sel);
        }
    }
    if actions.copy_block_tsv && count > 0 {
        let tsv = array_to_tsv(&array, names);

        if !tsv.is_empty() {
            ui.output_mut(|output| output.copied_text = tsv);
        }
    }
}

/// The parent block/array path for a block path, for "jump to parent". Strips
/// the last `/segment` and a trailing element index, e.g.
/// `regions[0]/permutations` → `regions`. `None` for a top-level block.
pub(in crate::app) fn parent_block_path(path: &str) -> Option<String> {
    let cut = path.rfind('/')?;
    let mut parent = path[..cut].to_string();
    if parent.ends_with(']') {
        if let Some(open) = parent.rfind('[') {
            parent.truncate(open);
        }
    }
    Some(parent)
}

/// A readable breadcrumb for a block path: cleaned segments (index/ordinal
/// suffixes dropped) joined with ` › `, e.g. `regions[0]/permutations` →
/// `regions › permutations`. Backed by the engine's `TagFieldPath`.
pub(super) fn breadcrumb_for_path(path: &str) -> String {
    blam_tags::TagFieldPath::parse(path).breadcrumb()
}

/// egui-memory key holding the block path that a pending "jump to parent" should
/// scroll into view on the next frame.
pub(in crate::app) fn jump_target_id() -> egui::Id {
    egui::Id::new("foundation_jump_to_block")
}

/// egui-memory key holding the exact (indexed) field path that a pending
/// reference-jump should scroll into view and pulse on the next frame. Consumed
/// by [`draw_field`] when it draws the matching leaf.
pub(in crate::app) fn field_jump_target_id() -> egui::Id {
    egui::Id::new("foundation_jump_to_field")
}

/// Whether the block clipboard can paste into the block/array whose header is
/// being drawn — decided up-front so the menu mirrors exactly what the engine
/// will accept (see `blam_tags::TagBlockMut::paste_element`).
#[derive(Clone, Copy)]
pub(in crate::app) enum PasteGate {
    /// No clipboard, or a clipboard from a different group / schema position —
    /// paste is simply unavailable and needs no explanation.
    Empty,
    /// A compatible clipboard holding `n` element(s) — paste / replace enabled.
    Ready(usize),
    /// A clipboard targets this same block but its elements are a different
    /// on-disk size, i.e. a different struct version. Disabled with a hover so
    /// the user learns why — guards against cross-version corruption until
    /// upgrade/downgrade lands.
    VersionMismatch,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_foundation_block_control(
    ui: &mut Ui,
    name: &str,
    selected_label: &str,
    selected_index: usize,
    count: usize,
    // Schema element-count cap (`TagBlockDefinition::max_count()`); `0` or
    // `None` means unbounded. Gates the grow buttons at capacity.
    max_count: Option<u32>,
    editable: bool,
    allow_structural: bool,
    view_scope: &str,
    tag_key: &str,
    path_salt: &str,
    depth: usize,
    default_open: bool,
    // `Some(open)` forces the open-state this frame (Search-fields filter).
    open_override: Option<bool>,
    // Whether the clipboard can paste here — gates the paste / replace menu
    // items and explains a blocked cross-version paste.
    paste_gate: PasteGate,
    block_size_label: Option<&str>,
    element_label: impl Fn(usize) -> String,
    add_contents: impl FnOnce(&mut Ui),
) -> BlockHeaderActions {
    let mut actions = BlockHeaderActions::default();
    // Key collapse state on the index-stripped path so a nested block/array
    // stays open/closed as the user pages through a parent element's indices
    // (matching resolve_open / search-highlight, which also ignore indices).
    // `path_salt` keeps its indexed form for the jump-to-block scroll below.
    let canonical_path = strip_node_indices(path_salt);

    let id = ui.make_persistent_id((
        "foundation_block_control",
        view_scope,
        tag_key,
        canonical_path.as_str(),
        depth,
        name,
    ));
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open && count > 0,
    );
    // Search-fields filter: force this block open/closed for the apply frame.
    // An empty block can never be opened.
    if let Some(open) = open_override {
        state.set_open(open && count > 0);
    }
    if count == 0 && state.is_open() {
        state.set_open(false);
    }

    let row_width = ui.available_width();
    let row_height = 30.0;
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::hover());
    let row_hovered = ui.rect_contains_pointer(row_rect);
    ui.painter().rect_filled(
        row_rect,
        5.0,
        if row_hovered {
            foundation_block_bar_hover()
        } else {
            foundation_block_bar()
        },
    );
    ui.painter()
        .rect_stroke(row_rect, 5.0, Stroke::new(1.0, foundation_block_edge()));

    // 3.4 jump-to-parent: if a child's "↑" targeted this block last frame, bring
    // its header into view (and clear the pending target).
    if ui
        .data(|d| d.get_temp::<String>(jump_target_id()))
        .as_deref()
        == Some(path_salt)
    {
        ui.scroll_to_rect(row_rect, Some(egui::Align::Center));
        ui.data_mut(|d| d.remove::<String>(jump_target_id()));
    }

    // At-capacity / empty gating mirrors Guerilla's enable rules.
    let (can_edit, has_sel, at_capacity, capacity_hint) =
        prepare_block_control_availability(editable, allow_structural, count, max_count);
    let mut selector_active = false;

    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(row_rect.shrink2(Vec2::new(6.0, 4.0))),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
            ui.horizontal_centered(|ui| {
                ui.add_space(depth as f32 * 5.0);
                // Jump-to-parent (nested blocks only); hover shows the breadcrumb.
                if depth > 0 {
                    let jump = icon_button(
                        ui,
                        ButtonIcon::JumpUp,
                        "Jump to parent block",
                        true,
                        Vec2::new(24.0, 22.0),
                        foundation_block_text(),
                    )
                    .on_hover_text(format!(
                        "Jump to parent block\n{}",
                        breadcrumb_for_path(path_salt)
                    ));
                    if jump.clicked() {
                        if let Some(parent) = parent_block_path(path_salt) {
                            ui.data_mut(|d| d.insert_temp(jump_target_id(), parent));
                        }
                    }
                }
                let toggle = foundation_header_toggle_cell(ui, state.is_open(), count > 0);
                if toggle.clicked() && count > 0 {
                    state.toggle(ui);
                }
                let clean_name = clean_field_name(name);
                let name_label = if findable_text_has_match(ui, &clean_name, FindTargetKind::Label)
                {
                    let (name_rect, name_label) =
                        ui.allocate_exact_size(Vec2::new(190.0, 20.0), Sense::click());
                    paint_findable_text(
                        ui,
                        name_rect.left_center(),
                        Align2::LEFT_CENTER,
                        &clean_name,
                        bold_font(12.5),
                        foundation_block_text(),
                        FindTargetKind::Label,
                    );
                    name_label
                } else {
                    ui.add_sized(
                        [190.0, 20.0],
                        egui::Label::new(
                            RichText::new(clean_name)
                                .color(foundation_block_text())
                                .font(bold_font(12.5)),
                        )
                        .sense(Sense::click()),
                    )
                };
                // Right-click the block name → copy / paste menu. Copy actions are
                // read-only and available for any element collection (including
                // fixed-size arrays); the size/content-changing paste & replace
                // actions are gated behind `allow_structural`.
                name_label
                    .on_hover_text("Right-click for copy / paste options")
                    .context_menu(|ui| {
                        // Copy + in-place replace are valid for blocks AND fixed-size
                        // arrays (no element-count change). The size-changing actions
                        // (paste/insert, replace-all, add/delete) are blocks only.
                        if ui
                            .add_enabled(count > 0, egui::Button::new("Copy element"))
                            .clicked()
                        {
                            actions.copy = true;
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(count > 0, egui::Button::new("Copy entire block"))
                            .clicked()
                        {
                            actions.copy_block = true;
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(count > 0, egui::Button::new("Copy block as TSV"))
                            .on_hover_text("Copy all elements as tab-separated rows (Excel)")
                            .clicked()
                        {
                            actions.copy_block_tsv = true;
                            ui.close_menu();
                        }
                        // In-place replace of the selected element — never changes
                        // the count, so it works for arrays too.
                        if matches!(paste_gate, PasteGate::Ready(_))
                            && ui
                                .add_enabled(
                                    count > 0,
                                    egui::Button::new("Replace selected element"),
                                )
                                .on_hover_text("Overwrite the selected element with the clipboard")
                                .clicked()
                        {
                            actions.replace_element = true;
                            ui.close_menu();
                        }
                        if allow_structural {
                            if ui
                                .add_enabled(count > 0, egui::Button::new("Paste TSV…"))
                                .on_hover_text(
                                    "Paste tab-separated rows back onto this block's elements",
                                )
                                .clicked()
                            {
                                actions.paste_tsv = true;

                                ui.close_menu();
                            }
                            ui.separator();
                            match paste_gate {
                                PasteGate::Ready(n) => {
                                    let noun = if n == 1 { "element" } else { "elements" };
                                    if ui.button(format!("Paste {n} {noun}")).clicked() {
                                        actions.paste = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Replace entire block").clicked() {
                                        actions.replace_block = true;
                                        ui.close_menu();
                                    }
                                }
                                PasteGate::VersionMismatch => {
                                    ui.add_enabled(false, egui::Button::new("Paste"))
                                        .on_disabled_hover_text(
                                            "Clipboard element is a different struct version \
                                             (different on-disk size) — pasting across versions \
                                             would corrupt the tag. Upgrade/downgrade between \
                                             versions isn't supported yet.",
                                        );
                                }
                                PasteGate::Empty => {
                                    ui.add_enabled(false, egui::Button::new("Paste"));
                                }
                            }
                        }
                    });
                // Keep the previous arrow directly beside the selected
                // reference, with the next arrow following it.
                if foundation_header_stepper_clicked(ui, "<", has_sel && selected_index > 0) {
                    actions.new_selection = Some(selected_index.saturating_sub(1));
                }

                // Instance selector dropdown — built lazily (only when open).
                let combo_width = foundation_selected_width(row_width);
                if has_sel {
                    let (combo_response, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt((
                            "block_instance",
                            view_scope,
                            tag_key,
                            path_salt,
                            depth,
                        ))
                        .selected_text(truncate_for_cell(selected_label, combo_width - 24.0))
                        .width(combo_width),
                        |ui| {
                            selector_active |= ui.rect_contains_pointer(ui.max_rect());
                            for i in 0..count {
                                if ui
                                    .selectable_label(i == selected_index, element_label(i))
                                    .clicked()
                                {
                                    actions.new_selection = Some(i);
                                }
                            }
                        },
                    );
                    selector_active |=
                        combo_response.response.hovered() || combo_response.response.has_focus();
                    if let Some(delta) = wheel_delta {
                        if let Some(next) = combo_scroll_next_index(selected_index, count, delta) {
                            actions.new_selection = Some(next);
                        }
                    }
                } else {
                    foundation_header_value_cell(ui, "NONE", combo_width);
                }

                // Next stepper follows the selected reference string.
                if foundation_header_stepper_clicked(ui, ">", has_sel && selected_index + 1 < count)
                {
                    actions.new_selection = Some(selected_index + 1);
                }

                // Index readout.
                ui.label(
                    RichText::new(if has_sel {
                        format!("[{selected_index}]")
                    } else {
                        "[--]".to_owned()
                    })
                    .color(foundation_block_text())
                    .small(),
                );

                if let Some(size_label) = block_size_label {
                    ui.label(
                        RichText::new(size_label)
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    )
                    .on_hover_text("Block memory usage: elements × element byte size");
                }

                // Structural edit buttons — only for variable-count blocks. Arrays
                // are fixed-size, so the count-changing actions don't apply and the
                // buttons are omitted entirely. The grow actions (Add / Insert /
                // Duplicate) are disabled once the block hits its schema cap.
                if allow_structural {
                    let hint = capacity_hint.as_deref();
                    if foundation_header_button_clicked_hint(
                        ui,
                        "Add",
                        can_edit && !at_capacity,
                        hint,
                    ) {
                        actions.add = true;
                    }
                    if foundation_header_button_clicked_hint(
                        ui,
                        "Insert",
                        can_edit && has_sel && !at_capacity,
                        hint,
                    ) {
                        actions.insert = true;
                    }
                    if foundation_header_button_clicked_hint(
                        ui,
                        "Duplicate",
                        can_edit && has_sel && !at_capacity,
                        hint,
                    ) {
                        actions.duplicate = true;
                    }
                    if foundation_header_button_clicked(ui, "Delete", can_edit && has_sel) {
                        actions.delete = true;
                    }
                    if foundation_header_button_clicked(ui, "Delete all", can_edit && has_sel) {
                        actions.delete_all = true;
                    }
                }
            });
        },
    );

    if has_sel && selector_active {
        let navigation_delta = ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowUp) {
                -1
            } else if input.key_pressed(egui::Key::ArrowDown) {
                1
            } else {
                0
            }
        });
        if count > 1 {
            if navigation_delta < 0 && selected_index > 0 {
                actions.new_selection = Some(selected_index - 1);
            } else if navigation_delta > 0 && selected_index + 1 < count {
                actions.new_selection = Some(selected_index + 1);
            }
        }
    }

    state.store(ui.ctx());

    if count == 0 {
        return actions;
    }

    state.show_body_unindented(ui, |ui| {
        Frame::none()
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0, foundation_group_edge()))
            .inner_margin(egui::Margin {
                left: 14.0 + depth as f32 * 5.0,
                right: 8.0,
                top: 8.0,
                bottom: 8.0,
            })
            // Render the body inline — no nested ScrollArea. The single outer
            // ScrollArea in `draw_tag_fields_scroll` owns all scrolling, so a
            // block element expands to its full height instead of growing an
            // inner scrollbar (which also let cross-boundary scroll-to fail).
            .show(ui, add_contents);
    });

    actions
}

fn prepare_block_control_availability(
    editable: bool,
    allow_structural: bool,
    count: usize,
    max_count: Option<u32>,
) -> (bool, bool, bool, Option<String>) {
    let can_edit = editable && allow_structural;
    let has_sel = count > 0;
    // `max_count` of 0 in the schema means "unbounded".
    let capacity = max_count.filter(|&m| m != 0).map(|m| m as usize);
    let at_capacity = capacity.is_some_and(|m| count >= m);
    let capacity_hint = capacity
        .filter(|_| at_capacity)
        .map(|m| format!("Block is at its schema maximum of {m} element(s)"));
    (can_edit, has_sel, at_capacity, capacity_hint)
}

pub(in crate::app) fn consume_mouse_wheel(ui: &Ui) {
    consume_mouse_wheel_ctx(ui.ctx());
}

fn consume_mouse_wheel_ctx(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        input
            .events
            .retain(|event| !matches!(event, egui::Event::MouseWheel { .. }));
        input.raw_scroll_delta = Vec2::ZERO;
        input.smooth_scroll_delta = Vec2::ZERO;
    });
}

pub(in crate::app) fn set_combo_scroll_cycle_enabled(ctx: &egui::Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(combo_scroll_cycle_enabled_id(), enabled));
}

fn combo_scroll_cycle_enabled(ui: &Ui) -> bool {
    ui.data(|data| {
        data.get_temp::<bool>(combo_scroll_cycle_enabled_id())
            .unwrap_or(true)
    })
}

pub(in crate::app) fn combo_box_with_scroll<R>(
    ui: &mut Ui,
    combo: egui::ComboBox,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (egui::InnerResponse<Option<R>>, Option<i32>) {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = 20.0;
            ui.spacing_mut().button_padding.y = 2.0;
            combo.show_ui(ui, add_contents)
        })
        .inner;
    let popup_open = response.inner.is_some();
    let delta = dropdown_wheel_delta(ui, &response.response, popup_open);
    (response, delta)
}

pub(in crate::app) fn combo_scroll_next_index(
    current: usize,
    len: usize,
    delta: i32,
) -> Option<usize> {
    if len == 0 || delta == 0 {
        return None;
    }

    let delta = delta.signum();
    let current = current.min(len - 1);
    let next = (current as i32 + delta).clamp(0, len as i32 - 1) as usize;
    (next != current).then_some(next)
}

pub(in crate::app) fn combo_scroll_next_i64(
    current: i64,
    min: i64,
    max: i64,
    delta: i32,
) -> Option<i64> {
    if min > max || delta == 0 {
        return None;
    }
    let delta = delta.signum() as i64;
    let current = current.clamp(min, max);
    let next = (current + delta).clamp(min, max);
    (next != current).then_some(next)
}

pub(in crate::app) fn dropdown_wheel_delta(
    ui: &Ui,
    response: &egui::Response,
    popup_open: bool,
) -> Option<i32> {
    if popup_open || !combo_scroll_cycle_enabled(ui) {
        return None;
    }
    let hovered = ui
        .ctx()
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|hover_pos| response.rect.contains(hover_pos));
    if !hovered {
        return None;
    }
    let delta = wheel_event_delta_from_context(ui.ctx());
    consume_mouse_wheel(ui);
    if delta == 0 {
        return None;
    }
    Some(delta)
}

fn wheel_event_delta_from_context(ctx: &egui::Context) -> i32 {
    ctx.input(|input| {
        let wheel_y = input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum::<f32>();
        if wheel_y > f32::EPSILON {
            -1
        } else if wheel_y < -f32::EPSILON {
            1
        } else {
            0
        }
    })
}

fn combo_scroll_cycle_enabled_id() -> egui::Id {
    egui::Id::new("combo_scroll_cycle_enabled")
}

pub(in crate::app) fn foundation_header_toggle_cell(
    ui: &mut Ui,
    open: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(24.0, 22.0), Sense::click());
    let fill = if enabled {
        foundation_disclosure_bg()
    } else if is_dark_mode() {
        Color32::from_rgb(52, 52, 52)
    } else {
        Color32::from_rgb(222, 222, 220)
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter()
        .rect_stroke(rect, 4.0, Stroke::new(1.0, foundation_input_edge()));
    let icon_color = if enabled {
        text_dark()
    } else {
        // Empty blocks cannot expand, but the disabled disclosure icon must
        // remain distinguishable from an unlabelled placeholder cell.
        Color32::from_rgb(176, 176, 176)
    };
    let icon = if open {
        ButtonIcon::Opened
    } else {
        ButtonIcon::Closed
    };
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    paint_button_icon_at(ui, icon, icon_rect, icon_color);
    response
}

pub(in crate::app) fn foundation_selected_width(row_width: f32) -> f32 {
    (row_width - 190.0 - 24.0 * 3.0 - 54.0 * 5.0 - 92.0).clamp(120.0, 420.0)
}

pub(in crate::app) fn foundation_header_value_cell(ui: &mut Ui, text: &str, max_width: f32) {
    let width = ui.available_width().min(max_width).max(180.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 4.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 4.0, Stroke::new(1.0, foundation_input_edge()));
    ui.painter().text(
        rect.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(text, width - 10.0),
        FontId::proportional(12.0),
        text_dark(),
    );
    if response.hovered() {
        response.on_hover_text(text);
    }
}

/// Interactive variant that reports whether the button was clicked.
pub(in crate::app) fn foundation_header_button_clicked(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
) -> bool {
    foundation_header_button_clicked_hint(ui, label, enabled, None)
}

pub(in crate::app) fn foundation_header_stepper_clicked(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
) -> bool {
    let (icon, tooltip) = match label {
        "<" => (ButtonIcon::Left, "Previous element"),
        ">" => (ButtonIcon::Right, "Next element"),
        _ => return false,
    };
    icon_button(
        ui,
        icon,
        tooltip,
        enabled,
        Vec2::new(24.0, 22.0),
        text_dark(),
    )
    .clicked()
}

/// Like [`foundation_header_button_clicked`] but shows `disabled_hint` as a

/// hover tooltip while the button is disabled (e.g. block at capacity).
pub(in crate::app) fn foundation_header_button_clicked_hint(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    disabled_hint: Option<&str>,
) -> bool {
    let response = if let Some(icon) = icon_for_foundation_button(label) {
        icon_button(
            ui,
            icon,
            foundation_icon_button_tooltip(label),
            enabled,
            Vec2::new(24.0, 22.0),
            text_dark(),
        )
    } else {
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(label).color(text_dark()))
                .min_size(Vec2::new(54.0, 22.0)),
        )
    };
    match disabled_hint {
        Some(hint) if !enabled => response.on_disabled_hover_text(hint).clicked(),
        _ => response.clicked(),
    }
}

fn foundation_icon_button_tooltip(label: &str) -> &str {
    match label {
        "..." => "Browse",
        "f()" => "Open function graph editor",
        "Open" => "Open",
        "Import" => "Import",
        "Clear" => "Clear",
        _ => label,
    }
}

// ── Block element selection (persisted in egui memory, keyed by block path) ──

pub(in crate::app) fn block_selected_index(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    path: &str,
    count: usize,
) -> usize {
    if count == 0 {
        return 0;
    }
    let id = edit.widget_id(("block_sel", path));
    let raw = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
    raw.min(count - 1)
}

pub(in crate::app) fn set_block_selected_index(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    path: &str,
    idx: usize,
) {
    let id = edit.widget_id(("block_sel", path));
    ui.data_mut(|d| d.insert_temp(id, idx));
}

pub(in crate::app) fn draw_foundation_bar(
    ui: &mut Ui,
    title: String,
    depth: usize,
    default_open: bool,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.scope(|ui| {
        draw_foundation_collapsing_header(
            ui,
            title.clone(),
            ("foundation_bar", title, depth),
            depth,
            default_open,
            None,
            foundation_section_bar(),
            |ui| {
                Frame::none()
                    .fill(foundation_group_bg())
                    .inner_margin(egui::Margin {
                        left: 8.0 + depth as f32 * 6.0,
                        right: 6.0,
                        top: 5.0,
                        bottom: 5.0,
                    })
                    .show(ui, add_contents);
            },
        );
    });
}

/// The signed index held by any block-index value variant.
pub(super) fn block_index_value(value: &TagFieldData) -> Option<i64> {
    match value {
        TagFieldData::CharBlockIndex(v) | TagFieldData::CustomCharBlockIndex(v) => Some(*v as i64),
        TagFieldData::ShortBlockIndex(v) | TagFieldData::CustomShortBlockIndex(v) => {
            Some(*v as i64)
        }
        TagFieldData::LongBlockIndex(v) | TagFieldData::CustomLongBlockIndex(v) => Some(*v as i64),
        _ => None,
    }
}

/// Resolve a block-index field's target block, returning `(element labels, full
/// target block path)`. Checks the field's own struct first (sibling target),
/// then walks up the ancestry from `root` (ancestor target — e.g. weapon's
/// "primary barrel" → the root "barrels" block). `None` for non-(plain)
/// block-index fields, custom indices (no target in the definition), or targets
/// that don't resolve — callers fall back to the numeric editor.
pub(in crate::app) fn block_index_target_options(
    tag_struct: &TagStruct<'_>,
    field: &TagField<'_>,
    names: &TagNameIndex,
    root: Option<TagStruct<'_>>,
    struct_path: &str,
) -> Option<(Vec<String>, String)> {
    let target_name = field.definition().block_index_target()?.name().to_owned();
    if target_name.is_empty() {
        return None;
    }
    // 1) The field's own struct (sibling block).
    if let Some(found) = find_target_block(tag_struct, &target_name, names, struct_path) {
        return Some(found);
    }
    // 2) Ancestors — walk parent structs up to the root.
    let root = root?;
    let mut current = struct_path;
    while !current.is_empty() {
        let parent = current.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let ancestor = if parent.is_empty() {
            root
        } else {
            root.descend(parent)?
        };
        if let Some(found) = find_target_block(&ancestor, &target_name, names, parent) {
            return Some(found);
        }
        if parent.is_empty() {
            break;
        }
        current = parent;
    }
    None
}

/// Some classic schemas expose parent links as plain signed shorts instead of
/// first-class block-index fields. Render only well-known parent references as
/// dropdowns so ordinary counters/indices remain numeric.
pub(in crate::app) fn semantic_short_index_target_options(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    tag_struct: &TagStruct<'_>,
    field: &TagField<'_>,
    names: &TagNameIndex,
    root: Option<TagStruct<'_>>,
    struct_path: &str,
) -> Option<(Vec<String>, String)> {
    if field.field_type() != TagFieldType::ShortInteger {
        return None;
    }
    let target_key = semantic_short_index_target_key(field.name())?;
    find_target_block_by_clean_key(tag_struct, target_key, names, struct_path).or_else(|| {
        find_ancestor_target_block_by_clean_key(ui, edit, root?, target_key, names, struct_path)
    })
}

pub(super) fn semantic_short_index_target_key(field_name: &str) -> Option<&'static str> {
    match clean_field_key(field_name).as_str() {
        "parent variant" | "variant" => Some("variants"),
        "parent node" => Some("nodes"),
        "damage section" | "indirect damage section" => Some("damage sections"),
        _ => None,
    }
}

fn find_ancestor_target_block_by_clean_key(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    root: TagStruct<'_>,
    target_key: &str,
    names: &TagNameIndex,
    struct_path: &str,
) -> Option<(Vec<String>, String)> {
    let mut current = Some(struct_path);
    while let Some(path) = current {
        let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
        let ancestor = if parent.is_empty() {
            root
        } else {
            root.descend(parent)?
        };
        if let Some(found) = find_target_block_by_clean_key(&ancestor, target_key, names, parent) {
            return Some(found);
        }
        if let Some(found) =
            find_nested_target_block_by_clean_key(ui, edit, &ancestor, target_key, names, parent, 4)
        {
            return Some(found);
        }
        current = (!parent.is_empty()).then_some(parent);
    }
    None
}

fn find_nested_target_block_by_clean_key(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    tag_struct: &TagStruct<'_>,
    target_key: &str,
    names: &TagNameIndex,
    struct_path: &str,
    depth_left: usize,
) -> Option<(Vec<String>, String)> {
    if depth_left == 0 {
        return None;
    }
    for field in tag_struct.fields_all() {
        let field_path = append_field_path(struct_path, field.name());
        if let Some(block) = field.as_block() {
            if clean_field_key(field.name()) == target_key {
                let labels = (0..block.len())
                    .map(|i| block_element_dropdown_label(block.element(i), names, i))
                    .collect();
                return Some((labels, field_path));
            }
            let count = block.len();
            if count > 0 {
                let selected = block_selected_index(ui, edit, &field_path, count);
                if let Some(element) = block.element(selected) {
                    let element_path = format!("{field_path}[{selected}]");
                    if let Some(found) = find_nested_target_block_by_clean_key(
                        ui,
                        edit,
                        &element,
                        target_key,
                        names,
                        &element_path,
                        depth_left - 1,
                    ) {
                        return Some(found);
                    }
                }
            }
        } else if let Some(nested) = field.as_struct() {
            if let Some(found) = find_nested_target_block_by_clean_key(
                ui,
                edit,
                &nested,
                target_key,
                names,
                &field_path,
                depth_left - 1,
            ) {
                return Some(found);
            }
        }
    }
    None
}

fn find_target_block_by_clean_key(
    tag_struct: &TagStruct<'_>,
    target_key: &str,
    names: &TagNameIndex,
    struct_path: &str,
) -> Option<(Vec<String>, String)> {
    for sibling in tag_struct.fields_all() {
        if let Some(block) = sibling.as_block() {
            if clean_field_key(sibling.name()) == target_key {
                let labels = (0..block.len())
                    .map(|i| block_element_dropdown_label(block.element(i), names, i))
                    .collect();
                return Some((labels, append_field_path(struct_path, sibling.name())));
            }
        }
    }
    None
}

/// Find a block field whose definition name is `target_name` directly within
/// `tag_struct`, returning `(element labels, full block path)`.
fn find_target_block(
    tag_struct: &TagStruct<'_>,
    target_name: &str,
    names: &TagNameIndex,
    struct_path: &str,
) -> Option<(Vec<String>, String)> {
    for sibling in tag_struct.fields_all() {
        if let Some(block) = sibling.as_block() {
            if block.definition().name() == target_name {
                let labels = (0..block.len())
                    .map(|i| block_element_dropdown_label(block.element(i), names, i))
                    .collect();
                return Some((labels, append_field_path(struct_path, sibling.name())));
            }
        }
    }
    None
}

/// A block-index field rendered like Foundation: a dropdown of the target
/// block's elements with a leading `<none>` (value −1), plus a "go to" button
/// that scrolls to the referenced element.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_foundation_block_index_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    current: i64,
    labels: &[String],
    target_block_path: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let editable = edit.editable && !meta.read_only;
    let in_range = current >= 0 && (current as usize) < labels.len();
    let selected_text = if in_range {
        labels[current as usize].clone()
    } else {
        "<none>".to_owned()
    };

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());

        if editable {
            let mut new_index: Option<i64> = None;
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt(("block_index", path))
                    .selected_text(truncate_for_cell(&selected_text, 280.0))
                    .width(300.0),
                |ui| {
                    if ui.selectable_label(current < 0, "<none>").clicked() {
                        new_index = Some(-1);
                    }
                    for (i, label) in labels.iter().enumerate() {
                        if ui.selectable_label(current == i as i64, label).clicked() {
                            new_index = Some(i as i64);
                        }
                    }
                },
            );
            if let Some(delta) = wheel_delta {
                if let Some(next) =
                    combo_scroll_next_i64(current, -1, labels.len() as i64 - 1, delta)
                {
                    new_index = Some(next);
                }
            }
            if let Some(index) = new_index {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: index.to_string(),
                });
            }
        } else {
            foundation_input_cell(ui, &selected_text, 300.0);
        }

        // "Go to" the referenced element: scroll to the target block and select
        // the element (reuses the 3.4 jump-to-block scroll mechanism).
        let go_to = ui.add_enabled(
            in_range,
            egui::Button::new(RichText::new("↳").color(text_dark()))
                .min_size(Vec2::new(54.0, 20.0)),
        );
        let go_to = if in_range {
            go_to.on_hover_text(format!(
                "Go to referenced element\n{target_block_path}[{current}]"
            ))
        } else {
            go_to.on_disabled_hover_text("No referenced element (index is <none>)")
        };
        if go_to.clicked() {
            ui.data_mut(|d| d.insert_temp(jump_target_id(), target_block_path.to_owned()));
            set_block_selected_index(ui, edit, target_block_path, current as usize);
        }
    });
}
