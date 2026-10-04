use super::*;

/// A row of a query run against a workspace that has since closed is inert:
/// the key could name an unrelated tag in whatever kit is active now.
#[test]
fn a_query_result_from_a_closed_workspace_opens_nothing() {
    let mut app = Baboon::for_test();
    let closed = app.add_kit();
    app.remove_kit(closed);
    app.commands.send(SearchCommand::QueryResult {
        kit: closed,
        action: QueryResultAction::Open {
            key: "file:objects/a.weapon".to_owned(),
            ref_target: Some((u32::from_be_bytes(*b"weap"), "objects/b".to_owned())),
        },
    });
    app.apply_commands(&egui::Context::default());
    assert_eq!(app.model.status, "That workspace has been closed");
    assert!(app.references.pending_ref_jump.is_none());
}

/// Opening a row of a "References to X" query queues a jump to the field that
/// points at X, to land once the referrer has loaded.
#[test]
fn opening_a_references_row_queues_the_jump_to_its_field() {
    let mut app = Baboon::for_test();
    let kit = app.model.kits[0].id;
    app.commands.send(SearchCommand::QueryResult {
        kit,
        action: QueryResultAction::Open {
            key: "file:objects/a.weapon".to_owned(),
            ref_target: Some((u32::from_be_bytes(*b"weap"), "objects/b".to_owned())),
        },
    });
    app.apply_commands(&egui::Context::default());
    let jump = app.references.pending_ref_jump.as_ref().expect("a jump is queued");
    assert_eq!(jump.tag_key, "file:objects/a.weapon");
    assert_eq!(jump.rel_path, "objects/b");
}
