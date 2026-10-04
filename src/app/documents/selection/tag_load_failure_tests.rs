use super::*;

/// A tag that fails to load says so. The terminal line was copied from the
/// folder-refactor handler and reported "Folder refactor failed".
#[test]
fn a_failed_tag_load_names_the_tag() {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.kit_and_view(0).open_tag_pane("objects/broken.model");

    app.handle_tag_loaded(
        kit,
        "objects/broken.model".to_owned(),
        Err("truncated".to_owned()),
    );

    let line = &app.kit_tools.terminal.lines.last().expect("a terminal line").text;
    assert_eq!(line, "Could not load objects/broken.model: truncated");
    assert_eq!(app.model.status, *line);
}
