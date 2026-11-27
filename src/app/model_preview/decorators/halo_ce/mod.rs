//! Halo CE's detail objects: a collection's types of sprite, each a
//! billboard standing on its bottom edge, painted on the ground by Sapien's
//! brush (MCC `haloce_mcc_sapien`'s `detail_object_tool_handler.cpp`).
//!
//! The game draws each instance as a quad of its type's sprite, turned to the
//! viewer in its vertex shader (`detail_object_type0` and `type1.vsh`). Baked
//! here, for Solid and Textured, it faces an eye looking along +Y, where the
//! screen-facing and viewer-facing kinds stand alike.

mod geometry;
mod placement;
mod read;
mod shading;

pub(in crate::app::model_preview) use geometry::bake_halo_ce;
pub(in crate::app::model_preview) use placement::{Instance, scatter_halo_ce};
pub(in crate::app::model_preview) use read::read_halo_ce_collection;
pub(in crate::app::model_preview) use shading::{
    DetailFacing, detail_object_pass, halo_ce_vertex_constants,
};

#[cfg(test)]
mod tests;

/// A `detail_object_collection`, as its instances are built from.
#[derive(Debug, Clone, Default)]
pub(crate) struct HaloCeCollection {
    /// `global z offset`, which the game adds at load (`sub_5A9040`).
    pub z_offset: f32,
    pub plate: Plate,
    pub types: Vec<DetailType>,
    /// Where the preview stands each detail object.
    pub instances: Vec<Instance>,
}

/// One `types` element.
#[derive(Debug, Clone, Default)]
pub(crate) struct DetailType {
    pub name: String,
    /// The plate's sequence it shows.
    pub sequence: usize,
    /// World units per texel of its sprite.
    pub size: f32,
    pub near_fade: f32,
    pub far_fade: f32,
    /// `minimum color` and `maximum color`.
    pub colors: [[f32; 3]; 2],
    /// `interpolate color in hsv`, and `...more colors`, which takes the hue
    /// the long way round.
    pub hsv: bool,
    pub more_colors: bool,
    pub color_override: f32,
    /// `ambient color`, alpha first, each 0 to 1.
    pub ambient: [f32; 4],
}

/// The sprite plate: its sprites by sequence, and each image's size, which
/// sizes a sprite drawn from it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plate {
    pub sequences: Vec<Vec<Sprite>>,
    pub images: Vec<[f32; 2]>,
}

/// A sprite: its image and its bounds on it, `[left, right, top, bottom]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sprite {
    pub image: usize,
    pub bounds: [f32; 4],
}

impl HaloCeCollection {
    /// Whether type `index` has a sprite to draw.
    pub(crate) fn draws(&self, index: usize) -> bool {
        self.types
            .get(index)
            .and_then(|kind| self.plate.sequences.get(kind.sequence))
            .is_some_and(|sprites| !sprites.is_empty())
    }
}
