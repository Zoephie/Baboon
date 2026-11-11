//! Help: the in-app documentation window, tutorials, HaloScript docs, field
//! docs, tag compatibility and map names.

use super::*;
use crate::app::editor::field_display_meta;

pub(in crate::app) mod docs;
pub(in crate::app) use docs::*;
mod markdown;
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
pub(in crate::app) use window::HelpWindow;

pub(in crate::app) const FOLDER_BROWSER_HELP_SECTION: &str = "Folder Browser";
pub(in crate::app) const STRING_EDITOR_HELP_SECTION: &str = "Strings Editor";
pub(in crate::app) const FIND_REPLACE_HELP_SECTION: &str = "Find and Replace";

/// Help: the About and help windows, tutorials, HaloScript and field docs, tag
/// compatibility and map names.
pub(in crate::app) struct HelpFeature {
    /// The documentation and tutorials, read once per session and shared
    /// with every Help window opened over them.
    pub(in crate::app) docs: Rc<HelpDocsState>,
    pub(in crate::app) tutorials: Rc<TutorialsState>,
    /// Parsed-once documentation overlay (help/units + explanations) per group
    /// JSON, keyed by definition file path. Built lazily during render.
    pub(in crate::app) def_docs_cache: HashMap<PathBuf, Rc<DefDocs>>,
}

/// What Help can be asked to do.
pub(in crate::app) enum HelpCommand {
    /// Open the Help window on this tab.
    Open(HelpPanelTab),
    /// Open the documentation directly on one dedicated section.
    ShowDocSection(&'static str),
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
                let help = self.help_window();
                help.tab = tab;
            }
            HelpCommand::ShowDocSection(section) => {
                let help = self.help_window();
                help.tab = HelpPanelTab::Doc;
                help.doc_section = Some(section.to_owned());
                help.doc_reveal_selection = true;
            }
            HelpCommand::ShowTagCompat {
                source_game,
                target_game,
                group,
            } => {
                let help = self.help_window();
                help.tag_compat.ensure_loaded(&locate_help_docs_root());
                help.tag_compat.focus(&source_game, &target_game, &group);
                help.tab = HelpPanelTab::TagCompat;
            }
            HelpCommand::ExportTagCompatSheet => self.export_tag_compat_sheet(),
        }
    }

    /// The open Help window, opening it on About if it is not.
    fn help_window(&mut self) -> &mut HelpWindow {
        // Failed loads are recoverable after documentation files are repaired.
        // Retry on an explicit Help request rather than caching the error for
        // the entire session or retrying file I/O on every rendered frame.
        if matches!(self.help.docs.as_ref(), HelpDocsState::Failed(_)) {
            self.help.docs = Rc::new(HelpDocsState::load());
            if let Some(window) = self.dialogs.get_mut::<HelpWindow>() {
                window.docs = self.help.docs.clone();
            }
        }
        if self.dialogs.get::<HelpWindow>().is_none() {
            self.dialogs
                .open(HelpWindow::new(&self.help, HelpPanelTab::About));
        }
        self.dialogs.get_mut::<HelpWindow>().expect("just opened")
    }
}
