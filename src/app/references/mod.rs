//! References: the reverse-dependency index, the content explorer, reference
//! jumps, and fixing a tag's dependencies.

use super::*;
use crate::app::search::QueryResultsWindow;
use crate::core::document::value::append_field_path_for;
use crate::app::editor::{
    OpenTagRequest, clean_field_name, field_jump_target_id, format_reference_path,
    is_inherited_parent_name, jump_target_id, parent_block_path, sanitize_ref_path,
};
use crate::app::browser::{
    ContentExplorer, FieldNav, PendingRefJump, RefOccurrence, TagQueryResults,
    contains_ignore_ascii_case,
};

pub(in crate::app) mod index;
pub(in crate::app) use index::*;
pub(in crate::app) mod dependencies;
pub(in crate::app) use dependencies::*;
pub(in crate::app) mod ref_jump;
pub(in crate::app) mod explorer;
pub(in crate::app) use explorer::ExplorerAct;

/// References: a reference jump waiting for its referrer to load, field
/// navigation, and a referenced tag waiting to open. The Content Explorer and
/// the query results are dialogs in the host.
pub(in crate::app) struct ReferencesFeature {
    /// A reference-jump awaiting its referrer tag to finish loading before we
    /// can walk it to locate the exact referencing field. Set from the
    /// "References to X" popup; drained by `apply_field_nav`.
    pub(in crate::app) pending_ref_jump: Option<PendingRefJump>,
    /// Active reference-jump navigation: force ancestor blocks open and glow the
    /// exact referencing field until its glow window expires.
    pub(in crate::app) field_nav: Option<FieldNav>,
    /// Pending "open referenced tag in a new tab" request.
    pub(in crate::app) pending_open: Option<OpenTagRequest>,
}

/// What references can be asked to do.
pub(in crate::app) enum ReferencesCommand {
    /// Carry out what the Content Explorer over `kit` asked for.
    Explorer { kit: KitId, act: ExplorerAct },
    /// Open the tag a field refers to, once the frame's drawing is over.
    Open(OpenTagRequest),
}

impl Baboon {
    pub(in crate::app) fn apply_references_command(&mut self, command: ReferencesCommand, ctx: &egui::Context) {
        match command {
            ReferencesCommand::Explorer { kit, act } => self.apply_explorer_act(kit, act, ctx),
            ReferencesCommand::Open(request) => self.references.pending_open = Some(request),
        }
    }
}
