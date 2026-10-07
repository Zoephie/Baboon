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
    let group_default_open = edit.default_open(depth <= 1);
    let open_override = edit.resolve_open(path_prefix, group_default_open);
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
        let inherited_default_open = edit.default_open(true);
        let open_override = edit.resolve_open(path_prefix, inherited_default_open);
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
    let entries: &[DefEntry] = edit
        .docs
        .map(|docs| docs.entries_for_struct(tag_struct))
        .unwrap_or(&[]);
    let parent_raw = tag_struct.raw();
    let reference_value_width = shared_tag_reference_value_width(ui, depth);
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
                        draw_injected_explanation_row(
                            ui,
                            title,
                            body,
                            depth,
                            path_prefix,
                            cursor + offset,
                            edit,
                        );
                    }
                }
                if let DefEntry::Field {
                    help,
                    unit,
                    range,
                    tag_reference_allowed,
                    read_only,
                    hidden,
                    ..
                } = &entries[match_idx]
                {
                    // The engine strips everything after `:` and the trailing
                    // `*`/`!` markers from the field name, so unit/range/help and
                    // the read-only and hidden flags are recovered from the
                    // definition here.
                    let mut meta = field_display_meta(name);
                    meta.help = help.clone();
                    meta.unit = unit.clone();
                    meta.range = range.clone();
                    meta.tag_reference_allowed = tag_reference_allowed.clone();
                    meta.read_only |= *read_only;
                    meta.advanced |= *hidden;
                    meta_override = Some(meta);
                }
                cursor = match_idx + 1;
            }
        }
        let field_path = append_field_path_for(path_prefix, &field);
        // A row wholly out of view stands in as the space it took last time.
        if let Some(height) = edit
            .row_heights
            .as_deref()
            .and_then(|heights| heights.skip(ui, &field_path))
        {
            // Allocated like a row, so the spacing after it lies outside what
            // the enclosing section measures, just as after a drawn row.
            ui.allocate_space(egui::vec2(0.0, height - ui.spacing().item_spacing.y));
            continue;
        }
        let top = ui.cursor().min.y;
        // Resolve a block-index field's target block (sibling or ancestor) for
        // the element dropdown; `None` falls back to the numeric editor.
        let root = edit.root;
        let block_index = block_index_target_options(tag_struct, &field, root, path_prefix);
        draw_field(
            ui,
            field,
            field_path.clone(),
            parent_raw,
            names,
            depth,
            expert_mode,
            path_prefix,
            edit,
            meta_override,
            block_index,
            reference_value_width,
        );
        let height = ui.cursor().min.y - top;
        if let Some(heights) = edit.row_heights.as_deref_mut() {
            heights.record(&field_path, height);
        }
    }
    // Any explanations after the last matched field.
    for (offset, entry) in entries[cursor..].iter().enumerate() {
        if let DefEntry::Explanation { title, body } = entry {
            draw_injected_explanation_row(
                ui,
                title,
                body,
                depth,
                path_prefix,
                cursor + offset,
                edit,
            );
        }
    }
}

pub(in crate::app) fn draw_field(
    ui: &mut Ui,
    field: TagField<'_>,
    field_path: String,
    parent_raw: &[u8],
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
    meta_override: Option<FieldDisplayMeta>,
    block_index: Option<BlockIndexTarget>,
    tag_reference_value_width: f32,
) {
    #[cfg(test)]
    FIELD_ROWS_BUILT.with(|built| built.set(built.get() + 1));
    mark_find_render_cell(ui, edit.tag_key, &field_path);
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
                edit.resolve_open(&field_path, true),
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
            egui::Frame::NONE.fill(glow_fill).show(ui, |ui| {
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
            let framed = egui::Frame::NONE.fill(fill).show(ui, |ui| {
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

        let nested_default_open = edit.default_open(true);
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
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    open_override: Option<bool>,
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
        // Full-width header bar (see draw_foundation_group), matching Foundation.
        draw_foundation_collapsing_header(
            ui,
            header,
            ("foundation_explanation", id_salt),
            depth,
            true,
            open_override,
            foundation_section_bar(),
            FindTargetKind::Documentation,
            has_body,
            has_body.then_some(ButtonIcon::Doc),
            |ui| {
                if has_body {
                    Frame::NONE
                        .fill(foundation_documentation_bg())
                        .corner_radius(foundation_body_rounding())
                        .inner_margin(egui::Margin::same(20))
                        .show(ui, |ui| {
                            // The box spans the full parent width (Foundation's
                            // border is Width=Auto in a stretch StackPanel); only the
                            // text itself is capped (~650px) and left-aligned.
                            ui.set_min_width(ui.available_width());
                            let text_width = ui.available_width().min(650.0);
                            ui.scope(|ui| {
                                ui.set_max_width(text_width);
                                let body = body.trim_end();
                                if let Some(text) = highlighted_italic_widget_text(
                                    ui,
                                    body,
                                    TextStyle::Monospace,
                                    text_dark(),
                                    FindTargetKind::Documentation,
                                ) {
                                    ui.label(text);
                                } else {
                                    ui.label(
                                        RichText::new(body)
                                            .color(text_dark())
                                            .monospace()
                                            .italics()
                                            .size(12.0),
                                    );
                                }
                            });
                        });
                }
            },
        );
    });
}

fn draw_injected_explanation_row(
    ui: &mut Ui,
    title: &str,
    body: &str,
    depth: usize,
    path_prefix: &str,
    entry_index: usize,
    edit: &FieldEditContext<'_>,
) {
    let path = documentation_path(path_prefix, entry_index);
    if !edit.field_visible(&path) {
        return;
    }
    let scroll_here = edit.field_nav.is_some()
        && ui
            .data(|data| data.get_temp::<String>(field_jump_target_id()))
            .as_deref()
            == Some(path.as_str());
    if scroll_here {
        let target = egui::Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(ui.available_width().max(1.0), 28.0),
        );
        ui.scroll_to_rect(target, Some(egui::Align::Center));
        ui.data_mut(|data| {
            data.remove::<String>(field_jump_target_id());
            if data.get_temp::<String>(jump_target_id()).as_deref() == Some(path.as_str()) {
                data.remove::<String>(jump_target_id());
            }
        });
        ui.ctx().request_repaint();
    }
    mark_find_render_cell(ui, edit.tag_key, &path);
    draw_foundation_explanation_row(
        ui,
        title,
        Some(body),
        depth,
        &path,
        edit.resolve_open(&path, true),
    );
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

pub(in crate::app) fn foundation_block_title(name: &str) -> String {
    clean_field_name(name)
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    // Asked of every field row on every frame. The exact test parses the name
    // as a field path, which was a fifth of a frame's allocations; a name
    // that doesn't contain a marker's words at all can't be one.
    if !contains_marker_words(name) {
        return false;
    }
    matches!(
        internal_marker_key(name).as_str(),
        "hide group id" | "end hide group id" | "whore function"
    )
}

/// Whether `name` contains "hide group id" or "whore function", in any case
/// and with `_` for a space, without allocating.
fn contains_marker_words(name: &str) -> bool {
    let name = name.as_bytes();
    [b"hide group id".as_slice(), b"whore function".as_slice()].iter().any(|words| {
        name.windows(words.len()).any(|window| {
            window.iter().zip(words.iter()).all(|(&have, &want)| {
                have.eq_ignore_ascii_case(&want) || (have == b'_' && want == b' ')
            })
        })
    })
}

fn internal_marker_key(name: &str) -> String {
    clean_field_key(name).replace('_', " ")
}

fn strip_index_suffix(segment: &str) -> &str {
    segment.split_once('[').map_or(segment, |(name, _)| name)
}

pub(super) fn inline_mapping_function_from_struct(
    tag_struct: TagStruct<'_>,
    struct_path: &str,
) -> Option<(FunctionView, String)> {
    // A Halo 2 `mapping_function` holds its function in a `data` byte-block,
    // which is always the H2 encoding. An empty block (a new element) opens as
    // what the engine grows it to on its first edit, a zeroed header: identity.
    // Nothing is written unless the function is edited.
    if let Some(bytes) = halo2_function_bytes_from_struct(tag_struct) {
        let function = if bytes.is_empty() {
            Some(TagFunction::H2(H2Function::new(FunctionType::Identity)))
        } else {
            h2_tag_function(&bytes)
        };
        if let Some(function) = function {
            return Some((
                FunctionView::from_function(function),
                append_field_path(struct_path, "data"),
            ));
        }
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
            // A function field with no bytes still gets the function editor,
            // seeded with what a fresh one would hold. Falling through left a
            // dead `data [0 bytes]` row that could not author the function it
            // was standing in for. New elements are seeded at creation now, so
            // this is for tags an earlier build already wrote that way.
            //
            // The seed is the editor's own default rather than what the runtime
            // would make of these bytes: `ensure_valid` repairs a short blob to
            // 32 *zeroes*, an identity clamped to 0..0, which is not what
            // creating one gives you. Matching creation keeps the two paths
            // showing the same thing. Nothing is written until the user edits.
            if field.is_function_data()
                && let Ok(function) = TagFunction::parse(
                    &blam_tags::default_function_definition_bytes(blam_tags::io::Endian::Le),
                )
            {
                return Some((FunctionView::from_function(function), data_path));
            }
            continue;
        }
    }
    None
}

pub(in crate::app) fn draw_foundation_group(
    ui: &mut Ui,
    title: String,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    depth: usize,
    default_open: bool,
    // `Some(open)` forces the open-state this frame (Search-fields filter);
    // `None` leaves the node's stored / default state untouched.
    open_override: Option<bool>,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.scope(|ui| {
        draw_foundation_collapsing_header(
            ui,
            title,
            id_salt,
            depth,
            default_open,
            open_override,
            foundation_section_bar(),
            FindTargetKind::Label,
            true,
            None,
            |ui| {
                Frame::NONE
                    .fill(foundation_group_bg())
                    .corner_radius(foundation_body_rounding())
                    .inner_margin(egui::Margin {
                        left: (8.0 + depth as f32 * 4.0) as i8,
                        right: 8,

                        top: 6,
                        bottom: 6,
                    })
                    .show(ui, add_contents);
            },
        );
    });
}

/// Draw a full-width modern collapsing header with the same rounded chevron
/// control used by block headers. Keeping this in one helper ensures groups,
/// explanations, and section bars share the same affordance.
fn add_foundation_header_spacing(ui: &mut Ui) {
    // egui has already advanced by `item_spacing.y` after the previous item.
    // Add only the remainder so adjacent header bars have an 8pt visual gap.
    ui.add_space((8.0 - ui.spacing().item_spacing.y).max(0.0));
}

const FOUNDATION_CONTAINER_RADIUS: f32 = 5.0;

fn foundation_header_rounding(joined_to_body: bool) -> egui::CornerRadius {
    if joined_to_body {
        egui::CornerRadius {
            nw: (FOUNDATION_CONTAINER_RADIUS) as u8,
            ne: (FOUNDATION_CONTAINER_RADIUS) as u8,
            sw: 0,
            se: 0,
        }
    } else {
        egui::CornerRadius::same((FOUNDATION_CONTAINER_RADIUS) as u8)
    }
}

fn foundation_body_rounding() -> egui::CornerRadius {
    egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: (FOUNDATION_CONTAINER_RADIUS) as u8,
        se: (FOUNDATION_CONTAINER_RADIUS) as u8,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_foundation_collapsing_header(
    ui: &mut Ui,
    title: String,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    depth: usize,
    default_open: bool,
    open_override: Option<bool>,
    bar_fill: Color32,
    find_kind: FindTargetKind,
    collapsible: bool,
    leading_icon: Option<ButtonIcon>,
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

    add_foundation_header_spacing(ui);
    let row_width = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(row_width, 40.0), Sense::hover());
    let header_background = ui.painter().add(egui::Shape::Noop);
    // This child paints into the reserved row without advancing the parent
    // cursor. `allocate_new_ui` would advance it to the child's 24pt content
    // boundary, effectively discarding the row's bottom padding.
    let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(row_rect.shrink(8.0)));
    header_ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
    header_ui.horizontal_centered(|ui| {
        ui.add_space(depth as f32 * 4.0);
        if collapsible {
            let toggle = foundation_header_toggle_cell(ui, state.is_open(), true);
            if toggle.clicked() {
                state.toggle(ui);
            }
        }
        if let Some(icon) = leading_icon {
            let (icon_rect, _) =
                ui.allocate_exact_size(Vec2::splat(BUTTON_ICON_SIZE), Sense::hover());
            paint_button_icon_at(ui, icon, icon_rect, foundation_block_text());
        }
        let label = if findable_text_has_match(ui, &title, find_kind) {
            let (label_rect, label) = ui.allocate_exact_size(
                Vec2::new(ui.available_width().max(80.0), 20.0),
                if collapsible {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            paint_findable_text(
                ui,
                label_rect.left_center(),
                Align2::LEFT_CENTER,
                &title,
                bold_font(12.5),
                foundation_block_text(),
                find_kind,
            );
            label
        } else {
            ui.add(
                egui::Label::new(
                    RichText::new(&title)
                        .color(foundation_block_text())
                        .font(bold_font(12.5)),
                )
                .sense(if collapsible {
                    Sense::click()
                } else {
                    Sense::hover()
                }),
            )
        };
        if collapsible && label.clicked() {
            state.toggle(ui);
        }
    });
    state.store(ui.ctx());
    let open = collapsible && state.is_open();
    let body_response = if open {
        // Widget allocation leaves the normal inter-item gap after the header.
        // Retract it so the body begins flush against the header bar.
        ui.add_space(-ui.spacing().item_spacing.y);
        state.show_body_unindented(ui, add_contents)
    } else {
        None
    };
    let joined_to_body = body_response.is_some();
    ui.painter().set(
        header_background,
        egui::Shape::rect_filled(
            row_rect,
            foundation_header_rounding(joined_to_body),
            bar_fill,
        ),
    );
    let container_rect = body_response.map_or(row_rect, |body| {
        egui::Rect::from_min_max(
            row_rect.min,
            egui::pos2(row_rect.max.x, body.response.rect.max.y),
        )
    });
    ui.painter().rect_stroke(
        container_rect,
        FOUNDATION_CONTAINER_RADIUS,
        Stroke::new(1.0_f32, foundation_block_edge()),
        egui::StrokeKind::Middle,
    );
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
    let labeler = BlockLabeler::new(edit, names);
    let selected_label = if count == 0 {
        "NONE".to_owned()
    } else {
        labeler.label(path_prefix, block, sel)
    };

    let block_default_open = edit.default_open(depth == 0 || is_priority_section(name));
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
                && strip_element_indices(&clip.block_path)
                    == strip_element_indices(path_prefix) =>
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
        edit.is_active_filter(),
        paste_gate,
        block_size_label.as_deref(),
        |i| labeler.label(path_prefix, block, i),
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
    if actions.reorganize {
        *edit.block_table_request = Some(BlockTableRequest {
            path: path_prefix.to_owned(),
            label: clean_field_name(name),
            view_scope: edit.view_scope.to_owned(),
            selected: sel,
        });
    }

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
            ui.copy_text(tsv);
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

    // Paste / replace from the clipboard. The elements are cloned only for an
    // action that was clicked: this runs for every block drawn, every frame,
    // and used to deep-copy the whole clipboard each time.
    if let Some(clip) = edit.block_clipboard
        && (actions.paste || actions.replace_element || actions.replace_block)
    {
        let elements = clip.elements.clone();
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
                // Stamped by the pane once this render returns; the field
                // renderers are shared and have no kit of their own.
                kit: None,
                opened_at: None,
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
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f32 / (1024.0 * 1024.0))
    } else {
        // A bulk container extraction is measured in gigabytes, and "8734.2 MiB"
        // is a number the reader has to divide themselves.
        format!("{:.1} GiB", bytes as f32 / (1024.0 * 1024.0 * 1024.0))
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
                // Stamped by the pane once this render returns; the field
                // renderers are shared and have no kit of their own.
                kit: None,
                opened_at: None,
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
            // Stamped by the pane once this render returns; the field
            // renderers are shared and have no kit of their own.
            kit: None,
            opened_at: None,
            tag_key: edit.tag_key.to_owned(),

            path: path.to_owned(),
            kind: BlockOpKind::DeleteAll,
            message: format!("Delete ALL {count} elements from this block?"),
            confirm_label: "Delete".to_owned(),
        });
    }
}

#[cfg(test)]
thread_local! {
    /// How many dropdown labels this thread has built, for tests that bound it.
    pub(in crate::app) static DROPDOWN_LABELS_BUILT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// The game's block-element label rules, read once per definitions folder and
/// game. `None` when the definitions can't be read, and the editor falls back
/// to its own content label.
pub(in crate::app) fn element_labels(
    definitions_root: Option<&std::path::Path>,
    game: Option<GameId>,
) -> Option<std::sync::Arc<blam_tags::element_label::ElementLabels>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Rules = Option<Arc<blam_tags::element_label::ElementLabels>>;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<(std::path::PathBuf, GameId), Rules>>> =
        OnceLock::new();
    let (root, game) = (definitions_root?, game?);
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((root.to_path_buf(), game))
        .or_insert_with(|| {
            blam_tags::element_label::ElementLabels::load(root.join(game.as_str()))
                .map(Arc::new)
                .ok()
        })
        .clone()
}

/// Labels block elements as the game's own editor writes them (blam-tags
/// `element_label`), with Baboon's `N. ` prefix. Built from a
/// [`FieldEditContext`] up front, so a dropdown can label rows while the
/// context is borrowed elsewhere.
#[derive(Clone)]
pub(in crate::app) struct BlockLabeler<'a> {
    rules: Option<std::sync::Arc<blam_tags::element_label::ElementLabels>>,
    group: u32,
    root: Option<TagStruct<'a>>,
    names: &'a TagNameIndex,
}

impl<'a> BlockLabeler<'a> {
    pub(in crate::app) fn new(edit: &FieldEditContext<'a>, names: &'a TagNameIndex) -> Self {
        Self::for_tag(edit.definitions_root, edit.game, edit.group_tag, edit.root, names)
    }

    /// The labeller for a tag of `group` in `game`, whose root is `root`.
    pub(in crate::app) fn for_tag(
        definitions_root: Option<&std::path::Path>,
        game: Option<GameId>,
        group: u32,
        root: Option<TagStruct<'a>>,
        names: &'a TagNameIndex,
    ) -> Self {
        BlockLabeler { rules: element_labels(definitions_root, game), group, root, names }
    }

    /// The label of element `index` of the block at `block_path`. The prefix
    /// is left off a label that already starts with it: the editors' fallback
    /// is `"N. <struct name>"`, and some callbacks print the index themselves.
    /// Without the game's rules, Baboon's own content label.
    pub(in crate::app) fn label(&self, block_path: &str, block: TagBlock<'_>, index: usize) -> String {
        let Some(rules) = &self.rules else {
            return block_element_dropdown_label(block.element(index), self.names, index);
        };
        #[cfg(test)]
        DROPDOWN_LABELS_BUILT.with(|count| count.set(count.get() + 1));
        let ctx = blam_tags::element_label::Context { group: Some(self.group) };
        let label = self
            .root
            .and_then(|root| rules.label_at_in(&ctx, root, block_path, index as i64))
            .unwrap_or_else(|| rules.label_in(&ctx, &[], block, index as i64));
        let prefix = format!("{index}. ");
        if label.starts_with(&prefix) { label } else { prefix + &label }
    }
}

pub(in crate::app) fn block_element_dropdown_label(
    element: Option<TagStruct<'_>>,
    names: &TagNameIndex,
    index: usize,
) -> String {
    #[cfg(test)]
    DROPDOWN_LABELS_BUILT.with(|count| count.set(count.get() + 1));
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
    let array_default_open = edit.default_open(depth == 0);
    let open_override = edit.resolve_open(path_prefix, array_default_open);
    // A clipboard is compatible when it came from this same array schema
    // position. Element subscripts are stripped so which parent block element is
    // selected doesn't matter — the array's shape is identical across siblings.
    // Arrays are fixed-count inline structs, not FieldSet-versioned, so there's
    // no version dimension to gate on (only group + schema position).
    let paste_gate = match edit.block_clipboard {
        Some(clip)
            if edit.editable
                && clip.group_tag == edit.group_tag
                && strip_element_indices(&clip.block_path)
                    == strip_element_indices(path_prefix) =>
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
        edit.is_active_filter(),
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
            ui.copy_text(tsv);
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

const BLOCK_JUMP_SCROLL_LEAD: f32 = 80.0;
const BLOCK_JUMP_HIGHLIGHT_SECONDS: f64 = 1.25;

/// Give the destination some visual context instead of pinning its header to
/// the very top edge. The leading area is roughly two ordinary field rows.
fn block_jump_scroll_rect(header: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(header.left(), header.top() - BLOCK_JUMP_SCROLL_LEAD),
        header.max,
    )
}

/// Field paths produced by the renderer include exact `#ordinal` suffixes,
/// while a few older callers still provide readable paths without them.
/// Preserve concrete element indices while accepting either spelling.
fn block_paths_match(left: &str, right: &str) -> bool {
    strip_field_ordinals(left) == strip_field_ordinals(right)
}

fn strip_field_ordinals(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut ordinal = false;
    for ch in path.chars() {
        match ch {
            '#' => ordinal = true,
            '/' | '[' if ordinal => {
                ordinal = false;
                out.push(ch);
            }
            _ if ordinal => {}
            _ => out.push(ch),
        }
    }
    out
}

fn block_jump_opens_path(target: &str, candidate: &str) -> bool {
    let target = strip_node_indices(target);
    let candidate = strip_node_indices(candidate);
    candidate.is_empty()
        || target == candidate
        || (target.len() > candidate.len()
            && target.starts_with(&candidate)
            && target.as_bytes()[candidate.len()] == b'/')
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
    // A filtered result can be used as an anchor into the unfiltered editor.
    show_search_jump: bool,
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
    // A block-index "Go to" can target a block hidden under one or more
    // collapsed containers. Force every ancestor and the target block itself
    // open until the target header consumes the one-shot jump below.
    let pending_jump = ui.data(|data| data.get_temp::<String>(jump_target_id()));
    if pending_jump
        .as_deref()
        .is_some_and(|target| block_jump_opens_path(target, path_salt))
    {
        state.set_open(count > 0);
    }
    if count == 0 && state.is_open() {
        state.set_open(false);
    }

    add_foundation_header_spacing(ui);
    let row_width = ui.available_width();
    let row_height = 40.0;
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::hover());
    let header_fill = foundation_block_bar();
    let header_background = ui.painter().add(egui::Shape::Noop);

    let jump_highlight_id = ui.make_persistent_id((
        "foundation_block_jump_highlight",
        view_scope,
        tag_key,
        path_salt,
        depth,
        name,
    ));
    let now = ui.input(|input| input.time);

    // 3.4 jump-to-parent: if a child's "↑" targeted this block last frame, bring
    // its header into view (and clear the pending target).
    if pending_jump
        .as_deref()
        .is_some_and(|target| block_paths_match(target, path_salt))
    {
        ui.scroll_to_rect(block_jump_scroll_rect(row_rect), Some(egui::Align::Min));
        ui.data_mut(|d| {
            d.remove::<String>(jump_target_id());
            d.insert_temp(jump_highlight_id, now + BLOCK_JUMP_HIGHLIGHT_SECONDS);
        });
        ui.ctx().request_repaint();
    }

    let jump_highlight_strength = ui
        .data(|data| data.get_temp::<f64>(jump_highlight_id))
        .map(|until| ((until - now) / BLOCK_JUMP_HIGHLIGHT_SECONDS).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    if jump_highlight_strength > 0.0 {
        ui.ctx().request_repaint();
    } else {
        ui.data_mut(|data| data.remove::<f64>(jump_highlight_id));
    }

    // At-capacity / empty gating mirrors Guerilla's enable rules.
    let (can_edit, has_sel, at_capacity, capacity_hint) =
        prepare_block_control_availability(editable, allow_structural, count, max_count);
    let mut selector_active = false;

    // Paint controls inside the already-reserved row without changing the
    // parent's cursor; otherwise the 8pt bottom padding is lost.
    let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(row_rect.shrink(8.0)));
    header_ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
    header_ui.horizontal_centered(|ui| {
        ui.add_space(depth as f32 * 5.0);
        let toggle = foundation_header_toggle_cell(ui, state.is_open(), count > 0);
        if toggle.clicked() && count > 0 {
            state.toggle(ui);
        }
        // Jump-to-parent follows the disclosure control for nested blocks;
        // hover shows the breadcrumb.
        if depth > 0 {
            let jump = icon_button(
                ui,
                ButtonIcon::JumpUp,
                "Jump to parent block",
                true,
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
        let block_title = foundation_block_title(name);
        let (name_rect, name_label) =
            ui.allocate_exact_size(Vec2::new(190.0, 20.0), Sense::click());
        let block_title_font = bold_font(12.5);
        if jump_highlight_strength > 0.0 {
            let galley = ui.painter().layout_no_wrap(
                block_title.clone(),
                block_title_font.clone(),
                foundation_block_text(),
            );
            let highlight_rect = egui::Rect::from_center_size(
                egui::pos2(
                    name_rect.left() + galley.size().x * 0.5,
                    name_rect.center().y,
                ),
                galley.size() + Vec2::new(7.0, 4.0),
            );
            ui.painter().rect_filled(
                highlight_rect,
                2.0,
                Color32::from_rgba_unmultiplied(
                    255,
                    220,
                    70,
                    (180.0 * jump_highlight_strength) as u8,
                ),
            );
        }
        paint_findable_text(
            ui,
            name_rect.left_center(),
            Align2::LEFT_CENTER,
            &block_title,
            block_title_font,
            foundation_block_text(),
            FindTargetKind::Block,
        );
        // Right-click the block name → copy / paste menu. Copy actions are
        // read-only and available for any element collection (including
        // fixed-size arrays); the size/content-changing paste & replace
        // actions are gated behind `allow_structural`.
        let name_label = name_label.on_hover_text("Right-click for block options");
        context_menu(&name_label, |ui| {
            draw_block_header_menu(ui, count, allow_structural, editable, paste_gate, &mut actions);
        });
        if show_search_jump
            && icon_button(
                ui,
                ButtonIcon::JumpTo,
                "Clear search and jump to this block",
                true,
                foundation_jump_cyan(),
            )
            .clicked()
        {
            ui.data_mut(|data| {
                data.insert_temp(
                    find_filter_block_jump_id(view_scope, tag_key),
                    path_salt.to_owned(),
                )
            });
        }
        // Keep the previous arrow directly beside the selected
        // reference, with the next arrow following it.
        if foundation_header_stepper_clicked(ui, "<", has_sel && selected_index > 0) {
            actions.new_selection = Some(selected_index.saturating_sub(1));
        }

        // Instance selector dropdown — built lazily (only when open).
        let combo_width = foundation_selected_width(row_width);
        {
            // Include the count so changed blocks retire the popup's old sizing.
            let popup_id = ui.make_persistent_id((
                "block_instance",
                view_scope,
                tag_key,
                path_salt,
                depth,
                count,
            ));
            let (response, selection, popup_open) = searchable_block_selector(
                ui,
                popup_id,
                selected_label,
                selected_index,
                count,
                combo_width,
                &element_label,
            );
            if let Some(index) = selection {
                actions.new_selection = Some(index);
            }
            // Search owns keyboard input while the popup is open.
            selector_active |= !popup_open && (response.hovered() || response.has_focus());
            let wheel_delta = dropdown_wheel_delta(ui, &response, popup_open);
            if let Some(delta) = wheel_delta {
                if let Some(next) = combo_scroll_next_index(selected_index, count, delta) {
                    actions.new_selection = Some(next);
                }
            }
        }

        // Next stepper follows the selected reference string.
        if foundation_header_stepper_clicked(ui, ">", has_sel && selected_index + 1 < count) {
            actions.new_selection = Some(selected_index + 1);
        }

        if let Some(size_label) = block_size_label {
            ui.label(
                RichText::new(size_label)
                    .color(subtle_dark())
                    .monospace()
                    .small(),
            )
            .on_hover_text("Block memory usage: elements × element byte size");
        }

        // Double the normal gap between entry navigation and block actions.
        ui.add_space(ui.spacing().item_spacing.x);

        // Structural edit buttons — only for variable-count blocks. Arrays
        // are fixed-size, so the count-changing actions don't apply and the
        // buttons are omitted entirely. The grow actions (Add / Insert /
        // Duplicate) are disabled once the block hits its schema cap.
        if allow_structural {
            let hint = capacity_hint.as_deref();
            if foundation_header_button_clicked_hint(ui, "Add", can_edit && !at_capacity, hint) {
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
        icon_menu_button(ui, ButtonIcon::Other, "Other block options", |ui| {
            draw_block_header_menu(
                ui, count, allow_structural, editable, paste_gate, &mut actions,
            );
        });
    });

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
    let body_response = if count > 0 && state.openness(ui.ctx()) > 0.0 {
        ui.add_space(-ui.spacing().item_spacing.y);
        state.show_body_unindented(ui, |ui| {
            Frame::NONE
                .fill(foundation_group_bg())
                .corner_radius(foundation_body_rounding())
                .inner_margin(egui::Margin {
                    left: (14.0 + depth as f32 * 5.0) as i8,
                    right: 8,
                    top: 8,
                    bottom: 8,
                })
                // Render the body inline — no nested ScrollArea. The single outer
                // ScrollArea in `draw_tag_fields_scroll` owns all scrolling, so a
                // block element expands to its full height instead of growing an
                // inner scrollbar (which also let cross-boundary scroll-to fail).
                .show(ui, add_contents);
        })
    } else {
        None
    };
    let joined_to_body = body_response.is_some();
    ui.painter().set(
        header_background,
        egui::Shape::rect_filled(
            row_rect,
            foundation_header_rounding(joined_to_body),
            header_fill,
        ),
    );
    let container_rect = body_response.map_or(row_rect, |body| {
        egui::Rect::from_min_max(
            row_rect.min,
            egui::pos2(row_rect.max.x, body.response.rect.max.y),
        )
    });
    ui.painter().rect_stroke(
        container_rect,
        FOUNDATION_CONTAINER_RADIUS,
        Stroke::new(1.0_f32, foundation_block_edge()),
        egui::StrokeKind::Middle,
    );

    actions
}

/// Shared by the block title's context menu and its visible Other button.
fn draw_block_header_menu(
    ui: &mut Ui,
    count: usize,
    allow_structural: bool,
    editable: bool,
    paste_gate: PasteGate,
    actions: &mut BlockHeaderActions,
) {
    if allow_structural {
        if icon_text_button(
            ui,
            ButtonIcon::TableView,
            "Reorganize Block Entries",
            editable,
        )
        .clicked() {
            actions.reorganize = true;
            close_menu(ui);
        }
        ui.separator();
    }
    // Copy + in-place replace are valid for blocks AND fixed-size
    // arrays (no element-count change). The size-changing actions
    // (paste/insert, replace-all, add/delete) are blocks only.
    if ui
        .add_enabled(count > 0, egui::Button::new("Copy element"))
        .clicked()
    {
        actions.copy = true;
        close_menu(ui);
    }
    if ui
        .add_enabled(count > 0, egui::Button::new("Copy entire block"))
        .clicked()
    {
        actions.copy_block = true;
        close_menu(ui);
    }
    if ui
        .add_enabled(count > 0, egui::Button::new("Copy block as TSV"))
        .on_hover_text("Copy all elements as tab-separated rows (Excel)")
        .clicked()
    {
        actions.copy_block_tsv = true;
        close_menu(ui);
    }
    // In-place replace of the selected element — never changes
    // the count, so it works for arrays too.
    if matches!(paste_gate, PasteGate::Ready(_))
        && ui
            .add_enabled(count > 0, egui::Button::new("Replace selected element"))
            .on_hover_text("Overwrite the selected element with the clipboard")
            .clicked()
    {
        actions.replace_element = true;
        close_menu(ui);
    }
    if allow_structural {
        if ui
            .add_enabled(count > 0, egui::Button::new("Paste TSV…"))
            .on_hover_text("Paste tab-separated rows back onto this block's elements")
            .clicked()
        {
            actions.paste_tsv = true;

            close_menu(ui);
        }
        ui.separator();
        match paste_gate {
            PasteGate::Ready(n) => {
                let noun = if n == 1 { "element" } else { "elements" };
                if ui.button(format!("Paste {n} {noun}")).clicked() {
                    actions.paste = true;
                    close_menu(ui);
                }
                if ui.button("Replace entire block").clicked() {
                    actions.replace_block = true;
                    close_menu(ui);
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
        input.smooth_scroll_delta = Vec2::ZERO;
    });
}

/// This frame's unsmoothed wheel travel in points, summed from the raw
/// `MouseWheel` events. egui 0.36 dropped `InputState::raw_scroll_delta`;
/// this rebuilds it the way egui maps a wheel event: line units scaled by
/// `line_scroll_speed`, page units by the screen height, and a wheel turned
/// with the horizontal-scroll modifier (Shift) moving sideways only.
pub(in crate::app) fn raw_wheel_delta(input: &egui::InputState, options: &egui::InputOptions) -> Vec2 {
    input
        .events
        .iter()
        .filter_map(|event| match event {
            egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } => {
                let delta = match unit {
                    egui::MouseWheelUnit::Point => *delta,
                    egui::MouseWheelUnit::Line => *delta * options.line_scroll_speed,
                    egui::MouseWheelUnit::Page => *delta * input.content_rect().height(),
                };
                let horizontal = modifiers.matches_any(options.horizontal_scroll_modifier);
                let vertical = modifiers.matches_any(options.vertical_scroll_modifier);
                Some(if horizontal && !vertical {
                    egui::vec2(delta.x + delta.y, 0.0)
                } else if vertical && !horizontal {
                    egui::vec2(0.0, delta.x + delta.y)
                } else {
                    delta
                })
            }
            _ => None,
        })
        .fold(Vec2::ZERO, |sum, delta| sum + delta)
}

fn input_options(ctx: &egui::Context) -> egui::InputOptions {
    ctx.options(|options| options.input_options.clone())
}

/// Scale this frame's wheel/trackpad scrolling by the user's scroll speed.
///
/// Only `smooth_scroll_delta` is scaled: it is what every `ScrollArea` (and
/// the tab bar's wheel) moves by. The model and bitmap viewports zoom on
/// `raw_scroll_delta`, which is left alone so the setting cannot change how
/// fast they zoom. egui spreads a wheel notch over several frames and hands
/// out a slice per frame, so scaling each slice scales the whole notch.
/// Called once per frame, before any pane draws.
pub(in crate::app) fn apply_scroll_speed(ctx: &egui::Context, scroll_speed: f32) {
    if scroll_speed != 1.0 {
        ctx.input_mut(|input| input.smooth_scroll_delta *= scroll_speed);
    }
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

/// How tall a combo popup may grow before it scrolls. Generous enough that
/// ordinary lists size to their contents, while a block with hundreds of
/// elements still gets a scrollable popup rather than a full-screen one.
pub(in crate::app) const COMBO_POPUP_MAX_HEIGHT: f32 = 420.0;

/// True on the first frame a combo popup is drawn, so a caller can reveal the
/// selected row exactly once. Scrolling on every frame would fight the user's
/// own scrolling and the wheel-cycling above.
///
/// The popup's `Ui` id is stable while it stays open, and the popup only draws
/// while open, so a gap in the pass counter means it was closed in between.
/// That distinguishes "just opened" from "still open" without threading state
/// through any of the call sites.
pub(in crate::app) fn combo_popup_just_opened(ui: &Ui) -> bool {
    let id = ui.id().with("combo_popup_last_pass");
    let now = ui.ctx().cumulative_pass_nr();
    let last = ui.data(|data| data.get_temp::<u64>(id));
    ui.data_mut(|data| data.insert_temp(id, now));
    !matches!(last, Some(previous) if previous + 1 >= now)
}

/// Animation Player-style picker with a fixed search field and scrollable rows.
/// Labels are only built while open; filtered rows retain their block indices.
#[allow(clippy::too_many_arguments)]
fn searchable_block_selector(
    ui: &mut Ui,
    popup_id: egui::Id,
    selected_label: &str,
    selected_index: usize,
    count: usize,
    width: f32,
    element_label: &impl Fn(usize) -> String,
) -> (egui::Response, Option<usize>, bool) {
    searchable_block_selector_with_none(
        ui,
        popup_id,
        selected_label,
        selected_index,
        count,
        width,
        element_label,
        None,
    )
}

/// `usize::MAX` represents the optional unassigned reference row.
#[allow(clippy::too_many_arguments)]
fn searchable_block_selector_with_none(
    ui: &mut Ui,
    popup_id: egui::Id,
    selected_label: &str,
    selected_index: usize,
    count: usize,
    width: f32,
    element_label: &impl Fn(usize) -> String,
    none_label: Option<&str>,
) -> (egui::Response, Option<usize>, bool) {
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = picker_button(
        ui,
        popup_id,
        selected_label,
        count,
        width,
        foundation_block_text(),
        count > 0 || none_label.is_some(),
    );
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut selection = None;
    // Open while its id is open in memory, as the button above toggles it;
    // left alone, a popup built from a response is always open.
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_id = popup_id.with("entry_search");
            let mut filter = ui
                .data(|data| data.get_temp::<String>(filter_id))
                .unwrap_or_default();
            let filter_changed =
                picker_search_field(ui, &mut filter, "search entries…", just_opened);
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            ui.separator();
            let query = filter.trim().to_lowercase();
            picker_results(ui, COMBO_POPUP_MAX_HEIGHT, filter_changed, |ui| {
                let mut shown = 0;
                if let Some(label) = none_label
                    && (query.is_empty() || label.to_lowercase().contains(&query))
                {
                    shown += 1;
                    let row = ui.selectable_label(selected_index == usize::MAX, label);
                    if just_opened && query.is_empty() && selected_index == usize::MAX {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        selection = Some(usize::MAX);
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                for index in 0..count {
                    let label = element_label(index);
                    if !query.is_empty() && !label.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    let row = ui.selectable_label(index == selected_index, label);
                    if just_opened && query.is_empty() && index == selected_index {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        selection = Some(index);
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No entries match.").color(subtle_dark()));
                }
            });
        });
    (response, selection, popup_open)
}

pub(in crate::app) fn combo_box_with_scroll<R>(
    ui: &mut Ui,
    combo: egui::ComboBox,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (egui::InnerResponse<Option<R>>, Option<i32>) {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size.y = BUTTON_HEIGHT;
            ui.spacing_mut().button_padding.y = 2.0;
            // egui sizes a combo popup to its contents and only scrolls once
            // they exceed `combo_height`, so this is the clamp -- not a fixed
            // height. The stock 200pt starts scrolling after a handful of rows,
            // which hides entries that are simply below the fold: a block's
            // instance selector would scroll rather than grow the moment an
            // element was added.
            ui.spacing_mut().combo_height = COMBO_POPUP_MAX_HEIGHT;
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
    // Hovering is not enough. A wheel gesture belongs to whoever it started on,
    // and a gesture that began as panel scrolling keeps the wheel for its whole
    // duration -- otherwise scrolling down a tag silently retunes every dropdown
    // that passes under the cursor, which is exactly what it did.
    if !claim_wheel_gesture(ui.ctx(), response.id) {
        return None;
    }
    let delta = wheel_event_delta_from_context(ui.ctx());
    consume_mouse_wheel(ui);
    if delta == 0 {
        return None;
    }
    Some(delta)
}

/// Who the in-flight wheel gesture belongs to.
#[derive(Clone, Copy, PartialEq)]
enum WheelOwner {
    /// A gesture is in flight and nothing has claimed it yet, because the frame
    /// it started on has not finished drawing.
    Undecided,
    /// A widget that acts on the wheel itself — a dropdown cycling, a
    /// viewport zooming — claimed it and keeps it until the gesture ends.
    Widget(egui::Id),
    /// Nothing claimed it, so it is panel scrolling and stays that way.
    Panel,
}

#[derive(Clone, Copy)]
struct WheelGesture {
    /// When a wheel event was last seen. A quiet gap ends the gesture.
    last: f64,
    owner: WheelOwner,
}

/// How long the wheel must be still before the next event starts a new gesture.
///
/// Long enough to span the gaps between physical wheel detents, short enough
/// that deliberately stopping and pointing at a dropdown starts a fresh one.
const WHEEL_GESTURE_GAP: f64 = 0.25;

fn wheel_gesture_id() -> egui::Id {
    egui::Id::new("wheel_gesture")
}

fn wheel_is_turning(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::MouseWheel { .. }))
    })
}

/// Open a wheel gesture, or continue the one already in flight. Call once per
/// frame **before** anything that might claim the wheel.
pub(in crate::app) fn begin_wheel_gesture(ctx: &egui::Context) {
    if !wheel_is_turning(ctx) {
        return;
    }
    let now = ctx.input(|input| input.time);
    let previous = ctx.data(|data| data.get_temp::<WheelGesture>(wheel_gesture_id()));
    let owner = match previous {
        Some(g) if now - g.last <= WHEEL_GESTURE_GAP => g.owner,
        // First event after a quiet spell: a new gesture, up for grabs.
        _ => WheelOwner::Undecided,
    };
    ctx.data_mut(|data| data.insert_temp(wheel_gesture_id(), WheelGesture { last: now, owner }));
}

/// Settle an unclaimed gesture as panel scrolling. Call once per frame **after**
/// everything that might claim the wheel.
///
/// This is what makes ownership sticky: a gesture nothing claimed on its first
/// frame is the panel's, and stays the panel's even when the cursor later slides
/// over a dropdown.
pub(in crate::app) fn end_wheel_gesture(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(gesture) = data.get_temp::<WheelGesture>(wheel_gesture_id()) {
            if gesture.owner == WheelOwner::Undecided {
                data.insert_temp(
                    wheel_gesture_id(),
                    WheelGesture {
                        owner: WheelOwner::Panel,
                        ..gesture
                    },
                );
            }
        }
    });
}

/// The wheel's vertical travel this frame, for a viewport that zooms on it.
///
/// Like a dropdown, a viewport may only take a wheel gesture that began on
/// it: one that started as panel scrolling keeps scrolling the panel when the
/// cursor slides over the viewport, instead of the viewport stealing it to
/// zoom. A gesture the viewport does own is consumed — including the smoothed
/// tail egui spreads over later frames — so the panel under it does not also
/// scroll while it zooms. Scaled by the user's zoom speed.
pub(in crate::app) fn viewport_wheel_zoom(ui: &Ui, response: &egui::Response) -> Option<f32> {
    if !response.hovered() || !claim_wheel_gesture(ui.ctx(), response.id) {
        return None;
    }
    let options = input_options(ui.ctx());
    let scroll = ui.input(|input| raw_wheel_delta(input, &options).y);
    ui.ctx().input_mut(|input| {
        input
            .events
            .retain(|event| !matches!(event, egui::Event::MouseWheel { .. }));
        input.smooth_scroll_delta = Vec2::ZERO;
    });
    let zoom_speed = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(zoom_speed_id()))
        .unwrap_or(1.0);
    (scroll.abs() > f32::EPSILON).then_some(scroll * zoom_speed)
}

/// Publish the user's viewport zoom speed for [`viewport_wheel_zoom`], once
/// per frame — the viewports draw far from `Baboon`, like the dropdowns that
/// [`set_combo_scroll_cycle_enabled`] reaches the same way.
pub(in crate::app) fn set_zoom_speed(ctx: &egui::Context, zoom_speed: f32) {
    ctx.data_mut(|data| data.insert_temp(zoom_speed_id(), zoom_speed));
}

fn zoom_speed_id() -> egui::Id {
    egui::Id::new("viewport_zoom_speed")
}

/// Whether `id` may act on this frame's wheel events.
fn claim_wheel_gesture(ctx: &egui::Context, id: egui::Id) -> bool {
    let Some(gesture) = ctx.data(|data| data.get_temp::<WheelGesture>(wheel_gesture_id())) else {
        return false;
    };
    match gesture.owner {
        WheelOwner::Widget(owner) => owner == id,
        WheelOwner::Panel => false,
        WheelOwner::Undecided => {
            ctx.data_mut(|data| {
                data.insert_temp(
                    wheel_gesture_id(),
                    WheelGesture {
                        owner: WheelOwner::Widget(id),
                        ..gesture
                    },
                );
            });
            true
        }
    }
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
    let icon = if open {
        ButtonIcon::Opened
    } else {
        ButtonIcon::Closed
    };
    icon_button(
        ui,
        icon,
        if open { "Collapse" } else { "Expand" },
        enabled,
        foundation_block_text(),
    )
}

pub(in crate::app) fn foundation_selected_width(row_width: f32) -> f32 {
    (row_width - 190.0 - 24.0 * 3.0 - 54.0 * 5.0 - 92.0 - 32.0).clamp(120.0, 420.0)
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
    icon_button(ui, icon, tooltip, enabled, text_dark()).clicked()
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
            text_dark(),
        )
    } else {
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(label).color(text_dark()))
                .min_size(Vec2::new(54.0, BUTTON_HEIGHT)),
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
    // A field navigation selects the element holding its target, once per
    // pane: after that the user's own paging through the block wins, even
    // while the target is still glowing.
    if let Some((nav, index)) = edit.field_nav.and_then(|nav| {
        (nav.tag_key == edit.tag_key)
            .then(|| nav.block_indices.iter().find(|(block, _)| block == path))
            .flatten()
            .map(|(_, index)| (nav, *index))
    }) {
        let applied_id = edit.widget_id(("block_sel_nav", path));
        let applied = ui.data(|d| d.get_temp::<f64>(applied_id));
        if applied != Some(nav.glow_until) {
            ui.data_mut(|d| {
                d.insert_temp(id, index);
                d.insert_temp(applied_id, nav.glow_until);
            });
        }
    }
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
            FindTargetKind::Label,
            true,
            None,
            |ui| {
                Frame::NONE
                    .fill(foundation_group_bg())
                    .corner_radius(foundation_body_rounding())
                    .inner_margin(egui::Margin {
                        left: (8.0 + depth as f32 * 6.0) as i8,
                        right: 6,
                        top: 5,
                        bottom: 5,
                    })
                    .show(ui, add_contents);
            },
        );
    });
}

/// The dropdown label of every element of `target`: what the block-index
/// picker labels each row, a row at a time.
#[cfg(test)]
pub(in crate::app) fn block_index_target_labels(
    root: Option<TagStruct<'_>>,
    target: &BlockIndexTarget,
    names: &TagNameIndex,
) -> Vec<String> {
    let block = root
        .and_then(|root| root.field_path(&target.path))
        .and_then(|field| field.as_block());
    (0..target.len)
        .map(|index| {
            block_element_dropdown_label(
                block.as_ref().and_then(|b| b.element(index)),
                names,
                index,
            )
        })
        .collect()
}

pub(in crate::app) fn block_index_target_options(
    tag_struct: &TagStruct<'_>,
    field: &TagField<'_>,
    root: Option<TagStruct<'_>>,
    struct_path: &str,
) -> Option<BlockIndexTarget> {
    let target = field.definition().block_index_target()?;
    crate::app::editor::declared_block_index_target(tag_struct, root, struct_path, target.name())
}

/// A block-index field rendered like Foundation: a dropdown of the target
/// block's elements with a leading `<none>` (value −1), plus a "go to" button
/// that scrolls to the referenced element.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_foundation_block_index_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    current: i64,
    target: &BlockIndexTarget,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let target_block_path = target.path.as_str();
    let editable = edit.editable && !meta.read_only;
    let in_range = current >= 0 && (current as usize) < target.len;
    // Only the selected element's label is needed to draw a closed combo. The
    // rest used to be built here too, every frame, for every block-index field
    // on screen: one recursive walk per element of the target block.
    let root = edit.root;
    let default_names = TagNameIndex::default();
    let names = edit.names.unwrap_or(&default_names);
    let labeler = BlockLabeler::new(edit, names);
    let target_label = |block: Option<TagBlock<'_>>, index: usize| match block {
        Some(block) => labeler.label(target_block_path, block, index),
        None => format!("{index}."),
    };
    let selected_text = if in_range {
        let block = root
            .and_then(|root| root.field_path(target_block_path))
            .and_then(|field| field.as_block());
        target_label(block, current as usize)
    } else {
        "<none>".to_owned()
    };

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());

        if editable {
            let mut new_index: Option<i64> = None;
            let block = std::cell::OnceCell::new();
            let popup_id = ui.make_persistent_id((
                "block_index",
                edit.view_scope,
                edit.tag_key,
                path,
                target.len,
            ));
            let (response, selection, popup_open) = searchable_block_selector_with_none(
                ui,
                popup_id,
                &selected_text,
                if in_range {
                    current as usize
                } else {
                    usize::MAX
                },
                target.len,
                300.0,
                &|index| {
                    let block = block.get_or_init(|| {
                        root.and_then(|root| root.field_path(target_block_path))
                            .and_then(|field| field.as_block())
                    });
                    target_label(*block, index)
                },
                Some("<none>"),
            );
            if let Some(index) = selection {
                new_index = Some(if index == usize::MAX {
                    -1
                } else {
                    index as i64
                });
            }
            let wheel_delta = dropdown_wheel_delta(ui, &response, popup_open);
            if let Some(delta) = wheel_delta {
                if let Some(next) = combo_scroll_next_i64(current, -1, target.len as i64 - 1, delta)
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
        let go_to_tooltip = format!("Go to referenced element\n{target_block_path}[{current}]");
        let go_to = icon_button(
            ui,
            ButtonIcon::JumpTo,
            &go_to_tooltip,
            in_range,
            text_dark(),
        );
        let go_to = if in_range {
            go_to
        } else {
            go_to.on_disabled_hover_text("No referenced element (index is <none>)")
        };
        if go_to.clicked() {
            ui.data_mut(|d| d.insert_temp(jump_target_id(), target_block_path.to_owned()));
            set_block_selected_index(ui, edit, target_block_path, current as usize);
            ui.ctx().request_repaint();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Block headers and block-index dropdowns show the game's own editor
    /// label (blam-tags `element_label`) with Baboon's `N. ` prefix, not
    /// Baboon's content guess. The prefix isn't doubled on the editor's own
    /// `"N. <struct>"` fallback, and a Halo 2 entry is found because the
    /// labeller passes the tag's group.
    #[test]
    fn block_labels_come_from_the_games_editor() {
        let defs = crate::core::bundled::locate_definitions_root();
        let names = TagNameIndex::default();
        let h3 = crate::core::test_kits::h3ek_tags();
        let chud = h3.join("ui/chud/globals.chud_globals_definition");
        if chud.exists() {
            let game = GameId::from_id("halo3_mcc");
            let tag = crate::core::source::read_tag_at_path(&chud, game, Some(&defs), u32::from_be_bytes(*b"chgd")).unwrap();
            let labeler = BlockLabeler::for_tag(Some(&defs), game, tag.group().tag, Some(tag.root()), &names);
            let skins = tag.root().field_path("skins").unwrap().as_block().unwrap();
            assert_eq!(labeler.label("skins", skins, 1), "1. dervish");
            let jungle = crate::core::source::read_tag_at_path(
                &h3.join("levels/solo/010_jungle/010_jungle.scenario"), game, Some(&defs), u32::from_be_bytes(*b"scnr")).unwrap();
            let labeler = BlockLabeler::for_tag(Some(&defs), game, jungle.group().tag, Some(jungle.root()), &names);
            let pvs = jungle.root().field_path("zone set pvs").unwrap().as_block().unwrap();
            assert_eq!(labeler.label("zone set pvs", pvs, 0), "0. scenario_zone_set_pvs_block");
        } else {
            eprintln!("skipped the H3 half: set BLAM_TEST_H3EK");
        }
        let h2 = crate::core::test_kits::h2ek_tags();
        let delta = h2.join("scenarios/solo/08b_deltacontrol/08b_deltacontrol.scenario");
        if delta.exists() {
            let game = GameId::from_id("halo2_mcc");
            let tag = crate::core::source::read_tag_at_path(&delta, game, Some(&defs), u32::from_be_bytes(*b"scnr")).unwrap();
            let labeler = BlockLabeler::for_tag(Some(&defs), game, tag.group().tag, Some(tag.root()), &names);
            let controls = tag.root().field_path("controls").unwrap().as_block().unwrap();
            assert_eq!(labeler.label("controls", controls, 0), "0. s8_hunter_door_switch dcr_holo_switch");
        } else {
            eprintln!("skipped the H2 half: set BLAM_TEST_H2EK");
        }
    }
    use crate::core::source::{load_iostore_container_set, read_entry};
    use std::path::{Path, PathBuf};

    #[test]
    fn picker_popup_matches_button_width_with_long_entries_and_reference_none() {
        for width in [150.0, 300.0, 700.0] {
            let ctx = egui::Context::default();
            ctx.set_fonts(foundation_fonts());
            ctx.set_global_style(foundation_style());
            let popup_id = egui::Id::new("reference_picker_width");
            let mut button_rect = egui::Rect::NOTHING;
            egui::Popup::open_id(&ctx, popup_id);
            for query in ["", "missing", ""] {
                ctx.data_mut(|data| {
                    data.insert_temp(popup_id.with("entry_search"), query.to_owned())
                });
                for _ in 0..5 {
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
                                button_rect = searchable_block_selector_with_none(
                                    ui,
                                    popup_id,
                                    "<none>",
                                    usize::MAX,
                                    2,
                                    width,
                                    &|index| {
                                        format!(
                                            "{index}. {}",
                                            "very long block entry name ".repeat(8)
                                        )
                                    },
                                    Some("<none>"),
                                )
                                .0
                                .rect;
                            });
                        },
                    );
                }
                let popup_rect = ctx.memory(|memory| memory.area_rect(popup_id).unwrap());
                assert!(
                    (popup_rect.width() - button_rect.width()).abs() < 1.0,
                    "button {button_rect:?}, popup {popup_rect:?}, query {query}"
                );
            }
            // Unassigning remains available even when the referenced block is empty.
            let mut choice = None;
            let frame = |events, choice: &mut Option<usize>| {
                crate::app::run_ui_test(&ctx, 
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            *choice = searchable_block_selector_with_none(
                                ui,
                                popup_id,
                                "<none>",
                                usize::MAX,
                                0,
                                width,
                                &|_| unreachable!("empty blocks have no entry labels"),
                                Some("<none>"),
                            )
                            .1;
                        });
                    },
                )
            };
            let output = frame(Vec::new(), &mut choice);
            let pos = output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape)
                        if shape.galley.text() == "<none>"
                            && shape.pos.y > button_rect.bottom() =>
                    {
                        Some(shape.pos + shape.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("unassigned row");
            let pointer = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(
                vec![egui::Event::PointerMoved(pos), pointer(true)],
                &mut choice,
            );
            frame(vec![pointer(false)], &mut choice);
            assert_eq!(choice, Some(usize::MAX));
        }
    }

    #[test]
    fn clearing_search_restores_the_full_popup_height() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let popup_id = egui::Id::new("test_block_search_resize");
        let frame = |events| {
            let mut response = None;
            let _ = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        response = Some(searchable_block_selector(
                            ui,
                            popup_id,
                            "0. entry 0",
                            0,
                            40,
                            240.0,
                            &|index| format!("{index}. entry {index}"),
                        ));
                    });
                },
            );
            response.unwrap().0
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let pos = frame(Vec::new()).rect.center();
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        frame(vec![pointer(pos, false)]);
        for _ in 0..5 {
            frame(Vec::new());
        }
        let height = || ctx.memory(|memory| memory.area_rect(popup_id).unwrap().height());
        let full_height = height();
        frame(vec![egui::Event::Text("entry 39".to_owned())]);
        for _ in 0..5 {
            frame(Vec::new());
        }
        let filtered_height = height();
        assert!(
            full_height - filtered_height > 200.0,
            "filtering should hug its single result"
        );
        let search_id = ctx
            .memory(|memory| memory.focused())
            .expect("search has focus");
        let search = ctx.read_response(search_id).unwrap();
        let clear_response = ctx
            .read_response(search_id.with("clear_search"))
            .expect("clear control exists");
        assert!(
            (clear_response.rect.right() - search.rect.right()).abs() < 0.1,
            "clear control must reach the search field's outer right edge"
        );
        let clear_pos = clear_response.rect.center();
        frame(vec![
            egui::Event::PointerMoved(clear_pos),
            pointer(clear_pos, true),
        ]);
        frame(vec![pointer(clear_pos, false)]);
        for _ in 0..5 {
            frame(Vec::new());
        }
        assert_eq!(
            ctx.data(|data| data.get_temp::<String>(popup_id.with("entry_search")))
                .as_deref(),
            Some(""),
            "search {:?}; clear {:?}",
            search.rect,
            ctx.read_response(search_id.with("clear_search"))
        );
        assert!(
            egui::Popup::is_id_open(&ctx, popup_id),
            "clearing keeps the picker open"
        );
        assert!(
            (height() - full_height).abs() < 1.0,
            "the cleared popup must recover its original height"
        );
    }

    #[test]
    fn typing_filters_without_closing_and_selects_the_original_entry_index() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let popup_id = egui::Id::new("test_block_search");
        let labels_built = std::cell::Cell::new(0);
        let labels = ["0. minor", "1. major", "2. minor veteran"];
        let frame = |events| {
            let mut result = None;
            let output = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        result = Some(searchable_block_selector(
                            ui,
                            popup_id,
                            labels[0],
                            0,
                            labels.len(),
                            240.0,
                            &|index| {
                                labels_built.set(labels_built.get() + 1);
                                labels[index].to_owned()
                            },
                        ));
                    });
                },
            );
            (result.unwrap(), output)
        };
        let ((response, _, _), _) = frame(Vec::new());
        assert_eq!(
            labels_built.get(),
            0,
            "closed pickers must build no list labels"
        );
        let button_pos = response.rect.center();
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![
            egui::Event::PointerMoved(button_pos),
            pointer(button_pos, true),
        ]);
        frame(vec![pointer(button_pos, false)]);
        frame(Vec::new());
        let ((_, _, open), output) = frame(vec![egui::Event::Text("VETERAN".to_owned())]);
        assert!(open, "typing must keep the popup open");
        assert_eq!(
            ctx.data(|data| data.get_temp::<String>(popup_id.with("entry_search")))
                .as_deref(),
            Some("VETERAN")
        );
        let text_rect = |text: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == text => {
                        Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                    }
                    _ => None,
                })
        };
        assert!(
            text_rect(labels[1]).is_none(),
            "nonmatching entries must be hidden"
        );
        assert!(
            text_rect("(3)").is_some(),
            "the count is the block total, not the number of matches"
        );
        let row_pos = text_rect(labels[2])
            .expect("case-insensitive search match")
            .center();
        frame(vec![
            egui::Event::PointerMoved(row_pos),
            pointer(row_pos, true),
        ]);
        let ((_, selection, _), _) = frame(vec![pointer(row_pos, false)]);
        assert_eq!(
            selection,
            Some(2),
            "filtered results must preserve the source index"
        );
        assert!(!egui::Popup::is_id_open(&ctx, popup_id));
        ctx.data_mut(|data| {
            data.insert_temp(popup_id.with("entry_search"), "no such entry".to_owned())
        });
        egui::Popup::open_id(&ctx, popup_id);
        let (_, output) = frame(Vec::new());
        assert!(output.shapes.iter().any(|clipped| matches!(&clipped.shape,
            egui::Shape::Text(shape) if shape.galley.text() == "No entries match.")));
    }

    #[test]
    fn block_jump_matches_exact_paths_and_opens_ancestors() {
        let target = "regions#4[2]/permutations#7";
        assert!(block_paths_match(target, "regions[2]/permutations"));
        assert!(!block_paths_match(target, "regions[1]/permutations"));
        assert!(block_jump_opens_path(target, "regions#4"));
        assert!(block_jump_opens_path(target, target));
        assert!(!block_jump_opens_path(target, "materials#5"));
    }

    #[test]
    fn block_jump_scroll_keeps_two_rows_of_leading_context() {
        let header = egui::Rect::from_min_size(
            egui::pos2(12.0, 240.0),
            egui::vec2(600.0, 40.0),
        );
        let target = block_jump_scroll_rect(header);

        assert_eq!(target.top(), 160.0);
        assert_eq!(target.bottom(), header.bottom());
        assert_eq!(target.left(), header.left());
        assert_eq!(target.right(), header.right());
    }

    #[test]
    fn block_index_target_uses_the_renderers_exact_widget_path() {
        let tag = TagFile::new(crate::app::test_definition_path(
            "haloreach_mcc/test_tag.json",
        ))
        .unwrap();
        let root = tag.root();
        let index = root
            .fields_all()
            .find(|field| field.name() == "short block index")
            .unwrap();
        let target = block_index_target_options(&root, &index, Some(root), "")
            .unwrap()
            .path;

        assert!(target.contains('#'), "target should carry an exact ordinal");
        assert!(
            root.field_path(&target).is_some(),
            "target path should resolve exactly"
        );
    }

    /// Reproduction probe for "a tag added to the scenario vehicle palette does
    /// not appear in the vehicles block's dropdown until save + reopen".
    /// Runs the same add-then-set-reference flow on every editing kit present.
    #[test]
    fn palette_addition_reaches_the_block_index_dropdown() {
        let cases = [
            ("halo3_mcc", "levels/multi/riverworld/riverworld.scenario"),
            ("haloreach_mcc", "levels/multi/35_island/35_island.scenario"),
            (
                "halo2_mcc",
                "scenarios/solo/05a_deltaapproach/05a_deltaapproach.scenario",
            ),
            ("haloce_mcc", "levels/d40/d40.scenario"),
        ];
        let defs = std::path::Path::new("definitions");
        let names = crate::core::format::TagNameIndex::default();
        let group = u32::from_be_bytes(*b"scnr");
        let mut failures = Vec::new();

        for (game, rel) in cases {
            let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(game, "")).join(rel);
            if !tag_path.exists() {
                eprintln!("skip {game}: {} missing", tag_path.display());
                continue;
            }
            let Ok(mut tag) =
                crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
            else {
                eprintln!("skip {game}: could not read scenario");
                continue;
            };

            // The dropdown a `vehicles` element's block-index field would show.
            let options = |tag: &blam_tags::TagFile| -> Option<Vec<String>> {
                let root = tag.root();
                for field in root.fields_all() {
                    if let Some(block) = field.as_block()
                        && field.name() == "vehicles"
                        && let Some(element) = block.element(0)
                    {
                        for sub in element.fields_all() {
                            if sub.definition().block_index_target().is_some()
                                && let Some(target) = block_index_target_options(
                                    &element,
                                    &sub,
                                    Some(root),
                                    "vehicles[0]",
                                )
                                && target.path.contains("palette")
                            {
                                return Some(block_index_target_labels(
                                    Some(root),
                                    &target,
                                    &names,
                                ));
                            }
                        }
                    }
                }
                None
            };
            let palette_len = |tag: &blam_tags::TagFile| -> Option<usize> {
                tag.root().fields_all().find_map(|field| {
                    (field.name() == "vehicle palette")
                        .then(|| field.as_block().map(|block| block.len()))
                        .flatten()
                })
            };

            let Some(before) = options(&tag) else {
                eprintln!("skip {game}: no vehicles block-index field resolved");
                continue;
            };
            let before_len = palette_len(&tag).unwrap_or(0);
            let mut dirty = Dirty::default();
            let status = crate::core::document::apply::apply_block_ops(
                &mut tag,
                vec![BlockOp {
                    path: "vehicle palette".to_owned(),
                    kind: BlockOpKind::Add,
                }],
                &mut dirty,
            );
            let after_len = palette_len(&tag).unwrap_or(0);
            let after = options(&tag).unwrap_or_default();
            eprintln!(
                "{game}: palette {before_len} -> {after_len}, dropdown {} -> {} ({status:?})",
                before.len(),
                after.len()
            );
            if after_len != before_len + 1 {
                failures.push(format!("{game}: palette did not grow"));
                continue;
            }
            if after.len() != before.len() + 1 {
                failures.push(format!(
                    "{game}: dropdown stayed at {} option(s) after the palette grew to {after_len}",
                    after.len()
                ));
                continue;
            }
            // And the label follows the reference the user then sets.
            let edit = PendingFieldEdit {
                path: format!("vehicle palette[{before_len}]/name"),
                input: "objects/vehicles/warthog/warthog.vehicle".to_owned(),
            };
            let applied =
                crate::core::document::apply::apply_pending_edits(&mut tag, vec![edit], &mut dirty);
            let labelled = options(&tag).unwrap_or_default();
            let last = labelled.last().cloned().unwrap_or_default();
            eprintln!("{game}: new label {last:?} ({:?})", applied.status);
            if !last.contains("warthog") {
                failures.push(format!(
                    "{game}: label did not pick up the reference: {last:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Every block-index field in a tag, at every depth, with the struct path
    /// the app itself would render it under (`name#ordinal` segments) -- so the
    /// ancestor walk in `block_index_target_options` is exercised through
    /// `root.descend`, not just the root-level shortcut.
    fn collect_block_index_fields(
        st: &blam_tags::TagStruct<'_>,
        path: &str,
        out: &mut Vec<String>,
        depth: usize,
    ) {
        if depth > 6 || out.len() > 400 {
            return;
        }
        for field in st.fields_all() {
            let field_path = crate::core::document::value::append_field_path_for(path, &field);
            if field.definition().block_index_target().is_some() {
                out.push(path.to_owned());
            }
            if let Some(block) = field.as_block()
                && block.len() > 0
                && let Some(element) = block.element(0)
            {
                let element_path = format!("{field_path}[0]");
                collect_block_index_fields(&element, &element_path, out, depth + 1);
            }
        }
    }

    /// The general form of Crisp's report: for a block-index field anywhere in
    /// a tag, adding an element to the block it targets must be offered by the
    /// dropdown immediately, with no save and reopen.
    #[test]
    fn adding_to_a_targeted_block_reaches_its_dropdown_at_every_depth() {
        let defs = std::path::Path::new("definitions");
        let cases = [
            (
                "halo3_mcc",
                "levels/multi/riverworld/riverworld.scenario",
                *b"scnr",
            ),
            (
                "haloreach_mcc",
                "levels/multi/35_island/35_island.scenario",
                *b"scnr",
            ),
        ];
        let mut failures = Vec::new();
        let mut checked = 0usize;

        for (game, rel, group_bytes) in cases {
            let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(game, "")).join(rel);
            if !tag_path.exists() {
                eprintln!("skip {game}: {} missing", tag_path.display());
                continue;
            }
            let group = u32::from_be_bytes(group_bytes);
            let Ok(tag) = crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
            else {
                eprintln!("skip {game}: unreadable");
                continue;
            };
            let mut struct_paths = Vec::new();
            collect_block_index_fields(&tag.root(), "", &mut struct_paths, 0);
            struct_paths.sort();
            struct_paths.dedup();
            eprintln!(
                "{game}: {} struct(s) holding block-index fields",
                struct_paths.len()
            );

            for struct_path in struct_paths.iter().take(40) {
                // Re-read per case: each one mutates the tag.
                let Ok(mut tag) =
                    crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
                else {
                    continue;
                };
                let resolve = |tag: &blam_tags::TagFile| -> Option<(String, usize)> {
                    let root = tag.root();
                    let st = if struct_path.is_empty() {
                        root
                    } else {
                        root.descend(struct_path)?
                    };
                    for field in st.fields_all() {
                        if field.definition().block_index_target().is_some()
                            && let Some(target) =
                                block_index_target_options(&st, &field, Some(root), struct_path)
                        {
                            return Some((target.path, target.len));
                        }
                    }
                    None
                };
                let Some((target, before)) = resolve(&tag) else {
                    continue;
                };
                let mut dirty = Dirty::default();
                let status = crate::core::document::apply::apply_block_ops(
                    &mut tag,
                    vec![BlockOp {
                        path: target.clone(),
                        kind: BlockOpKind::Add,
                    }],
                    &mut dirty,
                );
                if status.as_deref().is_some_and(|s| s.contains("failed")) {
                    eprintln!("  {struct_path:?} -> {target:?}: add failed: {status:?}");
                    continue;
                }
                let after = resolve(&tag).map(|(_, n)| n).unwrap_or(0);
                checked += 1;
                if after != before + 1 {
                    failures.push(format!(
                        "{game} {struct_path:?} -> target {target:?}: {before} option(s) before, \
                         {after} after adding an element (expected {})",
                        before + 1
                    ));
                }
            }
        }
        eprintln!("checked {checked} block-index field(s)");
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Crisp's report, stated exactly: adding an element to a block must make
    /// that block's own instance selector offer it, without a save and reopen.
    /// The selector lists `0..block.len()`, so this checks the length the
    /// renderer would read on the next frame.
    #[test]
    fn adding_an_element_grows_the_blocks_own_length() {
        let defs = std::path::Path::new("definitions");
        let cases = [
            (
                "halo3_mcc",
                "levels/multi/riverworld/riverworld.scenario",
                *b"scnr",
            ),
            (
                "haloreach_mcc",
                "levels/multi/35_island/35_island.scenario",
                *b"scnr",
            ),
            (
                "halo2_mcc",
                "scenarios/solo/05a_deltaapproach/05a_deltaapproach.scenario",
                *b"scnr",
            ),
            ("haloce_mcc", "levels/d40/d40.scenario", *b"scnr"),
        ];
        let mut failures = Vec::new();

        for (game, rel, group_bytes) in cases {
            let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(game, "")).join(rel);
            if !tag_path.exists() {
                eprintln!("skip {game}: missing");
                continue;
            }
            let group = u32::from_be_bytes(group_bytes);
            let Ok(tag) = crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
            else {
                eprintln!("skip {game}: unreadable");
                continue;
            };
            // Root-level blocks, split by whether they start empty: an
            // empty-on-disk block is the case that has bitten before.
            let mut blocks: Vec<(String, usize)> = Vec::new();
            for field in tag.root().fields_all() {
                if let Some(block) = field.as_block() {
                    blocks.push((field.name().to_owned(), block.len()));
                }
            }
            let empty = blocks.iter().filter(|(_, n)| *n == 0).count();
            eprintln!("{game}: {} root block(s), {empty} empty", blocks.len());

            let mut checked = 0usize;
            let mut stale = 0usize;
            for (name, before) in blocks {
                let Ok(mut tag) =
                    crate::core::source::read_tag_at_path(&tag_path, GameId::from_id(game), Some(defs), group)
                else {
                    continue;
                };
                let mut dirty = Dirty::default();
                let status = crate::core::document::apply::apply_block_ops(
                    &mut tag,
                    vec![BlockOp {
                        path: name.clone(),
                        kind: BlockOpKind::Add,
                    }],
                    &mut dirty,
                );
                if status.as_deref().is_some_and(|s| s.contains("failed")) {
                    continue;
                }
                let after = tag
                    .root()
                    .fields_all()
                    .find_map(|field| {
                        (field.name() == name)
                            .then(|| field.as_block().map(|block| block.len()))
                            .flatten()
                    })
                    .unwrap_or(0);
                checked += 1;
                if after != before + 1 {
                    stale += 1;
                    failures.push(format!(
                        "{game} block {name:?}: len {before} -> {after} after adding an element \
                         (status {status:?})"
                    ));
                }
            }
            eprintln!("{game}: checked {checked} block(s), {stale} stale");
        }
        assert!(
            failures.is_empty(),
            "{} failure(s):\n{failures:#?}",
            failures.len()
        );
    }

    /// One frame with one trackpad-sized scroll (small enough that egui
    /// applies it unsmoothed), returning `(smooth, raw)` as panes see them.
    fn scrolled(scroll_speed: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut seen = (0.0, 0.0);
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::Vec2::new(0.0, -4.0),
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |_| {
                apply_scroll_speed(&ctx, scroll_speed);
                let options = input_options(&ctx);
                seen = ctx.input(|input| {
                    (input.smooth_scroll_delta.y, raw_wheel_delta(input, &options).y)
                });
            },
        );
        seen
    }

    #[test]
    fn scroll_speed_scales_scrolling_but_not_viewport_zoom() {
        assert_eq!(scrolled(1.0), (-4.0, -4.0), "100% is egui's own speed");
        assert_eq!(scrolled(2.5), (-10.0, -4.0));
        assert_eq!(scrolled(0.5), (-2.0, -4.0));
    }

    /// A Windows wheel notch arrives in lines, which egui smooths over many
    /// frames; the distance travelled over the whole notch scales too.
    #[test]
    fn scroll_speed_scales_a_whole_smoothed_wheel_notch() {
        let travelled = |scroll_speed: f32| {
            let ctx = egui::Context::default();
            let mut total = 0.0;
            for frame in 0..240 {
                let events = if frame == 0 {
                    vec![egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::Vec2::new(0.0, -1.0),
                        modifiers: Default::default(),
                    }]
                } else {
                    Vec::new()
                };
                let _ = crate::app::run_ui_test(
                    &ctx,
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |_| {
                        apply_scroll_speed(&ctx, scroll_speed);
                        total += ctx.input(|input| input.smooth_scroll_delta.y);
                    },
                );
            }
            total
        };
        let base = travelled(1.0);
        assert!((base + 40.0).abs() < 0.01, "one line is 40 points: {base}");
        assert!((travelled(3.0) - base * 3.0).abs() < 0.01);
    }

    const VIEWPORT: egui::Rect = egui::Rect {
        min: egui::Pos2::new(0.0, 150.0),
        max: egui::Pos2::new(400.0, 350.0),
    };
    const OVER_PANEL: egui::Pos2 = egui::Pos2::new(200.0, 50.0);
    const OVER_VIEWPORT: egui::Pos2 = egui::Pos2::new(200.0, 250.0);

    /// One frame of a scrolling pane with a viewport in it: optionally a
    /// wheel event at `pointer`. Returns the viewport's zoom travel and the
    /// pane's scroll offset after the frame.
    fn frame(ctx: &egui::Context, wheel: bool, pointer: egui::Pos2) -> (Option<f32>, f32) {
        let mut events = vec![egui::Event::PointerMoved(pointer)];
        if wheel {
            // Trackpad-sized, so egui applies it this frame rather than
            // smoothing it over the next ones.
            events.push(egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::Vec2::new(0.0, -4.0),
                modifiers: Default::default(),
            });
        }
        let mut zoom = None;
        let mut offset = 0.0;
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(400.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                begin_wheel_gesture(ctx);
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        let output = egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                            ui.allocate_exact_size(
                                egui::Vec2::new(400.0, 150.0),
                                egui::Sense::hover(),
                            );
                            let (_, response) = ui.allocate_exact_size(
                                VIEWPORT.size(),
                                egui::Sense::click_and_drag(),
                            );
                            zoom = viewport_wheel_zoom(ui, &response);
                            ui.allocate_exact_size(
                                egui::Vec2::new(400.0, 4000.0),
                                egui::Sense::hover(),
                            );
                        });
                        offset = output.state.offset.y;
                    });
                end_wheel_gesture(ctx);
            },
        );
        (zoom, offset)
    }

    /// A pane that has been on screen a frame, as any the user scrolls has:
    /// egui hit-tests against the previous frame's widgets, so nothing is
    /// hovered on the first.
    fn ctx(pointer: egui::Pos2) -> egui::Context {
        let ctx = egui::Context::default();
        frame(&ctx, false, pointer);
        ctx
    }

    /// The reported defect: scrolling the pane, the cursor passes over the
    /// viewport, which took the wheel and zoomed instead of letting the pane
    /// keep scrolling.
    #[test]
    fn a_viewport_scrolled_past_mid_gesture_does_not_zoom() {
        let ctx = ctx(OVER_PANEL);
        let (zoom, mut offset) = frame(&ctx, true, OVER_PANEL);
        assert_eq!(zoom, None);
        assert!(offset > 0.0, "the pane scrolled");
        for _ in 0..5 {
            let (zoom, next) = frame(&ctx, true, OVER_VIEWPORT);
            assert_eq!(zoom, None, "the viewport stole a panel scroll");
            assert!(
                next > offset,
                "the pane stopped scrolling under the viewport"
            );
            offset = next;
        }
    }

    /// Deliberate use still zooms, and the pane under the viewport holds
    /// still while it does.
    #[test]
    fn a_viewport_pointed_at_first_zooms_and_the_pane_holds() {
        let ctx = ctx(OVER_VIEWPORT);
        for _ in 0..3 {
            let (zoom, offset) = frame(&ctx, true, OVER_VIEWPORT);
            assert_eq!(zoom, Some(-4.0));
            assert_eq!(offset, 0.0, "the pane scrolled under a zoom");
        }
    }

    #[test]
    fn zoom_speed_scales_the_zoom() {
        let ctx = ctx(OVER_VIEWPORT);
        set_zoom_speed(&ctx, 2.5);
        assert_eq!(frame(&ctx, true, OVER_VIEWPORT).0, Some(-10.0));
    }

    /// One frame: optionally a wheel event, and a dropdown at `rect` asking
    /// whether it may cycle. Returns whether it was allowed to.
    fn dropdown_frame(ctx: &egui::Context, wheel: bool, pointer: egui::Pos2, rect: egui::Rect) -> bool {
        let mut events = vec![egui::Event::PointerMoved(pointer)];
        if wheel {
            events.push(egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Line,
                delta: egui::Vec2::new(0.0, -1.0),
                modifiers: Default::default(),
            });
        }
        let mut claimed = false;
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(400.0, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                begin_wheel_gesture(ctx);
                egui::CentralPanel::default().show(ui, |ui| {
                    let response = ui.allocate_rect(rect, egui::Sense::hover());
                    claimed = dropdown_wheel_delta(ui, &response, false).is_some();
                });
                end_wheel_gesture(ctx);
            },
        );
        claimed
    }

    fn cycling_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        set_combo_scroll_cycle_enabled(&ctx, true);
        ctx
    }

    const BOX_RECT: egui::Rect = egui::Rect {
        min: egui::Pos2::new(10.0, 100.0),
        max: egui::Pos2::new(200.0, 120.0),
    };
    const OVER_BOX: egui::Pos2 = egui::Pos2::new(100.0, 110.0);
    const ABOVE_BOX: egui::Pos2 = egui::Pos2::new(100.0, 20.0);

    /// The reported defect: scrolling a tag pane, the cursor passes over a
    /// dropdown, and the dropdown silently changes value. The gesture started as
    /// panel scrolling and must stay panel scrolling.
    #[test]
    fn a_dropdown_scrolled_past_mid_gesture_does_not_change() {
        let ctx = cycling_ctx();
        // The gesture begins away from any dropdown.
        assert!(!dropdown_frame(&ctx, true, ABOVE_BOX, BOX_RECT));
        // Now the cursor slides over one while the wheel is still turning.
        for _ in 0..5 {
            assert!(
                !dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT),
                "a dropdown claimed a wheel gesture that began as panel scrolling"
            );
        }
    }

    /// Deliberate use still works: point at a dropdown, then scroll.
    #[test]
    fn a_dropdown_pointed_at_first_still_cycles() {
        let ctx = cycling_ctx();
        assert!(
            dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT),
            "a gesture starting on a dropdown should be the dropdown's"
        );
        // ...and keeps it for the rest of the gesture.
        assert!(dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT));
    }

    /// After the wheel goes quiet the next turn is a fresh gesture, so stopping
    /// and pointing at a dropdown works without moving the mouse away first.
    #[test]
    fn a_pause_starts_a_new_gesture() {
        let ctx = cycling_ctx();
        assert!(!dropdown_frame(&ctx, true, ABOVE_BOX, BOX_RECT));
        assert!(!dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT));
        // Frames with no wheel event: the gesture goes stale.
        for _ in 0..40 {
            dropdown_frame(&ctx, false, OVER_BOX, BOX_RECT);
        }
        assert!(
            dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT),
            "a new gesture over a dropdown should be claimable"
        );
    }

    /// The preference still wins outright.
    #[test]
    fn the_preference_disables_it_entirely() {
        let ctx = egui::Context::default();
        set_combo_scroll_cycle_enabled(&ctx, false);
        assert!(!dropdown_frame(&ctx, true, OVER_BOX, BOX_RECT));
    }

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn parent_block_path_and_breadcrumb() {
        assert_eq!(
            parent_block_path("regions[0]/permutations").as_deref(),
            Some("regions")
        );
        assert_eq!(parent_block_path("a/b/c").as_deref(), Some("a/b"));
        assert_eq!(parent_block_path("a/b[3]").as_deref(), Some("a"));
        assert_eq!(parent_block_path("regions"), None);

        assert_eq!(
            breadcrumb_for_path("regions[0]/permutations"),
            "regions › permutations"
        );
        assert_eq!(breadcrumb_for_path("variants"), "variants");
    }

    /// A closed block-index dropdown builds the label it shows, not one per
    /// element of its target block. Every block-index field on screen used to
    /// build all of them every frame, so a frame's labels grew with the size
    /// of every block an index pointed into.
    #[test]
    fn closed_block_index_dropdowns_do_not_label_every_target_element() {
        let mut tag = TagFile::new(crate::app::test_definition_path(
            "haloreach_mcc/test_tag.json",
        ))
        .unwrap();
        let target = {
            let root = tag.root();
            let index = root
                .fields_all()
                .find(|field| field.name() == "short block index")
                .unwrap();
            block_index_target_options(&root, &index, Some(root), "")
                .expect("the test tag's block index resolves")
                .path
        };
        let grow = |tag: &mut TagFile, by: usize| {
            for _ in 0..by {
                crate::core::document::apply::apply_block_ops(
                    tag,
                    vec![BlockOp {
                        path: target.clone(),
                        kind: BlockOpKind::Add,
                    }],
                    &mut Dirty::default(),
                );
            }
        };
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let labels_for_one_frame = |tag: &TagFile| {
            DROPDOWN_LABELS_BUILT.with(|count| count.set(0));
            with_test_edit_context(|edit| {
                let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        draw_fields_with_docs(
                            ui,
                            &tag.root(),
                            &TagNameIndex::default(),
                            0,
                            true,
                            "",
                            edit,
                            None,
                        );
                    });
                });
            });
            DROPDOWN_LABELS_BUILT.with(std::cell::Cell::get)
        };

        grow(&mut tag, 8);
        crate::core::document::apply::apply_field_edit(&mut tag, "short block index", "2").unwrap();
        let small = labels_for_one_frame(&tag);
        grow(&mut tag, 32);
        let large = labels_for_one_frame(&tag);

        assert_eq!(
            small, large,
            "a frame built {small} labels over 8 target elements and {large} over 40"
        );
    }

    /// A tag's field names have lost their trailing `*` and `!` (shipped
    /// tags store them stripped, and so do layouts built from the
    /// definitions), so read-only and hidden come from the definition. A `!`
    /// field shows only in expert mode, and a `*` field is marked read-only.
    #[test]
    fn read_only_and_hidden_come_from_the_definition() {
        let root = crate::core::test_kits::unique_temp_path("marker-definitions");
        let game = root.join("haloreach_mcc");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(
            game.join("marker_test.json"),
            r#"{"name":"marker_test","tag":"mrkt","version":1,"flags":0,"block":"marker_test_block",
                "blocks":{"marker_test_block":{"max_count":1,"struct":"marker_test_struct"}},
                "structs":{"marker_test_struct":{"guid":"0123456789abcdef0123456789abcdef","size":12,
                  "fields":[{"type":"long_integer","name":"visible"},{"type":"long_integer","name":"locked*"},
                            {"type":"long_integer","name":"secret!"},{"type":"terminator","name":null}]}}}"#,
        )
        .unwrap();
        let tag = TagFile::new(game.join("marker_test.json")).unwrap();
        assert!(
            tag.root().fields().all(|field| !field.name().contains(['*', '!'])),
            "the tag's own names should be stripped, as a shipped tag's are"
        );
        // The edit context's lifetime is the helper's own; test code may leak.
        let docs: &'static _ =
            Box::leak(Box::new(crate::app::help::build_def_docs(&root, GameId::HaloReach, "marker_test")));
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let texts = |expert_mode: bool| -> Vec<String> {
            let mut texts = Vec::new();
            with_test_edit_context(|edit| {
                edit.docs = Some(docs);
                for _ in 0..2 {
                    let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            draw_fields_with_docs(
                                ui,
                                &tag.root(),
                                &TagNameIndex::default(),
                                0,
                                expert_mode,
                                "",
                                edit,
                                None,
                            );
                        });
                    });
                    texts = output
                        .shapes
                        .iter()
                        .filter_map(|clipped| match &clipped.shape {
                            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                            _ => None,
                        })
                        .collect();
                }
            });
            texts
        };
        let normal = texts(false);
        assert!(normal.iter().any(|t| t == "visible"), "{normal:?}");
        assert!(normal.iter().any(|t| t == "locked"), "{normal:?}");
        assert!(!normal.iter().any(|t| t == "secret"), "a `!` field shows outside expert mode: {normal:?}");
        assert_eq!(
            normal.iter().filter(|t| *t == "read-only").count(),
            1,
            "only the `*` field is read-only: {normal:?}"
        );
        let expert = texts(true);
        assert!(expert.iter().any(|t| t == "secret"), "expert mode hides a `!` field: {expert:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A field navigation (reference jump, Find) selects the element holding
    /// its target in whatever pane draws the block. Panes are scoped `tile{N}`;
    /// the selection used to be written only under the pre-tiles `docked` /
    /// `floating` scopes, so no pane ever saw it.
    #[test]
    fn field_nav_selects_target_element_in_any_pane_scope() {
        let ctx = egui::Context::default();
        let nav = |glow_until: f64| FieldNav {
            kit: KitId(0),
            tag_key: "test".to_owned(),
            field_path: "sounds#2[3]/sound#0".to_owned(),
            block_indices: vec![("sounds#2".to_owned(), 3)],
            glow_until,
        };
        // The harness's context borrows for a lifetime local to its closure.
        let first: &'static FieldNav = Box::leak(Box::new(nav(10.0)));
        let second: &'static FieldNav = Box::leak(Box::new(nav(20.0)));
        let other_tag: &'static FieldNav = Box::leak(Box::new(FieldNav {
            tag_key: "other".to_owned(),
            ..nav(30.0)
        }));
        with_test_edit_context(|edit| {
            edit.view_scope = "tile7";
            let _ = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    assert_eq!(block_selected_index(ui, edit, "sounds#2", 5), 0);

                    edit.field_nav = Some(first);
                    assert_eq!(block_selected_index(ui, edit, "sounds#2", 5), 3);
                    // Blocks off the target path are left alone.
                    assert_eq!(block_selected_index(ui, edit, "other#4", 5), 0);

                    // Applied once: paging while the target still glows sticks.
                    set_block_selected_index(ui, edit, "sounds#2", 1);
                    assert_eq!(block_selected_index(ui, edit, "sounds#2", 5), 1);

                    // A new navigation to the same block selects again.
                    edit.field_nav = Some(second);
                    assert_eq!(block_selected_index(ui, edit, "sounds#2", 5), 3);

                    // Another tag's navigation does not move this pane.
                    set_block_selected_index(ui, edit, "sounds#2", 2);
                    edit.field_nav = Some(other_tag);
                    assert_eq!(block_selected_index(ui, edit, "sounds#2", 5), 2);
                });
            });
        });
    }

    #[test]
    fn screen_flash_explanation_fallback_present() {
        let text = known_explanation_text("screen flash").unwrap();
        assert!(text.contains("There are seven screen flash types"));
        assert!(text.contains("LIGHTEN"));

        assert!(text.contains("DST'"));
    }

    #[test]
    fn internal_placeholder_titles_do_not_leak() {
        assert_eq!(
            inline_function_label("dirty whore", "rumble/low frequency rumble"),
            "function"
        );
        assert_eq!(
            visible_container_title("dirty whore", "rumble/low frequency rumble"),
            "low frequency rumble"
        );
        assert!(is_internal_schema_marker_name("HIDE_GROUP_ID"));
        assert!(is_internal_schema_marker_name("END_HIDE_GROUP_ID"));
        assert!(is_internal_schema_marker_name("whore function"));
    }

    #[test]
    fn format_block_size_label_is_stable_and_human_readable() {
        assert_eq!(format_block_size_label(2, 36), "2 x 36 B = 72 B");
        assert_eq!(format_block_size_label(64, 36), "64 x 36 B = 2.2 KiB");
    }

    #[test]
    fn combo_scroll_next_index_clamps_and_uses_delta_direction() {
        assert_eq!(combo_scroll_next_index(1, 3, 1), Some(2));
        assert_eq!(combo_scroll_next_index(1, 3, 120), Some(2));
        assert_eq!(combo_scroll_next_index(1, 3, -1), Some(0));
        assert_eq!(combo_scroll_next_index(1, 3, -120), Some(0));
        assert_eq!(combo_scroll_next_index(0, 3, -1), None);
        assert_eq!(combo_scroll_next_index(2, 3, 1), None);
        assert_eq!(combo_scroll_next_index(0, 0, 1), None);
    }

    #[test]
    fn foundation_selected_width_reserves_only_current_header_cells() {
        assert_eq!(foundation_selected_width(1_000.0), 344.0);
        assert_eq!(foundation_selected_width(500.0), 120.0);
        assert_eq!(foundation_selected_width(2_000.0), 420.0);
    }

    static PAKS: std::sync::LazyLock<&'static str> =
        std::sync::LazyLock::new(|| crate::core::test_kits::leak(crate::core::test_kits::ce_paks()));

    /// Mirrors how the editor builds paths: the inherited-parent chain
    /// contributes a name-only prefix, leaves add `name#ordinal`.
    fn collect(st: blam_tags::TagStruct<'_>, prefix: &str, out: &mut Vec<String>, depth: usize) {
        if depth > 6 {
            return;
        }
        for (chain_struct, chain_prefix) in crate::app::editor::fields::inherited_struct_chain(st) {
            let base = if chain_prefix.is_empty() {
                prefix.to_string()
            } else if prefix.is_empty() {
                chain_prefix.clone()
            } else {
                format!("{prefix}/{chain_prefix}")
            };
            for field in chain_struct.fields() {
                if crate::app::editor::fields::is_inherited_parent_name(field.name()) {
                    continue;
                }
                let path = crate::app::editor::append_field_path_for(&base, &field);
                if let Some(block) = field.as_block() {
                    if let Some(child) = block.element(0) {
                        collect(child, &format!("{path}[0]"), out, depth + 1);
                    }
                } else if let Some(child) = field.as_struct() {
                    collect(child, &path, out, depth + 1);
                } else if field.value().is_some() {
                    out.push(path);
                }
            }
        }
    }

    #[test]
    fn every_ui_field_path_resolves_on_campaign_evolved_vehicles() {
        if !Path::new(*PAKS).exists() {
            eprintln!("skipping: {} not present", *PAKS);
            return;
        }
        let defs = Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
        let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
        let loaded =
            load_iostore_container_set(PathBuf::from(*PAKS), &names, &defs).expect("mount");
        let vehicles: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.display_path.ends_with(".vehicle"))
            .cloned()
            .collect();
        eprintln!("{} vehicle tags", vehicles.len());

        let mut total = 0usize;
        let mut broken: Vec<(String, String)> = Vec::new();
        for entry in vehicles.iter().take(8) {
            let Ok(mut tag) = read_entry(&loaded.source, entry) else {
                continue;
            };
            let mut paths = Vec::new();
            collect(tag.root(), "", &mut paths, 0);
            total += paths.len();
            for path in &paths {
                let mut root = tag.root_mut();
                if root.field_path_mut(path).is_none() {
                    broken.push((entry.display_path.clone(), path.clone()));
                }
            }
        }
        eprintln!("checked {total} paths; UNRESOLVABLE {}", broken.len());
        for (tagname, p) in broken.iter().take(30) {
            eprintln!("  {tagname}  ->  {p}");
        }
        assert!(broken.is_empty(), "{} unresolvable paths", broken.len());
    }
}
