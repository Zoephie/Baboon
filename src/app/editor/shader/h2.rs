//! Classic Halo 2 shader and template model construction.
//! It owns shader-specific models, edits, and presentation helpers; generic field editing, and the commands that apply edits, belong elsewhere.

use super::*;

pub(in crate::app) fn build_h2ek_shader_editor_model(
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    templates: &mut H2TemplateCache,
) -> Option<ShaderEditorModel> {
    if tag.classic_engine()? != blam_tags::classic::ClassicEngine::Halo2V4 {
        return None;
    }
    if !is_h2ek_shader_family_group(entry.group_tag) {
        return None;
    }

    let root = tag.root();
    let template_tag = h2_load_shader_template(source, root, templates);
    let template_root = template_tag.as_ref().map(|template| template.root());
    let mut top_rows = Vec::new();
    h2_push_direct_field_row(root, "template", "", names, &mut top_rows);
    let mut sections = Vec::new();
    h2_push_section(
        &mut sections,
        "STANDARD_PARAMETERS",
        h2_standard_parameter_rows(root, template_root, names),
    );
    sections.extend(h2_parameter_sections(root, template_root, names));
    h2_push_section(&mut sections, "RAW PARAMETERS", h2_raw_parameter_rows(root));

    if sections.is_empty() {
        return None;
    }

    Some(ShaderEditorModel {
        has_material_row: false,
        materials: Vec::new(),
        definition_path: String::new(),
        // Halo 2 shows its template through its own row, which already carries
        // an edit; these two belong to the Halo 3-era render_method grid.
        definition_edit_path: String::new(),
        shader_template_edit_path: String::new(),
        shader_template_path: None,
        categories: Vec::new(),
        unused_parameters: template_root
            .map(|template| h2_unused_parameters(root, template, names))
            .unwrap_or_default(),
        top_rows,
        sections,
        atmosphere_flags: ShaderFlagsRow {
            label: String::new(),
            path: String::new(),
            raw: 0,
            options: Vec::new(),
        },
        custom_fog_setting_index: empty_shader_grid_row(),
        sort_layer: empty_shader_grid_row(),
    })
}

fn h2_load_shader_template(
    source: Option<&TagSource>,
    root: TagStruct<'_>,
    templates: &mut H2TemplateCache,
) -> Option<Arc<TagFile>> {
    let source = source?;
    let reference = h2_shader_template_reference(root)?;
    templates.get(source, &reference)
}

/// The `.shader_template`s the H2 shader grid has read, by reference.
///
/// The grid is rebuilt every frame, and it used to read and parse the
/// template off disk each time. An entry is reused while the file's size and
/// modified time are unchanged, so a template saved elsewhere is picked up
/// on the next frame; sources that cannot change underneath (caches, paks)
/// keep theirs for the session. A failed read is cached the same way, so a
/// missing template is not retried every frame, but is as soon as it appears.
#[derive(Default)]
pub(in crate::app) struct H2TemplateCache {
    entries: HashMap<String, CachedTemplate>,
    #[cfg(test)]
    pub(in crate::app) loads: usize,
}

struct CachedTemplate {
    stamp: Option<(u64, std::time::SystemTime)>,
    tag: Option<Arc<TagFile>>,
}

impl H2TemplateCache {
    fn get(&mut self, source: &TagSource, reference: &str) -> Option<Arc<TagFile>> {
        let stamp = template_stamp(source, reference);
        if let Some(cached) = self.entries.get(reference)
            && cached.stamp == stamp
        {
            return cached.tag.clone();
        }
        #[cfg(test)]
        {
            self.loads += 1;
        }
        let tag = load_referenced_tag_from_source(source, reference, "shader_template", b"stem")
            .ok()
            .map(Arc::new);
        self.entries.insert(
            reference.to_owned(),
            CachedTemplate {
                stamp,
                tag: tag.clone(),
            },
        );
        tag
    }
}

/// Size and modified time of a loose template, the one kind of source whose
/// files can change during a session. `None` for everything else, and for a
/// loose file that is not there.
fn template_stamp(source: &TagSource, reference: &str) -> Option<(u64, std::time::SystemTime)> {
    let root = match source {
        TagSource::LooseFolder { root, .. } => root.clone(),
        TagSource::SingleFile { path } => blam_tags::paths::derive_tags_root(path)
            .or_else(|| path.parent().map(Path::to_path_buf))?,
        TagSource::MonolithicCache { .. } | TagSource::IoStoreContainerSet { .. } => return None,
    };
    let metadata = std::fs::metadata(resolve_tag_path(&root, reference, "shader_template")).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

fn h2_shader_template_reference(root: TagStruct<'_>) -> Option<String> {
    let value = root.field("template")?.value()?;
    let TagFieldData::TagReference(reference) = value else {
        return None;
    };
    let (group_tag, path) = reference.group_tag_and_name.as_ref()?;
    if *group_tag != u32::from_be_bytes(*b"stem") || path.is_empty() {
        return None;
    }
    Some(h2_normalize_shader_template_reference(path))
}

pub(super) fn h2_normalize_shader_template_reference(path: &str) -> String {
    let mut normalized = path.trim_end_matches('\0').to_owned();
    let lower = normalized.to_ascii_lowercase();
    for suffix in [".shader_template", ".stem"] {
        if lower.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }
    normalized
}

fn h2_push_section(sections: &mut Vec<ShaderEditorSection>, title: &str, rows: Vec<ShaderGridRow>) {
    if rows.is_empty() {
        return;
    }
    sections.push(ShaderEditorSection {
        title: title.to_owned(),
        option_name: String::new(),
        rows,
    });
}

fn h2_standard_parameter_rows(
    root: TagStruct<'_>,
    template: Option<TagStruct<'_>>,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    let baseline = TagFile::new(locate_definitions_root().join("halo2_mcc/shader.json")).ok();
    for field_name in [
        "material name",
        "flags",
        "Added depth bias offset",
        "Added depth bias slope scale",
        "specular type",
        "lightmap type",
        "lightmap specular brightness",
        "lightmap ambient bias",
        "shader LOD bias",
    ] {
        h2_push_direct_field_row(root, field_name, "", names, &mut rows);
        if let Some(row) = rows.last_mut().filter(|row| {
            row.edit
                .as_ref()
                .is_some_and(|edit| edit.path == escape_field_path_segment(field_name))
        }) {
            let default = if field_name == "material name" {
                template
                    .and_then(|template| template.field("default material name"))
                    .and_then(|field| field.value())
                    .or_else(|| {
                        baseline
                            .as_ref()
                            .and_then(|tag| tag.root().field(field_name))
                            .and_then(|field| field.value())
                    })
            } else {
                baseline
                    .as_ref()
                    .and_then(|tag| tag.root().field(field_name))
                    .and_then(|field| field.value())
            };
            if let Some(value) = default {
                let current = trim_formatted_value(&format_value(names, &value, false));
                let text = match row.edit.as_ref().map(|edit| &edit.kind) {
                    Some(ShaderRowEditKind::Enum(options)) => baseline
                        .as_ref()
                        .and_then(|tag| tag.root().read_int_any(field_name))
                        .and_then(|index| usize::try_from(index).ok())
                        .and_then(|index| options.get(index))
                        .cloned()
                        .unwrap_or(current),
                    Some(ShaderRowEditKind::Scalar | ShaderRowEditKind::Int) => {
                        format!("value: {current}")
                    }
                    _ => current,
                };
                row.default_cell = Some(ShaderGridCell {
                    text,
                    value_kind: "default",
                    color: None,
                });
            }
        }
    }
    if let Some(runtime) = root
        .field("runtime properties")
        .and_then(|field| field.as_block())
    {
        if let Some(element) = runtime.element(0) {
            for field in element.fields() {
                let path = format!(
                    "runtime properties[0]/{}",
                    escape_field_path_segment(field.name())
                );
                if let Some(row) = h2_shader_row_from_field(element, field, &path, "", names) {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn h2_push_direct_field_row(
    tag_struct: TagStruct<'_>,
    field_name: &str,
    path_prefix: &str,
    names: &TagNameIndex,
    rows: &mut Vec<ShaderGridRow>,
) {
    let Some(field) = tag_struct.field(field_name) else {
        return;
    };
    let path = if path_prefix.is_empty() {
        escape_field_path_segment(field_name)
    } else {
        append_field_path(path_prefix, &escape_field_path_segment(field_name))
    };
    if let Some(mut row) = h2_shader_row_from_field(tag_struct, field, &path, "", names) {
        if path_prefix.is_empty() {
            row.label = h2_standard_field_label(field_name).to_owned();
            h2_apply_standard_field_widget(field_name, &mut row);
        }
        rows.push(row);
    }
}

fn h2_apply_standard_field_widget(field_name: &str, row: &mut ShaderGridRow) {
    let Some(edit) = row.edit.as_mut() else {
        return;
    };
    match field_name {
        "flags" => {
            edit.kind = ShaderRowEditKind::Flags(vec![
                "water".to_owned(),
                "sort first".to_owned(),
                "no active camo".to_owned(),
            ]);
            row.value_cell.text = edit.current.clone();
            row.parameter_type = Some("flags".to_owned());
        }
        "specular type" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "none".to_owned(),
                "default shiny".to_owned(),
                "dull".to_owned(),
            ]);
            row.value_cell.text =
                h2_enum_display_value(&edit.current, &["none", "default shiny", "dull"]);
        }
        "lightmap type" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "diffuse".to_owned(),
                "default specular".to_owned(),
                "dull specular".to_owned(),
                "shiny specular".to_owned(),
            ]);
            row.value_cell.text = h2_enum_display_value(
                &edit.current,
                &[
                    "diffuse",
                    "default specular",
                    "dull specular",
                    "shiny specular",
                ],
            );
        }
        "shader LOD bias" => {
            edit.kind = ShaderRowEditKind::Enum(vec![
                "none".to_owned(),
                "4x size".to_owned(),
                "2x size".to_owned(),
                "1/2 size".to_owned(),
                "1/4 size".to_owned(),
                "never".to_owned(),
                "cinematic".to_owned(),
                "lowest".to_owned(),
            ]);
            row.value_cell.text = h2_enum_display_value(
                &edit.current,
                &[
                    "none",
                    "4x size",
                    "2x size",
                    "1/2 size",
                    "1/4 size",
                    "never",
                    "cinematic",
                    "lowest",
                ],
            );
        }
        _ => {}
    }
}

fn h2_enum_display_value(current: &str, options: &[&str]) -> String {
    current
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|index| options.get(index).copied())
        .unwrap_or(current)
        .to_owned()
}

fn h2_standard_field_label(field_name: &str) -> &str {
    match field_name {
        "material name" => "material_name",
        "Added depth bias offset" => "depth_bias_offset",
        "Added depth bias slope scale" => "depth_bias_slope_scale",
        "specular type" => "dynamic_light_specular_type",
        "lightmap type" => "lightmap_type",
        "lightmap specular brightness" => "lightmap_specular_brightness",
        "lightmap ambient bias" => "lightmap_ambient_bias",
        "shader LOD bias" => "shader_lod_bias",
        other => other,
    }
}

fn h2_fallback_parameter_section_title(root: TagStruct<'_>, names: &TagNameIndex) -> String {
    let Some(value) = root.field("template").and_then(|field| field.value()) else {
        return "PARAMETERS".to_owned();
    };
    let formatted = trim_formatted_value(&format_value(names, &value, false));
    let normalized = formatted.replace('\\', "/").to_ascii_lowercase();
    let Some(pos) = normalized.find("shader_templates/") else {
        return "PARAMETERS".to_owned();
    };
    let rest = &normalized[pos + "shader_templates/".len()..];
    let Some((folder, _)) = rest.split_once('/') else {
        return "PARAMETERS".to_owned();
    };
    if folder.is_empty() {
        "PARAMETERS".to_owned()
    } else {
        folder.replace('_', " ").to_ascii_uppercase()
    }
}

fn h2_parameter_sections(
    root: TagStruct<'_>,
    template: Option<TagStruct<'_>>,
    names: &TagNameIndex,
) -> Vec<ShaderEditorSection> {
    if let Some(template) = template {
        let sections = h2_template_parameter_sections(root, template, names);
        if template
            .field("categories")
            .and_then(|field| field.as_block())
            .is_some()
        {
            return sections;
        }
    }
    let mut sections = Vec::new();
    h2_push_section(
        &mut sections,
        &h2_fallback_parameter_section_title(root, names),
        h2_compact_parameter_rows(root, names),
    );
    sections
}

fn h2_compact_parameter_rows(root: TagStruct<'_>, names: &TagNameIndex) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    let Some(block) = root.field("parameters").and_then(|field| field.as_block()) else {
        return rows;
    };
    for (index, element) in block.iter().enumerate() {
        if let Some(row) = h2_compact_parameter_row(element, index, names) {
            rows.push(row);
        }
        if let Some(animated) = element
            .field("animation properties")
            .and_then(|field| field.as_block())
        {
            for (anim_index, animation) in animated.iter().enumerate() {
                let path = format!("parameters[{index}]/animation properties[{anim_index}]");
                if let Some(row) = h2_animation_parameter_row(element, animation, &path) {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn h2_unused_parameters(
    root: TagStruct<'_>,
    template: TagStruct<'_>,
    names: &TagNameIndex,
) -> Vec<UnusedShaderParameter> {
    if template
        .field("categories")
        .and_then(|field| field.as_block())
        .is_none()
    {
        return Vec::new();
    }
    let active = h2_template_parameter_names(template);
    let Some(parameters) = root.field("parameters").and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    parameters
        .iter()
        .enumerate()
        .filter_map(|(index, parameter)| {
            let name = h2_parameter_name(parameter, index);
            if active.contains(&name) {
                return None;
            }
            let mut rows = Vec::new();
            if let Some(row) = h2_compact_parameter_row(parameter, index, names) {
                rows.push(row);
            }
            if let Some(animations) = parameter
                .field("animation properties")
                .and_then(|field| field.as_block())
            {
                for (animation_index, animation) in animations.iter().enumerate() {
                    if let Some(row) = h2_animation_parameter_row(
                        parameter,
                        animation,
                        &format!("parameters[{index}]/animation properties[{animation_index}]"),
                    ) {
                        rows.push(row);
                    }
                }
            }
            mark_unused_shader_rows(&mut rows);
            Some(UnusedShaderParameter {
                name,
                category: None,
                rows,
                delete: BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Delete(index),
                },
            })
        })
        .collect()
}

fn h2_template_parameter_sections(
    shader_root: TagStruct<'_>,
    template_root: TagStruct<'_>,
    names: &TagNameIndex,
) -> Vec<ShaderEditorSection> {
    let mut sections = Vec::new();
    let instances = h2_shader_parameter_instances(shader_root);
    let postprocess = H2PostprocessBindings::from_root(shader_root);
    let Some(categories) = template_root
        .field("categories")
        .and_then(|field| field.as_block())
    else {
        return sections;
    };
    // Postprocess slots use the flattened template order, even though the UI
    // now keeps each category separate. Never restart this index per section.
    let mut template_index = 0usize;
    for category in categories.iter() {
        let Some(parameters) = category
            .field("parameters")
            .and_then(|field| field.as_block())
        else {
            continue;
        };
        let title = category
            .read_string_id("name")
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "PARAMETERS".to_owned())
            .replace('_', " ")
            .to_ascii_uppercase();
        let mut rows = Vec::new();
        for template_param in parameters.iter() {
            let name = h2_template_parameter_name(template_param);
            if name.is_empty() {
                continue;
            }
            let instance = instances.iter().find(|instance| instance.name == name);
            rows.extend(h2_template_parameter_display_rows(
                template_param,
                instance,
                &postprocess,
                template_index,
                names,
            ));
            template_index += 1;
        }
        h2_push_section(&mut sections, &title, rows);
    }
    sections
}

#[cfg(test)]
fn h2_template_parameter_rows(
    shader_root: TagStruct<'_>,
    template_root: TagStruct<'_>,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    h2_template_parameter_sections(shader_root, template_root, names)
        .into_iter()
        .flat_map(|section| section.rows)
        .collect()
}

struct H2ParameterInstance<'a> {
    index: usize,
    name: String,
    element: TagStruct<'a>,
}

struct H2LiveElement<'a> {
    index: usize,
    path: String,
    element: TagStruct<'a>,
}

impl H2LiveElement<'_> {
    fn path(&self, field: &str) -> String {
        append_field_path(&self.path, &escape_field_path_segment(field))
    }
}

struct H2PostprocessBindings<'a> {
    values: Vec<H2LiveElement<'a>>,
    colors: Vec<H2LiveElement<'a>>,
    bitmap_transforms: Vec<H2LiveElement<'a>>,
    value_overlays: Vec<H2LiveElement<'a>>,
    color_overlays: Vec<H2LiveElement<'a>>,
    bitmap_transform_overlays: Vec<H2LiveElement<'a>>,
    overlays: Vec<H2LiveElement<'a>>,
    overlay_references: Vec<H2LiveElement<'a>>,
    animated_parameters: Vec<H2LiveElement<'a>>,
    animated_parameter_references: Vec<H2LiveElement<'a>>,
}

impl<'a> H2PostprocessBindings<'a> {
    fn from_root(root: TagStruct<'a>) -> Self {
        let empty = Self {
            values: Vec::new(),
            colors: Vec::new(),
            bitmap_transforms: Vec::new(),
            value_overlays: Vec::new(),
            color_overlays: Vec::new(),
            bitmap_transform_overlays: Vec::new(),
            overlays: Vec::new(),
            overlay_references: Vec::new(),
            animated_parameters: Vec::new(),
            animated_parameter_references: Vec::new(),
        };
        let Some(postprocess) = root
            .field("postprocess definition")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
        else {
            return empty;
        };
        let base = "postprocess definition[0]";
        let values = h2_collect_postprocess_elements(postprocess, base, "values");
        let colors = h2_collect_postprocess_elements(postprocess, base, "colors");
        Self {
            values: if values.is_empty() {
                h2_collect_postprocess_elements(postprocess, base, "value properties")
            } else {
                values
            },
            colors: if colors.is_empty() {
                h2_collect_postprocess_elements(postprocess, base, "color properties")
            } else {
                colors
            },
            bitmap_transforms: h2_collect_postprocess_elements(
                postprocess,
                base,
                "bitmap transforms",
            ),
            value_overlays: h2_collect_postprocess_elements(postprocess, base, "value overlays"),
            color_overlays: h2_collect_postprocess_elements(postprocess, base, "color overlays"),
            bitmap_transform_overlays: h2_collect_postprocess_elements(
                postprocess,
                base,
                "bitmap transform overlays",
            ),
            overlays: h2_collect_postprocess_elements(postprocess, base, "overlays"),
            overlay_references: h2_collect_postprocess_elements(
                postprocess,
                base,
                "overlay references",
            ),
            animated_parameters: h2_collect_postprocess_elements(
                postprocess,
                base,
                "animated parameters",
            ),
            animated_parameter_references: h2_collect_postprocess_elements(
                postprocess,
                base,
                "animated parameter references",
            ),
        }
    }

    fn value(&self, parameter_index: usize) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_by_parameter(&self.values, parameter_index)
    }

    fn color(&self, parameter_index: usize) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_by_parameter(&self.colors, parameter_index)
    }

    fn bitmap_transform(
        &self,
        parameter_index: usize,
        animation_type: i32,
    ) -> Option<&H2LiveElement<'a>> {
        h2_find_postprocess_transform(&self.bitmap_transforms, parameter_index, animation_type)
    }

    fn function(&self, parameter_index: usize, animation_type: i32) -> Option<FunctionView> {
        // This module still carries the animation type as its stored index;
        // name it at the boundary.
        let typed = u32::try_from(animation_type)
            .ok()
            .and_then(Halo2ShaderAnimationType::from_index);
        self.function_view(parameter_index, animation_type)
            .map(|view| view.with_color_types(h2_animation_color_types(typed)))
    }

    fn function_view(&self, parameter_index: usize, animation_type: i32) -> Option<FunctionView> {
        let legacy = match animation_type {
            11 => h2_find_postprocess_by_parameter(&self.value_overlays, parameter_index),
            12 => h2_find_postprocess_by_parameter(&self.color_overlays, parameter_index),
            _ => h2_find_postprocess_transform(
                &self.bitmap_transform_overlays,
                parameter_index,
                animation_type,
            ),
        };
        if let Some(live) = legacy {
            let function_struct = h2_named_struct_field(live.element, "function")?;
            let function_path = live.path("function");
            return classic_halo2_function_view_from_struct(
                live.element,
                function_struct,
                &function_path,
                "function",
            );
        }
        let live = self.new_layout_overlay(parameter_index, animation_type)?;
        let function_struct = h2_named_struct_field(live.element, "function")?;
        let function_path = live.path("function");
        classic_halo2_function_view_from_struct(
            live.element,
            function_struct,
            &function_path,
            "function",
        )
    }

    fn new_layout_overlay(
        &self,
        parameter_index: usize,
        animation_type: i32,
    ) -> Option<&H2LiveElement<'a>> {
        let animated_index = self
            .animated_parameter_references
            .iter()
            .position(|reference| {
                h2_read_usize(reference.element, "parameter index") == Some(parameter_index)
            })?;
        let animated = self.animated_parameters.get(animated_index)?;
        let overlay_reference_index = animated
            .element
            .field("overlay references")
            .and_then(|field| field.as_struct())
            .and_then(|overlay_refs| h2_read_usize(overlay_refs, "block index data"))?;
        let overlay_reference = self.overlay_references.get(overlay_reference_index)?;
        let transform_index = h2_read_i32(overlay_reference.element, "transform index");
        if animation_type != 11
            && animation_type != 12
            && !transform_index.is_some_and(|index| {
                h2_bitmap_transform_index_aliases(animation_type).contains(&index)
            })
        {
            return None;
        }
        let overlay_index = h2_read_usize(overlay_reference.element, "overlay index")?;
        self.overlays.get(overlay_index)
    }
}

fn h2_collect_postprocess_elements<'a>(
    postprocess: TagStruct<'a>,
    base_path: &str,
    block_name: &str,
) -> Vec<H2LiveElement<'a>> {
    let Some(block) = postprocess
        .field(block_name)
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    let escaped = escape_field_path_segment(block_name);
    block
        .iter()
        .enumerate()
        .map(|(index, element)| H2LiveElement {
            index,
            path: format!("{base_path}/{escaped}[{index}]"),
            element,
        })
        .collect()
}

fn h2_find_postprocess_by_parameter<'a, 'b>(
    elements: &'b [H2LiveElement<'a>],
    parameter_index: usize,
) -> Option<&'b H2LiveElement<'a>> {
    elements.iter().find(|element| {
        h2_read_usize(element.element, "parameter index")
            .map(|index| index == parameter_index)
            .unwrap_or(element.index == parameter_index)
    })
}

fn h2_find_postprocess_transform<'a, 'b>(
    elements: &'b [H2LiveElement<'a>],
    parameter_index: usize,
    animation_type: i32,
) -> Option<&'b H2LiveElement<'a>> {
    elements.iter().find(|element| {
        if h2_read_usize(element.element, "parameter index") != Some(parameter_index) {
            return false;
        }
        let transform_index = h2_read_i32(element.element, "bitmap transform index")
            .or_else(|| h2_read_i32(element.element, "transform index"));
        let overlay_type = h2_read_i32(element.element, "animation property type");
        overlay_type == Some(animation_type)
            || transform_index.is_some_and(|index| {
                h2_bitmap_transform_index_aliases(animation_type).contains(&index)
            })
    })
}

fn h2_bitmap_transform_index_aliases(animation_type: i32) -> &'static [i32] {
    match animation_type {
        0 => &[0],
        1 => &[0, 1],
        2 => &[1, 2],
        3 => &[2, 3],
        4 => &[1, 2, 4],
        5 => &[2, 3, 5],
        6 => &[3, 6],
        7 => &[4, 7],
        13 => &[5, 13],
        _ => &[],
    }
}

fn h2_read_i32(element: TagStruct<'_>, field: &str) -> Option<i32> {
    element
        .read_int_any(field)
        .and_then(|value| i32::try_from(value).ok())
}

fn h2_read_usize(element: TagStruct<'_>, field: &str) -> Option<usize> {
    element
        .read_int_any(field)
        .and_then(|value| usize::try_from(value).ok())
}

fn h2_named_struct_field<'a>(element: TagStruct<'a>, name: &str) -> Option<TagStruct<'a>> {
    element
        .fields()
        .find(|field| field.name() == name && field.field_type() == TagFieldType::Struct)
        .and_then(|field| field.as_struct())
}

fn h2_shader_parameter_instances(root: TagStruct<'_>) -> Vec<H2ParameterInstance<'_>> {
    let Some(block) = root.field("parameters").and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    block
        .iter()
        .enumerate()
        .map(|(index, element)| H2ParameterInstance {
            index,
            name: h2_parameter_name(element, index),
            element,
        })
        .collect()
}

fn h2_template_parameter_display_rows(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    names: &TagNameIndex,
) -> Vec<ShaderGridRow> {
    let mut rows = Vec::new();
    if let Some(row) =
        h2_template_base_parameter_row(template_param, instance, postprocess, template_index, names)
    {
        rows.push(row);
    }
    if h2_template_parameter_type_index(template_param) == 0 {
        rows.extend(h2_template_bitmap_animation_rows(
            template_param,
            instance,
            postprocess,
            template_index,
        ));
    } else if h2_template_flags(template_param) & 1 != 0 {
        rows.push(h2_template_value_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
        ));
    }
    rows
}

fn h2_template_base_parameter_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    let label = h2_template_parameter_name(template_param);
    let parameter_type = h2_template_parameter_type_index(template_param);
    let (field_name, default_field, parameter_type_label) = match parameter_type {
        0 => ("bitmap", "default bitmap", "bitmap"),
        2 => ("const color", "default const color", "color"),
        1 | 3 => ("const value", "default const value", "value"),
        _ => ("const value", "default const value", "value"),
    };
    if parameter_type == 0 && h2_template_flags(template_param) & 2 != 0 {
        return None;
    }
    let default_cell =
        h2_template_default_cell(template_param, default_field, names).or_else(|| {
            Some(ShaderGridCell {
                text: h2_parameter_type_label(parameter_type).to_owned(),
                value_kind: "default",
                color: None,
            })
        });
    if parameter_type == 2 {
        if let Some(function) = instance.and_then(|instance| {
            h2_find_animation_by_type(instance.element, 12).and_then(|(anim_index, anim)| {
                let path = format!(
                    "parameters[{}]/animation properties[{anim_index}]",
                    instance.index
                );
                let function_struct = anim.field("function")?.as_struct()?;
                let function_path = append_field_path(&path, "function");
                h2_function_view_from_animation_property(anim, function_struct, &function_path)
            })
        }) {
            let mut row = h2_function_template_row(label, function, template_param, 12);
            row.default_cell = default_cell;
            row.parameter_type = Some(parameter_type_label.to_owned());
            return Some(row);
        }
    }
    let postprocess_value = match parameter_type {
        1 | 3 => postprocess.value(template_index).and_then(|live| {
            live.element
                .field("value")
                .and_then(|field| field.value())
                .map(|value| (live.path("value"), value))
        }),
        2 => postprocess.color(template_index).and_then(|live| {
            live.element
                .field("color")
                .and_then(|field| field.value())
                .map(|value| (live.path("color"), value))
        }),
        _ => None,
    };
    let parameter_value = instance.and_then(|instance| {
        instance
            .element
            .field(field_name)
            .and_then(|field| field.value())
            .map(|value| {
                (
                    format!(
                        "parameters[{index}]/{}",
                        escape_field_path_segment(field_name),
                        index = instance.index
                    ),
                    value,
                )
            })
    });
    let live_value = postprocess_value.or(parameter_value);
    let (value_text, value_kind, color, edit) = if let Some((path, value)) = live_value {
        let formatted = format_value(names, &value, false);
        let color = color_popup_for_value(&label, &value, &formatted);
        (
            if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted.clone()
            },
            "value",
            color,
            classic_shader_row_edit(&path, &value, &formatted),
        )
    } else {
        let fallback = h2_template_default_text(template_param, default_field, names)
            .unwrap_or_else(|| String::new());
        let current = if parameter_type == 2 && fallback.is_empty() {
            "0,0,0,1".to_owned()
        } else {
            fallback.clone()
        };
        let color = (parameter_type == 2).then(|| {
            MaterialColorPopup::new(&label, 0.0, 0.0, 0.0, 1.0).with_alpha_available(false)
        });
        let kind = if parameter_type == 2 {
            ShaderRowEditKind::H2CreateTemplateColor {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: label.clone(),
                parameter_type_index: parameter_type,
                field: field_name.to_owned(),
            }
        } else {
            ShaderRowEditKind::H2CreateTemplateValue {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: label.clone(),
                parameter_type_index: parameter_type,
                field: field_name.to_owned(),
            }
        };
        (
            fallback,
            "default",
            color,
            Some(ShaderRowEdit {
                path: format!(
                    "parameters/<{}>/{}",
                    label,
                    escape_field_path_segment(field_name)
                ),
                current,
                kind,
            }),
        )
    };
    Some(ShaderGridRow {
        label,
        default_cell,
        value_cell: ShaderGridCell {
            text: value_text,
            value_kind,
            color,
        },
        parameter_type: Some(parameter_type_label.to_owned()),
        is_overridden: instance.is_some(),
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_template_bitmap_animation_rows(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
) -> Vec<ShaderGridRow> {
    let flags = h2_template_bitmap_animation_flags(template_param);
    let is_3d = h2_template_bitmap_type_index(template_param) != 0;
    let mut rows = Vec::new();
    if flags & (1 << 0) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            0,
            "scale",
        ));
    }
    if flags & (1 << 1) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            1,
            "scale_x",
        ));
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            2,
            "scale_y",
        ));
        if is_3d {
            rows.push(h2_template_animation_row(
                template_param,
                instance,
                postprocess,
                template_index,
                3,
                "scale_z",
            ));
        }
    }
    if flags & (1 << 2) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            4,
            "translation_x",
        ));
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            5,
            "translation_y",
        ));
        if is_3d {
            rows.push(h2_template_animation_row(
                template_param,
                instance,
                postprocess,
                template_index,
                6,
                "translation_z",
            ));
        }
    }
    if flags & (1 << 3) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            7,
            "rotation",
        ));
    }
    if flags & (1 << 4) != 0 {
        rows.push(h2_template_animation_row(
            template_param,
            instance,
            postprocess,
            template_index,
            13,
            "index",
        ));
    }
    rows
}

fn h2_template_value_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
) -> ShaderGridRow {
    let is_color = h2_template_parameter_type_index(template_param) == 2;
    let suffix = if is_color { "tint" } else { "value" };
    h2_template_animation_row(
        template_param,
        instance,
        postprocess,
        template_index,
        if is_color { 12 } else { 11 },
        suffix,
    )
}

fn h2_template_animation_row(
    template_param: TagStruct<'_>,
    instance: Option<&H2ParameterInstance<'_>>,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    animation_type: i32,
    suffix: &str,
) -> ShaderGridRow {
    let base = h2_template_parameter_name(template_param);
    let label = format!("{base}_{suffix}");
    let function = postprocess
        .function(template_index, animation_type)
        .or_else(|| {
            instance.and_then(|instance| {
                h2_find_animation_by_type(instance.element, animation_type).and_then(
                    |(anim_index, anim)| {
                        let path = format!(
                            "parameters[{}]/animation properties[{anim_index}]",
                            instance.index
                        );
                        let function_struct = anim.field("function")?.as_struct()?;
                        let function_path = append_field_path(&path, "function");
                        h2_function_view_from_animation_property(
                            anim,
                            function_struct,
                            &function_path,
                        )
                    },
                )
            })
        });
    let mut row = if let Some(function) = function {
        h2_function_template_row(label, function, template_param, animation_type)
    } else if let Some(row) =
        h2_postprocess_constant_animation_row(&label, postprocess, template_index, animation_type)
    {
        row
    } else {
        let initial_function_data =
            h2_template_initial_function_data(template_param, animation_type);
        h2_missing_function_row(
            label,
            h2_template_animation_default_value(template_param, animation_type),
            h2_template_animation_default_color(template_param, animation_type),
            H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: base,
                parameter_type_index: h2_template_parameter_type_index(template_param),
                animation_type_index: animation_type,
                initial_function_data,
            },
        )
    };
    let default_color = h2_template_animation_default_color(template_param, animation_type)
        .map(|rgba| MaterialColorPopup::new(&row.label, rgba[0], rgba[1], rgba[2], rgba[3]));
    row.default_cell = Some(ShaderGridCell {
        text: if default_color.is_some() {
            "color: RGB".to_owned()
        } else {
            format!(
                "value: {}",
                format_shader_float(h2_template_animation_default_value(
                    template_param,
                    animation_type
                ))
            )
        },
        value_kind: "default",
        color: default_color,
    });
    row
}

fn h2_find_animation_by_type(
    instance: TagStruct<'_>,
    animation_type: i32,
) -> Option<(usize, TagStruct<'_>)> {
    let block = instance
        .field("animation properties")
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find(|(_, animation)| {
        animation
            .read_int_any("type")
            .and_then(|value| i32::try_from(value).ok())
            == Some(animation_type)
    })
}

fn h2_function_template_row(
    label: String,
    function: FunctionView,
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> ShaderGridRow {
    if function.function.color_graph_type() != ColorGraphType::Scalar {
        if let Some(rgba) = extract_constant_color(&function.function) {
            let block_path = match function.edit.as_ref().map(|edit| &edit.data) {
                Some(FunctionDataStorage::Halo2ByteBlock(path)) => path.clone(),
                _ => String::new(),
            };
            let color = MaterialColorPopup::new(&label, rgba[0], rgba[1], rgba[2], rgba[3]);
            let mut row = ShaderGridRow {
                label,
                default_cell: Some(ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "default",
                    color: h2_template_animation_default_color(template_param, animation_type).map(
                        |rgba| MaterialColorPopup::new("", rgba[0], rgba[1], rgba[2], rgba[3]),
                    ),
                }),
                value_cell: ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "value",
                    color: Some(color),
                },
                parameter_type: Some("color".to_owned()),
                is_overridden: true,
                function: None,
                edit: (!block_path.is_empty()).then(|| ShaderRowEdit {
                    path: block_path.clone(),
                    current: h2_color_edit_current(rgba),
                    kind: ShaderRowEditKind::H2FunctionColor {
                        block_path,
                        legacy_data: Some(function.function.to_bytes()),
                    },
                }),
                context_menu: None,
                create_anim_op: None,
                constant_function_view: None,
            };
            row.constant_function_view = Some(function);
            return row;
        }
        let mut row = shader_function_grid_row(label, function);
        row.default_cell =
            h2_template_animation_default_color(template_param, animation_type).map(|rgba| {
                ShaderGridCell {
                    text: "color: RGB".to_owned(),
                    value_kind: "default",
                    color: Some(MaterialColorPopup::new(
                        "", rgba[0], rgba[1], rgba[2], rgba[3],
                    )),
                }
            });
        return row;
    }

    // Only a Constant-type function becomes a numeric row: the edit writes a
    // constant, and a flat periodic (say) would lose its shape to it.
    if let Some(value) = function
        .function
        .as_constant()
        .filter(|_| function.function.function_type() == FunctionType::Constant)
    {
        let block_path = match function.edit.as_ref().map(|edit| &edit.data) {
            Some(FunctionDataStorage::Halo2ByteBlock(path)) => path.clone(),
            _ => String::new(),
        };
        let current = format_shader_float(value);
        let mut row = ShaderGridRow {
            label,
            default_cell: Some(ShaderGridCell {
                text: String::new(),
                value_kind: "default",
                color: None,
            }),
            value_cell: shader_value_cell(format!("value: {current}")),
            parameter_type: Some("animated scalar".to_owned()),
            is_overridden: true,
            function: None,
            edit: (!block_path.is_empty()).then(|| ShaderRowEdit {
                path: block_path.clone(),
                current,
                kind: ShaderRowEditKind::H2FunctionScalar {
                    block_path,
                    legacy_data: Some(function.function.to_bytes()),
                },
            }),
            context_menu: None,
            create_anim_op: None,
            constant_function_view: None,
        };
        row.constant_function_view = Some(function);
        return row;
    }
    // A Halo 2 function that is not a constant keeps the grid's placeholder
    // text and opens the function editor from it.
    let mut row = if function.function.as_h2().is_some() {
        ShaderGridRow {
            label,
            default_cell: None,
            value_cell: ShaderGridCell {
                text: "<function data goes here>".to_owned(),
                value_kind: "value",
                color: None,
            },
            parameter_type: Some("function".to_owned()),
            is_overridden: true,
            function: None,
            edit: None,
            context_menu: None,
            create_anim_op: None,
            constant_function_view: Some(function),
        }
    } else {
        shader_function_grid_row(label, function)
    };
    row.default_cell = Some(ShaderGridCell {
        text: format!(
            "value: {}",
            format_shader_float(h2_template_animation_default_value(
                template_param,
                animation_type
            ))
        ),
        value_kind: "default",
        color: None,
    });
    row
}

fn h2_postprocess_constant_animation_row(
    label: &str,
    postprocess: &H2PostprocessBindings<'_>,
    template_index: usize,
    animation_type: i32,
) -> Option<ShaderGridRow> {
    let (live, field_name, parameter_type) = match animation_type {
        11 => (postprocess.value(template_index)?, "value", "value"),
        12 => (postprocess.color(template_index)?, "color", "color"),
        _ => (
            postprocess
                .bitmap_transform(template_index, animation_type)
                .or_else(|| {
                    (animation_type == 0)
                        .then(|| postprocess.value(template_index))
                        .flatten()
                })?,
            "value",
            "value",
        ),
    };
    let field = live.element.field(field_name)?;
    let value = field.value()?;
    let formatted = format_value(&TagNameIndex::default(), &value, false);
    let color = color_popup_for_value(label, &value, &formatted);
    let path = live.path(field_name);
    let edit = classic_shader_row_edit(&path, &value, &formatted);
    Some(ShaderGridRow {
        label: label.to_owned(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                format!("value: {}", trim_formatted_value(&formatted))
            },
            value_kind: "value",
            color,
        },
        parameter_type: Some(parameter_type.to_owned()),
        is_overridden: false,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_missing_function_row(
    label: String,
    default_value: f32,
    default_color: Option<[f32; 4]>,
    op: H2ShaderParamOp,
) -> ShaderGridRow {
    let edit_path = format!(
        "h2-create-function:{}:{}",
        label,
        format_shader_float(default_value)
    );
    if let Some(rgba) = default_color {
        return ShaderGridRow {
            label,
            default_cell: None,
            value_cell: ShaderGridCell {
                text: "color: RGB".to_owned(),
                value_kind: "default",
                color: Some(MaterialColorPopup::new(
                    "", rgba[0], rgba[1], rgba[2], rgba[3],
                )),
            },
            parameter_type: Some("function".to_owned()),
            is_overridden: false,
            function: None,
            edit: Some(ShaderRowEdit {
                path: edit_path,
                current: h2_color_edit_current(rgba),
                kind: ShaderRowEditKind::H2CreateFunctionColor {
                    create_op: op.clone(),
                },
            }),
            context_menu: None,
            create_anim_op: Some(ShaderContextAction::H2ParameterOp(op)),
            constant_function_view: None,
        };
    }
    ShaderGridRow {
        label,
        default_cell: None,
        value_cell: ShaderGridCell {
            text: format!("value: {}", format_shader_float(default_value)),
            value_kind: "default",
            color: None,
        },
        parameter_type: Some("function".to_owned()),
        is_overridden: false,
        function: None,
        edit: Some(ShaderRowEdit {
            path: edit_path,
            current: format_shader_float(default_value),
            kind: ShaderRowEditKind::H2CreateFunctionScalar {
                create_op: op.clone(),
            },
        }),
        context_menu: None,
        create_anim_op: Some(ShaderContextAction::H2ParameterOp(op)),
        constant_function_view: None,
    }
}

fn h2_template_animation_default_value(template_param: TagStruct<'_>, animation_type: i32) -> f32 {
    match animation_type {
        0 | 1 | 2 | 3 => template_param.read_real("bitmap scale").unwrap_or(1.0),
        11 => template_param
            .read_real("default const value")
            .unwrap_or_default(),
        _ => 0.0,
    }
}

fn h2_template_animation_default_color(
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> Option<[f32; 4]> {
    (animation_type == 12)
        .then(|| h2_template_default_color(template_param))
        .flatten()
}

fn h2_template_initial_function_data(
    template_param: TagStruct<'_>,
    animation_type: i32,
) -> Vec<u8> {
    if let Some([r, g, b, a]) = h2_template_animation_default_color(template_param, animation_type)
    {
        return h2_constant_color_function_data(r, g, b, a, None);
    }
    h2_constant_scalar_function_data(
        h2_template_animation_default_value(template_param, animation_type),
        None,
    )
}

fn h2_template_default_color(template_param: TagStruct<'_>) -> Option<[f32; 4]> {
    let value = template_param.field("default const color")?.value()?;
    color_value_to_rgba(&value)
}

fn color_value_to_rgba(value: &TagFieldData) -> Option<[f32; 4]> {
    match value {
        TagFieldData::RealRgbColor(color) => Some([color.red, color.green, color.blue, 1.0]),
        TagFieldData::RealArgbColor(color) => {
            Some([color.red, color.green, color.blue, color.alpha])
        }
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some([
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                1.0,
            ])
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some([
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                byte_to_float(((raw >> 24) & 0xFF) as u8),
            ])
        }
        _ => None,
    }
}

fn h2_color_edit_current(rgba: [f32; 4]) -> String {
    format!("{},{},{},{}", rgba[0], rgba[1], rgba[2], rgba[3])
}

pub(super) fn h2_template_parameter_name(template_param: TagStruct<'_>) -> String {
    template_param.read_string_id("name").unwrap_or_default()
}

fn h2_template_parameter_type_index(template_param: TagStruct<'_>) -> i32 {
    template_param
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_flags(template_param: TagStruct<'_>) -> u32 {
    template_param
        .read_int_any("flags")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_bitmap_animation_flags(template_param: TagStruct<'_>) -> u32 {
    template_param
        .read_int_any("bitmap animation flags")
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_bitmap_type_index(template_param: TagStruct<'_>) -> i32 {
    template_param
        .read_int_any("bitmap type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_template_default_cell(
    template_param: TagStruct<'_>,
    field_name: &str,
    names: &TagNameIndex,
) -> Option<ShaderGridCell> {
    let value = template_param.field(field_name)?.value()?;
    let formatted = trim_formatted_value(&format_value(names, &value, false));
    let color = color_popup_for_value(
        &h2_template_parameter_name(template_param),
        &value,
        &formatted,
    );
    Some(ShaderGridCell {
        text: if color.is_some() {
            "color: RGB".to_owned()
        } else if field_name == "default const value" {
            format!("value: {formatted}")
        } else {
            formatted
        },
        value_kind: "default",
        color,
    })
}

fn h2_template_default_text(
    template_param: TagStruct<'_>,
    field_name: &str,
    names: &TagNameIndex,
) -> Option<String> {
    let value = template_param.field(field_name)?.value()?;
    Some(trim_formatted_value(&format_value(names, &value, false)))
}

fn h2_compact_parameter_row(
    element: TagStruct<'_>,
    index: usize,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    let label = h2_parameter_name(element, index);
    let parameter_type = h2_parameter_type_index(element);
    let (field_name, parameter_type_label) = match parameter_type {
        0 => ("bitmap", "bitmap"),
        2 => ("const color", "color"),
        1 | 3 => ("const value", "value"),
        _ => ("const value", "value"),
    };
    let field = element.field(field_name)?;
    let path = format!(
        "parameters[{index}]/{}",
        escape_field_path_segment(field_name)
    );
    let value = field.value()?;
    let formatted = format_value(names, &value, false);
    let color = color_popup_for_value(&label, &value, &formatted);
    let edit = classic_shader_row_edit(&path, &value, &formatted);
    Some(ShaderGridRow {
        label,
        default_cell: Some(ShaderGridCell {
            text: h2_parameter_type_label(parameter_type).to_owned(),
            value_kind: "default",
            color: None,
        }),
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted
            },
            value_kind: "value",
            color,
        },
        parameter_type: Some(parameter_type_label.to_owned()),
        is_overridden: true,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_animation_parameter_row(
    parameter: TagStruct<'_>,
    animation: TagStruct<'_>,
    animation_path: &str,
) -> Option<ShaderGridRow> {
    let function_struct = animation.field("function")?.as_struct()?;
    let function_path = append_field_path(animation_path, "function");
    let view =
        h2_function_view_from_animation_property(animation, function_struct, &function_path)?;
    let label = h2_animation_row_label(parameter, animation);
    let mut row = shader_function_grid_row(label, view);
    row.default_cell = Some(ShaderGridCell {
        text: h2_animation_type_label(animation).to_owned(),
        value_kind: "default",
        color: None,
    });
    Some(row)
}

fn h2_raw_parameter_rows(root: TagStruct<'_>) -> Vec<ShaderGridRow> {
    let count = root
        .field("parameters")
        .and_then(|field| field.as_block())
        .map(|block| block.len())
        .unwrap_or_default();
    vec![ShaderGridRow {
        label: "parameters".to_owned(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: count.to_string(),
            value_kind: "value",
            color: None,
        },
        parameter_type: Some("count".to_owned()),
        is_overridden: false,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }]
}

fn h2_parameter_name(element: TagStruct<'_>, index: usize) -> String {
    element
        .read_string_id("name")
        .or_else(|| element.read_string_id("parameter name"))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("parameter_{index}"))
}

fn h2_parameter_type_index(element: TagStruct<'_>) -> i32 {
    element
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
}

fn h2_parameter_type_label(index: i32) -> &'static str {
    match index {
        0 => "bitmap",
        1 => "value",
        2 => "color",
        3 => "switch",
        _ => "value",
    }
}

fn h2_animation_type_label(animation: TagStruct<'_>) -> &'static str {
    match animation
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok())
        .unwrap_or_default()
    {
        0 => "scale",
        1 => "scale x",
        2 => "scale y",
        3 => "scale z",
        4 => "translation x",
        5 => "translation y",
        6 => "translation z",
        7 => "rotation angle",
        8 => "rotation axis x",
        9 => "rotation axis y",
        10 => "rotation axis z",
        11 => "value",
        12 => "color",
        13 => "bitmap index",
        _ => "function",
    }
}

fn h2_animation_row_label(parameter: TagStruct<'_>, animation: TagStruct<'_>) -> String {
    let base = h2_parameter_name(parameter, 0);
    let suffix = h2_animation_type_label(animation).replace(' ', "_");
    if suffix == "value" || suffix == "color" {
        base
    } else {
        format!("{base}_{suffix}")
    }
}

fn h2_shader_row_from_field(
    parent: TagStruct<'_>,
    field: TagField<'_>,
    path: &str,
    label_prefix: &str,
    names: &TagNameIndex,
) -> Option<ShaderGridRow> {
    if let Some(function) = field.as_function() {
        return Some(shader_function_grid_row(
            h2_nested_label(label_prefix, field.name()),
            FunctionView::from_function(function),
        ));
    }
    if field.name() == "function" {
        if let Some(nested) = field.as_struct() {
            if let Some(function) = h2_function_view_from_animation_property(parent, nested, path) {
                return Some(shader_function_grid_row(
                    h2_nested_label(label_prefix, "animation function"),
                    function,
                ));
            }
        }
    }

    let value = field.value()?;
    if matches!(
        value,
        TagFieldData::Data(_) | TagFieldData::ApiInterop(_) | TagFieldData::Custom(_)
    ) {
        return None;
    }
    let label = h2_nested_label(label_prefix, field.name());
    let formatted = format_value(names, &value, false);
    let color = color_popup_for_value(&label, &value, &formatted);
    let edit = classic_shader_row_edit(path, &value, &formatted).or_else(|| {
        (field.name() == "flags").then(|| ShaderRowEdit {
            path: path.to_owned(),
            current: parent
                .read_int_any(field.name())
                .unwrap_or_default()
                .to_string(),
            kind: ShaderRowEditKind::Flags(vec![
                "water".to_owned(),
                "sort first".to_owned(),
                "no active camo".to_owned(),
            ]),
        })
    });
    let value_kind = if is_none_like_value(&formatted) {
        "default"
    } else {
        "value"
    };
    Some(ShaderGridRow {
        label,
        default_cell: None,
        value_cell: ShaderGridCell {
            text: if color.is_some() {
                "color: RGB".to_owned()
            } else {
                formatted
            },
            value_kind,
            color,
        },
        parameter_type: Some(classic_shader_value_kind(&value).to_owned()),
        is_overridden: false,
        function: None,
        edit,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    })
}

fn h2_function_view_from_animation_property(
    animation_property: TagStruct<'_>,
    function_struct: TagStruct<'_>,
    function_path: &str,
) -> Option<FunctionView> {
    let data_block_path = h2_function_data_path(function_struct, function_path)?;
    let bytes = halo2_function_bytes_from_struct(function_struct)?;
    let animation_type = animation_property
        .read_enum_name("type")
        .and_then(|name| Halo2ShaderAnimationType::from_schema_name(&name));
    let mut view = FunctionView::from_function(h2_tag_function(&bytes)?)
        .with_color_types(h2_animation_color_types(animation_type));
    view.input_name = animation_property
        .read_string_id("input name")
        .unwrap_or_default();
    view.range_name = animation_property
        .read_string_id("range name")
        .unwrap_or_default();
    view.output_index = animation_property
        .read_int_any("type")
        .and_then(|value| i32::try_from(value).ok());
    view.time_period_in_seconds = animation_property
        .read_real("time period")
        .or_else(|| animation_property.read_real("time period in seconds"))
        .unwrap_or_default();

    let animation_path = function_path
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or("");
    let sibling_path = |name: &str| {
        if animation_path.is_empty() {
            escape_field_path_segment(name)
        } else {
            append_field_path(animation_path, &escape_field_path_segment(name))
        }
    };
    let time_field = if animation_property.field("time period").is_some() {
        "time period"
    } else if animation_property.field("time period in seconds").is_some() {
        "time period in seconds"
    } else {
        ""
    };
    Some(
        view.with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(data_block_path),
            parameter_type: animation_property
                .field("type")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("type"))
                .unwrap_or_default(),
            input_name: animation_property
                .field("input name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("input name"))
                .unwrap_or_default(),
            range_name: animation_property
                .field("range name")
                .and_then(|field| field.value())
                .is_some()
                .then(|| sibling_path("range name"))
                .unwrap_or_default(),
            time_period: (!time_field.is_empty()
                && animation_property
                    .field(time_field)
                    .and_then(|field| field.value())
                    .is_some())
            .then(|| sibling_path(time_field))
            .unwrap_or_default(),
            block_path: animation_path.to_owned(),
            block_index: animation_path
                .rsplit_once('[')
                .and_then(|(_, rest)| rest.strip_suffix(']'))
                .and_then(|index| index.parse::<usize>().ok())
                .unwrap_or_default(),
        }),
    )
}

/// The color graph types Guerilla offers a shader animation: its color editor
/// (2/3/4-color) for color animations, its scalar editor otherwise. Shipped
/// shaders agree: all 7,000 non-color animation functions are scalar, and
/// 2,023 of 2,026 color ones are 2/3/4-color.
fn h2_animation_color_types(animation_type: Option<Halo2ShaderAnimationType>) -> ColorTypeChoices {
    match animation_type {
        Some(Halo2ShaderAnimationType::Color) => ColorTypeChoices::MultiColorOnly,
        _ => ColorTypeChoices::ScalarOnly,
    }
}

fn h2_function_data_path(function_struct: TagStruct<'_>, function_path: &str) -> Option<String> {
    function_struct.field("data")?.as_block()?;
    Some(append_field_path(function_path, "data"))
}

fn h2_nested_label(prefix: &str, name: &str) -> String {
    classic_nested_label(prefix, name)
}

fn classic_halo2_function_view_from_struct(
    parent: TagStruct<'_>,
    tag_struct: TagStruct<'_>,
    path: &str,
    _field_name: &str,
) -> Option<FunctionView> {
    let (data_block_path, bytes) = if let Some(bytes) = halo2_function_bytes_from_struct(tag_struct)
    {
        (append_field_path(path, "data"), bytes)
    } else {
        let inner = h2_named_struct_field(tag_struct, "function")?;
        (
            append_field_path(path, "function/data"),
            halo2_function_bytes_from_struct(inner)?,
        )
    };
    let mut view = FunctionView::from_function(h2_tag_function(&bytes)?);
    view.input_name = parent.read_string_id("input name").unwrap_or_default();
    view.range_name = parent.read_string_id("range name").unwrap_or_default();
    view.time_period_in_seconds = parent
        .read_real("time period")
        .or_else(|| parent.read_real("time period in seconds"))
        .unwrap_or_default();

    let parent_path = path.rsplit_once('/').map(|(base, _)| base).unwrap_or("");
    let sibling_path = |name: &str| {
        if parent_path.is_empty() {
            escape_field_path_segment(name)
        } else {
            append_field_path(parent_path, &escape_field_path_segment(name))
        }
    };
    let time_field = if parent.field("time period").is_some() {
        "time period"
    } else if parent.field("time period in seconds").is_some() {
        "time period in seconds"
    } else {
        ""
    };
    let input_editable = parent
        .field("input name")
        .and_then(|field| field.value())
        .is_some();
    let range_editable = parent
        .field("range name")
        .and_then(|field| field.value())
        .is_some();
    let time_editable = !time_field.is_empty()
        && parent
            .field(time_field)
            .and_then(|field| field.value())
            .is_some();

    Some(
        view.with_edit(FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(data_block_path),
            parameter_type: String::new(),
            input_name: input_editable
                .then(|| sibling_path("input name"))
                .unwrap_or_default(),
            range_name: range_editable
                .then(|| sibling_path("range name"))
                .unwrap_or_default(),
            time_period: time_editable
                .then(|| sibling_path(time_field))
                .unwrap_or_default(),
            block_path: String::new(),
            block_index: 0,
        }),
    )
}

pub(in crate::app) fn halo2_function_bytes_from_struct(
    tag_struct: TagStruct<'_>,
) -> Option<Vec<u8>> {
    let block = tag_struct.field("data")?.as_block()?;
    let mut bytes = Vec::with_capacity(block.len());
    for element in block.iter() {
        let value = element.read_int_any("Value")?;
        bytes.push(value as i8 as u8);
    }
    Some(bytes)
}

#[cfg(test)]
pub(in crate::app) fn first_halo2_byte_block_function_row(
    model: &ShaderEditorModel,
) -> Option<(Vec<u8>, String)> {
    for row in model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .chain(model.top_rows.iter())
    {
        if let Some(view) = row.function.as_ref() {
            if let Some(edit) = view.edit.as_ref() {
                if let FunctionDataStorage::Halo2ByteBlock(path) = &edit.data {
                    return Some((view.function.to_bytes(), path.clone()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
pub(in crate::app) fn shader_row_edit_path_and_kind(
    model: &ShaderEditorModel,
    label: &str,
) -> Option<(String, &'static str)> {
    let edit = model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .chain(model.top_rows.iter())
        .find(|row| row.label == label)?
        .edit
        .as_ref()?;
    let kind = shader_row_edit_kind_name(&edit.kind);
    Some((edit.path.clone(), kind))
}

#[cfg(test)]
pub(in crate::app) fn shader_row_value_text_for_test(
    model: &ShaderEditorModel,
    label: &str,
) -> Option<String> {
    model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .chain(model.top_rows.iter())
        .find(|row| row.label == label)
        .map(|row| row.value_cell.text.clone())
}

#[cfg(test)]
pub(in crate::app) fn h2_function_data_range_for_test(data: &[u8]) -> (bool, Option<f32>) {
    (
        h2_function_range_enabled(data),
        h2_function_range_value(data),
    )
}

#[cfg(test)]
pub(in crate::app) fn h2_function_data_with_range_for_test(
    data: &[u8],
    enabled: bool,
    value: Option<f32>,
) -> Vec<u8> {
    h2_function_data_with_range(data, enabled, value)
}

#[cfg(test)]
fn shader_row_edit_kind_name(kind: &ShaderRowEditKind) -> &'static str {
    match kind {
        ShaderRowEditKind::Scalar => "scalar",
        ShaderRowEditKind::Int => "int",
        ShaderRowEditKind::StringId => "string_id",
        ShaderRowEditKind::BitmapRef { .. } => "bitmap_ref",
        ShaderRowEditKind::ShaderTemplateRef => "shader_template_ref",
        ShaderRowEditKind::StructuralRef { .. } => "structural_ref",
        ShaderRowEditKind::Bool { .. } => "bool",
        ShaderRowEditKind::Enum(_) => "enum",
        ShaderRowEditKind::Flags(_) => "flags",
        ShaderRowEditKind::FunctionScalar { .. } => "function_scalar",
        ShaderRowEditKind::FunctionColor { .. } => "function_color",
        ShaderRowEditKind::ColorField { .. } => "color",
        ShaderRowEditKind::CreateFunctionColor { .. } => "create_function_color",
        ShaderRowEditKind::CreateFunctionScalar { .. } => "create_function_scalar",
        ShaderRowEditKind::H2FunctionScalar { .. } => "h2_function_scalar",
        ShaderRowEditKind::H2CreateFunctionScalar { .. } => "h2_create_function_scalar",
        ShaderRowEditKind::H2FunctionColor { .. } => "h2_function_color",
        ShaderRowEditKind::H2CreateFunctionColor { .. } => "h2_create_function_color",
        ShaderRowEditKind::CreateScalarParam { .. } => "create_scalar_param",
        ShaderRowEditKind::H2CreateTemplateValue { .. } => "h2_create_template_value",
        ShaderRowEditKind::H2CreateTemplateColor { .. } => "h2_create_template_color",
    }
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_labels_for_test(
    shader: &TagFile,
    template: &TagFile,
) -> Vec<String> {
    h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default())
        .into_iter()
        .map(|row| row.label)
        .collect()
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_edit_kind_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<&'static str> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    let edit = rows.iter().find(|row| row.label == label)?.edit.as_ref()?;
    Some(shader_row_edit_kind_name(&edit.kind))
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_value_text_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<String> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    rows.into_iter()
        .find(|row| row.label == label)
        .map(|row| row.value_cell.text)
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_value_color_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<(u8, u8, u8, u8)> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    rows.into_iter()
        .find(|row| row.label == label)
        .and_then(|row| row.value_cell.color)
        .map(|color| {
            let color = color.color32();
            (color.r(), color.g(), color.b(), color.a())
        })
}

#[cfg(test)]
pub(in crate::app) fn h2_template_row_function_data_path_for_test(
    shader: &TagFile,
    template: &TagFile,
    label: &str,
) -> Option<String> {
    let rows = h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
    let row = rows.into_iter().find(|row| row.label == label)?;
    let view = row
        .function
        .as_ref()
        .or(row.constant_function_view.as_ref())?;
    let edit = view.edit.as_ref()?;
    let FunctionDataStorage::Halo2ByteBlock(path) = &edit.data else {
        return None;
    };
    Some(path.clone())
}

#[cfg(test)]
pub(in crate::app) fn h2_shader_template_reference_for_test(tag: &TagFile) -> Option<String> {
    h2_shader_template_reference(tag.root())
}

#[cfg(test)]
pub(in crate::app) struct H2FunctionEditSummary {
    pub(in crate::app) bytes: Vec<u8>,
    pub(in crate::app) output_index: Option<i32>,
    pub(in crate::app) input_name: String,
    pub(in crate::app) range_name: String,
    pub(in crate::app) time_period: f32,
    pub(in crate::app) data_path: String,
    pub(in crate::app) parameter_type_path: String,
    pub(in crate::app) input_name_path: String,
    pub(in crate::app) range_name_path: String,
    pub(in crate::app) time_period_path: String,
}

#[cfg(test)]
pub(in crate::app) fn first_h2_function_edit_summary(
    model: &ShaderEditorModel,
) -> Option<H2FunctionEditSummary> {
    for row in model
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .chain(model.top_rows.iter())
    {
        let Some(view) = row.function.as_ref() else {
            continue;
        };
        let Some(edit) = view.edit.as_ref() else {
            continue;
        };
        let FunctionDataStorage::Halo2ByteBlock(data_path) = &edit.data else {
            continue;
        };
        return Some(H2FunctionEditSummary {
            bytes: view.function.to_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
            data_path: data_path.clone(),
            parameter_type_path: edit.parameter_type.clone(),
            input_name_path: edit.input_name.clone(),
            range_name_path: edit.range_name.clone(),
            time_period_path: edit.time_period.clone(),
        });
    }
    None
}

fn classic_shader_row_edit(
    path: &str,
    value: &TagFieldData,
    formatted: &str,
) -> Option<ShaderRowEdit> {
    match value {
        TagFieldData::RealRgbColor(color) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: format!("{},{},{},1", color.red, color.green, color.blue),
            kind: ShaderRowEditKind::ColorField { argb: false },
        }),
        TagFieldData::RealArgbColor(color) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: format!(
                "{},{},{},{}",
                color.red, color.green, color.blue, color.alpha
            ),
            kind: ShaderRowEditKind::ColorField { argb: true },
        }),
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current: format!(
                    "{},{},{},1",
                    byte_to_float(((raw >> 16) & 0xFF) as u8),
                    byte_to_float(((raw >> 8) & 0xFF) as u8),
                    byte_to_float((raw & 0xFF) as u8)
                ),
                kind: ShaderRowEditKind::ColorField { argb: false },
            })
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current: format!(
                    "{},{},{},{}",
                    byte_to_float(((raw >> 16) & 0xFF) as u8),
                    byte_to_float(((raw >> 8) & 0xFF) as u8),
                    byte_to_float((raw & 0xFF) as u8),
                    byte_to_float(((raw >> 24) & 0xFF) as u8)
                ),
                kind: ShaderRowEditKind::ColorField { argb: true },
            })
        }
        TagFieldData::TagReference(reference) => {
            let Some((group_tag, name)) = reference.group_tag_and_name.as_ref() else {
                return Some(ShaderRowEdit {
                    path: path.to_owned(),
                    current: "NONE".to_owned(),
                    kind: ShaderRowEditKind::StringId,
                });
            };
            if *group_tag != u32::from_be_bytes(*b"bitm") {
                if *group_tag == u32::from_be_bytes(*b"stem") {
                    let current = if name.is_empty() {
                        "NONE".to_owned()
                    } else {
                        format!(
                            "{}.shader_template",
                            h2_normalize_shader_template_reference(name).replace('\\', "/")
                        )
                    };
                    return Some(ShaderRowEdit {
                        path: path.to_owned(),
                        current,
                        kind: ShaderRowEditKind::ShaderTemplateRef,
                    });
                }
                return Some(ShaderRowEdit {
                    path: path.to_owned(),
                    current: formatted.to_owned(),
                    kind: ShaderRowEditKind::StringId,
                });
            }
            let current = if name.is_empty() {
                "NONE".to_owned()
            } else {
                format!("{}.bitmap", name.replace('\\', "/"))
            };
            Some(ShaderRowEdit {
                path: path.to_owned(),
                current,
                kind: ShaderRowEditKind::BitmapRef {
                    group_tag: *group_tag,
                    create: None,
                },
            })
        }
        TagFieldData::StringId(value) | TagFieldData::OldStringId(value) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: value.string.clone(),
            kind: ShaderRowEditKind::StringId,
        }),
        TagFieldData::Real(value)
        | TagFieldData::RealSlider(value)
        | TagFieldData::RealFraction(value) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: value.to_string(),
            kind: ShaderRowEditKind::Scalar,
        }),
        // Degrees, like every other angle box in the editor. This grid commits
        // through the same parser, so showing the stored radians here would mean
        // typing back what was shown divided the value by 57.3.
        TagFieldData::Angle(value) => Some(ShaderRowEdit {
            path: path.to_owned(),
            current: crate::app::editor::fields::fmt_angle(*value),
            kind: ShaderRowEditKind::Scalar,
        }),
        TagFieldData::CharInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ShortInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::LongInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ByteInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::WordInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::DwordInteger(value) => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::CharEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::ShortEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        TagFieldData::LongEnum { value, .. } => classic_int_edit(path, *value as i64, formatted),
        _ => None,
    }
}

fn classic_int_edit(path: &str, value: i64, formatted: &str) -> Option<ShaderRowEdit> {
    let normalized = formatted.trim().to_ascii_lowercase();
    let kind = if matches!(normalized.as_str(), "true" | "false") {
        ShaderRowEditKind::Bool { create: None }
    } else {
        ShaderRowEditKind::Int
    };
    Some(ShaderRowEdit {
        path: path.to_owned(),
        current: value.to_string(),
        kind,
    })
}

fn classic_shader_value_kind(value: &TagFieldData) -> &'static str {
    match value {
        TagFieldData::TagReference(_) => "tag reference",
        TagFieldData::RealRgbColor(_)
        | TagFieldData::RealArgbColor(_)
        | TagFieldData::RgbColor(_)
        | TagFieldData::ArgbColor(_) => "color",
        TagFieldData::Real(_)
        | TagFieldData::RealSlider(_)
        | TagFieldData::RealFraction(_)
        | TagFieldData::Angle(_) => "real",
        TagFieldData::CharEnum { .. }
        | TagFieldData::ShortEnum { .. }
        | TagFieldData::LongEnum { .. } => "enum",
        TagFieldData::ByteFlags { .. }
        | TagFieldData::WordFlags { .. }
        | TagFieldData::LongFlags { .. }
        | TagFieldData::ByteBlockFlags(_)
        | TagFieldData::WordBlockFlags(_)
        | TagFieldData::LongBlockFlags(_) => "flags",
        _ => "value",
    }
}

fn classic_nested_label(prefix: &str, name: &str) -> String {
    let name = clean_field_name(name);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix} {name}")
    }
}

pub(super) fn empty_shader_grid_row() -> ShaderGridRow {
    ShaderGridRow {
        label: String::new(),
        default_cell: None,
        value_cell: ShaderGridCell {
            text: String::new(),
            value_kind: "value",
            color: None,
        },
        parameter_type: None,
        is_overridden: false,
        function: None,
        edit: None,
        context_menu: None,
        create_anim_op: None,
        constant_function_view: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h2_shader_gauge_color_uses_primary_charged_as_range_input() {
        let root = crate::core::test_kits::h2ek_tags();
        let path =
            root.join("objects/weapons/pistol/plasma_pistol/shaders/plasma_pistol_gauge.shader");
        if !path.exists() {
            return;
        }
        let definitions = locate_definitions_root();
        let tag = crate::core::source::read_tag_at_path(
            &path,
            Some(GameId::Halo2),
            Some(&definitions),
            u32::from_be_bytes(*b"shad"),
        )
        .unwrap();
        let source = TagSource::LooseFolder {
            root,
            game: Some(GameId::Halo2),
            definitions_root: definitions,
        };
        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"shad")),
            &TagNameIndex::default(),
            Some(&source),
            &mut H2TemplateCache::default(),
        )
        .unwrap();
        let row = model
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.label == "meter_on_color")
            .unwrap();
        let view = row
            .function
            .as_ref()
            .or(row.constant_function_view.as_ref())
            .unwrap();
        assert_eq!(view.range_name, "primary_charged");
        let range_path = &view.edit.as_ref().unwrap().range_name;
        assert!(range_path.ends_with("/range name"));
        assert_eq!(
            tag.root()
                .descend(range_path.rsplit_once('/').unwrap().0)
                .unwrap()
                .read_string_id("range name")
                .as_deref(),
            Some("primary_charged")
        );
    }
    use crate::app::editor::{
        H2TemplateCache, build_h2ek_shader_editor_model, extract_constant_color,
        first_h2_function_edit_summary, first_halo2_byte_block_function_row,
        h2_constant_color_function_data, h2_constant_scalar_function_data,
        h2_function_data_range_for_test, h2_function_data_with_range_for_test,
        h2_shader_template_reference_for_test, h2_tag_function, h2_template_row_edit_kind_for_test,
        h2_template_row_function_data_path_for_test, h2_template_row_labels_for_test,
        h2_template_row_value_color_for_test, h2_template_row_value_text_for_test,
        halo2_function_bytes_from_struct, shader_row_edit_path_and_kind,
        shader_row_value_text_for_test,
    };
    use crate::core::document::apply::{
        apply_field_edit, apply_one_block_op, apply_one_h2_shader_param_op,
        replace_halo2_function_byte_block,
    };

    #[test]
    fn halo2_function_byte_block_replacement_roundtrips_bytes() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = h2_constant_scalar_function_data(-0.25, None);

        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        let function = h2_tag_function(&bytes).unwrap();
        let reparsed =
            h2_tag_function(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
        assert_eq!(reparsed.to_bytes(), function.to_bytes());
    }

    #[test]
    fn classic_halo2_shader_model_exposes_byte_block_function_row() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        tag.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header: vec![0; 64],
        };
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = h2_constant_scalar_function_data(0.75, None);
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let entry = h2_shader_entry(u32::from_be_bytes(*b"shad"));
        let model = build_h2ek_shader_editor_model(
            &tag,
            &entry,
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default(),
        )
        .unwrap();
        let (function_bytes, path) = first_halo2_byte_block_function_row(&model).unwrap();

        assert_eq!(function_bytes, bytes);
        assert_eq!(path, "parameters[0]/animation properties[0]/function/data");
    }

    #[test]
    fn h2ek_shader_model_routes_only_classic_halo2_shader_family() {
        let entry = h2_shader_entry(u32::from_be_bytes(*b"shad"));
        let mut classic = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        classic.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header: vec![0; 64],
        };
        assert!(
            build_h2ek_shader_editor_model(
                &classic,
                &entry,
                &TagNameIndex::default(),
                None,
                &mut H2TemplateCache::default()
            )
            .is_some()
        );

        let mcc = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        assert!(
            build_h2ek_shader_editor_model(
                &mcc,
                &entry,
                &TagNameIndex::default(),
                None,
                &mut H2TemplateCache::default()
            )
            .is_none()
        );

        let non_shader = classic;
        let non_shader_entry = h2_shader_entry(u32::from_be_bytes(*b"bitm"));
        assert!(
            build_h2ek_shader_editor_model(
                &non_shader,
                &non_shader_entry,
                &TagNameIndex::default(),
                None,
                &mut H2TemplateCache::default(),
            )
            .is_none()
        );
    }

    #[test]
    fn h2ek_shader_model_exposes_schema_backed_value_rows() {
        let mut tag = h2_classic_shader_tag();
        apply_field_edit(
            &mut tag,
            "template",
            "stem:shaders/shader_templates/transparent/plasma_mask_offset",
        )
        .unwrap();
        apply_field_edit(&mut tag, "material name", "test_material").unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/name", "diffuse_map").unwrap();
        apply_field_edit(&mut tag, "parameters[0]/type", "1").unwrap();
        apply_field_edit(&mut tag, "parameters[0]/const value", "0.5").unwrap();

        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"shad")),
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default(),
        )
        .unwrap();

        let material_name = shader_row_edit_path_and_kind(&model, "material_name").unwrap();
        assert_eq!(material_name, ("material name".to_owned(), "string_id"));

        let template = shader_row_edit_path_and_kind(&model, "template").unwrap();
        assert_eq!(template, ("template".to_owned(), "shader_template_ref"));
        assert_eq!(model.top_rows[0].label, "template");
        assert!(
            model
                .sections
                .iter()
                .all(|section| section.rows.iter().all(|row| row.label != "template"))
        );

        let const_value = shader_row_edit_path_and_kind(&model, "diffuse_map").unwrap();
        assert_eq!(
            const_value,
            ("parameters[0]/const value".to_owned(), "scalar")
        );
    }

    #[test]
    fn h2ek_shader_standard_rows_use_guerilla_widgets() {
        let mut tag = h2_classic_shader_tag();
        apply_field_edit(&mut tag, "flags", "5").unwrap();
        apply_field_edit(&mut tag, "specular type", "1").unwrap();
        apply_field_edit(&mut tag, "lightmap type", "2").unwrap();
        apply_field_edit(&mut tag, "shader LOD bias", "1").unwrap();

        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"shad")),
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default(),
        )
        .unwrap();

        assert_eq!(
            shader_row_edit_path_and_kind(&model, "flags"),
            Some(("flags".to_owned(), "flags"))
        );
        assert_eq!(
            shader_row_edit_path_and_kind(&model, "dynamic_light_specular_type"),
            Some(("specular type".to_owned(), "enum"))
        );
        assert_eq!(
            shader_row_value_text_for_test(&model, "dynamic_light_specular_type").as_deref(),
            Some("default shiny")
        );
        assert_eq!(
            shader_row_value_text_for_test(&model, "lightmap_type").as_deref(),
            Some("dull specular")
        );
        assert_eq!(
            shader_row_value_text_for_test(&model, "shader_lod_bias").as_deref(),
            Some("4x size")
        );
        let standard = model
            .sections
            .iter()
            .find(|section| section.title == "STANDARD_PARAMETERS")
            .unwrap();
        let specular = standard
            .rows
            .iter()
            .find(|row| row.label == "dynamic_light_specular_type")
            .unwrap();
        assert_eq!(specular.default_cell.as_ref().unwrap().text, "none");
        let lightmap = standard
            .rows
            .iter()
            .find(|row| row.label == "lightmap_type")
            .unwrap();
        assert_eq!(lightmap.default_cell.as_ref().unwrap().text, "diffuse");
    }

    #[test]
    fn h2ek_shader_range_flag_updates_same_length_function_data() {
        let mut data = vec![0; 28];
        data[0] = 1;
        data[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        data[8..12].copy_from_slice(&1.0f32.to_le_bytes());

        let ranged = h2_function_data_with_range_for_test(&data, true, Some(2.5));
        assert_eq!(ranged.len(), data.len());
        assert_eq!(h2_function_data_range_for_test(&ranged), (true, Some(2.5)));
        assert_eq!(ranged[4..8], data[4..8], "the range minimum is left alone");

        let unranged = h2_function_data_with_range_for_test(&ranged, false, None);
        assert_eq!(unranged.len(), data.len());
        assert_eq!(h2_function_data_range_for_test(&unranged).0, false);
    }

    /// A color function's bytes 4-19 are its color slots, not a range: a range
    /// edit wrote its value over color slot 1 and set the range flag on a function
    /// that has none. It now leaves a color function alone.
    #[test]
    fn h2ek_shader_range_edit_leaves_a_color_function_alone() {
        let mut data = vec![0; 28];
        data[0] = 1;
        data[1] = 2 << 4;
        data[4..8].copy_from_slice(&0xFF11_2233u32.to_le_bytes());
        data[8..12].copy_from_slice(&0xFF44_5566u32.to_le_bytes());

        assert_eq!(
            h2_function_data_with_range_for_test(&data, true, Some(2.5)),
            data
        );
    }

    #[test]
    fn h2ek_shader_template_reference_accepts_h2ek_extension_path() {
        let mut tag = h2_classic_shader_tag();
        apply_field_edit(
            &mut tag,
            "template",
            "stem:shaders\\shader_templates\\transparent\\plasma_mask_offset.shader_template",
        )
        .unwrap();

        assert_eq!(
            h2_shader_template_reference_for_test(&tag).as_deref(),
            Some("shaders\\shader_templates\\transparent\\plasma_mask_offset")
        );
    }

    #[test]
    fn h2ek_shader_keeps_template_categories_and_global_postprocess_indices() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        let categories = [
            "bump_mapping",
            "texture",
            "self_illumination",
            "environment_mapping",
            "light_response",
            "ambient_occlusion",
        ];
        for (index, category) in categories.iter().enumerate() {
            for (tag, path) in [
                (&mut template, "categories".to_owned()),
                (
                    &mut shader,
                    "postprocess definition[0]/value properties".to_owned(),
                ),
            ] {
                apply_one_block_op(
                    tag,
                    &BlockOp {
                        path,
                        kind: BlockOpKind::Add,
                    },
                )
                .unwrap();
            }
            apply_field_edit(
                &mut template,
                &format!("categories[{index}]/name"),
                category,
            )
            .unwrap();
            let parameters = format!("categories[{index}]/parameters");
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: parameters.clone(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(&mut template, &format!("{parameters}[0]/name"), category).unwrap();
            apply_field_edit(&mut template, &format!("{parameters}[0]/type"), "1").unwrap();
            apply_field_edit(
                &mut shader,
                &format!("postprocess definition[0]/value properties[{index}]/value"),
                &(index + 1).to_string(),
            )
            .unwrap();
        }
        let sections = h2_parameter_sections(
            shader.root(),
            Some(template.root()),
            &TagNameIndex::default(),
        );
        assert_eq!(sections.len(), categories.len());
        for (index, (section, category)) in sections.iter().zip(categories).enumerate() {
            assert_eq!(
                section.title,
                category.replace('_', " ").to_ascii_uppercase()
            );
            assert_eq!(section.rows.len(), 1);
            assert_eq!(section.rows[0].label, category);
            let edit = section.rows[0].edit.as_ref().unwrap();
            assert_eq!(
                edit.path,
                format!("postprocess definition[0]/value properties[{index}]/value"),
            );
            assert_eq!(edit.current.parse::<f32>().unwrap(), (index + 1) as f32);
        }
    }

    #[test]
    fn h2_unused_parameter_preserves_its_value_until_explicitly_cleared() {
        let mut shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        for (index, name) in ["keep_me", "unused_value"].iter().enumerate() {
            apply_one_block_op(
                &mut shader,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(&mut shader, &format!("parameters[{index}]/name"), name).unwrap();
            apply_field_edit(&mut shader, &format!("parameters[{index}]/type"), "1").unwrap();
            apply_field_edit(
                &mut shader,
                &format!("parameters[{index}]/const value"),
                "7.5",
            )
            .unwrap();
        }
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "keep_me").unwrap();
        let unused = h2_unused_parameters(shader.root(), template.root(), &TagNameIndex::default());
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].rows[0].label, "unused_value");
        assert_eq!(
            unused[0].rows[0]
                .edit
                .as_ref()
                .unwrap()
                .current
                .parse::<f32>()
                .unwrap(),
            7.5
        );
        assert_h2_write_atomic_verifies(&shader, "h2_unused_parameter");
        apply_one_block_op(&mut shader, &unused[0].delete).unwrap();
        assert!(
            h2_unused_parameters(shader.root(), template.root(), &TagNameIndex::default())
                .is_empty()
        );
        let parameters = shader
            .root()
            .field("parameters")
            .unwrap()
            .as_block()
            .unwrap();
        assert_eq!(parameters.len(), 1);
        assert_eq!(
            parameters
                .element(0)
                .unwrap()
                .read_string_id("name")
                .as_deref(),
            Some("keep_me")
        );
    }

    #[test]
    fn h2ek_shader_missing_template_falls_back_but_empty_template_marks_parameters_unused() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut shader, "parameters[0]/name", "brightness").unwrap();
        apply_field_edit(&mut shader, "parameters[0]/type", "1").unwrap();
        let template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        let sections = h2_parameter_sections(shader.root(), None, &TagNameIndex::default());
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].rows[0].label, "brightness");
        assert!(
            h2_parameter_sections(
                shader.root(),
                Some(template.root()),
                &TagNameIndex::default()
            )
            .is_empty()
        );
        assert_eq!(
            h2_unused_parameters(shader.root(), template.root(), &TagNameIndex::default()).len(),
            1
        );
    }

    #[test]
    fn h2ek_shader_template_rows_drive_visible_parameters() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut shader, "parameters[0]/name", "self_illum_color").unwrap();
        apply_field_edit(&mut shader, "parameters[0]/type", "2").unwrap();

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for index in 0..2 {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let parameter_path = format!("categories[0]/parameters[{index}]");
            let name = if index == 0 {
                "noise_map1"
            } else {
                "plasma_mask"
            };
            apply_field_edit(&mut template, &format!("{parameter_path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{parameter_path}/type"), "0").unwrap();
        }
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "6",
        )
        .unwrap();

        let labels = h2_template_row_labels_for_test(&shader, &template);

        for expected in [
            "noise_map1",
            "noise_map1_scale_x",
            "noise_map1_scale_y",
            "noise_map1_translation_x",
            "noise_map1_translation_y",
            "plasma_mask",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing {expected} in {labels:?}"
            );
        }
        assert!(!labels.iter().any(|label| label == "self_illum_color"));
    }

    #[test]
    fn h2ek_shader_3d_bitmap_template_rows_include_z_transform() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap type",
            "3D",
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "5",
        )
        .unwrap();

        let labels = h2_template_row_labels_for_test(&shader, &template);

        for expected in [
            "noyze0",
            "noyze0_scale",
            "noyze0_translation_x",
            "noyze0_translation_y",
            "noyze0_translation_z",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing {expected} in {labels:?}"
            );
        }
    }

    #[test]
    fn h2ek_shader_missing_function_rows_are_numeric_create_fields() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "5",
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap scale",
            "7.5",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_create_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_translation_x"),
            Some("h2_create_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_translation_x")
                .as_deref(),
            Some("value: 0.0")
        );
    }

    #[test]
    fn h2ek_shader_existing_constant_function_rows_stay_numeric() {
        let mut shader = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "noyze0".to_owned(),
                parameter_type_index: 0,
                animation_type_index: 0,
                initial_function_data: h2_constant_scalar_function_data(7.5, None),
            },
        )
        .unwrap();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap animation flags",
            "1",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/bitmap scale",
            "2.25",
        )
        .unwrap();
        let rows =
            h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
        let row = rows.iter().find(|row| row.label == "noyze0_scale").unwrap();
        assert_eq!(row.default_cell.as_ref().unwrap().text, "value: 2.25");
        let ShaderFunctionReset::Halo2(reset) = shader_function_default_edit(row).unwrap() else {
            panic!("Halo 2 Clear must use the byte-block writer");
        };
        apply_one_h2_shader_param_op(&mut shader, &reset).unwrap();
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 2.25")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_function_scalar"),
            "the cleared function remains editable"
        );
    }

    #[test]
    fn h2ek_shader_color_tint_rows_use_color_animation_type() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "color_wide",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/default const color",
            "1, 1, 1",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_wide_tint"),
            Some("h2_create_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_wide_tint"),
            Some((255, 255, 255, 255))
        );
        let rows =
            h2_template_parameter_rows(shader.root(), template.root(), &TagNameIndex::default());
        for label in ["color_wide", "color_wide_tint"] {
            let row = rows.iter().find(|row| row.label == label).unwrap();
            let color = row
                .default_cell
                .as_ref()
                .unwrap()
                .color
                .as_ref()
                .expect("Halo 2 color defaults include a preview");
            assert_eq!(color.color32(), Color32::WHITE);
        }
    }

    #[test]
    fn h2ek_shader_existing_constant_color_functions_render_swatch() {
        let mut shader = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "color_sharp".to_owned(),
                parameter_type_index: 2,
                animation_type_index: 12,
                initial_function_data: h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None),
            },
        )
        .unwrap();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "color_sharp",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp_tint"),
            Some("h2_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp_tint"),
            Some((255, 0, 0, 255))
        );
        assert_h2_write_atomic_verifies(&shader, "h2_color_function_existing");
    }

    #[test]
    fn h2ek_shader_postprocess_constants_initialize_template_rows() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/value properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/value properties[0]/value",
            "7.5",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/color properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/color properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/color properties[1]/color",
            "1, 0, 0",
        )
        .unwrap();

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for (index, (name, ty, flags)) in [("noyze0", "0", "1"), ("color_sharp", "2", "0")]
            .into_iter()
            .enumerate()
        {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("categories[0]/parameters[{index}]");
            apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
            apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
            if name == "noyze0" {
                apply_field_edit(
                    &mut template,
                    &format!("{path}/bitmap animation flags"),
                    "1",
                )
                .unwrap();
            }
        }

        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("scalar")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
            Some((255, 0, 0, 255))
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
            Some("color")
        );
    }

    #[test]
    fn h2ek_shader_legacy_animation_bytes_initialize_template_rows() {
        let mut shader = h2_classic_shader_tag();
        for (index, (name, ty, anim_ty, data)) in [
            ("noyze0", "0", "0", {
                let mut data = vec![0; 28];
                data[0] = 1;
                data[4..8].copy_from_slice(&7.5f32.to_le_bytes());
                data[8..12].copy_from_slice(&1.0f32.to_le_bytes());
                data
            }),
            ("color_sharp", "2", "12", {
                let mut data = vec![0; 28];
                data[0] = 1;
                data[1] = 0x20;
                data[4] = 0;
                data[5] = 0;
                data[6] = 255;
                data[7] = 255;
                data
            }),
            ("noyze1", "0", "5", {
                let mut data = vec![0; 52];
                data[0] = 3;
                data[2] = 0x0a;
                data[10] = 0x80;
                data[11] = 0x3f;
                data[22] = 0x80;
                data[23] = 0x3f;
                data
            }),
        ]
        .into_iter()
        .enumerate()
        {
            apply_one_block_op(
                &mut shader,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("parameters[{index}]");
            apply_field_edit(&mut shader, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut shader, &format!("{path}/type"), ty).unwrap();
            apply_one_block_op(
                &mut shader,
                &BlockOp {
                    path: format!("{path}/animation properties"),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(
                &mut shader,
                &format!("{path}/animation properties[0]/type"),
                anim_ty,
            )
            .unwrap();
            seed_halo2_raw_function_byte_block_for_test(
                &mut shader,
                &format!("{path}/animation properties[0]/function/data"),
                &data,
            );
        }

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        for (index, (name, ty, flags, bitmap_flags)) in [
            ("noyze0", "0", "1", "1"),
            ("color_sharp", "2", "1", "0"),
            ("noyze1", "0", "1", "4"),
        ]
        .into_iter()
        .enumerate()
        {
            apply_one_block_op(
                &mut template,
                &BlockOp {
                    path: "categories[0]/parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            let path = format!("categories[0]/parameters[{index}]");
            apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
            apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
            apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
            apply_field_edit(
                &mut template,
                &format!("{path}/bitmap animation flags"),
                bitmap_flags,
            )
            .unwrap();
            if name == "noyze1" {
                apply_field_edit(&mut template, &format!("{path}/bitmap type"), "3D").unwrap();
            }
        }

        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
            Some("value: 7.5")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
            Some("h2_function_scalar")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
            Some((255, 0, 0, 255))
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
            Some("h2_function_color")
        );
        assert_eq!(
            h2_template_row_value_text_for_test(&shader, &template, "noyze1_translation_y")
                .as_deref(),
            Some("<function data goes here>")
        );
        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "noyze1_translation_y"),
            None
        );
        assert_eq!(
            h2_template_row_function_data_path_for_test(&shader, &template, "noyze1_translation_y")
                .as_deref(),
            Some("parameters[2]/animation properties[0]/function/data")
        );

        let mut legacy_scale = vec![0; 28];
        legacy_scale[0] = 1;
        legacy_scale[4..8].copy_from_slice(&7.5f32.to_le_bytes());
        let scale_edit = h2_constant_scalar_function_data(5.0, Some(&legacy_scale));
        assert_eq!(scale_edit.len(), 28);
        assert_eq!(
            f32::from_le_bytes(scale_edit[4..8].try_into().unwrap()),
            5.0
        );
        let color_edit = h2_constant_color_function_data(
            0.0,
            1.0,
            0.0,
            1.0,
            Some(&[
                1, 0x20, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]),
        );
        assert_eq!(&color_edit[..8], &[1, 0x20, 0, 0, 0, 255, 0, 255]);
    }

    #[test]
    fn h2ek_shader_postprocess_color_overlay_initializes_tint_swatch() {
        let mut shader = h2_classic_shader_tag();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/overlays".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/overlay references".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/overlay references[0]/overlay index",
            "0",
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/overlay references[0]/transform index",
            "0",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/animated parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/animated parameters[0]/overlay references/block index data",
            "0",
        )
        .unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "postprocess definition[0]/animated parameter references".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            "postprocess definition[0]/animated parameter references[0]/parameter index",
            "0",
        )
        .unwrap();
        seed_halo2_wrapped_function_byte_block_for_test(
            &mut shader,
            "postprocess definition[0]/overlays[0]/function",
            &h2_constant_color_function_data(1.0, 1.0, 0.0, 1.0, None),
        );

        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "center_line",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "center_line_tint"),
            Some("h2_function_color")
        );
        assert_eq!(
            h2_template_row_value_color_for_test(&shader, &template, "center_line_tint"),
            Some((255, 255, 0, 255))
        );

        apply_one_h2_shader_param_op(
            &mut shader,
            &H2ShaderParamOp::EditFunctionData {
                block_path: "postprocess definition[0]/overlays[0]/function/function/data"
                    .to_owned(),
                data: h2_constant_color_function_data(0.0, 0.0, 0.0, 1.0, None),
            },
        )
        .unwrap();
        let overlay = shader
            .root()
            .descend("postprocess definition[0]/overlays[0]/function")
            .unwrap();
        let function_struct = overlay
            .fields()
            .find(|field| field.name() == "function" && field.field_type() == TagFieldType::Struct)
            .and_then(|field| field.as_struct())
            .unwrap();
        let bytes = halo2_function_bytes_from_struct(function_struct).unwrap();
        let function = h2_tag_function(&bytes).unwrap();
        assert_eq!(
            extract_constant_color(&function),
            Some([0.0, 0.0, 0.0, 1.0])
        );
    }

    #[test]
    fn h2_shader_color_function_create_and_edit_reparse() {
        let mut tag = h2_classic_shader_tag();
        let red = h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None);
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "center_line".to_owned(),
                parameter_type_index: 2,
                animation_type_index: 12,
                initial_function_data: red,
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        let animation = parameters
            .element(0)
            .unwrap()
            .field("animation properties")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
            .unwrap();
        assert_eq!(animation.read_int_any("type"), Some(12));
        let data_path = "parameters[0]/animation properties[0]/function/data";
        let grey = h2_constant_color_function_data(0.5, 0.5, 0.5, 1.0, None);
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EditFunctionData {
                block_path: data_path.to_owned(),
                data: grey.clone(),
            },
        )
        .unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), grey);
        let function =
            h2_tag_function(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
        let color = extract_constant_color(&function).unwrap();
        for (actual, expected) in
            color
                .iter()
                .zip([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0])
        {
            assert!((actual - expected).abs() < 0.0001);
        }
        assert_h2_write_atomic_verifies(&tag, "h2_color_function_edit");
    }

    #[test]
    fn h2ek_shader_missing_template_value_row_is_create_editable() {
        let shader = h2_classic_shader_tag();
        let mut template =
            TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/name",
            "plasma_factor",
        )
        .unwrap();
        apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "1").unwrap();
        apply_field_edit(
            &mut template,
            "categories[0]/parameters[0]/default const value",
            "0.35",
        )
        .unwrap();

        assert_eq!(
            h2_template_row_edit_kind_for_test(&shader, &template, "plasma_factor"),
            Some("h2_create_template_value")
        );
    }

    #[test]
    fn h2_shader_template_value_edit_creates_single_parameter() {
        let mut tag = h2_classic_shader_tag();
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EditTemplateBackedValue {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "plasma_brightness".to_owned(),
                parameter_type_index: 1,
                field: "const value".to_owned(),
                input: "1.25".to_owned(),
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(parameters.len(), 1);
        let parameter = parameters.element(0).unwrap();
        assert_eq!(
            parameter.read_string_id("name").as_deref(),
            Some("plasma_brightness")
        );
        assert_eq!(parameter.read_int_any("type"), Some(1));
        assert_eq!(parameter.read_real("const value"), Some(1.25));
    }

    #[test]
    fn h2_shader_template_function_create_materializes_backing_data() {
        let mut tag = h2_classic_shader_tag();
        let bytes = h2_constant_scalar_function_data(0.5, None);
        apply_one_h2_shader_param_op(
            &mut tag,
            &H2ShaderParamOp::EnsureAnimationProperty {
                parameters_block_path: "parameters".to_owned(),
                parameter_name: "noise_map1".to_owned(),
                parameter_type_index: 0,
                animation_type_index: 5,
                initial_function_data: bytes.clone(),
            },
        )
        .unwrap();

        let parameters = tag
            .root()
            .field("parameters")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(parameters.len(), 1);
        let parameter = parameters.element(0).unwrap();
        assert_eq!(
            parameter.read_string_id("name").as_deref(),
            Some("noise_map1")
        );
        assert_eq!(parameter.read_int_any("type"), Some(0));
        let animation = parameter
            .field("animation properties")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
            .unwrap();
        assert_eq!(animation.read_int_any("type"), Some(5));
        let mapping = animation
            .field("function")
            .and_then(|field| field.as_struct())
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        assert_h2_write_atomic_verifies(&tag, "h2_function_create");
    }

    #[test]
    fn h2ek_shader_function_row_exposes_byte_block_and_wrapper_paths() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/name", "animated_scalar").unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut tag, "parameters[0]/animation properties[0]/type", "8").unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/input name",
            "time",
        )
        .unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/range name",
            "random",
        )
        .unwrap();
        apply_field_edit(
            &mut tag,
            "parameters[0]/animation properties[0]/time period",
            "2.5",
        )
        .unwrap();
        let bytes = h2_constant_scalar_function_data(0.75, None);
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        );

        let model = build_h2ek_shader_editor_model(
            &tag,
            &h2_shader_entry(u32::from_be_bytes(*b"shad")),
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default(),
        )
        .unwrap();
        let summary = first_h2_function_edit_summary(&model).expect("function row");

        assert_eq!(summary.bytes, bytes);
        assert_eq!(summary.output_index, Some(8));
        assert_eq!(summary.input_name, "time");
        assert_eq!(summary.range_name, "random");
        assert_eq!(summary.time_period, 2.5);
        assert_eq!(
            summary.data_path,
            "parameters[0]/animation properties[0]/function/data"
        );
        assert_eq!(
            summary.parameter_type_path,
            "parameters[0]/animation properties[0]/type"
        );
        assert_eq!(
            summary.input_name_path,
            "parameters[0]/animation properties[0]/input name"
        );
        assert_eq!(
            summary.range_name_path,
            "parameters[0]/animation properties[0]/range name"
        );
        assert_eq!(
            summary.time_period_path,
            "parameters[0]/animation properties[0]/time period"
        );
    }

    #[test]
    fn h2ek_shader_input_name_edit_writes_without_truncated_struct_panic() {
        let mut tag = h2_classic_shader_tag();
        for _ in 0..2 {
            apply_one_block_op(
                &mut tag,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[1]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = h2_constant_scalar_function_data(0.75, None);
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            "parameters[1]/animation properties[0]/function/data",
            &bytes,
        );

        apply_field_edit(
            &mut tag,
            "parameters[1]/animation properties[0]/input name",
            "shield_strength",
        )
        .unwrap();

        assert_h2_write_atomic_verifies(&tag, "h2_input_name_edit");
        let written = tag.write_to_bytes().expect("write edited h2 shader");
        assert!(
            written
                .windows("shield_strength".len())
                .any(|window| { window == "shield_strength".as_bytes() })
        );
    }

    #[test]
    fn halo2_function_byte_block_rejects_invalid_mapping_function_before_clear() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let original = h2_constant_scalar_function_data(0.25, None);
        let block_path = "parameters[0]/animation properties[0]/function/data";
        seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);

        assert!(replace_halo2_function_byte_block(&mut tag, block_path, &[1, 2, 3]).is_err());

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), original);
    }

    #[test]
    fn halo2_function_byte_block_same_length_edit_writes_in_place() {
        let mut tag = h2_classic_shader_tag();
        for _ in 0..7 {
            apply_one_block_op(
                &mut tag,
                &BlockOp {
                    path: "parameters".to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
        }
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[6]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let block_path = "parameters[6]/animation properties[0]/function/data";
        let original = h2_constant_scalar_function_data(0.25, None);
        seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);
        let edited = h2_constant_scalar_function_data(0.75, None);

        replace_halo2_function_byte_block(&mut tag, block_path, &edited).unwrap();

        let mapping = tag
            .root()
            .descend("parameters[6]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), edited);
        assert_h2_write_atomic_verifies(&tag, "h2_function_same_len");
    }

    #[test]
    fn damage_effect_vibration_byte_block_same_length_edit_preserves_36_bytes() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let block_path = "parameters[0]/animation properties[0]/function/data";
        let mut original = vec![0; 36];
        original[0] = 2;
        original[1] = 0;
        original[2] = 1;
        original[20..24].copy_from_slice(&0.8f32.to_le_bytes());
        original[24..28].copy_from_slice(&0.4f32.to_le_bytes());
        original[32..36].copy_from_slice(&1.0f32.to_le_bytes());
        seed_halo2_raw_function_byte_block_for_test(&mut tag, block_path, &original);
        let mut edited = original.clone();
        edited[2] = 2;
        edited[20..24].copy_from_slice(&1.0f32.to_le_bytes());
        edited[24..28].copy_from_slice(&0.7f32.to_le_bytes());

        replace_halo2_function_byte_block(&mut tag, block_path, &edited).unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        let written = halo2_function_bytes_from_struct(mapping).unwrap();
        assert_eq!(written.len(), 36);
        assert_eq!(written, edited);
        assert_eq!(&written[32..36], &original[32..36]);
    }

    #[test]
    fn halo2_function_byte_block_existing_length_change_rebuilds_block() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let block_path = "parameters[0]/animation properties[0]/function/data";
        seed_halo2_function_byte_block_for_test(
            &mut tag,
            block_path,
            &h2_constant_scalar_function_data(0.25, None),
        );
        let mut linear_key = vec![0u8; 32];
        linear_key[0] = 5;
        linear_key[4..8].copy_from_slice(&0.0f32.to_le_bytes());
        linear_key[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        for &(x, y) in &[(0.0_f32, 0.0_f32), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)] {
            linear_key.extend_from_slice(&x.to_le_bytes());
            linear_key.extend_from_slice(&y.to_le_bytes());
        }
        for _ in 0..12 {
            linear_key.extend_from_slice(&0.0_f32.to_le_bytes());
        }

        replace_halo2_function_byte_block(&mut tag, block_path, &linear_key).unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(
            halo2_function_bytes_from_struct(mapping).unwrap(),
            linear_key
        );
        assert_h2_write_atomic_verifies(&tag, "h2_function_resize");
    }

    #[test]
    fn halo2_function_byte_block_empty_creation_rebuilds_block() {
        let mut tag = h2_classic_shader_tag();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters[0]/animation properties".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let bytes = h2_constant_scalar_function_data(0.25, None);
        replace_halo2_function_byte_block(
            &mut tag,
            "parameters[0]/animation properties[0]/function/data",
            &bytes,
        )
        .unwrap();

        let mapping = tag
            .root()
            .descend("parameters[0]/animation properties[0]/function")
            .unwrap();
        assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
        assert_h2_write_atomic_verifies(&tag, "h2_function_empty_create");
    }

    fn h2_classic_shader_tag() -> TagFile {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(b"hsmr");
        header[56..58].copy_from_slice(&0u16.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        tag.container = blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header,
        };
        tag
    }

    fn assert_h2_write_atomic_verifies(tag: &TagFile, name: &str) {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "baboon_{name}_{}_{}.shader",
            std::process::id(),
            stamp
        ));
        let _ = fs::remove_file(&path);
        tag.write_atomic(&path).unwrap_or_else(|error| {
            panic!(
                "write_atomic verification failed for {}: {error}",
                path.display()
            )
        });
        let _ = fs::remove_file(&path);
    }

    fn seed_halo2_function_byte_block_for_test(tag: &mut TagFile, block_path: &str, data: &[u8]) {
        h2_tag_function(data).expect("seed data is an H2 block");
        seed_halo2_raw_function_byte_block_for_test(tag, block_path, data);
    }

    fn seed_halo2_raw_function_byte_block_for_test(
        tag: &mut TagFile,
        block_path: &str,
        data: &[u8],
    ) {
        apply_one_block_op(
            tag,
            &BlockOp {
                path: block_path.to_owned(),
                kind: BlockOpKind::DeleteAll,
            },
        )
        .unwrap();
        for (index, byte) in data.iter().copied().enumerate() {
            apply_one_block_op(
                tag,
                &BlockOp {
                    path: block_path.to_owned(),
                    kind: BlockOpKind::Add,
                },
            )
            .unwrap();
            apply_field_edit(
                tag,
                &format!("{block_path}[{index}]/Value"),
                &(byte as i8).to_string(),
            )
            .unwrap();
        }
    }

    fn seed_halo2_wrapped_function_byte_block_for_test(
        tag: &mut TagFile,
        wrapper_path: &str,
        data: &[u8],
    ) {
        h2_tag_function(data).expect("seed data is an H2 block");
        let mut root = tag.root_mut();
        let mut wrapper_field = root.field_path_mut(wrapper_path).unwrap();
        let mut wrapper = wrapper_field.as_struct_mut().unwrap();
        let mut wrote = false;
        wrapper.for_each_field_mut(|mut field| {
            if wrote
                || field.as_ref().name() != "function"
                || field.as_ref().field_type() != TagFieldType::Struct
            {
                return;
            }
            let Some(mut mapping) = field.as_struct_mut() else {
                return;
            };
            let Some(mut data_field) = mapping.field_mut("data") else {
                return;
            };
            let Some(mut block) = data_field.as_block_mut() else {
                return;
            };
            block.clear();
            for byte in data.iter().copied() {
                let index = block.add_element();
                let mut element = block.element_mut(index).unwrap();
                element
                    .field_mut("Value")
                    .unwrap()
                    .set(TagFieldData::CharInteger(byte as i8))
                    .unwrap();
            }
            wrote = true;
        });
        assert!(wrote, "failed to seed wrapped H2 function bytes");
    }

    /// The H2 shader grid is rebuilt every frame, and used to read and parse
    /// its `.shader_template` off disk each time. Now the template is read
    /// once, and again only when the file changes.
    #[test]
    fn the_h2_shader_grid_reads_its_template_once_per_change() {
        let root = std::env::temp_dir().join(format!(
            "baboon-h2-template-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("shaders")).unwrap();
        let template_path = root.join("shaders/test.shader_template");
        std::fs::write(&template_path, b"not a template").unwrap();

        let mut tag = h2_classic_shader_tag();
        crate::core::document::apply::apply_field_edit(&mut tag, "template", "stem:shaders/test")
            .unwrap();
        let entry = h2_shader_entry(u32::from_be_bytes(*b"shad"));
        let source = TagSource::LooseFolder {
            root: root.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        };
        let mut templates = H2TemplateCache::default();
        let frames = |templates: &mut H2TemplateCache| {
            for _ in 0..3 {
                build_h2ek_shader_editor_model(
                    &tag,
                    &entry,
                    &TagNameIndex::default(),
                    Some(&source),
                    templates,
                );
            }
        };

        frames(&mut templates);
        assert_eq!(templates.loads, 1, "three frames, one read");
        // A different size is a different file, whatever the clock says.
        std::fs::write(&template_path, b"still not a template, but longer").unwrap();
        frames(&mut templates);

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(templates.loads, 2, "a changed file is read again, once");
    }

    fn h2_shader_entry(group_tag: u32) -> TagEntry {
        TagEntry {
            key: "objects/test/example.shader".into(),
            display_path: "objects/test/example.shader".into(),
            group_tag,
            group_name: Some("shader".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("example.shader")),
        }
    }
}
