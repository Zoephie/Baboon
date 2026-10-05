//! Scalar, enum, bounds, color, and multi-component value rows.
//! It owns generic schema-driven field presentation; tag-specific panels and application workflow coordination belong elsewhere.

use super::*;

pub(in crate::app) fn draw_foundation_value_row(
    ui: &mut Ui,
    field: TagField<'_>,
    meta: &FieldDisplayMeta,
    type_name: &str,
    value: &TagFieldData,
    names: &TagNameIndex,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
    // The target block of a block-index field, when it could be found among
    // the struct's siblings or ancestors; `None` for non-block-index fields and
    // unresolvable (custom) indices → numeric editor.
    block_index: Option<&BlockIndexTarget>,
    tag_reference_value_width: f32,
) {
    if let (Some(target), Some(index)) = (block_index, block_index_value(value)) {
        draw_foundation_block_index_row(ui, meta, index, target, depth, path, edit);
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
            tag_reference_value_width,
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

/// A color value row: one editable cell per channel plus a clickable swatch
/// that opens the color picker. ARGB rows show all four components in a/r/g/b
/// order.

pub(in crate::app) fn draw_foundation_color_row(
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
    // Same order the color parser reads: "a, r, g, b" / "r, g, b".
    let channels: &[(&str, f32)] = if argb {
        &[("a", a), ("r", r), ("g", g), ("b", b)]
    } else {
        &[("r", r), ("g", g), ("b", b)]
    };
    let parts = channels
        .iter()
        .map(|(label, channel)| ((*label).to_owned(), format_pc_float(*channel)))
        .collect::<Vec<_>>();
    let swatch = Color32::from_rgb(
        float_channel_to_u8(r),
        float_channel_to_u8(g),
        float_channel_to_u8(b),
    );
    let editable = edit.editable && !meta.read_only;

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 12.0);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        draw_foundation_component_cells(ui, &parts, 76.0, path, editable, edit);

        let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        ui.painter().rect_filled(rect, 2.0, swatch);
        ui.painter()
            .rect_stroke(
                rect,
                2.0,
                Stroke::new(1.0_f32, foundation_input_edge()),
                egui::StrokeKind::Middle,
            );
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if editable {
                "Click to edit color"
            } else {
                "Click to inspect color"
            });
        if response.clicked() {
            let mut popup =
                MaterialColorPopup::new(&meta.label, r, g, b, a).with_alpha_available(argb);
            if editable {
                popup = popup.with_color_field(edit.tag_key, path, argb);
            }
            *edit.color_request = Some(popup);
        }
        draw_field_help(ui, meta);
    });
}

pub(in crate::app) fn draw_foundation_multi_value_row(
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

pub(in crate::app) fn draw_foundation_bounds_row(
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
    let mut lower = edit.buffers.take(&lower_key, lower_value);
    let mut upper = edit.buffers.take(&upper_key, upper_value);

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        if editable {
            let lower_response = foundation_text_edit_cell(ui, &mut lower.text, 92.0, lower_id);
            ui.label(RichText::new("to").color(subtle_dark()).small());
            let upper_response = foundation_text_edit_cell(ui, &mut upper.text, 92.0, upper_id);
            lower.note_response(&lower_response);
            upper.note_response(&upper_response);
            let commit = lower.should_commit(ui, &lower_response)
                || upper.should_commit(ui, &upper_response);
            if commit {
                edit.pending.push(PendingFieldEdit {
                    path: path.to_owned(),
                    input: format!("{}..{}", lower.text.trim(), upper.text.trim()),
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

    edit.buffers.put(lower_key, lower);
    edit.buffers.put(upper_key, upper);
}

pub(in crate::app) fn draw_foundation_component_edit_row(
    ui: &mut Ui,
    meta: &FieldDisplayMeta,
    parts: &[(String, String)],
    suffix: &str,
    depth: usize,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) {
    let indent = depth as f32 * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());
        let editable = edit.editable && !meta.read_only;
        draw_foundation_component_cells(ui, parts, 92.0, path, editable, edit);
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

/// One labelled cell per component, inside the caller's row layout. When
/// `editable`, each cell is a text box, and once any of them commits a change
/// the whole value is queued as one edit: every component's text joined with
/// `", "`, in `parts` order — the order the field's parser reads them in.
fn draw_foundation_component_cells(
    ui: &mut Ui,
    parts: &[(String, String)],
    width: f32,
    path: &str,
    editable: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let buffer_key = format!("{}|{}", edit.tag_key, path);
    let ids = parts
        .iter()
        .map(|(label, _)| edit.widget_id(("component", &buffer_key, label)))
        .collect::<Vec<_>>();
    let mut drafts = Vec::with_capacity(parts.len());
    for (label, value) in parts {
        let key = format!("{buffer_key}|component|{label}");
        let draft = edit.buffers.take(&key, value);
        drafts.push((key, draft));
    }

    let mut responses = Vec::with_capacity(parts.len());
    for (index, ((label, _), (_, draft))) in parts.iter().zip(drafts.iter_mut()).enumerate() {
        if !label.is_empty() {
            ui.label(RichText::new(label.as_str()).color(subtle_dark()).small());
        }
        if editable {
            let response = foundation_text_edit_cell(ui, &mut draft.text, width, ids[index]);
            draft.note_response(&response);
            responses.push(response);
        } else {
            foundation_input_cell(ui, &draft.text, width);
        }
    }
    if editable {
        let changed = drafts.iter().any(|(_, draft)| draft.changed);
        let committed = responses
            .iter()
            .zip(drafts.iter())
            .any(|(response, (_, draft))| draft.should_commit(ui, response));
        if committed && changed {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: drafts
                    .iter()
                    .map(|(_, draft)| draft.text.trim())
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
    }

    for (key, draft) in drafts {
        edit.buffers.put(key, draft);
    }
}

pub(in crate::app) fn draw_foundation_text_row(
    ui: &mut Ui,
    name: &str,
    value: &str,
    suffix: &str,
    depth: usize,
) {
    let meta = field_display_meta(name);
    draw_foundation_meta_text_row(ui, &meta, value, suffix, depth);
}

pub(in crate::app) fn draw_foundation_meta_text_row(
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

pub(in crate::app) fn draw_foundation_editable_text_row(
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
    let draft = edit.buffers.draft_mut(buffer_key, value);

    ui.horizontal(|ui| {
        ui.add_space(indent);
        foundation_label_cell(ui, &meta.label, meta.help.as_deref());

        let width = foundation_value_width(&draft.text, available_value_width);
        let response = foundation_text_edit_cell(ui, &mut draft.text, width, id);
        draft.note_response(&response);
        if draft.should_commit(ui, &response) {
            edit.pending.push(PendingFieldEdit {
                path: path.to_owned(),
                input: draft.text.trim().to_owned(),
            });
        }
        if !suffix.is_empty() {
            ui.label(RichText::new(suffix).color(subtle_dark()).small());
        }
        draw_field_help(ui, meta);
    });
}

/// Red used to flag tag references whose target file is missing on disk.
pub(in crate::app) const REFERENCE_MISSING_COLOR: Color32 = Color32::from_rgb(216, 92, 92);

#[cfg(test)]
mod tests {
    use super::*;

    // Foundation unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Both editable and read-only values must accept focus for selection/copy.
    /// Drive real pointer input through component rows, not just the cell helper.
    #[test]
    fn editable_and_read_only_value_rows_can_be_clicked_into() {
        assert!(
            click_across_value_row(true),
            "an editable value row must take a caret, or its text cannot be selected or copied"
        );
        assert!(
            click_across_value_row(false),
            "a read-only row must accept focus so its value can be selected and copied"
        );
    }

    /// Render one `real_vector_3d` row (the `Point 0 / x y z` shape from the
    /// report) and click along it until something takes keyboard focus.
    /// Returns whether anything ever did.
    fn click_across_value_row(editable: bool) -> bool {
        let ctx = egui::Context::default();
        let mut tag = TagFile::new(crate::app::test_definition_path(
            "halo4_mcc/camera_track.json",
        ))
        .unwrap();
        crate::core::document::apply::add_block_element(&mut tag, "control points").unwrap();
        let mut focused = false;

        with_test_edit_context(|edit| {
            edit.editable = editable;
            // Sweep the row rather than trusting a hand-computed cell position:
            // the widths are layout details, and a click that lands on the label
            // by accident would report "not editable" for the wrong reason.
            for step in 0..90 {
                let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(900.0, 200.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(pointer),
                        click(true),
                        click(false),
                    ],
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag
                            .root()
                            .field_path("control points[0]/position")
                            .expect("the control point's position field");
                        let value = field.value().expect("position has a value");
                        let meta = field_display_meta(field.name());
                        draw_foundation_value_row(
                            ui,
                            field,
                            &meta,
                            field.type_name(),
                            &value,
                            &TagNameIndex::default(),
                            0,
                            "control points[0]/position",
                            edit,
                            None,
                            300.0,
                        );
                    });
                });
                if ctx.memory(|memory| memory.focused()).is_some() {
                    focused = true;
                    break;
                }
            }
        });

        focused
    }

    /// Typing a channel value into a color row edits the color. The cells used
    /// to be read-only text, so the swatch's picker was the only way in, and
    /// setting exact RGB numbers was impossible (reported by a Halo CE modder).
    /// Drives real pointer and keyboard input through the whole value row, then
    /// applies whatever edit it queued the way the editor does.
    #[test]
    fn color_channels_are_typed_into_directly() {
        // Float ARGB: the first cell is alpha.
        let mut light = TagFile::new(crate::app::test_definition_path("haloce_mcc/light.json")).unwrap();
        let path = "color lower bound";
        let pending = type_into_first_value_cell(&light, path, "0.25");
        assert_eq!(pending.len(), 1, "one committed edit for the whole color");
        assert_eq!(pending[0].path, path);
        crate::core::document::apply::apply_field_edit(&mut light, path, &pending[0].input)
            .unwrap();
        match light.root().field_path(path).unwrap().value() {
            Some(TagFieldData::RealArgbColor(c)) => {
                assert_eq!((c.alpha, c.red, c.green, c.blue), (0.25, 0.0, 0.0, 0.0));
            }
            other => panic!("expected a real ARGB color, got {other:?}"),
        }

        // Packed ARGB: edited in the same 0-1 channels the row shows, stored as bytes.
        let mut hud =
            TagFile::new(crate::app::test_definition_path("haloce_mcc/grenade_hud_interface.json")).unwrap();
        let path = "override icon color";
        let pending = type_into_first_value_cell(&hud, path, "1");
        assert_eq!(pending.len(), 1);
        crate::core::document::apply::apply_field_edit(&mut hud, path, &pending[0].input).unwrap();
        match hud.root().field_path(path).unwrap().value() {
            Some(TagFieldData::ArgbColor(c)) => assert_eq!(c.0, 0xFF00_0000),
            other => panic!("expected a packed ARGB color, got {other:?}"),
        }
    }

    /// Click along a value row until a cell takes focus, replace its text with
    /// `text`, press Enter, and return the edits the row queued.
    fn type_into_first_value_cell(tag: &TagFile, path: &str, text: &str) -> Vec<PendingFieldEdit> {
        let ctx = egui::Context::default();
        let mut pending = Vec::new();
        with_test_edit_context(|edit| {
            let frame = |events: Vec<egui::Event>, edit: &mut FieldEditContext<'_>| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(900.0, 200.0))),
                    events,
                    ..Default::default()
                };
                let _ = crate::app::run_ui_test(&ctx, input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let field = tag.root().field_path(path).expect("color field");
                        let value = field.value().expect("color value");
                        let meta = field_display_meta(field.name());
                        draw_foundation_value_row(
                            ui, field, &meta, field.type_name(), &value,
                            &TagNameIndex::default(), 0, path, edit, None, 300.0,
                        );
                    });
                });
            };
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            for step in 0..90 {
                let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                frame(vec![egui::Event::PointerMoved(pointer), click(true), click(false)], edit);
                if ctx.memory(|memory| memory.focused()).is_some() {
                    break;
                }
            }
            assert!(ctx.memory(|memory| memory.focused()).is_some(), "no cell in the row took focus");
            frame(
                vec![
                    key(egui::Key::A, egui::Modifiers::COMMAND),
                    egui::Event::Text(text.to_owned()),
                ],
                edit,
            );
            assert!(edit.pending.is_empty(), "typing alone must not commit");
            frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)], edit);
            pending = std::mem::take(edit.pending);
        });
        pending
    }
}
