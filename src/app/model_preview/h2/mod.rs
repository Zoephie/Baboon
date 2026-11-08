//! Halo 2: an object's functions and what it holds, and the bloom, lights,
//! light volumes, contrails, liquid and PRT the preview draws for it. Its
//! shader passes, which Halo CE's share, are in `dx9`.

pub(in crate::app::model_preview) mod materials;
pub(in crate::app::model_preview) mod object;
pub(in crate::app::model_preview) mod particles;
pub(in crate::app::model_preview) mod attached;
pub(super) mod bloom;
pub(super) mod contrails;
pub(super) mod light_volumes;
pub(super) mod liquid;
pub(super) mod lights;
pub(super) mod effect_part;
pub(in crate::app::model_preview) mod passes;
pub(in crate::app) mod prt;
pub(in crate::app::model_preview) mod render;
pub(in crate::app::model_preview) mod slot;

/// What a Halo 2 object brings to its preview beyond its geometry: its PRT,
/// hologram control, flares, lights, light volumes, contrails and liquids.
#[derive(Debug, Clone, Default)]
pub(crate) struct H2Features {
    /// Halo 2 precomputed radiance transfer, on the few models that have it.
    pub prt: Option<super::h2::prt::H2Prt>,
    /// Halo 2: the function whose value above nothing turns on the model's
    /// hologram shader.
    pub hologram_control: Option<String>,
    /// Halo 2 lens flares, on the markers of the object and what it holds.
    pub flares: Vec<super::h2::lights::H2FlareAt>,
    /// Halo 2 sphere lights on the object and what it holds.
    pub point_lights: Vec<super::h2::lights::H2PointLight>,
    pub gel_lights: Vec<super::h2::lights::H2GelLight>,
    pub light_volumes: Vec<super::h2::light_volumes::H2LightVolume>,
    pub contrails: Vec<super::h2::contrails::H2ContrailAt>,
    pub liquids: Vec<super::h2::liquid::H2LiquidAt>,
}
