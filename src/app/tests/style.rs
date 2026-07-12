//! Unit tests for shared visual-style helpers.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[test]
fn material_text_for_bg_chooses_contrasting_foreground() {
    assert_eq!(
        material_text_for_bg(Color32::from_rgb(42, 43, 41)),
        Color32::from_gray(232)
    );
    assert_eq!(
        material_text_for_bg(Color32::from_rgb(232, 191, 171)),
        Color32::from_gray(20)
    );
}
