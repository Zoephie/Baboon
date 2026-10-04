use super::*;

fn commit() -> GitReviewCommit {
    GitReviewCommit {
        hash: "9d31cd291ebb8d3e".to_owned(),
        short_hash: "9d31cd2".to_owned(),
        author: "Paddy Tee".to_owned(),
        date: "2026-09-20".to_owned(),
        subject: "Add Git Review search".to_owned(),
    }
}

#[test]
fn commit_filter_matches_title_author_and_hash_case_insensitively() {
    let commit = commit();
    assert!(commit_matches_filter(&commit, "git review"));
    assert!(commit_matches_filter(&commit, "PADDY"));
    assert!(commit_matches_filter(&commit, "9D31CD2"));
    assert!(!commit_matches_filter(&commit, "unrelated"));
}

#[test]
fn empty_commit_filter_matches_every_commit() {
    assert!(commit_matches_filter(&commit(), "  "));
}

#[test]
fn commit_tooltip_only_shows_truncated_text_without_full_hash() {
    let commit = commit();
    let metadata = "9d31cd2  ·  Paddy Tee  ·  2026-09-20";
    assert_eq!(
        commit_row_tooltip(&commit, &commit.subject, metadata, metadata),
        None
    );
    assert_eq!(
        commit_row_tooltip(&commit, "Add Git…", metadata, metadata),
        Some(commit.subject.clone())
    );
    assert_eq!(
        commit_row_tooltip(&commit, &commit.subject, metadata, "9d31cd2…"),
        Some(metadata.to_owned())
    );
    assert_eq!(
        commit_row_tooltip(&commit, "Add Git…", metadata, "9d31cd2…"),
        Some(format!("{}\n{metadata}", commit.subject))
    );
    assert!(
        !commit_row_tooltip(&commit, "Add Git…", metadata, "9d31cd2…")
            .unwrap()
            .contains(&commit.hash)
    );
}

#[test]
fn changed_tags_header_uses_selected_revision_title() {
    let commit = commit();
    assert_eq!(
        selected_revision_title(&GitReviewSelection::Local, &[commit.clone()]),
        "Local Changes"
    );
    assert_eq!(
        selected_revision_title(&GitReviewSelection::Commit(commit.hash.clone()), &[commit],),
        "Add Git Review search"
    );
}

#[test]
fn change_summary_groups_git_statuses() {
    let files = ['A', '?', 'M', 'R', 'D']
        .into_iter()
        .map(|status| GitReviewFile {
            status: status.to_string(),
            path: format!("{status}.tag"),
            group_tag: 0,
        })
        .collect::<Vec<_>>();
    let counts = change_counts(&files);
    assert_eq!(counts.added, 2);
    assert_eq!(counts.modified, 2);
    assert_eq!(counts.removed, 1);
}
