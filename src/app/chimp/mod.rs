//! Chimp: the Campaign Evolved Unreal package workspace.
//!
//! Chimp is deliberately scoped to a loaded Campaign Evolved kit. It shares
//! that kit's Paks root but owns its own package index, documents and editor
//! state; none of those concepts are forced through the editing-kit/tag model.

use super::*;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

mod level;
pub(in crate::app) use level::read_cells;
mod level_blend;
mod level_export;
mod level_segment;
mod mesh_weld;
use level_export::{ExportStage, MeshDetail};
pub(in crate::app) use level_export::{write_blend_export, write_segmented_usd};
use level_segment::SegmentBudget;
use std::io::{Cursor, Write};

use blam_tags::iostore::asset::texture2d::{Texture2dSurfaces, decode_texture2d_surfaces};
use blam_tags::iostore::container::writer::{
    PackageOverride, PackageReplacement, overwrite_packages_in_place_with,
    write_package_mod_container,
};
use blam_tags::iostore::object::archive::ExportContext;
use blam_tags::iostore::object::edit::{
    count_object_references, default_value_for_type, property_type_for_slot, set_property_slot,
    validate_value_for_type,
};
use blam_tags::iostore::object::export::{Export, ExportBlock, read_export_in, write_export_in};
use blam_tags::iostore::object::hand_written as chimp_hw;
use blam_tags::iostore::object::native::{NativeStruct, PerPlatformValue};
use blam_tags::iostore::object::tail_models::{TailContext, parse_texture_chain_tail};
use blam_tags::iostore::object::usmap::PropertyType;
use blam_tags::iostore::object::value::{FName, PropValue, PropertyBlock};
use blam_tags::iostore::package::builder::{read_payloads, write_package};
use blam_tags::iostore::package::imports::{
    ImportSlot, ImportTarget, import_package_index, import_slot_of, public_export_hash,
    read_import_slots, write_import_slots,
};
use blam_tags::iostore::package::name_map::FMappedName;
use blam_tags::iostore::package::ue_types::{FPackageObjectIndex, FPackageObjectIndexType};
use blam_tags::iostore::package::zen::{EExportFilterFlags, EZenPackageVersion, FZenPackageHeader};
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::world::{CE_HEADER_VERSION, CE_TOC_VERSION, PackageProvider, World};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::mods::container_write::{
    ContainerWriteMode, ContainerWriteOutcome, container_triplet as triplet,
    remove_container_triplet,
};

mod browser_ui;
mod document_ui;
mod edit;
mod extract;
mod header_model;
mod header_ui;
mod package;
mod property_editor;
mod save;
mod session;
mod state;
#[cfg(test)]
mod test_support;

use document_ui::*;
pub(in crate::app) use edit::*;
pub(in crate::app) use extract::*;
use header_model::*;
use header_ui::*;
pub(in crate::app) use package::*;
use property_editor::*;
pub(in crate::app) use save::*;
use session::*;
pub(in crate::app) use state::*;
#[cfg(test)]
use test_support::*;

impl Baboon {
    /// Whether the active kit is showing its Chimp surface rather than tags.
    ///
    /// Undo and redo act on the selected package there, not on the selected
    /// tag, which is hidden: changing it would change something the user
    /// cannot see.
    pub(in crate::app) fn chimp_surface_is_active(&self) -> bool {
        self.model.prefs.enable_chimp && self.views[self.model.kits[self.model.active].id].surface == KitSurface::Chimp
    }
}
pub(in crate::app) mod prompts_window;
pub(in crate::app) use prompts_window::*;

/// What Chimp's panes and windows ask of the application.
pub(in crate::app) enum ChimpCommand {
    /// `package`'s pane was drawn, and `edit` is what it changed. A frame
    /// without an edit closes the run that coalesces into one undo step, as a
    /// drag across a value is one step and not one per frame.
    PaneDrawn {
        kit: KitId,
        package: String,
        edit: Option<ChimpEdit>,
    },
    /// Export a mesh, with the textures `with_textures` asks for.
    ExportMesh {
        prompt: ChimpMeshTexturePrompt,
        with_textures: ChimpTextureScope,
    },
    ExportTexture(ChimpTextureExportPrompt),
    ExportLevel(ChimpLevelExportPrompt),
    /// The discard prompt's Save: open the save dialog for the close it holds.
    SaveBeforeClose(ChimpDiscardPrompt),
    /// The discard prompt's Discard: restore its packages, then run its close.
    Discard(ChimpDiscardPrompt),
    /// The save dialog's choice for a kit's modified packages.
    Save {
        kit: KitId,
        action: ChimpSaveAction,
        pending_close_action: Option<PendingCloseAction>,
    },
}

impl Baboon {
    pub(in crate::app) fn apply_chimp_command(&mut self, command: ChimpCommand, ctx: &egui::Context) {
        match command {
            ChimpCommand::PaneDrawn { kit, package, edit } => {
                let Some(kit_index) = self.model.kit_index(kit) else {
                    return;
                };
                let now = ctx.input(|input| input.time);
                let Some((world, document, pane)) = self.chimp_document_and_pane(kit_index, &package)
                else {
                    return;
                };
                match edit {
                    Some(edit) => {
                        apply_chimp_edit(&world, document, pane, edit, now);
                    }
                    None => end_chimp_edit_run(document),
                }
            }
            ChimpCommand::ExportMesh {
                prompt,
                with_textures,
            } => self.start_chimp_mesh_export(prompt, with_textures, ctx.clone()),
            ChimpCommand::ExportTexture(prompt) => self.start_chimp_texture_export(prompt, ctx.clone()),
            ChimpCommand::ExportLevel(prompt) => self.start_chimp_level_export(prompt, ctx.clone()),
            ChimpCommand::SaveBeforeClose(prompt) => self.save_chimp_before_close(prompt),
            ChimpCommand::Discard(prompt) => self.discard_chimp_for_prompt(prompt, ctx),
            ChimpCommand::Save {
                kit,
                action,
                pending_close_action,
            } => self.save_chimp_changes(kit, action, pending_close_action, ctx),
        }
    }
}

/// Chimp's app-wide prompts and jobs: mesh texture, texture export and level
/// export prompts, the level job, writes in flight, the discard prompt and the
/// usmap path being typed.
pub(in crate::app) struct ChimpFeature {
    /// A Chimp mesh export waiting on the choice to export its textures too.
    pub(in crate::app) chimp_mesh_texture_prompt: Option<ChimpMeshTexturePrompt>,
    /// A Chimp texture export waiting on the choice of image format.
    pub(in crate::app) chimp_texture_export_prompt: Option<ChimpTextureExportPrompt>,
    pub(in crate::app) chimp_level_export_prompt: Option<ChimpLevelExportPrompt>,
    pub(in crate::app) chimp_level_job: Option<ChimpLevelJob>,
    /// Kits with a Chimp save running, and the close to run once it lands.
    pub(in crate::app) chimp_writes: HashMap<KitId, Option<PendingCloseAction>>,
    /// Pending workspace-wide Chimp discard, optionally continuing a close
    /// transaction after the packages have been restored.
    pub(in crate::app) chimp_discard_prompt: Option<ChimpDiscardPrompt>,
    pub(in crate::app) chimp_usmap_path_input: String,
}
