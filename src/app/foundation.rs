use super::*;

/// Remove `[index]` segments from a rendered field path so it can be matched
/// against the index-independent paths in a [`FieldFilter`].
/// e.g. `"contact points[0]/markers[2]"` → `"contact points/markers"`.
pub(super) fn strip_node_indices(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_bracket = false;
    for ch in path.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if !in_bracket => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Whether a tag offers the "Search fields" box. Shader/material tags use the
/// dedicated grid surface and sound tags have no meaningful block tree, so they
/// are excluded.
pub(super) fn supports_field_search(entry: &TagEntry) -> bool {
    !(is_material_tag(entry)
        || is_material_shader_tag(entry)
        || is_shader_tag(entry)
        || &entry.group_tag.to_be_bytes() == b"snd!")
}

/// Resolve the field-filter action to apply *this* frame. Returns `Some` only
/// on the frame the (trimmed, lowercased) query changes, so the collapse is a
/// one-shot the user can then adjust by hand. Clearing a previously-applied
/// query yields one `RestoreDefaults` pass that re-expands the editor.
pub(super) fn compute_pending_field_filter(
    tag: &TagFile,
    supports: bool,
    passive: bool,
    tag_key: &str,
    field_search: &HashMap<String, String>,
    field_search_applied: &mut HashMap<String, String>,
) -> Option<FieldFilterAction> {
    if !supports {
        return None;
    }
    let query = field_search
        .get(tag_key)
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if query.is_empty() {
        // Re-expand to defaults once, but only if a search was actually active.
        return field_search_applied
            .remove(tag_key)
            .map(|_| FieldFilterAction::RestoreDefaults);
    }
    if passive {
        // Passive mode highlights + keeps matches open every frame (it never
        // collapses), so re-apply continuously rather than one-shot.
        field_search_applied.insert(tag_key.to_owned(), query.clone());
        return Some(FieldFilterAction::Apply(compute_field_filter(
            tag, &query, true,
        )));
    }
    if field_search_applied.get(tag_key).map(String::as_str) == Some(query.as_str()) {
        // Already applied; leave the user's manual expand/collapse intact.
        return None;
    }
    field_search_applied.insert(tag_key.to_owned(), query.clone());
    Some(FieldFilterAction::Apply(compute_field_filter(
        tag, &query, false,
    )))
}

/// Build the set of collapsible nodes to open for a "Search fields" query:
/// every struct / block / array whose (display) name contains `query`, plus
/// all of their ancestor nodes, plus the ancestors of any matching leaf field.
/// `query` must already be lowercased and non-empty.
pub(super) fn compute_field_filter(tag: &TagFile, query: &str, passive: bool) -> FieldFilter {
    let mut open_paths = std::collections::HashSet::new();
    let mut highlight_paths = std::collections::HashSet::new();
    collect_open_paths(tag.root(), "", query, &mut open_paths, &mut highlight_paths);
    FieldFilter {
        open_paths,
        highlight_paths,
        passive,
    }
}

/// Returns whether `tag_struct` (or anything beneath it) matched, so the
/// caller can mark the containing node open. Records every node on a match path
/// in `open_paths`, and every field (leaf or node) whose own name matched in
/// `highlight_paths`.
fn collect_open_paths(
    tag_struct: TagStruct<'_>,
    canon_prefix: &str,
    query: &str,
    open_paths: &mut std::collections::HashSet<String>,
    highlight_paths: &mut std::collections::HashSet<String>,
) -> bool {
    let mut any = false;
    for field in tag_struct.fields() {
        let name_matches = clean_field_name(field.name())
            .to_ascii_lowercase()
            .contains(query);
        // Canonical path = raw field names joined by '/', no element indices.
        let canon = if canon_prefix.is_empty() {
            field.name().to_owned()
        } else {
            format!("{canon_prefix}/{}", field.name())
        };

        if name_matches {
            highlight_paths.insert(canon.clone());
        }

        let child_matched = if let Some(nested) = field.as_struct() {
            collect_open_paths(nested, &canon, query, open_paths, highlight_paths)
        } else if let Some(block) = field.as_block() {
            block
                .element(0)
                .map(|el| collect_open_paths(el, &canon, query, open_paths, highlight_paths))
                .unwrap_or(false)
        } else if let Some(array) = field.as_array() {
            array
                .element(0)
                .map(|el| collect_open_paths(el, &canon, query, open_paths, highlight_paths))
                .unwrap_or(false)
        } else {
            // Leaf field: a name match opens its ancestors but adds no node.
            false
        };

        let is_node =
            field.as_struct().is_some() || field.as_block().is_some() || field.as_array().is_some();
        if is_node && (name_matches || child_matched) {
            open_paths.insert(canon);
        }
        any |= name_matches || child_matched;
    }
    any
}

pub(super) fn draw_struct_fields(
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
        ("struct", path_prefix, depth),
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

pub(super) fn draw_inherited_object_fields(
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
            ("inherited_struct", path_prefix.as_str()),
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

pub(super) fn inherited_struct_chain(tag_struct: TagStruct<'_>) -> Vec<(TagStruct<'_>, String)> {
    let mut chain = vec![(tag_struct, String::new())];
    let mut current = tag_struct;
    let mut path_prefix = String::new();
    while let Some(parent_field) = inherited_parent_field(current) {
        let Some(parent_struct) = parent_field.as_struct() else {
            break;
        };
        path_prefix = append_field_path(&path_prefix, parent_field.name());
        chain.push((parent_struct, path_prefix.clone()));
        current = parent_struct;
    }
    chain
}

pub(super) fn inherited_parent_field_name(tag_struct: TagStruct<'_>) -> Option<&str> {
    inherited_parent_field(tag_struct).map(|field| field.name())
}

pub(super) fn inherited_parent_field(tag_struct: TagStruct<'_>) -> Option<TagField<'_>> {
    tag_struct
        .fields()
        .find(|field| field.as_struct().is_some() && is_inherited_parent_name(field.name()))
}

pub(super) fn is_inherited_parent_name(name: &str) -> bool {
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
pub(super) fn draw_fields_with_docs(
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
                        draw_foundation_explanation_row(
                            ui,
                            title,
                            Some(body),
                            depth,
                            (path_prefix, cursor + offset),
                        );
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
        );
    }
    // Any explanations after the last matched field.
    for (offset, entry) in entries[cursor..].iter().enumerate() {
        if let DefEntry::Explanation { title, body } = entry {
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

pub(super) fn draw_field(
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
) {
    let field_path = append_field_path(path_prefix, field.name());
    // `meta_override` carries help/units recovered from the JSON definition
    // (shipped tags strip them); fall back to parsing the tag's own field name.
    let meta = meta_override.unwrap_or_else(|| field_display_meta(field.name()));
    if meta.advanced && !expert_mode {
        return;
    }
    if is_internal_schema_marker_name(field.name()) {
        return;
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
    // Passive field-search: tint rows whose name matched the query.
    let highlight = edit.field_highlighted(&field_path);
    let highlight_fill = egui::Color32::from_rgba_unmultiplied(255, 214, 0, 38);
    if let Some(function) = field.as_function() {
        if highlight {
            egui::Frame::none().fill(highlight_fill).show(ui, |ui| {
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
        if highlight {
            egui::Frame::none().fill(highlight_fill).show(ui, |ui| {
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
                );
            });
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
        let nested_default_open = depth == 0 || is_priority_section(field.name());
        let open_override = edit.resolve_open(&field_path, nested_default_open);
        draw_foundation_group(
            ui,
            visible_container_title(field.name(), path_prefix),
            ("field_struct", &field_path),
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

pub(super) fn draw_struct_fields_inline(
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

pub(super) fn draw_foundation_explanation_row(
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
        ui.visuals_mut().collapsing_header_frame = true;
        let response = egui::CollapsingHeader::new(
            RichText::new(header)
                .color(text_dark())
                .font(bold_font(12.5)),
        )
        .id_salt(("foundation_explanation", id_salt))
        .default_open(true)
        .show_background(true)
        .show(ui, |ui| {
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
        });
        if response.fully_open() {
            ui.add_space(3.0);
        }
    });
}

fn known_explanation_text(name: &str) -> Option<String> {
    (clean_field_key(name) == "screen flash").then(|| {
        "There are seven screen flash types:\n\nNONE: DST'= DST\nLIGHTEN: DST'= DST(1 - A) + C\nDARKEN: DST'= DST(1 - A) - C\nMAX: DST'= MAX[DST(1 - C), (C - A)(1-DST)]\nMIN: DST'= MIN[DST(1 - C), (C + A)(1-DST)]\nTINT: DST'= DST(1 - C) + (A*PIN[2C - 1, 0, 1] + A)(1-DST)\nINVERT: DST'= DST(1 - C) + A)\n\nIn the above equations C and A represent the color and alpha of the screen flash, DST represents the color in the framebuffer before the screen flash is applied, and DST' represents the color after the screen flash is applied.".to_owned()
    })
}

fn visible_container_title(name: &str, path_prefix: &str) -> String {
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

fn inline_function_label(name: &str, path_prefix: &str) -> String {
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

fn is_internal_schema_marker_name(name: &str) -> bool {
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
    if let Some(bytes) = halo2_function_bytes_from_struct(tag_struct) {
        if !bytes.is_empty() {
            let data_path = append_field_path(struct_path, "data");
            if let Some(view) = legacy_mapping_function_view_for_path(&bytes, struct_path) {
                return Some((view, data_path));
            }
            if let Ok(function) = TagFunction::parse(&bytes) {
                return Some((FunctionView::from_function(function), data_path));
            }
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
            continue;
        }
        if let Some(view) = legacy_mapping_function_view(&bytes) {
            return Some((view, data_path));
        }
    }
    None
}

fn is_vibration_function_path(path: &str) -> bool {
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

fn legacy_mapping_function_view(bytes: &[u8]) -> Option<FunctionView> {
    let h2_legacy = H2LegacyFunctionView::parse(bytes.to_vec())?;
    let function = decode_hex(&constant_function_hex(0.0))
        .ok()
        .and_then(|data| TagFunction::parse(&data).ok())?;
    Some(FunctionView::from_function(function).with_h2_legacy(h2_legacy))
}

pub(super) fn draw_foundation_group(
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
        // Make the header bar span the full container width (egui only fills
        // width when this is set), matching Foundation's full-width header.
        ui.visuals_mut().collapsing_header_frame = true;
        let mut header = egui::CollapsingHeader::new(
            RichText::new(title)
                .color(text_dark())
                .font(bold_font(12.5)),
        )
        .id_salt(id_salt)
        .show_background(true);
        header = match open_override {
            // `open` and `default_open` are mutually exclusive in egui.
            Some(_) => header.open(open_override),
            None => header.default_open(default_open),
        };
        let response = header.show(ui, |ui| {
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
        });
        if response.fully_open() {
            ui.add_space(3.0);
        }
    });
}

pub(super) fn draw_foundation_block(
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
    // A clipboard is compatible when it came from the same group + block path.
    let clipboard_len = edit
        .block_clipboard
        .filter(|clip| {
            edit.editable && clip.group_tag == edit.group_tag && clip.block_path == path_prefix
        })
        .map(|clip| clip.elements.len());
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
        clipboard_len,
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

pub(super) fn format_block_size_label(count: usize, element_size: usize) -> String {
    let total = count.saturating_mul(element_size);
    format!(
        "{} x {} B = {}",
        count,
        element_size,
        format_byte_count(total)
    )
}

pub(super) fn format_byte_count(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f32 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f32 / (1024.0 * 1024.0))
    }
}

/// Translate header button clicks into block selection changes / deferred ops.
pub(super) fn handle_block_actions(
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

pub(super) fn block_element_dropdown_label(
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

pub(super) fn block_element_content_label(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    first_named_string_label(element)
        .or_else(|| first_tag_reference_label(element, names))
        .or_else(|| first_string_label(element))
        .or_else(|| first_scalar_label(element, names))
}

pub(super) fn first_tag_reference_label(
    element: TagStruct<'_>,
    names: &TagNameIndex,
) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        if let Some(value) = field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            if let TagFieldData::TagReference(reference) = value {
                if reference.group_tag_and_name.is_some() {
                    let label = format_foundation_scalar_value(
                        names,
                        &TagFieldData::TagReference(reference),
                    );
                    if !label.trim().is_empty() && label != "NONE" {
                        return Some(label);
                    }
                }
            }
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_tag_reference_label(nested, names) {
                return Some(label);
            }
        }
    }
    None
}

pub(super) fn first_named_string_label(element: TagStruct<'_>) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        if let Some(value) = field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            if is_name_like_field(field.name()) {
                if let Some(label) = stringish_label(&value) {
                    return Some(label);
                }
            }
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_named_string_label(nested) {
                return Some(label);
            }
        }
    }
    None
}

pub(super) fn first_string_label(element: TagStruct<'_>) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        if let Some(value) = field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            if let Some(label) = stringish_label(&value) {
                return Some(label);
            }
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_string_label(nested) {
                return Some(label);
            }
        }
    }
    None
}

pub(super) fn first_scalar_label(element: TagStruct<'_>, names: &TagNameIndex) -> Option<String> {
    let parent_raw = element.raw();
    for field in element.fields() {
        if let Some(value) = field_value_with_legacy_inline_old_string_id(field, parent_raw) {
            if scalar_is_useful_for_block_label(&value) {
                let value = format_foundation_scalar_value(names, &value);
                if label_has_content(&value) {
                    return Some(format!("{}: {value}", clean_field_name(field.name())));
                }
            }
        }
        if let Some(nested) = field.as_struct() {
            if let Some(label) = first_scalar_label(nested, names) {
                return Some(label);
            }
        }
    }
    None
}

pub(super) fn field_value_with_legacy_inline_old_string_id(
    field: TagField<'_>,
    parent_raw: &[u8],
) -> Option<TagFieldData> {
    if let Some(value) = field.value() {
        return Some(value);
    }
    legacy_inline_old_string_id(field, parent_raw)
        .map(|string| TagFieldData::OldStringId(StringIdData { string }))
}

pub(super) fn legacy_inline_old_string_id(
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

pub(super) fn is_name_like_field(name: &str) -> bool {
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

pub(super) fn stringish_label(value: &TagFieldData) -> Option<String> {
    let raw = match value {
        TagFieldData::String(text) | TagFieldData::LongString(text) => text.as_str(),
        TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => id.string.as_str(),
        _ => return None,
    };
    let label = trim_formatted_value(raw);
    label_has_content(&label).then_some(label)
}

pub(super) fn scalar_is_useful_for_block_label(value: &TagFieldData) -> bool {
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

pub(super) fn label_has_content(label: &str) -> bool {
    let trimmed = label.trim();
    !trimmed.is_empty() && trimmed != "NONE"
}

pub(super) fn draw_foundation_array(
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
    // A clipboard is compatible when it came from this same array path.
    let clipboard_len = edit
        .block_clipboard
        .filter(|clip| {
            edit.editable && clip.group_tag == edit.group_tag && clip.block_path == path_prefix
        })
        .map(|clip| clip.elements.len());
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
        clipboard_len,
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
fn parent_block_path(path: &str) -> Option<String> {
    let cut = path.rfind('/')?;
    let mut parent = path[..cut].to_string();
    if parent.ends_with(']') {
        if let Some(open) = parent.rfind('[') {
            parent.truncate(open);
        }
    }
    Some(parent)
}

/// A readable breadcrumb for a block path: cleaned segments (index suffixes
/// dropped) joined with ` › `, e.g. `regions[0]/permutations` → `regions › permutations`.
fn breadcrumb_for_path(path: &str) -> String {
    path.split('/')
        .map(|segment| clean_field_name(segment.split('[').next().unwrap_or(segment)))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" › ")
}

/// egui-memory key holding the block path that a pending "jump to parent" should
/// scroll into view on the next frame.
fn jump_target_id() -> egui::Id {
    egui::Id::new("foundation_jump_to_block")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_foundation_block_control(
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
    // `Some(n)` when a compatible clipboard holds `n` element(s) (enables the
    // paste / replace menu items); `None` disables them.
    clipboard_len: Option<usize>,
    block_size_label: Option<&str>,
    element_label: impl Fn(usize) -> String,
    add_contents: impl FnOnce(&mut Ui),
) -> BlockHeaderActions {
    let mut actions = BlockHeaderActions::default();
    let id = ui.make_persistent_id((
        "foundation_block_control",
        view_scope,
        tag_key,
        path_salt,
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
    let row_height = 26.0;
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::hover());
    ui.painter()
        .rect_filled(row_rect, 0.0, foundation_block_bar());

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
    let can_edit = editable && allow_structural;
    let has_sel = count > 0;
    // `max_count` of 0 in the schema means "unbounded".
    let capacity = max_count.filter(|&m| m != 0).map(|m| m as usize);
    let at_capacity = capacity.is_some_and(|m| count >= m);
    let capacity_hint = capacity
        .filter(|_| at_capacity)
        .map(|m| format!("Block is at its schema maximum of {m} element(s)"));
    let mut selector_active = false;

    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(row_rect.shrink2(Vec2::new(4.0, 3.0))),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
            ui.horizontal_centered(|ui| {
                ui.add_space(depth as f32 * 5.0);
                // Jump-to-parent (nested blocks only); hover shows the breadcrumb.
                if depth > 0 {
                    let jump = ui
                        .add(egui::Button::new(
                            RichText::new("↑").color(foundation_block_text()),
                        ))
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
                let name_label = ui.add_sized(
                    [190.0, 20.0],
                    egui::Label::new(
                        RichText::new(clean_field_name(name))
                            .color(foundation_block_text())
                            .font(bold_font(12.5)),
                    )
                    .sense(Sense::click()),
                );
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
                        if clipboard_len.is_some()
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
                            match clipboard_len {
                                Some(n) => {
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
                                None => {
                                    ui.add_enabled(false, egui::Button::new("Paste"));
                                }
                            }
                        }
                    });
                foundation_header_icon_cell(ui, "[]");

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

                // Prev / next steppers.
                if foundation_header_button_clicked(ui, "<", has_sel && selected_index > 0) {
                    actions.new_selection = Some(selected_index.saturating_sub(1));
                }
                if foundation_header_button_clicked(ui, ">", has_sel && selected_index + 1 < count)
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
            .show(ui, |ui| {
                if depth > 0 {
                    egui::ScrollArea::vertical()
                        .id_salt((
                            "foundation_block_body",
                            view_scope,
                            tag_key,
                            path_salt,
                            depth,
                        ))
                        .max_height(420.0)
                        .min_scrolled_height(160.0)
                        .auto_shrink([false, false])
                        .show(ui, add_contents);
                } else {
                    add_contents(ui);
                }
            });
    });

    actions
}

pub(super) fn consume_mouse_wheel(ui: &Ui) {
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

pub(super) fn set_combo_scroll_cycle_enabled(ctx: &egui::Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(combo_scroll_cycle_enabled_id(), enabled));
}

fn combo_scroll_cycle_enabled(ui: &Ui) -> bool {
    ui.data(|data| {
        data.get_temp::<bool>(combo_scroll_cycle_enabled_id())
            .unwrap_or(true)
    })
}

pub(super) fn combo_box_with_scroll<R>(
    ui: &mut Ui,
    combo: egui::ComboBox,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> (egui::InnerResponse<Option<R>>, Option<i32>) {
    let response = combo.show_ui(ui, add_contents);
    let popup_open = response.inner.is_some();
    let delta = dropdown_wheel_delta(ui, &response.response, popup_open);
    (response, delta)
}

pub(super) fn combo_scroll_next_index(current: usize, len: usize, delta: i32) -> Option<usize> {
    if len == 0 || delta == 0 {
        return None;
    }
    let delta = delta.signum();
    let current = current.min(len - 1);
    let next = (current as i32 + delta).clamp(0, len as i32 - 1) as usize;
    (next != current).then_some(next)
}

pub(super) fn combo_scroll_next_i64(current: i64, min: i64, max: i64, delta: i32) -> Option<i64> {
    if min > max || delta == 0 {
        return None;
    }
    let delta = delta.signum() as i64;
    let current = current.clamp(min, max);
    let next = (current + delta).clamp(min, max);
    (next != current).then_some(next)
}

pub(super) fn dropdown_wheel_delta(
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

pub(super) fn foundation_header_toggle_cell(
    ui: &mut Ui,
    open: bool,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::click());
    let fill = if enabled {
        foundation_input()
    } else {
        Color32::from_rgb(222, 222, 220)
    };
    ui.painter().rect_filled(rect, 1.0, fill);
    ui.painter()
        .rect_stroke(rect, 1.0, Stroke::new(1.0, foundation_input_edge()));
    let glyph = if open { "-" } else { "+" };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(18.0),
        if enabled { text_dark() } else { subtle_dark() },
    );
    response
}

pub(super) fn foundation_selected_width(row_width: f32) -> f32 {
    (row_width - 190.0 - 22.0 * 4.0 - 54.0 * 5.0 - 92.0).clamp(120.0, 420.0)
}

pub(super) fn foundation_header_icon_cell(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 1.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 1.0, Stroke::new(1.0, foundation_input_edge()));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.0),
        subtle_dark(),
    );
}

pub(super) fn foundation_header_value_cell(ui: &mut Ui, text: &str, max_width: f32) {
    let width = ui.available_width().min(max_width).max(180.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
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
pub(super) fn foundation_header_button_clicked(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    foundation_header_button_clicked_hint(ui, label, enabled, None)
}

/// Like [`foundation_header_button_clicked`] but shows `disabled_hint` as a
/// hover tooltip while the button is disabled (e.g. block at capacity).
pub(super) fn foundation_header_button_clicked_hint(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    disabled_hint: Option<&str>,
) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(text_dark())).min_size(Vec2::new(54.0, 20.0)),
    );
    match disabled_hint {
        Some(hint) if !enabled => response.on_disabled_hover_text(hint).clicked(),
        _ => response.clicked(),
    }
}

// ── Block element selection (persisted in egui memory, keyed by block path) ──

pub(super) fn block_selected_index(
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

pub(super) fn set_block_selected_index(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    path: &str,
    idx: usize,
) {
    let id = edit.widget_id(("block_sel", path));
    ui.data_mut(|d| d.insert_temp(id, idx));
}

pub(super) fn draw_foundation_bar(
    ui: &mut Ui,
    title: String,
    depth: usize,
    default_open: bool,
    add_contents: impl FnOnce(&mut Ui),
) {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = foundation_section_bar();
        ui.visuals_mut().widgets.hovered.bg_fill = Color32::from_rgb(205, 205, 201);
        ui.visuals_mut().widgets.active.bg_fill = Color32::from_rgb(196, 196, 192);
        let response = egui::CollapsingHeader::new(
            RichText::new(title)
                .color(text_dark())
                .font(bold_font(12.5)),
        )
        .default_open(default_open)
        .show_background(true)
        .show(ui, |ui| {
            Frame::none()
                .fill(foundation_group_bg())
                .inner_margin(egui::Margin {
                    left: 8.0 + depth as f32 * 6.0,
                    right: 6.0,
                    top: 5.0,
                    bottom: 5.0,
                })
                .show(ui, add_contents);
        });
        if response.fully_open() {
            ui.add_space(2.0);
        }
    });
}

/// The signed index held by any block-index value variant.
fn block_index_value(value: &TagFieldData) -> Option<i64> {
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
pub(super) fn block_index_target_options(
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
pub(super) fn semantic_short_index_target_options(
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

fn semantic_short_index_target_key(field_name: &str) -> Option<&'static str> {
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
pub(super) fn draw_foundation_block_index_row(
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

pub(super) fn draw_foundation_value_row(
    ui: &mut Ui,
    field: TagField<'_>,
    meta: &FieldDisplayMeta,
    type_name: &str,
    value: &TagFieldData,
    names: &TagNameIndex,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    // Resolved (element labels, target block field name) for a block-index field
    // whose target block was found among the struct's siblings; `None` for
    // non-block-index fields and unresolvable (custom) indices → numeric editor.
    block_index: Option<&(Vec<String>, String)>,
    semantic_short_index: Option<&(Vec<String>, String)>,
) {
    if let (Some((labels, target_path)), Some(index)) = (block_index, block_index_value(value)) {
        draw_foundation_block_index_row(ui, meta, index, labels, target_path, depth, path, edit);
        return;
    }
    if let (TagFieldData::ShortInteger(index), Some((labels, target_path))) =
        (value, semantic_short_index)
    {
        draw_foundation_block_index_row(
            ui,
            meta,
            *index as i64,
            labels,
            target_path,
            depth,
            path,
            edit,
        );
        return;
    }
    if let TagFieldData::TagReference(reference) = value {
        let formatted = format_foundation_scalar_value(names, value);
        // The on-disk tag-ref path is null-terminated; strip the trailing NUL
        // so it resolves on disk (and tool-import paths are clean).
        let target = reference
            .group_tag_and_name
            .as_ref()
            .map(|(g, p)| (*g, sanitize_ref_path(p)))
            .filter(|(_, p)| !p.is_empty());
        let import_verb = target
            .as_ref()
            .and_then(|(group, _)| geometry_import_verb(names, *group));
        draw_foundation_tag_reference_row(
            ui,
            meta,
            &formatted,
            target,
            import_verb,
            depth,
            path,
            edit,
        );
        return;
    }

    if let Some((raw, flag_names)) = flag_value_parts(value) {
        draw_foundation_flags_row(ui, meta, raw, &flag_names, field, depth, path, edit);
        return;
    }

    if let Some(blam_tags::TagOptions::Enum {
        names: options,
        current,
    }) = field.options()
    {
        draw_foundation_enum_row(ui, meta, &options, current, depth, path, edit);
        return;
    }

    if matches!(
        value,
        TagFieldData::RealRgbColor(_)
            | TagFieldData::RealArgbColor(_)
            | TagFieldData::RgbColor(_)
            | TagFieldData::ArgbColor(_)
    ) {
        draw_foundation_color_row(ui, meta, value, depth, path, edit);
        return;
    }

    if let Some((lower, upper)) = foundation_bounds_values(value) {
        draw_foundation_bounds_row(
            ui,
            meta,
            &lower,
            &upper,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    if let Some(parts) = foundation_editable_component_parts(value) {
        draw_foundation_component_edit_row(
            ui,
            meta,
            &parts,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    let formatted = format_foundation_scalar_value(names, value);
    if edit.editable && !meta.read_only && is_text_editable_value(value) {
        draw_foundation_editable_text_row(
            ui,
            meta,
            &formatted,
            field_suffix(meta, type_name).as_str(),
            depth,
            path,
            edit,
        );
        return;
    }

    if let Some(parts) = foundation_value_parts(value) {
        draw_foundation_multi_value_row(
            ui,
            meta,
            &parts,
            field_suffix(meta, type_name).as_str(),
            depth,
        );
        return;
    }

    draw_foundation_meta_text_row(
        ui,
        meta,
        &formatted,
        field_suffix(meta, type_name).as_str(),
        depth,
    );
}

/// A color value row: channel readouts plus a clickable swatch that opens the
/// color picker. ARGB rows show all four components in a/r/g/b order.
pub(super) fn draw_foundation_color_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &TagFieldData,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    // (alpha, red, green, blue, is_argb). RGB rows pin alpha to 1.0.
    let (a, r, g, b, argb) = match value {
        TagFieldData::RealRgbColor(c) => (1.0, c.red, c.green, c.blue, false),
        TagFieldData::RealArgbColor(c) => (c.alpha, c.red, c.green, c.blue, true),
        TagFieldData::RgbColor(c) => {
            let raw = c.0;
            (
                1.0,
                ((raw >> 16) & 0xFF) as f32 / 255.0,
                ((raw >> 8) & 0xFF) as f32 / 255.0,
                (raw & 0xFF) as f32 / 255.0,
                false,
            )
        }
        TagFieldData::ArgbColor(c) => {
            let raw = c.0;
            (
                ((raw >> 24) & 0xFF) as f32 / 255.0,
                ((raw >> 16) & 0xFF) as f32 / 255.0,
                ((raw >> 8) & 0xFF) as f32 / 255.0,
                (raw & 0xFF) as f32 / 255.0,
                true,
            )
        }
        _ => return,
    };
    let channels: &[(&str, f32)] = if argb {
        &[("a", a), ("r", r), ("g", g), ("b", b)]
    } else {
        &[("r", r), ("g", g), ("b", b)]
    };
    let swatch = Color32::from_rgb(
        float_channel_to_u8(r),
        float_channel_to_u8(g),
        float_channel_to_u8(b),
    );
    let editable = edit.editable && !meta.read_only;

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        for (label, channel) in channels {
            ui.label(RichText::new(*label).color(subtle_dark()).small());
            foundation_input_cell(ui, &format_pc_float(*channel), 76.0);
        }

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        ui.painter().rect_filled(rect, 2.0, swatch);
        ui.painter()
            .rect_stroke(rect, 2.0, Stroke::new(1.0, foundation_input_edge()));
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if editable {
                "Click to edit color"
            } else {
                "Click to inspect color"
            });
        if response.clicked() {
            let mut popup = MaterialColorPopup::new(&meta.label, r, g, b, a);
            if editable {
                popup = popup.with_color_field(edit.tag_key, path, argb);
            }
            *edit.color_request = Some(popup);
        }
        draw_field_help(ui, meta);
    });
}

pub(super) fn draw_foundation_multi_value_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        for (label, value) in parts {
            if !label.is_empty() {
                ui.label(RichText::new(label).color(subtle_dark()).small());
            }
            foundation_input_cell(ui, value, 92.0);
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

pub(super) fn draw_foundation_bounds_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    lower_value: &str,
    upper_value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let lower_key = format!("{buffer_key}|lower");
    let upper_key = format!("{buffer_key}|upper");
    let lower_id = edit.widget_id(("bounds_lower", &buffer_key));
    let upper_id = edit.widget_id(("bounds_upper", &buffer_key));
    let lower_has_focus = ui.memory(|memory| memory.has_focus(lower_id));
    let upper_has_focus = ui.memory(|memory| memory.has_focus(upper_id));
    let mut lower = edit
        .buffers
        .remove(&lower_key)
        .unwrap_or_else(|| lower_value.to_owned());
    let mut upper = edit
        .buffers
        .remove(&upper_key)
        .unwrap_or_else(|| upper_value.to_owned());
    if !lower_has_focus && !upper_has_focus {
        if lower != lower_value {
            lower = lower_value.to_owned();
        }
        if upper != upper_value {
            upper = upper_value.to_owned();
        }
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        if editable {
            let lower_response = foundation_text_edit_cell(ui, &mut lower, 92.0, lower_id);
            ui.label(RichText::new("to").color(subtle_dark()).small());
            let upper_response = foundation_text_edit_cell(ui, &mut upper, 92.0, upper_id);
            let commit = (lower_response.lost_focus() || upper_response.lost_focus())
                && (lower.trim() != lower_value.trim() || upper.trim() != upper_value.trim());
            if commit {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: format!("{}..{}", lower.trim(), upper.trim()),
                });
            }
        } else {
            foundation_input_cell(ui, lower_value, 92.0);
            ui.label(RichText::new("to").color(subtle_dark()).small());
            foundation_input_cell(ui, upper_value, 92.0);
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });

    edit.buffers.insert(lower_key, lower);
    edit.buffers.insert(upper_key, upper);
}

pub(super) fn draw_foundation_component_edit_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let mut values = Vec::with_capacity(parts.len());
    let mut responses = Vec::with_capacity(parts.len());
    let ids = parts
        .iter()
        .map(|(label, _)| edit.widget_id(("component", &buffer_key, label)))
        .collect::<Vec<_>>();
    let any_focus = ids
        .iter()
        .any(|id| ui.memory(|memory| memory.has_focus(*id)));
    for (label, value) in parts {
        let key = format!("{buffer_key}|component|{label}");
        let mut buffer = edit.buffers.remove(&key).unwrap_or_else(|| value.clone());
        if !any_focus && buffer != *value {
            buffer = value.clone();
        }
        values.push((label.clone(), value.clone(), key, buffer));
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        for (index, (label, _, _, buffer)) in values.iter_mut().enumerate() {
            if !label.is_empty() {
                ui.label(RichText::new(label.as_str()).color(subtle_dark()).small());
            }
            if editable {
                responses.push(foundation_text_edit_cell(ui, buffer, 92.0, ids[index]));
            } else {
                foundation_input_cell(ui, buffer, 92.0);
            }
        }
        if editable {
            let changed = values
                .iter()
                .any(|(_, value, _, buffer)| buffer.trim() != value.trim());
            let committed = responses.iter().any(egui::Response::lost_focus);
            if committed && changed {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: values
                        .iter()
                        .map(|(_, _, _, buffer)| buffer.trim())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });

    for (_, _, key, buffer) in values {
        edit.buffers.insert(key, buffer);
    }
}

pub(super) fn draw_foundation_text_row(
    ui: &mut Ui,
    name: &str,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    let meta = field_display_meta(name);
    draw_foundation_meta_text_row(ui, &meta, value, suffix, depth);
}

pub(super) fn draw_foundation_meta_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    let indent = depth as f32 * 12.0;
    let suffix_reserve = if suffix.is_empty() { 0.0 } else { 96.0 };
    let available_value_width =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0)
            .clamp(180.0, 920.0);
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        foundation_input_cell(
            ui,
            value,
            foundation_value_width(value, available_value_width),
        );
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

pub(super) fn draw_foundation_editable_text_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    let suffix_reserve = if suffix.is_empty() { 0.0 } else { 96.0 };
    let available_value_width =
        (ui.available_width() - indent - FOUNDATION_LABEL_WIDTH - suffix_reserve - 28.0)
            .clamp(180.0, 920.0);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("text", &buffer_key));
    let buffer = edit
        .buffers
        .entry(buffer_key.clone())
        .or_insert_with(|| value.to_owned());
    if !ui.memory(|memory| memory.has_focus(id)) && buffer != value {
        *buffer = value.to_owned();
    }

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let width = foundation_value_width(buffer, available_value_width);
        let response = foundation_text_edit_cell(ui, buffer, width, id);
        let commit = response.lost_focus() && buffer.trim() != value.trim();
        if commit {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: buffer.trim().to_owned(),
            });
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

/// Red used to flag tag references whose target file is missing on disk.
pub(super) const REFERENCE_MISSING_COLOR: Color32 = Color32::from_rgb(216, 92, 92);

/// Resolve a tag reference to its on-disk path (loose-folder root) and report
/// whether the file is **absent**. Returns `false` (i.e. "not known missing")
/// when there is no loose-folder root or the group's extension is unknown, so
/// non-folder sources never show false "missing" reds.
///
/// NOTE: this stats the filesystem per call. Reference rows are bounded by the
/// visible fields of one tag, so the per-frame cost is small; R3 will replace
/// this with a generation-invalidated cache shared with the bitmap-row checks.
pub(super) fn reference_target_missing(
    tags_root: Option<&Path>,
    group_tag: u32,
    rel_path: &str,
) -> bool {
    let Some(root) = tags_root else {
        return false;
    };
    let Some(ext) = blam_tags::paths::group_tag_to_extension(group_tag) else {
        return false;
    };
    let mut rel = rel_path.replace('/', "\\");
    if !ext.is_empty() {
        if let Some(stripped) = rel
            .strip_suffix(&format!(".{ext}"))
            .or_else(|| rel.strip_suffix(&format!(".{}", ext.to_ascii_uppercase())))
        {
            rel = stripped.to_owned();
        }
    }
    if rel.trim().is_empty() {
        return false;
    }
    !blam_tags::paths::resolve_tag_path(root, &rel, ext).exists()
}

pub(super) fn draw_foundation_tag_reference_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    value: &str,
    target: Option<(u32, String)>,
    // `Some(verb)` for references to a geometry tag (render/collision/physics
    // model or animation graph): shows an Import button that runs `tool <verb>`.
    import_verb: Option<&'static str>,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let suffix = "tag reference";
    let indent = depth as f32 * 12.0;
    let icon_group = tag_reference_icon_group(meta, target.as_ref());
    let icon_label_gap = 3.0;
    let icon_reserve = 16.0 + icon_label_gap;
    let available_value_width =
        (ui.available_width() - indent - icon_reserve - FOUNDATION_LABEL_WIDTH - 260.0)
            .clamp(220.0, 760.0);
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let id = edit.widget_id(("tag_ref", &buffer_key));
    let buffer = edit
        .buffers
        .entry(buffer_key.clone())
        .or_insert_with(|| value.to_owned());
    if !ui.memory(|memory| memory.has_focus(id)) && buffer != value {
        *buffer = value.to_owned();
    }

    let droppable = edit.editable && !meta.read_only;
    let row_response = ui
        .horizontal(|ui| {
            ui.add_space(indent);
            let previous_spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = icon_label_gap;
            match icon_group {
                Some(group_tag) => draw_tag_icon(ui, group_tag, 16.0),
                None => draw_default_tag_icon(ui, 16.0),
            }
            foundation_label_cell_with_empty_help_gutter(
                ui,
                &meta.label,
                meta.help.as_deref(),
                false,
            );
            ui.spacing_mut().item_spacing.x = previous_spacing;
            let editable = edit.editable && !meta.read_only;
            let has_ref = target.is_some();
            // A non-empty reference whose target file is absent on disk.
            let missing = target
                .as_ref()
                .is_some_and(|(group, rel)| reference_target_missing(edit.tags_root, *group, rel));
            if editable {
                let response = foundation_text_edit_cell(
                    ui,
                    buffer,
                    foundation_value_width(buffer, available_value_width),
                    id,
                );
                if response.lost_focus() && buffer.trim() != value.trim() {
                    let input = buffer.trim().to_owned();
                    if let Some(required_group) = target.as_ref().map(|(group, _)| *group) {
                        match parse_tag_reference(&input) {
                            Ok(parsed) if tag_reference_group_allowed(&parsed, required_group) => {
                                edit.pending.push(PendingFieldEdit {
                                    path: path.to_owned(),
                                    input,
                                });
                            }
                            Ok(_) => {
                                if let Some(status) = edit.status.as_deref_mut() {
                                    *status = format!(
                                        "Reference must be a {} tag",
                                        tag_group_display_name(required_group)
                                    );
                                }
                            }
                            Err(error) => {
                                if let Some(status) = edit.status.as_deref_mut() {
                                    *status = format!("Invalid tag reference: {error}");
                                }
                            }
                        }
                    } else {
                        edit.pending.push(PendingFieldEdit {
                            path: path.to_owned(),
                            input,
                        });
                    }
                }
            } else if !has_ref {
                foundation_input_cell_colored(
                    ui,
                    "(no reference)",
                    foundation_value_width("(no reference)", available_value_width),
                    subtle_dark(),
                    Some("This reference is empty"),
                );
            } else if missing {
                foundation_input_cell_colored(
                    ui,
                    value,
                    foundation_value_width(value, available_value_width),
                    REFERENCE_MISSING_COLOR,
                    Some("Referenced tag not found on disk"),
                );
            } else {
                foundation_input_cell(
                    ui,
                    value,
                    foundation_value_width(value, available_value_width),
                );
            }
            // Flag a broken reference even while the field is being edited.
            if missing {
                ui.label(
                    RichText::new("⚠ missing")
                        .color(REFERENCE_MISSING_COLOR)
                        .small(),
                )
                .on_hover_text("Referenced tag not found on disk");
            }
            let browse_clicked =
                foundation_header_button_clicked(ui, "...", editable && edit.tags_root.is_some());
            // Open: load the referenced tag in a new tab (resolved against the
            // loose-folder tags root). Enabled only when the ref is non-empty.
            if foundation_header_button_clicked(ui, "Open", target.is_some()) {
                if let Some((group_tag, rel_path)) = target.clone() {
                    // Alt-click opens the referenced tag in a floating window.
                    let float = ui.input(|i| i.modifiers.alt);
                    *edit.open_request = Some(OpenTagRequest {
                        group_tag,
                        rel_path,
                        float,
                    });
                }
            }
            // Import: only for geometry references (render/collision/physics model,
            // animation graph). Runs the matching `tool` command in the background.
            if let (Some(verb), Some((_, rel_path))) = (import_verb, target.as_ref()) {
                if foundation_header_button_clicked(ui, "Import", edit.tags_root.is_some()) {
                    *edit.tool_import = Some(ToolImportRequest {
                        verb,
                        source_dir: model_source_dir(rel_path),
                    });
                }
            }
            if browse_clicked {
                if let Some(tags_root) = edit.tags_root {
                    let start_ref = target.as_ref().map(|(_, rel_path)| rel_path.as_str());
                    let required_group = target.as_ref().map(|(group, _)| *group);
                    match choose_tag_reference_input(tags_root, start_ref, required_group) {
                        Ok(Some(input)) => {
                            *buffer = input.clone();
                            edit.pending.push(PendingFieldEdit {
                                path: path.to_owned(),
                                input,
                            });
                        }
                        Ok(None) => {}
                        Err(error) => {
                            if let Some(status) = edit.status.as_deref_mut() {
                                *status = error;
                            }
                        }
                    }
                }
            }
            if ui
                .add_enabled(
                    editable,
                    egui::Button::new(RichText::new("Clear").color(text_dark()))
                        .min_size(Vec2::new(54.0, 20.0)),
                )
                .clicked()
            {
                buffer.clear();
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: "NONE".to_owned(),
                });
            }
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
            draw_field_help(ui, meta);
        })
        .response;

    // Drag-and-drop: drop a tag from the browser onto this row to set the
    // reference. Accept only when the field is editable and the dropped group
    // matches the current target's group (an empty reference accepts any group,
    // mirroring free-form typing).
    if droppable {
        let accepts = |payload: &DraggedTagRef| match &target {
            Some((group, _)) => *group == payload.group_tag,
            None => true,
        };
        if let Some(payload) = row_response.dnd_hover_payload::<DraggedTagRef>() {
            let color = if accepts(&payload) {
                Color32::from_rgb(120, 170, 90)
            } else {
                REFERENCE_MISSING_COLOR
            };
            ui.painter()
                .rect_stroke(row_response.rect, 3.0, Stroke::new(1.5, color));
        }
        if let Some(payload) = row_response.dnd_release_payload::<DraggedTagRef>() {
            if accepts(&payload) {
                *buffer = payload.input.clone();
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: payload.input.clone(),
                });
            }
        }
    }
}

fn tag_reference_icon_group(
    meta: &FieldDisplayMeta,
    target: Option<&(u32, String)>,
) -> Option<u32> {
    match meta.tag_reference_allowed.as_slice() {
        [group] => Some(*group),
        [] => target.map(|(group, _)| *group),
        _ => target.map(|(group, _)| *group),
    }
}

/// Strip the trailing NUL terminator (and surrounding whitespace) from an
/// on-disk tag-reference path so it resolves as a real file path.
pub(super) fn sanitize_ref_path(path: &str) -> String {
    path.replace('\u{0}', "").trim().to_owned()
}

/// The `tool` verb to (re)import the geometry tag a reference points at, or
/// `None` for any other group. Matched on the resolved group name so it's
/// independent of fourcc byte order.
pub(super) fn geometry_import_verb(names: &TagNameIndex, group_tag: u32) -> Option<&'static str> {
    // Prefer the loaded name index, but fall back to the library's built-in
    // group→extension table so the button still appears if definitions failed
    // to load for this source.
    let group_name = names
        .name_for(group_tag)
        .or_else(|| blam_tags::paths::group_tag_to_extension(group_tag))?;
    match group_name {
        "render_model" => Some("render"),
        "collision_model" => Some("collision"),
        "physics_model" => Some("physics"),
        "model_animation_graph" => Some("model-animations-uncompressed"),
        _ => None,
    }
}

/// The `tool` source directory for a geometry tag reference: the parent of the
/// tag path. e.g. `objects\characters\masterchief\masterchief` →
/// `objects\characters\masterchief` (the dir `tool render` expects).
pub(super) fn model_source_dir(rel_path: &str) -> String {
    rel_path
        .rsplit_once('\\')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_else(|| rel_path.to_owned())
}

pub(super) fn tag_reference_start_dir(tags_root: &Path, rel_path: &str) -> PathBuf {
    let cleaned = sanitize_ref_path(rel_path).replace('/', "\\");
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case("NONE") {
        return tags_root.to_path_buf();
    }

    let candidate = tags_root.join(PathBuf::from(cleaned));
    candidate
        .parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tags_root.to_path_buf())
}

pub(super) fn choose_tag_reference_input(
    tags_root: &Path,
    start_ref: Option<&str>,
    required_group: Option<u32>,
) -> Result<Option<String>, String> {
    let start_dir = start_ref
        .map(|rel_path| tag_reference_start_dir(tags_root, rel_path))
        .unwrap_or_else(|| tags_root.to_path_buf());
    let mut dialog = rfd::FileDialog::new()
        .set_title("Select Tag Reference")
        .set_directory(start_dir);
    if let Some(group_tag) = required_group
        && let Some(extension) = blam_tags::paths::group_tag_to_extension(group_tag)
    {
        dialog = dialog.add_filter(extension, &[extension]);
    }
    let picked = dialog.pick_file();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let rel = tag_reference_relative_path(&picked, tags_root)?;
    let extension = rel
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| "Selected tag file has no extension".to_owned())?;
    let group_tag = extension_to_group_tag(extension)
        .ok_or_else(|| format!("Unknown tag extension: {extension}"))?;
    let path = rel.with_extension("").to_string_lossy().replace('/', "\\");
    Ok(Some(format!("{}:{path}", format_group_tag(group_tag))))
}

fn tag_reference_group_allowed(reference: &TagReferenceData, required_group: u32) -> bool {
    reference
        .group_tag_and_name
        .as_ref()
        .is_none_or(|(group, _)| *group == required_group)
}

fn tag_group_display_name(group_tag: u32) -> String {
    blam_tags::paths::group_tag_to_extension(group_tag)
        .map(str::to_owned)
        .unwrap_or_else(|| format_group_tag(group_tag))
}

pub(super) fn tag_reference_relative_path(
    picked: &Path,
    tags_root: &Path,
) -> Result<PathBuf, String> {
    picked
        .strip_prefix(tags_root)
        .map(Path::to_path_buf)
        .map_err(|_| "Selected file must be inside the tags folder".to_owned())
}

pub(super) fn tag_reference_relative_path_with_extension(
    picked: &Path,
    tags_root: &Path,
) -> Result<String, String> {
    let rel = tag_reference_relative_path(picked, tags_root)?;
    if rel.extension().and_then(|ext| ext.to_str()).is_none() {
        return Err("Selected tag file has no extension".to_owned());
    }
    Ok(rel.to_string_lossy().replace('/', "\\"))
}

pub(super) fn draw_foundation_flags_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    raw: u64,
    flag_names: &[(u32, String)],
    field: TagField<'_>,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let options = match field.options() {
        Some(blam_tags::TagOptions::Flags(options)) => options,
        _ => Vec::new(),
    };
    let display_flags = if options.is_empty() {
        flag_names
            .iter()
            .map(|(bit, label)| (*bit, label.clone(), true))
            .collect::<Vec<_>>()
    } else {
        options
            .iter()
            .map(|option| (option.bit, option.name.to_owned(), option.is_set))
            .collect::<Vec<_>>()
    };

    let indent = depth as f32 * 12.0;
    let row_width = ui.available_width().max(620.0);
    let panel_width = (row_width - indent - FOUNDATION_LABEL_WIDTH - 40.0).clamp(360.0, 760.0);
    let flag_row_height = 21.0;
    let panel_height = if display_flags.is_empty() {
        32.0
    } else {
        12.0 + flag_row_height * display_flags.len() as f32 + 24.0
    };
    let total_height = panel_height.max(32.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(row_width, total_height), Sense::hover());
    let painter = ui.painter().clone();

    let label_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(indent + 4.0, 4.0),
        Vec2::new(FOUNDATION_LABEL_WIDTH - 8.0, 24.0),
    );
    painter.text(
        label_rect.left_center(),
        Align2::LEFT_CENTER,
        truncate_for_cell(&meta.label, label_rect.width()),
        FontId::proportional(12.5),
        text_dark(),
    );

    let flags_rect = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(indent + FOUNDATION_LABEL_WIDTH, 0.0),
        Vec2::new(panel_width, panel_height),
    );
    painter.rect_filled(flags_rect, 0.0, foundation_input());
    painter.rect_stroke(flags_rect, 0.0, Stroke::new(1.0, foundation_input_edge()));

    if display_flags.is_empty() {
        painter.text(
            flags_rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            format!("0x{raw:04X} (none set)"),
            FontId::proportional(12.5),
            text_dark(),
        );
    } else {
        let mut next_mask = raw;
        for (index, (bit, label, is_set)) in display_flags.iter().enumerate() {
            let row_top = flags_rect.top() + 6.0 + index as f32 * flag_row_height;
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(flags_rect.left() + 8.0, row_top),
                Vec2::new(flags_rect.width() - 16.0, flag_row_height),
            );
            let checkbox_rect = egui::Rect::from_min_size(
                row_rect.left_top() + Vec2::new(0.0, 3.0),
                Vec2::splat(13.0),
            );
            let enabled = edit.editable && !meta.read_only;
            let response = ui.interact(
                row_rect,
                ui.make_persistent_id((edit.view_scope, edit.tag_key, path, "flag", *bit)),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if response.hovered() {
                painter.rect_filled(row_rect, 0.0, foundation_flag_hover());
                response.clone().on_hover_text(label);
            }

            painter.rect_filled(checkbox_rect, 0.0, foundation_checkbox_bg(enabled));
            painter.rect_stroke(
                checkbox_rect,
                0.0,
                Stroke::new(1.0, foundation_input_edge()),
            );
            if *is_set {
                let stroke = Stroke::new(1.6, text_dark());
                painter.line_segment(
                    [
                        checkbox_rect.left_center() + Vec2::new(3.0, 0.0),
                        checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        checkbox_rect.center() + Vec2::new(-1.0, 3.0),
                        checkbox_rect.right_center() + Vec2::new(-2.0, -4.0),
                    ],
                    stroke,
                );
            }

            painter.text(
                row_rect.left_center() + Vec2::new(20.0, 0.0),
                Align2::LEFT_CENTER,
                truncate_for_cell(label, row_rect.width() - 24.0),
                FontId::proportional(12.5),
                text_dark(),
            );

            if response.clicked() {
                if let Some(bit_mask) = 1u64.checked_shl(*bit) {
                    if *is_set {
                        next_mask &= !bit_mask;
                    } else {
                        next_mask |= bit_mask;
                    }
                    edit.pending.push(PendingFieldEdit {
                        path: path.to_owned(),
                        input: next_mask.to_string(),
                    });
                }
            }
        }

        painter.text(
            flags_rect.left_bottom() + Vec2::new(8.0, -5.0),
            Align2::LEFT_BOTTOM,
            format!("0x{raw:04X}"),
            FontId::proportional(11.5),
            subtle_dark(),
        );
    }

    if meta.help.is_some() || meta.read_only {
        ui.allocate_new_ui(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                flags_rect.right_top() + Vec2::new(8.0, 0.0),
                Vec2::new(120.0, 24.0),
            )),
            |ui| draw_field_help(ui, meta),
        );
    }
    ui.add_space(4.0);
}

pub(super) fn draw_foundation_function_row(
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
        Frame::none()
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0, foundation_group_edge()))
            .inner_margin(egui::Margin::same(6.0))
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
                        let can_edit = edit.editable && !meta.read_only;
                        if ui
                            .add_enabled(
                                can_edit,
                                egui::Button::new("f()").min_size(Vec2::new(30.0, 20.0)),
                            )
                            .on_hover_text(if can_edit {
                                "Open function graph editor"
                            } else {
                                "Function is read-only"
                            })
                            .clicked()
                        {
                            *edit.function_request = Some(FunctionPopup::new(
                                edit.tag_key.to_owned(),
                                clean_field_name(path),
                                FunctionView::from_function(function.clone())
                                    .with_edit(foundation_function_edit_paths(path)),
                                true,
                            ));
                        }
                    });
                    ui.add_space(4.0);
                    ui.push_id(("function", path), |ui| {
                        // Inline preview is always read-only; the editable
                        // editor lives in the f() popup.
                        let mut view = FunctionView::from_function(function.clone());
                        let mut selected = 0usize;
                        draw_function_editor_contents(ui, &mut view, false, &mut selected);
                    });
                });
            });
        draw_field_help(ui, meta);
    });
}

pub(super) fn draw_foundation_inline_function_row(
    ui: &mut Ui,
    label: String,
    mut view: FunctionView,
    depth: usize,
    data_path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    view = view.with_edit(foundation_function_edit_paths(data_path));
    ui.horizontal_top(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &label, None);
        Frame::none()
            .fill(foundation_group_bg())
            .stroke(Stroke::new(1.0, foundation_group_edge()))
            .inner_margin(egui::Margin::same(6.0))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_min_width(640.0);
                    let previous = FunctionSnapshot::from_view(&view);
                    let mut selected = 0usize;
                    let changed = if view.h2_legacy.is_some() {
                        draw_h2_legacy_function_editor_contents(ui, &mut view, edit.editable)
                    } else {
                        draw_function_editor_contents(ui, &mut view, edit.editable, &mut selected)
                    };
                    if changed {
                        let batch = push_function_edit(
                            &foundation_function_edit_paths(data_path),
                            &previous,
                            &view,
                        );
                        edit.pending.extend(batch.edits);
                        edit.function_data_ops.extend(batch.data_ops);
                    }
                });
            });
    });
}

pub(super) fn foundation_function_edit_paths(data_path: &str) -> FunctionEditPaths {
    FunctionEditPaths {
        data: if is_vibration_function_data_path(data_path) {
            FunctionDataStorage::Halo2ByteBlock(data_path.to_owned())
        } else {
            FunctionDataStorage::DataField(data_path.to_owned())
        },
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    }
}

fn is_vibration_function_data_path(path: &str) -> bool {
    is_vibration_function_path(path)
}

/// First-pass editable function types — others stay read-only (graph +
/// controls disabled) but still round-trip on save.
pub(super) fn draw_foundation_enum_row(
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
        ui.add_enabled_ui(edit.editable && !meta.read_only, |ui| {
            let (_, wheel_delta) = combo_box_with_scroll(
                ui,
                egui::ComboBox::from_id_salt((edit.view_scope, edit.tag_key, path, "enum"))
                    .width(240.0)
                    .selected_text(enum_option_label(options, selected)),
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

pub(super) fn foundation_label_cell(ui: &mut Ui, text: &str, help: Option<&str>) {
    foundation_label_cell_with_empty_help_gutter(ui, text, help, true);
}

fn foundation_label_cell_with_empty_help_gutter(
    ui: &mut Ui,
    text: &str,
    help: Option<&str>,
    reserve_empty_help_gutter: bool,
) {
    let width = FOUNDATION_LABEL_WIDTH;
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // Reserve a gutter for the "?" documentation cue. Foundation always reserves
    // the space (the cue is Hidden, not Collapsed, when absent) so field names
    // stay aligned whether or not a field has a doc string.
    let gutter = if help.is_some() || reserve_empty_help_gutter {
        11.0
    } else {
        2.0
    };
    if help.is_some() {
        // The cue: a bold blue "?" left of the name (Foundation uses #3DA1CC).
        ui.painter().text(
            rect.left_center() + Vec2::new(2.0, 0.0),
            Align2::LEFT_CENTER,
            "?",
            bold_font(12.5),
            Color32::from_rgb(61, 161, 204),
        );
    }
    let shown = truncate_for_cell(text, width - gutter - 4.0);
    let truncated = shown != text;
    ui.painter().text(
        rect.left_center() + Vec2::new(gutter, 0.0),
        Align2::LEFT_CENTER,
        shown,
        FontId::proportional(12.5),
        text_dark(),
    );
    // Hovering the name (or the cue) shows the field documentation (prefixed with
    // the full name when the displayed label was truncated).
    let tip = match (help, truncated) {
        (Some(help), true) => Some(format!("{text}\n\n{help}")),
        (Some(help), false) => Some(help.to_owned()),
        (None, true) => Some(text.to_owned()),
        (None, false) => None,
    };
    if let Some(tip) = tip {
        response.on_hover_text(tip);
    }
}

pub(super) fn foundation_input_cell(ui: &mut Ui, text: &str, width: f32) {
    foundation_input_cell_colored(ui, text, width, text_dark(), None);
}

/// Like [`foundation_input_cell`] but with an explicit text color and an
/// optional hover tooltip override (used to flag missing tag references in red).
pub(super) fn foundation_input_cell_colored(
    ui: &mut Ui,
    text: &str,
    width: f32,
    color: Color32,
    hover: Option<&str>,
) {
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter().rect_filled(rect, 0.0, foundation_input());
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
    ui.painter().text(
        rect.left_center() + Vec2::new(5.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(text, width - 10.0),
        FontId::proportional(12.5),
        color,
    );
    if response.hovered() {
        response.on_hover_text(hover.unwrap_or(text));
    }
}

pub(super) fn foundation_text_edit_cell(
    ui: &mut Ui,
    text: &mut String,
    width: f32,
    id: egui::Id,
) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.visuals_mut().widgets.inactive.bg_fill = foundation_input();
            ui.visuals_mut().widgets.hovered.bg_fill = foundation_input();
            ui.visuals_mut().widgets.active.bg_fill = foundation_input();
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, text_dark());
            ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, text_dark());
            ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0, text_dark());
            ui.add_sized(
                [width, 24.0],
                egui::TextEdit::singleline(text)
                    .id(id)
                    .font(TextStyle::Monospace)
                    .text_color(text_dark())
                    .margin(Vec2::new(4.0, 2.0)),
            )
        })
        .inner;
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    response
}

pub(super) fn text_edit_cursor_to_start_on_tab_focus(ui: &Ui, response: &egui::Response) {
    if response.gained_focus() && ui.input(|input| input.key_pressed(egui::Key::Tab)) {
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(0),
                )));
            state.store(ui.ctx(), response.id);
        }
    }
}

pub(super) fn foundation_value_width(value: &str, available: f32) -> f32 {
    if value.len() > 48 {
        available
    } else if value.len() > 18 {
        available.min(520.0).max(300.0)
    } else {
        available.min(180.0).max(140.0)
    }
}

pub(super) fn flag_value_parts(value: &TagFieldData) -> Option<(u64, Vec<(u32, String)>)> {
    match value {
        TagFieldData::ByteFlags { value, names } => Some((*value as u64, names.clone())),
        TagFieldData::WordFlags { value, names } => Some((*value as u64, names.clone())),
        TagFieldData::LongFlags { value, names } => Some((*value as u32 as u64, names.clone())),
        TagFieldData::ByteBlockFlags(value) => Some((*value as u64, Vec::new())),
        TagFieldData::WordBlockFlags(value) => Some((*value as u64, Vec::new())),
        TagFieldData::LongBlockFlags(value) => Some((*value as u32 as u64, Vec::new())),
        _ => None,
    }
}

pub(super) fn foundation_value_parts(value: &TagFieldData) -> Option<Vec<(String, String)>> {
    let pair = |a: &str, av: String, b: &str, bv: String| {
        Some(vec![(a.to_owned(), av), (b.to_owned(), bv)])
    };
    let triple = |a: &str, av: String, b: &str, bv: String, c: &str, cv: String| {
        Some(vec![
            (a.to_owned(), av),
            (b.to_owned(), bv),
            (c.to_owned(), cv),
        ])
    };
    match value {
        TagFieldData::Point2d(p) => pair("x", p.x.to_string(), "y", p.y.to_string()),
        TagFieldData::Rectangle2d(r) => Some(vec![
            ("top".to_owned(), r.top.to_string()),
            ("left".to_owned(), r.left.to_string()),
            ("bottom".to_owned(), r.bottom.to_string()),
            ("right".to_owned(), r.right.to_string()),
        ]),
        TagFieldData::RealPoint2d(p) => pair("x", fmt_real(p.x), "y", fmt_real(p.y)),
        TagFieldData::RealPoint3d(p) => {
            triple("x", fmt_real(p.x), "y", fmt_real(p.y), "z", fmt_real(p.z))
        }
        TagFieldData::RealVector2d(v) => pair("i", fmt_real(v.i), "j", fmt_real(v.j)),
        TagFieldData::RealVector3d(v) => {
            triple("i", fmt_real(v.i), "j", fmt_real(v.j), "k", fmt_real(v.k))
        }
        TagFieldData::RealQuaternion(q) => Some(vec![
            ("i".to_owned(), fmt_real(q.i)),
            ("j".to_owned(), fmt_real(q.j)),
            ("k".to_owned(), fmt_real(q.k)),
            ("w".to_owned(), fmt_real(q.w)),
        ]),
        TagFieldData::RealEulerAngles2d(e) => {
            pair("yaw", fmt_real(e.yaw), "pitch", fmt_real(e.pitch))
        }
        TagFieldData::RealEulerAngles3d(e) => Some(vec![
            ("yaw".to_owned(), fmt_real(e.yaw)),
            ("pitch".to_owned(), fmt_real(e.pitch)),
            ("roll".to_owned(), fmt_real(e.roll)),
        ]),
        TagFieldData::RealPlane2d(p) => {
            triple("i", fmt_real(p.i), "j", fmt_real(p.j), "d", fmt_real(p.d))
        }
        TagFieldData::RealPlane3d(p) => Some(vec![
            ("i".to_owned(), fmt_real(p.i)),
            ("j".to_owned(), fmt_real(p.j)),
            ("k".to_owned(), fmt_real(p.k)),
            ("d".to_owned(), fmt_real(p.d)),
        ]),
        TagFieldData::ShortIntegerBounds(b) => {
            pair("low", b.lower.to_string(), "high", b.upper.to_string())
        }
        TagFieldData::AngleBounds(b)
        | TagFieldData::RealBounds(b)
        | TagFieldData::FractionBounds(b) => {
            pair("low", fmt_real(b.lower), "high", fmt_real(b.upper))
        }
        _ => None,
    }
}

pub(super) fn foundation_bounds_values(value: &TagFieldData) -> Option<(String, String)> {
    match value {
        TagFieldData::ShortIntegerBounds(b) => Some((b.lower.to_string(), b.upper.to_string())),
        TagFieldData::AngleBounds(b)
        | TagFieldData::RealBounds(b)
        | TagFieldData::FractionBounds(b) => Some((fmt_real(b.lower), fmt_real(b.upper))),
        _ => None,
    }
}

pub(super) fn foundation_editable_component_parts(
    value: &TagFieldData,
) -> Option<Vec<(String, String)>> {
    match value {
        TagFieldData::RealPoint2d(p) => Some(vec![
            ("x".to_owned(), fmt_real(p.x)),
            ("y".to_owned(), fmt_real(p.y)),
        ]),
        TagFieldData::RealPoint3d(p) => Some(vec![
            ("x".to_owned(), fmt_real(p.x)),
            ("y".to_owned(), fmt_real(p.y)),
            ("z".to_owned(), fmt_real(p.z)),
        ]),
        TagFieldData::RealVector2d(v) => Some(vec![
            ("i".to_owned(), fmt_real(v.i)),
            ("j".to_owned(), fmt_real(v.j)),
        ]),
        TagFieldData::RealVector3d(v) => Some(vec![
            ("i".to_owned(), fmt_real(v.i)),
            ("j".to_owned(), fmt_real(v.j)),
            ("k".to_owned(), fmt_real(v.k)),
        ]),
        TagFieldData::RealQuaternion(q) => Some(vec![
            ("i".to_owned(), fmt_real(q.i)),
            ("j".to_owned(), fmt_real(q.j)),
            ("k".to_owned(), fmt_real(q.k)),
            ("w".to_owned(), fmt_real(q.w)),
        ]),
        _ => None,
    }
}

/// Export a block's elements as tab-separated rows (header = leaf scalar field
/// names; one row per element). Nested block/struct fields are omitted (flat
/// export). Tabs/newlines in values are flattened to spaces so columns align.
pub(super) fn block_to_tsv(block: &TagBlock<'_>, names: &TagNameIndex) -> String {
    elements_to_tsv(block.len(), names, |index| block.element(index))
}

/// TSV export for a fixed-size array (read-only — arrays have no clipboard
/// snapshot, but their values can still be copied out).
pub(super) fn array_to_tsv(array: &blam_tags::TagArray<'_>, names: &TagNameIndex) -> String {
    elements_to_tsv(array.len(), names, |index| array.element(index))
}

/// Shared TSV body: header row of leaf scalar field names, one row per element.
fn elements_to_tsv<'a>(
    count: usize,
    names: &TagNameIndex,
    get: impl Fn(usize) -> Option<TagStruct<'a>>,
) -> String {
    let Some(first) = get(0) else {
        return String::new();
    };
    let is_leaf = |field: &TagField<'_>| {
        field.as_block().is_none() && field.as_struct().is_none() && field.value().is_some()
    };
    let columns: Vec<String> = first
        .fields()
        .filter(is_leaf)
        .map(|field| clean_field_name(field.name()))
        .collect();
    if columns.is_empty() {
        return String::new();
    }
    let mut out = columns.join("\t");
    for index in 0..count {
        out.push('\n');
        if let Some(element) = get(index) {
            let cells: Vec<String> = element
                .fields()
                .filter(is_leaf)
                .filter_map(|field| {
                    field.value().map(|value| {
                        format_foundation_scalar_value(names, &value).replace(['\t', '\n'], " ")
                    })
                })
                .collect();
            out.push_str(&cells.join("\t"));
        }
    }
    out
}

/// Leaf scalar columns of a block element as `(clean name, full stored name)`
/// pairs — the inverse of [`block_to_tsv`]'s header, used by TSV import to map a
/// pasted column header back to the field path segment to write.
pub(super) fn block_leaf_columns(block: &TagBlock<'_>) -> Vec<(String, String)> {
    let Some(first) = block.element(0) else {
        return Vec::new();
    };
    first
        .fields()
        .filter(|field| {
            field.as_block().is_none() && field.as_struct().is_none() && field.value().is_some()
        })
        .map(|field| (clean_field_name(field.name()), field.name().to_owned()))
        .collect()
}

pub(super) fn format_foundation_scalar_value(names: &TagNameIndex, value: &TagFieldData) -> String {
    match value {
        TagFieldData::Angle(v)
        | TagFieldData::Real(v)
        | TagFieldData::RealSlider(v)
        | TagFieldData::RealFraction(v) => fmt_real(*v),
        TagFieldData::RealRgbColor(c) => format!(
            "r {}  g {}  b {}",
            fmt_real(c.red),
            fmt_real(c.green),
            fmt_real(c.blue)
        ),
        TagFieldData::RealArgbColor(c) => format!(
            "a {}  r {}  g {}  b {}",
            fmt_real(c.alpha),
            fmt_real(c.red),
            fmt_real(c.green),
            fmt_real(c.blue)
        ),
        TagFieldData::RealHsvColor(c) => format!(
            "h {}  s {}  v {}",
            fmt_real(c.hue),
            fmt_real(c.saturation),
            fmt_real(c.value)
        ),
        TagFieldData::RealAhsvColor(c) => format!(
            "a {}  h {}  s {}  v {}",
            fmt_real(c.alpha),
            fmt_real(c.hue),
            fmt_real(c.saturation),
            fmt_real(c.value)
        ),
        _ => trim_formatted_value(&format_value(names, value, false)),
    }
}

pub(super) fn fmt_real(value: f32) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let truncated = (value * 100.0).trunc() / 100.0;
    let mut text = format!("{truncated:.2}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

pub(super) fn is_hidden_non_expert_value(value: &TagFieldData, expert_mode: bool) -> bool {
    !expert_mode && matches!(value, TagFieldData::Custom(bytes) if bytes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_index_value_reads_all_variants() {
        use blam_tags::TagFieldData::*;
        assert_eq!(block_index_value(&CharBlockIndex(-1)), Some(-1));
        assert_eq!(block_index_value(&ShortBlockIndex(5)), Some(5));
        assert_eq!(block_index_value(&LongBlockIndex(42)), Some(42));
        assert_eq!(block_index_value(&CustomShortBlockIndex(3)), Some(3));
        // Non-block-index values don't read as a block index.
        assert_eq!(block_index_value(&LongInteger(7)), None);
    }

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

    fn with_test_edit_context(assertion: impl FnOnce(&FieldEditContext<'_>)) {
        let definitions_root = locate_definitions_root();
        let mut buffers = HashMap::new();
        let mut pending = Vec::new();
        let mut block_ops = Vec::new();
        let mut block_confirm = None;
        let mut open_request = None;
        let mut sound_play_request = None;
        let mut sound_extract_request = None;
        let mut tool_import = None;
        let mut bitmap_reimport = None;
        let mut shader_ops = Vec::new();
        let mut shader_param_ops = Vec::new();
        let mut h2_shader_param_ops = Vec::new();
        let mut function_data_ops = Vec::new();
        let mut model_variant_ops = Vec::new();
        let mut color_request = None;
        let mut function_request = None;
        let mut block_clip_request = None;
        let mut tsv_paste_request = None;
        let edit = FieldEditContext {
            view_scope: "test",
            tag_key: "test",
            group_tag: parse_group_tag("jpt!").unwrap(),
            root: None,
            game: Some("halo3_mcc"),
            definitions_root: Some(definitions_root.as_path()),
            definition_group_name: Some("damage_effect"),
            tags_root: None,
            status: None,
            editable: true,
            show_block_sizes: false,
            buffers: &mut buffers,
            pending: &mut pending,
            block_ops: &mut block_ops,
            block_confirm: &mut block_confirm,
            open_request: &mut open_request,
            sound_play_request: &mut sound_play_request,
            sound_status: None,
            sound_volume: 1.0,
            sound_extract_request: &mut sound_extract_request,
            sound_language: None,
            tool_import: &mut tool_import,
            bitmap_reimport: &mut bitmap_reimport,
            shader_ops: &mut shader_ops,
            shader_param_ops: &mut shader_param_ops,
            h2_shader_param_ops: &mut h2_shader_param_ops,
            function_data_ops: &mut function_data_ops,
            model_variant_ops: &mut model_variant_ops,
            color_request: &mut color_request,
            function_request: &mut function_request,
            block_clipboard: None,
            docs: None,
            tsv_paste_request: &mut tsv_paste_request,
            block_clip_request: &mut block_clip_request,
            field_filter: None,
        };
        assertion(&edit);
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
    fn legacy_mapping_function_bytes_build_inline_function_view() {
        let mut raw = vec![0; 20];
        raw[0] = 4;
        raw[1] = 1;
        raw[2] = 5;
        raw[4..8].copy_from_slice(&0.8f32.to_le_bytes());
        raw[8..12].copy_from_slice(&0.4f32.to_le_bytes());
        raw[12..16].copy_from_slice(&0.25f32.to_le_bytes());

        let view = legacy_mapping_function_view(&raw).expect("legacy data should parse");

        assert!(view.h2_legacy.is_some());
        assert_eq!(view.data_bytes(), raw);
    }

    #[test]
    fn tag_reference_picker_paths_must_be_under_tags_root() {
        let tags_root = PathBuf::from("tags");
        let picked = tags_root
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.bitmap");

        assert_eq!(
            tag_reference_relative_path_with_extension(&picked, &tags_root).unwrap(),
            r"objects\characters\brute\bitmaps\mask.bitmap"
        );

        let outside = PathBuf::from("data")
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.tif");
        assert_eq!(
            tag_reference_relative_path_with_extension(&outside, &tags_root).unwrap_err(),
            "Selected file must be inside the tags folder"
        );
    }

    #[test]
    fn tag_reference_group_validator_allows_none_and_matching_group() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let empty = TagReferenceData {
            group_tag_and_name: None,
        };
        let matching = TagReferenceData {
            group_tag_and_name: Some((render_model, r"objects\foo\foo".to_owned())),
        };
        let mismatched = TagReferenceData {
            group_tag_and_name: Some((collision_model, r"objects\foo\foo".to_owned())),
        };

        assert!(tag_reference_group_allowed(&empty, render_model));
        assert!(tag_reference_group_allowed(&matching, render_model));
        assert!(!tag_reference_group_allowed(&mismatched, render_model));
    }

    #[test]
    fn tag_reference_icon_prefers_single_declared_group_then_value() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();
        let target = (collision_model, r"objects\foo\foo".to_owned());
        let meta = |allowed| FieldDisplayMeta {
            label: "reference".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
        };

        assert_eq!(
            tag_reference_icon_group(&meta(vec![render_model]), Some(&target)),
            Some(render_model)
        );
        assert_eq!(
            tag_reference_icon_group(&meta(Vec::new()), Some(&target)),
            Some(collision_model)
        );
        assert_eq!(
            tag_reference_icon_group(&meta(vec![biped, vehicle]), Some(&target)),
            Some(collision_model)
        );
        assert_eq!(
            tag_reference_icon_group(&meta(vec![biped, vehicle]), None),
            None
        );
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
    fn semantic_short_index_target_names_cover_damage_sections() {
        let cases = [
            ("parent variant", Some("variants")),
            ("variant", Some("variants")),
            ("parent node", Some("nodes")),
            ("damage section", Some("damage sections")),
            ("indirect damage section", Some("damage sections")),
            ("runtime region index", None),
        ];

        for (field_name, expected) in cases {
            assert_eq!(semantic_short_index_target_key(field_name), expected);
        }
    }
}

pub(super) fn draw_resource(
    ui: &mut Ui,
    name: &str,
    resource: TagResource<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path_prefix: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let kind = match resource.kind() {
        TagResourceKind::Null => "null",
        TagResourceKind::Exploded => "exploded",
        TagResourceKind::Xsync => "xsync",
    };
    draw_foundation_bar(
        ui,
        format!("{}    pageable resource ({kind})", clean_field_name(name)),
        depth,
        false,
        |ui| {
            draw_foundation_text_row(
                ui,
                "inline bytes",
                &hex_bytes(resource.inline_bytes()),
                "bytes",
                depth + 1,
            );
            if let Some(payload) = resource.exploded_payload() {
                draw_foundation_text_row(
                    ui,
                    "exploded payload",
                    &format!("{} bytes", payload.len()),
                    "bytes",
                    depth + 1,
                );
            }
            if let Some(payload) = resource.xsync_payload() {
                draw_foundation_text_row(
                    ui,
                    "xsync payload",
                    &format!("{} bytes", payload.len()),
                    "bytes",
                    depth + 1,
                );
            }
            if resource.xsync_state().is_some() {
                draw_foundation_text_row(
                    ui,
                    "hydration",
                    "hydrated from XSync state",
                    "xsync",
                    depth + 1,
                );
            }
            if let Some(nested) = resource.as_struct() {
                ui.separator();
                draw_struct_fields_inline(
                    ui,
                    nested,
                    names,
                    depth + 1,
                    expert_mode,
                    path_prefix,
                    edit,
                );
            }
        },
    );
}
