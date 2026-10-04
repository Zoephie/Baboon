use super::*;

#[test]
fn fresh_wizard_requires_storage_first() {
    let state = FirstRunWizardState::new(None);
    assert_eq!(state.page, FirstRunPage::Storage);
    assert_eq!(state.selected_storage, None);
    assert!(!state.editing_kit_detection_ran);
}

#[test]
fn interrupted_wizard_resumes_after_storage_selection() {
    let state = FirstRunWizardState::new(Some(crate::storage::StorageMode::Portable));
    assert_eq!(state.page, FirstRunPage::Interface);
    assert_eq!(
        state.committed_storage,
        Some(crate::storage::StorageMode::Portable)
    );
}
