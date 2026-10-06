//! Render-method lookup, cache access, and edit-target construction.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing, and the commands that apply edits, belong elsewhere.

use super::*;

pub(in crate::app) fn render_method_flags_mask(render_method: &RenderMethod) -> u64 {
    let mut mask = 0u64;
    for flag in render_method.flags.get() {
        let bit = match flag {
            GlobalRenderMethodFlags::DontFogMe => 0,
            GlobalRenderMethodFlags::UseCustomSetting => 1,
            GlobalRenderMethodFlags::CalculateZCamera => 2,
        };
        mask |= 1u64 << bit;
    }
    mask
}

pub(in crate::app) fn cached_render_method_definition(
    source: &TagSource,
    reference: &str,
    cache: &mut HashMap<String, Option<Arc<RenderMethodDefinition>>>,
) -> Option<Arc<RenderMethodDefinition>> {
    if reference.is_empty() {
        return None;
    }
    let key = format!("rmdf:{reference}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let parsed =
        load_referenced_tag_from_source(source, reference, "render_method_definition", b"rmdf")
            .ok()
            .and_then(|tag| {
                // Custom kits recompile these tags, and blam-tags' enum
                // resolver panics on a schema name it has no variant for. A
                // definition that cannot be read must degrade to "none" — the
                // raw-field fallback — not take the process down mid-frame.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    RenderMethodDefinition::from_tag(&tag).ok()
                }))
                .ok()
                .flatten()
                .map(Arc::new)
            });
    cache.insert(key, parsed.clone());
    parsed
}

pub(in crate::app) fn cached_render_method_option(
    source: &TagSource,
    reference: &str,
    cache: &mut HashMap<String, Option<Arc<RenderMethodOption>>>,
) -> Option<Arc<RenderMethodOption>> {
    if reference.is_empty() {
        return None;
    }
    let key = format!("rmop:{reference}");
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let parsed =
        load_referenced_tag_from_source(source, reference, "render_method_option", b"rmop")
            .ok()
            .and_then(|tag| {
                // The likeliest panic in the whole shader chain: a custom
                // rmop's `source extern` / `parameter type` naming something
                // blam-tags has no variant for. Degrade to "section omitted".
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    RenderMethodOption::from_tag(&tag).ok()
                }))
                .ok()
                .flatten()
                .map(Arc::new)
            });
    cache.insert(key, parsed.clone());
    parsed
}

pub(in crate::app) fn render_method_edit_prefix(tag: &TagFile) -> String {
    if tag.root().field("render_method").is_some() {
        "render_method".to_owned()
    } else {
        String::new()
    }
}

pub(in crate::app) fn render_method_existing_field_path(
    tag: &TagFile,
    edit_prefix: &str,
    candidates: &[&str],
) -> String {
    for candidate in candidates {
        let path = append_field_path(edit_prefix, candidate);
        if tag.root().field_path(&path).is_some() {
            return path;
        }
    }
    candidates
        .first()
        .map(|candidate| append_field_path(edit_prefix, candidate))
        .unwrap_or_default()
}

/// One global material type a shader carries: its label, current value and
/// the field path that edits it.
pub(in crate::app) struct ShaderMaterialName {
    pub(in crate::app) label: String,
    pub(in crate::app) value: String,
    pub(in crate::app) edit_path: String,
}

/// The shader's global material types: the root's `material name` string-id
/// (`material name 0`…`3` on a terrain shader, one per channel, up to 7 on
/// Reach's mux shader).
///
/// They live on the shader's own root struct, not in its render method. This
/// used to read `render_method/global material type`, which no Halo 3, ODST,
/// Reach, Halo 4 or H2A shader has, so the row always showed
/// `default_material` and an edit failed to resolve.
pub(in crate::app) fn read_shader_material_names(tag: &TagFile) -> Vec<ShaderMaterialName> {
    let root = tag.root();
    root.fields()
        .filter(|field| field.clean_name().starts_with("material name"))
        .filter_map(|field| {
            let value = match field.value()? {
                TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => id.string,
                _ => return None,
            };
            Some(ShaderMaterialName {
                label: field.clean_name().into_owned(),
                value: if value.is_empty() {
                    "default_material".to_owned()
                } else {
                    value
                },
                edit_path: append_field_path_for("", &field),
            })
        })
        .collect()
}

/// Build the tag field paths for the `animated_index`-th animated
/// parameter of `param_index`-th render-method parameter. Relies on the
/// parsed `parameters` / `animated parameters` Vecs being 1:1 with their
/// schema blocks (both `from_struct` readers are infallible, so no
/// elements are skipped).
pub(in crate::app) fn animated_param_paths(
    prefix: &str,
    param_index: usize,
    animated_index: usize,
) -> FunctionEditPaths {
    let block_path = append_field_path(
        prefix,
        &format!("parameters[{param_index}]/animated parameters"),
    );
    let base = format!("{block_path}[{animated_index}]");
    FunctionEditPaths {
        data: FunctionDataStorage::DataField(append_field_path(&base, "function/data")),
        parameter_type: append_field_path(&base, "type"),
        input_name: append_field_path(&base, "input name"),
        range_name: append_field_path(&base, "range name"),
        time_period: append_field_path(&base, "time period"),
        block_path,
        block_index: animated_index,
    }
}

pub(in crate::app) fn existing_shader_function_target(
    edit_prefix: &str,
    param_index: usize,
    output_type_index: i32,
) -> ShaderFunctionCreateTarget {
    ShaderFunctionCreateTarget::ExistingParameter {
        animated_block_path: append_field_path(
            edit_prefix,
            &format!("parameters[{param_index}]/animated parameters"),
        ),
        output_type_index,
    }
}

pub(in crate::app) fn new_shader_function_target(
    edit_prefix: &str,
    parameter_name: &str,
    parameter_type_index: i32,
    output_type_index: i32,
) -> ShaderFunctionCreateTarget {
    ShaderFunctionCreateTarget::NewParameter {
        parameters_block_path: append_field_path(edit_prefix, "parameters"),
        parameter_name: parameter_name.to_owned(),
        parameter_type_index,
        output_type_index,
    }
}

pub(in crate::app) fn shader_parameter_type_index(parameter: &RenderMethodOptionParameter) -> i32 {
    // Canonical schema index, consumed only as an internal selector by
    // shader_parameter_type_initial_field's write (which routes through the
    // edit system's declaration-index == wire assumption). Avoids exposing
    // the raw wire value.
    parameter
        .parameter_type
        .map(|kind| kind.get() as i32)
        .unwrap_or(RenderMethodParameterType::Real as i32)
}

pub(in crate::app) fn shader_parameter_type_initial_field(
    parameter_type_index: i32,
) -> ShaderParamInitialField {
    ShaderParamInitialField {
        field: "parameter type".to_owned(),
        input: parameter_type_index.to_string(),
    }
}

pub(in crate::app) fn shader_function_action(
    target: &ShaderFunctionCreateTarget,
    initial_function_hex: String,
) -> ShaderContextAction {
    match target {
        ShaderFunctionCreateTarget::ExistingParameter {
            animated_block_path,
            output_type_index,
        } => ShaderContextAction::AnimatedParameter(ShaderOp {
            animated_block_path: animated_block_path.clone(),
            output_type_index: *output_type_index,
            initial_function_hex,
        }),
        ShaderFunctionCreateTarget::NewParameter {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            output_type_index,
        } => ShaderContextAction::ParameterOp(ShaderParamOp {
            parameters_block_path: parameters_block_path.clone(),
            parameter_name: parameter_name.clone(),
            initial_fields: vec![shader_parameter_type_initial_field(*parameter_type_index)],
            animated_parameters: vec![ShaderParamInitialAnimated {
                output_type_index: *output_type_index,
                initial_function_hex,
            }],
        }),
    }
}

pub(in crate::app) fn push_shader_context_action(
    edit: &mut FieldEditContext<'_>,
    action: &ShaderContextAction,
) {
    edit.push_ops(shader_context_action_ops(action));
}

/// The edits `action` makes.
pub(in crate::app) fn shader_context_action_ops(action: &ShaderContextAction) -> DeferredOps {
    let mut ops = DeferredOps::default();
    match action {
        ShaderContextAction::AnimatedParameter(op) => ops.shader_ops.push(op.clone()),
        ShaderContextAction::FieldEdits(edits) => ops.pending.extend(edits.iter().cloned()),
        ShaderContextAction::ParameterOp(op) => ops.shader_param_ops.push(op.clone()),
        ShaderContextAction::H2ParameterOp(op) => ops.h2_shader_param_ops.push(op.clone()),
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shader model, editing, and thumbnail unit tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// End to end against the shipped H3 tags: create a bool parameter the way the
    /// grid's "Override Default" does, and read back what landed.
    #[test]
    fn enabling_a_bool_shader_parameter_writes_it() {
        let path = std::path::Path::new(crate::core::test_kits::tag_path(
            "halo3_mcc",
            "objects/characters/brute/shaders/armor_lights.shader",
        ));
        if !path.exists() {
            eprintln!("skipping: no H3 editing kit");
            return;
        }
        let bytes = std::fs::read(path).expect("read shader");
        let mut tag = blam_tags::TagFile::read_from_bytes(&bytes).expect("parse shader");
        let prefix = render_method_edit_prefix(&tag);
        let block = append_field_path(&prefix, "parameters");

        for name in [
            "no_dynamic_lights",
            "use_material_texture",
            "order3_area_specular",
        ] {
            let op = ShaderParamOp {
                parameters_block_path: block.clone(),
                parameter_name: name.to_owned(),
                initial_fields: vec![
                    ShaderParamInitialField {
                        field: "parameter type".to_owned(),
                        // `bool` in the parameter-type enum.
                        input: "4".to_owned(),
                    },
                    ShaderParamInitialField {
                        field: "int/bool".to_owned(),
                        input: "1".to_owned(),
                    },
                ],
                animated_parameters: Vec::new(),
            };
            let message = apply_one_shader_param_op(&mut tag, &op)
                .unwrap_or_else(|error| panic!("enabling {name} failed: {error}"));
            eprintln!("{message}");
        }

        // The values have to survive a save, not just the in-memory write.
        let saved = tag.write_to_bytes().expect("serialize");
        let reopened = blam_tags::TagFile::read_from_bytes(&saved).expect("reparse");
        let root = reopened.root();
        let parameters = root
            .field_path(&block)
            .and_then(|field| field.as_block())
            .expect("parameters block");
        let mut enabled = Vec::new();
        for index in 0..parameters.len() {
            let Some(element) = parameters.element(index) else {
                continue;
            };
            let name = element
                .field("parameter name")
                .and_then(|field| field.value())
                .and_then(|value| match value {
                    blam_tags::TagFieldData::StringId(s)
                    | blam_tags::TagFieldData::OldStringId(s) => Some(s.string.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let value = element
                .field_path("int\\bool")
                .and_then(|field| field.value())
                .and_then(|value| match value {
                    blam_tags::TagFieldData::LongInteger(v) => Some(v),
                    _ => None,
                });
            if value == Some(1) {
                enabled.push(name);
            }
        }
        for expected in [
            "no_dynamic_lights",
            "use_material_texture",
            "order3_area_specular",
        ] {
            assert!(
                enabled.iter().any(|name| name == expected),
                "{expected} is not enabled in the saved tag; enabled: {enabled:?}"
            );
        }
    }

    /// The two structural references — `definition` and `shader template` — were
    /// drawn as painted text with editing switched off, so a shader's render
    /// method definition could be read but never changed, unlike Foundation's
    /// expert mode. Guards the part that is easy to get wrong: the tag field paths
    /// the editor commits through. `definition` sits on the render_method block,
    /// but the template reference hangs off `postprocess[0]`, not the root.
    ///
    /// Ignored by default — it needs a loose Halo 3 tag tree.
    ///
    /// Run with:
    ///   H3_TAGS=~/Halo/halo3_mcc/tags \
    ///     cargo test structural_shader_references -- --ignored --nocapture
    #[test]
    #[ignore = "requires a loose Halo 3 tag tree; set H3_TAGS"]
    fn structural_shader_references_resolve_to_editable_reference_fields() {
        let Ok(root) = std::env::var("H3_TAGS") else {
            eprintln!("skipping: set H3_TAGS to a loose Halo 3 tags directory");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let source = crate::core::source::TagSource::LooseFolder {
            root: root.clone(),
            game: Some(GameId::Halo3),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };

        // Any shader that resolves its render-method chain will do; walk until one
        // builds a grid, so this does not hinge on one hand-picked tag.
        let mut rmdf_cache = std::collections::HashMap::new();
        let mut rmop_cache = std::collections::HashMap::new();
        let mut checked = 0usize;
        for entry in walkdir_shaders(&root).into_iter().take(400) {
            let Ok(tag) = blam_tags::TagFile::read(&entry) else {
                continue;
            };
            let Some(model) = super::build_shader_editor_model(
                &tag,
                u32::from_be_bytes(*b"rmsh"),
                Some(&source),
                &mut rmdf_cache,
                &mut rmop_cache,
            ) else {
                continue;
            };

            assert!(
                !model.definition_edit_path.is_empty(),
                "{}: no edit path for `definition`",
                entry.display()
            );
            let field = tag
                .root()
                .field_path(&model.definition_edit_path)
                .unwrap_or_else(|| panic!("{}: definition path does not resolve", entry.display()));
            let Some(blam_tags::TagFieldData::TagReference(reference)) = field.value() else {
                panic!("{}: `definition` is not a tag reference", entry.display());
            };
            assert_eq!(
                reference.group_tag_and_name.map(|(_, name)| name),
                Some(model.definition_path.clone()),
                "{}: the edit path addresses a different reference than the row shows",
                entry.display()
            );

            // The template only exists once a postprocess block does.
            if let Some(template) = model.shader_template_path.as_deref() {
                assert!(
                    !model.shader_template_edit_path.is_empty(),
                    "{}: no edit path for `shader template`",
                    entry.display()
                );
                let field = tag
                    .root()
                    .field_path(&model.shader_template_edit_path)
                    .unwrap_or_else(|| {
                        panic!("{}: shader template path does not resolve", entry.display())
                    });
                let Some(blam_tags::TagFieldData::TagReference(reference)) = field.value() else {
                    panic!(
                        "{}: `shader template` is not a tag reference",
                        entry.display()
                    );
                };
                assert_eq!(
                    reference.group_tag_and_name.map(|(_, name)| name),
                    Some(template.to_owned()),
                    "{}: template edit path addresses a different reference",
                    entry.display()
                );
            }
            checked += 1;
            if checked >= 5 {
                break;
            }
        }
        assert!(
            checked > 0,
            "no shader in this tag tree built a grid to check"
        );
        println!("checked {checked} shader(s)");
    }

    fn walkdir_shaders(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "shader") {
                    out.push(path);
                    if out.len() > 400 {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// The row being drawn is only half of "editable" — the commit has to land.
    /// This drives the same path the widget pushes (`apply_pending_edits` at the
    /// resolved edit path) and checks the reference actually changes on the tag.
    ///
    /// Ignored by default — it needs a loose Halo 3 tag tree.
    ///
    /// Run with:
    ///   H3_TAGS=~/Halo/halo3_mcc/tags \
    ///     cargo test committing_a_structural_reference -- --ignored --nocapture
    #[test]
    #[ignore = "requires a loose Halo 3 tag tree; set H3_TAGS"]
    fn committing_a_structural_reference_rewrites_the_tag() {
        let Ok(root) = std::env::var("H3_TAGS") else {
            eprintln!("skipping: set H3_TAGS to a loose Halo 3 tags directory");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let source = crate::core::source::TagSource::LooseFolder {
            root: root.clone(),
            game: Some(GameId::Halo3),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let mut rmdf_cache = std::collections::HashMap::new();
        let mut rmop_cache = std::collections::HashMap::new();

        for entry in walkdir_shaders(&root).into_iter().take(400) {
            let Ok(mut tag) = blam_tags::TagFile::read(&entry) else {
                continue;
            };
            let Some(model) = super::build_shader_editor_model(
                &tag,
                u32::from_be_bytes(*b"rmsh"),
                Some(&source),
                &mut rmdf_cache,
                &mut rmop_cache,
            ) else {
                continue;
            };
            if model.definition_edit_path.is_empty() {
                continue;
            }

            let before = model.definition_path.clone();
            let after = "shaders\\custom_definition";
            assert_ne!(before, after, "pick a value that is actually a change");

            let mut dirty = crate::app::Dirty::default();
            let applied = crate::core::document::apply::apply_pending_edits(
                &mut tag,
                vec![crate::app::PendingFieldEdit {
                    path: model.definition_edit_path.clone(),
                    input: format!("{after}.render_method_definition"),
                }],
                &mut dirty,
            );
            assert!(
                applied
                    .outcomes
                    .iter()
                    .all(|outcome| outcome.result.is_ok()),
                "{}: commit failed: {:?}",
                entry.display(),
                applied.status
            );
            assert!(dirty.is_set(), "a committed edit marks the document dirty");

            let field = tag
                .root()
                .field_path(&model.definition_edit_path)
                .expect("definition still resolves");
            let Some(blam_tags::TagFieldData::TagReference(reference)) = field.value() else {
                panic!("definition stopped being a tag reference");
            };
            assert_eq!(
                reference
                    .group_tag_and_name
                    .map(|(_, name)| name)
                    .as_deref(),
                Some(after),
                "{}: the reference did not take the committed value",
                entry.display()
            );
            println!("{}: {before:?} -> {after:?}", entry.display());
            return;
        }
        panic!("no shader in this tag tree built a grid to commit against");
    }
}
