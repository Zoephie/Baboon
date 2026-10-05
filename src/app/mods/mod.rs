//! Campaign Evolved mods: the .baboon project and its overlays, the container
//! write lease, overwriting a container tag in place, Export Mod and its
//! review, and their windows.

use super::*;
use crate::app::shell::session::save_last_session;
use crate::app::shell::{WorkerMessage, spawn_worker};
use crate::app::kits::{KitId, KitStamp, tag_tree_id};
use crate::app::compare::{TagFieldDiff, describe_tag, diff_tags};
use crate::app::chimp::ChimpMount;
use crate::app::editor::{
    EditSinks, FieldEditContext, FieldFilter, FieldFilterAction, draw_foundation_group,
    draw_struct_fields_inline, strip_node_indices,
};
use crate::app::export::ensure_export_directory;
use crate::app::browser::{
    ModifiedTags, added_text, added_wash, modified_text, removed_text, removed_wash,
};

pub(in crate::app) mod project;
pub(in crate::app) use project::*;
pub(in crate::app) mod container_write;
pub(in crate::app) use container_write::*;
pub(in crate::app) mod mod_export_window;
pub(in crate::app) mod exported_mod_window;
pub(in crate::app) mod overwrite_confirm;
pub(in crate::app) mod clear_stash_confirm;
pub(in crate::app) mod in_place;
pub(in crate::app) mod export;
pub(in crate::app) use export::*;
pub(in crate::app) mod review;
pub(in crate::app) use review::*;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Campaign Evolved mods: container write leases and the remounts they leave,
/// and Export Mod with its review. Their prompts are dialogs in the host.
pub(in crate::app) struct ModsFeature {
    /// Container writes currently in flight, by lease id. A lease outlives the
    /// UI-thread call that took it exactly when the write runs on a worker, and
    /// the terminal `WorkerMessage` carries the id back so the completion
    /// handler can find it. Keyed rather than stacked: two workspaces may be
    /// writing to two different containers at the same time.
    pub(in crate::app) container_write_leases: HashMap<ContainerLeaseId, ContainerWriteLease>,
    pub(in crate::app) next_container_lease: u64,
    /// Workspaces whose Unreal package mount a finished container write idled
    /// and must start again. Queued rather than remounted in place because the
    /// release runs from paths that have no `egui::Context` to spawn with.
    pub(in crate::app) pending_chimp_remounts: Vec<KitId>,
    /// What the last mod exported in this session was called, so exporting
    /// again offers the same name and replaces that mod's files rather than
    /// making the user retype it. Deliberately not persisted: it describes what
    /// this session has been working on, not a preference.
    pub(in crate::app) last_mod_export_name: Option<String>,
}

/// What mods can be asked to do. Each names the workspace it was raised
/// from: they all write through that kit's source, so the handler returns to
/// it first and drops the request if it has closed — overwriting the game's
/// paks in place is the last thing that should land on whichever game is
/// focused by now.
pub(in crate::app) enum ModsCommand {
    /// Overwrite the tag at `key` in the game's containers. With
    /// `stop_asking`, also stop confirming overwrites — applied only here,
    /// when the user commits to one.
    Overwrite {
        kit: KitId,
        key: String,
        stop_asking: bool,
    },
    /// Export a mod instead of overwriting.
    ExportInstead { kit: KitId },
    /// Throw away the workspace's stashed project changes.
    ClearStash { kit: KitId },
    /// Write the reviewed changes as a mod at `output`.
    WriteReviewedMod {
        kit: KitId,
        /// The name the mod was given, offered again by the next export.
        name: String,
        snapshot: CampaignProjectSnapshot,
        included: HashSet<String>,
        output: PathBuf,
    },
    /// Ask for a folder and save the open review's diagnostic into it.
    SaveReviewDiagnostic,
}

impl Baboon {
    pub(in crate::app) fn apply_mods_command(&mut self, command: ModsCommand, ctx: &egui::Context) {
        match command {
            ModsCommand::Overwrite {
                kit,
                key,
                stop_asking,
            } => {
                if stop_asking && self.model.prefs.confirm_container_overwrite {
                    self.model.prefs.confirm_container_overwrite = false;
                    self.persist_prefs_if_changed();
                }
                if self.focus_navigation_kit(kit) {
                    self.begin_overwrite_current_tag_in_place(&key, &ctx);
                }
            }
            ModsCommand::ExportInstead { kit } => {
                if self.focus_navigation_kit(kit) {
                    self.export_mod();
                }
            }
            ModsCommand::ClearStash { kit } => {
                // Resolved rather than assumed: the workspace may have been
                // closed while the confirmation was up.
                if let Some(index) = self.model.resolve_kit(kit) {
                    self.clear_campaign_stash(index, &ctx);
                }
            }
            ModsCommand::WriteReviewedMod {
                kit,
                name,
                snapshot,
                included,
                output,
            } => {
                // Kept for the next export in this session, so replacing a
                // mod's files does not mean typing its name again.
                self.mods.last_mod_export_name = Some(name);
                if self.focus_navigation_kit(kit) {
                    self.write_reviewed_mod(&snapshot, &included, output, &ctx);
                }
            }
            ModsCommand::SaveReviewDiagnostic => self.save_review_diagnostic_to_picked_folder(),
        }
    }
}

#[cfg(test)]
mod command_tests;
