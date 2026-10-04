//! Help: the in-app documentation window, tutorials, HaloScript docs, field
//! docs, tag compatibility and map names.

use super::*;

pub(in crate::app) mod docs;
pub(in crate::app) use docs::*;
pub(in crate::app) mod script_docs;
pub(in crate::app) use script_docs::*;
pub(in crate::app) mod field_docs;
pub(in crate::app) use field_docs::*;
pub(in crate::app) mod tutorials;
pub(in crate::app) use tutorials::*;
pub(in crate::app) mod tag_compat;
pub(in crate::app) use tag_compat::*;
pub(in crate::app) mod map_names;
pub(in crate::app) use map_names::*;
pub(in crate::app) mod window;

/// Help: the About and help windows, tutorials, HaloScript and field docs, tag
/// compatibility and map names.
pub(in crate::app) struct HelpFeature {
    pub(in crate::app) about_open: bool,
    pub(in crate::app) help_panel_tab: HelpPanelTab,
    pub(in crate::app) help_docs: HelpDocsState,
    pub(in crate::app) tutorials: TutorialsState,
    pub(in crate::app) tutorials_game: String,
    pub(in crate::app) tutorials_category: TutorialCategory,
    pub(in crate::app) script_docs: ScriptDocsUiState,
    pub(in crate::app) tag_compat: TagCompatUiState,
    pub(in crate::app) map_names_game_tab: MapNamesGameTab,
    /// Parsed-once documentation overlay (help/units + explanations) per group
    /// JSON, keyed by definition file path. Built lazily during render.
    pub(in crate::app) def_docs_cache: HashMap<PathBuf, Rc<DefDocs>>,
}
