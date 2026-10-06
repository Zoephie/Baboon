//! Searching tags: Find, the field-value index and search, and the source
//! listings (map ids, sounds by class).

use super::*;
use crate::app::shell::{FieldValueMatch, WorkerMessage, spawn_worker};
use crate::app::kits::{KitId, KitStamp};
use crate::app::help::{DefDocs, DefEntry, build_def_docs};
use crate::app::editor::{
    clean_field_name, find_full_field_name, format_foundation_scalar_value, foundation_block_title,
    is_inherited_parent_name, is_previewable_geometry_group_for_game, jump_target_id,
    lost_focus_once, supports_field_search,
};
use crate::app::browser::{PendingRefJump, RefOccurrence, TagQueryResults};

pub(in crate::app) mod find;
pub(in crate::app) use find::*;
pub(in crate::app) mod find_window;
pub(in crate::app) use find_window::{
    FindWindow, draw_icon_window_header, draw_icon_window_header_without_close,
};
pub(in crate::app) mod find_state;
pub(in crate::app) use find_state::*;
pub(in crate::app) mod field_index;
pub(in crate::app) use field_index::*;
pub(in crate::app) mod field_search;
pub(in crate::app) use field_search::*;
pub(in crate::app) mod result_windows;
pub(in crate::app) use result_windows::{FieldValueSearchWindow, QueryResultAction, QueryResultsWindow};
pub(in crate::app) mod listings;


/// Search: the Find dialog, tag query results, the field-value search and a
/// Find hit waiting to be opened.
pub(in crate::app) struct SearchFeature {
    /// Modeless find-in-tag dialog and its exact occurrence list.
    pub(in crate::app) find: FindDialogState,
    pub(in crate::app) field_value_searching: bool,
    /// Find result waiting for its target open tab to finish parsing.
    pub(in crate::app) pending_find_jump: Option<FindOccurrence>,
}

/// What search can be asked to do.
pub(in crate::app) enum SearchCommand {
    /// Find's query or options changed to these: search again from the first
    /// match.
    FindChanged(FindQuery),
    /// Show only what Find matches, or everything.
    FindFilter(bool),
    /// The Find window closed: stop finding.
    FindClose,
    /// Move Find to the match `delta` away from the current one.
    FindStep(isize),
    /// Carry out what a row of the query results window asked for.
    QueryResult {
        kit: KitId,
        action: QueryResultAction,
    },
    /// Search field values for `query`, in tags of `group` if it names one.
    RunFieldValueSearch { query: String, group: String },
    /// Build the active kit's field-value index.
    BuildFieldIndex,
    /// Open Find on the tag at `key` in `kit`, which becomes the active and
    /// selected tag.
    FindInTag { kit: KitId, key: String },
}

impl Baboon {
    /// Open Find, or select its query again if it is open.
    pub(in crate::app) fn open_find(&mut self) {
        self.search.find.open = true;
        self.dialogs.open(FindWindow { focus_query: true });
    }

    pub(in crate::app) fn apply_search_command(&mut self, command: SearchCommand, ctx: &egui::Context) {
        match command {
            SearchCommand::FindChanged(query) => {
                self.search.find.set_query(query);
                self.search.find.active = None;
                self.search.find.results_key = None;
                self.refresh_find(&ctx);
                if let Some(hit) = self.search.find.active_occurrence().cloned() {
                    self.activate_find_occurrence(&ctx, hit);
                }
            }
            SearchCommand::FindFilter(filter) => self.search.find.filter_results = filter,
            SearchCommand::FindClose => self.search.find.close(),
            SearchCommand::FindStep(delta) => self.step_find(&ctx, delta),
            SearchCommand::QueryResult { kit, action } => self.apply_query_result_action(kit, action, ctx),
            SearchCommand::RunFieldValueSearch { query, group } => {
                self.begin_field_value_search(&query, &group, ctx.clone())
            }
            SearchCommand::BuildFieldIndex => self.begin_build_field_index(ctx.clone()),
            SearchCommand::FindInTag { kit, key } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.focus_kit(index);
                    self.model.kits[index].selected_key = Some(key);
                    self.search.find.within = FindWithin::CurrentTag;
                    self.open_find();
                }
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A row of a query run against a workspace that has since closed is inert:
    /// the key could name an unrelated tag in whatever kit is active now.
    #[test]
    fn a_query_result_from_a_closed_workspace_opens_nothing() {
        let mut app = Baboon::for_test();
        let closed = app.add_kit();
        app.remove_kit(closed);
        app.commands.send(SearchCommand::QueryResult {
            kit: closed,
            action: QueryResultAction::Open {
                key: "file:objects/a.weapon".to_owned(),
                ref_target: Some((u32::from_be_bytes(*b"weap"), "objects/b".to_owned())),
            },
        });
        app.apply_commands(&egui::Context::default());
        assert_eq!(app.model.status, "That workspace has been closed");
        assert!(app.references.pending_ref_jump.is_none());
    }

    /// Opening a row of a "References to X" query queues a jump to the field that
    /// points at X, to land once the referrer has loaded.
    #[test]
    fn opening_a_references_row_queues_the_jump_to_its_field() {
        let mut app = Baboon::for_test();
        let kit = app.model.kits[0].id;
        app.commands.send(SearchCommand::QueryResult {
            kit,
            action: QueryResultAction::Open {
                key: "file:objects/a.weapon".to_owned(),
                ref_target: Some((u32::from_be_bytes(*b"weap"), "objects/b".to_owned())),
            },
        });
        app.apply_commands(&egui::Context::default());
        let jump = app.references.pending_ref_jump.as_ref().expect("a jump is queued");
        assert_eq!(jump.tag_key, "file:objects/a.weapon");
        assert_eq!(jump.rel_path, "objects/b");
    }
}
