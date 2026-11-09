//! The GL toolkit every pipeline draws with: objects that clean up after
//! themselves when a later step fails.

pub(in crate::app::model_preview) mod blit;
pub(in crate::app::model_preview) mod buffer;
pub(in crate::app::model_preview) mod chain;
pub(in crate::app::model_preview) mod glsl;
#[cfg(test)]
pub(in crate::app::model_preview) mod glslang;
pub(in crate::app::model_preview) mod link;
pub(in crate::app::model_preview) mod pass_state;
pub(in crate::app::model_preview) mod program;
pub(in crate::app::model_preview) mod screen;
pub(in crate::app::model_preview) mod scope;
pub(in crate::app::model_preview) mod target;
pub(in crate::app::model_preview) mod texture;
