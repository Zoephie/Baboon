//! What Halo 3, ODST, Reach and Halo 4 share: render method shaders and their
//! passes, an object's attached effects and their parts, light volumes,
//! strips, screen effects and PRT.

pub(crate) mod effects;
mod features;
pub(in crate::app::model_preview) mod materials;
pub(in crate::app::model_preview) mod pipeline;
pub(in crate::app) mod prt;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod screen_lights;
pub(crate) mod shaders;
pub(in crate::app::model_preview) mod shadow;
pub(crate) mod shield;
pub(in crate::app::model_preview) mod sprites;
pub(in crate::app::model_preview) mod state;

/// What more than one of the gen3 games' objects bring to their previews:
/// Halo 3's, ODST's, Reach's and Halo 4's, each field naming those it is for.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gen3Features {
    pub prt: Option<super::gen3::prt::Gen3Prt>,
    /// Reach's and Halo 4's cheap particle emitters, on their attached
    /// effects' parts.
    pub cheap_emitters: Vec<super::particles::cheap::CheapEmitterAt>,
    /// The markers a Reach or Halo 4 unit's IK chains can reach for: its own
    /// and its held weapons', in bind space on the nodes they ride.
    pub ik_markers: Vec<super::animation::ik::reach::IkMarker>,
    pub flares: Vec<super::gen3::effects::flares::Gen3FlareAt>,
    pub light_volumes: Vec<super::gen3::effects::light_volumes::Gen3LightVolumeAt>,
    pub strips: Vec<std::sync::Arc<super::gen3::effects::strips::Gen3StripAt>>,
    pub screen_effects: Vec<super::gen3::effects::screen::Gen3ScreenEffectAt>,
}
