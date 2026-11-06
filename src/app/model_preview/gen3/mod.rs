//! What Halo 3, ODST, Reach and Halo 4 share: render method shaders and their
//! passes, an object's attached effects and their parts, light volumes,
//! strips, screen effects and PRT.

pub(super) mod effects;
pub(in crate::app::model_preview) mod pipeline;
pub(crate) mod light_volumes;
pub(in crate::app::model_preview) mod render;
pub(in crate::app) mod prt;
pub(crate) mod screen_effects;
pub(in crate::app::model_preview) mod screen_lights;
pub(crate) mod shaders;
pub(in crate::app::model_preview) mod shadow;
pub(in crate::app::model_preview) mod state;
pub(crate) mod strips;
