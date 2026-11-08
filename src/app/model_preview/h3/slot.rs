//! What Halo 3 and ODST keep for each of the renderer's slots: the leaves
//! and their maps, and how long VISR has been on.

use eframe::glow::{self, HasContext as _};

use super::super::common::slots::GameSlot;
use super::super::frame::ModelGpuFrame;
use super::super::renderer::sprite_maps;

/// Halo 3's and ODST's state for one of the renderer's slots.
#[derive(Default)]
pub(in crate::app::model_preview) struct H3Slot {
    pub(in crate::app::model_preview) leaves: super::leaves::H3LeafField,
    /// How long VISR has been on, as of when.
    pub(in crate::app::model_preview) vision_clock: Option<(f32, f32)>,
    /// Each of `h3.leaves`' maps, of `textures_for`.
    pub(in crate::app::model_preview) leaf_textures: Vec<[Option<glow::NativeTexture>; 4]>,
    textures_for: Option<u64>,
}

impl GameSlot for H3Slot {
    fn restart(&mut self) {
        self.leaves = Default::default();
        self.vision_clock = None;
    }
}

impl H3Slot {
    /// The leaves' maps, uploaded once per geometry.
    pub(in crate::app::model_preview) unsafe fn sync_textures(&mut self, gl: &glow::Context, frame: &ModelGpuFrame) {
        if self.textures_for == Some(frame.geometry_id) {
            return;
        }
        unsafe {
            for texture in self.leaf_textures.drain(..).flatten().flatten() {
                gl.delete_texture(texture);
            }
            self.leaf_textures = frame.preview.h3.leaves.iter().map(|leaves| sprite_maps(gl, leaves.shader())).collect();
        }
        self.textures_for = Some(frame.geometry_id);
    }

    /// How long VISR has been on as of `time`, the clock kept running while
    /// time goes on and started over when it goes back.
    pub(in crate::app::model_preview) fn vision_on_for(&mut self, time: f32) -> f32 {
        let on_for = match self.vision_clock {
            Some((last, seconds)) if time >= last => seconds + (time - last),
            _ => 0.0,
        };
        self.vision_clock = Some((time, on_for));
        on_for
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visr_counts_on_while_time_goes_on_and_starts_over_when_it_goes_back() {
        let mut slot = H3Slot::default();
        assert_eq!(slot.vision_on_for(2.0), 0.0, "it counts from its first frame");
        assert_eq!(slot.vision_on_for(2.5), 0.5);
        assert_eq!(slot.vision_on_for(4.0), 2.0);
        assert_eq!(slot.vision_on_for(1.0), 0.0, "time going back starts it over");
        assert_eq!(slot.vision_on_for(1.25), 0.25);
        slot.restart();
        assert_eq!(slot.vision_on_for(3.0), 0.0, "the slot's next model starts it over");
    }
}
