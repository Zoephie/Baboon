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
pub(in crate::app) use window::draw_help_window;

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

/// What Help can be asked to do.
pub(in crate::app) enum HelpCommand {
    /// Open the Help window on this tab.
    Open(HelpPanelTab),
    /// Open Help on the compatibility of one tag group crossing from one game
    /// to another.
    ShowTagCompat {
        source_game: String,
        target_game: String,
        group: String,
    },
    /// Ask where to write the compatibility rows on show, and write them.
    ExportTagCompatSheet,
}

impl Baboon {
    pub(in crate::app) fn apply_help_command(&mut self, command: HelpCommand) {
        match command {
            HelpCommand::Open(tab) => {
                self.help.help_panel_tab = tab;
                self.help.about_open = true;
            }
            HelpCommand::ShowTagCompat {
                source_game,
                target_game,
                group,
            } => {
                self.help.tag_compat.ensure_loaded(&locate_help_docs_root());
                self.help.tag_compat.focus(&source_game, &target_game, &group);
                self.help.help_panel_tab = HelpPanelTab::TagCompat;
                self.help.about_open = true;
            }
            HelpCommand::ExportTagCompatSheet => self.export_tag_compat_sheet(),
        }
    }
}
