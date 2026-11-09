//! Halo: Campaign Evolved's Unreal models in the preview: a `.model`'s meshes
//! found through its mesh synchronization assets, MetaHuman heads, and the
//! Chimp JMS export of what the preview shows. ("ce" in these names is
//! Campaign Evolved, not the classic Halo CE in `ce/`.)

use super::*;
use crate::core::source::MountedContainer;
use blam_tags::iostore::IoStoreArchive;
use blam_tags::iostore::container_header::EIoContainerHeaderVersion;
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::ue_types::{EIoStoreTocVersion, FPackageObjectIndex};
use blam_tags::iostore::unversioned::{
    MeshRef, MeshSyncRegions, Permutation, PropValue, PropertyBlock, Region,
};
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::zen::FZenPackageHeader;
use blam_tags::jms::{UeMeshPart, UeStaticPart, UeWorldPart};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex};

mod export;
pub(in crate::app) use export::*;
mod index;
pub(in crate::app) use index::*;
mod metahuman;
use metahuman::*;
mod meshes;
use meshes::*;
mod packages;
use packages::*;
#[cfg(test)]
mod tests;
