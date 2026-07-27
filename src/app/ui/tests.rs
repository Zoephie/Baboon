//! Unit tests for top-level UI helpers.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;
use std::collections::{HashMap, HashSet};

#[test]
fn editing_kit_menu_uses_each_shortcut_once_in_reverse_engine_order() {
    let games: Vec<&str> = editing_kit_menu_shortcuts()
        .map(|shortcut| shortcut.game)
        .collect();

    assert_eq!(
        games,
        vec![
            "haloce_evolved",
            "halo2amp_mcc",
            "halo4_mcc",
            "haloreach_mcc",
            "halo3odst_mcc",
            "halo3_mcc",
            "halo2_mcc",
            "haloce_mcc",
        ]
    );
    assert_eq!(
        games
            .iter()
            .map(|game| game_display_name(game))
            .collect::<Vec<_>>(),
        vec![
            "Halo: Campaign Evolved",
            "Halo 2 Anniversary Multiplayer",
            "Halo 4",
            "Halo: Reach",
            "Halo 3: ODST",
            "Halo 3",
            "Halo 2",
            "Halo: Combat Evolved",
        ]
    );
    assert_eq!(
        games.iter().copied().collect::<HashSet<_>>().len(),
        games.len()
    );
    assert_eq!(games.len(), EDITING_KIT_SHORTCUTS.len());
}

#[test]
fn editing_kit_menu_filters_invalid_built_ins_without_reordering_valid_ones() {
    let root = std::env::temp_dir().join(format!(
        "baboon-visible-kits-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let h2 = root.join("h2");
    let h4 = root.join("h4");
    let invalid = root.join("h3");
    std::fs::create_dir_all(h2.join("tags")).unwrap();
    std::fs::create_dir_all(h4.join("tags")).unwrap();
    std::fs::create_dir_all(&invalid).unwrap();
    let paths = HashMap::from([
        ("halo2_mcc".to_owned(), h2),
        ("halo4_mcc".to_owned(), h4),
        ("halo3_mcc".to_owned(), invalid),
    ]);

    let validation = EditingKitValidationCache::new(&paths, &[]);
    let games = visible_builtin_editing_kit_shortcuts(&validation)
        .into_iter()
        .map(|shortcut| shortcut.game)
        .collect::<Vec<_>>();
    assert_eq!(games, vec!["halo4_mcc", "halo2_mcc"]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn shared_menu_entries_put_custom_profiles_first_in_creation_order() {
    let root = std::env::temp_dir().join(format!(
        "baboon-menu-order-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let h2 = root.join("h2");
    std::fs::create_dir_all(h2.join("tags")).unwrap();
    let profiles = vec![
        CustomEditingKitProfile {
            id: "one".to_owned(),
            name: "First".to_owned(),
            game: "halo3_mcc".to_owned(),
            root: root.join("temporarily-missing-one"),
            icon: None,
        },
        CustomEditingKitProfile {
            id: "two".to_owned(),
            name: "Second".to_owned(),
            game: "haloreach_mcc".to_owned(),
            root: root.join("temporarily-missing-two"),
            icon: None,
        },
    ];
    let paths = HashMap::from([("halo2_mcc".to_owned(), h2)]);
    let validation = EditingKitValidationCache::new(&paths, &profiles);
    let entries = visible_editing_kit_menu_entries(&profiles, &validation);

    assert!(matches!(
        &entries[0],
        EditingKitMenuEntry::Custom(profile) if profile.id == "one"
    ));
    assert!(matches!(
        &entries[1],
        EditingKitMenuEntry::Custom(profile) if profile.id == "two"
    ));
    assert!(matches!(
        entries[2],
        EditingKitMenuEntry::BuiltIn(shortcut) if shortcut.game == "halo2_mcc"
    ));
    assert_eq!(entries.len(), 3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn editing_kit_menu_games_have_distinct_embedded_primary_icons() {
    let mut icons: HashSet<&'static [u8]> = HashSet::new();

    for shortcut in editing_kit_menu_shortcuts() {
        let bytes = get_game_banner_bytes(shortcut.game);
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .unwrap_or_else(|error| panic!("{} icon is not a valid PNG: {error}", shortcut.game));
        assert_eq!(
            image.width(),
            image.height(),
            "{} icon is not square",
            shortcut.game
        );
        assert!(
            image.width() >= 200,
            "{} icon is too small for DPI-aware downsampling",
            shortcut.game
        );
        assert!(
            icons.insert(bytes),
            "{} reuses another editing kit's primary icon",
            shortcut.game
        );
    }

    assert_eq!(icons.len(), EDITING_KIT_SHORTCUTS.len());
}

#[test]
fn editing_kit_menu_rows_keep_icons_aligned_and_separators_outside_click_targets() {
    for scale in [MIN_UI_SCALE, MAX_UI_SCALE] {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(scale);
        let mut first_row = egui::Rect::NOTHING;
        let mut separator = egui::Rect::NOTHING;
        let mut second_row = egui::Rect::NOTHING;

        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(360.0, 160.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_min_width(EDITING_KIT_MENU_MIN_WIDTH);
                    first_row =
                        editing_kit_menu_row(ui, "Halo 4", "H4", None, false, true).rect;
                    separator = ui.separator().rect;
                    second_row = editing_kit_menu_row(
                        ui,
                        "Halo 2 Anniversary Multiplayer",
                        "H2A",
                        None,
                        false,
                        true,
                    )
                    .rect;
                });
            },
        );

        let first_layout = editing_kit_menu_row_layout(first_row);
        let second_layout = editing_kit_menu_row_layout(second_row);
        assert_eq!(first_layout.icon_rect.min.x, second_layout.icon_rect.min.x);
        assert_eq!(first_layout.icon_rect.max.x, second_layout.icon_rect.max.x);
        assert_eq!(
            first_layout.icon_rect.size(),
            Vec2::splat(EDITING_KIT_MENU_ICON_SIZE)
        );
        assert!(first_row.contains(first_layout.icon_rect.min));
        assert!(first_row.contains(first_layout.icon_rect.max));
        assert!(
            first_layout.label_rect.right() + EDITING_KIT_MENU_ICON_GAP
                <= first_layout.icon_rect.left()
        );
        assert!(first_row.max.y <= separator.min.y);
        assert!(separator.max.y <= second_row.min.y);

        let pixels_per_point = ctx.pixels_per_point();
        assert!(
            (first_layout.icon_rect.width() * pixels_per_point
                - EDITING_KIT_MENU_ICON_SIZE * pixels_per_point)
                .abs()
                < f32::EPSILON
        );
    }
}

#[test]
fn terminal_line_visuals_are_distinct_by_severity() {
    let normal = terminal_line_color(TerminalLineSeverity::Normal);
    assert_eq!(terminal_line_color(TerminalLineSeverity::Summary), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Warning), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Error), normal);
    assert_ne!(terminal_line_color(TerminalLineSeverity::Success), normal);
    assert!(terminal_line_is_strong(TerminalLineSeverity::Summary));
    assert!(terminal_line_is_strong(TerminalLineSeverity::Error));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Warning));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Success));
    assert!(!terminal_line_is_strong(TerminalLineSeverity::Normal));
}

#[test]
fn monitor_commands_are_game_specific() {
    assert_eq!(
        monitor_commands_for_game(Some("halo2_mcc")),
        &[
            "monitor-bitmaps",
            "monitor-bitmaps-data-and-tags",
            "monitor-models",
            "monitor-structures",
        ]
    );
    assert_eq!(
        monitor_commands_for_game(Some("halo4_mcc")),
        &["monitor-bitmaps", "monitor-strings"]
    );
    assert!(monitor_commands_for_game(Some("haloce_mcc")).is_empty());
    assert!(monitor_commands_for_game(None).is_empty());
}
