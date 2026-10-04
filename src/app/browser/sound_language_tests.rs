use super::*;

#[test]
fn extraction_names_the_games_default_language() {
    assert_eq!(sound_language_label(Some("halo3_mcc"), None), "English");
    assert_eq!(
        sound_language_label(Some("halo4_mcc"), None),
        "English (US)"
    );
    assert_eq!(
        sound_language_label(Some("halo3_mcc"), Some("portuguese")),
        "Portuguese"
    );
}
