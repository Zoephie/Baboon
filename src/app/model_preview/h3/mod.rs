//! Halo 3 and ODST: leaves, VISR, and the lights and flares the preview
//! draws for them (Reach's flares read through here too). What Halo 3 shares
//! with Reach and Halo 4 is in `gen3`; its draw and bloom are the renderer's.

pub(in crate::app::model_preview) mod bloom;
pub(in crate::app::model_preview) mod composite;
pub(crate) mod leaves;
pub(crate) mod lights;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;
pub(crate) mod vision;
