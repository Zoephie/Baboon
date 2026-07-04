use super::*;
use blam_tags::math::{RealPoint3d, RealQuaternion, RealVector3d};
use blam_tags::render_model::{Marker, Node, RenderMesh};

/// Renderer-facing preview geometry derived from a [`RenderModel`]. Lives in
/// Baboon (not blam-tags) since it is purely a GUI concern.
#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreview {
    pub regions: Vec<RenderModelPreviewRegion>,
    pub vertices: Vec<RenderModelPreviewVertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<RenderModelPreviewBatch>,
    pub markers: Vec<RenderModelPreviewMarker>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewRegion {
    pub name: String,
    pub permutations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderModelPreviewVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewBatch {
    pub region_name: String,
    pub permutation_name: String,
    pub material_index: u16,
    pub part_type: i8,
    pub index_start: u32,
    pub index_count: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RenderModelPreviewMarker {
    pub name: String,
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
}

pub(super) fn draw_model_preview_panel(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    state: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    edit: &mut FieldEditContext<'_>,
) {
    let is_model = is_model_group(entry.group_tag, names);
    if !is_model {
        return;
    }

    egui::CollapsingHeader::new(RichText::new("Render model").strong().color(text_dark()))
        .id_salt(("model_preview", &entry.key))
        .default_open(true)
        .show(ui, |ui| {
            // The parse is synchronous; on the first frame for a tag, show a
            // spinner, kick the (blocking) parse, and repaint so the decoded
            // model appears next frame instead of a blank panel. (A future
            // change can move the parse to a worker thread — see plan 1.9.)
            let needs_load =
                state.loaded_key.as_deref() != Some(entry.key.as_str()) || state.data.is_none();
            if needs_load {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading model…").color(subtle_dark()));
                });
                ensure_model_preview_loaded(tag, entry, names, source, state);
                ui.ctx().request_repaint();
                return;
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new("Scale").color(subtle_dark()));
                ui.add(
                    egui::Slider::new(&mut state.scale, 0.05..=5.0)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                );
                if ui.button("Reset").clicked() {
                    state.yaw = -0.45;
                    state.pitch = 0.25;
                    state.pan = Vec2::ZERO;
                    state.scale = 1.0;
                }
                ui.checkbox(&mut state.show_markers, "Markers");
                if state.show_markers {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.marker_filter)
                            .hint_text("filter markers…")
                            .desired_width(110.0),
                    );
                }
                ui.checkbox(&mut state.show_wireframe, "Wireframe");
                ui.checkbox(&mut state.show_backfaces, "Backfaces");
                ui.label(RichText::new("Viewport").color(subtle_dark()));
                ui.add(
                    egui::Slider::new(
                        model_preview_size,
                        MIN_MODEL_PREVIEW_SIZE..=MAX_MODEL_PREVIEW_SIZE,
                    )
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
                );
                ui.label(
                    RichText::new(format!("{:.0}%", *model_preview_size * 100.0))
                        .color(subtle_dark()),
                );
                if ui.button("Refresh model").clicked() {
                    state.loaded_key = None;
                    state.data = None;
                    ensure_model_preview_loaded(tag, entry, names, source, state);
                }
            });

            let Some(data_result) = state.data.take() else {
                ui.label(RichText::new("No preview loaded").color(subtle_dark()));
                return;
            };
            let mut restore_data = Some(data_result);
            let data = match restore_data.as_ref().expect("preview data just set") {
                Ok(data) => data,
                Err(error) => {
                    ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                    state.data = restore_data.take();
                    return;
                }
            };

            let mut mutation_requested = false;
            let desired_viewport = model_viewport_size(ui.available_width(), *model_preview_size);
            let can_place_controls_beside = ui.available_width() >= desired_viewport.x + 360.0;
            if can_place_controls_beside {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        draw_model_viewport_with_stats(ui, data, state, desired_viewport)
                    });
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        if draw_variant_controls(ui, data, state, edit) {
                            mutation_requested = true;
                        }
                    });
                });
            } else {
                draw_model_viewport_with_stats(ui, data, state, desired_viewport);
                ui.add_space(8.0);
                if draw_variant_controls(ui, data, state, edit) {
                    mutation_requested = true;
                }
            }
            if mutation_requested {
                state.loaded_key = None;
                state.data = None;
            } else {
                state.data = restore_data.take();
            }
        });
    ui.add_space(8.0);
}

fn model_viewport_size(available_width: f32, model_preview_size: f32) -> Vec2 {
    let scale = model_preview_size.clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE);
    let desired = Vec2::new(470.0 * scale, 300.0 * scale);
    let width = desired.x.min(available_width.max(280.0)).max(280.0);
    Vec2::new(width, desired.y * (width / desired.x))
}

fn draw_model_viewport_with_stats(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    draw_model_viewport(ui, data, state, desired_size);
    ui.small(
        RichText::new(format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ))
        .color(subtle_dark()),
    );
}

fn ensure_model_preview_loaded(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    state: &mut ModelPreviewState,
) {
    if state.loaded_key.as_deref() == Some(entry.key.as_str()) && state.data.is_some() {
        return;
    }
    state.loaded_key = Some(entry.key.clone());
    state.data = Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load_model_preview(model_tag, entry, names, source)
        }))
        .map_err(|_| "Render model preview crashed while parsing this tag.".to_owned())
        .and_then(|result| result)
        .map(|data| {
            state.render_model_path = Some(data.render_model_path.clone());
            // Auto-select the canonical variant (named `default`, else the first)
            // so the preview opens showing a complete configured model.
            let default_variant = default_variant_index(&data.variants);
            reset_model_preview_selection(state, &data, default_variant);
            data
        }),
    );
}

fn load_model_preview(
    model_tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
) -> Result<ModelPreviewData, String> {
    // Halo CE `gbxmodel` (mod2) and a bare `render_model` (mode) ARE the
    // render geometry — there is no `.model` (hlmt) wrapper carrying a
    // "render model" reference, so preview the tag itself.
    let group = model_tag.header.group_tag.to_be_bytes();
    if matches!(&group, b"mode" | b"mod2") {
        let preview = build_render_preview(model_tag)?;
        if preview.batches.is_empty() {
            return Err("This render tag has no previewable draw batches.".to_owned());
        }
        let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
        let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
        return Ok(ModelPreviewData {
            source_key: entry.key.clone(),
            render_model_path: entry.display_path.clone(),
            preview,
            draw_triangles,
            variants: Vec::new(),
        });
    }

    let Some((group_tag, rel_path)) = model_tag.root().read_tag_ref_with_group("render model")
    else {
        return Err("This model tag has no render model reference.".to_owned());
    };
    if rel_path.trim().is_empty() {
        return Err("This model tag has an empty render model reference.".to_owned());
    }
    let Some(TagSource::LooseFolder { root, .. }) = source else {
        return Err("Render model preview requires a loaded loose-folder editing kit.".to_owned());
    };
    let extension = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
        .unwrap_or("render_model");
    let mut normalized = rel_path.replace('/', "\\");
    if let Some(stripped) = normalized.strip_suffix(&format!(".{extension}")) {
        normalized = stripped.to_owned();
    }
    let path = resolve_tag_path(root, &normalized, extension);
    if !path.exists() {
        return Err(format!(
            "Referenced render_model was not found: {}",
            path.display()
        ));
    }
    let render_entry = TagEntry {
        key: format!("file:{}", path.display()),
        display_path: format!("{}.{}", normalized.replace('\\', "/"), extension),
        group_tag,
        group_name: names.name_for(group_tag).map(str::to_owned),
        location: TagEntryLocation::LooseFile(path),
    };
    let render_tag =
        read_entry(source.unwrap(), &render_entry).map_err(|error| error.to_string())?;
    let preview = build_render_preview(&render_tag)?;
    if preview.batches.is_empty() {
        return Err("Referenced render_model has no previewable draw batches.".to_owned());
    }
    let max_preview_edge = preview_edge_limit(preview.bounds_min, preview.bounds_max);
    let draw_triangles = build_model_source_triangles(&preview, max_preview_edge);
    Ok(ModelPreviewData {
        source_key: render_entry.key,
        render_model_path: normalized,
        preview,
        draw_triangles,
        variants: read_model_variants(model_tag),
    })
}

/// Build preview geometry from a render-geometry tag — a `render_model`
/// (`mode`), a Halo CE `gbxmodel` (`mod2`), or a Halo 2 `render_model`.
///
/// One tag→geometry path for every engine: blam-tags' `RenderModel::from_tag`
/// / `derive_render_meshes` game-dispatch (H3 reads `render geometry`, H2 the
/// `sections`, Halo CE the gbxmodel `geometries`), so batches carry the render
/// model's own region/permutation names and stay in sync with the variant
/// selection. JMS is export-only — never used for rendering.
fn build_render_preview(render_tag: &TagFile) -> Result<RenderModelPreview, String> {
    let render_model = RenderModel::from_tag(render_tag).map_err(|error| error.to_string())?;
    let render_meshes =
        RenderModel::derive_render_meshes(render_tag).map_err(|error| error.to_string())?;
    Ok(render_model_to_preview(&render_model, &render_meshes))
}

fn expand_preview_bounds_local(min: &mut [f32; 3], max: &mut [f32; 3], point: [f32; 3]) {
    for axis in 0..3 {
        min[axis] = min[axis].min(point[axis]);
        max[axis] = max[axis].max(point[axis]);
    }
}

struct RawVariantRegion {
    perm: Option<String>,
    parent: i128,
}
struct RawVariant {
    name: String,
    regions: Vec<(String, RawVariantRegion)>,
}

/// Resolve a region's effective permutation for variant `vi`, following the
/// per-region `parent variant` chain when the variant doesn't set it directly.
fn resolve_variant_region(
    raw: &[RawVariant],
    vi: usize,
    region: &str,
    depth: usize,
) -> Option<String> {
    if depth > raw.len() {
        return None; // cycle guard
    }
    let rr = raw
        .get(vi)?
        .regions
        .iter()
        .find(|(name, _)| name == region)
        .map(|(_, r)| r)?;
    if let Some(perm) = &rr.perm {
        return Some(perm.clone());
    }
    if rr.parent >= 0 && rr.parent as usize != vi {
        return resolve_variant_region(raw, rr.parent as usize, region, depth + 1);
    }
    None
}

fn read_model_variants(tag: &TagFile) -> Vec<ModelVariantPreview> {
    let Some(variants) = tag.root().field_path("variants").and_then(|f| f.as_block()) else {
        return Vec::new();
    };
    // Pass 1: read each variant's raw region entries (own perm + parent index).
    let mut raw: Vec<RawVariant> = Vec::with_capacity(variants.len());
    for index in 0..variants.len() {
        let Some(variant) = variants.element(index) else {
            continue;
        };
        let name =
            read_named_string_exact(&variant, "name").unwrap_or_else(|| format!("variant {index}"));
        let mut regions = Vec::new();
        if let Some(region_block) = variant.field("regions").and_then(|f| f.as_block()) {
            for region_index in 0..region_block.len() {
                let Some(region) = region_block.element(region_index) else {
                    continue;
                };
                let Some(region_name) = read_named_string_exact(&region, "region name") else {
                    continue;
                };
                let perm = region
                    .field("permutations")
                    .and_then(|f| f.as_block())
                    .and_then(|perms| perms.element(0))
                    .and_then(|perm| read_named_string_exact(&perm, "permutation name"));
                let parent = region.read_int_any("parent variant").unwrap_or(-1);
                regions.push((region_name, RawVariantRegion { perm, parent }));
            }
        }
        raw.push(RawVariant { name, regions });
    }
    // Pass 2: resolve each region through the parent chain into a flat map.
    let mut out = Vec::with_capacity(raw.len());
    for vi in 0..raw.len() {
        let mut regions = HashMap::new();
        let mut listed_regions = std::collections::HashSet::new();
        for (region_name, _) in &raw[vi].regions {
            listed_regions.insert(region_name.clone());
            if regions.contains_key(region_name) {
                continue;
            }
            if let Some(perm) = resolve_variant_region(&raw, vi, region_name, 0) {
                regions.insert(region_name.clone(), perm);
            }
        }
        out.push(ModelVariantPreview {
            name: raw[vi].name.clone(),
            regions,
            listed_regions,
        });
    }
    out
}

/// The variant to select on load: one literally named `default`, else the first
/// (the canonical entry — `chief`, `minor`, `minor_scl`). `None` for models with
/// no variants (CE gbxmodels), which fall back to base permutations.
fn default_variant_index(variants: &[ModelVariantPreview]) -> Option<usize> {
    if variants.is_empty() {
        return None;
    }
    variants
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case("default"))
        .or(Some(0))
}

fn read_named_string_exact(tag_struct: &TagStruct<'_>, expected: &str) -> Option<String> {
    for field in tag_struct.fields() {
        let name = field.name();
        if field_name_matches(name, expected) {
            match field.value()? {
                TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => {
                    if !id.string.is_empty() {
                        return Some(id.string);
                    }
                }
                TagFieldData::String(value) | TagFieldData::LongString(value) => {
                    if !value.is_empty() {
                        return Some(value);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn field_name_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
        || clean_field_name(actual).eq_ignore_ascii_case(expected)
        || clean_field_name_basic(actual).eq_ignore_ascii_case(expected)
}

fn reset_model_preview_selection(
    state: &mut ModelPreviewState,
    data: &ModelPreviewData,
    variant: Option<usize>,
) {
    state.selected_variant = variant;
    state.region_selections = compute_variant_selection(data, variant);
}

/// The region selection (enabled + permutation per region) that selecting
/// `variant` (`None` = base/`<None>`) produces. Pure — used both to apply a
/// variant and to reverse-detect which variant the live selection matches.
fn compute_variant_selection(
    data: &ModelPreviewData,
    variant: Option<usize>,
) -> HashMap<String, ModelRegionSelection> {
    let mut selections = HashMap::new();
    let selected_variant = variant.and_then(|idx| data.variants.get(idx));
    let variant_aliases = selected_variant
        .map(|variant| variant_permutation_aliases(&variant.name))
        .unwrap_or_default();
    for region in &data.preview.regions {
        let default_perm = region.permutations.first().cloned().unwrap_or_default();
        let variant_perm = selected_variant.and_then(|v| v.regions.get(&region.name));
        let alias_perm = matching_variant_permutation(region, &variant_aliases);
        let explicit_perm = variant_perm
            .filter(|name| region.permutations.iter().any(|p| p == *name))
            .cloned();
        // Whether the variant's region block NAMES this region at all. A listed
        // region with no resolvable permutation is explicitly REMOVED (hidden); an
        // UNLISTED region is just uncustomised and falls back to the base
        // permutation (shown) — e.g. the major elite doesn't list `helmet`, so it
        // gets the base helmet, while spec-ops lists it empty to drop it.
        let listed = selected_variant.is_some_and(|v| v.listed_regions.contains(&region.name));
        let permutation = alias_perm
            .clone()
            .or(explicit_perm.clone())
            .unwrap_or(default_perm);
        let enabled = match selected_variant {
            Some(_) => {
                if alias_perm.is_some() || explicit_perm.is_some() {
                    true // the variant provides a permutation for this region
                } else if listed {
                    false // listed with an empty permutation => explicitly removed
                } else {
                    !region.permutations.is_empty() // uncustomised => base, shown
                }
            }
            None => !region.permutations.is_empty(),
        };
        selections.insert(
            region.name.clone(),
            ModelRegionSelection {
                enabled,
                permutation,
            },
        );
    }
    selections
}

/// Reverse-sync: which variant choice (`None` = `<None>`/base, `Some(idx)` =
/// a named variant) the live region selection currently matches, or `None` if
/// it matches no known variant ("(custom)"). Checks the active selection first
/// so an exact match stays put.
fn detect_active_variant(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<Option<usize>> {
    let matches = |choice: Option<usize>| {
        let computed = compute_variant_selection(data, choice);
        computed.len() == state.region_selections.len()
            && computed
                .iter()
                .all(|(name, sel)| state.region_selections.get(name) == Some(sel))
    };
    let current = state.selected_variant;
    if matches(current) {
        return Some(current);
    }
    if current.is_some() && matches(None) {
        return Some(None);
    }
    for idx in 0..data.variants.len() {
        let choice = Some(idx);
        if choice != current && matches(choice) {
            return Some(choice);
        }
    }
    None // matches no known variant → "(custom)"
}

fn matching_variant_permutation(
    region: &RenderModelPreviewRegion,
    aliases: &[String],
) -> Option<String> {
    aliases.iter().find_map(|alias| {
        region
            .permutations
            .iter()
            .find(|permutation| permutation.eq_ignore_ascii_case(alias))
            .cloned()
    })
}

fn variant_permutation_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    push_unique_alias(&mut aliases, name);
    if let Some((base, _)) = name.rsplit_once('_') {
        push_unique_alias(&mut aliases, base);
    }
    aliases
}

fn push_unique_alias(aliases: &mut Vec<String>, alias: &str) {
    let alias = alias.trim();
    if alias.is_empty() {
        return;
    }
    if !aliases
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(alias))
    {
        aliases.push(alias.to_owned());
    }
}

fn draw_variant_controls(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    let mut mutation_requested = false;
    // Reverse-sync: reflect manual region/permutation tweaks in the combo —
    // show the matching variant, or "(custom)" when the live selection matches
    // none. When it lands exactly on a variant, adopt it so Update/Drop target
    // the shown variant.
    let active_variant = detect_active_variant(data, state);
    if let Some(choice) = active_variant {
        state.selected_variant = choice;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Variant").color(subtle_dark()));
        let selected = match active_variant {
            Some(None) => "<None>",
            Some(Some(idx)) => data
                .variants
                .get(idx)
                .map(|variant| variant.name.as_str())
                .unwrap_or("<None>"),
            None => "(custom)",
        };
        let (_, wheel_delta) = combo_box_with_scroll(
            ui,
            egui::ComboBox::from_id_salt(("model_preview_variant", &data.source_key))
                .selected_text(selected)
                .width(180.0),
            |ui| {
                if ui
                    .selectable_label(state.selected_variant.is_none(), "<None>")
                    .clicked()
                {
                    reset_model_preview_selection(state, data, None);
                }
                for index in 0..data.variants.len() {
                    if ui
                        .selectable_label(
                            state.selected_variant == Some(index),
                            &data.variants[index].name,
                        )
                        .clicked()
                    {
                        reset_model_preview_selection(state, data, Some(index));
                    }
                }
            },
        );
        if let Some(delta) = wheel_delta {
            let current = state
                .selected_variant
                .map(|index| index as i32 + 1)
                .unwrap_or(0);
            if let Some(next) =
                combo_scroll_next_index(current as usize, data.variants.len() + 1, delta)
            {
                let selected_variant = if next == 0 { None } else { Some(next - 1) };
                reset_model_preview_selection(state, data, selected_variant);
            }
        }
    });
    ui.add_space(6.0);

    egui::ScrollArea::vertical()
        .id_salt(("model_preview_regions", &data.source_key))
        .max_height(230.0)
        .show(ui, |ui| {
            for region in &data.preview.regions {
                let selection = state
                    .region_selections
                    .entry(region.name.clone())
                    .or_insert_with(|| ModelRegionSelection {
                        enabled: !region.permutations.is_empty(),
                        permutation: region.permutations.first().cloned().unwrap_or_default(),
                    });
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut selection.enabled, "");
                    ui.label(RichText::new(&region.name).color(text_dark()).strong());
                    for permutation in &region.permutations {
                        let selected = selection.permutation == *permutation;
                        let response = ui.selectable_label(selected, permutation);
                        if response.clicked() {
                            selection.permutation = permutation.clone();
                            selection.enabled = true;
                        }
                    }
                });
            }
        });

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("New variant").color(subtle_dark()));
        ui.add_enabled(
            edit.editable,
            egui::TextEdit::singleline(&mut state.new_variant_name).desired_width(130.0),
        );
        let chosen_regions = selected_variant_regions(data, state);
        let create_name = normalized_new_variant_name(data, state);
        let can_create = edit.editable && create_name.is_some() && !chosen_regions.is_empty();
        if ui
            .add_enabled(
                can_create,
                egui::Button::new("Create new variant from selection..."),
            )
            .on_hover_text("Create a .model variant using the visible region selections.")
            .clicked()
        {
            let name = create_name.expect("button enabled only when name is valid");
            edit.model_variant_ops.push(ModelVariantOp::Create {
                name,
                regions: chosen_regions.clone(),
            });
            state.new_variant_name.clear();
            mutation_requested = true;
        }
        let can_update =
            edit.editable && state.selected_variant.is_some() && !chosen_regions.is_empty();
        if ui
            .add_enabled(
                can_update,
                egui::Button::new("Update existing variant from selection..."),
            )
            .on_hover_text("Replace the selected variant's region permutations.")
            .clicked()
        {
            edit.model_variant_ops.push(ModelVariantOp::Update {
                variant_index: state
                    .selected_variant
                    .expect("button enabled only when a variant is selected"),
                regions: chosen_regions,
            });
            mutation_requested = true;
        }
        let can_drop = edit.editable && state.selected_variant.is_some();
        if ui
            .add_enabled(can_drop, egui::Button::new("Drop Variant"))
            .on_hover_text("Delete the selected variant from the .model tag.")
            .clicked()
        {
            edit.model_variant_ops.push(ModelVariantOp::Drop {
                variant_index: state
                    .selected_variant
                    .expect("button enabled only when a variant is selected"),
            });
            state.selected_variant = None;
            mutation_requested = true;
        }
    });
    mutation_requested
}

fn selected_variant_regions(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Vec<ModelVariantRegionChoice> {
    data.preview
        .regions
        .iter()
        .filter_map(|region| {
            let selection = state.region_selections.get(&region.name)?;
            if !selection.enabled
                || selection.permutation.is_empty()
                || !region
                    .permutations
                    .iter()
                    .any(|p| p == &selection.permutation)
            {
                return None;
            }
            Some(ModelVariantRegionChoice {
                region_name: region.name.clone(),
                permutation_name: selection.permutation.clone(),
            })
        })
        .collect()
}

fn normalized_new_variant_name(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
) -> Option<String> {
    let name = state.new_variant_name.trim();
    if name.is_empty() {
        return None;
    }
    if data
        .variants
        .iter()
        .any(|variant| variant.name.eq_ignore_ascii_case(name))
    {
        return None;
    }
    Some(name.to_owned())
}

fn draw_model_viewport(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(228, 238, 244));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));

    if response.dragged_by(egui::PointerButton::Middle) {
        state.pan += response.drag_delta();
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        if ui.input(|i| i.modifiers.shift) {
            state.pan += delta;
        } else {
            state.yaw += delta.x * 0.01;
            state.pitch = (state.pitch + delta.y * 0.01).clamp(-1.45, 1.45);
        }
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            state.scale = (state.scale * (scroll / 450.0).exp()).clamp(0.05, 5.0);
        }
    }

    let camera = PreviewCamera::new(data, state, rect);
    collect_visible_triangles_into(
        data,
        &state.region_selections,
        state.show_backfaces,
        &camera,
        &mut state.projected_triangles,
    );
    state
        .projected_triangles
        .sort_by(|a, b| b.depth.total_cmp(&a.depth));

    let mut mesh = egui::epaint::Mesh::default();
    mesh.vertices.reserve(state.projected_triangles.len() * 3);
    mesh.indices.reserve(state.projected_triangles.len() * 3);
    for tri in &state.projected_triangles {
        let start = mesh.vertices.len() as u32;
        for (point, fill) in tri.points.into_iter().zip(tri.fills) {
            mesh.colored_vertex(point, fill);
        }
        mesh.add_triangle(start, start + 1, start + 2);
    }
    painter.add(egui::Shape::mesh(mesh));

    let wire_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(20, 35, 45, 110));
    let wire_edge_limit = camera.screen_radius() * 0.55;
    if state.show_wireframe {
        for tri in &state.projected_triangles {
            draw_wireframe_edges(&painter, tri.points, wire_edge_limit, wire_stroke);
        }
    }

    if state.show_markers {
        let hover_pos = if response.hovered() {
            ui.input(|i| i.pointer.hover_pos())
        } else {
            None
        };
        let marker_filter = state.marker_filter.trim().to_ascii_lowercase();
        for marker in &data.preview.markers {
            // Name filter (case-insensitive substring; empty = show all).
            if !marker_filter.is_empty()
                && !marker.name.to_ascii_lowercase().contains(&marker_filter)
            {
                continue;
            }
            let projected = camera.project(marker.position);
            let axis_deltas = marker_axis_screen_deltas(&camera, marker.axes);
            draw_marker_axes(&painter, projected.pos, axis_deltas);
            if hover_pos.is_some_and(|pos| marker_axes_hovered(pos, projected.pos, axis_deltas)) {
                let text_pos = projected.pos + Vec2::new(7.0, -7.0);
                let label_rect = egui::Rect::from_min_size(
                    text_pos,
                    Vec2::new(marker.name.len() as f32 * 6.0 + 8.0, 16.0),
                );
                painter.rect_filled(
                    label_rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                );
                painter.text(
                    text_pos + Vec2::new(4.0, 1.0),
                    Align2::LEFT_TOP,
                    &marker.name,
                    FontId::proportional(10.0),
                    Color32::from_rgb(255, 230, 40),
                );
            }
        }
    }
}

const MARKER_AXIS_SCREEN_LENGTH: f32 = 15.0;

fn marker_axis_screen_deltas(camera: &PreviewCamera, axes: [[f32; 3]; 3]) -> [Vec2; 3] {
    axes.map(|axis| {
        let view_axis = camera.rotate_vector(axis);
        let screen = Vec2::new(view_axis[0], -view_axis[2]);
        let len = screen.length();
        if len <= 0.001 {
            Vec2::new(0.0, -MARKER_AXIS_SCREEN_LENGTH * 0.45)
        } else {
            screen / len * MARKER_AXIS_SCREEN_LENGTH
        }
    })
}

fn draw_marker_axes(painter: &egui::Painter, origin: egui::Pos2, axis_deltas: [Vec2; 3]) {
    let colors = [
        Color32::from_rgb(220, 35, 28),
        Color32::from_rgb(20, 180, 45),
        Color32::from_rgb(40, 85, 235),
    ];
    for (delta, color) in axis_deltas.into_iter().zip(colors) {
        let end = origin + delta;
        painter.line_segment(
            [origin, end],
            Stroke::new(2.5, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
        );
        painter.line_segment([origin, end], Stroke::new(1.35, color));
    }
}

fn marker_axes_hovered(pos: egui::Pos2, origin: egui::Pos2, axis_deltas: [Vec2; 3]) -> bool {
    screen_edge_length(pos, origin) <= 7.0
        || axis_deltas
            .into_iter()
            .any(|delta| point_segment_distance(pos, origin, origin + delta) <= 5.0)
}

fn point_segment_distance(point: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let ap = point - a;
    let denom = ab.dot(ab);
    if denom <= f32::EPSILON {
        return screen_edge_length(point, a);
    }
    let t = (ap.dot(ab) / denom).clamp(0.0, 1.0);
    screen_edge_length(point, a + ab * t)
}

fn collect_visible_triangles_into(
    data: &ModelPreviewData,
    region_selections: &HashMap<String, ModelRegionSelection>,
    show_backfaces: bool,
    camera: &PreviewCamera,
    out: &mut Vec<ModelProjectedTriangle>,
) {
    out.clear();
    out.reserve(data.draw_triangles.len());
    for triangle in &data.draw_triangles {
        let Some(batch) = data.preview.batches.get(triangle.batch_index) else {
            continue;
        };
        let Some(selection) = region_selections.get(&batch.region_name) else {
            continue;
        };
        if !selection.enabled || selection.permutation != batch.permutation_name {
            continue;
        }
        let pa = camera.project(triangle.positions[0]);
        let pb = camera.project(triangle.positions[1]);
        let pc = camera.project(triangle.positions[2]);
        if !show_backfaces && projected_signed_area(pa.pos, pb.pos, pc.pos) >= -0.25 {
            continue;
        }
        if projected_max_edge(pa.pos, pb.pos, pc.pos) > camera.screen_radius() * 0.9 {
            continue;
        }
        if !camera.rect.intersects(egui::Rect::from_min_max(
            egui::pos2(
                pa.pos.x.min(pb.pos.x).min(pc.pos.x),
                pa.pos.y.min(pb.pos.y).min(pc.pos.y),
            ),
            egui::pos2(
                pa.pos.x.max(pb.pos.x).max(pc.pos.x),
                pa.pos.y.max(pb.pos.y).max(pc.pos.y),
            ),
        )) {
            continue;
        }
        out.push(ModelProjectedTriangle {
            points: [pa.pos, pb.pos, pc.pos],
            depth: (pa.depth + pb.depth + pc.depth) / 3.0,
            fills: [
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[0])),
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[1])),
                shade_model_color(triangle.fill, camera.rotate_vector(triangle.normals[2])),
            ],
        });
    }
}

fn draw_wireframe_edges(
    painter: &egui::Painter,
    points: [egui::Pos2; 3],
    max_edge: f32,
    stroke: Stroke,
) {
    for (a, b) in [
        (points[0], points[1]),
        (points[1], points[2]),
        (points[2], points[0]),
    ] {
        if screen_edge_length(a, b) <= max_edge {
            painter.line_segment([a, b], stroke);
        }
    }
}

fn projected_signed_area(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn projected_max_edge(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> f32 {
    screen_edge_length(a, b)
        .max(screen_edge_length(b, c))
        .max(screen_edge_length(c, a))
}

fn screen_edge_length(a: egui::Pos2, b: egui::Pos2) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn preview_edge_limit(min: [f32; 3], max: [f32; 3]) -> f32 {
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    let diagonal = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
    diagonal * 0.45
}

fn triangle_max_edge(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    edge_length(a, b)
        .max(edge_length(b, c))
        .max(edge_length(c, a))
}

fn edge_length(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn build_model_source_triangles(
    preview: &RenderModelPreview,
    max_preview_edge: f32,
) -> Vec<ModelSourceTriangle> {
    let mut out = Vec::with_capacity(preview.indices.len() / 3);
    for (batch_index, batch) in preview.batches.iter().enumerate() {
        let start = batch.index_start as usize;
        let end = start
            .saturating_add(batch.index_count as usize)
            .min(preview.indices.len());
        let fill = material_color(batch.material_index);
        for chunk in preview.indices[start..end].chunks_exact(3) {
            let Some(a) = preview.vertices.get(chunk[0] as usize) else {
                continue;
            };
            let Some(b) = preview.vertices.get(chunk[1] as usize) else {
                continue;
            };
            let Some(c) = preview.vertices.get(chunk[2] as usize) else {
                continue;
            };
            let [pa, pb, pc] = [a.position, b.position, c.position];
            let max_edge = triangle_max_edge(pa, pb, pc);
            if max_edge > max_preview_edge {
                continue;
            }
            let face_normal = triangle_normal(pa, pb, pc);
            out.push(ModelSourceTriangle {
                batch_index,
                positions: [pa, pb, pc],
                normals: [
                    usable_normal_or(a.normal, face_normal),
                    usable_normal_or(b.normal, face_normal),
                    usable_normal_or(c.normal, face_normal),
                ],
                fill,
            });
        }
    }
    out
}

fn material_color(index: u16) -> Color32 {
    const COLORS: &[(u8, u8, u8)] = &[
        (132, 168, 188),
        (176, 166, 128),
        (142, 182, 150),
        (180, 136, 134),
        (150, 145, 190),
        (186, 154, 104),
        (126, 174, 176),
    ];
    let (r, g, b) = COLORS[index as usize % COLORS.len()];
    Color32::from_rgb(r, g, b)
}

fn shade_model_color(base: Color32, normal_view: [f32; 3]) -> Color32 {
    let normal = normalize3(normal_view);
    let key = dot3(normal, normalize3([-0.35, -0.55, 0.76])).max(0.0);
    let fill = dot3(normal, normalize3([0.72, 0.22, 0.36])).max(0.0);
    let rim = (1.0 - normal[1].abs()).clamp(0.0, 1.0).powi(2);
    let overhead = (normal[2] * 0.5 + 0.5).clamp(0.0, 1.0);
    let shade = (0.42 + key * 0.46 + fill * 0.16 + rim * 0.10 + overhead * 0.08).clamp(0.32, 1.22);
    let highlight = (key * key * 22.0).clamp(0.0, 24.0);
    Color32::from_rgb(
        shade_channel(base.r(), shade, highlight),
        shade_channel(base.g(), shade, highlight),
        shade_channel(base.b(), shade, highlight),
    )
}

fn shade_channel(value: u8, shade: f32, highlight: f32) -> u8 {
    ((value as f32 * shade + highlight).round()).clamp(0.0, 255.0) as u8
}

fn usable_normal_or(normal: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    if length_squared3(normal) <= 0.0001 {
        return fallback;
    }
    let normalized = normalize3(normal);
    if length_squared3(normalized) > 0.25 {
        normalized
    } else {
        fallback
    }
}

fn triangle_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    normalize3(cross3(sub3(b, a), sub3(c, a)))
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = length_squared3(v).sqrt();
    if len <= f32::EPSILON {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

fn length_squared3(v: [f32; 3]) -> f32 {
    dot3(v, v)
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

struct ProjectedPoint {
    pos: egui::Pos2,
    depth: f32,
}

struct PreviewCamera {
    rect: egui::Rect,
    center: [f32; 3],
    radius: f32,
    yaw: f32,
    pitch: f32,
    scale: f32,
    pan: Vec2,
}

impl PreviewCamera {
    fn new(data: &ModelPreviewData, state: &ModelPreviewState, rect: egui::Rect) -> Self {
        let min = data.preview.bounds_min;
        let max = data.preview.bounds_max;
        let center = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        let extent = [
            (max[0] - min[0]).abs(),
            (max[1] - min[1]).abs(),
            (max[2] - min[2]).abs(),
        ];
        let radius =
            ((extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt() * 0.5)
                .max(0.001);
        Self {
            rect,
            center,
            radius,
            yaw: state.yaw,
            pitch: state.pitch,
            scale: state.scale,
            pan: state.pan,
        }
    }

    fn project(&self, point: [f32; 3]) -> ProjectedPoint {
        let mut x = (point[0] - self.center[0]) * self.scale;
        let mut y = (point[1] - self.center[1]) * self.scale;
        let mut z = (point[2] - self.center[2]) * self.scale;
        let rotated = self.rotate_vector([x, y, z]);
        x = rotated[0];
        y = rotated[1];
        z = rotated[2];
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        let screen = self.rect.center() + self.pan + Vec2::new(x * fit, -z * fit);
        ProjectedPoint {
            pos: screen,
            depth: y,
        }
    }

    fn rotate_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        let mut x = vector[0];
        let mut y = vector[1];
        let mut z = vector[2];
        let (sy, cy) = self.yaw.sin_cos();
        let yaw_x = x * cy - y * sy;
        let yaw_y = x * sy + y * cy;
        x = yaw_x;
        y = yaw_y;
        let (sp, cp) = self.pitch.sin_cos();
        let pitch_y = y * cp - z * sp;
        let pitch_z = y * sp + z * cp;
        y = pitch_y;
        z = pitch_z;
        [x, y, z]
    }

    fn screen_radius(&self) -> f32 {
        let fit = self.rect.width().min(self.rect.height()) / (self.radius * 2.2).max(0.001);
        self.radius * self.scale * fit
    }
}

/// Build flat preview geometry with draw batches grouped by region and
/// permutation. Ported from blam-tags so the GUI owns its preview type; the
/// render meshes are derived separately via `RenderModel::derive_render_meshes`.
fn render_model_to_preview(
    model: &RenderModel,
    render_meshes: &[RenderMesh],
) -> RenderModelPreview {
    let node_world = preview_node_world_transforms(&model.nodes);
    let mut preview = RenderModelPreview {
        regions: model
            .regions
            .iter()
            .map(|region| RenderModelPreviewRegion {
                name: region.name.clone(),
                permutations: region
                    .permutations
                    .iter()
                    .map(|permutation| permutation.name.clone())
                    .collect(),
            })
            .collect(),
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };

    for region in &model.regions {
        for permutation in &region.permutations {
            let first_mesh = permutation.mesh_index.max(0) as usize;
            let mesh_count = permutation.mesh_count.max(0) as usize;
            for mesh_index in first_mesh..first_mesh.saturating_add(mesh_count) {
                let Some(mesh) = render_meshes.get(mesh_index) else {
                    continue;
                };
                for part in &mesh.parts {
                    let index_start = preview.indices.len() as u32;
                    for source_index in
                        part.index_start..part.index_start.saturating_add(part.index_count)
                    {
                        let Some(&vertex_index) = mesh.indices.get(source_index as usize) else {
                            continue;
                        };
                        let Some(vertex) = mesh.vertices.get(vertex_index as usize) else {
                            continue;
                        };
                        let position = point3_to_array(vertex.position);
                        let normal = vector3_to_array(vertex.normal);
                        expand_preview_bounds_local(
                            &mut preview.bounds_min,
                            &mut preview.bounds_max,
                            position,
                        );
                        let new_index = preview.vertices.len() as u32;
                        preview
                            .vertices
                            .push(RenderModelPreviewVertex { position, normal });
                        preview.indices.push(new_index);
                    }
                    let index_count = preview.indices.len() as u32 - index_start;
                    if index_count > 0 {
                        preview.batches.push(RenderModelPreviewBatch {
                            region_name: region.name.clone(),
                            permutation_name: permutation.name.clone(),
                            material_index: part.material_index,
                            part_type: part.part_type as i8,
                            index_start,
                            index_count,
                        });
                    }
                }
            }
        }
    }

    if preview.vertices.is_empty() {
        preview.bounds_min = [0.0; 3];
        preview.bounds_max = [0.0; 3];
    }

    for group in &model.marker_groups {
        for marker in &group.markers {
            preview.markers.push(RenderModelPreviewMarker {
                name: group.name.clone(),
                position: transform_preview_marker_position(marker, &node_world),
                axes: transform_preview_marker_axes(marker, &node_world),
            });
        }
    }

    preview
}

fn preview_node_world_transforms(nodes: &[Node]) -> Vec<(RealQuaternion, RealPoint3d)> {
    let mut world: Vec<(RealQuaternion, RealPoint3d)> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let local_rot = node.default_rotation.normalized();
        let local_trans = node.default_translation;
        if node.parent_node >= 0
            && let Some((parent_rot, parent_trans)) = world.get(node.parent_node as usize).copied()
        {
            let rot = (parent_rot * local_rot).normalized();
            let trans = parent_trans + (parent_rot * local_trans.as_vector());
            world.push((rot, trans));
            continue;
        }
        world.push((local_rot, local_trans));
    }
    world
}

fn transform_preview_marker_position(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [f32; 3] {
    let local = marker.translation;
    let world = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, trans)| *trans + (*rot * local.as_vector()))
            .unwrap_or(local)
    } else {
        local
    };
    point3_to_array(world)
}

fn transform_preview_marker_axes(
    marker: &Marker,
    node_world: &[(RealQuaternion, RealPoint3d)],
) -> [[f32; 3]; 3] {
    let local_rot = marker.rotation.normalized();
    let world_rot = if marker.node_index >= 0 {
        node_world
            .get(marker.node_index as usize)
            .map(|(rot, _)| (*rot * local_rot).normalized())
            .unwrap_or(local_rot)
    } else {
        local_rot
    };
    [
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 1.0,
                    j: 0.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 1.0,
                    k: 0.0,
                },
        ),
        vector3_to_array(
            world_rot
                * RealVector3d {
                    i: 0.0,
                    j: 0.0,
                    k: 1.0,
                },
        ),
    ]
}

fn point3_to_array(p: RealPoint3d) -> [f32; 3] {
    [p.x, p.y, p.z]
}

fn vector3_to_array(v: RealVector3d) -> [f32; 3] {
    [v.i, v.j, v.k]
}
