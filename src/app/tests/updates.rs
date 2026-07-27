use super::*;

fn release(channel: UpdateChannel, tag: &str, commit: &str) -> UpdateCheckResult {
    UpdateCheckResult {
        channel,
        latest_tag: tag.to_owned(),
        release_url: "https://example.invalid/release".to_owned(),
        commit: commit.to_owned(),
    }
}

#[test]
fn stable_channel_compares_release_tags_against_the_package_version() {
    let current = env!("CARGO_PKG_VERSION");
    assert!(!is_update_available(&release(
        UpdateChannel::Stable,
        current,
        ""
    )));
    assert!(is_update_available(&release(
        UpdateChannel::Stable,
        "v999.0.0",
        ""
    )));
    assert!(!is_update_available(&release(
        UpdateChannel::Stable,
        "v0.0.1",
        ""
    )));
}

#[test]
fn stable_channel_ignores_the_release_commit() {
    // A stable release's identity is its tag; whatever commit it points at
    // must not change the verdict.
    let up_to_date = release(UpdateChannel::Stable, env!("CARGO_PKG_VERSION"), "deadbeef");
    assert!(!is_update_available(&up_to_date));
}

#[test]
fn development_channel_is_current_only_when_the_commit_matches() {
    // The development release always carries the `dev` tag, so a version
    // comparison would call every build current — the commit is the only
    // thing that separates one from the next.
    let matching = release(UpdateChannel::Development, "dev", BABOON_BUILD_COMMIT);
    assert_eq!(
        is_update_available(&matching),
        BABOON_BUILD_COMMIT.is_empty(),
        "a development release built from this very commit is not an update"
    );

    let different = release(
        UpdateChannel::Development,
        "dev",
        "0123456789abcdef0123456789abcdef01234567",
    );
    assert!(is_update_available(&different));
}

#[test]
fn an_unknown_build_commit_never_counts_as_current() {
    assert!(!is_same_commit("", "0123456"));
    assert!(!is_same_commit("0123456", ""));
    assert!(!is_same_commit("", ""));
    assert!(is_same_commit("0123ABC", "0123abc"));
}

#[test]
fn development_status_names_the_offered_commit_and_the_running_one() {
    let offered = "0123456789abcdef0123456789abcdef01234567";
    let status = update_check_status(&release(UpdateChannel::Development, "dev", offered));
    assert!(status.contains("0123456"), "{status}");
    assert!(
        !status.contains(offered),
        "the full hash is too long for a status line: {status}"
    );
    // The release is reached through the status bar's link, so repeating the
    // URL in prose would only duplicate it.
    assert!(!status.contains("http"), "{status}");
}

#[test]
fn a_missing_release_reads_differently_per_channel() {
    let stable = update_check_error_status(UpdateChannel::Stable, NO_PUBLIC_RELEASE_MESSAGE);
    let development =
        update_check_error_status(UpdateChannel::Development, NO_PUBLIC_RELEASE_MESSAGE);
    assert_ne!(stable, development);
    assert!(development.contains("development"), "{development}");
    // The sentinel is internal plumbing and must never reach the status line.
    assert!(!stable.contains(NO_PUBLIC_RELEASE_MESSAGE));
    assert!(!development.contains(NO_PUBLIC_RELEASE_MESSAGE));

    let failure = update_check_error_status(UpdateChannel::Stable, "connection refused");
    assert_eq!(failure, "Update check failed: connection refused");
}

#[test]
fn a_modified_working_tree_is_not_an_update_over_the_commit_it_sits_on() {
    // Otherwise every launch of every local build announces an update.
    assert!(is_same_commit("0123456789abcdef-dirty", "0123456789abcdef"));
    assert!(is_same_commit("0123456789abcdef", "0123456789abcdef-dirty"));
    // A dirty build on an older commit really is behind, and still says so.
    assert!(!is_same_commit(
        "0123456789abcdef-dirty",
        "fedcba9876543210"
    ));
}

#[test]
fn short_commit_abbreviates_hashes_and_keeps_the_dirty_marker() {
    assert_eq!(short_commit("0123456789abcdef"), "0123456");
    assert_eq!(short_commit("0123456789abcdef-dirty"), "0123456-dirty");
    assert_eq!(short_commit(""), "");
    assert_eq!(short_commit("main"), "main");
}
