//! What a Halo 2 shader template's pass reads as: its stages, state,
//! textures, parameters, vertex constants and animations, before it becomes
//! a `Dx9DrawPass`.

use super::super::super::dx9::passes::{Dx9Animation, Dx9Blend, Dx9Depth};
use crate::core::h2_pixel_shader::AlphaTest;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum Dx9ParameterValue {
    Bitmap(String),
    Value(f32),
    Color([f32; 3]),
    /// Filled at runtime, by this option name of the pass's `source extern`.
    Extern(String),
    /// Nothing in the shader, template or pass supplies it.
    Missing,
}
/// A vertex constant the pass fills from a parameter: for a bitmap, a row of
/// its texture transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Dx9VertexConstant {
    pub(in crate::app) parameter: String,
    pub(in crate::app) bank: String,
    pub(in crate::app) register: i32,
    pub(in crate::app) components: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct Dx9PassTexture {
    pub(in crate::app) parameter: String,
    pub(in crate::app) mode: String,
    pub(in crate::app) value: Dx9ParameterValue,
}
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct Dx9Pass {
    pub(in crate::app) path: String,
    pub(in crate::app) implementation: usize,
    pub(in crate::app) vertex_shader: String,
    /// `None` draws without blending.
    pub(in crate::app) blend: Option<Dx9Blend>,
    pub(in crate::app) depth: Dx9Depth,
    pub(in crate::app) channels: String,
    pub(in crate::app) custom_channels: Option<[bool; 4]>,
    pub(in crate::app) alpha_test: Option<AlphaTest>,
    /// One per texture stage, in stage order.
    pub(in crate::app) textures: Vec<Dx9PassTexture>,
    pub(in crate::app) parameters: Vec<(String, Dx9ParameterValue)>,
    pub(in crate::app) vertex_constants: Vec<Dx9VertexConstant>,
    /// What the shader animates of `parameters`, by parameter.
    pub(in crate::app) animations: std::collections::BTreeMap<String, Vec<Dx9Animation>>,
}
impl Dx9Pass {
    /// The kit's generated source for this pass's implementation.
    pub(in crate::app) fn pixel_shader_file(&self) -> String {
        format!(
            "{}_{}.psh",
            self.path.replace(['\\', '/'], "_"),
            self.implementation
        )
    }
}
