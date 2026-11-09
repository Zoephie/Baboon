//! Reach: object lights, camo, shadow and shield, several of which Halo 4
//! draws through too. Its lighting, bloom and passes are the renderer's.

mod features;
pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod attached;
pub(super) mod camo;
pub(in crate::app::model_preview) mod bloom;
pub(in crate::app::model_preview) mod composite;
pub(in crate::app::model_preview) mod lighting;
pub(in crate::app::model_preview) mod light_pass;
pub(super) mod lights;
pub(in crate::app::model_preview) mod render;
pub(super) mod shadow;
pub(in crate::app::model_preview) mod shadow_pass;
pub(super) mod shield;

/// What only a Reach object brings to its preview: its lights and their
/// screen-space passes, shield, camo, self shadow and composite noise.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReachFeatures {
    /// Reach's attached `light`s and `cheap_light`s.
    pub lights: Vec<super::reach::lights::ReachLight>,
    pub cheap_lights: Vec<super::reach::lights::ReachCheapLight>,
    /// `light_apply`, `_frustum`, `_specular` and `_frustum_specular`, by
    /// `combo` (frustum 1, specular 2), for its screen-space lights.
    pub light_passes: Vec<Option<std::sync::Arc<super::gen3::shaders::H3Pass>>>,
    /// Reach's shield impact, as its damage sections choose it.
    pub shield: Option<std::sync::Arc<super::reach::shield::ReachShield>>,
    /// `globals`' active camo curves, for a Reach object.
    pub camo: Option<std::sync::Arc<super::reach::camo::ReachCamo>>,
    /// Whether a Reach object shadows itself, and its map's biases.
    pub shadow_caster: Option<super::reach::shadow::ReachShadowCaster>,
    /// Reach's final composite noise map, for an object with an area screen
    /// effect that adds bright or dark noise.
    pub screen_noise: Option<super::gen3::materials::H3Bitmap>,
}
