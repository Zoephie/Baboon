use super::*;

pub(super) const COMPANION_REFERENCE_PREFIX: &str = "__baboon_companion__/";

pub(super) fn prepare_companion_outputs(
    draft: &mut TagConversionDraft,
    main_output: &Path,
    target_tags_root: &Path,
    dependency_schema: &Path,
) -> Result<Vec<PathBuf>, String> {
    let parent = main_output
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", main_output.display()))?;
    let stem = main_output
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("{} has no valid file name", main_output.display()))?;
    let mut outputs = Vec::with_capacity(draft.companion_tags.len());
    let mut references = HashMap::new();
    for companion in &draft.companion_tags {
        let output = parent.join(format!(
            "{stem}{}.{}",
            companion.file_suffix, companion.extension
        ));
        let reference = output_reference_path(&output, target_tags_root)?;
        if references
            .insert(companion.key.clone(), reference)
            .is_some()
        {
            return Err(format!("Duplicate companion key {}", companion.key));
        }
        outputs.push(output);
    }
    resolve_companion_references(draft.tag.root_mut(), &references)?;
    draft
        .tag
        .rebuild_dependency_list(dependency_schema)
        .map_err(|error| format!("Could not rebuild converted tag dependencies: {error}"))?;
    for companion in &mut draft.companion_tags {
        resolve_companion_references(companion.tag.root_mut(), &references)?;
        companion
            .tag
            .rebuild_dependency_list(dependency_schema)
            .map_err(|error| {
                format!(
                    "Could not rebuild {} companion dependencies: {error}",
                    companion.group_name
                )
            })?;
    }
    verify_roundtrip(&draft.tag, main_output)?;
    for (companion, output) in draft.companion_tags.iter().zip(&outputs) {
        verify_roundtrip(&companion.tag, output)?;
    }
    Ok(outputs)
}

fn output_reference_path(output: &Path, target_tags_root: &Path) -> Result<String, String> {
    let output = normalize_conversion_path(output);
    let root = normalize_conversion_path(target_tags_root);
    let relative = output.strip_prefix(&root).map_err(|_| {
        format!(
            "Companion output {} is outside target tags root {}",
            output.display(),
            root.display()
        )
    })?;
    let mut reference = relative.to_path_buf();
    reference.set_extension("");
    Ok(reference
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("\\"))
}

fn resolve_companion_references(
    mut structure: TagStructMut<'_>,
    references: &HashMap<String, String>,
) -> Result<(), String> {
    let field_count = structure.as_ref().fields().count();
    for ordinal in 0..field_count {
        let Some(mut field) = structure.field_at_mut(ordinal) else {
            continue;
        };
        match field.as_ref().field_type() {
            TagFieldType::TagReference => {
                let Some(TagFieldData::TagReference(reference)) = field.as_ref().value() else {
                    continue;
                };
                let Some((group, path)) = reference.group_tag_and_name else {
                    continue;
                };
                let Some(key) = path.strip_prefix(COMPANION_REFERENCE_PREFIX) else {
                    continue;
                };
                let resolved = references
                    .get(key)
                    .ok_or_else(|| format!("Missing generated companion output for {key}"))?;
                field
                    .set(TagFieldData::TagReference(TagReferenceData {
                        group_tag_and_name: Some((group, resolved.clone())),
                    }))
                    .map_err(|error| {
                        format!("Could not resolve companion reference {key}: {error:?}")
                    })?;
            }
            TagFieldType::Struct => {
                if let Some(nested) = field.as_struct_mut() {
                    resolve_companion_references(nested, references)?;
                }
            }
            TagFieldType::Block => {
                if let Some(mut block) = field.as_block_mut() {
                    for index in 0..block.len() {
                        if let Some(element) = block.element_mut(index) {
                            resolve_companion_references(element, references)?;
                        }
                    }
                }
            }
            TagFieldType::Array => {
                if let Some(mut array) = field.as_array_mut() {
                    for index in 0..array.len() {
                        if let Some(element) = array.element_mut(index) {
                            resolve_companion_references(element, references)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn verify_roundtrip(tag: &TagFile, output: &Path) -> Result<(), String> {
    let bytes = tag
        .write_to_bytes()
        .map_err(|error| format!("Could not serialize {}: {error}", output.display()))?;
    TagFile::read_from_bytes(&bytes)
        .map_err(|error| format!("Could not reopen {}: {error}", output.display()))?;
    Ok(())
}

pub(super) fn convert_h3_player_responses_to_reach_companions(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    context: &mut ConversionContext<'_>,
) -> Result<HashSet<String>, String> {
    if context.source_game != "halo3_mcc" || context.target_game != "haloreach_mcc" {
        return Ok(HashSet::new());
    }
    let Some(responses) = field_by_key(source, "player responses").and_then(|f| f.as_block())
    else {
        return Ok(HashSet::new());
    };
    if responses.is_empty() {
        return Ok(HashSet::new());
    }

    let camera_fields = [
        "impulse duration",
        "rotation",
        "pushback",
        "jitter",
        "shake duration",
        "random translation",
        "random rotation",
        "wobble function period",
        "wobble weight",
        "wobble function",
    ];
    let has_camera = camera_fields
        .iter()
        .any(|key| field_by_key(source, key).is_some_and(field_has_meaningful_value));

    let mut camera = if has_camera {
        let mut draft = create_companion_tag("camera", "__camera_shake", "camera_shake", context)?;
        populate_camera_shake(source, &mut draft.tag, context);
        record_companion_layout(&draft, context);
        Some(draft)
    } else {
        None
    };

    let mut response = create_companion_tag(
        "response",
        "__damage_response",
        "damage_response_definition",
        context,
    )?;
    let classes_ordinal = field_ordinal_by_key(response.tag.root(), "classes")
        .ok_or_else(|| "Reach damage response companion has no classes block".to_owned())?;
    let mut response_root = response.tag.root_mut();
    let mut classes_field = response_root
        .field_at_mut(classes_ordinal)
        .ok_or_else(|| "Reach damage response classes field is unavailable".to_owned())?;
    let mut classes = classes_field
        .as_block_mut()
        .ok_or_else(|| "Reach damage response classes is not a block".to_owned())?;
    classes.clear();
    let count = responses
        .len()
        .min(classes.definition().max_count() as usize);
    for index in 0..count {
        let class_index = classes.add_element();
        let Some(class) = classes.element_mut(class_index) else {
            return Err(format!(
                "Could not allocate Reach player response class {index}"
            ));
        };
        initialize_block_index_defaults(class);
        let Some(mut class) = classes.element_mut(class_index) else {
            return Err(format!(
                "Could not reopen Reach player response class {index}"
            ));
        };
        let source_response = responses.element(index).unwrap();
        populate_response_class(source_response, &mut class, index, has_camera, context)?;
    }
    if responses.len() > count {
        context.report.truncated += responses.len() - count;
        context.report.issues.push(ConversionIssue {
            kind: ConversionIssueKind::Truncated,
            path: "player responses".to_owned(),
            message: format!(
                "Reach damage response supports {count} classes; omitted {} H3 response(s)",
                responses.len() - count
            ),
        });
    }
    drop(classes);
    drop(classes_field);
    drop(response_root);
    record_companion_layout(&response, context);

    set_companion_reference(target, "damage response", "response", context)?;
    context.companion_tags.push(response);
    if let Some(camera) = camera.take() {
        context.companion_tags.push(camera);
    }
    context.report.issues.push(ConversionIssue {
        kind: ConversionIssueKind::Warning,
        path: "player responses".to_owned(),
        message: "Generated Reach damage-response, rumble, and camera-shake companion tags from H3 inline player response data".to_owned(),
    });

    Ok(HashSet::from([
        "player responses".to_owned(),
        "damage response".to_owned(),
        "impulse duration".to_owned(),
        "fade function".to_owned(),
        "rotation".to_owned(),
        "pushback".to_owned(),
        "jitter".to_owned(),
        "shake duration".to_owned(),
        "falloff function".to_owned(),
        "random translation".to_owned(),
        "random rotation".to_owned(),
        "wobble function".to_owned(),
        "wobble function period".to_owned(),
        "wobble weight".to_owned(),
    ]))
}

fn populate_response_class(
    source: TagStruct<'_>,
    target: &mut TagStructMut<'_>,
    index: usize,
    has_camera: bool,
    context: &mut ConversionContext<'_>,
) -> Result<(), String> {
    copy_named_field(source, "response type", target, "type", context);
    if let (Some(screen), Some(ordinal)) = (
        struct_field_by_key(source, "screen flash"),
        field_ordinal_by_key(target.as_ref(), "directional flash"),
    ) && let Some(mut field) = target.field_at_mut(ordinal)
        && let Some(mut flash) = field.as_struct_mut()
    {
        for (source_key, target_key) in [
            ("duration", "duration"),
            ("fade function", "fade function"),
            ("maximum intensity", "center alpha"),
            ("maximum intensity", "offscreen alpha"),
            ("maximum intensity", "inner alpha"),
            ("maximum intensity", "outer alpha"),
            ("color", "flash color"),
            ("color", "arrow color"),
        ] {
            copy_named_field(screen, source_key, &mut flash, target_key, context);
        }
    }

    if let Some(rumble_source) = struct_field_by_key(source, "rumble")
        && struct_has_meaningful_value(rumble_source)
    {
        let key = format!("rumble_{index}");
        let mut rumble =
            create_companion_tag(&key, &format!("__rumble_{index}"), "rumble", context)?;
        if let Some(ordinal) = field_ordinal_by_key(rumble.tag.root(), "rumble")
            && let Some(mut field) = rumble.tag.root_mut().field_at_mut(ordinal)
            && let Some(rumble_target) = field.as_struct_mut()
        {
            convert_struct(
                rumble_source,
                rumble_target,
                "player responses/rumble",
                false,
                context,
            );
        }
        set_companion_reference(target, "rumble", &key, context)?;
        record_companion_layout(&rumble, context);
        context.companion_tags.push(rumble);
    }
    if has_camera {
        set_companion_reference(target, "camera shake", "camera", context)?;
    }

    if let Some(sound) = struct_field_by_key(source, "sound effect")
        && struct_has_meaningful_value(sound)
        && let Some(ordinal) = field_ordinal_by_key(target.as_ref(), "global sound effect")
        && let Some(mut field) = target.field_at_mut(ordinal)
        && let Some(mut block) = field.as_block_mut()
    {
        block.clear();
        let element_index = block.add_element();
        if let Some(element) = block.element_mut(element_index) {
            initialize_block_index_defaults(element);
        }
        if let Some(mut element) = block.element_mut(element_index) {
            copy_named_field(sound, "effect name", &mut element, "effect name", context);
            if let Some(TagFieldData::Real(duration)) =
                field_by_key(sound, "duration").and_then(|field| field.value())
            {
                set_constant_mapping(&mut element, "scale => duration", duration)?;
            }
        }
    }
    Ok(())
}

fn populate_camera_shake(
    source: TagStruct<'_>,
    target: &mut TagFile,
    context: &mut ConversionContext<'_>,
) {
    let mut root = target.root_mut();
    for (section, fields) in [
        (
            "camera impulse",
            &["impulse duration", "rotation", "pushback", "jitter"][..],
        ),
        (
            "camera shake",
            &[
                "shake duration",
                "random translation",
                "random rotation",
                "wobble function period",
                "wobble weight",
                "wobble function",
            ][..],
        ),
    ] {
        let Some(ordinal) = field_ordinal_by_key(root.as_ref(), section) else {
            continue;
        };
        let Some(mut field) = root.field_at_mut(ordinal) else {
            continue;
        };
        let Some(mut target_section) = field.as_struct_mut() else {
            continue;
        };
        for key in fields {
            copy_named_field(source, key, &mut target_section, key, context);
        }
    }
    context.report.issues.push(ConversionIssue {
        kind: ConversionIssueKind::Warning,
        path: "camera shake/mapping".to_owned(),
        message: "H3 fade and falloff enums have no byte-identical Reach mapping-function representation; scalar camera data was preserved and Reach mapping defaults were retained".to_owned(),
    });
}

fn copy_named_field(
    source: TagStruct<'_>,
    source_key: &str,
    target: &mut TagStructMut<'_>,
    target_key: &str,
    context: &mut ConversionContext<'_>,
) {
    let (Some(source_field), Some(ordinal)) = (
        field_by_key(source, source_key),
        field_ordinal_by_key(target.as_ref(), target_key),
    ) else {
        return;
    };
    if let Some(target_field) = target.field_at_mut(ordinal) {
        convert_field(source_field, target_field, target_key, false, context);
    }
}

fn set_constant_mapping(
    target: &mut TagStructMut<'_>,
    key: &str,
    value: f32,
) -> Result<(), String> {
    let Some(ordinal) = field_ordinal_by_key(target.as_ref(), key) else {
        return Ok(());
    };
    let Some(mut field) = target.field_at_mut(ordinal) else {
        return Ok(());
    };
    let Some(mut mapping) = field.as_struct_mut() else {
        return Ok(());
    };
    let Some(data_ordinal) = field_ordinal_by_key(mapping.as_ref(), "data") else {
        return Ok(());
    };
    let Some(mut data) = mapping.field_at_mut(data_ordinal) else {
        return Ok(());
    };
    let mut bytes = vec![0u8; 32];
    bytes[0] = FunctionType::Constant as u8;
    bytes[1] = FunctionFlags::GPU;
    bytes[4..8].copy_from_slice(&value.to_le_bytes());
    bytes[8..12].copy_from_slice(&value.to_le_bytes());
    data.set(TagFieldData::Data(bytes))
        .map_err(|error| format!("Could not set companion mapping function: {error:?}"))
}

fn set_companion_reference(
    target: &mut TagStructMut<'_>,
    target_key: &str,
    companion_key: &str,
    context: &ConversionContext<'_>,
) -> Result<(), String> {
    let group_name = match companion_key {
        "response" => "damage_response_definition",
        "camera" => "camera_shake",
        key if key.starts_with("rumble_") => "rumble",
        _ => return Err(format!("Unknown companion reference {companion_key}")),
    };
    let group = context
        .target_groups
        .by_name
        .get(group_name)
        .copied()
        .ok_or_else(|| format!("{} has no {group_name} group", context.target_game))?;
    let ordinal = field_ordinal_by_key(target.as_ref(), target_key)
        .ok_or_else(|| format!("Target companion layout has no {target_key} field"))?;
    let mut field = target
        .field_at_mut(ordinal)
        .ok_or_else(|| format!("Target companion field {target_key} is unavailable"))?;
    field
        .set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((
                group,
                format!("{COMPANION_REFERENCE_PREFIX}{companion_key}"),
            )),
        }))
        .map_err(|error| format!("Could not set {target_key} companion reference: {error:?}"))
}

fn record_companion_layout(draft: &CompanionTagDraft, context: &mut ConversionContext<'_>) {
    context.report.issues.push(ConversionIssue {
        kind: ConversionIssueKind::Warning,
        path: format!("companion/{}", draft.group_name),
        message: match draft.native_layout_template.as_ref() {
            Some(path) => format!("Used native companion layout template {}", path.display()),
            None => "Used generated companion layout; native compatibility remains unverified"
                .to_owned(),
        },
    });
}
