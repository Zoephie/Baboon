//! Animation playback for the `.model` preview: listing the linked graph's
//! animations, decoding a selection into per-node frames off the UI thread,
//! and sampling the current time into GPU skinning matrices. Geometry,
//! rendering, and the panel presentation belong elsewhere.

/// Reach's blend screens: the aim, look and other pose overlays laid over an
/// action.
pub(super) mod blend_screens;
/// Inverse kinematics: a held weapon's hand, Reach's chains and Halo 4's
/// IK sets.
pub(super) mod ik;

mod pose;
pub(crate) use pose::*;
mod playback;
pub(crate) use playback::*;
mod skinning;
pub(in crate::app::model_preview) use skinning::*;
mod decode;
mod graph;
#[cfg(test)]
mod tests;
mod worker;
