use super::*;
use crate::core::source::{TagEntry, TagEntryLocation};
use std::path::PathBuf;

fn entry(display_path: &str, group: &[u8; 4]) -> TagEntry {
    TagEntry {
        key: display_path.to_owned(),
        display_path: display_path.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
    }
}

#[test]
fn matches_filename_not_parent_folders() {
    let entries = vec![
        entry("floodcombat_elite/garbage/hg_arm/hg_arm.model", b"mode"),
        entry("characters/elite/elite.model", b"mode"),
    ];
    // "elite" should match only the tag whose *filename* contains it.
    let matches = compute_filter_matches(&entries, "elite");
    assert_eq!(matches, vec![1]);
}

#[test]
fn matches_group_tag_and_is_case_insensitive() {
    let entries = vec![
        entry("fx/spark.effect", b"effe"),
        entry("weapons/rifle.weapon", b"weap"),
    ];
    // Group four-CC match, regardless of query case.
    assert_eq!(compute_filter_matches(&entries, "WEAP"), vec![1]);
}

#[test]
fn reference_input_uses_fourcc_and_backslash_path_without_extension() {
    let e = entry("objects/weapons/rifle/rifle.weapon", b"weap");
    // "weap" four-CC, backslash path, extension stripped.
    assert_eq!(
        entry_reference_input(&e),
        "weap:objects\\weapons\\rifle\\rifle"
    );
}

#[test]
fn malformed_filter_warnings() {
    // Well-formed filters: no warning.
    assert!(browser_filter_warning("").is_none());
    assert!(browser_filter_warning("elite").is_none());
    assert!(browser_filter_warning("arm | rifle").is_none());
    assert!(browser_filter_warning("^elite.model$").is_none());
    assert!(browser_filter_warning("weapon$").is_none());
    // Empty OR operand.
    assert!(browser_filter_warning("foo |").is_some());
    assert!(browser_filter_warning("a || b").is_some());
    // Anchor-only term.
    assert!(browser_filter_warning("^").is_some());
    assert!(browser_filter_warning("foo ^").is_some());
}

#[test]
fn boolean_and_or_and_anchors() {
    let entries = vec![
        entry("characters/elite/elite_arm.model", b"mode"),
        entry("characters/elite/elite.model", b"mode"),
        entry("weapons/rifle.weapon", b"weap"),
    ];
    // AND: both terms must match the same entry.
    assert_eq!(compute_filter_matches(&entries, "elite arm"), vec![0]);
    // OR: either side matches.
    assert_eq!(compute_filter_matches(&entries, "arm | rifle"), vec![0, 2]);
    // Prefix anchor on filename.
    assert_eq!(compute_filter_matches(&entries, "^elite_"), vec![0]);
    // Suffix anchor on filename.
    assert_eq!(compute_filter_matches(&entries, "weapon$"), vec![2]);
    // Exact filename anchor.
    assert_eq!(compute_filter_matches(&entries, "^elite.model$"), vec![1]);
}

#[test]
fn folder_hlsl_include_collector_finds_nested_include_entries() {
    let entries = vec![
        entry("rasterizer/hlsl/ssao.hlsl_include", b"hlsl"),
        entry("rasterizer/hlsl/post/tonemap.hlsl_include", b"hlsl"),
        entry("rasterizer/bitmaps/noise.bitmap", b"bitm"),
    ];
    let tree = crate::core::source::build_tree(&entries);
    let rasterizer = tree
        .children
        .iter()
        .find(|node| node.label == "rasterizer")
        .expect("rasterizer folder");

    assert_eq!(
        collect_hlsl_include_keys(rasterizer, &entries),
        vec![
            "rasterizer/hlsl/ssao.hlsl_include".to_owned(),
            "rasterizer/hlsl/post/tonemap.hlsl_include".to_owned(),
        ]
    );
}

#[test]
fn folder_material_shader_collector_finds_nested_material_shader_entries() {
    let entries = vec![
        entry(
            "shaders/material_shaders/decals/base.material_shader",
            b"mats",
        ),
        entry(
            "shaders/material_shaders/decals/palette/palette.material_shader",
            b"mats",
        ),
        entry("shaders/material_shaders/decals/noise.bitmap", b"bitm"),
    ];
    let tree = crate::core::source::build_tree(&entries);
    let shaders = tree
        .children
        .iter()
        .find(|node| node.label == "shaders")
        .expect("shaders folder");

    assert_eq!(
        collect_material_shader_keys(shaders, &entries),
        vec![
            "shaders/material_shaders/decals/base.material_shader".to_owned(),
            "shaders/material_shaders/decals/palette/palette.material_shader".to_owned(),
        ]
    );
}

#[test]
fn tag_extract_menu_covers_every_group_with_an_asset_extractor() {
    for group in [
        b"hlmt", b"mode", b"mod2", b"coll", b"phmo", b"jmad", b"antr",
    ] {
        assert!(supports_tag_extract_menu(u32::from_be_bytes(*group)));
    }
    // Per-asset extraction moved into this menu, so the button has to enable
    // for these groups too — otherwise the items are unreachable.
    for group in [b"bitm", b"mats", b"hlsl"] {
        assert!(
            supports_tag_extract_menu(u32::from_be_bytes(*group)),
            "{} should enable the Extract menu",
            String::from_utf8_lossy(group)
        );
    }
    // Level geometry: a BSP exports one ASS, and a scenario exports one per
    // BSP it references. The scenario's *script* items still sit inline
    // rather than in this menu, but its geometry item lives here, so the
    // menu must enable for both groups.
    for group in [b"sbsp", b"scnr"] {
        assert!(
            supports_tag_extract_menu(u32::from_be_bytes(*group)),
            "{} should enable the Extract menu",
            String::from_utf8_lossy(group)
        );
    }
    // A particle_model exports its source JMI plus one JMS per object.
    // `pmdf` is Halo 3 / Reach / Halo 4; `PRTM` is Halo 2's unrelated
    // tag of the same name, which routes through the same menu item.
    for group in [b"pmdf", b"PRTM"] {
        assert!(
            supports_tag_extract_menu(u32::from_be_bytes(*group)),
            "{} should enable the Extract menu",
            String::from_utf8_lossy(group)
        );
    }
    // A plain weapon has no extractor at all — it must leave it disabled.
    assert!(!supports_tag_extract_menu(u32::from_be_bytes(*b"weap")));
}
