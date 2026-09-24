use super::*;

fn stored(prefs: &GuiPrefs) -> GuiPrefs {
    prefs_from_value(&prefs_to_value(prefs, &HashSet::new(), true))
}

#[test]
fn old_preferences_keep_the_bitmap_view_defaults() {
    let prefs = prefs_from_value(&json!({}));
    assert_eq!(
        prefs.bitmap_preview_view,
        BitmapPreviewViewSettings::default()
    );
}

#[test]
fn bitmap_view_choices_survive_a_save_and_load() {
    let mut prefs = GuiPrefs::default();
    prefs.bitmap_preview_view = BitmapPreviewViewSettings {
        bg: BitmapPreviewBg::Magenta,
        show_checkerboard: false,
        show_border: false,
    };

    assert_eq!(
        stored(&prefs).bitmap_preview_view,
        prefs.bitmap_preview_view
    );
}

#[test]
fn an_unknown_bitmap_background_uses_the_default() {
    let prefs = prefs_from_value(&json!({
        "bitmap_preview_background": "chartreuse",
        "bitmap_preview_checkerboard": false,
        "bitmap_preview_border": false,
    }));

    assert_eq!(prefs.bitmap_preview_view.bg, BitmapPreviewBg::DarkGray);
    assert!(!prefs.bitmap_preview_view.show_checkerboard);
    assert!(!prefs.bitmap_preview_view.show_border);
}
