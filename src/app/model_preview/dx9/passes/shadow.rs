//! The self-shadow passes, and the shadow apply that Halo 3 shares.

use super::*;

/// The passes behind the preview's own key light shadow. Under the default
/// lights the game draws no shadows at all: layers 10 and 11 only run for a
/// light object that casts them.
pub(crate) fn h2_shadow_passes() -> [(H2Layer, H2DrawPass); 4] {
    let pass = |stage, fragment: String, compare, write| H2DrawPass {
        stage,
        fragment,
        blend: None,
        depth_bias: 0.0,
        depth: H2Depth { compare: Some(compare), write },
        cull: H2Cull::Frame,
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
            H2Layer::ShadowCaster,
            pass(
                H2VertexStage::ShadowCaster,
                format!("{H2_GLSL_VERSION}void main() {{}}\n"),
                H2Compare::LessOrEqual,
                true,
            ),
        ),
        (
            H2Layer::ShadowMask,
            pass(
                H2VertexStage::ShadowMask,
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
                H2Compare::Equal,
                false,
            ),
        ),
        (
            H2Layer::GelMask,
            H2DrawPass {
                channels: [false, false, false, true],
                ..pass(
                    H2VertexStage::LightGel,
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
                    H2Compare::Equal,
                    false,
                )
            },
        ),
        (
            H2Layer::ShadowApply,
            H2DrawPass {
                depth_bias: H3_SHADOW_APPLY_BIAS,
                // The accumulation's alpha is what covers the background.
                channels: [true, true, true, false],
                blend: Some(H2Blend {
                    equation: H2BlendEquation::Add,
                    source: H2BlendFactor::Zero,
                    destination: H2BlendFactor::SrcAlpha,
                    constant: [0; 4],
                }),
                ..pass(
                    H2VertexStage::ShadowMask,
                    format!("{H2_GLSL_VERSION}{H3_SHADOW_APPLY}"),
                    // The lit pass's depth is another program's: near, not equal.
                    H2Compare::LessOrEqual,
                    false,
                )
            },
        ),
    ]
}

const H3_SHADOW_APPLY_BIAS: f32 = -0.00002;

/// `shadow_apply_bilinear`'s pixel shader in the preview's box: one object
/// filling the map. `FASTER_SHADOWS` at `DX_VERSION` 11 leaves no slope to bias
/// by, that being the shadow generate's own.
const H3_SHADOW_APPLY: &str = "in vec4 v_tc0;
uniform sampler2D u_shadow_map;
uniform sampler2D u_normal_target;
uniform vec3 u_shadow_toward;
uniform float u_shadow_size;
uniform float u_shadow_alpha;
out vec4 color;
float closer(ivec2 texel, ivec2 offset, float depth) {
    return step(depth, texelFetch(u_shadow_map, texel + offset, 0).r);
}
void main() {
    vec3 place = v_tc0.xyz * 0.5 + 0.5;
    vec3 normal = texelFetch(u_normal_target, ivec2(gl_FragCoord.xy), 0).xyz * 2.0 - 1.0;
    float cosine = dot(normal, u_shadow_toward);
    float falloff = clamp(place.z * 2.0 - 1.0, 0.0, 1.0);
    falloff *= falloff;
    float darkness = u_shadow_alpha * (1.0 - falloff * falloff) * clamp(cosine, 0.0, 1.0);
    float darken = 1.0;
    vec2 pixel = place.xy * u_shadow_size;
    if (darkness > 0.001 && all(greaterThan(pixel, vec2(1.0))) && all(lessThan(pixel, vec2(u_shadow_size - 2.0)))) {
        ivec2 t = ivec2(floor(pixel));
        vec2 f = fract(pixel);
        vec2 r = 1.0 - f;
        float lit = r.x * r.y * closer(t, ivec2(-1, -1), place.z)
            + r.y * closer(t, ivec2(0, -1), place.z)
            + f.x * r.y * closer(t, ivec2(1, -1), place.z)
            + r.x * closer(t, ivec2(-1, 0), place.z)
            + closer(t, ivec2(0, 0), place.z)
            + f.x * closer(t, ivec2(1, 0), place.z)
            + r.x * f.y * closer(t, ivec2(-1, 1), place.z)
            + f.y * closer(t, ivec2(0, 1), place.z)
            + f.x * f.y * closer(t, ivec2(1, 1), place.z);
        lit *= 0.25;
        darken = clamp(1.01 - darkness + lit * darkness, 0.0, 1.0);
        darken *= darken;
    }
    color = vec4(0.0, 0.0, 0.0, darken);
}
";
