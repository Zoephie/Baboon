//! Halo 2's decorator sets: classes of permutations, each a model of the
//! set's own, a sprite quad facing the viewer, or a decal on the ground, and
//! each drawn through an ordinary `shad` of the set's `shaders`.
//!
//! The game keeps a placement in a 24-byte record (MCC `halo2_tag_test`'s
//! `sub_6B20A0` and `sub_94AE50` read it): its class and permutation, a yaw,
//! tint and scale in a few bits each, a normal in four bits an axis, and its
//! place within an 8-unit cell. Everything a decorator looks like follows from
//! those bits, a sprite's pick among its sequence too, so the preview stands
//! its decorators as such records ([`placement`]) and builds them as the
//! cache builder does ([`geometry`]).

mod constants;
mod geometry;
mod placement;
mod read;

pub(in crate::app::model_preview) use constants::{
    HALO_2_RIGID_CONSTANTS, halo_2_vertex_constants,
};
pub(in crate::app::model_preview) use geometry::{bake_halo_2, halo_2_type_name};
pub(in crate::app::model_preview) use placement::{Record, scatter_halo_2};
pub(in crate::app::model_preview) use read::read_halo_2_set;

#[cfg(test)]
mod tests;

/// A Halo 2 `decorator_set`, as its placements are built from.
#[derive(Debug, Clone, Default)]
pub(crate) struct Halo2Set {
    /// The `shad` each `shaders` element names.
    pub shaders: Vec<String>,
    /// Each shader's base map's sprites, for the quads and decals that draw it.
    pub plates: Vec<Option<Plate>>,
    pub classes: Vec<Class>,
    /// Each model's range of `indices`, a triangle list.
    pub models: Vec<std::ops::Range<usize>>,
    pub vertices: Vec<RawVertex>,
    pub indices: Vec<u32>,
    /// Where the preview stands each decorator.
    pub records: Vec<Record>,
}

impl Halo2Set {
    /// Every permutation of every class, in order: the set's types.
    pub(crate) fn types(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.classes.iter().enumerate().flat_map(|(class, entry)| {
            (0..entry.permutations.len()).map(move |permutation| (class, permutation))
        })
    }

    /// The type a class's permutation is, among [`Self::types`].
    pub(crate) fn type_index(&self, class: usize, permutation: usize) -> usize {
        self.classes[..class]
            .iter()
            .map(|entry| entry.permutations.len())
            .sum::<usize>()
            + permutation
    }

    /// Whether type `index` draws: its class's kind known, its shader one
    /// of the set's.
    pub(crate) fn draws(&self, index: usize) -> bool {
        self.types().nth(index).is_some_and(|(class, permutation)| {
            let entry = &self.classes[class];
            entry.kind.is_some()
                && entry.permutations[permutation]
                    .shader
                    .is_some_and(|shader| shader < self.shaders.len())
        })
    }
}

/// One `classes` element: what its permutations draw as.
#[derive(Debug, Clone, Default)]
pub(crate) struct Class {
    pub name: String,
    /// `None` for a `type` this build does not know.
    pub kind: Option<ClassKind>,
    pub permutations: Vec<Permutation>,
}

/// A class's `type`, by the option's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClassKind {
    Model,
    FloatingDecal,
    ProjectedDecal,
    ScreenFacingQuad,
    AxisRotatingQuad,
    CrossQuad,
}

impl ClassKind {
    pub(crate) fn from_option_name(name: &str) -> Option<Self> {
        Some(match name {
            "model" => Self::Model,
            "floating decal" => Self::FloatingDecal,
            "projected decal" => Self::ProjectedDecal,
            "screen facing quad" => Self::ScreenFacingQuad,
            "axis rotating quad" => Self::AxisRotatingQuad,
            "cross quad" => Self::CrossQuad,
            _ => return None,
        })
    }
}

/// One `permutations` element.
#[derive(Debug, Clone, Default)]
pub(crate) struct Permutation {
    pub name: String,
    /// Into the set's `shaders`.
    pub shader: Option<usize>,
    pub align_to_normal: bool,
    /// A model's index into the set's `models`; a quad's or decal's sequence
    /// of its base map.
    pub index: usize,
    pub distribution_weight: f32,
    pub scale: [f32; 2],
    /// `tint 1` and `tint 2`.
    pub tints: [[f32; 3]; 2],
    /// `base map tint percentage` and `lightmap tint percentage`, held to 0
    /// to 1 as the game holds them.
    pub base_map_tint: f32,
    pub lightmap_tint: f32,
    /// `fade distance`'s rate, `c53.x` (`0xE979C4`): a decorator shrinks to
    /// nothing between `2 / k` and `3 / k` from the eye. `None` for an option
    /// this build does not know, which does not fade.
    pub fade: Option<f32>,
}

/// One `raw vertices` element.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
    pub texcoord: [f32; 2],
}

/// A base map's sprites, by sequence, and its first image's size, which
/// sizes a quad.
#[derive(Debug, Clone, Default)]
pub(crate) struct Plate {
    pub sequences: Vec<Vec<Sprite>>,
    pub size: [f32; 2],
}

/// A sprite's bounds on its plate, `[left, right, top, bottom]`, and its
/// registration point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sprite {
    pub bounds: [f32; 4],
    pub registration: [f32; 2],
}

/// A plate with no sprites, as the cache builder takes it: the whole
/// image, registered at its centre.
const WHOLE_IMAGE: Sprite = Sprite {
    bounds: [0.0, 1.0, 0.0, 1.0],
    registration: [0.5, 0.5],
};
