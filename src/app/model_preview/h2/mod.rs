//! Halo 2: an object's functions and what it holds, and the bloom, lights,
//! light volumes, contrails, liquid and PRT the preview draws for it. Its
//! shader passes, which Halo CE's share, are in `dx9`.

pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod attached;
pub(super) mod bloom;
pub(super) mod contrails;
pub(super) mod light_volumes;
pub(super) mod liquid;
pub(super) mod lights;
pub(in crate::app) mod prt;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;
