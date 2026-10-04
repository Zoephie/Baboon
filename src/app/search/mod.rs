//! Searching tags: Find, the field-value index and search, and the source
//! listings (map ids, sounds by class).

use super::*;

pub(in crate::app) mod find;
pub(in crate::app) use find::*;
pub(in crate::app) mod find_window;
pub(in crate::app) use find_window::{draw_find_window, draw_icon_window_header, draw_icon_window_header_without_close};
pub(in crate::app) mod find_state;
pub(in crate::app) use find_state::*;
pub(in crate::app) mod field_index;
pub(in crate::app) use field_index::*;
pub(in crate::app) mod field_search;
pub(in crate::app) use field_search::*;
pub(in crate::app) mod result_windows;
pub(in crate::app) use result_windows::{QueryResultAction, draw_field_value_search_window, draw_query_results_window};
pub(in crate::app) mod listings;

#[cfg(test)]
mod listing_entries_tests;

/// Search: the Find dialog, tag query results, the field-value search and a
/// Find hit waiting to be opened.
pub(in crate::app) struct SearchFeature {
    /// Modeless find-in-tag dialog and its exact occurrence list.
    pub(in crate::app) find: FindDialogState,
    pub(in crate::app) query_results: Option<TagQueryResults>,
    pub(in crate::app) field_value_search_open: bool,
    pub(in crate::app) field_value_query: String,
    pub(in crate::app) field_value_group: String,
    pub(in crate::app) field_value_searching: bool,
    /// Find result waiting for its target open tab to finish parsing.
    pub(in crate::app) pending_find_jump: Option<FindOccurrence>,
}

/// What search can be asked to do.
pub(in crate::app) enum SearchCommand {
    /// Find's query or options changed: search again from the first match.
    FindChanged,
    /// Move Find to the match `delta` away from the current one.
    FindStep(isize),
    /// Carry out what a row of the query results window asked for.
    QueryResult {
        kit: KitId,
        action: QueryResultAction,
    },
    /// Search field values for the query in the Search Field Values window.
    RunFieldValueSearch,
    /// Build the active kit's field-value index.
    BuildFieldIndex,
}

impl Baboon {
    pub(in crate::app) fn apply_search_command(&mut self, command: SearchCommand) {
        let ctx = self.egui_ctx.clone();
        match command {
            SearchCommand::FindChanged => {
                self.search.find.active = None;
                self.search.find.results_key = None;
                self.refresh_find(&ctx);
                if let Some(hit) = self.search.find.active_occurrence().cloned() {
                    self.activate_find_occurrence(&ctx, hit);
                }
            }
            SearchCommand::FindStep(delta) => self.step_find(&ctx, delta),
            SearchCommand::QueryResult { kit, action } => self.apply_query_result_action(kit, action),
            SearchCommand::RunFieldValueSearch => self.begin_field_value_search(ctx),
            SearchCommand::BuildFieldIndex => self.begin_build_field_index(ctx),
        }
    }
}

#[cfg(test)]
mod command_tests;
