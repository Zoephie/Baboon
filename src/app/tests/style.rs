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

#[test]
fn dark_active_tab_is_lighter_than_every_tab_rack_background() {
    let active = active_tab_for(true);
    assert_eq!(active, Color32::from_rgb(72, 72, 72));
    assert!(active.r() > 55, "document/Chimp inactive gray");
    assert!(active.r() > 31, "workspace inactive gray");
}

#[test]
fn light_active_tab_keeps_the_existing_menu_bar_color() {
    assert_eq!(active_tab_for(false), Color32::from_rgb(161, 161, 157));
}
