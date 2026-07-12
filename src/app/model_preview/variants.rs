//! Model variant discovery, selection, and controls.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;

/// Resolve a region's effective permutation for variant `vi`, following the
/// per-region `parent variant` chain when the variant doesn't set it directly.
pub(super) fn resolve_variant_region(
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

pub(super) fn read_model_variants(tag: &TagFile) -> Vec<ModelVariantPreview> {
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
pub(super) fn default_variant_index(variants: &[ModelVariantPreview]) -> Option<usize> {
    if variants.is_empty() {
        return None;
    }
    variants
        .iter()
        .position(|v| v.name.eq_ignore_ascii_case("default"))
        .or(Some(0))
}

pub(super) fn read_named_string_exact(
    tag_struct: &TagStruct<'_>,
    expected: &str,
) -> Option<String> {
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

pub(super) fn field_name_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
        || clean_field_name(actual).eq_ignore_ascii_case(expected)
        || clean_field_name_basic(actual).eq_ignore_ascii_case(expected)
}

pub(super) fn reset_model_preview_selection(
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
pub(super) fn compute_variant_selection(
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
pub(super) fn detect_active_variant(
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

pub(super) fn matching_variant_permutation(
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

pub(super) fn variant_permutation_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    push_unique_alias(&mut aliases, name);
    if let Some((base, _)) = name.rsplit_once('_') {
        push_unique_alias(&mut aliases, base);
    }
    aliases
}

pub(super) fn push_unique_alias(aliases: &mut Vec<String>, alias: &str) {
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

pub(super) fn draw_variant_controls(
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

pub(super) fn selected_variant_regions(
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

pub(super) fn normalized_new_variant_name(
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
