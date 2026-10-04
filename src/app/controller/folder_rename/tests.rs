use super::*;

fn siblings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_new_name_is_accepted_and_trimmed() {
    assert_eq!(
        validate_loose_folder_rename("  shadow", "creep", &siblings(&["creep", "ghost"])),
        Ok("shadow".to_owned())
    );
}

#[test]
fn the_same_name_and_a_case_only_change_are_refused() {
    let here = siblings(&["creep"]);
    assert!(validate_loose_folder_rename("creep", "creep", &here).is_err());
    assert!(validate_loose_folder_rename("Creep", "creep", &here).is_err());
}

#[test]
fn a_sibling_folder_or_file_conflicts_ignoring_case() {
    let here = siblings(&["creep", "Ghost", "shadow.vehicle"]);
    assert!(validate_loose_folder_rename("ghost", "creep", &here).is_err());
    // A dotted name is refused before the sibling check ever sees it, but
    // a plain one that matches a file's full name is still a conflict.
    let here = siblings(&["creep", "readme"]);
    assert!(validate_loose_folder_rename("README", "creep", &here).is_err());
}

#[test]
fn separators_and_illegal_characters_are_refused() {
    let here = siblings(&["creep"]);
    for bad in ["a/b", "a\\b", "", "  ", "..", "a:b", "x.y", "CON"] {
        assert!(
            validate_loose_folder_rename(bad, "creep", &here).is_err(),
            "{bad:?} should be refused"
        );
    }
}

#[test]
fn outside_referrers_exclude_tags_inside_the_folder() {
    let names = TagNameIndex::default();
    let vehicle = parse_group_tag("vehi").unwrap();
    let model = parse_group_tag("hlmt").unwrap();
    let inside_vehicle = TagEntry {
        key: "file:tags/objects/vehicles/creep/shadow.vehicle".to_owned(),
        display_path: "objects/vehicles/creep/shadow.vehicle".to_owned(),
        group_tag: vehicle,
        group_name: Some("vehicle".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(
            "tags/objects/vehicles/creep/shadow.vehicle",
        )),
    };
    let inside_model = TagEntry {
        key: "file:tags/objects/vehicles/creep/shadow.model".to_owned(),
        display_path: "objects/vehicles/creep/shadow.model".to_owned(),
        group_tag: model,
        group_name: Some("model".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(
            "tags/objects/vehicles/creep/shadow.model",
        )),
    };
    let reference = |group_tag, rel_path: &str| DependencyRef {
        group_tag,
        rel_path: rel_path.to_owned(),
    };
    let mut index = ReverseDependencyIndex::default();
    // The vehicle references its own model: inside -> inside.
    index.set_tag_dependencies(
        inside_vehicle.key.clone(),
        vec![reference(model, "objects\\vehicles\\creep\\shadow")],
    );
    // A scenario outside references the vehicle.
    index.set_tag_dependencies(
        "file:tags/levels/a10/a10.scenario".to_owned(),
        vec![reference(vehicle, "objects\\vehicles\\creep\\shadow")],
    );
    // Something unrelated.
    index.set_tag_dependencies(
        "file:tags/levels/a30/a30.scenario".to_owned(),
        vec![reference(vehicle, "objects\\vehicles\\ghost\\ghost")],
    );

    let keys = outside_referrer_keys(&[inside_vehicle, inside_model], &index, &names);

    assert_eq!(
        keys.into_iter().collect::<Vec<_>>(),
        vec!["file:tags/levels/a10/a10.scenario".to_owned()]
    );
}

/// Every tag reference under `root`, as `(referencing tag, referenced path)`.
fn all_references(root: &Path, game: &str) -> Vec<(String, String)> {
    let source = TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: GameId::from_id(game),
        definitions_root: locate_definitions_root(),
    };
    let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::from_id(game).unwrap()).unwrap();
    let mut out = Vec::new();
    for entry in scan_folder_subtree_entries(root, Path::new(""), &names).unwrap() {
        let tag = read_entry(&source, &entry)
            .unwrap_or_else(|error| panic!("{} reads: {error}", entry.display_path));
        let mut refs = Vec::new();
        collect_tag_references(tag.root(), "", &mut refs);
        out.extend(
            refs.into_iter()
                .map(|reference| (entry.display_path.clone(), reference.rel_path)),
        );
    }
    out
}

fn copy_tree(from: &Path, to: &Path) {
    for item in walkdir::WalkDir::new(from) {
        let item = item.unwrap();
        let target = to.join(item.path().strip_prefix(from).unwrap());
        if item.file_type().is_dir() {
            fs::create_dir_all(&target).unwrap();
        } else {
            fs::copy(item.path(), &target).unwrap();
        }
    }
}

/// Rename `folder` (inside `vehicle`, copied out of a real kit) and check
/// that the tags outside it that referenced it now reference the new path,
/// and that nothing references the old one.
fn renames_and_rewrites(var: &str, game: &str, vehicle: &str, folder: &str) {
    let Some(kit) = std::env::var_os(var).map(PathBuf::from) else {
        eprintln!("skipping: set {var} to a {game} kit's tags folder");
        return;
    };
    let root = crate::test_kits::unique_temp_dir("folder-rename");
    copy_tree(&kit.join(vehicle), &root.join(vehicle));
    let old_prefix = format!("{}\\{folder}\\", vehicle.replace('/', "\\"));
    let new_prefix = format!("{}\\{folder}_renamed\\", vehicle.replace('/', "\\"));
    let referenced_before = all_references(&root, game)
        .into_iter()
        .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&old_prefix))
        .count();
    assert!(referenced_before > 0, "the copy references {old_prefix}");

    let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::from_id(game).unwrap()).unwrap();
    let (tx, _rx) = mpsc::channel();
    let rel = PathBuf::from(vehicle).join(folder);
    let done = run_folder_refactor_job(
        root.clone(),
        rel.clone(),
        root.join(vehicle),
        Some(format!("{folder}_renamed")),
        true,
        "Renaming".to_owned(),
        names,
        GameId::from_id(game),
        Vec::new(),
        None,
        &tx,
    )
    .unwrap();

    assert!(!root.join(&rel).exists());
    assert!(
        root.join(vehicle)
            .join(format!("{folder}_renamed"))
            .is_dir()
    );
    assert!(done.status.starts_with("Renamed"), "{}", done.status);
    assert!(!done.status.contains("NOT"), "{}", done.status);
    assert!(!done.old_to_new_keys.is_empty());
    let after = all_references(&root, game);
    let stale = after
        .iter()
        .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&old_prefix))
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "still pointing at the old folder: {stale:?}"
    );
    let renamed = after
        .iter()
        .filter(|(_, target)| target.to_ascii_lowercase().starts_with(&new_prefix))
        .count();
    assert_eq!(renamed, referenced_before);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn renaming_a_halo3_folder_rewrites_references_into_it() {
    renames_and_rewrites(
        "BLAM_TEST_H3EK",
        "halo3_mcc",
        "objects/vehicles/ghost",
        "shaders",
    );
}

#[test]
fn renaming_a_halo_ce_folder_rewrites_references_into_it() {
    renames_and_rewrites("BLAM_TEST_HCEEK", "haloce_mcc", "vehicles/ghost", "shaders");
}
