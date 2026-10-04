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
