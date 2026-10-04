use super::*;
use crate::app::controller::loading::loaded_source_status;
use crate::app::controller::affected_move_rewrite_entries;
use crate::app::controller::bytes_contain_any_ascii_case_insensitive;
use crate::app::controller::rewrite_reference_needles;
use crate::app::controller::build_folder_reference_rewrites;
use crate::app::controller::normalize_container_tag_rel;
use crate::app::controller::new_container_template_for;
use crate::app::controller::new_container_package;

fn entry(display_path: &str, group_tag: u32) -> TagEntry {
    TagEntry {
        key: format!("file:{display_path}"),
        display_path: display_path.to_owned(),
        group_tag,
        group_name: None,
        location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
    }
}

fn abs_entry(root: &Path, display_path: &str, group_tag: u32) -> TagEntry {
    TagEntry {
        key: file_entry_key(&root.join(display_path)),
        display_path: display_path.to_owned(),
        group_tag,
        group_name: None,
        location: TagEntryLocation::LooseFile(root.join(display_path)),
    }
}

fn container_entry(key: &str, display_path: &str, group_tag: u32) -> TagEntry {
    TagEntry {
        key: key.to_owned(),
        display_path: display_path.to_owned(),
        group_tag,
        group_name: None,
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: format!("Tags/{display_path}.ubulk"),
        },
    }
}

fn loose_source_with_counts(label: &str, entries: Vec<TagEntry>) -> LoadedSourceData {
    LoadedSourceData {
        label: label.to_owned(),
        source: TagSource::LooseFolder {
            root: PathBuf::from("C:/kit/tags"),
            game: Some(GameId::Halo3),
            definitions_root: PathBuf::from("C:/kit/definitions"),
        },
        names: TagNameIndex::default(),
        game: Some(GameId::Halo3),
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: entries,
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    }
}

#[test]
fn loose_folder_status_does_not_report_zero_loaded_tags_before_scan() {
    let source = loose_source_with_counts("H3EK/tags (halo3_mcc)", Vec::new());

    assert_eq!(
        loaded_source_status(&source),
        "Browsing tags from H3EK/tags (halo3_mcc)"
    );
}

#[test]
fn loose_folder_status_uses_recursive_index_when_available() {
    let shader = parse_group_tag("rmsh").unwrap();
    let source = loose_source_with_counts(
        "H3EK/tags (halo3_mcc)",
        vec![
            entry("objects/a.shader", shader),
            entry("objects/b.shader", shader),
        ],
    );

    assert_eq!(
        loaded_source_status(&source),
        "Found 2 tag(s) in H3EK/tags (halo3_mcc)"
    );
}

#[test]
fn dependency_entry_reference_path_strips_only_group_extension() {
    let names = TagNameIndex::default();
    let bitmap = parse_group_tag("bitm").unwrap();
    let entry = entry("objects/weapons/decal_road_1.bitmap.bitmap", bitmap);

    assert_eq!(
        dependency_entry_reference_path(&entry, &names).unwrap(),
        "objects\\weapons\\decal_road_1.bitmap"
    );
}

#[test]
fn container_reference_resolution_uses_group_and_normalized_path() {
    let names = TagNameIndex::default();
    let render_model = parse_group_tag("mode").unwrap();
    let weapon = parse_group_tag("weap").unwrap();
    let display_stem = "objects/shared/example";
    let entries = vec![
        entry(&format!("{display_stem}.render_model"), render_model),
        container_entry(
            "ublock:shared:model",
            &format!("{display_stem}.render_model"),
            render_model,
        ),
        container_entry(
            "ublock:shared:weapon",
            &format!("{display_stem}.weapon"),
            weapon,
        ),
    ];

    let model = container_entry_for_reference(
        &entries,
        render_model,
        "OBJECTS/SHARED/EXAMPLE.RENDER_MODEL",
        &names,
    )
    .expect("render-model reference should resolve");
    assert_eq!(model.key, "ublock:shared:model");

    let weapon_entry =
        container_entry_for_reference(&entries, weapon, r"objects\shared\example", &names)
            .expect("same path in another group should resolve independently");
    assert_eq!(weapon_entry.key, "ublock:shared:weapon");

    assert!(
        container_entry_for_reference(
            &entries,
            render_model,
            r"objects\shared\missing",
            &names
        )
        .is_none()
    );
}

/// A tag created this session is a reference target like any other: it is
/// addressed by the same logical path, and "Open referenced tag" resolves
/// through this lookup. Excluding it reported the tag as missing.
#[test]
fn container_reference_resolution_finds_an_unsaved_new_tag() {
    let names = TagNameIndex::default();
    let camera_track = parse_group_tag("trak").unwrap();
    let entries = vec![new_container_entry(
        "test/example.camera_track",
        camera_track,
        "camera_track",
    )];

    let found = container_entry_for_reference(
        &entries,
        camera_track,
        r"test\example.camera_track",
        &names,
    )
    .expect("a new tag should resolve as a reference target");
    assert_eq!(found.key, "newtag:/Game/Tags/test/example-camera_track");
}

/// The capability matrix for a brand-new container tag, in one place: what
/// it can do, and the two things it deliberately cannot. Every one of these
/// gates gets its answer from a `match` on `TagEntryLocation`, and each one
/// that forgot the `NewContainer` arm broke the tag in a different way —
/// the editability gate made every field and block button inert.
#[test]
fn a_new_container_tag_has_the_expected_capabilities() {
    let camera_track = parse_group_tag("trak").unwrap();
    let entry = new_container_entry("test/example.camera_track", camera_track, "camera_track");
    let tag = TagFile::new("definitions/haloce_evolved/camera_track.json").unwrap();

    assert!(
        crate::app::is_editable_tag(&entry, &tag),
        "fields and block controls must be live for a new tag"
    );
    assert!(
        crate::app::supports_rename_menu(&entry),
        "rename/move is the only way to correct a mistyped new-tag path"
    );
    // No `.ubulk` behind it, so there is nothing to pull out.
    assert!(
        !crate::app::is_embedded_tag_entry(&entry),
        "a new tag has no embedded payload to extract"
    );
}

fn new_container_entry(display_path: &str, group_tag: u32, group_name: &str) -> TagEntry {
    let logical = display_path
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(display_path);
    let package = new_container_package(logical, group_name);
    TagEntry {
        key: new_tag_entry_key(&package),
        display_path: display_path.to_owned(),
        group_tag,
        group_name: Some(group_name.to_owned()),
        location: TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/other-camera_track.uasset".to_owned(),
            },
            package,
            group_tag,
        },
    }
}

/// A group the game ships no tag of is authorable when its wrapper can be
/// derived, and refused when it cannot.
///
/// Both halves matter. Only checking that `cinematic_scene` is allowed
/// would pass just as well if the gate had been deleted outright, and the
/// refusal is what keeps a tag from being created that could never be
/// saved — the group's Unreal class names other packages, and no import map
/// for those can be derived from the group alone.
#[test]
fn a_group_with_no_shipped_tag_is_authorable_only_when_its_wrapper_derives() {
    // Nothing to clone: the decision falls to whether the group is bare.
    let derived = new_container_template_for(None, "cinematic_scene")
        .expect("cinematic_scene ships no tag but its wrapper derives");
    assert!(matches!(
        derived,
        NewContainerTemplate::Derived { ref group } if group == "cinematic_scene"
    ));
    for group in ["scenario_hs_source_file", "flock", "point_physics"] {
        assert!(
            matches!(
                new_container_template_for(None, group),
                Ok(NewContainerTemplate::Derived { .. })
            ),
            "{group} is bare and should derive"
        );
    }

    // `object` and `unit` carry `AssetReference`, so there is nothing to
    // derive and nothing to clone.
    for group in ["object", "unit", "item", "device"] {
        let error = new_container_template_for(None, group)
            .expect_err("{group} must not be authorable without a donor");
        assert!(
            error.contains(group),
            "the refusal should name the group, got: {error}"
        );
    }

    // A donor always wins, bare or not: cloning is the path with the most
    // mileage on it and is right for every group the game actually ships.
    let donor =
        new_container_template_for(Some((3, "Tags/x-biped.uasset".to_owned())), "biped")
            .expect("a donor is always usable");
    assert!(matches!(
        donor,
        NewContainerTemplate::Donor { container: 3, .. }
    ));
}

/// Renaming a new tag must land on exactly the key and package that
/// creating it at that path would have produced — the save and project-
/// overlay paths identify the tag by them, so a second derivation that
/// drifted would strand the renamed tag.
#[test]
fn renaming_a_new_tag_derives_the_same_identity_as_creating_it_there() {
    let created = new_container_package("objects/foo/bar", "camera_track");
    assert_eq!(created, "/Game/Tags/objects/foo/bar-camera_track");
    assert_eq!(
        new_tag_entry_key(&created),
        "newtag:/Game/Tags/objects/foo/bar-camera_track"
    );

    // The rename path normalizes its input first — backslashes, case, and
    // stray separators must not fork the identity.
    let renamed = new_container_package(
        &normalize_container_tag_rel("/Objects\\Foo//Bar/"),
        "camera_track",
    );
    assert_eq!(renamed, created);
}

#[test]
fn dependency_candidate_index_matches_by_group_and_leaf_name() {
    let names = TagNameIndex::default();
    let bitmap = parse_group_tag("bitm").unwrap();
    let shader = parse_group_tag("rmsh").unwrap();
    let entries = vec![
        entry("objects/new/run.bitmap", bitmap),
        entry("objects/new/run.shader", shader),
    ];

    let index = build_dependency_candidate_index(&entries, &names);

    assert_eq!(
        index
            .get(&(bitmap, "run".to_owned()))
            .cloned()
            .unwrap_or_default(),
        vec!["objects\\new\\run".to_owned()]
    );
    assert_eq!(
        index
            .get(&(shader, "run".to_owned()))
            .cloned()
            .unwrap_or_default(),
        vec!["objects\\new\\run".to_owned()]
    );
}

#[test]
fn folder_reference_rewrites_point_moved_tags_at_new_folder() {
    let names = TagNameIndex::default();
    let bitmap = parse_group_tag("bitm").unwrap();
    let root = Path::new("C:/kit/tags");
    let source = root.join("objects/old");
    let destination = root.join("objects/new/old");
    let entries = vec![abs_entry(
        root,
        "objects/old/decal_road_1.bitmap.bitmap",
        bitmap,
    )];

    let rewrites =
        build_folder_reference_rewrites(root, &source, &destination, &entries, &names);

    assert_eq!(
        rewrites
            .get(&(bitmap, "objects\\old\\decal_road_1.bitmap".to_owned()))
            .cloned(),
        Some("objects\\new\\old\\decal_road_1.bitmap".to_owned())
    );
}

#[test]
fn rewrite_reference_prefilter_matches_ascii_case_insensitively() {
    let shader = parse_group_tag("rmsh").unwrap();
    let mut rewrites = HashMap::new();
    rewrites.insert(
        (shader, "objects\\characters\\bugger\\bugger".to_owned()),
        "zoeph_test\\bugger\\bugger".to_owned(),
    );
    let needles = rewrite_reference_needles(&rewrites);

    assert!(bytes_contain_any_ascii_case_insensitive(
        b"xx OBJECTS\\CHARACTERS\\BUGGER\\BUGGER yy",
        &needles
    ));
    assert!(!bytes_contain_any_ascii_case_insensitive(
        b"objects\\characters\\dervish\\dervish",
        &needles
    ));
}

#[test]
fn affected_move_entries_include_moved_tags_and_external_dependents() {
    let shader = parse_group_tag("rmsh").unwrap();
    let model = parse_group_tag("hlmt").unwrap();
    let old_shader = entry("objects/characters/jackal/jackal.shader", shader);
    let new_shader = entry("zoeph_test/jackal/jackal.shader", shader);
    let outside_model = entry("objects/characters/shared/shared.model", model);
    let unrelated = entry("objects/characters/brute/brute.model", model);
    let all_entries = vec![old_shader.clone(), outside_model.clone(), unrelated];
    let old_entries = vec![old_shader.clone()];
    let new_entries = vec![new_shader.clone()];
    let mut rewrites = HashMap::new();
    rewrites.insert(
        (shader, "objects\\characters\\jackal\\jackal".to_owned()),
        "zoeph_test\\jackal\\jackal".to_owned(),
    );
    let mut reverse = ReverseDependencyIndex::default();
    reverse.set_tag_dependencies(
        outside_model.key.clone(),
        vec![DependencyRef {
            group_tag: shader,
            rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
        }],
    );
    reverse.set_tag_dependencies(
        old_shader.key.clone(),
        vec![DependencyRef {
            group_tag: shader,
            rel_path: "objects\\characters\\jackal\\jackal".to_owned(),
        }],
    );

    let affected = affected_move_rewrite_entries(
        &all_entries,
        &old_entries,
        &new_entries,
        &rewrites,
        Some(&reverse),
    );
    let affected_keys = affected
        .into_iter()
        .map(|entry| entry.key)
        .collect::<HashSet<_>>();

    assert_eq!(affected_keys.len(), 2);
    assert!(affected_keys.contains(&new_shader.key));
    assert!(affected_keys.contains(&outside_model.key));
}
