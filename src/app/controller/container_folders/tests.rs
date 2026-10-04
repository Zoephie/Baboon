use super::*;

fn entry(display_path: &str) -> TagEntry {
    TagEntry {
        key: display_path.into(),
        display_path: display_path.into(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: None,
        location: TagEntryLocation::LooseFile(std::path::PathBuf::from(display_path)),
    }
}

fn folders(paths: &[&str]) -> std::collections::BTreeSet<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

#[test]
fn normalizes_separators_and_strays() {
    assert_eq!(
        normalize_folder_rel("/objects\\vehicles/"),
        "objects/vehicles"
    );
    assert_eq!(
        normalize_folder_rel("objects//vehicles"),
        "objects/vehicles"
    );
    assert_eq!(normalize_folder_rel("  "), "");
}

#[test]
fn siblings_include_subfolders_and_tag_leaves() {
    let entries = vec![
        entry("objects/vehicles/warthog.model"),
        entry("objects/characters/masterchief.biped"),
        entry("sound/ambient.sound"),
    ];
    let pending = folders(&["objects/pending"]);

    let mut names = folder_siblings(&entries, &pending, Some("objects"));
    names.sort();
    // `warthog.model` is a leaf of `objects/vehicles`, not of `objects`.
    assert_eq!(names, vec!["characters", "pending", "vehicles"]);

    let mut root = folder_siblings(&entries, &pending, None);
    root.sort();
    assert_eq!(root, vec!["objects", "sound"]);
}

/// A pak's directory index cannot hold a name that is both a file and a
/// directory, so this has to be refused here rather than at write time.
#[test]
fn a_folder_cannot_collide_with_a_tag_leaf_in_the_same_parent() {
    let entries = vec![entry("objects/warthog.model")];
    let siblings = folder_siblings(&entries, &folders(&[]), Some("objects"));
    assert!(validate_folder_leaf_name("warthog.model", &siblings).is_err());
    assert!(validate_folder_leaf_name("vehicles", &siblings).is_ok());
}

/// A container's `display_path` is lowercased at mount, so a seed kept in
/// the user's casing would draw a second node beside the real folder as
/// soon as a tag landed in it — and keep re-creating the empty one.
#[test]
fn a_container_folder_is_seeded_in_display_path_casing() {
    assert_eq!(
        container_folder_rel(Some("objects"), "Vehicles"),
        "objects/vehicles"
    );
    assert_eq!(container_folder_rel(None, "Objects"), "objects");
    assert_eq!(container_folder_rel(Some(""), "Objects"), "objects");

    // The seeded path must reach the same node an entry would build.
    let entries = vec![entry("objects/vehicles/warthog.model")];
    let seeded = container_folder_rel(Some("objects"), "Vehicles");
    let tree = crate::source::build_tree_with_folders(&entries, &[seeded]);
    let objects = tree
        .children
        .iter()
        .find(|child| child.label == "objects")
        .expect("objects node");
    assert_eq!(
        objects.children.len(),
        1,
        "seeding must not add a second `vehicles` beside the real one"
    );
}

#[test]
fn folder_collisions_are_case_insensitive() {
    let siblings = vec!["Vehicles".to_owned()];
    assert!(validate_folder_leaf_name("vehicles", &siblings).is_err());
    assert!(validate_folder_leaf_name("VEHICLES", &siblings).is_err());
}

/// The naming contract is shared with the tag duplicate path, so a folder
/// nothing could be created inside is refused up front.
#[test]
fn folder_names_inherit_the_shared_leaf_rules() {
    for invalid in [
        "",
        "  ",
        ".",
        "..",
        "a/b",
        "a.b",
        "trailing ",
        "CON",
        "we:ird",
    ] {
        assert!(
            validate_folder_leaf_name(invalid, &[]).is_err(),
            "{invalid:?} should be refused"
        );
    }
    assert_eq!(
        validate_folder_leaf_name(" vehicles", &[]).unwrap(),
        "vehicles"
    );
}
