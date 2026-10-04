//! Bringing tags in: Import Tags across games, single-tag import with its
//! profile check, monolithic cache import, the conversion job, and Blam! asset
//! import.

use super::*;
use crate::app::kits::{KitId, KitStamp, ToolCommandArgKind, pick_tool_command_path};
use crate::app::help::HelpCommand;
use crate::app::editor::lost_focus_once;

pub(in crate::app) mod tags;
pub(in crate::app) use tags::*;
pub(in crate::app) mod cache;
pub(in crate::app) use cache::*;
pub(in crate::app) mod conversion;
pub(in crate::app) use conversion::*;
pub(in crate::app) mod blam;
pub(in crate::app) use blam::*;
pub(in crate::app) mod blam_workflow;
pub(in crate::app) mod blam_pane;
pub(in crate::app) use blam_pane::draw_blam_pane;
pub(in crate::app) mod import_tag_dialog;
pub(in crate::app) mod tags_window;
pub(in crate::app) mod cache_window;
pub(in crate::app) mod single_tag;

#[cfg(test)]
mod campaign_import_gate_tests;
pub(in crate::app) mod reports;
pub(in crate::app) use reports::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Import: the template cache conversions share. Its windows — Import Tags,
/// cache import, single-tag import and its discard prompt — are dialogs in
/// the host.
pub(in crate::app) struct ImportFeature {
    /// The last import's index of native layout templates, kept for the next
    /// one. Building it walks the destination kit's whole tag tree — about a
    /// second for a real kit — and the result depends only on which kit it is,
    /// so paying that once per session beats paying it once per tag.
    pub(in crate::app) native_template_cache: Option<NativeTemplateCache>,
}

/// What the import windows commit to. Each acts on the state its window
/// holds.
pub(in crate::app) enum ImportCommand {
    /// Import Tags: work out what the chosen source is.
    ResolveSource,
    /// Import Tags: pick the source file or folder.
    BrowseSourceFile,
    BrowseSourceFolder,
    /// Import Tags: run the import.
    Import,
    /// Import Tags: write the tag whose data loss the user has accepted.
    AcceptLosses,
    /// Import Tags: write the tags a folder run held back, same bargain.
    AcceptHeldBack,
    /// Import into containers: preview the conversion of the chosen tag.
    AnalyzeTag,
    /// Import into containers: bring the tag in.
    ConfirmTag,
    /// Import into containers: discard the unsaved edits it would replace.
    Discard,
    /// Import from a cache: run, over every tag or only `only`.
    StartCacheImport { only: Option<HashSet<String>> },
    /// Import from a cache: find which tags the kit already has.
    ScanCacheConflicts,
    /// Run the Blam! import the kit's pane is set up for.
    BlamImport { kit: KitId },
}

impl Baboon {
    pub(in crate::app) fn apply_import_command(&mut self, command: ImportCommand, ctx: &egui::Context) {
        match command {
            ImportCommand::ResolveSource => self.resolve_import_source(&ctx),
            ImportCommand::BrowseSourceFile => self.choose_import_source_file(&ctx),
            ImportCommand::BrowseSourceFolder => self.choose_import_source_folder(&ctx),
            ImportCommand::Import => self.begin_tag_import(),
            ImportCommand::AcceptLosses => self.accept_import_losses(&ctx),
            ImportCommand::AcceptHeldBack => self.accept_held_back_imports(),
            ImportCommand::AnalyzeTag => self.analyze_import_conversion(),
            ImportCommand::ConfirmTag => self.confirm_import_tag(),
            ImportCommand::Discard => self.apply_import_discard(),
            ImportCommand::StartCacheImport { only } => self.start_cache_import(ctx.clone(), only),
            ImportCommand::ScanCacheConflicts => self.scan_cache_import_conflicts(ctx.clone()),
            ImportCommand::BlamImport { kit } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.begin_blam_import(index, ctx.clone());
                }
            }
        }
    }
}
