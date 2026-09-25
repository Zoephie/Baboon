//! Chimp property editor: drawing and editing an export's reflected and hand-written values.
//! It owns the per-value widgets; the document pane around them belongs in `document_ui`.

use super::*;

pub(super) fn draw_chimp_export_editor(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    usmap: &Usmap,
) -> bool {
    let Some(export) = document.exports.get_mut(document.selected_export) else {
        ui.label("This package has no exports.");
        return false;
    };
    ui.heading(&export.object);
    ui.label(
        RichText::new(export.class.as_deref().unwrap_or("Unknown class")).color(subtle_dark()),
    );
    ui.add_space(6.0);
    let class = export.class.clone().unwrap_or_default();
    match &mut export.decoded {
        Ok(decoded) => match &mut decoded.block {
            ExportBlock::Reflected(block) => {
                ui.label(
                    RichText::new(
                        "Filled circle = editable scalar; outlined values are preserved read-only.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        draw_chimp_property_block(
                            ui,
                            block,
                            &class,
                            &mut document.header.name_map,
                            usmap,
                            0,
                        )
                    })
                    .inner
            }
            ExportBlock::NotSerialized => {
                ui.label("This class has no reflected property block.");
                ui.label("Its native payload is preserved byte-for-byte.");
                false
            }
            ExportBlock::Unreflected(block) => {
                ui.label("Reflection data for this class is not available.");
                ui.label(format!(
                    "{} untyped bytes are preserved byte-for-byte.",
                    block.rest.len()
                ));
                false
            }
        },
        Err(error) => {
            ui.colored_label(Color32::from_rgb(210, 120, 80), error);
            ui.label("The raw export remains available for extraction and is never rewritten.");
            false
        }
    }
}

fn draw_chimp_property_block(
    ui: &mut Ui,
    block: &mut PropertyBlock,
    class: &str,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    let existing: std::collections::BTreeSet<u32> = block
        .entries
        .iter()
        .filter_map(|entry| entry.slot.map(|slot| slot.index))
        .collect();
    // Flattened once for the block. Every row used to flatten the class's
    // whole schema chain again to find its own type, each frame.
    let schema = (!class.is_empty())
        .then(|| blam_tags::iostore::object::block::flattened_schema(class, usmap).ok())
        .flatten();
    if let Some(schema) = &schema {
        let omitted: Vec<(&str, u8, &PropertyType)> = schema
            .iter()
            .enumerate()
            .filter(|(index, (property, _, _))| {
                !existing.contains(&(*index as u32))
                    && default_value_for_type(&property.ty, usmap).is_ok()
            })
            .map(|(_, (property, array_index, _))| {
                (property.name.as_str(), *array_index, &property.ty)
            })
            .collect();
        if !omitted.is_empty() {
            let added = right_opening_menu_button(
                ui,
                format!("Add omitted property… ({})", omitted.len()),
                300.0,
                |ui| {
                    style_list_menu(ui);
                    ui.set_max_height(360.0);
                    let mut added = false;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for &(name, array_index, ty) in &omitted {
                            let label = if array_index == 0 {
                                name.to_owned()
                            } else {
                                format!("{name}[{array_index}]")
                            };
                            if ui.button(label).on_hover_text(format!("{ty:?}")).clicked()
                                && let Ok(value) = default_value_for_type(ty, usmap)
                                && set_property_slot(block, class, name, array_index, value, usmap)
                                    .is_ok()
                            {
                                changed = true;
                                added = true;
                            }
                        }
                    });
                    added
                },
            )
            .inner
            .unwrap_or(false);
            if added {
                ui.close_menu();
            }
            ui.separator();
        }
    }
    for (index, entry) in block.entries.iter_mut().enumerate() {
        let id = ui.make_persistent_id((depth, index, entry.name.as_ref()));
        let declared = entry
            .slot
            .and_then(|slot| declared_slot_type(schema.as_deref()?, slot));
        let label = match entry.slot.map(|slot| slot.array_index) {
            Some(array_index) if array_index > 0 => {
                format!("{}[{array_index}]", entry.name)
            }
            _ => entry.name.to_string(),
        };
        ui.horizontal_top(|ui| {
            ui.set_min_height(24.0);
            ui.label(RichText::new(label).strong()).on_hover_text(
                declared
                    .map(|ty| format!("{ty:?}"))
                    .unwrap_or_else(|| "Native field without a USMAP slot".to_owned()),
            );
            changed |= chimp_property_value_cell(ui, |ui| {
                draw_chimp_value(ui, id, &mut entry.value, declared, names, usmap, depth)
            })
            .inner;
        });
        ui.separator();
    }
    changed
}

/// A slot's declared type in an already-flattened schema: the rule
/// `property_type_for_slot` applies, without flattening per call.
fn declared_slot_type<'u>(
    schema: &[(
        &'u blam_tags::iostore::object::usmap::UsmapProperty,
        u8,
        &'u str,
    )],
    slot: blam_tags::iostore::object::value::SchemaSlot,
) -> Option<&'u PropertyType> {
    let (property, array_index, _) = schema.get(slot.index as usize)?;
    (*array_index == slot.array_index).then_some(&property.ty)
}

/// Look an enum up by name. `Usmap` keeps enums in a list, and the property
/// editor did a linear search per enum-valued row, per frame. Hits are
/// checked against the list, so a stale position only costs the search.
fn usmap_enum<'u>(
    usmap: &'u Usmap,
    name: &str,
) -> Option<&'u blam_tags::iostore::object::usmap::UsmapEnum> {
    thread_local! {
        static POSITIONS: std::cell::RefCell<HashMap<String, usize>> =
            std::cell::RefCell::new(HashMap::new());
    }
    let cached = POSITIONS.with(|positions| positions.borrow().get(name).copied());
    if let Some(position) = cached
        && let Some(definition) = usmap.enums.get(position)
        && definition.name == name
    {
        return Some(definition);
    }
    let position = usmap.enums.iter().position(|item| item.name == name)?;
    POSITIONS.with(|positions| {
        positions.borrow_mut().insert(name.to_owned(), position);
    });
    usmap.enums.get(position)
}

fn chimp_property_value_cell<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    // `Ui::with_layout` inherits the parent's full remaining cross-axis size.
    // In a vertical ScrollArea that made one compact scalar editor consume the
    // entire viewport and then centered the widget inside it. Start each value
    // cell at one normal row instead; `allocate_ui_with_layout` still expands
    // when a nested struct or container genuinely needs additional height.
    let row_height = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_height),
        egui::Layout::right_to_left(egui::Align::Min),
        add_contents,
    )
}

fn draw_chimp_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut PropValue,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    if let Some(PropertyType::Optional(inner)) = declared
        && !matches!(value, PropValue::Unset)
    {
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.small_button("Unset").clicked() {
                *value = PropValue::Unset;
                changed = true;
            } else {
                changed |= draw_chimp_value(
                    ui,
                    id.with("optional"),
                    value,
                    Some(inner),
                    names,
                    usmap,
                    depth,
                );
            }
        });
        return changed;
    }
    match value {
        PropValue::Bool(value) => ui.checkbox(value, "").changed(),
        PropValue::Int(value) => draw_chimp_integer(ui, value, declared, usmap),
        PropValue::Float(value) => ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
        PropValue::Str(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                value.set_text(text);
            }
            changed
        }
        PropValue::Name(value) => edit_chimp_fname(ui, id, value, names),
        PropValue::Object(value) => ui
            .add(egui::DragValue::new(value).prefix("Object "))
            .on_hover_text("FPackageIndex: negative = import, positive = export, zero = None")
            .changed(),
        PropValue::SoftObject(value) => {
            let mut changed = false;
            egui::CollapsingHeader::new("Soft object path")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= draw_chimp_fname(ui, "Package", &mut value.package, names);
                    changed |= draw_chimp_fname(ui, "Asset", &mut value.asset, names);
                    let mut sub_path = value.sub_path.to_string();
                    ui.horizontal(|ui| {
                        ui.label("Sub-path");
                        if ui.text_edit_singleline(&mut sub_path).changed() {
                            value.sub_path.set_text(sub_path);
                            changed = true;
                        }
                    });
                });
            changed
        }
        PropValue::Struct(block) => {
            let nested_class = match declared {
                Some(PropertyType::Struct(name)) => name.as_str(),
                _ => "",
            };
            egui::CollapsingHeader::new(format!("Struct ({} properties)", block.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    draw_chimp_property_block(ui, block, nested_class, names, usmap, depth + 1)
                })
                .body_returned
                .unwrap_or(false)
        }
        PropValue::Array(values) => draw_chimp_sequence(
            ui,
            id,
            "Array",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Array(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            true,
        ),
        PropValue::Set(values) => draw_chimp_sequence(
            ui,
            id,
            "Set",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Set(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            false,
        ),
        PropValue::Map(values) => draw_chimp_map(ui, id, values, declared, names, usmap, depth),
        PropValue::WithRemovals { removals, inner } => {
            let removal_type = match declared {
                Some(PropertyType::Map(key, _)) | Some(PropertyType::Set(key)) => {
                    Some(key.as_ref())
                }
                _ => None,
            };
            let mut changed = false;
            egui::CollapsingHeader::new("Delta-serialized container")
                .id_salt(id)
                .show(ui, |ui| {
                    let mut replace_whole = removals.is_none();
                    if ui
                        .checkbox(&mut replace_whole, "Replace whole container")
                        .changed()
                    {
                        *removals = (!replace_whole).then(Vec::new);
                        changed = true;
                    }
                    if let Some(removals) = removals {
                        changed |= draw_chimp_sequence(
                            ui,
                            id.with("removals"),
                            "Removed values",
                            removals,
                            removal_type,
                            names,
                            usmap,
                            depth + 1,
                            true,
                        );
                    }
                    changed |= draw_chimp_value(
                        ui,
                        id.with("inner"),
                        inner,
                        declared,
                        names,
                        usmap,
                        depth + 1,
                    );
                });
            changed
        }
        PropValue::Native(value) => draw_chimp_native(ui, id, value),
        PropValue::HandWritten(value) => {
            draw_chimp_hand_written(ui, id, value, names, usmap, depth)
        }
        PropValue::Delegate { object, function } => {
            let mut changed = false;
            egui::CollapsingHeader::new("Delegate")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= ui
                        .add(egui::DragValue::new(object).prefix("Object "))
                        .changed();
                    changed |= draw_chimp_fname(ui, "Function", function, names);
                });
            changed
        }
        PropValue::MulticastDelegate(values) => {
            draw_chimp_multicast_delegate(ui, id, values, names)
        }
        PropValue::FieldPath { path, owner } => {
            let mut changed = ui
                .add(egui::DragValue::new(owner).prefix("Owner "))
                .changed();
            egui::CollapsingHeader::new(format!("Field path ({} segments)", path.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    let mut remove = None;
                    for (index, segment) in path.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= draw_chimp_fname(ui, &format!("[{index}]"), segment, names);
                            if ui.small_button("−").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        path.remove(index);
                        changed = true;
                    }
                    if ui.small_button("+ segment").clicked() {
                        path.push(blam_tags::iostore::object::value::FName::none());
                        changed = true;
                    }
                });
            changed
        }
        PropValue::Unset => {
            if let Some(PropertyType::Optional(inner)) = declared
                && ui.button("Set value").clicked()
            {
                match default_value_for_type(inner, usmap) {
                    Ok(default) => {
                        *value = default;
                        return true;
                    }
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(210, 120, 80), error.to_string());
                    }
                }
            } else {
                ui.label("Unset");
            }
            false
        }
        PropValue::Raw(bytes) => {
            ui.colored_label(
                Color32::from_rgb(190, 150, 70),
                format!("{} untyped bytes · preserved read-only", bytes.len()),
            );
            false
        }
    }
}

fn draw_chimp_fname(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FName,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        let id = ui.auto_id_with(("chimp_fname", label));
        edit_chimp_fname(ui, id, value, names)
    })
    .inner
}

/// An FName text box that interns its name once, when the edit is committed.
///
/// Interning adds any name the package does not have yet to its name map, and
/// the map is written into the saved package. Interning per keystroke, as
/// this used to, left every prefix typed on the way ("R", "Ro", "Roc", …) in
/// the package for good. The text lives in egui memory while the box has
/// focus; leaving it commits, and Escape discards.
fn edit_chimp_fname(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut blam_tags::iostore::object::value::FName,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let draft_id = id.with("fname_draft");
    let current = value.to_string();
    let mut text = ui
        .data(|data| data.get_temp::<String>(draft_id))
        .unwrap_or_else(|| current.clone());
    let response = ui.add(egui::TextEdit::singleline(&mut text).id(id));
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(draft_id, text));
        return false;
    }
    ui.data_mut(|data| data.remove::<String>(draft_id));
    let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if !response.lost_focus() || cancelled || text == current {
        return false;
    }
    *value = blam_tags::iostore::object::edit::intern_name(names, &text);
    true
}

fn draw_chimp_fstr(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FStr,
) -> bool {
    let mut text = value.to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(&mut text).changed() {
            value.set_text(text);
            changed = true;
        }
    });
    changed
}

fn draw_chimp_integer(
    ui: &mut Ui,
    value: &mut i64,
    declared: Option<&PropertyType>,
    usmap: &Usmap,
) -> bool {
    let ty = match declared {
        Some(PropertyType::Enum { inner, enum_name }) => {
            if let Some(definition) = usmap_enum(usmap, enum_name) {
                let selected = definition
                    .values
                    .iter()
                    .find(|(candidate, _)| *candidate == *value as u64)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("Unknown");
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(format!("{selected} ({})", *value as u64))
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            Some(inner.as_ref())
        }
        Some(PropertyType::Byte {
            enum_name: Some(enum_name),
        }) => {
            if let Some(definition) = usmap_enum(usmap, enum_name) {
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(
                        definition
                            .values
                            .iter()
                            .find(|(candidate, _)| *candidate == *value as u64)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| value.to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            declared
        }
        other => other,
    };
    match ty {
        Some(PropertyType::UInt64) => {
            let mut unsigned = *value as u64;
            let changed = ui.add(egui::DragValue::new(&mut unsigned)).changed();
            if changed {
                *value = unsigned as i64;
            }
            changed
        }
        Some(PropertyType::UInt32) => ui
            .add(egui::DragValue::new(value).range(0..=u32::MAX as i64))
            .changed(),
        Some(PropertyType::UInt16) => ui
            .add(egui::DragValue::new(value).range(0..=u16::MAX as i64))
            .changed(),
        Some(PropertyType::Int8) => ui
            .add(egui::DragValue::new(value).range(i8::MIN as i64..=i8::MAX as i64))
            .changed(),
        Some(PropertyType::Int16) => ui
            .add(egui::DragValue::new(value).range(i16::MIN as i64..=i16::MAX as i64))
            .changed(),
        Some(PropertyType::Int) => ui
            .add(egui::DragValue::new(value).range(i32::MIN as i64..=i32::MAX as i64))
            .changed(),
        Some(PropertyType::Byte { .. }) => ui
            .add(egui::DragValue::new(value).range(0..=u8::MAX as i64))
            .changed(),
        _ => ui.add(egui::DragValue::new(value)).changed(),
    }
}

#[derive(Clone, Copy)]
enum ChimpListAction {
    Remove(usize),
    Duplicate(usize),
    MoveUp(usize),
    MoveDown(usize),
}

fn draw_chimp_list<T: Clone>(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<T>,
    default: Option<T>,
    mut draw: impl FnMut(&mut Ui, usize, &mut T) -> bool,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(format!("{label} ({})", values.len()))
        .id_salt(id)
        .show(ui, |ui| {
            let mut action = None;
            for (index, value) in values.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.label(format!("[{index}]"));
                        if ui.small_button("↑").clicked() {
                            action = Some(ChimpListAction::MoveUp(index));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some(ChimpListAction::MoveDown(index));
                        }
                        if ui.small_button("Duplicate").clicked() {
                            action = Some(ChimpListAction::Duplicate(index));
                        }
                        if ui.small_button("Remove").clicked() {
                            action = Some(ChimpListAction::Remove(index));
                        }
                    });
                    changed |= draw(ui, index, value);
                    ui.separator();
                });
            }
            if let Some(default) = default
                && ui.small_button("+ Add").clicked()
            {
                values.push(default);
                changed = true;
            }
            match action {
                Some(ChimpListAction::Remove(index)) => {
                    values.remove(index);
                    changed = true;
                }
                Some(ChimpListAction::Duplicate(index)) => {
                    values.insert(index + 1, values[index].clone());
                    changed = true;
                }
                Some(ChimpListAction::MoveUp(index)) if index > 0 => {
                    values.swap(index, index - 1);
                    changed = true;
                }
                Some(ChimpListAction::MoveDown(index)) if index + 1 < values.len() => {
                    values.swap(index, index + 1);
                    changed = true;
                }
                _ => {}
            }
        });
    changed
}

fn draw_chimp_sequence(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<PropValue>,
    inner_type: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
    allow_duplicates: bool,
) -> bool {
    let default = inner_type.and_then(|ty| default_value_for_type(ty, usmap).ok());
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, label, values, default, |ui, index, value| {
        draw_chimp_value(
            ui,
            id.with(index),
            value,
            inner_type,
            names,
            usmap,
            depth + 1,
        )
    });
    // Only an edit can introduce a duplicate, so the quadratic scan runs on
    // the frame something changed rather than every frame.
    if changed && !allow_duplicates {
        let mut duplicate = false;
        for left in 0..values.len() {
            duplicate |= values[left + 1..]
                .iter()
                .any(|right| values[left].semantic_eq(right));
        }
        if duplicate {
            *values = before;
            changed = false;
            ui.colored_label(
                Color32::from_rgb(210, 120, 80),
                "Sets cannot contain duplicate values.",
            );
        }
    }
    changed
}

fn draw_chimp_map(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(PropValue, PropValue)>,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let (key_type, value_type) = match declared {
        Some(PropertyType::Map(key, value)) => (Some(key.as_ref()), Some(value.as_ref())),
        _ => (None, None),
    };
    let default = key_type
        .and_then(|key| default_value_for_type(key, usmap).ok())
        .zip(value_type.and_then(|value| default_value_for_type(value, usmap).ok()));
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, "Map", values, default, |ui, index, pair| {
        let mut changed = false;
        ui.label(RichText::new("Key").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "key")),
            &mut pair.0,
            key_type,
            names,
            usmap,
            depth + 1,
        );
        ui.label(RichText::new("Value").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "value")),
            &mut pair.1,
            value_type,
            names,
            usmap,
            depth + 1,
        );
        changed
    });
    // As for sets: only an edit can introduce a duplicate key.
    let mut duplicate = false;
    if changed {
        for left in 0..values.len() {
            duplicate |= values[left + 1..]
                .iter()
                .any(|right| values[left].0.semantic_eq(&right.0));
        }
    }
    if duplicate {
        *values = before;
        changed = false;
        ui.colored_label(
            Color32::from_rgb(210, 120, 80),
            "Maps cannot contain duplicate keys.",
        );
    }
    changed
}

fn draw_chimp_multicast_delegate(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(i32, blam_tags::iostore::object::value::FName)>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    draw_chimp_list(
        ui,
        id,
        "Multicast delegate",
        values,
        Some((0, blam_tags::iostore::object::value::FName::none())),
        |ui, _, (object, function)| {
            let mut changed = ui
                .add(egui::DragValue::new(object).prefix("Object "))
                .changed();
            changed |= draw_chimp_fname(ui, "Function", function, names);
            changed
        },
    )
}

fn draw_chimp_f64_values(ui: &mut Ui, values: &mut [f64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_f32_values(ui: &mut Ui, values: &mut [f32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_i64_values(ui: &mut Ui, values: &mut [i64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_i32_values(ui: &mut Ui, values: &mut [i32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_native(ui: &mut Ui, id: egui::Id, value: &mut NativeStruct) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Native struct")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                NativeStruct::Vec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec4d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec2d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::TwoVec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec3f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Vec2f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::LinearColor(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4d(values) => draw_chimp_f64_values(ui, values.as_mut()),
                NativeStruct::Ints(values) => draw_chimp_i64_values(ui, values),
                NativeStruct::Guid(values) => {
                    let mut changed = false;
                    ui.horizontal_wrapped(|ui| {
                        for (index, value) in values.iter_mut().enumerate() {
                            changed |= ui
                                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                                .changed();
                        }
                    });
                    changed
                }
                NativeStruct::Color(values) => {
                    let mut changed = false;
                    for (label, value) in ["B", "G", "R", "A"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::Box3d { min, max, is_valid } => {
                    ui.label("Minimum");
                    let mut changed = draw_chimp_f64_values(ui, min);
                    ui.label("Maximum");
                    changed |= draw_chimp_f64_values(ui, max);
                    changed |= ui
                        .add(egui::DragValue::new(is_valid).prefix("Valid: "))
                        .changed();
                    changed
                }
                NativeStruct::RichCurveKey {
                    interp_mode,
                    tangent_mode,
                    tangent_weight_mode,
                    values,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(interp_mode).prefix("Interpolation: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_mode).prefix("Tangent: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_weight_mode).prefix("Weight: "))
                        .changed();
                    changed |= draw_chimp_f32_values(ui, values);
                    changed
                }
                NativeStruct::FontCharacter {
                    start_u,
                    start_v,
                    size_u,
                    size_v,
                    texture_index,
                    vertical_offset,
                } => {
                    let mut changed = false;
                    for (label, value) in [
                        ("Start U", start_u),
                        ("Start V", start_v),
                        ("Size U", size_u),
                        ("Size V", size_v),
                        ("Vertical offset", vertical_offset),
                    ] {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed |= ui
                        .add(egui::DragValue::new(texture_index).prefix("Texture: "))
                        .changed();
                    changed
                }
                NativeStruct::PackedBits(value) => ui
                    .add(egui::DragValue::new(value).prefix("Bits: "))
                    .changed(),
                NativeStruct::I32(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::I64(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::FrameRange {
                    lower_kind,
                    lower,
                    upper_kind,
                    upper,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(lower_kind).prefix("Lower kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(lower).prefix("Lower: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper_kind).prefix("Upper kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper).prefix("Upper: "))
                        .changed();
                    changed
                }
                NativeStruct::EvaluationKey(values) => {
                    let mut changed = false;
                    for (label, value) in ["Sequence", "Track", "Section"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::PerPlatform { cooked, value } => {
                    let mut changed = ui.checkbox(cooked, "Cooked").changed();
                    changed |= match value {
                        PerPlatformValue::Int(value) => {
                            ui.add(egui::DragValue::new(value)).changed()
                        }
                        PerPlatformValue::Float(value) => {
                            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                        }
                        PerPlatformValue::Bool(value) => ui.checkbox(value, "Value").changed(),
                        PerPlatformValue::FrameRate(numerator, denominator) => {
                            let mut inner = ui
                                .add(egui::DragValue::new(numerator).prefix("Numerator: "))
                                .changed();
                            inner |= ui
                                .add(egui::DragValue::new(denominator).prefix("Denominator: "))
                                .changed();
                            inner
                        }
                    };
                    changed
                }
                NativeStruct::EmptySerializer => {
                    ui.label("No serialized fields");
                    false
                }
            };
        });
    changed
}

fn draw_chimp_optional_f32(ui: &mut Ui, label: &str, value: &mut Option<f32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0.0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value).speed(0.01)).changed();
    }
    changed
}

fn draw_chimp_optional_i32(ui: &mut Ui, label: &str, value: &mut Option<i32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_optional_u64(ui: &mut Ui, label: &str, value: &mut Option<u64>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_tree_entry(ui: &mut Ui, value: &mut chimp_hw::TreeEntry) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.start).prefix("Start: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.size).prefix("Size: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.capacity).prefix("Capacity: "))
        .changed();
    changed
}

fn draw_chimp_tree_node(ui: &mut Ui, value: &mut chimp_hw::TreeNode) -> bool {
    let mut changed = false;
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower_kind).prefix("Lower kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower).prefix("Lower: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper_kind).prefix("Upper kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper).prefix("Upper: "))
        .changed();
    for (label, item) in [
        ("Parent children", &mut value.parent_children_handle),
        ("Parent index", &mut value.parent_index),
        ("Children id", &mut value.children_id),
        ("Data id", &mut value.data_id),
    ] {
        changed |= ui
            .add(egui::DragValue::new(item).prefix(format!("{label}: ")))
            .changed();
    }
    changed
}

fn draw_chimp_shader_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::ShaderValueType,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.kind).prefix("Kind: "))
        .changed();
    changed |= ui
        .checkbox(&mut value.is_dynamic_array, "Dynamic array")
        .changed();
    match &mut value.body {
        chimp_hw::ShaderValueTypeBody::Struct { name, elements } => {
            changed |= draw_chimp_fname(ui, "Struct", name, names);
            changed |= draw_chimp_list(
                ui,
                id.with("elements"),
                "Elements",
                elements,
                None,
                |ui, index, (name, value)| {
                    let mut item_changed = draw_chimp_fname(ui, "Name", name, names);
                    item_changed |=
                        draw_chimp_shader_value(ui, id.with(("element", index)), value, names);
                    item_changed
                },
            );
        }
        chimp_hw::ShaderValueTypeBody::Dimension { dimension, counts } => {
            changed |= ui
                .add(egui::DragValue::new(dimension).prefix("Dimension: "))
                .changed();
            changed |= draw_chimp_list(
                ui,
                id.with("counts"),
                "Counts",
                counts,
                Some(0),
                |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
            );
        }
    }
    changed
}

fn draw_chimp_text_argument(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextFormatArgument,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    match value {
        chimp_hw::TextFormatArgument::Int(value) => ui.add(egui::DragValue::new(value)).changed(),
        chimp_hw::TextFormatArgument::UInt(value) | chimp_hw::TextFormatArgument::Gender(value) => {
            ui.add(egui::DragValue::new(value)).changed()
        }
        chimp_hw::TextFormatArgument::Float(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Double(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Text(value) => {
            draw_chimp_text(ui, id.with("text"), value, names)
        }
    }
}

fn draw_chimp_text(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextValue,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.flags).prefix("Flags: "))
        .changed();
    match &mut value.history {
        chimp_hw::TextHistory::None { culture_invariant } => {
            let mut present = culture_invariant.is_some();
            if ui
                .checkbox(&mut present, "Culture-invariant string")
                .changed()
            {
                *culture_invariant = present.then(Default::default);
                changed = true;
            }
            if let Some(text) = culture_invariant {
                changed |= draw_chimp_fstr(ui, "Value", text);
            }
        }
        chimp_hw::TextHistory::Base {
            namespace,
            key,
            source,
        } => {
            changed |= draw_chimp_fstr(ui, "Namespace", namespace);
            changed |= draw_chimp_fstr(ui, "Key", key);
            changed |= draw_chimp_fstr(ui, "Source", source);
        }
        chimp_hw::TextHistory::StringTableEntry { table_id, key } => {
            changed |= draw_chimp_fname(ui, "Table", table_id, names);
            changed |= draw_chimp_fstr(ui, "Key", key);
        }
        chimp_hw::TextHistory::OrderedFormat {
            source_fmt,
            arguments,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, argument| draw_chimp_text_argument(ui, id.with(index), argument, names),
            );
        }
        chimp_hw::TextHistory::NamedFormat {
            kind,
            source_fmt,
            arguments,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, (name, argument)| {
                    let mut item_changed = draw_chimp_fstr(ui, "Name", name);
                    item_changed |= draw_chimp_text_argument(ui, id.with(index), argument, names);
                    item_changed
                },
            );
        }
        chimp_hw::TextHistory::AsNumber {
            kind,
            currency_code,
            source_value,
            options,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            if let Some(currency) = currency_code {
                changed |= draw_chimp_fstr(ui, "Currency", currency);
            }
            changed |= draw_chimp_text_argument(ui, id.with("value"), source_value, names);
            if let Some(options) = options {
                changed |= ui
                    .checkbox(&mut options.always_sign, "Always sign")
                    .changed();
                changed |= ui
                    .checkbox(&mut options.use_grouping, "Use grouping")
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut options.rounding_mode).prefix("Rounding: "))
                    .changed();
                for (label, field) in [
                    ("Minimum integral", &mut options.minimum_integral_digits),
                    ("Maximum integral", &mut options.maximum_integral_digits),
                    ("Minimum fractional", &mut options.minimum_fractional_digits),
                    ("Maximum fractional", &mut options.maximum_fractional_digits),
                ] {
                    changed |= ui
                        .add(egui::DragValue::new(field).prefix(format!("{label}: ")))
                        .changed();
                }
            }
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::AsDateTime {
            kind,
            source_date_time,
            date_style,
            time_style,
            custom_pattern,
            time_zone,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(source_date_time).prefix("Ticks: "))
                .changed();
            for (label, value) in [("Date style", date_style), ("Time style", time_style)] {
                let mut present = value.is_some();
                if ui.checkbox(&mut present, label).changed() {
                    *value = present.then_some(0);
                    changed = true;
                }
                if let Some(value) = value {
                    changed |= ui.add(egui::DragValue::new(value)).changed();
                }
            }
            if let Some(pattern) = custom_pattern {
                changed |= draw_chimp_fstr(ui, "Pattern", pattern);
            }
            changed |= draw_chimp_fstr(ui, "Time zone", time_zone);
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::Transform {
            source_text,
            transform_type,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_text, names);
            changed |= ui
                .add(egui::DragValue::new(transform_type).prefix("Transform: "))
                .changed();
        }
        chimp_hw::TextHistory::TextGenerator {
            generator_type_id,
            contents,
        } => {
            changed |= draw_chimp_fname(ui, "Generator", generator_type_id, names);
            ui.label(match contents {
                Some(bytes) => format!("{} generator bytes · preserved read-only", bytes.len()),
                None => "No generator payload".to_owned(),
            });
        }
    }
    changed
}

fn draw_chimp_sampler(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::WeightedRandomSampler,
) -> bool {
    let mut changed = draw_chimp_list(
        ui,
        id.with("prob"),
        "Probabilities",
        &mut value.prob,
        Some(0.0),
        |ui, _, value| ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
    );
    changed |= draw_chimp_list(
        ui,
        id.with("alias"),
        "Aliases",
        &mut value.alias,
        Some(0),
        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
    );
    changed |= ui
        .add(egui::DragValue::new(&mut value.total_weight).prefix("Total weight: "))
        .changed();
    changed
}

fn draw_chimp_property_bag_type(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagPropertyType,
) -> bool {
    use chimp_hw::PropertyBagPropertyType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [
                T::None,
                T::Bool,
                T::Byte,
                T::Int32,
                T::Int64,
                T::Float,
                T::Double,
                T::Name,
                T::String,
                T::Text,
                T::Enum,
                T::Struct,
                T::Object,
                T::SoftObject,
                T::Class,
                T::SoftClass,
                T::UInt32,
                T::UInt64,
            ] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown type is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_property_bag_container(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagContainerType,
) -> bool {
    use chimp_hw::PropertyBagContainerType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [T::None, T::Array, T::Set] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown container is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_hand_written(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::HandWritten,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Typed Unreal structure")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                chimp_hw::HandWritten::MaterialLayersTree(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("nodes"),
                        "Nodes",
                        &mut value.nodes,
                        Some([0; 4]),
                        |ui, _, values| {
                            let mut changed = false;
                            for value in values {
                                changed |= ui.add(egui::DragValue::new(value)).changed();
                            }
                            changed
                        },
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("payloads"),
                        "Payloads",
                        &mut value.payloads,
                        Some([0; 2]),
                        |ui, _, values| {
                            values.iter_mut().fold(false, |changed, value| {
                                ui.add(egui::DragValue::new(value)).changed() || changed
                            })
                        },
                    );
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.root).prefix("Root: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MovieSceneInlineValue(value) => {
                    let mut inner = draw_chimp_fstr(ui, "Type", &mut value.type_name);
                    if let Some(payload) = &mut value.payload {
                        let class = value.type_name.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::EvaluationTree(value) => {
                    let mut inner = draw_chimp_tree_node(ui, &mut value.root);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_entries"),
                        "Child entries",
                        &mut value.child_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_nodes"),
                        "Child nodes",
                        &mut value.child_nodes,
                        None,
                        |ui, _, value| draw_chimp_tree_node(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("data_entries"),
                        "Data entries",
                        &mut value.data_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("items"),
                        "Items",
                        &mut value.items,
                        None,
                        |ui, _, value| match value {
                            chimp_hw::TreeItem::EntityAndMetaDataIndex { entity, meta_data } => {
                                ui.add(egui::DragValue::new(entity).prefix("Entity: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(meta_data).prefix("Metadata: "))
                                        .changed()
                            }
                            chimp_hw::TreeItem::SubSequence { sequence_id, flags } => {
                                ui.add(egui::DragValue::new(sequence_id).prefix("Sequence: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(flags).prefix("Flags: "))
                                        .changed()
                            }
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::ShaderValueType(value) => {
                    draw_chimp_shader_value(ui, id, value, names)
                }
                chimp_hw::HandWritten::PerQualityLevel(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.default_bits).prefix("Default bits: "))
                        .changed();
                    inner |= draw_chimp_list(
                        ui,
                        id.with("overrides"),
                        "Overrides",
                        &mut value.overrides,
                        Some((0, 0)),
                        |ui, _, (quality, bits)| {
                            ui.add(egui::DragValue::new(quality).prefix("Quality: "))
                                .changed()
                                | ui.add(egui::DragValue::new(bits).prefix("Bits: "))
                                    .changed()
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::FontData(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.font_face_asset).prefix("Face asset: "),
                        )
                        .changed();
                    let mut inline = value.inline_face.is_some();
                    if ui.checkbox(&mut inline, "Inline face").changed() {
                        value.inline_face = inline.then(|| chimp_hw::InlineFontFace {
                            filename: Default::default(),
                            hinting: 0,
                            loading_policy: 0,
                        });
                        inner = true;
                    }
                    if let Some(face) = &mut value.inline_face {
                        inner |= draw_chimp_fstr(ui, "Filename", &mut face.filename);
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.hinting).prefix("Hinting: "))
                            .changed();
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.loading_policy).prefix("Loading: "))
                            .changed();
                    }
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.sub_face_index).prefix("Sub-face: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MaterialOverrideNanite(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= draw_chimp_optional_i32(
                        ui,
                        "Override material",
                        &mut value.override_material,
                    );
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.properties,
                        "MaterialOverrideNanite",
                        names,
                        usmap,
                        depth + 1,
                    );
                    inner
                }
                chimp_hw::HandWritten::TimeWarpVariant(value) => match value {
                    chimp_hw::TimeWarpVariant::Literal(value) => {
                        ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                    }
                    chimp_hw::TimeWarpVariant::Typed {
                        kind,
                        object,
                        payload,
                    } => {
                        let mut inner = ui
                            .add(egui::DragValue::new(kind).prefix("Kind: "))
                            .changed();
                        inner |= draw_chimp_optional_i32(ui, "Object", object);
                        if let Some(payload) = payload {
                            inner |=
                                draw_chimp_property_block(ui, payload, "", names, usmap, depth + 1);
                        }
                        inner
                    }
                },
                chimp_hw::HandWritten::LocatorFragment(value) => {
                    let mut inner =
                        draw_chimp_fname(ui, "Fragment type", &mut value.fragment_type, names);
                    if let Some(payload) = &mut value.payload {
                        let class = value.fragment_type.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::Text(value) => draw_chimp_text(ui, id, value, names),
                chimp_hw::HandWritten::MovieSceneChannel(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.pre_infinity_extrap)
                                .prefix("Pre extrapolation: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.post_infinity_extrap)
                                .prefix("Post extrapolation: "),
                        )
                        .changed();
                    ui.label(format!("{} time bytes · preserved", value.times.data.len()));
                    ui.label(format!(
                        "{} value bytes · preserved",
                        value.values.data.len()
                    ));
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.default_value)
                                .speed(0.01)
                                .prefix("Default: "),
                        )
                        .changed();
                    inner |= ui
                        .checkbox(&mut value.has_default_value, "Has default")
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_numerator)
                                .prefix("Tick numerator: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_denominator)
                                .prefix("Tick denominator: "),
                        )
                        .changed();
                    inner |= ui.checkbox(&mut value.show_curve, "Show curve").changed();
                    inner
                }
                chimp_hw::HandWritten::PcgPoint(value) => {
                    let mut inner = draw_chimp_f64_values(ui, &mut value.transform);
                    inner |= draw_chimp_optional_f32(ui, "Density", &mut value.density);
                    for (label, point) in [
                        ("Bounds minimum", &mut value.bounds_min),
                        ("Bounds maximum", &mut value.bounds_max),
                    ] {
                        let mut present = point.is_some();
                        if ui.checkbox(&mut present, label).changed() {
                            *point = present.then_some([0.0; 3]);
                            inner = true;
                        }
                        if let Some(point) = point {
                            inner |= draw_chimp_f64_values(ui, point);
                        }
                    }
                    let mut color = value.color.is_some();
                    if ui.checkbox(&mut color, "Color").changed() {
                        value.color = color.then_some([0.0; 4]);
                        inner = true;
                    }
                    if let Some(color) = &mut value.color {
                        inner |= draw_chimp_f64_values(ui, color);
                    }
                    inner |= draw_chimp_optional_f32(ui, "Steepness", &mut value.steepness);
                    inner |= draw_chimp_optional_i32(ui, "Seed", &mut value.seed);
                    inner |=
                        draw_chimp_optional_u64(ui, "Metadata entry", &mut value.metadata_entry);
                    inner
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingLod(value) => {
                    draw_chimp_sampler(ui, id, value)
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingRegion(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("triangles"),
                        "Triangle indices",
                        &mut value.triangle_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("bones"),
                        "Bone indices",
                        &mut value.bone_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_sampler(ui, id.with("sampler"), &mut value.sampler);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("vertices"),
                        "Vertices",
                        &mut value.vertices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner
                }
                chimp_hw::HandWritten::NiagaraVariable(value) => {
                    let mut inner = draw_chimp_fname(ui, "Name", &mut value.name, names);
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.type_def,
                        "NiagaraTypeDefinition",
                        names,
                        usmap,
                        depth + 1,
                    );
                    match &mut value.payload {
                        chimp_hw::NiagaraPayload::None => {}
                        chimp_hw::NiagaraPayload::Offset(value) => {
                            inner |= ui
                                .add(egui::DragValue::new(value).prefix("Offset: "))
                                .changed()
                        }
                        chimp_hw::NiagaraPayload::VarData(bytes) => {
                            ui.label(format!(
                                "{} variable-data bytes · preserved read-only",
                                bytes.len()
                            ));
                        }
                    }
                    inner
                }
                chimp_hw::HandWritten::NiagaraGpuParamInfo(value) => {
                    let mut inner = draw_chimp_fstr(ui, "HLSL symbol", &mut value.hlsl_symbol);
                    inner |= draw_chimp_fstr(ui, "DI class", &mut value.di_class_name);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("functions"),
                        "Generated functions",
                        &mut value.generated_functions,
                        None,
                        |ui, index, function| {
                            let mut item = draw_chimp_fname(
                                ui,
                                "Definition",
                                &mut function.definition_name,
                                names,
                            );
                            item |= draw_chimp_fstr(ui, "Instance", &mut function.instance_name);
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "specifiers")),
                                "Specifiers",
                                &mut function.specifiers,
                                Some((
                                    blam_tags::iostore::object::value::FName::none(),
                                    blam_tags::iostore::object::value::FName::none(),
                                )),
                                |ui, _, (name, value)| {
                                    draw_chimp_fname(ui, "Name", name, names)
                                        | draw_chimp_fname(ui, "Value", value, names)
                                },
                            );
                            let default_reference = chimp_hw::NiagaraVariableCommonReference {
                                name: blam_tags::iostore::object::value::FName::none(),
                                underlying_type: 0,
                            };
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "inputs")),
                                "Variadic inputs",
                                &mut function.variadic_inputs,
                                Some(default_reference.clone()),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "outputs")),
                                "Variadic outputs",
                                &mut function.variadic_outputs,
                                Some(default_reference),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::InstancedPropertyBag(value) => {
                    let mut inner = ui
                        .add(egui::DragValue::new(&mut value.serial_size).prefix("Serial size: "))
                        .changed();
                    if let Some(descriptors) = &mut value.descriptors {
                        let default_descriptor = chimp_hw::PropertyBagDesc {
                            value_type_object: 0,
                            id: Default::default(),
                            name: blam_tags::iostore::object::value::FName::none(),
                            value_type: chimp_hw::PropertyBagPropertyType::None,
                            container_types: Vec::new(),
                        };
                        inner |= draw_chimp_list(
                            ui,
                            id.with("descriptors"),
                            "Descriptors",
                            descriptors,
                            Some(default_descriptor),
                            |ui, index, descriptor| {
                                let mut item = ui
                                    .add(
                                        egui::DragValue::new(&mut descriptor.value_type_object)
                                            .prefix("Type object: "),
                                    )
                                    .changed();
                                item |= draw_chimp_fname(ui, "Name", &mut descriptor.name, names);
                                ui.label("ID");
                                item |= draw_chimp_i32_values(ui, &mut descriptor.id.0);
                                ui.horizontal(|ui| {
                                    ui.label("Type");
                                    item |= draw_chimp_property_bag_type(
                                        ui,
                                        id.with((index, "type")),
                                        &mut descriptor.value_type,
                                    );
                                });
                                item |= draw_chimp_list(
                                    ui,
                                    id.with((index, "containers")),
                                    "Containers",
                                    &mut descriptor.container_types,
                                    Some(chimp_hw::PropertyBagContainerType::None),
                                    |ui, container, value| {
                                        draw_chimp_property_bag_container(
                                            ui,
                                            id.with((index, container)),
                                            value,
                                        )
                                    },
                                );
                                item
                            },
                        );
                    }
                    if let Some(values) = &mut value.values {
                        inner |= draw_chimp_property_block(ui, values, "", names, usmap, depth + 1);
                    }
                    inner
                }
            };
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::iostore::object::value::FName;
    use blam_tags::iostore::package::name_map::{EMappedNameType, FNameMap};

    /// The property editor reads each row's type out of one flattened schema
    /// instead of asking the engine per row. Same answer, for every slot of
    /// every class in the bundled schema, including a slot whose static-array
    /// index does not match.
    #[test]
    fn declared_slot_types_match_the_engine_for_every_slot() {
        use blam_tags::iostore::object::value::SchemaSlot;
        let usmap = Usmap::meteorite().unwrap();
        let mut compared = 0usize;
        for class in &usmap.structs {
            let Ok(schema) =
                blam_tags::iostore::object::block::flattened_schema(&class.name, &usmap)
            else {
                continue;
            };
            for (index, (_, array_index, _)) in schema.iter().enumerate() {
                for array_index in [*array_index, array_index.wrapping_add(1)] {
                    let slot = SchemaSlot {
                        index: index as u32,
                        array_index,
                        zero_masked: false,
                    };
                    assert_eq!(
                        declared_slot_type(&schema, slot),
                        property_type_for_slot(&class.name, slot, &usmap)
                            .ok()
                            .as_ref(),
                        "{}[{index}]",
                        class.name
                    );
                    compared += 1;
                }
            }
            // The omitted-property menu lists the same slots the engine's
            // catalog does.
            let catalog =
                blam_tags::iostore::object::edit::editable_schema_slots(&class.name, &usmap)
                    .unwrap();
            assert_eq!(catalog.len(), schema.len());
            for ((name, slot, ty), (property, array_index, _)) in catalog.iter().zip(&schema) {
                assert_eq!(
                    (name.as_str(), slot.array_index, ty),
                    (property.name.as_str(), *array_index, &property.ty)
                );
            }
        }
        assert!(compared > 10_000, "compared only {compared} slots");
    }

    /// A position remembered for one enum list is checked before it is used.
    #[test]
    fn a_stale_enum_position_still_finds_the_enum() {
        let mut usmap = Usmap::meteorite().unwrap();
        let name = usmap.enums[0].name.clone();
        assert_eq!(usmap_enum(&usmap, &name).unwrap().name, name);
        let last = usmap.enums.len() - 1;
        usmap.enums.swap(0, last);
        assert_eq!(usmap_enum(&usmap, &name).unwrap().name, name);
        assert!(usmap_enum(&usmap, "NoSuchEnumAnywhere").is_none());
    }

    #[test]
    fn chimp_scalar_property_row_does_not_consume_the_scroll_viewport() {
        let context = egui::Context::default();
        let mut value = 0_i64;
        let mut row_height = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_200.0, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let row = ui.horizontal_top(|ui| {
                            ui.set_min_height(24.0);
                            ui.label("NumReplicatedProperties");
                            chimp_property_value_cell(ui, |ui| {
                                ui.add(egui::DragValue::new(&mut value));
                            });
                        });
                        row_height = Some(row.response.rect.height());
                    });
                });
            },
        );

        let row_height = row_height.expect("the property row was rendered");
        assert!(
            row_height <= 40.0,
            "a scalar property row expanded to {row_height}px"
        );
    }

    /// Draw one FName box for a frame of `events`.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        value: &mut FName,
        names: &mut FNameMap,
    ) -> bool {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(400.0, 100.0),
            )),
            events,
            ..Default::default()
        };
        let mut changed = false;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed |= draw_chimp_fname(ui, "Name", value, names);
            });
        });
        changed
    }

    /// Typing a new name must add one name to the package, not one per
    /// keystroke: every name interned is written into the saved package.
    #[test]
    fn typing_an_fname_interns_one_name_on_commit() {
        let ctx = egui::Context::default();
        let mut names =
            FNameMap::create_from_names(EMappedNameType::Package, vec!["None".to_owned()]);
        let mut value = FName::new(0, 0, "None");
        let before = names.len();

        // Click across the row until the text box has focus.
        for step in 0..40 {
            let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
            let click = |pressed| egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(
                &ctx,
                vec![
                    egui::Event::PointerMoved(pointer),
                    click(true),
                    click(false),
                ],
                &mut value,
                &mut names,
            );
            if ctx.memory(|memory| memory.focused()).is_some() {
                break;
            }
        }
        assert!(
            ctx.memory(|memory| memory.focused()).is_some(),
            "the box never took focus"
        );

        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let select_all = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        frame(
            &ctx,
            vec![key(egui::Key::A, select_all)],
            &mut value,
            &mut names,
        );
        let mut changes = 0;
        for letter in "Rocket".chars() {
            if frame(
                &ctx,
                vec![egui::Event::Text(letter.to_string())],
                &mut value,
                &mut names,
            ) {
                changes += 1;
            }
        }
        assert_eq!(names.len(), before, "nothing is interned while typing");
        if frame(
            &ctx,
            vec![key(egui::Key::Enter, Default::default())],
            &mut value,
            &mut names,
        ) {
            changes += 1;
        }

        assert_eq!(changes, 1, "one committed change");
        assert_eq!(value.to_string(), "Rocket");
        assert_eq!(
            names.len(),
            before + 1,
            "exactly one new name: {:?}",
            names.names()
        );
    }
}
