use super::*;

/// A Git Review request names its kit by id, so it lands on that kit's
/// review whatever has happened to the kit list since the click.
#[test]
fn a_git_review_command_reaches_the_kit_it_names() {
    let mut app = Baboon::for_test();
    let first = app.model.kits[0].id;
    let second = app.add_kit();
    app.commands.send(CompareCommand::GitReview {
        kit: second,
        action: GitReviewAction::Refresh,
    });
    app.apply_commands(&egui::Context::default());
    // Neither kit has a folder source, so a refresh says so on that kit only.
    assert!(app.views[second].git_review.error.is_some());
    assert!(app.views[first].git_review.error.is_none());
}

/// A kit closed between the click and the frame applying it has no view; the
/// request is dropped rather than panicking on it.
#[test]
fn a_git_review_command_for_a_closed_kit_does_nothing() {
    let mut app = Baboon::for_test();
    let closed = app.add_kit();
    app.commands.send(CompareCommand::GitReview {
        kit: closed,
        action: GitReviewAction::Refresh,
    });
    app.remove_kit(closed);
    app.apply_commands(&egui::Context::default());
    assert!(app.model.kit_index(closed).is_none());
}
