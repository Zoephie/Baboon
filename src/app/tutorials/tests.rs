use super::*;

const VALID_CATALOG: &str = r#"{
        "version": 3,
        "tutorials": [{
            "game": "haloce_evolved",
            "category": "3d",
            "kind": "video",
            "title": "Tutorial",
            "creator": "Creator",
            "url": "https://www.youtube.com/watch?v=example",
            "thumbnail": "tutorials/example.png"
        }]
    }"#;

#[test]
fn shipped_tutorial_catalog_and_thumbnail_are_valid() {
    let root = locate_help_docs_root();
    let catalog = load_tutorial_catalog(&root).expect("shipped tutorial catalog should load");
    let campaign_evolved = catalog
        .entries_for("haloce_evolved", TutorialCategory::ThreeD)
        .collect::<Vec<_>>();
    assert_eq!(campaign_evolved.len(), 2);
    for shortcut in EDITING_KIT_SHORTCUTS {
        if shortcut.game != "haloce_evolved" {
            for category in TUTORIAL_CATEGORIES {
                assert_eq!(
                    catalog.entries_for(shortcut.game, category).count(),
                    0,
                    "{} {} should currently have an empty tutorial section",
                    shortcut.game,
                    category.label()
                );
            }
        }
    }
    let sound = catalog
        .entries_for("haloce_evolved", TutorialCategory::Sound)
        .collect::<Vec<_>>();
    assert_eq!(sound.len(), 1);
    assert_eq!(
        catalog
            .entries_for("haloce_evolved", TutorialCategory::Script)
            .count(),
        0
    );

    assert!(campaign_evolved.iter().any(
        |entry| entry.url.as_deref() == Some("https://www.youtube.com/watch?v=2xL2AiuaFwE")
    ));
    assert!(campaign_evolved.iter().any(
        |entry| entry.url.as_deref() == Some("https://www.youtube.com/watch?v=Vc_uxtYe-2U")
    ));

    for entry in campaign_evolved {
        assert_eq!(entry.kind, TutorialKind::Video);
        let thumbnail = entry
            .thumbnail
            .as_deref()
            .expect("video tutorial should name a thumbnail");
        let bytes = std::fs::read(root.join(thumbnail))
            .expect("shipped tutorial thumbnail should exist");
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .expect("shipped tutorial thumbnail should decode as PNG");

        let build_thumbnail = Path::new(env!("OUT_DIR")).join("docs").join(thumbnail);
        assert!(
            build_thumbnail.is_file(),
            "build script should package the tutorial thumbnail at {}",
            build_thumbnail.display()
        );
    }

    let sound_guide = sound[0];
    assert_eq!(sound_guide.kind, TutorialKind::Article);
    assert_eq!(
        sound_guide.title,
        "Campaign Evolved Audio Replacement Guide"
    );
    assert_eq!(
        sound_guide.title_url.as_deref(),
        Some("https://discord.com/channels/615301822474878977/1531551984577155212")
    );
    assert_eq!(sound_guide.creator, "ellaviolet");
    let article_links = sound_guide
        .blocks
        .iter()
        .flat_map(block_spans)
        .filter_map(|span| span.url.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        article_links,
        [
            "https://www.nexusmods.com/site/mods/1812",
            "https://drive.google.com/file/d/1XGGJC45ISYHR0yBbW27BpnkWSFdpqXgn/view?usp=sharing",
            "https://grepwin.com/",
        ]
    );
}

#[test]
fn tutorial_catalog_rejects_unknown_games_and_invalid_json() {
    let unknown_game = VALID_CATALOG.replace("haloce_evolved", "unknown_game");
    let unknown_category = VALID_CATALOG.replace("\"3d\"", "\"unknown\"");
    let insecure_title_link = VALID_CATALOG.replace(
        "\"title\": \"Tutorial\"",
        "\"title\": \"Tutorial\", \"title_url\": \"http://example.com\"",
    );
    assert!(parse_tutorial_catalog(&unknown_game).is_err());
    assert!(parse_tutorial_catalog(&unknown_category).is_err());
    assert!(parse_tutorial_catalog(&insecure_title_link).is_err());
    assert!(parse_tutorial_catalog("{ not json }").is_err());
}

#[test]
fn missing_thumbnail_keeps_tutorial_metadata_available() {
    let mut catalog = parse_tutorial_catalog(VALID_CATALOG).unwrap();
    hydrate_tutorial_thumbnails(
        &egui::Context::default(),
        Path::new("definitely-missing-tutorial-root"),
        &mut catalog,
    );
    let entry = &catalog.tutorials[0];
    assert!(entry.thumbnail_texture.is_none());
    assert!(entry.thumbnail_error.is_some());
    assert_eq!(entry.title, "Tutorial");
    assert_eq!(
        entry.url.as_deref(),
        Some("https://www.youtube.com/watch?v=example")
    );
}

fn block_spans(block: &TutorialBlock) -> Vec<&TutorialSpan> {
    match block {
        TutorialBlock::Heading { .. } => Vec::new(),
        TutorialBlock::Paragraph { spans } => spans.iter().collect(),
        TutorialBlock::NumberedSteps { items } => {
            items.iter().flat_map(|item| item.iter()).collect()
        }
    }
}
