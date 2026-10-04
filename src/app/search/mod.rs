//! Searching tags: Find, the field-value index and search, and the source
//! listings (map ids, sounds by class).

use super::*;

pub(in crate::app) mod find;
pub(in crate::app) use find::*;
pub(in crate::app) mod find_window;
pub(in crate::app) use find_window::{draw_icon_window_header, draw_icon_window_header_without_close};
pub(in crate::app) mod find_state;
pub(in crate::app) use find_state::*;
pub(in crate::app) mod field_index;
pub(in crate::app) use field_index::*;
pub(in crate::app) mod field_search;
pub(in crate::app) use field_search::*;
pub(in crate::app) mod result_windows;
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
