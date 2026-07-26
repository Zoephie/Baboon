//! Data and loaded thumbnail state for Baboon's game-filtered tutorial catalog.
//! Tutorial metadata remains editable beside the other packaged help documents.

use super::*;
use serde::Deserialize;
use std::path::Component;

const TUTORIALS_FILE: &str = "tutorials.json";
const TUTORIALS_SCHEMA_VERSION: u32 = 1;

pub(super) enum TutorialsState {
    Loaded(TutorialCatalog),
    Failed(String),
}

impl TutorialsState {
    pub(super) fn load(ctx: &egui::Context) -> Self {
        let root = locate_help_docs_root();
        match load_tutorial_catalog(&root) {
            Ok(mut catalog) => {
                hydrate_tutorial_thumbnails(ctx, &root, &mut catalog);
                Self::Loaded(catalog)
            }
            Err(error) => Self::Failed(error),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct TutorialCatalog {
    version: u32,
    pub(super) tutorials: Vec<TutorialEntry>,
}

impl TutorialCatalog {
    pub(super) fn entries_for_game<'a>(
        &'a self,
        game: &'a str,
    ) -> impl Iterator<Item = &'a TutorialEntry> {
        self.tutorials
            .iter()
            .filter(move |tutorial| tutorial.game == game)
    }
}

#[derive(Deserialize)]
pub(super) struct TutorialEntry {
    pub(super) game: String,
    pub(super) title: String,
    pub(super) creator: String,
    pub(super) url: String,
    pub(super) thumbnail: String,
    #[serde(skip)]
    pub(super) thumbnail_texture: Option<egui::TextureHandle>,
    #[serde(skip)]
    pub(super) thumbnail_error: Option<String>,
}

fn load_tutorial_catalog(root: &Path) -> Result<TutorialCatalog, String> {
    let path = root.join(TUTORIALS_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    parse_tutorial_catalog(&text)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))
}

fn parse_tutorial_catalog(text: &str) -> Result<TutorialCatalog, String> {
    let catalog =
        serde_json::from_str::<TutorialCatalog>(text).map_err(|error| error.to_string())?;
    validate_tutorial_catalog(&catalog)?;
    Ok(catalog)
}

fn validate_tutorial_catalog(catalog: &TutorialCatalog) -> Result<(), String> {
    if catalog.version != TUTORIALS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported tutorial catalog version {}; expected {TUTORIALS_SCHEMA_VERSION}",
            catalog.version
        ));
    }

    for (index, tutorial) in catalog.tutorials.iter().enumerate() {
        if !EDITING_KIT_SHORTCUTS
            .iter()
            .any(|shortcut| shortcut.game == tutorial.game)
        {
            return Err(format!(
                "tutorial {index} uses unknown game id {:?}",
                tutorial.game
            ));
        }
        if tutorial.title.trim().is_empty() {
            return Err(format!("tutorial {index} has an empty title"));
        }
        if tutorial.creator.trim().is_empty() {
            return Err(format!("tutorial {index} has an empty creator"));
        }
        if !tutorial.url.starts_with("https://") {
            return Err(format!("tutorial {index} must use an https URL"));
        }
        validate_thumbnail_path(index, &tutorial.thumbnail)?;
    }

    Ok(())
}

fn validate_thumbnail_path(index: usize, thumbnail: &str) -> Result<(), String> {
    let path = Path::new(thumbnail);
    if thumbnail.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "tutorial {index} thumbnail must be a relative path inside docs"
        ));
    }
    Ok(())
}

fn hydrate_tutorial_thumbnails(ctx: &egui::Context, root: &Path, catalog: &mut TutorialCatalog) {
    for (index, tutorial) in catalog.tutorials.iter_mut().enumerate() {
        let path = root.join(&tutorial.thumbnail);
        let texture = std::fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))
            .and_then(|bytes| {
                load_png_texture(
                    ctx,
                    &format!("tutorial_thumbnail_{}_{}", tutorial.game, index),
                    &bytes,
                )
                .ok_or_else(|| format!("Could not decode {} as PNG", path.display()))
            });
        match texture {
            Ok(texture) => tutorial.thumbnail_texture = Some(texture),
            Err(error) => tutorial.thumbnail_error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CATALOG: &str = r#"{
        "version": 1,
        "tutorials": [{
            "game": "haloce_evolved",
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
            .entries_for_game("haloce_evolved")
            .collect::<Vec<_>>();
        assert_eq!(campaign_evolved.len(), 2);
        for shortcut in EDITING_KIT_SHORTCUTS {
            if shortcut.game != "haloce_evolved" {
                assert_eq!(
                    catalog.entries_for_game(shortcut.game).count(),
                    0,
                    "{} should currently have an empty tutorial section",
                    shortcut.game
                );
            }
        }

        assert!(
            campaign_evolved
                .iter()
                .any(|entry| entry.url == "https://www.youtube.com/watch?v=2xL2AiuaFwE")
        );
        assert!(
            campaign_evolved
                .iter()
                .any(|entry| entry.url == "https://www.youtube.com/watch?v=Vc_uxtYe-2U")
        );

        for entry in campaign_evolved {
            let bytes = std::fs::read(root.join(&entry.thumbnail))
                .expect("shipped tutorial thumbnail should exist");
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .expect("shipped tutorial thumbnail should decode as PNG");

            let build_thumbnail = Path::new(env!("OUT_DIR"))
                .join("docs")
                .join(&entry.thumbnail);
            assert!(
                build_thumbnail.is_file(),
                "build script should package the tutorial thumbnail at {}",
                build_thumbnail.display()
            );
        }
    }

    #[test]
    fn tutorial_catalog_rejects_unknown_games_and_invalid_json() {
        let unknown_game = VALID_CATALOG.replace("haloce_evolved", "unknown_game");
        assert!(parse_tutorial_catalog(&unknown_game).is_err());
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
        assert_eq!(entry.url, "https://www.youtube.com/watch?v=example");
    }
}
