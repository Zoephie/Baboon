//! What the Campaign Evolved import gate does with a tag from another game.
//!
//! The gate compared an imported tag against one profile with
//! `compare_root_layout`, which by construction looks at the *root* struct only:
//! group, version, root size, root field list. Anything short of a clean match
//! was treated as one kind of problem -- benign drift the user could wave
//! through with "Import anyway".
//!
//! That conflates two situations which need opposite answers. A tag saved by an
//! older toolset against a drifted Campaign Evolved layout really is safe to
//! wave through, and the override exists for it. A tag authored for *another
//! game* is not, and `model_animation_graph` is the case that proves it.
//!
//! It proves it harder than expected. The Reach and Campaign Evolved jmad root
//! structs are not merely the same size -- they are field-for-field identical
//! once `collect_fields` drops the zero-byte `explanation` and `terminator`
//! sentinels, their only textual difference. So `compare_root_layout` returns
//! `Match`: a Halo Reach animation graph imports into Campaign Evolved under a
//! green "Schema matches" tick, with no warning to wave through and no override
//! to tick. Meanwhile four nested structs are the wrong size --
//! `shared_model_animation_block` 212 vs 200, `animation_graph_node_block` 44 vs
//! 40, `animation_ik_set_item` 4 vs 8, `new_animation_blend_screen_block_struct`
//! 44 vs 48.
//!
//! The consequence for the fix: no amount of root-level comparison can tell the
//! two situations apart, so the classifier asks
//! `blam_tags::struct_trees_are_wire_identical` instead, which walks the whole
//! struct graph. These tests pin the hole and that it is now closed.

use super::*;

fn definitions() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions")
}

fn definition(game: &str, group: &str) -> std::path::PathBuf {
    definitions().join(game).join(format!("{group}.json"))
}

fn tag_from(game: &str, group: &str) -> TagFile {
    TagFile::new(definition(game, group))
        .unwrap_or_else(|error| panic!("build {game}/{group}: {error}"))
}

fn group_tag_of(game: &str, group: &str) -> u32 {
    tag_from(game, group).header.group_tag
}

/// The hole, stated as an assertion rather than as prose.
///
/// A Halo Reach animation graph does not merely survive the gate's hard checks
/// against Campaign Evolved -- it earns a clean `Match`, the same verdict a
/// genuine Campaign Evolved tag gets. There is no warning and no override in
/// this path; the tag simply imports.
///
/// This is what forces the classifier to look deeper than the root struct. If
/// this test ever starts failing because the severity moved off `Match`, the
/// definitions changed and the classifier's evidence wants rechecking -- it is
/// not licence to go back to root-level comparison.
#[test]
fn a_reach_animation_graph_is_indistinguishable_from_campaign_evolved_at_the_root() {
    let reach = tag_from("haloreach_mcc", "model_animation_graph");
    let evolved = tag_from("haloce_evolved", "model_animation_graph");
    let cmp = blam_tags::compare_root_layout(&evolved, &reach);

    assert!(cmp.group_match, "both games call this group jmad");
    assert!(cmp.version_match, "both declare group version 1");
    assert_eq!(
        cmp.expected_root_size, 440,
        "if this number moves the whole premise wants rechecking",
    );
    assert!(cmp.root_size_match, "the root struct is 440 bytes on both sides");
    assert_eq!(
        cmp.severity,
        blam_tags::LayoutSeverity::Match,
        "the root structs are field-for-field identical, so the root-only \
         comparison reports a clean match for another game's tag",
    );

    // And yet the two disagree, four structs down.
    let reach_shared = nested_struct_size(&reach, "shared_model_animation_block");
    let evolved_shared = nested_struct_size(&evolved, "shared_model_animation_block");
    assert_eq!(
        (reach_shared, evolved_shared),
        (Some(212), Some(200)),
        "shared_model_animation_block is the difference the root cannot see",
    );
}

/// Walk a tag's layout for a struct by name and report its declared size.
/// Cycle-safe by construction: a struct already visited is not re-entered.
fn nested_struct_size(tag: &TagFile, wanted: &str) -> Option<usize> {
    fn walk(
        structure: blam_tags::TagStructDefinition<'_>,
        wanted: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> Option<usize> {
        if structure.name() == wanted {
            return Some(structure.size());
        }
        if !seen.insert(structure.name().to_owned()) {
            return None;
        }
        for field in structure.fields() {
            let nested = field
                .as_struct()
                .or_else(|| field.as_block().map(|b| b.struct_definition()))
                .or_else(|| field.as_array().map(|a| a.struct_definition()))
                .or_else(|| field.as_resource().map(|r| r.struct_definition()));
            if let Some(found) = nested.and_then(|nested| walk(nested, wanted, seen)) {
                return Some(found);
            }
        }
        None
    }
    walk(
        tag.definitions().root_struct(),
        wanted,
        &mut std::collections::HashSet::new(),
    )
}

/// The fix. A Reach animation graph is classified as needing conversion, so the
/// dialog never offers to copy its bytes.
#[test]
fn a_reach_animation_graph_is_classified_as_needing_conversion() {
    let reach = tag_from("haloreach_mcc", "model_animation_graph");
    let group_tag = group_tag_of("haloce_evolved", "model_animation_graph");
    let (verdicts, mode) = classify_import_source_for(CAMPAIGN_EVOLVED_GAME, group_tag, &reach);

    match mode {
        ImportMode::Convert { source_game, draft } => {
            assert_eq!(source_game, "haloreach_mcc");
            assert!(draft.is_none(), "nothing has been converted yet");
        }
        ImportMode::Native { .. } => panic!("a Reach jmad must not import as native bytes"),
    }

    // The classification rests on two facts, so assert both rather than just
    // the conclusion: Reach claims the tag, and Campaign Evolved does not.
    let fit = |game: &str| {
        verdicts
            .iter()
            .find(|(candidate, _)| candidate == game)
            .map(|(_, fit)| fit)
            .unwrap_or_else(|| panic!("{game} defines model_animation_graph"))
    };
    assert!(fit("haloreach_mcc").is_identical(), "Reach claims it outright");
    match fit(CAMPAIGN_EVOLVED_GAME) {
        // The walk reports the *first* divergence in declaration order, which is
        // `animation_graph_node_block` under `definitions/skeleton nodes` --
        // Reach carries two extra flag bytes there. It is one of the four
        // structs that change size; the others are only reachable later.
        ProfileFit::Diverges(where_) => {
            assert!(
                where_.contains("skeleton nodes") && where_.contains("40") && where_.contains("44"),
                "the divergence should name where and by how much, got: {where_}",
            );
        }
        _ => panic!("Campaign Evolved must not claim a Reach animation graph"),
    }
}

/// The other half: a genuine Campaign Evolved tag still imports as a plain byte
/// copy. The fix must not cost the happy path.
#[test]
fn a_campaign_evolved_tag_still_imports_natively() {
    let evolved = tag_from("haloce_evolved", "model_animation_graph");
    let group_tag = group_tag_of("haloce_evolved", "model_animation_graph");
    let (_, mode) = classify_import_source_for(CAMPAIGN_EVOLVED_GAME, group_tag, &evolved);

    match mode {
        ImportMode::Native { comparison, import_anyway } => {
            assert_eq!(
                comparison.map(|cmp| cmp.severity),
                Some(blam_tags::LayoutSeverity::Match),
            );
            assert!(!import_anyway, "a clean match needs no override");
        }
        ImportMode::Convert { source_game, .. } => {
            panic!("a Campaign Evolved tag was mistaken for a {source_game} one")
        }
    }
}

/// A group Campaign Evolved and Reach agree on exactly must not be dragged into
/// the conversion path by the presence of a Reach match. When the destination
/// matches cleanly, that settles it -- no other profile's opinion applies.
///
/// `sound_looping` is one of the 49 groups whose Reach and Campaign Evolved
/// definitions are wire-identical.
#[test]
fn a_group_both_games_agree_on_imports_natively() {
    let reach = tag_from("haloreach_mcc", "sound_looping");
    let group_tag = group_tag_of("haloce_evolved", "sound_looping");
    let (verdicts, mode) = classify_import_source_for(CAMPAIGN_EVOLVED_GAME, group_tag, &reach);

    assert!(
        matches!(mode, ImportMode::Native { .. }),
        "sound_looping is identical across the two games, so the bytes are \
         already the right shape; verdicts were {verdicts:?}",
    );
}

/// The import path end to end on a real HREK file: classified as needing
/// conversion, converted, and the converted tag is what would land.
///
/// Self-skips without HREK, so watch for the skip line before trusting a green
/// run here.
#[test]
fn a_real_hrek_animation_graph_imports_as_a_conversion() {
    let source_path = std::path::Path::new(
        "D:/SteamLibrary/steamapps/common/HREK/tags/cinematics/052lb_reflection/objects/052lb_reflection_030/elevator_1.model_animation_graph",
    );
    if !source_path.is_file() {
        eprintln!("skipping: HREK is not installed at the expected path");
        return;
    }
    let bytes = std::fs::read(source_path).expect("read the HREK graph");
    let imported = TagFile::read_from_bytes(&bytes).expect("parse it");
    let group_tag = imported.header.group_tag;

    // 1. The gate recognizes it as another game's tag rather than waving it
    //    through on a root-struct match.
    let (verdicts, mode) = classify_import_source_for(CAMPAIGN_EVOLVED_GAME, group_tag, &imported);
    let ImportMode::Convert { source_game, .. } = mode else {
        panic!("a real Reach animation graph must not import as native bytes: {verdicts:?}");
    };
    assert_eq!(source_game, "haloreach_mcc");

    // 2. It converts, and the converted tag is a Campaign Evolved one.
    let draft = analyze_conversion(
        &imported,
        &source_game,
        CAMPAIGN_EVOLVED_GAME,
        &locate_definitions_root(),
        None,
    )
    .unwrap_or_else(|error| panic!("the import path must be able to convert it: {error}"));

    assert!(
        draft.report.transferred_resources > 0,
        "the animation payload has to come with it",
    );
    assert_eq!(draft.target_group_name, "model_animation_graph");

    // 3. And what lands parses, at the destination's generation.
    let mut landed = draft.tag;
    apply_editing_kit_mcc_header(&mut landed, CAMPAIGN_EVOLVED_GAME).expect("stamp it");
    let written = landed.write_to_bytes().expect("serialize what would land");
    let reopened = TagFile::read_from_bytes(&written).expect("the paks would be able to read it");
    assert_eq!(reopened.header.group_tag, group_tag);
}

/// A group only Campaign Evolved defines cannot be claimed by any other
/// profile, so the verdict list names exactly one game.
#[test]
fn a_campaign_evolved_only_group_is_claimed_by_nothing_else() {
    let evolved = tag_from("haloce_evolved", "skull_globals");
    let group_tag = group_tag_of("haloce_evolved", "skull_globals");
    let (verdicts, _) = classify_import_source_for(CAMPAIGN_EVOLVED_GAME, group_tag, &evolved);

    assert_eq!(
        verdicts
            .iter()
            .map(|(game, _)| game.as_str())
            .collect::<Vec<_>>(),
        vec![CAMPAIGN_EVOLVED_GAME],
        "skull_globals exists only in Campaign Evolved",
    );
}
