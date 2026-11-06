//! Halo 4: its material shaders and what each draw submits, object lighting
//! and lights, flares, camo, shield, shadow, particles and tracers. Its passes
//! are the renderer's.

pub(in crate::app::model_preview) mod bloom;
pub(super) mod camo;
pub(in crate::app::model_preview) mod composite;
pub(in crate::app::model_preview) mod effect_gl;
pub(in crate::app::model_preview) mod flare_pass;
pub(super) mod flares;
pub(in crate::app::model_preview) mod holograms;
pub(super) mod lighting;
pub(in crate::app::model_preview) mod light_pass;
pub(super) mod lights;
pub(in crate::app::model_preview) mod light_volume_pass;
pub(in crate::app::model_preview) mod particle_pass;
pub(super) mod particles;
pub(in crate::app::model_preview) mod render;
pub(crate) mod shaders;
pub(super) mod shadow;
pub(in crate::app::model_preview) mod shadow_pass;
pub(super) mod shield;
pub(super) mod submit;
pub(in crate::app::model_preview) mod tracer_pass;
pub(super) mod tracers;
