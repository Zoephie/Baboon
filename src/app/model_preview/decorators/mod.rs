//! Decorator sets: the grass, flowers and rocks a level scatters by the
//! thousand, previewed on a patch of ground of their own.
//!
//! Halo 3, ODST, Reach and Halo 4 share one design. A set names a render
//! model whose one mesh holds a subpart per decorator type, a texture that is
//! its only material, and the explicit shader its `render shader` chooses.
//! The game draws each type as one strip, instanced per placement.
//!
//! A level bakes where its decorators stand and how they are lit, so a set on
//! its own has neither. The preview lays them out the way the game's brush
//! does ([`layout`]), from the ranges each type gives. See
//! `docs/decorator-preview-plan.md`.
//!
//! Every flag and enum is read by its option name from the tag's own layout.
//! Layouts add and drop options: the newer Halo 3 maps' sets have a third
//! render flag, and Reach renamed every `render shader` option.

mod bake;
mod gpu;
mod halo2;
mod halo_ce;
mod instances;
mod layout;
mod material;
mod read;
mod shader;
mod stream;
mod wind;

pub(in crate::app::model_preview) use bake::{PERMUTATION, bake_placements, relayout, type_name};
pub(in crate::app::model_preview) use gpu::{
    DecoratorBuffers, INSTANCE_ATTRIBUTES, decorator_value,
};
pub(in crate::app::model_preview) use halo_ce::{
    DetailFacing, bake_halo_ce, detail_object_pass, halo_ce_vertex_constants,
    read_halo_ce_collection, scatter_halo_ce,
};
pub(in crate::app::model_preview) use halo2::{
    HALO_2_RIGID_CONSTANTS, bake_halo_2, halo_2_type_name, halo_2_vertex_constants,
    read_halo_2_set, scatter_halo_2,
};
pub(in crate::app::model_preview) use layout::{Layout, scatter};
pub(in crate::app::model_preview) use material::{
    resolve_decorator_material, resolve_detail_object_material,
};
pub(in crate::app::model_preview) use read::read_decorator_set;
pub(in crate::app::model_preview) use shader::DecoratorShader;
pub(in crate::app::model_preview) use stream::{
    DECORATOR_STREAM_ATTRIBUTES, DecoratorStream, DecoratorStreamVertex,
};
pub(in crate::app::model_preview) use wind::WindDefinition;

#[cfg(test)]
mod tests;

/// A decorator set, as the preview draws it.
#[derive(Debug, Clone, Default)]
pub(crate) struct DecoratorSet {
    /// The shader `render shader` names; `None` for an option this build
    /// does not know.
    pub shader: Option<DecoratorShader>,
    /// `render two sided`: drawn with no culling.
    pub two_sided: bool,
    /// The set's only material.
    pub texture: String,
    /// `translucency A`, `B`, `C` and `translucency`, as the tag has them:
    /// the shader's `translucency`, which its dynamic lights and sun read.
    pub translucency: [f32; 4],
    /// `wavelength X`, `wavelength Y`, `wave speed` and `wave frequency`: the
    /// wavy shader's `wave_flow`, as the tag has them.
    pub wave: [f32; 4],
    /// `shaded dark` and `shaded bright`: the shaded shader's `contrast`.
    pub shaded: [f32; 2],
    /// The wind it sways in: the globals' default.
    pub wind: Option<std::sync::Arc<WindDefinition>>,
    pub types: Vec<DecoratorType>,
    /// Each type's mesh, by type.
    pub meshes: Vec<DecoratorMesh>,
    /// The patch [`Self::placements`] fill.
    pub layout: Layout,
    /// A Halo 2 set's own: its classes, models and placements. Its types
    /// are its permutations, which [`Self::types`] names alone.
    pub halo2: Option<std::sync::Arc<halo2::Halo2Set>>,
    /// A Halo CE detail object collection's own: its sprite plate and
    /// instances. Its types are its own, which [`Self::types`] names.
    pub halo_ce: Option<std::sync::Arc<halo_ce::HaloCeCollection>>,
    /// Where the preview stands each decorator.
    pub placements: Vec<Placement>,
}

/// One `decorator types` element: which mesh it draws, and the ranges the
/// brush picks each placement's look from.
#[derive(Debug, Clone, Default)]
pub(crate) struct DecoratorType {
    /// The render model instance placement its `mesh` names.
    pub mesh_name: String,
    pub random_rotation: bool,
    pub scale: [f32; 2],
    /// In degrees.
    pub tilt: [f32; 2],
    pub wind: [f32; 2],
    /// `color 0`, `color 1` and `color 2`, which a placement mixes.
    pub colors: [[f32; 3]; 3],
    pub ground_tint: [f32; 2],
    pub hover: [f32; 2],
    pub minimum_distance: f32,
    /// Reach's and Halo 4's `placement weight`; Halo 3's types have none, and
    /// are picked evenly.
    pub placement_weight: Option<f32>,
}

/// One type's mesh: its subpart of the render model's decorator mesh, as a
/// triangle list.
#[derive(Debug, Clone, Default)]
pub(crate) struct DecoratorMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub texcoords: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// One decorator stood on the ground, as the game's placement keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    pub type_index: usize,
    pub position: [f32; 3],
    /// A unit quaternion, `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub scale: f32,
    /// How far the wind moves it, from the type's wind range.
    pub motion: f32,
    /// The type's three colours mixed at random.
    pub tint: [f32; 3],
    /// How far its colour leans toward the ground's.
    pub ground_tint: f32,
}

impl DecoratorSet {
    /// Whether its shader moves it on the clock: the wind's, or the waves'.
    pub(crate) fn moves(&self) -> bool {
        self.shader.is_some_and(DecoratorShader::moves)
    }

    /// How many of type `index` the patch stands.
    pub(crate) fn placed(&self, index: usize) -> usize {
        if let Some(set) = &self.halo2 {
            return set
                .records
                .iter()
                .filter(|record| set.type_index(record.class, record.permutation) == index)
                .count();
        }
        if let Some(collection) = &self.halo_ce {
            return collection
                .instances
                .iter()
                .filter(|instance| instance.type_index == index)
                .count();
        }
        self.placements
            .iter()
            .filter(|placement| placement.type_index == index)
            .count()
    }

    /// Whether type `index` has anything to draw.
    pub(crate) fn draws_type(&self, index: usize) -> bool {
        if let Some(set) = &self.halo2 {
            return set.draws(index);
        }
        if let Some(collection) = &self.halo_ce {
            return collection.draws(index);
        }
        self.meshes
            .get(index)
            .is_some_and(|mesh| !mesh.indices.is_empty())
    }

    /// The type a batch's region names ([`bake::type_name`]).
    pub(crate) fn type_of_region(&self, region: &str) -> Option<usize> {
        (0..self.types.len())
            .find(|&index| bake::type_name(index, &self.types[index].mesh_name) == region)
    }
}
