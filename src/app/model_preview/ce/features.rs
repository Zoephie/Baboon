//! Which of the preview's features are Halo CE's own, by the path the load gate names, and how each
//! moves and what it reads.

use super::super::common::features::{Feature, features};

features!(super::CeFeatures {
    lights => "ce.lights",
    antennas => "ce.antennas",
    effects => "ce.effects",
    contrails => "ce.contrails",
    light_volumes => "ce.light_volumes",
    lightning => "ce.lightning",
    glows => "ce.glows",
    flags => "ce.flags",
    particle_systems => "ce.particle_systems";
    skip firing
});

// Halo CE's read no Halo 2 function names; an effect runs at rest only when
// no event plays it.
impl Feature for super::lights::CeLight {}

impl Feature for super::widgets::CeLightVolume {}

impl Feature for super::effects::CeEffectAt {
    /// An event's effect runs only when played, and asks for frames then.
    fn moves(&self) -> bool {
        self.event.is_none()
    }
}

impl Feature for super::contrails::CeContrailAt {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::widgets::CeLightning {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::widgets::CeGlow {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::flags::CeFlagAt {
    fn moves(&self) -> bool {
        true
    }
}

impl Feature for super::particle_systems::CeParticleSystemAt {
    fn moves(&self) -> bool {
        true
    }
}
