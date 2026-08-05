//! Reading the shipped compatibility database at runtime.
//!
//! The generator's own tests prove the data is right. These prove the app can
//! open it, that the queries answer the questions the window asks, and that the
//! two halves agree — a database the build step is happy with and the reader
//! cannot open is worse than no database.

use super::*;

fn database() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs")
}

fn loaded() -> TagCompatUiState {
    let mut state = TagCompatUiState::default();
    state.ensure_loaded(&database());
    assert!(state.error().is_none(), "{:?}", state.error());
    state
}

#[test]
fn the_shipped_database_opens_and_covers_both_directions() {
    let state = loaded();
    let labels: Vec<String> = state.pairs.iter().map(CompatPair::label).collect();
    assert_eq!(
        labels,
        vec![
            "haloreach_mcc → haloce_evolved".to_owned(),
            "haloce_evolved → haloreach_mcc".to_owned(),
        ],
        "the sheet has to answer both directions",
    );
}

#[test]
fn a_missing_database_reports_rather_than_panics() {
    let mut state = TagCompatUiState::default();
    state.ensure_loaded(Path::new("no/such/directory"));
    assert!(state.error().is_some(), "a missing file must be reported, not ignored");
}

/// The window opens filtered to losses, because thirty thousand rows that
/// transfer fine are not what a reader came for.
#[test]
fn the_default_view_shows_only_what_is_lost() {
    let mut state = loaded();
    assert!(state.losses_only, "losses are the default view");
    state.refresh();
    assert!(!state.groups.is_empty(), "some groups do lose something");
    assert!(
        state.groups.iter().all(|row| row.verdict.is_loss()),
        "the filter must not leak clean groups into the loss view",
    );
}

/// Turning the filter off has to bring back the groups that convert cleanly,
/// or "only what is lost" is not a filter, it is the only view.
#[test]
fn clearing_the_filter_shows_the_clean_groups_too() {
    let mut state = loaded();
    state.refresh();
    let lossy = state.groups.len();
    state.losses_only = false;
    state.refresh();
    assert!(
        state.groups.len() > lossy,
        "unfiltered must be a superset: {} vs {lossy}",
        state.groups.len(),
    );
    assert!(
        state.groups.iter().any(|row| row.group == "sound_looping"),
        "sound_looping converts cleanly and should appear once the filter is off",
    );
}

/// The animation graph is the group this whole effort exists for, so the window
/// has to be able to answer for it specifically.
#[test]
fn the_animation_graph_reports_its_losses_with_locations() {
    let mut state = loaded();
    state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
    state.refresh();

    assert_eq!(state.selected_group.as_deref(), Some("model_animation_graph"));
    assert!(!state.fields.is_empty(), "there are rows to show");

    let dropped: Vec<&str> = state
        .fields
        .iter()
        .filter(|row| row.verdict == CompatVerdict::SourceOnly)
        .filter_map(|row| row.source_name.as_deref())
        .collect();
    for expected in ["node joint flags", "additional flags"] {
        assert!(dropped.contains(&expected), "{expected} is dropped: {dropped:?}");
    }

    let renamed = state
        .fields
        .iter()
        .find(|row| row.verdict == CompatVerdict::RenamedProvable)
        .expect("the blend-screen weight source is a recorded rename");
    assert!(
        !renamed.first_path.is_empty(),
        "every row needs a location — 'a field changed' is not actionable on an 83-struct group",
    );
}

/// `focus` is the hook the import dialog uses. It has to land on the answer,
/// including turning off the loss filter: a caller asking about a specific
/// group wants all of it.
#[test]
fn focusing_a_group_selects_it_and_shows_everything() {
    let mut state = loaded();
    state.focus("haloreach_mcc", "haloce_evolved", "sound_looping");
    state.refresh();
    assert!(!state.losses_only);
    assert_eq!(state.selected_group.as_deref(), Some("sound_looping"));
    assert_eq!(state.pairs[state.pair].source_game, "haloreach_mcc");
}

/// The export is the "sheet" deliverable, and it exports what is on screen —
/// so a reader gets what they were looking at, not a different query.
#[test]
fn the_export_matches_what_is_on_screen() {
    let mut state = loaded();
    state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
    state.refresh();
    let csv = state.visible_csv();

    let lines: Vec<&str> = csv.lines().collect();
    assert!(lines[0].starts_with("source_game,"), "a header row");
    assert_eq!(
        lines.len() - 1,
        state.fields.len(),
        "one row per visible field, no more and no fewer",
    );
    assert!(csv.contains("model_animation_graph"));
    for line in &lines[1..] {
        assert_eq!(line.matches('"').count() % 2, 0, "unbalanced quoting: {line}");
    }
}

/// Render the tab for real, against the shipped database.
///
/// The state tests above prove the queries answer correctly; this proves the
/// widget that shows them survives contact with the answers. An egui panel that
/// panics on an empty selection, a grid whose column count disagrees with the
/// cells pushed into it, a `SidePanel` nested somewhere it cannot go — none of
/// that shows up until something actually lays it out, and a native GL window
/// is not something a test can click through.
#[test]
fn the_tab_lays_out_against_the_shipped_database() {
    let mut state = loaded();

    let render = |state: &mut TagCompatUiState| {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(1100.0, 700.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| super::super::ui::help::draw_tag_compat_body_for_tests(ui, state));
            },
        );
    };

    // Nothing selected yet — the empty state is the first thing a user sees, so
    // it has to lay out before anything is chosen.
    render(&mut state);
    assert!(state.selected_group.is_none());
    assert!(!state.groups.is_empty(), "the group list populated during the frame");

    // A group selected, with rows for the grid to lay out.
    state.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
    render(&mut state);
    assert!(!state.fields.is_empty(), "the grid had rows to lay out");

    // And the clean case, where the grid is empty and the "nothing to report"
    // message takes its place.
    state.focus("haloreach_mcc", "haloce_evolved", "sound_looping");
    state.losses_only = true;
    render(&mut state);
    assert!(state.fields.is_empty(), "sound_looping loses nothing");
}

/// The reverse direction is a different question with a different answer, and
/// the window must not silently show one for the other.
#[test]
fn the_two_directions_disagree_about_what_is_dropped() {
    let mut forward = loaded();
    forward.focus("haloreach_mcc", "haloce_evolved", "model_animation_graph");
    forward.refresh();
    let mut backward = loaded();
    backward.focus("haloce_evolved", "haloreach_mcc", "model_animation_graph");
    backward.refresh();

    let dropped = |state: &TagCompatUiState| -> Vec<String> {
        state
            .fields
            .iter()
            .filter(|row| row.verdict == CompatVerdict::SourceOnly)
            .filter_map(|row| row.source_name.clone())
            .collect()
    };
    assert_ne!(
        dropped(&forward),
        dropped(&backward),
        "Reach loses node flags going in; Campaign Evolved loses its own additions coming back",
    );
}
