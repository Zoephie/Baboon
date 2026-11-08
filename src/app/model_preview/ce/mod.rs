//! Halo CE: what an object brings with it, its functions, and the effects,
//! lights and shader passes the preview draws for it.

pub(in crate::app::model_preview) mod materials;
pub(in crate::app::model_preview) mod object;
pub(super) mod attachments;
pub(super) mod contrails;
pub(in crate::app::model_preview) mod draw;
pub(super) mod firing;
pub(super) mod flags;
pub(super) mod functions;
pub(super) mod lights;
pub(super) mod particle_systems;
pub(super) mod particles;
pub(super) mod passes;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;
pub(super) mod widgets;

/// What a Halo CE object brings to its preview beyond its geometry: its
/// lights' flares, antennas, effects, firing, contrails and widgets.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeFeatures {
    /// Halo CE: the lens flares of the object and what it holds.
    pub lights: Vec<super::ce::lights::CeLight>,
    /// Halo CE: the antennas among the object's widgets.
    pub antennas: Vec<super::antennas::ce::CeAntenna>,
    /// Halo CE effects the object's attachments hang on it, then those it
    /// names for its events.
    pub effects: Vec<super::ce::particles::CeEffectAt>,
    /// Halo CE weapons whose triggers the panel fires: the opened one, or
    /// the one the opened unit holds.
    pub firing: Vec<super::ce::firing::CeWeaponFiring>,
    pub contrails: Vec<super::ce::contrails::CeContrailAt>,
    pub light_volumes: Vec<super::ce::widgets::CeLightVolume>,
    pub lightning: Vec<super::ce::widgets::CeLightning>,
    pub glows: Vec<super::ce::widgets::CeGlow>,
    pub flags: Vec<super::ce::flags::CeFlagAt>,
    pub particle_systems: Vec<super::ce::particle_systems::CeParticleSystemAt>,
}
