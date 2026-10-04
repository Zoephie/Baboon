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
pub(in crate::app) use browser_ui::draw_chimp_workspace;
use browser_ui::send_chimp_extractions;
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
        self.model.prefs.enable_chimp
            && self.views[self.model.kits[self.model.active].id].surface == KitSurface::Chimp
    }
}
pub(in crate::app) mod prompts_window;

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
    /// Export the mesh the open texture prompt is for, with the textures
    /// `with_textures` asks for.
    ExportMesh { with_textures: ChimpTextureScope },
    /// Export the texture the open format prompt is for.
    ExportTexture,
    /// Export the level the open level prompt is for.
    ExportLevel,
    /// The open discard prompt's Save: open the save dialog for the close it
    /// holds.
    SaveBeforeClose,
    /// The open discard prompt's Discard: restore its packages, then run its
    /// close.
    Discard,
    /// The save dialog's choice for a kit's modified packages.
    Save {
        kit: KitId,
        action: ChimpSaveAction,
        pending_close_action: Option<PendingCloseAction>,
    },
    /// Open the save dialog for a kit's modified packages.
    OpenSaveDialog {
        kit: KitId,
    },
    /// Write something out of an open package.
    Extract {
        kit: KitId,
        package: String,
        what: ChimpExtraction,
    },
    /// Find which mounted packages import `package`.
    ScanReferrers {
        kit: KitId,
        package: String,
    },
    /// `package`'s pane took focus: it is the one undo and the menus act on.
    Focus {
        kit: KitId,
        package: String,
    },
    /// Close open packages; modified ones refuse.
    Close {
        kit: KitId,
        which: ChimpClose,
    },
    /// Bring the kit's open packages in line with the panes its tile tree
    /// holds, which a drag or a close can have changed.
    SyncOpenPackages {
        kit: KitId,
    },
    /// Start (or retry) indexing the kit's Unreal packages.
    Mount {
        kit: KitId,
    },
    /// Open `package`, or focus its pane if it is already open.
    Open {
        kit: KitId,
        package: String,
    },
    /// Write a legacy-pak file to where the user picks.
    ExtractPakFile {
        kit: KitId,
        path: String,
    },
}

/// What [`ChimpCommand::Extract`] writes out of a package.
pub(in crate::app) enum ChimpExtraction {
    Package,
    Json,
    /// The pane's selected export.
    Export,
    Texture,
    Mesh(ChimpMeshFormat),
    Level(ChimpLevelFormat),
}

/// Which packages [`ChimpCommand::Close`] closes. Resolved when it is applied,
/// against the open packages as they are then.
pub(in crate::app) enum ChimpClose {
    These(Vec<String>),
    All,
    AllBut(String),
}

impl Baboon {
    pub(in crate::app) fn apply_chimp_command(
        &mut self,
        command: ChimpCommand,
        ctx: &egui::Context,
    ) {
        match command {
            ChimpCommand::PaneDrawn { kit, package, edit } => {
                let Some(kit_index) = self.model.kit_index(kit) else {
                    return;
                };
                let now = ctx.input(|input| input.time);
                let Some((world, document, pane)) =
                    self.chimp_document_and_pane(kit_index, &package)
                else {
                    return;
                };
                match edit {
                    Some(edit) => {
                        apply_chimp_edit(&world, document, pane, edit, now);
                    }
                    None => end_chimp_edit_run(document),
                }
                refresh_chimp_header_usage(document, pane);
            }
            ChimpCommand::ExportMesh { with_textures } => {
                if let Some(prompt) = self.dialogs.close::<ChimpMeshTexturePrompt>() {
                    self.start_chimp_mesh_export(prompt, with_textures, ctx.clone());
                }
            }
            ChimpCommand::ExportTexture => {
                if let Some(prompt) = self.dialogs.close::<ChimpTextureExportPrompt>() {
                    self.start_chimp_texture_export(prompt, ctx.clone());
                }
            }
            ChimpCommand::ExportLevel => {
                if let Some(prompt) = self.dialogs.close::<ChimpLevelExportPrompt>() {
                    self.start_chimp_level_export(prompt, ctx.clone());
                }
            }
            ChimpCommand::SaveBeforeClose => {
                if let Some(prompt) = self.dialogs.close::<ChimpDiscardPrompt>() {
                    self.save_chimp_before_close(prompt);
                }
            }
            ChimpCommand::Discard => {
                if let Some(prompt) = self.dialogs.close::<ChimpDiscardPrompt>() {
                    self.discard_chimp_for_prompt(prompt, ctx);
                }
            }
            ChimpCommand::Save {
                kit,
                action,
                pending_close_action,
            } => self.save_chimp_changes(kit, action, pending_close_action, ctx),
            ChimpCommand::OpenSaveDialog { kit } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.open_chimp_save_dialog(kit_index);
                }
            }
            ChimpCommand::Extract { kit, package, what } => {
                let Some(kit_index) = self.model.kit_index(kit) else {
                    return;
                };
                match what {
                    ChimpExtraction::Package => self.extract_chimp_package(kit_index, &package),
                    ChimpExtraction::Json => self.extract_chimp_json(kit_index, &package),
                    ChimpExtraction::Export => self.extract_chimp_export(kit_index, &package),
                    ChimpExtraction::Texture => {
                        self.begin_extract_chimp_texture(kit_index, &package)
                    }
                    ChimpExtraction::Mesh(format) => {
                        self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone())
                    }
                    ChimpExtraction::Level(format) => {
                        self.begin_export_chimp_level(kit_index, &package, format)
                    }
                }
            }
            ChimpCommand::ScanReferrers { kit, package } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.begin_chimp_referrer_scan(kit_index, package, ctx.clone());
                }
            }
            ChimpCommand::Focus { kit, package } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.model.kits[kit_index].chimp.selected_package = Some(package);
                }
            }
            ChimpCommand::Close { kit, which } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.close_chimp_packages(kit_index, which);
                }
            }
            ChimpCommand::Mount { kit } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.begin_chimp_mount(kit_index, ctx.clone());
                }
            }
            ChimpCommand::Open { kit, package } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.begin_chimp_open_package(kit_index, package, ctx.clone());
                }
            }
            ChimpCommand::ExtractPakFile { kit, path } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    self.extract_chimp_pak_file(kit_index, &path);
                }
            }
            ChimpCommand::SyncOpenPackages { kit } => {
                if let Some(kit_index) = self.model.kit_index(kit) {
                    let kit = &mut self.model.kits[kit_index];
                    self.views[kit.id].chimp.sync_open_packages(&mut kit.chimp);
                }
            }
        }
    }

    /// Close `which` of a kit's open packages, saying so if a modified one
    /// refused.
    fn close_chimp_packages(&mut self, kit_index: usize, which: ChimpClose) {
        let open = &self.model.kits[kit_index].chimp.open_packages;
        let mut requested = match which {
            ChimpClose::These(packages) => packages,
            ChimpClose::All => open.clone(),
            ChimpClose::AllBut(keep) => open
                .iter()
                .filter(|package| **package != keep)
                .cloned()
                .collect(),
        };
        requested.sort();
        requested.dedup();
        let mut blocked = false;
        for package in requested {
            if !self.close_chimp_package(kit_index, &package) {
                blocked = true;
            }
        }
        if blocked {
            self.model.status =
                "Save or discard modified Chimp packages before closing them.".to_owned();
        }
    }
}

/// Chimp's app-wide jobs: the level job, writes in flight and the usmap path
/// being typed. Its prompts are dialogs in the host.
pub(in crate::app) struct ChimpFeature {
    pub(in crate::app) chimp_level_job: Option<ChimpLevelJob>,
    /// Kits with a Chimp save running, and the close to run once it lands.
    pub(in crate::app) chimp_writes: HashMap<KitId, Option<PendingCloseAction>>,
    pub(in crate::app) chimp_usmap_path_input: String,
}
