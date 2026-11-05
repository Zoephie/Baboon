//! Presentation groups for Halo 2 HUD fields. Selection uses the original
//! struct ordinals: section boundaries never become serialized field paths.

use super::*;

struct HudDoc<'a> {
    index: usize,
    title: &'a str,
    body: &'a str,
}

fn documentation<'a>(
    edit: &FieldEditContext<'a>,
    st: &TagStruct<'_>,
    title: &str,
) -> Option<HudDoc<'a>> {
    edit.docs?
        .entries_for_struct(st)
        .iter()
        .enumerate()
        .find_map(|(index, entry)| {
            if let DefEntry::Explanation { title: name, body } = entry
                && name.eq_ignore_ascii_case(title)
            {
                Some(HudDoc {
                    index,
                    title: name,
                    body,
                })
            } else {
                None
            }
        })
}

fn section(
    ui: &mut Ui,
    st: TagStruct<'_>,
    title: &str,
    path: &str,
    ordinals: &[usize],
    doc: Option<&HudDoc<'_>>,
    depth: usize,
    edit: &mut FieldEditContext<'_>,
    contents: impl FnOnce(&mut Ui, &mut FieldEditContext<'_>),
) {
    let mut anchors = st
        .fields_all()
        .filter(|field| ordinals.contains(&field.ordinal()))
        .map(|field| append_field_path_for(path, &field))
        .collect::<Vec<_>>();
    if let Some(doc) = doc {
        anchors.push(documentation_path(path, doc.index));
    }
    if !anchors.iter().any(|anchor| edit.field_visible(anchor)) {
        return;
    }
    let default_open = edit.default_open(true);
    let requested = anchors
        .iter()
        .filter_map(|anchor| edit.resolve_open(anchor, default_open))
        .collect::<Vec<_>>();
    let open = if requested.contains(&true) {
        Some(true)
    } else if requested.contains(&false) {
        Some(false)
    } else {
        None
    };
    draw_foundation_group(
        ui,
        title.to_owned(),
        ("hud_section", strip_element_indices(path), title),
        depth,
        default_open,
        open,
        |ui| contents(ui, edit),
    );
}

fn body_documentation(
    ui: &mut Ui,
    path: &str,
    doc: Option<&HudDoc<'_>>,
    edit: &FieldEditContext<'_>,
) {
    let Some(doc) = doc.filter(|doc| !doc.body.trim().is_empty()) else {
        return;
    };
    let doc_path = documentation_path(path, doc.index);
    if !edit.field_visible(&doc_path) {
        return;
    }
    mark_find_render_cell(ui, edit.tag_key, &doc_path);
    if ui
        .data(|data| data.get_temp::<String>(field_jump_target_id()))
        .as_deref()
        == Some(doc_path.as_str())
    {
        ui.scroll_to_cursor(Some(egui::Align::Center));
        ui.data_mut(|data| {
            data.remove::<String>(field_jump_target_id());
        });
    }
    Frame::NONE
        .fill(foundation_documentation_bg())
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            let galley = findable_wrapped_galley(
                ui,
                doc.body.trim_end(),
                FontId::monospace(12.0),
                text_dark(),
                FindTargetKind::Documentation,
            );
            ui.add(egui::Label::new(galley));
        });
}

fn subheading(
    ui: &mut Ui,
    title: &str,
    path: &str,
    doc: Option<&HudDoc<'_>>,
    depth: usize,
    edit: &FieldEditContext<'_>,
) {
    if let Some(doc) = doc {
        mark_find_render_cell(ui, edit.tag_key, &documentation_path(path, doc.index));
    }
    draw_foundation_explanation_row(ui, title, None, depth, ("hud_axis", path, title), None);
}

pub(super) fn draw_hud_struct(
    ui: &mut Ui,
    st: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    expert_mode: bool,
    path: &str,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    if !is_h2_hud_function_tag(edit.game, edit.group_tag) {
        return false;
    }
    match st.definition().name() {
        "hud_widget_state_definition_struct_2" => {
            let flags = st
                .fields_all()
                .filter(|field| field.field_type() == TagFieldType::WordFlags)
                .map(|field| field.ordinal())
                .collect::<Vec<_>>();
            if flags.len() != 8 {
                return false;
            }
            let remaining = st
                .fields_all()
                .filter(|field| {
                    !flags.contains(&field.ordinal())
                        && field.field_type() != TagFieldType::Explanation
                })
                .map(|field| field.ordinal())
                .collect::<Vec<_>>();
            let doc = documentation(edit, &st, "widget state");
            section(
                ui,
                st,
                "Widget States",
                path,
                &flags,
                doc.as_ref(),
                depth,
                edit,
                |ui, edit| {
                    body_documentation(ui, path, doc.as_ref(), edit);
                    draw_selected_fields(
                        ui,
                        &st,
                        names,
                        depth + 1,
                        expert_mode,
                        path,
                        edit,
                        &flags,
                    );
                },
            );
            draw_selected_fields(ui, &st, names, depth, expert_mode, path, edit, &remaining);
            true
        }
        "hud_widget_effect_block_struct" => {
            let functions = st
                .fields_all()
                .filter(|field| {
                    field.as_struct().is_some_and(|nested| {
                        nested.definition().name() == "hud_widget_effect_function_struct_2"
                    })
                })
                .map(|field| field.ordinal())
                .collect::<Vec<_>>();
            if functions.len() != 5 {
                return false;
            }
            let remaining = st
                .fields_all()
                .filter(|field| {
                    !functions.contains(&field.ordinal())
                        && field.field_type() != TagFieldType::Explanation
                })
                .map(|field| field.ordinal())
                .collect::<Vec<_>>();
            if let Some(doc) = documentation(edit, &st, "WIDGET EFFECTS") {
                draw_injected_explanation_row(
                    ui, doc.title, doc.body, depth, path, doc.index, edit,
                );
            }
            draw_selected_fields(ui, &st, names, depth, expert_mode, path, edit, &remaining);
            for (title, old_title, slots, axes) in [
                (
                    "Scale",
                    "horizontal and vertical scale",
                    &functions[..2],
                    &["Horizontal Scale", "Vertical Scale"][..],
                ),
                ("Theta", "theta", &functions[2..3], &[][..]),
                (
                    "Offset",
                    "horizontal and vertical offset",
                    &functions[3..],
                    &["Horizontal Offset", "Vertical Offset"][..],
                ),
            ] {
                let doc = documentation(edit, &st, old_title);
                section(
                    ui,
                    st,
                    title,
                    path,
                    slots,
                    doc.as_ref(),
                    depth,
                    edit,
                    |ui, edit| {
                        body_documentation(ui, path, doc.as_ref(), edit);
                        for (index, ordinal) in slots.iter().enumerate() {
                            let field_visible = st
                                .fields_all()
                                .find(|field| field.ordinal() == *ordinal)
                                .is_some_and(|field| {
                                    edit.field_visible(&append_field_path_for(path, &field))
                                });
                            if !field_visible {
                                continue;
                            }
                            if let Some(axis) = axes.get(index) {
                                subheading(ui, axis, path, doc.as_ref(), depth + 1, edit);
                            }
                            draw_selected_fields(
                                ui,
                                &st,
                                names,
                                depth + 1,
                                expert_mode,
                                path,
                                edit,
                                &[*ordinal],
                            );
                        }
                    },
                );
            }
            true
        }
        _ => false,
    }
}
