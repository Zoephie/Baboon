use super::*;

thread_local! {
    /// Result rows laid out, to tell a virtualized list from one that
    /// lays out every row and lets egui cull the painting.
    pub(super) static ROWS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn an_expanded_result_lists_its_fields_under_it() {
    use QueryResultRow::*;
    let expansion = |index: usize| match index {
        1 => Some(None),
        2 => Some(Some(0)),
        3 => Some(Some(2)),
        _ => None,
    };
    assert_eq!(
        query_result_rows(5, true, expansion),
        [
            Entry(0),
            Entry(1),
            Loading(1),
            Entry(2),
            NoOccurrences(2),
            Entry(3),
            Occurrence(3, 0),
            Occurrence(3, 1),
            Entry(4),
        ]
    );
    assert_eq!(
        query_result_rows(2, false, expansion),
        [Entry(0), Entry(1)],
        "only a references query expands"
    );
}

/// The window draws the rows in view, not all of them.
#[test]
fn the_query_results_window_draws_only_rows_in_view() {
    let mut app = Baboon::for_test();
    let entries: Vec<TagEntry> = (0..5_000)
        .map(|index| TagEntry {
            key: format!("file:sound/row_{index}.sound"),
            display_path: format!("sound/row_{index}.sound"),
            group_tag: u32::from_be_bytes(*b"snd!"),
            group_name: Some("sound".to_owned()),
            location: TagEntryLocation::LooseFile(format!("sound/row_{index}.sound").into()),
        })
        .collect();
    let ctx = egui::Context::default();
    let mut painted = Vec::new();
    for _ in 0..2 {
        ROWS_BUILT.with(|built| built.set(0));
        app.dialogs.open(QueryResultsWindow::new(TagQueryResults {
            kit: app.model.kits[0].id,
            title: "Sounds".to_owned(),
            entries: entries.clone(),
            annotations: Vec::new(),
            note: None,
            ref_target: None,
        }));
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |_| app.dialogs.draw(&cx!(app, &ctx), &app_reads!(app)),
        );
        painted = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .filter(|text| text.starts_with("sound/row_"))
            .collect();
    }
    assert!(painted.contains(&"sound/row_0.sound".to_owned()));
    assert!(!painted.contains(&"sound/row_4999.sound".to_owned()));
    let built = ROWS_BUILT.with(std::cell::Cell::get);
    assert!(built < 100, "laid out {built} of 5,000 rows");
}
