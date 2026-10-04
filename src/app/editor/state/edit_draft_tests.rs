use super::*;

#[test]
fn changed_draft_is_not_replaced_by_stale_model_value() {
    let mut draft = EditDraft::new("10");
    draft.text = "25".to_owned();
    draft.changed = true;
    draft.synchronize("10");
    assert_eq!(draft.text, "25");
    assert!(draft.changed);
}

#[test]
fn successful_commit_becomes_the_new_clean_baseline() {
    let mut draft = EditDraft::new("10");
    draft.text = "25".to_owned();
    draft.changed = true;
    draft.synchronize("25");
    assert_eq!(draft.text, "25");
    assert!(!draft.changed);
    draft.synchronize("30");
    assert_eq!(draft.text, "30");
}
