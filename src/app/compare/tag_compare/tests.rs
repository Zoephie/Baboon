use super::*;

#[test]
fn git_head_reads_committed_tag_instead_of_working_copy() {
    let root = std::env::temp_dir().join(format!("baboon-git-head-{}", uuid::Uuid::new_v4()));
    let kit = root.join("kit");
    let tag = kit.join("tags").join("objects").join("example.model");
    std::fs::create_dir_all(tag.parent().unwrap()).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(&tag, b"committed tag").unwrap();
    git(&["add", "kit/tags/objects/example.model"]);
    git(&[
        "-c",
        "user.name=Baboon Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "test tag",
    ]);
    std::fs::write(&tag, b"edited tag").unwrap();
    assert_eq!(
        git_tag_bytes(&kit.join("tags"), &tag, "HEAD").unwrap(),
        b"committed tag"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_history_pages_only_commits_that_changed_the_tag() {
    let root =
        std::env::temp_dir().join(format!("baboon-git-history-{}", uuid::Uuid::new_v4()));
    let tag = root.join("kit/tags/objects/example.model");
    std::fs::create_dir_all(tag.parent().unwrap()).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    for index in 0..12 {
        std::fs::write(&tag, format!("version {index}")).unwrap();
        git(&["add", "kit/tags/objects/example.model"]);
        git(&[
            "-c",
            "user.name=Baboon Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            &format!("Tag revision {index}"),
        ]);
    }
    std::fs::write(root.join("unrelated.txt"), b"unrelated").unwrap();
    git(&["add", "unrelated.txt"]);
    git(&[
        "-c",
        "user.name=Baboon Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "Unrelated change",
    ]);
    let tags_root = root.join("kit/tags");
    std::fs::write(&tag, b"uncommitted local version").unwrap();
    let (first, has_more) = git_tag_history(&tags_root, &tag, 0).unwrap();
    assert_eq!(first.len(), 10);
    assert!(has_more);
    assert_eq!(first[0].subject, "Tag revision 11");
    assert_eq!(first[0].author, "Baboon Test");
    let latest_parent = git_commit_parent(&tags_root, &first[0].hash)
        .unwrap()
        .unwrap();
    assert_eq!(
        git_tag_bytes_if_present(&tags_root, &tag, &latest_parent).unwrap(),
        Some(b"version 10".to_vec())
    );
    assert_eq!(
        git_tag_bytes_if_present(&tags_root, &tag, &first[0].hash).unwrap(),
        Some(b"version 11".to_vec())
    );
    assert_eq!(first[9].subject, "Tag revision 2");
    let (older, has_more) = git_tag_history(&tags_root, &tag, first.len()).unwrap();
    assert_eq!(older.len(), 2);
    assert!(!has_more);
    assert_eq!(older[0].subject, "Tag revision 1");
    assert_eq!(older[1].subject, "Tag revision 0");
    assert_eq!(
        git_tag_bytes(&tags_root, &tag, &older[0].hash).unwrap(),
        b"version 1"
    );
    let parent = git_commit_parent(&tags_root, &older[0].hash)
        .unwrap()
        .unwrap();
    assert_eq!(
        git_tag_bytes_if_present(&tags_root, &tag, &parent).unwrap(),
        Some(b"version 0".to_vec())
    );
    assert_eq!(git_commit_parent(&tags_root, &older[1].hash).unwrap(), None);
    assert_eq!(
        git_tag_bytes_if_present(&tags_root, &tag, &older[1].hash).unwrap(),
        Some(b"version 0".to_vec())
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_tag_is_shown_as_added_or_removed() {
    let tag = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
    let added = comparison_results_with_missing(None, Some(&tag));
    assert_eq!(added.diffs[0].b, "added — tag");
    assert_eq!(block_change_icon(&added.diffs[0]), Some(ButtonIcon::Add));
    assert_eq!(added.reverse_diffs[0].a, "removed — tag");
    assert_eq!(
        block_change_icon(&added.reverse_diffs[0]),
        Some(ButtonIcon::Remove)
    );

    let removed = comparison_results_with_missing(Some(&tag), None);
    assert_eq!(removed.diffs[0].a, "removed — tag");
    assert_eq!(
        block_change_icon(&removed.diffs[0]),
        Some(ButtonIcon::Remove)
    );
}

#[test]
fn displayed_paths_use_the_platform_separator_without_changing_keys() {
    let path = r"C:\kit/tags\objects/example.model";
    let shown = native_display_path(path);
    assert_eq!(
        shown,
        path.replace(['\\', '/'], &std::path::MAIN_SEPARATOR.to_string())
    );
    assert_eq!(path, r"C:\kit/tags\objects/example.model");
}

#[test]
fn long_paths_keep_the_filename_visible() {
    let full = "C:/long/folder/example.model";
    let shown = truncate_start(full, 18.0, |text| text.chars().count() as f32);
    assert!(shown.starts_with('…'));
    assert!(shown.ends_with("example.model"));
    assert!(shown.chars().count() as f32 <= 18.0);
    assert_eq!(
        truncate_start(full, 40.0, |text| text.chars().count() as f32),
        full
    );
}

#[test]
fn matching_tag_keeps_path_and_type_below_tags_root() {
    let base = std::env::temp_dir();
    let current_root = base.join("H2R").join("tags");
    let reference_root = base.join("H2EK").join("tags");
    let relative = Path::new("objects")
        .join("characters")
        .join("brute")
        .join("brute.model");
    let key = file_entry_key(&current_root.join(&relative));
    assert_eq!(
        matching_tag_path(&key, &current_root, &reference_root),
        Some(reference_root.join(relative))
    );
    let elsewhere = format!(
        "file:{}",
        base.join("Elsewhere").join("brute.model").display()
    );
    assert_eq!(
        matching_tag_path(&elsewhere, &current_root, &reference_root),
        None
    );
}

/// A Git read the window has moved past — the history page it asked for
/// before a compare — is dropped; the one it is waiting on applies.
#[test]
fn a_superseded_compare_git_read_is_dropped() {
    let mut app = Baboon::for_test();
    app.dialogs.open(TagDiffState {
        kit: app.model.kits[0].id,
        a_key: "file:a.weapon".to_owned(),
        source: TagCompareSource::GitHistory,
        b_kit: None,
        b_key: None,
        b_path: None,
        comparison_kit_root: None,
        git_history: GitHistoryState::default(),
        error: None,
        filters: TagDiffFilters::default(),
        swapped: false,
        results: None,
        git_pending: Some(2),
    });
    let page = |subject: &str| TagCompareGitUpdate::History {
        append: false,
        result: Ok((
            vec![GitHistoryCommit {
                hash: subject.to_owned(),
                short_hash: subject.to_owned(),
                date: String::new(),
                author: String::new(),
                subject: subject.to_owned(),
            }],
            false,
        )),
    };

    app.handle_tag_compare_git(1, Ok(page("old")));
    let state = app.dialogs.get::<TagDiffState>().unwrap();
    assert!(state.git_history.commits.is_empty(), "superseded: dropped");
    assert_eq!(state.git_pending, Some(2));

    app.handle_tag_compare_git(2, Ok(page("new")));
    let state = app.dialogs.get::<TagDiffState>().unwrap();
    assert_eq!(state.git_history.commits[0].subject, "new");
    assert_eq!(state.git_pending, None);
}

#[test]
fn open_tag_selection_uses_its_owning_kit() {
    let key = "same-key".to_owned();
    let mut first = Kit::empty(KitId(1), Default::default());
    let mut second = Kit::empty(KitId(2), Default::default());
    first.parsed_tags.insert(
        key.clone(),
        TagDocument::clean(TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap()),
    );
    second.parsed_tags.insert(
        key.clone(),
        TagDocument::clean(TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap()),
    );
    let kits = [first, second];
    let selected = selected_open_tag(&kits, Some(KitId(2)), Some(&key)).unwrap();
    assert!(std::ptr::eq(selected, &kits[1].parsed_tags[&key].tag));
    assert!(selected_open_tag(&kits, Some(KitId(3)), Some(&key)).is_none());
}

#[test]
fn diff_filters_follow_which_columns_have_values() {
    let row = |a: &str, b: &str| TagFieldDiff {
        path: "field".to_owned(),
        base_path: None,
        a: a.to_owned(),
        b: b.to_owned(),
    };
    let mut filters = TagDiffFilters::default();
    assert!(show_diff(filters, &row("old", "new")));
    assert!(show_diff(filters, &row("old", "")));
    assert!(show_diff(filters, &row("", "new")));

    filters.both = false;
    assert!(!show_diff(filters, &row("old", "new")));
    assert!(show_diff(filters, &row("old", "")));
    assert!(show_diff(filters, &row("", "new")));

    filters.current_only = false;
    assert!(!show_diff(filters, &row("old", "")));
    assert!(show_diff(filters, &row("", "new")));
    filters.comparison_only = false;
    assert!(!show_diff(filters, &row("", "new")));
}

#[test]
fn numeric_delta_shows_precise_finite_numeric_changes() {
    let row = |a: &str, b: &str| TagFieldDiff {
        path: "field".to_owned(),
        base_path: None,
        a: a.to_owned(),
        b: b.to_owned(),
    };
    assert_eq!(
        numeric_delta(&row("3", "4.5")),
        Some((Ordering::Greater, "1.5".to_owned()))
    );
    assert_eq!(
        numeric_delta(&row("-2", "-3")),
        Some((Ordering::Less, "1".to_owned()))
    );
    assert_eq!(
        numeric_delta(&row("0.2", "0.3")),
        Some((Ordering::Greater, "0.1".to_owned()))
    );
    assert_eq!(
        numeric_delta(&row("1e-3", "2e-3")),
        Some((Ordering::Greater, "0.001".to_owned()))
    );
    assert_eq!(
        numeric_delta(&row("0", "1e-16")),
        Some((Ordering::Greater, "1e-16".to_owned()))
    );
    assert_eq!(numeric_delta(&row("1e2", "100")), None);
    assert_eq!(numeric_delta(&row("", "3")), None);
    assert_eq!(numeric_delta(&row("1", "NaN")), None);
    assert_eq!(numeric_delta(&row("1 red", "2 blue")), None);
}

#[test]
fn block_change_icons_only_mark_added_or_removed_elements() {
    let row = |base_path: Option<&str>, a: &str, b: &str| TagFieldDiff {
        path: "variants[0]".to_owned(),
        base_path: base_path.map(str::to_owned),
        a: a.to_owned(),
        b: b.to_owned(),
    };
    assert_eq!(
        block_change_icon(&row(None, "", "added — variant")),
        Some(ButtonIcon::Add)
    );
    assert_eq!(
        block_change_icon(&row(Some("variants[0]"), "removed — variant", "")),
        Some(ButtonIcon::Remove)
    );
    assert_eq!(block_change_icon(&row(None, "", "field value")), None);
    assert_eq!(
        block_change_icon(&row(Some("variants[0]"), "variant", "variant")),
        None
    );
}

#[test]
fn swap_recomputes_block_changes_in_the_opposite_direction() {
    let a = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
    let mut b = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
    crate::app::add_block_element(&mut b, "sound classes").unwrap();
    let results = comparison_results(&a, &b);
    assert!(
        results
            .diffs
            .iter()
            .any(|diff| block_change_icon(diff) == Some(ButtonIcon::Add))
    );
    assert!(
        results
            .reverse_diffs
            .iter()
            .any(|diff| block_change_icon(diff) == Some(ButtonIcon::Remove))
    );
}

#[test]
fn swap_keeps_one_sided_filters_tied_to_the_same_tag() {
    let filters = TagDiffFilters {
        both: false,
        current_only: true,
        comparison_only: false,
    };
    let swapped = filters_for_display(filters, true);
    assert!(!swapped.current_only);
    assert!(swapped.comparison_only);
    let reverse_row = TagFieldDiff {
        path: "field".to_owned(),
        base_path: None,
        a: String::new(),
        b: "current value".to_owned(),
    };
    assert!(show_diff(swapped, &reverse_row));
    assert_eq!(
        numeric_delta(&TagFieldDiff {
            a: "40".to_owned(),
            b: "70".to_owned(),
            ..reverse_row
        }),
        Some((Ordering::Greater, "30".to_owned()))
    );
}
