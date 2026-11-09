//! The self-shadow passes, and the shadow apply that Halo 3 shares.

use super::*;
use crate::app::model_preview::gen3::shadow::{H3_SHADOW_APPLY, H3_SHADOW_APPLY_BIAS};

/// The passes behind the preview's own key light shadow. Under the default
/// lights the game draws no shadows at all: layers 10 and 11 only run for a
/// light object that casts them.
pub(crate) fn h2_shadow_passes() -> [(Dx9Layer, Dx9DrawPass); 4] {
    let pass = |stage, fragment: String, compare, write| Dx9DrawPass {
        stage,
        fragment,
        blend: None,
        depth_bias: 0.0,
        depth: Dx9Depth { compare: Some(compare), write },
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
    };
    [
        (
            Dx9Layer::ShadowCaster,
            pass(
                Dx9VertexStage::ShadowCaster,
                format!("{H2_GLSL_VERSION}void main() {{}}\n"),
                Dx9Compare::LessOrEqual,
                true,
            ),
        ),
        (
            Dx9Layer::ShadowMask,
            pass(
                Dx9VertexStage::ShadowMask,
                format!(
                    "{H2_GLSL_VERSION}in vec4 v_tc0;
uniform sampler2D u_shadow_map;
void main() {{
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    if (place.z - {H2_SHADOW_BIAS:.4} <= texture(u_shadow_map, place.xy).r) {{
        discard;
    }}
}}
"
                ),
                Dx9Compare::Equal,
                false,
            ),
        ),
        (
            Dx9Layer::GelMask,
            Dx9DrawPass {
                channels: [false, false, false, true],
                ..pass(
                    Dx9VertexStage::LightGel,
                    format!(
                        "{H2_GLSL_VERSION}in vec4 v_tc3;
in vec4 v_color1;
uniform sampler2D u_gel_map;
out vec4 color;
void main() {{
    color = vec4(1.0, 1.0, 1.0, v_color1.r > 0.5 ? textureProj(u_gel_map, v_tc3).b : 0.0);
}}
"
                    ),
                    Dx9Compare::Equal,
                    false,
                )
            },
        ),
        (
            Dx9Layer::ShadowApply,
            Dx9DrawPass {
                depth_bias: H3_SHADOW_APPLY_BIAS,
                // The accumulation's alpha is what covers the background.
                channels: [true, true, true, false],
                blend: Some(Dx9Blend {
                    equation: Dx9BlendEquation::Add,
                    source: Dx9BlendFactor::Zero,
                    destination: Dx9BlendFactor::SrcAlpha,
                    constant: [0; 4],
                }),
                ..pass(
                    Dx9VertexStage::ShadowMask,
                    format!("{H2_GLSL_VERSION}{H3_SHADOW_APPLY}"),
                    // The lit pass's depth is another program's: near, not equal.
                    Dx9Compare::LessOrEqual,
                    false,
                )
            },
        ),
    ]
}
