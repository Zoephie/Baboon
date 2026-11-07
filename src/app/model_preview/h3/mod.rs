//! Halo 3 and ODST: leaves, VISR, and the lights and flares the preview
//! draws for them (Reach's flares read through here too). What Halo 3 shares
//! with Reach and Halo 4 is in `gen3`; its draw and bloom are the renderer's.

pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod bloom;
pub(in crate::app::model_preview) mod composite;
pub(crate) mod leaves;
pub(crate) mod lights;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;
pub(crate) mod vision;

/// What only a Halo 3 or ODST object brings to its preview: its lights,
/// shield override, leaves, VISR and motion blur.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3Features {
    /// The model's own `shield impact parameter override`.
    pub shield: Option<std::sync::Arc<super::gen3::shaders::H3ShieldSettings>>,
    pub lights: Vec<super::h3::lights::H3Light>,
    pub leaves: Vec<std::sync::Arc<super::h3::leaves::H3LeavesAt>>,
    /// ODST's VISR, for an object with an effect that turns it on.
    pub vision: Option<std::sync::Arc<super::h3::vision::H3Vision>>,
    /// ODST's motion blur, for an object with an effect that moves the view.
    pub motion_blur: Option<super::h3::vision::H3MotionBlur>,
}
