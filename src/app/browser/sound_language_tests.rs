use super::*;

#[test]
fn extraction_names_the_games_default_language() {
    assert_eq!(sound_language_label(Some(GameId::Halo3), None), "English");
    assert_eq!(
        sound_language_label(Some(GameId::Halo4), None),
        "English (US)"
    );
    assert_eq!(
        sound_language_label(Some(GameId::Halo3), Some("portuguese")),
        "Portuguese"
    );
}
