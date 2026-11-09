//! Halo: Campaign Evolved's Unreal models in the preview: a `.model`'s meshes
//! found through its mesh synchronization assets, MetaHuman heads, and the
//! Chimp JMS export of what the preview shows. ("ce" in these names is
//! Campaign Evolved, not the classic Halo CE in `ce/`.)

use crate::core::source::MountedContainer;

mod export;
pub(in crate::app) use export::*;
mod index;
pub(in crate::app) use index::*;
mod meshes;
mod metahuman;
mod packages;
pub(in crate::app::model_preview) mod rules;
#[cfg(test)]
mod tests;
