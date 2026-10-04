//! Data and loaded thumbnail state for Baboon's game-filtered tutorial catalog.
//! Tutorial metadata remains editable beside the other packaged help documents.

use super::*;
use serde::Deserialize;
use std::path::Component;

const TUTORIALS_FILE: &str = "tutorials.json";
const TUTORIALS_SCHEMA_VERSION: u32 = 3;

pub(in crate::app) const TUTORIAL_CATEGORIES: [TutorialCategory; 3] = [
    TutorialCategory::ThreeD,
    TutorialCategory::Sound,
    TutorialCategory::Script,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub(in crate::app) enum TutorialCategory {
    #[serde(rename = "3d")]
    ThreeD,
    #[serde(rename = "sound")]
    Sound,
    #[serde(rename = "script")]
    Script,
}

impl TutorialCategory {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::ThreeD => "3D",
            Self::Sound => "Sound",
            Self::Script => "Script",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(in crate::app) enum TutorialKind {
    Video,
    Article,
}

pub(in crate::app) enum TutorialsState {
    Loaded(TutorialCatalog),
    Failed(String),
}

impl TutorialsState {
    pub(in crate::app) fn load(ctx: &egui::Context) -> Self {
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
pub(in crate::app) struct TutorialCatalog {
    version: u32,
    pub(in crate::app) tutorials: Vec<TutorialEntry>,
}

impl TutorialCatalog {
    pub(in crate::app) fn entries_for<'a>(
        &'a self,
        game: &'a str,
        category: TutorialCategory,
    ) -> impl Iterator<Item = &'a TutorialEntry> {
        self.tutorials
            .iter()
            .filter(move |tutorial| tutorial.game == game && tutorial.category == category)
    }
}

#[derive(Deserialize)]
pub(in crate::app) struct TutorialEntry {
    pub(in crate::app) game: String,
    pub(in crate::app) category: TutorialCategory,
    pub(in crate::app) kind: TutorialKind,
    pub(in crate::app) title: String,
    pub(in crate::app) title_url: Option<String>,
    pub(in crate::app) creator: String,
    pub(in crate::app) url: Option<String>,
    pub(in crate::app) thumbnail: Option<String>,
    #[serde(default)]
    pub(in crate::app) blocks: Vec<TutorialBlock>,
    #[serde(skip)]
    pub(in crate::app) thumbnail_texture: Option<egui::TextureHandle>,
    #[serde(skip)]
    pub(in crate::app) thumbnail_error: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
pub(in crate::app) enum TutorialBlock {
    #[serde(rename = "heading")]
    Heading { text: String },
    #[serde(rename = "paragraph")]
    Paragraph { spans: Vec<TutorialSpan> },
    #[serde(rename = "numbered_steps")]
    NumberedSteps { items: Vec<Vec<TutorialSpan>> },
}

#[derive(Deserialize)]
pub(in crate::app) struct TutorialSpan {
    pub(in crate::app) text: String,
    pub(in crate::app) url: Option<String>,
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
            .any(|shortcut| shortcut.game.as_str() == tutorial.game)
        {
            return Err(format!(
                "tutorial {index} uses unknown game id {:?}",
                tutorial.game
            ));
        }
        if tutorial.title.trim().is_empty() {
            return Err(format!("tutorial {index} has an empty title"));
        }
        if let Some(title_url) = tutorial.title_url.as_deref()
            && !title_url.starts_with("https://")
        {
            return Err(format!("tutorial {index} title link must use an https URL"));
        }
        if tutorial.creator.trim().is_empty() {
            return Err(format!("tutorial {index} has an empty creator"));
        }
        match tutorial.kind {
            TutorialKind::Video => {
                let url = tutorial
                    .url
                    .as_deref()
                    .ok_or_else(|| format!("video tutorial {index} is missing its URL"))?;
                if !url.starts_with("https://") {
                    return Err(format!("video tutorial {index} must use an https URL"));
                }
                let thumbnail = tutorial
                    .thumbnail
                    .as_deref()
                    .ok_or_else(|| format!("video tutorial {index} is missing its thumbnail"))?;
                validate_thumbnail_path(index, thumbnail)?;
            }
            TutorialKind::Article => {
                if tutorial.blocks.is_empty() {
                    return Err(format!("article tutorial {index} has no content blocks"));
                }
                validate_article_blocks(index, &tutorial.blocks)?;
            }
        }
    }

    Ok(())
}

fn validate_article_blocks(index: usize, blocks: &[TutorialBlock]) -> Result<(), String> {
    for block in blocks {
        match block {
            TutorialBlock::Heading { text } => {
                if text.trim().is_empty() {
                    return Err(format!("article tutorial {index} has an empty heading"));
                }
            }
            TutorialBlock::Paragraph { spans } => validate_spans(index, spans)?,
            TutorialBlock::NumberedSteps { items } => {
                if items.is_empty() {
                    return Err(format!("article tutorial {index} has no numbered steps"));
                }
                for spans in items {
                    validate_spans(index, spans)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_spans(index: usize, spans: &[TutorialSpan]) -> Result<(), String> {
    if spans.is_empty() || spans.iter().all(|span| span.text.trim().is_empty()) {
        return Err(format!("article tutorial {index} has an empty text block"));
    }
    for span in spans {
        if let Some(url) = span.url.as_deref()
            && !url.starts_with("https://")
        {
            return Err(format!(
                "article tutorial {index} contains a link without an https URL"
            ));
        }
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
        let Some(thumbnail) = tutorial.thumbnail.as_deref() else {
            continue;
        };
        let path = root.join(thumbnail);
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
mod tests;
