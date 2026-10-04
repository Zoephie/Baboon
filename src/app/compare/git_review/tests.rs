use super::*;

fn profile(id: &str, git_tracked: bool) -> CustomEditingKitProfile {
    CustomEditingKitProfile {
        read_only: false,
        git_tracked,
        id: id.to_owned(),
        name: id.to_owned(),
        game: "halo2_mcc".to_owned(),
        root: PathBuf::from(id),
        icon: None,
        tags_folder: None,
        data_folder: None,
    }
}

#[test]
fn git_review_requires_the_matching_profile_to_be_git_tracked() {
    let identity = EditingKitProfileIdentity {
        id: "tracked".to_owned(),
        name: "Tracked".to_owned(),
    };
    let profiles = vec![profile("tracked", true), profile("other", false)];
    assert!(profile_has_git_tracking(Some(&identity), &profiles));

    let disabled = EditingKitProfileIdentity {
        id: "other".to_owned(),
        name: "Other".to_owned(),
    };
    assert!(!profile_has_git_tracking(Some(&disabled), &profiles));
    assert!(!profile_has_git_tracking(None, &profiles));
}

#[test]
fn status_parsers_hide_non_tags_and_keep_tag_types() {
    let files =
        parse_local_status(" M tags/a.weapon\nR  old/path -> tags/b.weapon\n?? notes.txt\n");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].status, "M");
    assert_eq!(files[0].path, "tags/a.weapon");
    assert_eq!(files[0].group_tag, u32::from_be_bytes(*b"weap"));
    assert_eq!(files[1].path, "tags/b.weapon");
}

#[test]
fn parses_commit_name_status() {
    let files =
        parse_name_status("M\ttags/a.weapon\nR100\told.weapon\tnew.weapon\nD\tREADME.md\n");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].status, "M");
    assert_eq!(files[1].path, "new.weapon");
}

#[test]
fn git_paths_become_native_worktree_paths_without_changing_git_strings() {
    let repo = Path::new("repo-root");
    let path = git_worktree_path(repo, "tags/objects/rifle.weapon");
    assert_eq!(path, repo.join("tags").join("objects").join("rifle.weapon"));
    #[cfg(windows)]
    assert_eq!(
        native_git_display_path("tags/objects/rifle.weapon"),
        r"tags\objects\rifle.weapon"
    );
    #[cfg(not(windows))]
    assert_eq!(
        native_git_display_path("tags/objects/rifle.weapon"),
        "tags/objects/rifle.weapon"
    );
}

#[test]
fn git_repository_root_uses_native_separators_for_shell_handoffs() {
    let root = PathBuf::from(native_git_display_path("C:/Program Files/H2CE"));
    #[cfg(windows)]
    assert_eq!(root.to_string_lossy(), r"C:\Program Files\H2CE");
    #[cfg(not(windows))]
    assert_eq!(root.to_string_lossy(), "C:/Program Files/H2CE");
}

#[test]
fn local_changes_sort_by_full_path_not_status() {
    let mut files = parse_local_status(
        "?? tags/z.weapon\n M tags/b.weapon\n?? tags/a.weapon\n D tags/c.weapon\n",
    );
    sort_files_by_full_path(&mut files);
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        [
            "tags/a.weapon",
            "tags/b.weapon",
            "tags/c.weapon",
            "tags/z.weapon"
        ]
    );
}

#[test]
fn refresh_restores_only_revisions_still_in_the_same_repository() {
    let repo = Path::new("repo");
    let commit = GitReviewCommit {
        hash: "abc123".to_owned(),
        short_hash: "abc123".to_owned(),
        author: String::new(),
        date: String::new(),
        subject: String::new(),
    };
    assert!(can_restore_review_selection(
        Some(repo),
        &GitReviewSelection::Local,
        repo,
        &[],
    ));
    assert!(can_restore_review_selection(
        Some(repo),
        &GitReviewSelection::Commit(commit.hash.clone()),
        repo,
        &[commit],
    ));
    assert!(!can_restore_review_selection(
        Some(repo),
        &GitReviewSelection::Commit("missing".to_owned()),
        repo,
        &[],
    ));
    assert!(!can_restore_review_selection(
        Some(Path::new("other")),
        &GitReviewSelection::Local,
        repo,
        &[],
    ));
}

/// Jobs finish in any order. A slow comparison for a file the user has
/// already clicked past must not land over the newer one.
#[test]
fn an_older_git_review_result_is_dropped() {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.views[app.model.kits[0].id].git_review.request = 2;
    app.views[app.model.kits[0].id].git_review.loading = true;
    let view = |path: &str| GitReviewView {
        selected_path: Some(path.to_owned()),
        ..Default::default()
    };

    app.handle_git_review_updated(kit, 1, Ok(view("first.weapon")));
    let state = &app.views[app.model.kits[0].id].git_review;
    assert_eq!(state.selected_path, None, "stale: not applied");
    assert!(state.loading, "the newer job is still running");

    app.handle_git_review_updated(kit, 2, Ok(view("second.weapon")));
    let state = &app.views[app.model.kits[0].id].git_review;
    assert_eq!(state.selected_path.as_deref(), Some("second.weapon"));
    assert!(!state.loading);
}

/// Opening a reviewed file finds the entry the kit's own scan made, by
/// key, rather than adding a second one for the same file.
#[test]
fn opening_a_reviewed_file_finds_the_scanned_entry() {
    let root = std::env::temp_dir().join(format!("baboon-git-open-{}", uuid::Uuid::new_v4()));
    let tags = root.join("tags");
    let path = tags.join("objects").join("rifle.weapon");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = vec![0u8; 64];
    bytes[36..40].copy_from_slice(b"weap");
    fs::write(&path, &bytes).unwrap();
    let entry = TagEntry {
        key: file_entry_key(&path),
        display_path: "objects/rifle.weapon".to_owned(),
        group_tag: u32::from_be_bytes(*b"weap"),
        group_name: Some("weapon".to_owned()),
        location: TagEntryLocation::LooseFile(path.clone()),
    };
    let entries = vec![entry.clone()];

    let mut app = Baboon::for_test();
    app.model.kits[0].source = Some(LoadedSourceData {
        label: "test".to_owned(),
        source: TagSource::LooseFolder {
            root: tags.clone(),
            game: None,
            definitions_root: PathBuf::new(),
        },
        names: Default::default(),
        game: None,
        tree: crate::core::source::build_tree(&entries),
        group_tree: crate::core::source::build_tree(&entries),
        entries,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    });
    app.views[app.model.kits[0].id].git_review.repo_root = Some(root.clone());
    let generation = app.model.kits[0].generation;

    app.open_git_review_file(0, "tags/objects/rifle.weapon");
    let _ = fs::remove_dir_all(&root);

    assert_eq!(app.views[app.model.kits[0].id].git_review.pending_open, Some(entry.key));
    assert_eq!(app.model.kits[0].generation, generation, "no entry was added");
    assert_eq!(app.model.kits[0].source.as_ref().unwrap().entries.len(), 1);
}
