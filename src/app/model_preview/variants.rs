//! Model variant discovery, selection, and controls.
//! It owns model-preview data preparation and rendering; tag mutation and general editor presentation belong elsewhere.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(permutation: Option<&str>, probability: f32) -> RawVariantEntry {
        RawVariantEntry { permutation: permutation.map(str::to_owned), probability }
    }

    fn variant(region: &str, entries: Vec<RawVariantEntry>, parent: i128) -> RawVariant {
        RawVariant {
            name: String::new(),
            regions: vec![(region.to_owned(), RawVariantRegion { entries, parent })],
            muted_nodes: Vec::new(),
        }
    }

    #[test]
    fn a_child_keeps_its_entries_and_gains_what_its_parent_adds() {
        let raw = [
            variant("hull", vec![entry(Some("base"), 1.0), entry(Some("rust"), 3.0)], -1),
            variant("hull", vec![entry(Some("base"), 2.0)], 0),
            variant("hull", Vec::new(), 2),
        ];
        assert_eq!(
            variant_region_entries(&raw, 1, "hull", &mut Vec::new()).unwrap(),
            vec![entry(Some("base"), 2.0), entry(Some("rust"), 3.0)],
        );
        assert!(variant_region_entries(&raw, 1, "wing", &mut Vec::new()).is_none());
        assert_eq!(variant_region_entries(&raw, 2, "hull", &mut Vec::new()), Some(Vec::new()), "a cycle ends");
    }

    #[test]
    fn a_group_chooses_by_part_of_a_name_through_its_subgroups() {
        let member = |subgroup, instances: &str, probability| InstanceGroupMember {
            subgroup,
            instances: instances.to_owned(),
            probability,
        };
        let placements: Vec<_> =
            ["%shin_guard_left", "%shin_guard_right", "%hand_guard", "%wrist"].map(str::to_owned).into();
        let groups = [
            InstanceGroup {
                choose_all: true,
                members: vec![member(None, "shin_guard", 0.0), member(Some(1), "", 0.0)],
            },
            InstanceGroup {
                choose_all: false,
                members: vec![member(None, "wrist", 0.1), member(None, "hand", 0.6), member(None, "shin", 0.6)],
            },
            InstanceGroup { choose_all: false, members: vec![member(None, "wrist", 0.0)] },
        ];
        assert_eq!(instance_group_mask(&groups, 0, &placements, 0), 0b0111);
        assert_eq!(instance_group_mask(&groups, 1, &placements, 0), 0b0100, "the first of the likeliest");
        assert_eq!(instance_group_mask(&groups, 2, &placements, 0), 0, "nothing can be rolled");
        assert_eq!(instance_group_mask(&groups, 9, &placements, 0), 0);
    }

    /// A Reach variant with no instance group leaves every placement to its
    /// permutations; Halo 3's takes none; one with a group takes its choice.
    #[test]
    fn a_reach_variant_without_an_instance_group_allows_every_instance() {
        let placements: Vec<_> = ["%shoulder", "%knife"].map(str::to_owned).into();
        let groups = [InstanceGroup {
            choose_all: true,
            members: vec![InstanceGroupMember { subgroup: None, instances: "knife".to_owned(), probability: 0.0 }],
        }];
        assert_eq!(variant_instance_mask(&groups, None, &placements, true), u128::MAX);
        assert_eq!(variant_instance_mask(&groups, None, &placements, false), 0, "Halo 3 chooses none");
        assert_eq!(variant_instance_mask(&groups, Some(0), &placements, true), 0b10);
        assert_eq!(variant_instance_mask(&groups, Some(0), &placements, false), 0b10);
    }

    #[test]
    fn the_likeliest_entry_wins_and_the_first_breaks_a_tie() {
        let entries = [entry(Some("a"), 1.0), entry(Some("b"), 2.0), entry(Some("c"), 2.0)];
        assert_eq!(likeliest_entry(&entries), Some(&entries[1]));
        assert_eq!(likeliest_entry(&[]), None);
    }

    #[test]
    fn an_object_s_default_variant_comes_first() {
        let variant = |name: &str| ModelVariantPreview {
            name: name.to_owned(),
            regions: HashMap::new(),
            hidden_regions: Default::default(),
            muted_nodes: Vec::new(),
        };
        let mut data = model_preview_data(
            String::new(),
            String::new(),
            RenderModelPreview::default(),
            vec![variant("first"), variant("default"), variant("chosen")],
        );
        assert_eq!(default_variant_index(&data), Some(1));
        data.default_variant = Some("chosen".to_owned());
        assert_eq!(default_variant_index(&data), Some(2));
        data.default_variant = Some("gone".to_owned());
        assert_eq!(default_variant_index(&data), Some(1));
        data.variants.remove(1);
        assert_eq!(default_variant_index(&data), Some(0));
        data.variants.clear();
        assert_eq!(default_variant_index(&data), None);
    }

    #[test]
    fn a_tilde_permutation_is_never_the_default() {
        let region = |permutations: &[&str]| RenderModelPreviewRegion {
            name: String::new(),
            permutations: permutations.iter().map(|name| (*name).to_owned()).collect(),
        };
        assert_eq!(default_permutation(&region(&["~hidden", "base"])), "base");
        assert_eq!(default_permutation(&region(&["~only"])), "~only");
    }

    #[test]
    fn variant_picker_filters_and_preserves_none_and_original_indices() {
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        ctx.set_global_style(foundation_style());
        let variants = ["minor", "major", "special veteran"]
            .into_iter()
            .map(|name| ModelVariantPreview {
                name: name.to_owned(),
                regions: HashMap::new(),
                hidden_regions: Default::default(),
                muted_nodes: Vec::new(),
            })
            .collect::<Vec<_>>();
        let frame = |events| {
            let mut result = None;
            let mut popup_id = None;
            let output = crate::app::run_ui_test(&ctx, 
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        popup_id = Some(ui.make_persistent_id(("model_preview_variant", "test")));
                        result = Some(draw_variant_picker(
                            ui,
                            "test",
                            "0. minor",
                            &variants,
                            Some(0),
                        ));
                    });
                },
            );
            (result.unwrap().0, output, popup_id.unwrap())
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(Vec::new());
        let pos = egui::pos2(70.0, 18.0);
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        frame(vec![pointer(pos, false)]);
        frame(Vec::new());
        let (_, output, popup_id) = frame(vec![egui::Event::Text("VETERAN".to_owned())]);
        let row_pos = |output: &egui::FullOutput, text: &str| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if shape.galley.text() == text => {
                        Some(shape.pos + shape.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("picker row")
        };
        let pos = row_pos(&output, "2. special veteran");
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        assert_eq!(frame(vec![pointer(pos, false)]).0, Some(Some(2)));
        ctx.data_mut(|data| data.insert_temp(popup_id.with("entry_search"), String::new()));
        egui::Popup::open_id(&ctx, popup_id);
        frame(Vec::new());
        let (_, output, _) = frame(Vec::new());
        let pos = row_pos(&output, "<None>");
        frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        assert_eq!(frame(vec![pointer(pos, false)]).0, Some(None));
    }
}

/// The entries of a region in variant `vi`, with those of its `parent
/// variant` it doesn't name itself, as `model_postprocess_variant_inheritance`
/// merges them. `None` when the variant doesn't list the region.
pub(super) fn variant_region_entries(
    raw: &[RawVariant],
    vi: usize,
    region: &str,
    seen: &mut Vec<usize>,
) -> Option<Vec<RawVariantEntry>> {
    let own = raw.get(vi)?.regions.iter().find(|(name, _)| name == region).map(|(_, own)| own)?;
    let mut entries = own.entries.clone();
    seen.push(vi);
    if let Ok(parent) = usize::try_from(own.parent)
        && !seen.contains(&parent)
    {
        for entry in variant_region_entries(raw, parent, region, seen).unwrap_or_default() {
            if !entries.iter().any(|known| known.permutation == entry.permutation) {
                entries.push(entry);
            }
        }
    }
    Some(entries)
}

/// The game draws one of these at random by probability. A preview keeps to
/// the likeliest, the earliest of equals, so a variant always looks the same.
fn likeliest_entry(entries: &[RawVariantEntry]) -> Option<&RawVariantEntry> {
    entries.iter().rev().max_by(|a, b| a.probability.total_cmp(&b.probability))
}

/// What a permutation entry draws: a state it starts in if it has any likely
/// ones, otherwise its own permutation.
fn read_variant_entry(permutation: &TagStruct<'_>) -> RawVariantEntry {
    let state = permutation.field("states").and_then(|field| field.as_block()).and_then(|states| {
        (0..states.len())
            .filter_map(|index| states.element(index))
            .map(|state| (state.read_real("initial probability").unwrap_or(0.0), state))
            .filter(|(probability, _)| *probability > 0.0)
            .rev()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, state)| state)
    });
    RawVariantEntry {
        permutation: read_named_string_exact(state.as_ref().unwrap_or(permutation), "permutation name"),
        probability: permutation.read_real("probability").unwrap_or(0.0),
    }
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
                let entries = region
                    .field("permutations")
                    .and_then(|f| f.as_block())
                    .map(|block| {
                        (0..block.len())
                            .filter_map(|index| block.element(index))
                            .map(|entry| read_variant_entry(&entry))
                            .collect()
                    })
                    .unwrap_or_default();
                let parent = region.read_int_any("parent variant").unwrap_or(-1);
                regions.push((region_name, RawVariantRegion { entries, parent }));
            }
        }
        // Halo 4's; no earlier game has the block.
        let muted_nodes = variant
            .field("muted nodes")
            .and_then(|field| field.as_block())
            .map(|block| {
                (0..block.len())
                    .filter_map(|index| block.element(index))
                    .filter_map(|entry| entry.read_string_id("node name"))
                    .filter(|name| !name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        raw.push(RawVariant { name, regions, muted_nodes });
    }
    // Pass 2: resolve each region through the parent chain into a flat map.
    let mut out = Vec::with_capacity(raw.len());
    for vi in 0..raw.len() {
        let mut regions = HashMap::new();
        let mut hidden_regions = std::collections::HashSet::new();
        for (region_name, _) in &raw[vi].regions {
            let entries = variant_region_entries(&raw, vi, region_name, &mut Vec::new()).unwrap_or_default();
            // A region with no entries at all is not this variant's to choose.
            match likeliest_entry(&entries).map(|entry| entry.permutation.clone()) {
                Some(Some(permutation)) => {
                    regions.insert(region_name.clone(), permutation);
                }
                Some(None) => {
                    hidden_regions.insert(region_name.clone());
                }
                None => {}
            }
        }
        out.push(ModelVariantPreview {
            name: raw[vi].name.clone(),
            regions,
            hidden_regions,
            muted_nodes: raw[vi].muted_nodes.clone(),
        });
    }
    out
}

/// One `instance groups` entry of a `.model`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct InstanceGroup {
    choose_all: bool,
    members: Vec<InstanceGroupMember>,
}

#[derive(Debug, Clone, PartialEq)]
struct InstanceGroupMember {
    subgroup: Option<usize>,
    instances: String,
    probability: f32,
}

/// The tag's masks are left to `tool`, which matches a member's name anywhere
/// in a placement's.
fn member_mask(member: &InstanceGroupMember, placements: &[String]) -> u128 {
    if member.instances.is_empty() {
        return 0;
    }
    let matching = placements.iter().enumerate().filter(|(_, name)| name.contains(&member.instances));
    matching.filter(|(index, _)| *index < 128).fold(0, |mask, (index, _)| mask | 1 << index)
}

/// `model_instance_group_choose_instances`, which rolls for a `choose one`
/// group. A preview shows its likeliest member, the first breaking a tie.
fn instance_group_mask(groups: &[InstanceGroup], group: usize, placements: &[String], depth: usize) -> u128 {
    let Some(group) = groups.get(group).filter(|_| depth <= groups.len()) else {
        return 0;
    };
    let likeliest = group.members.iter().filter(|member| member.probability > 0.0).fold(
        None::<&InstanceGroupMember>,
        |best, member| match best {
            Some(best) if best.probability >= member.probability => Some(best),
            _ => Some(member),
        },
    );
    let chosen: Vec<_> = if group.choose_all { group.members.iter().collect() } else { likeliest.into_iter().collect() };
    chosen.iter().fold(0, |mask, member| {
        let subgroup = member.subgroup.map_or(0, |subgroup| {
            instance_group_mask(groups, subgroup, placements, depth + 1)
        });
        mask | subgroup | member_mask(member, placements)
    })
}

/// What a variant's instance group chooses. With none, Reach's
/// `model_instance_group_choose_instances` (X360 0x8320E3A8) sets every bit,
/// leaving the permutations' masks to decide
/// (`model_compute_instance_bitfield_visible`); Halo 3's
/// `model_select_initial_instances` (March 2007 X360 0x82BE8720) leaves none.
fn variant_instance_mask(groups: &[InstanceGroup], group: Option<usize>, placements: &[String], reach: bool) -> u128 {
    match group {
        Some(group) => instance_group_mask(groups, group, placements, 0),
        None if reach => u128::MAX,
        None => 0,
    }
}

/// What each variant chooses of `placements`, in the tag's order.
pub(super) fn read_variant_instances(tag: &TagFile, placements: &[String], reach: bool) -> Vec<u128> {
    let root = tag.root();
    let Some(variants) = root.field_path("variants").and_then(|field| field.as_block()) else {
        return Vec::new();
    };
    let groups: Vec<InstanceGroup> = super::ce::attachments::elements(root, "instance groups")
        .iter()
        .map(|group| InstanceGroup {
            choose_all: group.read_enum_name("choice").as_deref() == Some("choose all members"),
            members: super::ce::attachments::elements(*group, "member list")
                .iter()
                .map(|member| InstanceGroupMember {
                    subgroup: usize::try_from(member.read_int_any("subgroup").unwrap_or(-1)).ok(),
                    instances: member.read_string_id("instances").unwrap_or_default(),
                    probability: member.read_real("probability").unwrap_or(0.0),
                })
                .collect(),
        })
        .collect();
    (0..variants.len())
        .map(|index| {
            let group = variants.element(index).and_then(|variant| variant.read_int_any("instance group"));
            let group = group.and_then(|group| usize::try_from(group).ok());
            variant_instance_mask(&groups, group, placements, reach)
        })
        .collect()
}

/// The variant to select on load: the one an object names, else one literally
/// named `default`, else the first (the canonical entry — `chief`, `minor`,
/// `minor_scl`). `None` for models with no variants (CE gbxmodels), which fall
/// back to base permutations.
pub(super) fn default_variant_index(data: &ModelPreviewData) -> Option<usize> {
    let named = |name: &str| data.variants.iter().position(|v| v.name.eq_ignore_ascii_case(name));
    data.default_variant
        .as_deref()
        .and_then(named)
        .or_else(|| named("default"))
        .or((!data.variants.is_empty()).then_some(0))
}

pub(super) fn read_named_string_exact(
    tag_struct: &TagStruct<'_>,
    expected: &str,
) -> Option<String> {
    for field in tag_struct.fields() {
        let name = field.name();
        if field_name_matches(name, expected) {
            match field.value()? {
                TagFieldData::StringId(id) | TagFieldData::OldStringId(id) if !id.string.is_empty() => {
                    return Some(id.string);
                }
                TagFieldData::String(value) | TagFieldData::LongString(value) if !value.is_empty() => {
                    return Some(value);
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
    for region in &data.preview.regions {
        let named = selected_variant.and_then(|variant| variant.regions.get(&region.name));
        let hidden = selected_variant.is_some_and(|variant| variant.hidden_regions.contains(&region.name));
        let (enabled, permutation) = match named {
            Some(name) if region.permutations.contains(name) => (true, name.clone()),
            // The game resolves a name the render model lacks to no permutation.
            Some(_) => (false, default_permutation(region)),
            None => {
                let empty = data.preview.instances.empty_regions.contains(&region.name);
                (!hidden && !empty && !region.permutations.is_empty(), default_permutation(region))
            }
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

/// What `model_select_initial_permutation` falls back on: an ordinary
/// permutation, as a name starting `~` keeps one out of its choosing.
fn default_permutation(region: &RenderModelPreviewRegion) -> String {
    region
        .permutations
        .iter()
        .find(|permutation| !permutation.starts_with('~'))
        .or(region.permutations.first())
        .cloned()
        .unwrap_or_default()
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

pub(super) fn draw_variant_controls(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    let region_name_width = data
        .preview
        .regions
        .iter()
        .filter(|region| !is_model_physics_overlay_region(data, state, region))
        .map(|region| {
            ui.painter()
                .layout_no_wrap(region.name.clone(), bold_font(13.0), text_dark())
                .size()
                .x
        })
        .fold(90.0_f32, f32::max)
        .min(180.0);
    let row_width = ui.available_width().max(1.0);
    ui.set_max_width(row_width);
    let mut visible_rows = 0;
    for region in &data.preview.regions {
        if is_model_physics_overlay_region(data, state, region) {
            continue;
        }
        if visible_rows > 0 {
            ui.separator();
        }
        visible_rows += 1;
        let selection = state
            .region_selections
            .entry(region.name.clone())
            .or_insert_with(|| ModelRegionSelection {
                enabled: !region.permutations.is_empty(),
                permutation: default_permutation(region),
            });
        ui.horizontal_wrapped(|ui| {
            ui.baboon_checkbox(&mut selection.enabled, "");
            ui.add_sized(
                Vec2::new(region_name_width, BUTTON_HEIGHT),
                egui::Label::new(
                    RichText::new(&region.name)
                        .color(text_dark())
                        .font(bold_font(13.0)),
                ),
            );
            for permutation in &region.permutations {
                let selected = selection.permutation == *permutation;
                let response = selectable_text_button(ui, permutation, selected);
                if response.clicked() {
                    selection.permutation = permutation.clone();
                    selection.enabled = true;
                }
            }
        });
    }
}

/// Search variants without losing the `<None>` reset or wheel navigation.
fn draw_variant_picker(
    ui: &mut Ui,
    source_key: &str,
    selected_label: &str,
    variants: &[ModelVariantPreview],
    selected: Option<usize>,
) -> (Option<Option<usize>>, Option<i32>) {
    let popup_id = ui.make_persistent_id(("model_preview_variant", source_key));
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let response = picker_button(
        ui,
        popup_id,
        selected_label,
        variants.len(),
        150.0,
        text_dark(),
        true,
    );
    let just_opened = response.clicked() && !open;
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    let popup_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let mut choice = None;
    // Open while its id is open in memory, as the button above toggles it;
    // left alone, a popup built from a response is always open.
    egui::Popup::from_response(&response)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            picker_popup_width(ui, &response);
            let filter_id = popup_id.with("entry_search");
            let mut filter = ui
                .data(|data| data.get_temp::<String>(filter_id))
                .unwrap_or_default();
            let changed = picker_search_field(ui, &mut filter, "search variants…", just_opened);
            ui.data_mut(|data| data.insert_temp(filter_id, filter.clone()));
            ui.separator();
            let query = filter.trim().to_lowercase();
            picker_results(ui, 300.0, changed, |ui| {
                let mut shown = 0;
                if query.is_empty() || "<none>".contains(&query) {
                    shown += 1;
                    if ui.selectable_label(selected.is_none(), "<None>").clicked() {
                        choice = Some(None);
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                for (index, variant) in variants.iter().enumerate() {
                    let label = format!("{index}. {}", variant.name);
                    if !query.is_empty() && !label.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    let row = ui.selectable_label(selected == Some(index), label);
                    if just_opened && query.is_empty() && selected == Some(index) {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        choice = Some(Some(index));
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                }
                if shown == 0 {
                    ui.label(RichText::new("No variants match.").color(subtle_dark()));
                }
            });
        });
    (choice, dropdown_wheel_delta(ui, &response, popup_open))
}

pub(super) fn draw_variant_selector(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
) {
    // Reverse-sync: reflect manual region/permutation tweaks in the combo —
    // show the matching variant, or "(custom)" when the live selection matches
    // none. When it lands exactly on a variant, adopt it so Update/Drop target
    // the shown variant.
    let mut active_variant = detect_active_variant(data, state);
    if let Some(choice) = active_variant {
        state.selected_variant = choice;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(
            ui,
            ButtonIcon::Left,
            "Previous variant",
            state.selected_variant.is_some_and(|index| index > 0),
            text_dark(),
        )
        .clicked()
        {
            let previous = state.selected_variant.expect("previous variant is enabled") - 1;
            reset_model_preview_selection(state, data, Some(previous));
            active_variant = Some(Some(previous));
        }
        let selected = match active_variant {
            Some(None) => "<None>".to_owned(),
            Some(Some(idx)) => data
                .variants
                .get(idx)
                .map(|variant| format!("{idx}. {}", variant.name))
                .unwrap_or_else(|| "<None>".to_owned()),
            None => "(custom)".to_owned(),
        };
        let (choice, wheel_delta) = draw_variant_picker(
            ui,
            &data.source_key,
            &selected,
            &data.variants,
            state.selected_variant,
        );
        if let Some(choice) = choice {
            reset_model_preview_selection(state, data, choice);
        }
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
        let next = match state.selected_variant {
            Some(index) if index + 1 < data.variants.len() => Some(index + 1),
            None if !data.variants.is_empty() => Some(0),
            _ => None,
        };
        if icon_button(
            ui,
            ButtonIcon::Right,
            "Next variant",
            next.is_some(),
            text_dark(),
        )
        .clicked()
        {
            reset_model_preview_selection(state, data, next);
        }
    });
}

pub(super) fn draw_variant_header_actions(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    let mut mutation_requested = false;
    icon_text_dropdown_button(ui, ButtonIcon::Save, "Save", |ui| {
        ui.set_min_width(240.0);
        ui.label("New Variant Name");
        ui.add_enabled(
            edit.editable,
            crate::app::ui_kit::singleline_text_edit(&mut state.new_variant_name).desired_width(220.0),
        );
        let chosen_regions = selected_variant_regions(data, state);
        let create_name = normalized_new_variant_name(data, state);
        let can_create = edit.editable && create_name.is_some() && !chosen_regions.is_empty();
        if icon_text_button(ui, ButtonIcon::Add, "Save as New Variant", can_create)
            .on_hover_text("Create a .model variant using the visible region selections.")
            .clicked()
        {
            let name = create_name.expect("button enabled only when name is valid");
            state.variant_after_load = Some(name.clone());
            edit.model_variant_ops.push(ModelVariantOp::Create { name, regions: chosen_regions.clone() });
            state.new_variant_name.clear();
            mutation_requested = true;
            close_menu(ui);
        }
        let can_update =
            edit.editable && state.selected_variant.is_some() && !chosen_regions.is_empty();
        if ui
            .add_enabled(can_update, egui::Button::new("Update Selected Variant"))
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
            close_menu(ui);
        }
    });
    let can_delete = edit.editable && state.selected_variant.is_some();
    if icon_text_button(ui, ButtonIcon::Garbage, "Delete", can_delete)
        .on_hover_text("Delete selected variant")
        .clicked()
    {
        let variant_index = state
            .selected_variant
            .expect("button enabled only when a variant is selected");
        *edit.block_confirm = Some(BlockConfirm {
            opened_at: None,
            kit: None,
            tag_key: edit.tag_key.to_owned(),
            path: "variants".to_owned(),
            kind: BlockOpKind::Delete(variant_index),
            message: format!(
                "Delete element {variant_index} of {} from this block?",
                data.variants.len()
            ),
            confirm_label: "Delete".to_owned(),
        });
    }
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
            // Physics is an overlay toggle, not a .model variant region.
            if is_model_physics_overlay_region(data, state, region) {
                return None;
            }
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

/// The physics overlay uses a synthetic `physics/default` draw batch. Hide it
/// from Model Setup while preserving its internal selection for draw filtering.
/// A genuine render region named `physics` (or a standalone physics tag) must
/// still appear normally.
pub(super) fn is_model_physics_overlay_region(
    data: &ModelPreviewData,
    state: &ModelPreviewState,
    region: &RenderModelPreviewRegion,
) -> bool {
    state.overlays_loaded
        && region.name == PHYSICS_REGION
        && data.preview.batches.iter().any(|batch| {
            batch.layer == ModelPreviewLayer::Physics && batch.region_name == region.name
        })
        && !data.preview.batches.iter().any(|batch| {
            batch.layer == ModelPreviewLayer::Render && batch.region_name == region.name
        })
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
