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

use super::controller::{
    ContainerWriteMode, ContainerWriteOutcome, container_triplet as triplet,
    remove_container_triplet,
};

mod browser_ui;
mod document_ui;
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
