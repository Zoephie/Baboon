//! Theme files: reading, inheriting, exporting and reloading them.

use super::export::{export_theme, straight_hex};
use super::library::{RENAMES, ThemeLibrary, rename_problems};
use super::state::ThemeState;
use super::{Appearance, BuiltIn, Token, install, installed};
use eframe::egui::Color32;

fn reports_of(library: &ThemeLibrary, id: &str) -> Vec<String> {
    match library.resolve(id) {
        Ok(resolved) => resolved.reports.iter().map(ToString::to_string).collect(),
        Err(report) => vec![format!("BROKEN {report}")],
    }
}

/// Export then load gives back the same colours: each built-in, and a theme
/// that extends one.
#[test]
fn an_exported_theme_reads_back_as_the_same_colours() {
    let child = r##"{ "name": "Child", "extends": "baboon-dark", "colors": { "accent": "#FF8800", "button.hover": "#FFFFFF33" } }"##;
    let library = ThemeLibrary::from_texts(&[("child", child)]);
    for id in [
        "baboon-dark",
        "baboon-light",
        "baboon-light-classic",
        "child",
    ] {
        let original = library.resolve(id).unwrap().colors;
        let text = export_theme(&library, id).unwrap();
        let reread = ThemeLibrary::from_texts(&[("exported", &text)]);
        let resolved = reread.resolve("exported").unwrap();
        assert_eq!(resolved.colors, original, "{id}");
        assert!(resolved.reports.is_empty(), "{id}: {:?}", resolved.reports);
    }
}

/// The round trip notices a token the export drops.
#[test]
fn the_round_trip_notices_a_dropped_token() {
    let library = ThemeLibrary::default();
    let text = export_theme(&library, "baboon-light").unwrap();
    let dropped: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("\"accent\":"))
        .map(|line| format!("{line}\n"))
        .collect();
    // A theme of its own: no extends, so the dropped token is filled from the
    // built-in of its appearance and reported.
    let reread = ThemeLibrary::from_texts(&[(
        "exported",
        &dropped.replace("\"extends\": \"baboon-light\",\n", ""),
    )]);
    let resolved = reread.resolve("exported").unwrap();
    assert!(
        resolved
            .reports
            .iter()
            .any(|report| report.message.contains("leaves 1 token(s) unset")
                && report.message.contains("accent")),
        "{:?}",
        resolved.reports
    );
}

/// Exported hex reproduces the stored premultiplied colour exactly, even
/// where the plain conversion would round to another value.
#[test]
fn exported_hex_reads_back_exactly() {
    for a in [1u8, 26, 51, 64, 77, 128, 200, 254] {
        for c in 0..=a {
            let color = Color32::from_rgba_premultiplied(c, c / 2, 0, a);
            assert_eq!(
                super::file::parse_color(&straight_hex(color)),
                Some(color),
                "{color:?}"
            );
        }
    }
}

/// Malformed files never panic and each gives exactly its report.
#[test]
fn malformed_theme_files_report_exactly_what_is_wrong() {
    let library = ThemeLibrary::from_texts(&[
        (
            "truncated",
            r##"{ "name": "T", "colors": { "text": "#FFF" "##,
        ),
        ("not_object", "[1, 2]"),
        (
            "wrong_types",
            r##"{ "name": 3, "appearance": "dim", "extends": 7, "palette": [], "colors": { "text": 5 } }"##,
        ),
        (
            "bad_hex",
            r##"{ "name": "B", "extends": "baboon-dark", "colors": { "text": "#GG0000", "accent": "red" } }"##,
        ),
        (
            "unknown",
            r##"{ "name": "U", "extends": "baboon-dark", "flavour": "x", "colors": { "no.such.token": "#000" } }"##,
        ),
        ("self", r##"{ "name": "S", "extends": "self" }"##),
        ("ping", r##"{ "name": "Ping", "extends": "pong" }"##),
        ("pong", r##"{ "name": "Pong", "extends": "ping" }"##),
        (
            "missing_ref",
            r##"{ "name": "M", "extends": "baboon-dark", "colors": { "text": "$nope" } }"##,
        ),
        (
            "orphan",
            r##"{ "name": "O", "appearance": "dark", "extends": "gone" }"##,
        ),
        ("d0", r##"{ "name": "D0", "extends": "d1" }"##),
        ("d1", r##"{ "name": "D1", "extends": "d2" }"##),
        ("d2", r##"{ "name": "D2", "extends": "d3" }"##),
        ("d3", r##"{ "name": "D3", "extends": "d4" }"##),
        ("d4", r##"{ "name": "D4", "extends": "d5" }"##),
        ("d5", r##"{ "name": "D5", "extends": "d6" }"##),
        ("d6", r##"{ "name": "D6", "extends": "d7" }"##),
        ("d7", r##"{ "name": "D7", "extends": "d8" }"##),
        ("d8", r##"{ "name": "D8", "extends": "d9" }"##),
        ("d9", r##"{ "name": "D9", "extends": "baboon-dark" }"##),
    ]);
    let broken = |id: &str| {
        reports_of(&library, id)
            .first()
            .is_some_and(|report| report.starts_with("BROKEN"))
    };
    assert!(broken("truncated"));
    assert!(reports_of(&library, "truncated")[0].contains("is not valid JSON"));
    assert_eq!(
        reports_of(&library, "not_object"),
        ["BROKEN not_object.json: is not a JSON object"]
    );
    let wrong = reports_of(&library, "wrong_types");
    for expected in [
        "wrong_types.json: name: is not a name; the file name is used",
        "wrong_types.json: appearance: is not \"dark\" or \"light\"; ignored",
        "wrong_types.json: extends: is not a theme id; ignored",
        "wrong_types.json: palette: is not an object; ignored",
        "wrong_types.json: text: is not a colour; ignored",
    ] {
        assert!(
            wrong.iter().any(|report| report == expected),
            "missing {expected:?} in {wrong:?}"
        );
    }
    // In the order the table declares the tokens.
    assert_eq!(
        reports_of(&library, "bad_hex"),
        [
            "bad_hex.json: text: #GG0000 is not a colour; ignored",
            "bad_hex.json: accent: red is not a colour; ignored",
        ]
    );
    assert_eq!(
        reports_of(&library, "unknown"),
        [
            "unknown.json: flavour: is not a theme key; ignored",
            "unknown.json: no.such.token: is not a token id; ignored",
        ]
    );
    assert_eq!(
        reports_of(&library, "self"),
        ["BROKEN self.json: extends: goes round in a circle through self"]
    );
    assert_eq!(
        reports_of(&library, "ping"),
        ["BROKEN ping.json: extends: goes round in a circle through ping"]
    );
    assert_eq!(
        reports_of(&library, "missing_ref"),
        ["missing_ref.json: text: $nope is not in the palette; ignored"]
    );
    let orphan = reports_of(&library, "orphan");
    assert_eq!(
        orphan[0],
        "orphan.json: extends: names no theme (gone); the built-in of its appearance is used"
    );
    assert_eq!(
        reports_of(&library, "d0"),
        ["BROKEN d0.json: extends: is more than 8 links deep"]
    );
    assert!(!broken("d1"), "eight links are allowed");
}

/// A child that only overrides palette entries recolours every token its
/// parent painted with them: references are looked up after the merge.
#[test]
fn a_palette_only_child_recolours_its_parent() {
    let parent = r##"{ "name": "P", "extends": "baboon-dark", "palette": { "brand": "#112233" }, "colors": { "accent": "$brand", "progress.bar": "$brand" } }"##;
    let child = r##"{ "name": "C", "extends": "parent", "palette": { "brand": "#FF8800" } }"##;
    let library = ThemeLibrary::from_texts(&[("parent", parent), ("child", child)]);
    let colors = library.resolve("child").unwrap().colors;
    assert_eq!(
        colors.get(Token::Accent),
        Color32::from_rgb(0xFF, 0x88, 0x00)
    );
    assert_eq!(
        colors.get(Token::ProgressBar),
        Color32::from_rgb(0xFF, 0x88, 0x00)
    );
    let parent_colors = library.resolve("parent").unwrap().colors;
    assert_eq!(
        parent_colors.get(Token::Accent),
        Color32::from_rgb(0x11, 0x22, 0x33)
    );
}

/// A theme with no extends takes its appearance from its file, or a guess from
/// its background, and its unset tokens from that appearance's built-in.
#[test]
fn a_theme_of_its_own_fills_from_the_built_in_of_its_appearance() {
    let library = ThemeLibrary::from_texts(&[
        (
            "pale",
            r##"{ "name": "Pale", "colors": { "background": "#F0F0F0" } }"##,
        ),
        (
            "dim",
            r##"{ "name": "Dim", "appearance": "dark", "colors": { "background": "#101010" } }"##,
        ),
    ]);
    let pale = library.resolve("pale").unwrap();
    assert_eq!(pale.colors.appearance, Appearance::Light);
    assert_eq!(
        pale.colors.get(Token::Text),
        BuiltIn::Light.colors().get(Token::Text)
    );
    assert!(
        pale.reports
            .iter()
            .any(|report| report.message.contains("light is guessed"))
    );
    let dim = library.resolve("dim").unwrap();
    assert_eq!(
        dim.colors.get(Token::Text),
        BuiltIn::Dark.colors().get(Token::Text)
    );
}

/// A reload that fails keeps every colour drawn; one that works installs.
#[test]
fn a_failed_reload_leaves_the_installed_colours_alone() {
    let _lock = super::tests::INSTALL_LOCK.lock().unwrap();
    let before = installed();
    let good = r##"{ "name": "G", "extends": "baboon-dark", "colors": { "accent": "#FF8800" } }"##;
    let mut state = ThemeState::new(None);
    let now = std::time::Instant::now();
    state.reload_with(ThemeLibrary::from_texts(&[("good", good)]), now);
    state.apply("good", BuiltIn::Dark, now);
    let applied = installed();
    assert_eq!(
        applied.get(Token::Accent),
        Color32::from_rgb(0xFF, 0x88, 0x00)
    );
    state.reload_with(ThemeLibrary::from_texts(&[("good", "{ not json")]), now);
    assert_eq!(
        installed(),
        applied,
        "a failed reload changed what is drawn"
    );
    assert!(
        state
            .take_status()
            .unwrap()
            .contains("could not be reloaded")
    );
    assert!(
        state
            .problems()
            .iter()
            .any(|report| report.message.contains("is not valid JSON"))
    );
    state.reload_with(
        ThemeLibrary::from_texts(&[("good", &good.replace("FF8800", "00FF00"))]),
        now,
    );
    assert_eq!(
        installed().get(Token::Accent),
        Color32::from_rgb(0, 0xFF, 0)
    );
    install(&before);
}

/// A chosen theme that cannot be applied shows its slot's built-in and says so.
#[test]
fn a_theme_that_cannot_be_applied_shows_the_slots_built_in() {
    let _lock = super::tests::INSTALL_LOCK.lock().unwrap();
    let before = installed();
    let mut state = ThemeState::new(None);
    let now = std::time::Instant::now();
    state.apply("missing", BuiltIn::Dark, now);
    assert_eq!(installed(), BuiltIn::Dark.colors());
    assert!(
        state
            .take_status()
            .unwrap()
            .contains("could not be applied")
    );
    install(&before);
}

/// A theme using a renamed id keeps its colour; the table itself points only
/// at live tokens.
#[test]
fn renamed_ids_are_read_as_their_new_ones() {
    assert_eq!(rename_problems(RENAMES), Vec::<String>::new());
    let renames = [("tree.folder.open", "tree.folder.open.icon")];
    let library = ThemeLibrary::from_texts(&[(
        "old",
        r##"{ "name": "Old", "extends": "baboon-dark", "colors": { "tree.folder.open": "#00FF00" } }"##,
    )]);
    let resolved = library.resolve_with_renames("old", &renames).unwrap();
    assert_eq!(
        resolved.colors.get(Token::TreeFolderOpenIcon),
        Color32::from_rgb(0, 0xFF, 0)
    );
    assert!(
        resolved
            .reports
            .iter()
            .any(|report| report.message.contains("was renamed tree.folder.open.icon"))
    );
    assert_eq!(
        rename_problems(&[("old.id", "no.such.token")]),
        ["old.id is renamed no.such.token, which no token has"],
        "a rename to a missing token is caught"
    );
}

/// The folder's own problems: a built-in's id and a duplicate id are skipped.
#[test]
fn reserved_and_duplicate_ids_are_skipped() {
    let folder = std::env::temp_dir().join(format!("baboon-themes-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("baboon-dark.json"), r#"{ "name": "Mine" }"#).unwrap();
    std::fs::write(
        folder.join("Forerunner.json"),
        r#"{ "name": "A", "extends": "baboon-dark" }"#,
    )
    .unwrap();
    std::fs::write(
        folder.join("forerunner.JSON"),
        r#"{ "name": "B", "extends": "baboon-dark" }"#,
    )
    .unwrap();
    let library = ThemeLibrary::scan(&folder);
    let reports: Vec<String> = library.reports.iter().map(ToString::to_string).collect();
    let _ = std::fs::remove_dir_all(&folder);
    assert!(
        reports
            .iter()
            .any(|report| report.contains("baboon-dark is a built-in theme's id")),
        "{reports:?}"
    );
    let duplicates = reports
        .iter()
        .filter(|report| report.contains("already has the id forerunner"))
        .count();
    // A case-sensitive file system holds both files; a case-insensitive one
    // holds one, and reports nothing.
    assert!(duplicates <= 1, "{reports:?}");
    assert!(library.listed("forerunner").is_some());
}

/// `text` with the part between the token markers replaced by `table`.
fn with_token_table(text: &str, table: &str) -> Option<String> {
    let begin = "<!-- tokens:begin -->";
    let end = "<!-- tokens:end -->";
    let start = text.find(begin)? + begin.len();
    let stop = text.find(end)?;
    Some(format!("{}\n{}{}", &text[..start], table, &text[stop..]))
}

/// `docs/themes.md` lists every token as the table declares it. Set
/// `BABOON_WRITE_THEME_DOCS` to write the generated list into it.
#[test]
fn the_authoring_guide_lists_every_token() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("themes.md");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    let table = super::export::token_reference_markdown();
    let expected = with_token_table(&text, &table).expect("docs/themes.md has the token markers");
    if std::env::var_os("BABOON_WRITE_THEME_DOCS").is_some() {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    assert!(
        text == expected,
        "docs/themes.md's token list is out of date: run this test with BABOON_WRITE_THEME_DOCS=1"
    );
    // The comparison notices a token missing from the guide.
    let missing = with_token_table(
        &text,
        &table.replacen("| `accent` |", "| `accent_gone` |", 1),
    )
    .unwrap();
    assert_ne!(missing, expected);
}

/// The example themes in `docs/theme-examples` read with no problems: two
/// that extend a built-in, and Nightfall, written from scratch with every
/// token. Set `BABOON_WRITE_THEME_DOCS` to write Nightfall from its roots.
#[test]
fn the_example_themes_read_cleanly() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("theme-examples");
    if std::env::var_os("BABOON_WRITE_THEME_DOCS").is_some() {
        // A dark theme from five colours, the rest following them by rule
        // where a rule fits and the dark built-in's elsewhere.
        let roots = r##"{ "name": "Nightfall", "extends": "baboon-dark", "colors": {
            "background": "#1B1D2A", "text": "#E4E6F0", "accent": "#4B4F8A",
            "status.warning": "#E8C068", "status.success": "#86D19A" } }"##;
        let colors = ThemeLibrary::from_texts(&[("roots", roots)])
            .resolve("roots")
            .unwrap()
            .colors;
        std::fs::write(
            folder.join("nightfall.json"),
            super::export::complete_theme_text("Nightfall", &colors),
        )
        .unwrap();
        std::fs::write(
            folder.join("baboon-theme.schema.json"),
            super::export::schema(),
        )
        .unwrap();
    }
    let library = ThemeLibrary::scan(&folder);
    assert!(library.reports.is_empty(), "{:?}", library.reports);
    for id in ["ember", "harbor", "nightfall"] {
        let resolved = library
            .resolve(id)
            .unwrap_or_else(|report| panic!("{report}"));
        assert!(resolved.reports.is_empty(), "{id}: {:?}", resolved.reports);
    }
    let nightfall = library.resolve("nightfall").unwrap();
    assert!(
        nightfall.root_built_in.is_none(),
        "Nightfall extends nothing"
    );
    assert_eq!(
        nightfall.explicit.len(),
        Token::ALL.len(),
        "Nightfall sets every token"
    );
    let ember = library.resolve("ember").unwrap();
    assert!(
        !ember.following.is_empty(),
        "Ember's accent carries its dependents"
    );
}

/// A previewed theme is drawn until the preview ends, then the applied
/// theme's colours come back; a theme that cannot be resolved is not drawn.
#[test]
fn a_preview_gives_back_the_applied_colours() {
    let _lock = super::tests::INSTALL_LOCK.lock().unwrap();
    let before = installed();
    let mut state = ThemeState::new(None);
    let now = std::time::Instant::now();
    state.reload_with(ThemeLibrary::from_texts(&[("broken", "{ nope")]), now);
    state.apply("baboon-dark", BuiltIn::Dark, now);
    state.preview(Some("baboon-light"));
    assert_eq!(installed(), BuiltIn::Light.colors());
    state.preview(Some("broken"));
    assert_eq!(
        installed(),
        BuiltIn::Light.colors(),
        "a broken theme draws nothing"
    );
    state.preview(None);
    assert_eq!(installed(), BuiltIn::Dark.colors());
    // Ending no preview installs nothing: the Theme Editor's draft stays.
    install(&BuiltIn::ClassicLight.colors());
    state.preview(None);
    assert_eq!(installed(), BuiltIn::ClassicLight.colors());
    install(&before);
}
