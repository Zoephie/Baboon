//! A confirmed colour or function popup edits the kit it was opened from.
//!
//! The shader and material grids used to write the shared popup directly,
//! with no record of their kit, so the stamp left by whichever popup was
//! opened before decided where the edit went: dropped, or applied to another
//! kit's tag with the same key.

use super::*;
use crate::app::editor::{ColorPopupWindow, MaterialColorPopup};

fn two_kits() -> (Baboon, KitId, KitId) {
    let mut app = Baboon::for_test();
    let a = app.model.kits[0].id;
    let b = KitId(a.0 + 1);
    app.push_kit(Kit::empty(b, TagNameIndex::default()));
    (app, a, b)
}

/// A colour popup opened from `kit`, as a tag pane opens one.
fn open_popup(app: &mut Baboon, kit: KitId) {
    app.dialogs.open(ColorPopupWindow {
        popup: Some(MaterialColorPopup::new("color", 1.0, 0.5, 0.25, 1.0)),
        kit,
    });
}

/// The kit the open colour popup applies to.
fn popup_kit(app: &mut Baboon) -> Option<usize> {
    let opened_from = app
        .dialogs
        .get::<ColorPopupWindow>()
        .map(|window| window.kit);
    app.popup_target_kit(opened_from)
}

#[test]
fn a_grid_popup_opened_after_another_kits_popup_edits_its_own_kit() {
    let (mut app, a, b) = two_kits();
    // A normal swatch in B opens the picker; it is stamped with B.
    open_popup(&mut app, b);
    assert_eq!(popup_kit(&mut app), Some(1));

    // Then the shader grid in A opens one, B still active.
    app.model.active = 1;
    open_popup(&mut app, a);
    assert_eq!(
        popup_kit(&mut app),
        Some(0),
        "the edit lands in A, where the popup was opened"
    );
}

#[test]
fn a_popup_from_a_closed_kit_is_dropped_not_redirected() {
    let (mut app, _, b) = two_kits();
    open_popup(&mut app, b);
    app.model.kits.pop();
    assert_eq!(popup_kit(&mut app), None);
    // A popup with no recorded kit still applies to the active one.
    assert_eq!(app.popup_target_kit(None), Some(app.model.active));
}
