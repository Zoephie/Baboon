//! Reading a model's variants and instance groups off its tags.

use super::super::common::fields::read_named_string_exact;
use super::super::{HashMap, ModelVariantPreview, TagFile, TagStruct};

/// One permutation a variant's region may take. `None` names nothing, which
/// leaves the region undrawn.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct RawVariantEntry {
    pub(in crate::app::model_preview) permutation: Option<String>,
    pub(in crate::app::model_preview) probability: f32,
}

pub(in crate::app::model_preview) struct RawVariantRegion {
    pub(in crate::app::model_preview) entries: Vec<RawVariantEntry>,
    pub(in crate::app::model_preview) parent: i128,
}

pub(in crate::app::model_preview) struct RawVariant {
    pub(in crate::app::model_preview) name: String,
    pub(in crate::app::model_preview) regions: Vec<(String, RawVariantRegion)>,
    pub(in crate::app::model_preview) muted_nodes: Vec<String>,
}

/// The entries of a region in variant `vi`, with those of its `parent
/// variant` it doesn't name itself, as `model_postprocess_variant_inheritance`
/// merges them. `None` when the variant doesn't list the region.
pub(in crate::app::model_preview) fn variant_region_entries(
    raw: &[RawVariant],
    vi: usize,
    region: &str,
    seen: &mut Vec<usize>,
) -> Option<Vec<RawVariantEntry>> {
    let own = raw
        .get(vi)?
        .regions
        .iter()
        .find(|(name, _)| name == region)
        .map(|(_, own)| own)?;
    let mut entries = own.entries.clone();
    seen.push(vi);
    if let Ok(parent) = usize::try_from(own.parent)
        && !seen.contains(&parent)
    {
        for entry in variant_region_entries(raw, parent, region, seen).unwrap_or_default() {
            if !entries
                .iter()
                .any(|known| known.permutation == entry.permutation)
            {
                entries.push(entry);
            }
        }
    }
    Some(entries)
}

/// The game draws one of these at random by probability. A preview keeps to
/// the likeliest, the earliest of equals, so a variant always looks the same.
fn likeliest_entry(entries: &[RawVariantEntry]) -> Option<&RawVariantEntry> {
    entries
        .iter()
        .rev()
        .max_by(|a, b| a.probability.total_cmp(&b.probability))
}

/// What a permutation entry draws: a state it starts in if it has any likely
/// ones, otherwise its own permutation.
fn read_variant_entry(permutation: &TagStruct<'_>) -> RawVariantEntry {
    let state = permutation
        .field("states")
        .and_then(|field| field.as_block())
        .and_then(|states| {
            (0..states.len())
                .filter_map(|index| states.element(index))
                .map(|state| (state.read_real("initial probability").unwrap_or(0.0), state))
                .filter(|(probability, _)| *probability > 0.0)
                .rev()
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, state)| state)
        });
    RawVariantEntry {
        permutation: read_named_string_exact(
            state.as_ref().unwrap_or(permutation),
            "permutation name",
        ),
        probability: permutation.read_real("probability").unwrap_or(0.0),
    }
}

pub(in crate::app::model_preview) fn read_model_variants(
    tag: &TagFile,
) -> Vec<ModelVariantPreview> {
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
        raw.push(RawVariant {
            name,
            regions,
            muted_nodes,
        });
    }
    // Pass 2: resolve each region through the parent chain into a flat map.
    let mut out = Vec::with_capacity(raw.len());
    for vi in 0..raw.len() {
        let mut regions = HashMap::new();
        let mut hidden_regions = std::collections::HashSet::new();
        for (region_name, _) in &raw[vi].regions {
            let entries =
                variant_region_entries(&raw, vi, region_name, &mut Vec::new()).unwrap_or_default();
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
pub(in crate::app::model_preview) struct InstanceGroup {
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
    let matching = placements
        .iter()
        .enumerate()
        .filter(|(_, name)| name.contains(&member.instances));
    matching
        .filter(|(index, _)| *index < 128)
        .fold(0, |mask, (index, _)| mask | 1 << index)
}

/// `model_instance_group_choose_instances`, which rolls for a `choose one`
/// group. A preview shows its likeliest member, the first breaking a tie.
fn instance_group_mask(
    groups: &[InstanceGroup],
    group: usize,
    placements: &[String],
    depth: usize,
) -> u128 {
    let Some(group) = groups.get(group).filter(|_| depth <= groups.len()) else {
        return 0;
    };
    let likeliest = group
        .members
        .iter()
        .filter(|member| member.probability > 0.0)
        .fold(None::<&InstanceGroupMember>, |best, member| match best {
            Some(best) if best.probability >= member.probability => Some(best),
            _ => Some(member),
        });
    let chosen: Vec<_> = if group.choose_all {
        group.members.iter().collect()
    } else {
        likeliest.into_iter().collect()
    };
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
fn variant_instance_mask(
    groups: &[InstanceGroup],
    group: Option<usize>,
    placements: &[String],
    reach: bool,
) -> u128 {
    match group {
        Some(group) => instance_group_mask(groups, group, placements, 0),
        None if reach => u128::MAX,
        None => 0,
    }
}

/// What each variant chooses of `placements`, in the tag's order.
pub(in crate::app::model_preview) fn read_variant_instances(
    tag: &TagFile,
    placements: &[String],
    reach: bool,
) -> Vec<u128> {
    let root = tag.root();
    let Some(variants) = root
        .field_path("variants")
        .and_then(|field| field.as_block())
    else {
        return Vec::new();
    };
    let groups: Vec<InstanceGroup> =
        super::super::common::fields::elements(root, "instance groups")
            .iter()
            .map(|group| InstanceGroup {
                choose_all: group
                    .field("choice")
                    .is_some_and(|choice| choice.enum_is("choose all members")),
                members: super::super::common::fields::elements(*group, "member list")
                    .iter()
                    .map(|member| InstanceGroupMember {
                        subgroup: usize::try_from(member.read_int_any("subgroup").unwrap_or(-1))
                            .ok(),
                        instances: member.read_string_id("instances").unwrap_or_default(),
                        probability: member.read_real("probability").unwrap_or(0.0),
                    })
                    .collect(),
            })
            .collect();
    (0..variants.len())
        .map(|index| {
            let group = variants
                .element(index)
                .and_then(|variant| variant.read_int_any("instance group"));
            let group = group.and_then(|group| usize::try_from(group).ok());
            variant_instance_mask(&groups, group, placements, reach)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(permutation: Option<&str>, probability: f32) -> RawVariantEntry {
        RawVariantEntry {
            permutation: permutation.map(str::to_owned),
            probability,
        }
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
            variant(
                "hull",
                vec![entry(Some("base"), 1.0), entry(Some("rust"), 3.0)],
                -1,
            ),
            variant("hull", vec![entry(Some("base"), 2.0)], 0),
            variant("hull", Vec::new(), 2),
        ];
        assert_eq!(
            variant_region_entries(&raw, 1, "hull", &mut Vec::new()).unwrap(),
            vec![entry(Some("base"), 2.0), entry(Some("rust"), 3.0)],
        );
        assert!(variant_region_entries(&raw, 1, "wing", &mut Vec::new()).is_none());
        assert_eq!(
            variant_region_entries(&raw, 2, "hull", &mut Vec::new()),
            Some(Vec::new()),
            "a cycle ends"
        );
    }

    #[test]
    fn a_group_chooses_by_part_of_a_name_through_its_subgroups() {
        let member = |subgroup, instances: &str, probability| InstanceGroupMember {
            subgroup,
            instances: instances.to_owned(),
            probability,
        };
        let placements: Vec<_> = [
            "%shin_guard_left",
            "%shin_guard_right",
            "%hand_guard",
            "%wrist",
        ]
        .map(str::to_owned)
        .into();
        let groups = [
            InstanceGroup {
                choose_all: true,
                members: vec![member(None, "shin_guard", 0.0), member(Some(1), "", 0.0)],
            },
            InstanceGroup {
                choose_all: false,
                members: vec![
                    member(None, "wrist", 0.1),
                    member(None, "hand", 0.6),
                    member(None, "shin", 0.6),
                ],
            },
            InstanceGroup {
                choose_all: false,
                members: vec![member(None, "wrist", 0.0)],
            },
        ];
        assert_eq!(instance_group_mask(&groups, 0, &placements, 0), 0b0111);
        assert_eq!(
            instance_group_mask(&groups, 1, &placements, 0),
            0b0100,
            "the first of the likeliest"
        );
        assert_eq!(
            instance_group_mask(&groups, 2, &placements, 0),
            0,
            "nothing can be rolled"
        );
        assert_eq!(instance_group_mask(&groups, 9, &placements, 0), 0);
    }

    /// A Reach variant with no instance group leaves every placement to its
    /// permutations; Halo 3's takes none; one with a group takes its choice.
    #[test]
    fn a_reach_variant_without_an_instance_group_allows_every_instance() {
        let placements: Vec<_> = ["%shoulder", "%knife"].map(str::to_owned).into();
        let groups = [InstanceGroup {
            choose_all: true,
            members: vec![InstanceGroupMember {
                subgroup: None,
                instances: "knife".to_owned(),
                probability: 0.0,
            }],
        }];
        assert_eq!(
            variant_instance_mask(&groups, None, &placements, true),
            u128::MAX
        );
        assert_eq!(
            variant_instance_mask(&groups, None, &placements, false),
            0,
            "Halo 3 chooses none"
        );
        assert_eq!(
            variant_instance_mask(&groups, Some(0), &placements, true),
            0b10
        );
        assert_eq!(
            variant_instance_mask(&groups, Some(0), &placements, false),
            0b10
        );
    }

    #[test]
    fn the_likeliest_entry_wins_and_the_first_breaks_a_tie() {
        let entries = [
            entry(Some("a"), 1.0),
            entry(Some("b"), 2.0),
            entry(Some("c"), 2.0),
        ];
        assert_eq!(likeliest_entry(&entries), Some(&entries[1]));
        assert_eq!(likeliest_entry(&[]), None);
    }
}
