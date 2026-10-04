use std::path::{Path, PathBuf};

use super::*;

#[test]
fn restored_loose_tag_uses_the_current_sources_key() {
    let root = std::env::temp_dir().join(format!("baboon-session-key-{}", std::process::id()));
    let path = root.join("objects").join("characters").join("brute.model");
    std::fs::create_dir_all(path.parent().expect("tag has parent")).expect("create tag path");
    std::fs::write(&path, b"tag").expect("create tag");
    let canonical = std::fs::canonicalize(&path).expect("canonical tag path");
    let entry = crate::core::source::TagEntry {
        key: file_entry_key(&canonical),
        display_path: "objects/characters/brute.model".to_owned(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".to_owned()),
        location: crate::core::source::TagEntryLocation::LooseFile(canonical.clone()),
    };

    assert_eq!(
        super::loose_entry_key_for_canonical_path(std::iter::once(&entry), &canonical),
        Some(entry.key.clone())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn last_opened_workspace_heading_prefers_the_named_project() {
    let source = PathBuf::from("Games").join("Halo Infinite");
    let project = PathBuf::from("Baboon Projects").join("Campaign Overhaul.baboon");

    assert_eq!(
        super::last_opened_workspace_heading(
            None,
            Some("halo_infinite"),
            &source,
            Some(&project)
        ),
        (
            "Campaign Overhaul".to_owned(),
            Some(project.display().to_string())
        )
    );
}

#[test]
fn last_opened_workspace_heading_keeps_the_source_fallback() {
    let source = Path::new(r"C:\Editing Kits\Custom Kit");

    assert_eq!(
        super::last_opened_workspace_heading(None, None, source, None),
        (source.display().to_string(), None)
    );
}

#[test]
fn last_opened_workspace_heading_prefers_the_custom_editing_kit_profile() {
    let source = Path::new(r"C:\Editing Kits\H2EK");

    assert_eq!(
        super::last_opened_workspace_heading(
            Some(("Halo 2 Rebalance", source)),
            Some("halo2_mcc"),
            source,
            None
        ),
        (
            "Halo 2 Rebalance".to_owned(),
            Some(source.display().to_string())
        )
    );
}
