//! Halo 3 and ODST: their objects' features (lights, leaves, VISR and the
//! motion suck) and their place in the gen3 pipeline (`render.rs`): the self
//! shadow, the object lights, the picture effects, and their bloom and
//! composite. Reach's flares read through `lights.rs` too. What Halo 3
//! shares with Reach and Halo 4 is in `gen3`.

mod features;
pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod bloom;
pub(in crate::app::model_preview) mod composite;
pub(crate) mod leaves;
pub(crate) mod lights;
mod light_pass;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;
pub(crate) mod vision;
mod vision_pass;

/// What only a Halo 3 or ODST object brings to its preview: its lights,
/// shield override, leaves, VISR and motion blur.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3Features {
    /// The model's own `shield impact parameter override`.
    pub shield: Option<std::sync::Arc<super::gen3::shaders::Gen3ShieldSettings>>,
    pub lights: Vec<super::h3::lights::H3Light>,
    pub leaves: Vec<std::sync::Arc<super::h3::leaves::H3LeavesAt>>,
    /// ODST's VISR, for an object with an effect that turns it on.
    pub vision: Option<std::sync::Arc<super::h3::vision::H3Vision>>,
    /// ODST's motion blur, for an object with an effect that moves the view.
    pub motion_blur: Option<super::h3::vision::H3MotionBlur>,
}
