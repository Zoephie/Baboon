//! What the dx9 passes read off a frame: which layers draw, which lightmap
//! layer lights them, and which way the object's light comes from.

use super::super::camera_light;
use super::super::frame::ModelGpuFrame;
use super::passes::{Dx9Layer, object_light_direction};

impl ModelGpuFrame {
    /// The passes are the game's shaders, so they draw with Game Shaders on
    /// alone, the transparent ones too; the camo layer with camo on.
    pub(in crate::app::model_preview) fn dx9_draws(&self, layer: Dx9Layer) -> bool {
        match layer {
            Dx9Layer::ActiveCamo => self.active_camo,
            _ => self.game_shaders,
        }
    }

    pub(in crate::app::model_preview) fn dx9_lightmap_layer(&self) -> Dx9Layer {
        if self.default_lights {
            Dx9Layer::Lightmap
        } else {
            Dx9Layer::ObjectLightmap
        }
    }

    /// Which way the object path's direct light lies from a surface.
    pub(in crate::app::model_preview) fn dx9_object_light(&self) -> [f32; 3] {
        if self.light_follows_camera {
            camera_light(self.camera.yaw, self.camera.pitch)
        } else {
            object_light_direction()
        }
    }
}
