//! Halo 4: its material shaders and what each draw submits, object lighting
//! and lights, flares, camo, shield, shadow, particles and tracers. Its passes
//! are the renderer's.

pub(super) mod camo;
pub(super) mod flares;
pub(super) mod lighting;
pub(super) mod lights;
pub(super) mod particles;
pub(crate) mod shaders;
pub(super) mod shadow;
pub(super) mod shield;
pub(super) mod submit;
pub(super) mod tracers;
