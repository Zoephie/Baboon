//! What Halo 2 keeps for each of the renderer's slots: its contrails'
//! simulation, and the maps its flares, light volumes, liquids, contrails
//! and gels draw with.

use std::sync::Arc;

use eframe::glow::{self, HasContext as _};

use super::super::common::slots::GameSlot;
use super::super::dx9::passes::H2Image;
use super::super::renderer::{ModelGpuFrame, upload_h2_image};

/// The maps Halo 2's sprites draw with, uploaded once per geometry.
#[derive(Default)]
pub(in crate::app::model_preview) struct H2Textures {
    /// Each flare's images, of `h2_flares`.
    pub(in crate::app::model_preview) flares: Vec<Vec<Option<glow::NativeTexture>>>,
    /// Each of `h2_light_volumes`' bitmaps, by volume.
    pub(in crate::app::model_preview) light_volumes: Vec<Vec<Option<glow::NativeTexture>>>,
    /// Each of `h2_liquids`' arcs' bitmaps.
    pub(in crate::app::model_preview) liquids: Vec<Vec<Option<glow::NativeTexture>>>,
    pub(in crate::app::model_preview) contrails: Vec<Option<glow::NativeTexture>>,
}

impl H2Textures {
    /// Every map held, deleted.
    unsafe fn delete(&mut self, gl: &glow::Context) {
        let Self { flares, light_volumes, liquids, contrails } = std::mem::take(self);
        let deleted = flares.into_iter().chain(light_volumes).chain(liquids).flatten().chain(contrails).flatten();
        for texture in deleted {
            unsafe { gl.delete_texture(texture) };
        }
    }

    /// The maps of what `frame`'s geometry draws.
    unsafe fn upload(gl: &glow::Context, frame: &ModelGpuFrame) -> Self {
        let preview = &frame.preview;
        let image = |image: &Arc<H2Image>| unsafe { upload_h2_image(gl, image) };
        Self {
            flares: preview.h2_flares.iter().map(|at| at.flare.images.iter().map(image).collect()).collect(),
            light_volumes: preview
                .h2_light_volumes
                .iter()
                .map(|at| at.volumes().iter().map(|volume| volume.bitmap().and_then(image)).collect())
                .collect(),
            liquids: preview
                .h2_liquids
                .iter()
                .map(|at| at.arcs().iter().map(|arc| arc.bitmap().and_then(image)).collect())
                .collect(),
            contrails: preview.h2_contrails.iter().map(|at| at.bitmap().and_then(image)).collect(),
        }
    }
}

/// Halo 2's state for one of the renderer's slots.
#[derive(Default)]
pub(in crate::app::model_preview) struct H2Slot {
    pub(in crate::app::model_preview) contrails: super::contrails::H2ContrailField,
    /// The geometry `textures` are for.
    textures_for: Option<u64>,
    pub(in crate::app::model_preview) textures: H2Textures,
    /// Each of `h2_gel_lights`' gels, of `gels_for`.
    gels: Vec<Option<glow::NativeTexture>>,
    gels_for: Option<u64>,
}

impl GameSlot for H2Slot {
    fn restart(&mut self) {
        self.contrails = Default::default();
    }
}

impl H2Slot {
    /// The maps of what `frame`'s geometry draws, uploaded once per geometry.
    pub(in crate::app::model_preview) unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.textures_for == Some(frame.geometry_id) {
            return;
        }
        unsafe {
            self.textures.delete(gl);
            self.textures = H2Textures::upload(gl, frame);
        }
        self.textures_for = Some(frame.geometry_id);
    }

    /// The gel lights' maps, uploaded once per geometry.
    pub(in crate::app::model_preview) unsafe fn gels(
        &mut self,
        gl: &glow::Context,
        frame: &ModelGpuFrame,
    ) -> &[Option<glow::NativeTexture>] {
        if self.gels_for != Some(frame.geometry_id) {
            for texture in self.gels.drain(..).flatten() {
                unsafe { gl.delete_texture(texture) };
            }
            let gels = frame.preview.h2_gel_lights.iter().map(|light| light.gel());
            self.gels = gels.map(|gel| gel.and_then(|gel| unsafe { upload_h2_image(gl, gel) })).collect();
            self.gels_for = Some(frame.geometry_id);
        }
        &self.gels
    }
}
