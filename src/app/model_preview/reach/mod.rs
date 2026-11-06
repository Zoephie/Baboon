//! Reach: object lights, camo, shadow and shield, several of which Halo 4
//! draws through too. Its lighting, bloom and passes are the renderer's.

pub(super) mod camo;
pub(in crate::app::model_preview) mod bloom;
pub(in crate::app::model_preview) mod composite;
pub(in crate::app::model_preview) mod lighting;
pub(in crate::app::model_preview) mod light_pass;
pub(super) mod lights;
pub(in crate::app::model_preview) mod render;
pub(super) mod shadow;
pub(in crate::app::model_preview) mod shadow_pass;
pub(super) mod shield;
