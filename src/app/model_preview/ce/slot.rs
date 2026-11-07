//! What Halo CE keeps for each of the renderer's slots: its effects' and
//! widgets' simulations, and the maps its sprites draw with.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::glow::{self, HasContext as _};

use super::super::common::slots::GameSlot;
use super::super::common::image::H2Image;
use super::super::renderer::{ModelGpuFrame, upload_h2_image};

/// A map per image, per sheet, per particle system.
type SystemMaps = Vec<Vec<Vec<Option<glow::NativeTexture>>>>;

/// Halo CE's simulations for one slot, which run on from frame to frame.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeSimulations {
    pub(in crate::app::model_preview) contrails: super::contrails::CeContrailField,
    pub(in crate::app::model_preview) glows: super::widgets::CeGlowField,
    pub(in crate::app::model_preview) flags: super::flags::CeFlagField,
    /// The effects' particles, systems and lights.
    pub(in crate::app::model_preview) particles: super::particles::CeParticleField,
    /// The attached particle systems.
    pub(in crate::app::model_preview) pctl: super::particle_systems::CeParticleSystemField,
}

impl CeSimulations {
    /// Each simulation by name, for the G-sim gate.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn each(&self, visit: &mut dyn FnMut(&str, &dyn std::fmt::Debug)) {
        let Self { contrails, glows, flags, particles, pctl } = self;
        visit("ce_contrails", contrails);
        visit("ce_glows", glows);
        visit("ce_flags", flags);
        visit("ce_particles", particles);
        visit("ce_pctl", pctl);
    }
}

/// The maps Halo CE's sprites draw with, uploaded once per geometry.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeTextures {
    /// Each light's lens flare's images.
    pub(in crate::app::model_preview) light_flares: Vec<Vec<Option<glow::NativeTexture>>>,
    /// The flares of the lights the effects make, by effect.
    pub(in crate::app::model_preview) effect_flares: HashMap<usize, Vec<Vec<Option<glow::NativeTexture>>>>,
    pub(in crate::app::model_preview) antennas: Vec<Option<glow::NativeTexture>>,
    /// The effects' particles' images, by effect.
    pub(in crate::app::model_preview) particles: HashMap<usize, Vec<Vec<Option<glow::NativeTexture>>>>,
    pub(in crate::app::model_preview) contrails: Vec<Vec<Option<glow::NativeTexture>>>,
    pub(in crate::app::model_preview) light_volumes: Vec<Option<glow::NativeTexture>>,
    pub(in crate::app::model_preview) lightning: Vec<Option<glow::NativeTexture>>,
    pub(in crate::app::model_preview) glows: Vec<Option<glow::NativeTexture>>,
    /// The attached particle systems' sheets.
    pub(in crate::app::model_preview) pctl: SystemMaps,
    /// The effects' particle systems' sheets, by effect.
    pub(in crate::app::model_preview) effect_pctl: HashMap<usize, SystemMaps>,
    /// Second maps, uploaded as sprites first ask for them.
    pub(in crate::app::model_preview) secondary: HashMap<usize, glow::NativeTexture>,
}

impl CeTextures {
    /// Every map held, deleted.
    unsafe fn delete(&mut self, gl: &glow::Context) {
        let Self { light_flares, effect_flares, antennas, particles, contrails, light_volumes, lightning, glows, pctl, effect_pctl, secondary } =
            std::mem::take(self);
        let deleted = light_flares
            .into_iter()
            .flatten()
            .chain(effect_flares.into_values().flatten().flatten())
            .chain(antennas)
            .chain(particles.into_values().flatten().flatten())
            .chain(contrails.into_iter().flatten())
            .chain(light_volumes)
            .chain(lightning)
            .chain(glows)
            .chain(pctl.into_iter().flatten().flatten())
            .chain(effect_pctl.into_values().flatten().flatten().flatten())
            .flatten()
            .chain(secondary.into_values());
        for texture in deleted {
            unsafe { gl.delete_texture(texture) };
        }
    }

    /// The maps of what `frame`'s geometry draws.
    unsafe fn upload(gl: &glow::Context, frame: &ModelGpuFrame) -> Self {
        let preview = &frame.preview;
        let image = |image: &Option<Arc<H2Image>>| image.as_ref().and_then(|image| unsafe { upload_h2_image(gl, image) });
        let sheets = |sheets: &[Arc<super::particles::CeSpriteSheet>]| -> Vec<Vec<Option<glow::NativeTexture>>> {
            sheets.iter().map(|sheet| sheet.images.iter().map(image).collect()).collect()
        };
        let flare = |light: &super::lights::CeLight| -> Vec<Option<glow::NativeTexture>> {
            let images = light.flare.iter().flat_map(|flare| &flare.images);
            images.map(|image| unsafe { upload_h2_image(gl, image) }).collect()
        };
        let reachable = super::particles::reachable_defs(&preview.ce.effects);
        let by_effect = |def: &Arc<super::particles::CeEffectDef>| super::particles::def_key(def);
        unsafe {
            Self {
                light_flares: preview.ce.lights.iter().map(flare).collect(),
                effect_flares: reachable
                    .iter()
                    .map(|def| (by_effect(def), def.lights.iter().map(|light| flare(&light.light)).collect()))
                    .collect(),
                antennas: preview.ce.antennas.iter().map(|antenna| upload_h2_image(gl, &antenna.image)).collect(),
                particles: reachable
                    .iter()
                    .map(|def| (by_effect(def), def.particles.iter().map(|particle| particle.images.iter().map(image).collect()).collect()))
                    .collect(),
                contrails: preview.ce.contrails.iter().map(|contrail| contrail.def.images.iter().map(image).collect()).collect(),
                light_volumes: preview.ce.light_volumes.iter().map(|volume| upload_h2_image(gl, &volume.image)).collect(),
                lightning: preview.ce.lightning.iter().map(|lightning| upload_h2_image(gl, &lightning.image)).collect(),
                glows: preview.ce.glows.iter().map(|glow| upload_h2_image(gl, &glow.image)).collect(),
                pctl: preview.ce.particle_systems.iter().map(|system| sheets(&system.def.sheets)).collect(),
                effect_pctl: reachable
                    .iter()
                    .map(|def| (by_effect(def), def.systems.iter().map(|system| sheets(&system.sheets)).collect()))
                    .collect(),
                secondary: HashMap::new(),
            }
        }
    }
}

/// Halo CE's state for one of the renderer's slots.
#[derive(Default)]
pub(in crate::app::model_preview) struct CeSlot {
    /// The geometry the maps are for.
    textures_for: Option<u64>,
    pub(in crate::app::model_preview) simulations: CeSimulations,
    pub(in crate::app::model_preview) textures: CeTextures,
}

impl GameSlot for CeSlot {
    fn restart(&mut self) {
        self.simulations = CeSimulations::default();
    }
}

impl CeSlot {
    /// The maps of what `frame`'s geometry draws, uploaded once per geometry.
    pub(in crate::app::model_preview) unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.textures_for == Some(frame.geometry_id) {
            return;
        }
        unsafe {
            self.textures.delete(gl);
            self.textures = CeTextures::upload(gl, frame);
        }
        self.textures_for = Some(frame.geometry_id);
    }
}
