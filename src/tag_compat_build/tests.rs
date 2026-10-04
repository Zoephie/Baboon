//! The compatibility database, checked against the definitions it is derived
//! from.
//!
//! These run for real on CI rather than self-skipping: `build.rs` fails the
//! build without the `definitions/` submodule, so it is always present.

use super::*;

fn definitions() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions")
}

fn catalog() -> ReviewedCatalogHandle {
    ReviewedCatalogHandle::embedded().expect("the engine's mapping catalog parses")
}

fn reach_to_ce() -> Vec<PairReportHandle> {
    let catalog = catalog();
    vec![
        analyze_pair(&definitions(), &catalog, "haloreach_mcc", "haloce_evolved")
            .expect("analyze Reach to Campaign Evolved"),
    ]
}

/// Regenerating from the pinned submodule must reproduce the checked-in
/// artifact.
///
/// Compared on the generator's own content digest rather than on the file:
/// SQLite page allocation is not byte-reproducible, so a file comparison would
/// fail for reasons that have nothing to do with the data.
#[test]
fn the_checked_in_database_matches_the_definitions_it_was_built_from() {
    let (definitions, output, _) = default_paths();
    if !output.exists() {
        eprintln!(
            "skipping: {} has not been generated yet — run `cargo run --bin build_tag_compat`",
            output.display(),
        );
        return;
    }
    let pairs: Vec<(String, String)> = DEFAULT_PAIRS
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    let catalog = catalog();
    let reports: Vec<_> = pairs
        .iter()
        .map(|(source, target)| {
            analyze_pair(&definitions, &catalog, source, target).expect("analyze pair")
        })
        .collect();

    let connection = rusqlite::Connection::open(&output).expect("open the checked-in database");
    let stored: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key='content_digest'",
            [],
            |row| row.get(0),
        )
        .expect("the database records a content digest");
    assert_eq!(
        stored,
        content_digest(&reports),
        "{} is stale — run `cargo run --bin build_tag_compat`",
        output.display(),
    );
}

/// The database has to agree with the engine about `model_animation_graph`,
/// since the conversion gate and the sheet a user reads must not tell two
/// different stories.
#[test]
fn the_animation_graph_reports_its_four_size_changed_structs() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];
    let jmad = report
        .groups
        .iter()
        .find(|group| group.group == "model_animation_graph")
        .expect("both games define model_animation_graph");

    assert_eq!(jmad.size_diff_structs, 4);
    assert_eq!(
        jmad.verdict,
        CompatVerdict::SourceOnly,
        "Reach carries node flags Campaign Evolved does not, so the group is lossy",
    );
    assert!(
        jmad.blocked_reason.is_none(),
        "it converts, it is not refused"
    );
    assert!(
        jmad.source_only_fields >= 2,
        "at least the two node-flag bytes"
    );
}

/// The alias rescue has to survive the trip into the database, or the sheet
/// overstates the loss exactly where a reader most needs it accurate.
#[test]
fn the_blend_screen_rename_reaches_the_database_as_a_rename() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];
    let jmad = report
        .groups
        .iter()
        .find(|group| group.group == "model_animation_graph")
        .expect("model_animation_graph");

    let renamed = jmad
        .fields
        .iter()
        .find(|row| row.source_name.as_deref() == Some("weight source"))
        .expect("Reach declares a blend-screen weight source");
    assert_eq!(renamed.verdict, CompatVerdict::RenamedProvable);
    assert_eq!(
        renamed.target_name.as_deref(),
        Some("primary weight source")
    );
    assert_eq!(renamed.rule, "schema_alias");
}

/// A group both games declare identically must produce no losses at all —
/// otherwise the "only losses" filter is useless and the sheet is noise.
#[test]
fn an_identical_group_reports_nothing_to_review() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];
    let looping = report
        .groups
        .iter()
        .find(|group| group.group == "sound_looping")
        .expect("both games define sound_looping");
    assert_eq!(looping.verdict, CompatVerdict::Identical);
    assert_eq!(looping.source_only_fields, 0);
    assert_eq!(looping.size_diff_structs, 0);
}

/// Groups only one game defines are listed and explained rather than silently
/// omitted. A sheet that quietly drops rows cannot be used to answer "what
/// about X?".
#[test]
fn a_group_only_one_game_defines_is_listed_with_a_reason() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];

    // Campaign Evolved ships these; Reach does not.
    let ce_only = report
        .groups
        .iter()
        .find(|group| group.group == "skull_globals")
        .expect("skull_globals must be listed even though Reach lacks it");
    assert_eq!(ce_only.verdict, CompatVerdict::HardBlocked);
    assert!(
        ce_only
            .blocked_reason
            .as_deref()
            .is_some_and(|r| r.contains("haloreach_mcc")),
        "the reason must name the game that lacks it: {:?}",
        ce_only.blocked_reason,
    );
}

/// The measured shape of the two games' overlap, pinned so a definitions bump
/// that changes it is noticed here.
#[test]
fn the_two_games_share_the_expected_number_of_groups() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];
    let shared = report
        .groups
        .iter()
        .filter(|group| group.blocked_reason.is_none())
        .count();
    // 131 groups exist in both games; the reviewed catalog refuses `sound`
    // between every pair, which leaves 130 that can cross.
    assert_eq!(
        shared, 130,
        "Reach and Campaign Evolved share 130 tag groups that can cross"
    );
}

/// Every reviewed rename scoped to this pair has to actually fire. A rule
/// written for a field name the schemas no longer carry reads as coverage and
/// provides none.
#[test]
fn every_reviewed_rename_for_this_pair_matches_a_field() {
    let catalog = catalog();
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];

    let mut stale = Vec::new();
    for group in &report.groups {
        for source in catalog
            .0
            .renames(&group.group, &report.source_game, &report.target_game)
            .keys()
        {
            let present = group.fields.iter().any(|row| {
                row.source_name.as_deref() == Some(source.as_str())
                    || row.target_name.as_deref() == Some(source.as_str())
            });
            if !present {
                stale.push(format!("{}/{source}", group.group));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "reviewed renames that match nothing: {stale:?}"
    );
}

/// The reviewed layer must actually be read. It was silently empty once: the
/// generator read a catalog file that had been deleted, turned the missing
/// file into an empty catalog, and every test above still passed over zero
/// rules — so the database said a sound tag crosses from Reach to Campaign
/// Evolved. The catalog refuses `sound` between every pair of games (a sound
/// tag is a header for audio that lives elsewhere); that refusal has to reach
/// this pair.
#[test]
fn the_reviewed_sound_refusal_reaches_reach_to_campaign_evolved() {
    let reports = reach_to_ce();
    let PairReportHandle(report) = &reports[0];
    let sound = report
        .groups
        .iter()
        .find(|group| group.group == "sound")
        .expect("both games define sound");
    let reason = sound
        .blocked_reason
        .as_deref()
        .expect("sound is refused by the reviewed catalog");
    assert!(
        reason.contains("A sound tag is a header"),
        "sound is blocked, but not by the reviewed refusal: {reason}"
    );
}

/// A catalog file that is missing or does not parse is an error, not an empty
/// catalog.
#[test]
fn a_missing_or_malformed_catalog_file_is_an_error() {
    let missing = std::env::temp_dir().join(format!(
        "baboon-no-such-catalog-{}.json",
        std::process::id()
    ));
    assert!(ReviewedCatalogHandle::load(&missing).is_err());
    let malformed = std::env::temp_dir().join(format!(
        "baboon-malformed-catalog-{}.json",
        std::process::id()
    ));
    std::fs::write(&malformed, b"{ not json").expect("write the malformed catalog");
    let result = ReviewedCatalogHandle::load(&malformed);
    let _ = std::fs::remove_file(&malformed);
    assert!(result.is_err());
}

/// `--suggest-drops` has to emit something a reviewer can paste, for the group
/// whose review actually blocks the import.
#[test]
fn suggested_drops_name_the_animation_graph_losses() {
    let reports = reach_to_ce();
    let suggestion = suggest_drops(&reports, Some("model_animation_graph"));
    let parsed: serde_json::Value =
        serde_json::from_str(&suggestion).expect("the suggestion is valid JSON");
    let entries = parsed["accepted_field_drops"]
        .as_array()
        .expect("an accepted_field_drops array");

    assert!(!entries.is_empty(), "jmad has losses to catalogue");
    let paths: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry["source_path"].as_str())
        .collect();
    for expected in ["node joint flags", "additional flags"] {
        assert!(
            paths.contains(&expected),
            "{expected} should be suggested: {paths:?}"
        );
    }
    for entry in entries {
        assert_eq!(entry["group"], "model_animation_graph");
        assert!(
            entry["reason"]
                .as_str()
                .is_some_and(|r| r.contains("REVIEW")),
            "a suggestion is a prompt to review, not a decision",
        );
    }
}

/// The CSV is the deliverable a reader opens outside the app, so it has to be
/// well-formed and lead with the losses.
#[test]
fn the_csv_leads_with_losses_and_quotes_correctly() {
    let reports = reach_to_ce();
    let directory = std::env::temp_dir().join("baboon-tag-compat-csv-test");
    let path = directory.join("sheet.csv");
    write_csv(&reports, &path).expect("write the sheet");
    let text = std::fs::read_to_string(&path).expect("read it back");
    let _ = std::fs::remove_dir_all(&directory);

    let mut lines = text.lines();
    assert!(
        lines
            .next()
            .is_some_and(|header| header.starts_with("source_game,"))
    );
    let first = lines.next().expect("at least one row");
    assert!(
        first.contains("hard_blocked") || first.contains("source_only"),
        "losses sort first, got: {first}",
    );
    for line in text.lines() {
        assert_eq!(
            line.matches('"').count() % 2,
            0,
            "unbalanced quoting in: {line}",
        );
    }
}
