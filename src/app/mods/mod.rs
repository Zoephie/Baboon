//! Campaign Evolved mods: the .baboon project and its overlays, the container
//! write lease, overwriting a container tag in place, Export Mod and its
//! review, and their windows.

use super::*;

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

/// Campaign Evolved mods: the overwrite and clear-stash prompts, container
/// write leases and the remounts they leave, and Export Mod with its review.
pub(in crate::app) struct ModsFeature {
    /// Pending in-place overwrite confirmation (the tag key) for a container tag.
    pub(in crate::app) overwrite_confirm: Option<OverwriteConfirm>,
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
    /// Pending confirmation for the Campaign Evolved "clear modifications"
    /// toolbar action, which is irreversible.
    pub(in crate::app) clear_stash_confirm: Option<ClearStashConfirm>,
    /// Shown after Export Mod, explaining what to do with the files.
    pub(in crate::app) exported_mod: Option<ExportedMod>,
    /// Review of a pending Export Mod, before anything is written.
    pub(in crate::app) mod_export: Option<ModExportDialog>,
}
