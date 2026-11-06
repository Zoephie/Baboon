//! Reach: object lights, camo, shadow and shield, several of which Halo 4
//! draws through too. Its lighting, bloom and passes are the renderer's.

pub(super) mod camo;
pub(super) mod lights;
pub(in crate::app::model_preview) mod render;
pub(super) mod shadow;
pub(super) mod shield;
