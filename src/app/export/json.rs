//! json export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(in crate::app) fn export_tag_json(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let value = tag_to_json(&tag, entry);
    let text = serde_json::to_string_pretty(&value)?;
    fs::write(output, text)?;
    Ok(format!("Wrote JSON {}", output.display()))
}

pub(in crate::app) fn export_loose_folder_json(
    root: &Path,
    rel_path: &Path,
    names: &TagNameIndex,
    output: &Path,
) -> anyhow::Result<String> {
    let entries = scan_folder_subtree_entries(root, rel_path, names)?;
    if entries.is_empty() {
        anyhow::bail!("no tag files found in {}", root.join(rel_path).display());
    }
    let source = TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: None,
        definitions_root: PathBuf::new(),
    };
    export_tag_json_entries(&source, &entries, output)
}

pub(in crate::app) fn export_tag_json_entries(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    fs::create_dir_all(output)?;
    let mut written = 0usize;
    let mut failures = Vec::new();

    for entry in entries {
        let path = output.join(tag_json_relative_path(entry));
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                failures.push(format!("{}: {error}", entry.display_path));
                continue;
            }
        }

        let result = (|| -> anyhow::Result<()> {
            let tag = read_entry(source, entry)?;
            let value = tag_to_json(&tag, entry);
            let text = serde_json::to_string_pretty(&value)?;
            fs::write(&path, text)?;
            Ok(())
        })();

        match result {
            Ok(()) => written += 1,
            Err(error) => failures.push(format!("{}: {error}", entry.display_path)),
        }
    }

    if written == 0 && !failures.is_empty() {
        anyhow::bail!("failed to dump folder JSON: {}", failures.join("; "));
    }
    if written == 0 {
        anyhow::bail!("no tag files found");
    }

    let mut message = format!("Wrote {written} JSON tag file(s) to {}", output.display());
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

pub(in crate::app) fn tag_to_json(tag: &TagFile, entry: &TagEntry) -> Value {
    json!({
        "path": entry.display_path,
        "group": format_group_tag(tag.group().tag),
        "group_name": entry.group_name,
        "version": tag.group().version,
        "endian": match tag.endian {
            Endian::Le => "LE",
            Endian::Be => "BE",
        },
        "fields": struct_to_json(tag.root()),
    })
}

pub(in crate::app) fn struct_to_json(tag_struct: TagStruct<'_>) -> Value {
    Value::Array(tag_struct.fields().map(field_to_json).collect())
}

pub(in crate::app) fn field_to_json(field: TagField<'_>) -> Value {
    if let Some(value) = field.value() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": field.type_name(),
            "value": field_value_to_json(value),
        });
    }
    if let Some(block) = field.as_block() {
        let elements = block.iter().map(struct_to_json).collect::<Vec<_>>();
        return json!({
            "name": clean_field_name(field.name()),
            "type": "block",
            "count": block.len(),
            "elements": elements,
        });
    }
    if let Some(array) = field.as_array() {
        let elements = array.iter().map(struct_to_json).collect::<Vec<_>>();
        return json!({
            "name": clean_field_name(field.name()),
            "type": "array",
            "count": array.len(),
            "elements": elements,
        });
    }
    if let Some(nested) = field.as_struct() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": "struct",
            "fields": struct_to_json(nested),
        });
    }
    if let Some(resource) = field.as_resource() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": "pageable_resource",
            "kind": format!("{:?}", resource.kind()),
            "inline_bytes": resource.inline_bytes().len(),
            "exploded_payload_bytes": resource.exploded_payload().map(|payload| payload.len()),
            "xsync_payload_bytes": resource.xsync_payload().map(|payload| payload.len()),
            "header": resource.as_struct().map(struct_to_json),
        });
    }
    json!({
        "name": clean_field_name(field.name()),
        "type": field.type_name(),
    })
}

pub(in crate::app) fn field_value_to_json(value: TagFieldData) -> Value {
    match value {
        TagFieldData::String(s) | TagFieldData::LongString(s) => json!(s),
        TagFieldData::StringId(s) | TagFieldData::OldStringId(s) => {
            json!({ "string": s.string })
        }
        TagFieldData::TagReference(reference) => match reference.group_tag_and_name {
            Some((group_tag, path)) => json!({
                "group": format_group_tag(group_tag),
                "path": path,
            }),
            None => Value::Null,
        },
        TagFieldData::CharInteger(v) => json!(v),
        TagFieldData::ShortInteger(v) => json!(v),
        TagFieldData::LongInteger(v) => json!(v),
        TagFieldData::Int64Integer(v) => json!(v),
        TagFieldData::ByteInteger(v) => json!(v),
        TagFieldData::WordInteger(v) => json!(v),
        TagFieldData::DwordInteger(v) => json!(v),
        TagFieldData::QwordInteger(v) => json!(v),
        TagFieldData::Tag(v) => json!(format_group_tag(v)),
        TagFieldData::CharEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::ShortEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::LongEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::ByteFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::WordFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::LongFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::Data(bytes) => json!({ "bytes": bytes.len() }),
        TagFieldData::ApiInterop(value) => json!({ "raw_bytes": value.raw.len() }),
        TagFieldData::Custom(bytes) => json!({ "bytes": bytes.len() }),
        other => json!(format!("{other:?}")),
    }
}

#[cfg(test)]
mod import_info_tests {
    use super::*;

    #[test]
    fn import_info_paths_are_sanitized_to_relative_paths() {
        assert_eq!(
            sanitize_import_info_path(r"c:\mcc\source\objects\brute\brute.jms"),
            PathBuf::from("mcc")
                .join("source")
                .join("objects")
                .join("brute")
                .join("brute.jms")
        );
        assert_eq!(
            sanitize_import_info_path(r"..\..\escape.jms"),
            PathBuf::from("escape.jms")
        );
        assert_eq!(sanitize_import_info_path(""), PathBuf::from("file"));
    }

    #[test]
    fn h2_render_model_import_info_resolves_from_root_block() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
        {
            let mut root = tag.root_mut();
            let mut import_info_field = root.field_path_mut("import info").unwrap();
            let mut import_info = import_info_field.as_block_mut().unwrap();
            import_info.add_element();
        }

        let root = tag.root();
        let import_info = resolve_import_info_struct(&tag, root).expect("root import info block");

        assert!(import_info.field("files").is_some());
    }
}
