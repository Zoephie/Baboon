//! Halo 4's effects on the GPU: the shared effect program, particles and their pass, and the tracer and light volume passes.

pub(in crate::app::model_preview) mod emitter_terms;
pub(in crate::app::model_preview) mod gl;
pub(in crate::app::model_preview) mod light_volume_pass;
pub(in crate::app::model_preview) mod light_volumes;
pub(in crate::app::model_preview) mod particle_pass;
pub(in crate::app::model_preview) mod particles;
pub(in crate::app::model_preview) mod tracer_pass;
