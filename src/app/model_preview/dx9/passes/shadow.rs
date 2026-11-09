//! The self-shadow passes, and the plain pass the games' own shadow passes
//! are made from.

use super::super::super::gl::glsl::fill;
use super::Dx9VertexStage;
use super::layer::Dx9Layer;
use super::lights::SHADOW_BIAS;
use super::pass::{Dx9Compare, Dx9Cull, Dx9Depth, Dx9DrawPass, PASS_GLSL_VERSION, identity_rows};

/// A pass of the self shadow's: no blend, bias or textures, culling by the
/// frame, testing depth by `compare` and writing it if `write`.
pub(in crate::app::model_preview) fn shadow_pass(
    stage: Dx9VertexStage,
    fragment: String,
    compare: Dx9Compare,
    write: bool,
) -> Dx9DrawPass {
    Dx9DrawPass {
        stage,
        fragment,
        blend: None,
        depth_bias: 0.0,
        depth: Dx9Depth {
            compare: Some(compare),
            write,
        },
        cull: Dx9Cull::Frame,
        hidden_by_camo: false,
        numeric: None,
        channels: [false; 4],
        textures: Vec::new(),
        constants: Vec::new(),
        texture_rows: identity_rows(),
        tint_facing: [1.0; 4],
        tint_glancing: [1.0; 4],
        change_colors: Vec::new(),
        light_colors: Vec::new(),
        camo_amounts: Vec::new(),
        light_direct: [0.0; 3],
        light_indirect: [0.0; 3],
        animated_constants: Vec::new(),
        animated_rows: Vec::new(),
        animated_row_values: Vec::new(),
        animated_tint_facing: Vec::new(),
        animated_tint_glancing: Vec::new(),
        ce_animation: None,
    }
}

/// The passes behind the preview's own key light shadow. Under the default
/// lights the game draws no shadows at all: layers 10 and 11 only run for a
/// light object that casts them. The games add their own beside these (see
/// `load::games::shadow_passes`).
pub(crate) fn dx9_shadow_passes() -> [(Dx9Layer, Dx9DrawPass); 2] {
    let pass = shadow_pass;
    [
        (
            Dx9Layer::ShadowCaster,
            pass(
                Dx9VertexStage::ShadowCaster,
                format!("{PASS_GLSL_VERSION}void main() {{}}\n"),
                Dx9Compare::LessOrEqual,
                true,
            ),
        ),
        (
            Dx9Layer::ShadowMask,
            pass(
                Dx9VertexStage::ShadowMask,
                fill(
                    include_str!("shaders/shadow_mask.frag"),
                    &[
                        ("version", &PASS_GLSL_VERSION),
                        ("bias", &format!("{SHADOW_BIAS:.4}")),
                    ],
                ),
                Dx9Compare::Equal,
                false,
            ),
        ),
    ]
}
