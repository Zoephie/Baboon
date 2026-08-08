//! Right-click → Extract → particle geometry actually extracts.
//!
//! This wiring has two halves that are edited in different files and can
//! drift apart silently: the browser's `supports_*` gate decides whether
//! the menu item is drawn, and `extract_geometry_for_entry`'s match arm
//! decides whether the action does anything. Either one alone looks
//! finished — a gate with no arm shows a menu entry that errors, an arm
//! with no gate is unreachable.
//!
//! The gate half lives with the other groups in
//! `browser::filter`'s `tag_extract_menu_covers_every_group_with_an_asset_extractor`.
//! What is asserted here is the action half, each test opening by
//! re-checking its own gate so the pair stays visible in one place.
//!
//! Skips silently when the corresponding tag set is absent.

use std::path::{Path, PathBuf};

use crate::app::browser::supports_tag_extract_menu;
use crate::app::export::extract_geometry_for_entry;
use crate::source::{TagEntry, TagEntryLocation, TagSource};

/// Root of an extracted MCC tag set, via `BLAM_TEST_<KIT>_TAGS` or the
/// conventional local layout.
fn kit_tags(kit: &str) -> Option<PathBuf> {
    let var = format!("BLAM_TEST_{}_TAGS", kit.to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home)
        .join("Halo")
        .join(format!("{kit}_mcc"))
        .join("tags");
    p.is_dir().then_some(p)
}

fn loose_source(root: &Path, game: &str) -> TagSource {
    TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: Some(game.to_owned()),
        definitions_root: crate::app::locate_definitions_root(),
    }
}

fn entry_for(root: &Path, rel: &str, group: &[u8; 4]) -> TagEntry {
    let path = root.join(rel);
    TagEntry {
        key: format!("file:{}", path.display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*group),
        group_name: Some("particle_model".to_owned()),
        location: TagEntryLocation::LooseFile(path),
    }
}

/// The gen3 action writes the manifest plus one JMS per object, at the
/// paths the manifest names.
#[test]
fn extracting_a_gen3_particle_model_writes_a_resolvable_jmi() {
    assert!(
        supports_tag_extract_menu(u32::from_be_bytes(*b"pmdf")),
        "the menu item that reaches this action is not drawn",
    );
    let Some(root) = kit_tags("haloreach") else {
        return;
    };
    let rel = "fx/particles/models/debris/generic_shards/generic_shards.particle_model";
    if !root.join(rel).is_file() {
        return;
    }
    let out = std::env::temp_dir().join("baboon_pm_extract_gen3");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("create output dir");

    let summary = extract_geometry_for_entry(
        &loose_source(&root, "haloreach_mcc"),
        &entry_for(&root, rel, b"pmdf"),
        &out,
    )
    .expect("extract particle geometry");
    assert!(summary.contains("8 objects"), "unexpected summary: {summary}");

    // The manifest must sit where `import particle model` expects, and
    // every line it names must resolve to a real JMS beside it. A
    // dangling line is a silent import failure, not a warning.
    let jmi = out.join("generic_shards").join("generic_shards.jmi");
    assert!(jmi.is_file(), "no manifest at {}", jmi.display());
    let text = std::fs::read_to_string(&jmi).expect("read manifest");
    assert!(text.contains("\r\n"), "manifest must use CRLF");
    let objects: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(';'))
        .skip(2)
        .collect();
    assert_eq!(objects.len(), 8, "manifest lists {} objects", objects.len());
    for name in objects {
        let jms = jmi
            .parent()
            .unwrap()
            .join(name)
            .join("render")
            .join(format!("{name}.JMS"));
        assert!(jms.is_file(), "manifest names `{name}` but {} is missing", jms.display());
        let body = std::fs::read_to_string(&jms).expect("read JMS");
        assert!(body.contains(";### VERTICES ###"), "`{name}` JMS has no vertices section");
    }

    // Lowercase, because tool.exe's manifest-vs-directory check is a
    // case-sensitive `strncmp(ext, ".jmi", 5)`.
    assert_eq!(jmi.extension().and_then(|e| e.to_str()), Some("jmi"));

    let _ = std::fs::remove_dir_all(&out);
}

/// Halo 2 goes through the same action but a different decode, and its
/// summary must not claim the names were invented — `PRTM` stores them.
#[test]
fn extracting_a_halo2_particle_model_keeps_its_object_names() {
    assert!(
        supports_tag_extract_menu(u32::from_be_bytes(*b"PRTM")),
        "the menu item that reaches this action is not drawn",
    );
    let Some(root) = kit_tags("halo2") else {
        return;
    };
    let rel = "effects/particle_models/urban_debris/urban_debris.particle_model";
    if !root.join(rel).is_file() {
        return;
    }
    let out = std::env::temp_dir().join("baboon_pm_extract_h2");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("create output dir");

    let summary = extract_geometry_for_entry(
        &loose_source(&root, "halo2_mcc"),
        &entry_for(&root, rel, b"PRTM"),
        &out,
    )
    .expect("extract particle geometry");
    assert!(summary.contains("10 objects"), "unexpected summary: {summary}");
    assert!(
        !summary.contains("numbered from the tag name"),
        "Halo 2 stores its object names — the summary must not say otherwise: {summary}",
    );

    assert!(
        out.join("urban_debris")
            .join("can_1")
            .join("render")
            .join("can_1.JMS")
            .is_file(),
        "shipped object name `can_1` did not reach the output tree",
    );

    let _ = std::fs::remove_dir_all(&out);
}
