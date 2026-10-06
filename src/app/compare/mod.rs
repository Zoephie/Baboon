//! Comparing tags: Git Review, Tag Compare, and the tag diff they and mod
//! review share.

use super::*;
use crate::app::shell::{WorkerMessage, spawn_worker};
use crate::app::kits::{EditingKitValidationCache, Kit, KitId};
use crate::core::document::value::append_field_path;
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
mod tests {
    use super::*;

    /// A Git Review request names its kit by id, so it lands on that kit's
    /// review whatever has happened to the kit list since the click.
    #[test]
    fn a_git_review_command_reaches_the_kit_it_names() {
        let mut app = Baboon::for_test();
        let first = app.model.kits[0].id;
        let second = app.add_kit();
        app.commands.send(CompareCommand::GitReview {
            kit: second,
            action: GitReviewAction::Refresh,
        });
        app.apply_commands(&egui::Context::default());
        // Neither kit has a folder source, so a refresh says so on that kit only.
        assert!(app.views[second].git_review.error.is_some());
        assert!(app.views[first].git_review.error.is_none());
    }

    /// A kit closed between the click and the frame applying it has no view; the
    /// request is dropped rather than panicking on it.
    #[test]
    fn a_git_review_command_for_a_closed_kit_does_nothing() {
        let mut app = Baboon::for_test();
        let closed = app.add_kit();
        app.commands.send(CompareCommand::GitReview {
            kit: closed,
            action: GitReviewAction::Refresh,
        });
        app.remove_kit(closed);
        app.apply_commands(&egui::Context::default());
        assert!(app.model.kit_index(closed).is_none());
    }
}
