use super::*;

#[test]
fn no_preferences_starts_first_run() {
    assert!(!first_run_complete_from_text(None));
}

#[test]
fn existing_preferences_without_marker_are_complete() {
    assert!(first_run_complete_from_text(Some("{}")));
}

#[test]
fn explicit_false_resumes_and_true_finishes_setup() {
    assert!(!first_run_complete_from_text(Some(
        r#"{"first_run_complete":false}"#
    )));
    assert!(first_run_complete_from_text(Some(
        r#"{"first_run_complete":true}"#
    )));
}

#[test]
fn malformed_existing_preferences_still_skip_first_run() {
    assert!(first_run_complete_from_text(Some("not json")));
}
