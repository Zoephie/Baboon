//! Bringing tags in: Import Tags across games, single-tag import with its
//! profile check, monolithic cache import, the conversion job, and Blam! asset
//! import.

use super::*;

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
pub(in crate::app) use import_tag_dialog::{draw_import_discard_confirm, draw_import_tag_window};
pub(in crate::app) mod tags_window;
pub(in crate::app) use tags_window::draw_tag_import_window;
pub(in crate::app) mod cache_window;
pub(in crate::app) use cache_window::draw_cache_import_window;
pub(in crate::app) mod single_tag;

#[cfg(test)]
mod campaign_import_gate_tests;
pub(in crate::app) mod reports;
pub(in crate::app) use reports::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Import: the Import Tags and cache import windows, single-tag import and its
/// discard prompt, and the template cache conversions share.
pub(in crate::app) struct ImportFeature {
    /// Import Tags: pull loose tags from another game's kit into the active
    /// one. One dialog for both a single tag and a whole folder — which of the
    /// two it is follows from the path, not from a mode the user has to pick.
    pub(in crate::app) tag_import_dialog: Option<TagImportDialog>,
    /// Import Cache Folder: convert a folder of a monolithic cache's big-endian
    /// tags into an open editing kit, at their own paths, following references.
    /// Separate from `tag_import_dialog` because almost nothing is shared: there
    /// is no path to resolve, no game to guess, and no destination to choose.
    pub(in crate::app) cache_import_dialog: Option<CacheImportDialog>,
    /// The last import's index of native layout templates, kept for the next
    /// one. Building it walks the destination kit's whole tag tree — about a
    /// second for a real kit — and the result depends only on which kit it is,
    /// so paying that once per session beats paying it once per tag.
    pub(in crate::app) native_template_cache: Option<NativeTemplateCache>,
    /// Import-a-tag-file dialog (Campaign Evolved), when open.
    pub(in crate::app) import_tag_dialog: Option<ImportTagDialog>,
    /// Pending "discard unsaved edits and replace with the imported tag?" prompt.
    pub(in crate::app) import_discard_confirm: Option<PendingImport>,
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
    pub(in crate::app) fn apply_import_command(&mut self, command: ImportCommand) {
        let ctx = self.egui_ctx.clone();
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
            ImportCommand::StartCacheImport { only } => self.start_cache_import(ctx, only),
            ImportCommand::ScanCacheConflicts => self.scan_cache_import_conflicts(ctx),
            ImportCommand::BlamImport { kit } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.begin_blam_import(index, ctx);
                }
            }
        }
    }
}
