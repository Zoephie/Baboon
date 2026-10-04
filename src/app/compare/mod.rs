//! Comparing tags: Git Review, Tag Compare, and the tag diff they and mod
//! review share.

use super::*;

pub(in crate::app) mod git_review;
pub(in crate::app) use git_review::*;
pub(in crate::app) mod git_review_window;
pub(in crate::app) use git_review_window::{GitReviewAction, draw_git_review};
pub(in crate::app) mod tag_compare;
pub(in crate::app) use tag_compare::draw_tag_diff_window;
pub(in crate::app) mod diff;
pub(in crate::app) use diff::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Tag comparison: the open Tag Compare.
pub(in crate::app) struct CompareFeature {
    /// "Compare Tags" (Tag Diff) window state.
    pub(in crate::app) tag_diff: Option<TagDiffState>,
}

/// What tag comparison can be asked to do.
pub(in crate::app) enum CompareCommand {
    /// Carry out what a kit's Git Review pane asked for.
    GitReview { kit: KitId, action: GitReviewAction },
}

impl Baboon {
    pub(in crate::app) fn apply_compare_command(&mut self, command: CompareCommand, ctx: &egui::Context) {
        match command {
            CompareCommand::GitReview { kit, action } => self.apply_git_review_action(kit, action, ctx),
        }
    }
}

#[cfg(test)]
mod command_tests;
