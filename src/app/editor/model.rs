//! Object/model summaries and reference presentation.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn is_object_family_group(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"bipd" // biped
            | b"vehi" // vehicle
            | b"weap" // weapon
            | b"eqip" // equipment
            | b"scen" // scenery
            | b"mach" // device_machine
            | b"ctrl" // device_control
            | b"crat" // crate
            | b"bloc" // crate-like block
            | b"ssce" // sound_scenery
            | b"gint" // giant
            | b"proj" // projectile
            | b"obje" // object (base)
    )
}

/// Show the connected `.model` reference at the top of object-family tags
/// (biped, vehicle, weapon, scenery, …) with a working Open button.
pub(in crate::app) fn draw_object_model_summary(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    edit: &mut FieldEditContext<'_>,
) {
    if !is_object_family_group(entry.group_tag) {
        return;
    }
    let Some(model) = find_model_reference(tag.root(), names, 0, "") else {
        return;
    };
    let formatted = format_reference_path(names, model.group_tag, &model.rel_path);
    let meta = FieldDisplayMeta {
        label: "model".to_owned(),
        unit: None,
        range: None,
        help: Some("Object model tag reference".to_owned()),
        tag_reference_allowed: Vec::new(),
        read_only: false,
        advanced: false,
    };
    ui.add_space(4.0);
    let import_verb = geometry_import_verb(names, model.group_tag);
    draw_foundation_tag_reference_row(
        ui,
        &meta,
        &formatted,
        Some((model.group_tag, model.rel_path)),
        import_verb,
        0,
        &model.field_path,
        edit,
        shared_tag_reference_value_width(ui, 0),
    );
}

pub(in crate::app) struct ModelReferenceInfo {
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) rel_path: String,
    pub(in crate::app) field_path: String,
}

/// Like `find_model_reference` but returns the raw `(group_tag, rel_path)` so
/// the caller can resolve/open the target.
pub(in crate::app) fn find_model_reference(
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,

    depth: usize,
    path_prefix: &str,
) -> Option<ModelReferenceInfo> {
    if depth > 8 {
        return None;
    }
    for field in tag_struct.fields() {
        let field_path = append_field_path(path_prefix, field.name());
        match field.value() {
            Some(TagFieldData::TagReference(reference)) => {
                let Some((group_tag, path)) = reference.group_tag_and_name.as_ref() else {
                    continue;
                };
                if !is_model_group(*group_tag, names) || path.is_empty() {
                    continue;
                }
                return Some(ModelReferenceInfo {
                    group_tag: *group_tag,
                    rel_path: path.clone(),
                    field_path,
                });
            }
            Some(_) => continue,
            None => {}
        }
        if let Some(nested) = field.as_struct() {
            if let Some(found) = find_model_reference(nested, names, depth + 1, &field_path) {
                return Some(found);
            }
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().take(4).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().take(8).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

pub(in crate::app) fn is_model_group(group_tag: u32, names: &TagNameIndex) -> bool {
    group_tag == u32::from_be_bytes(*b"hlmt")
        || names.name_for(group_tag) == Some("model")
        || group_tag_to_extension(group_tag) == Some("model")
        // Halo CE has no `.model` (hlmt) wrapper — objects reference a
        // `.gbxmodel` (mod2) directly, which IS the render geometry, so
        // treat it as previewable in its own right.
        || group_tag == u32::from_be_bytes(*b"mod2")
        || names.name_for(group_tag) == Some("gbxmodel")
}

pub(in crate::app) fn format_reference_path(
    names: &TagNameIndex,
    group_tag: u32,
    path: &str,
) -> String {
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        format!("{path}.{extension}")
    } else {
        format!("{}:{path}", format_group_tag(group_tag))
    }
}

pub(in crate::app) fn draw_tag_metadata(ui: &mut Ui, tag: &TagFile, names: &TagNameIndex) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Header group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, tag.group().tag)).color(text_dark()));
        ui.label(RichText::new("Version:").color(subtle_dark()));

        ui.monospace(RichText::new(tag.group().version.to_string()).color(text_dark()));
        ui.label(RichText::new("Endian:").color(subtle_dark()));
        ui.monospace(
            RichText::new(match tag.endian {
                Endian::Le => "LE",
                Endian::Be => "BE",
            })
            .color(text_dark()),
        );
    });
}
