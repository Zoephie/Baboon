//! Deserialization and state for Baboon's embedded help documentation.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;
use serde::Deserialize;

const HELP_DOCS_FILE: &str = "help.json";

#[derive(Clone)]
pub(super) enum HelpDocsState {
    Loaded(HelpDocs),
    Failed(String),
}

impl HelpDocsState {
    pub(super) fn load() -> Self {
        match load_help_docs() {
            Ok(docs) => HelpDocsState::Loaded(docs),
            Err(error) => HelpDocsState::Failed(error),
        }
    }
}

#[derive(Clone, Deserialize)]
pub(super) struct HelpDocs {
    /// External document schema revision reserved for compatibility checks and
    /// migrations when the help JSON shape changes.
    pub(super) version: u32,
    pub(super) tabs: Vec<HelpDocTab>,
}

#[derive(Clone, Deserialize)]
pub(super) struct HelpDocTab {
    pub(super) id: String,
    /// Data-owned tab label retained for future multi-tab help navigation; the
    /// current single Doc tab is still selected by its stable `id`.
    pub(super) title: String,
    pub(super) sections: Vec<HelpDocSection>,
}

#[derive(Clone, Deserialize)]
pub(super) struct HelpDocSection {
    pub(super) title: String,
    pub(super) blocks: Vec<HelpDocBlock>,
}

/// A hand-editable documentation block. The JSON uses an internal `"kind"`
/// tag, e.g. `{ "kind": "paragraph", "text": "..." }`, so future docs edits
/// do not need Rust-specific enum knowledge.
#[derive(Clone, Deserialize)]
#[serde(tag = "kind")]
pub(super) enum HelpDocBlock {
    #[serde(rename = "paragraph")]
    Paragraph { text: String },
    #[serde(rename = "bullets")]
    Bullets { items: Vec<String> },
}

impl HelpDocs {
    pub(super) fn tab(&self, id: &str) -> Option<&HelpDocTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }
}

fn load_help_docs() -> Result<HelpDocs, String> {
    let path = locate_help_docs_root().join(HELP_DOCS_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    serde_json::from_str::<HelpDocs>(&text)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))
}
