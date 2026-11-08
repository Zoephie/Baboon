//! What Halo 3, ODST, Reach and Halo 4 share: render method shaders and their
//! passes, an object's attached effects and their parts, light volumes,
//! strips, screen effects and PRT.

pub(crate) mod flares;
pub(in crate::app::model_preview) mod materials;
pub(in crate::app::model_preview) mod pipeline;
pub(crate) mod light_volumes;
pub(in crate::app::model_preview) mod render;
pub(in crate::app) mod prt;
pub(crate) mod screen_effects;
pub(in crate::app::model_preview) mod screen_lights;
pub(crate) mod shaders;
pub(in crate::app::model_preview) mod shadow;
pub(crate) mod shield;
pub(in crate::app::model_preview) mod sprites;
pub(in crate::app::model_preview) mod state;
pub(crate) mod strips;

/// What more than one of the gen3 games' objects bring to their previews:
/// Halo 3's, ODST's, Reach's and Halo 4's, each field naming those it is for.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gen3Features {
    pub prt: Option<super::gen3::prt::H3Prt>,
    /// Reach's and Halo 4's cheap particle emitters, on their attached
    /// effects' parts.
    pub cheap_emitters: Vec<super::particles::cheap::ReachCheapEmitterAt>,
    /// The markers a Reach or Halo 4 unit's IK chains can reach for: its own
    /// and its held weapons', in bind space on the nodes they ride.
    pub ik_markers: Vec<super::animation::ik::reach::ReachIkMarker>,
    pub flares: Vec<super::gen3::flares::H3FlareAt>,
    pub light_volumes: Vec<super::gen3::light_volumes::H3LightVolumeAt>,
    pub strips: Vec<std::sync::Arc<super::gen3::strips::H3StripAt>>,
    pub screen_effects: Vec<super::gen3::screen_effects::H3ScreenEffectAt>,
}
