//! The DX9 pipeline Halo CE and Halo 2 draw through: a shader's passes, and
//! the vertex stages that feed them.

pub(in crate::app::model_preview) mod binds;
pub(in crate::app::model_preview) mod draw;
mod frame;
pub(in crate::app) mod passes;
pub(in crate::app::model_preview) mod pipeline;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod resolved;
pub(in crate::app::model_preview) mod shadow;
pub(in crate::app::model_preview) mod sprites;
pub(in crate::app) mod stage;
