//! Comparing tags: Git Review, Tag Compare, and the tag diff they and mod
//! review share.

use super::*;
use crate::app::shell::{WorkerMessage, spawn_worker};
use crate::app::kits::{EditingKitValidationCache, Kit, KitId};
use crate::core::document::value::{append_field_path, extension_to_group_tag};
use crate::app::browser::native_display_path;

pub(in crate::app) mod git_review;
pub(in crate::app) use git_review::*;
pub(in crate::app) mod git_review_window;
pub(in crate::app) use git_review_window::{GitReviewAction, draw_git_review};
pub(in crate::app) mod tag_compare;
pub(in crate::app) mod diff;
pub(in crate::app) use diff::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;


/// What tag comparison can be asked to do.
pub(in crate::app) enum CompareCommand {
    /// Carry out what a kit's Git Review pane asked for.
    GitReview { kit: KitId, action: GitReviewAction },
    /// Open Git Review over `kit`.
    OpenGitReview { kit: KitId },
}

impl Baboon {
    pub(in crate::app) fn apply_compare_command(&mut self, command: CompareCommand, ctx: &egui::Context) {
        match command {
            CompareCommand::GitReview { kit, action } => self.apply_git_review_action(kit, action, ctx),
            CompareCommand::OpenGitReview { kit } => {
                if let Some(index) = self.model.kit_index(kit) {
                    self.model.active = index;
                    self.open_git_review(ctx);
                }
            }
        }
    }
}

#[cfg(test)]
mod command_tests;
