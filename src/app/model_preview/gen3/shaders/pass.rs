//! A translated pass: its programs' sources, its varyings and its blend.

use super::*;

/// `default_vs` of `screen.fx`, for a triangle that covers the picture.
pub(in crate::app::model_preview) const SCREEN_VERTEX: &str = include_str!("screen.vert");

/// What a pass names, whichever stage reads it.
#[derive(Debug)]
pub(crate) struct Gen3Pass {
    /// Over the whole picture, with no geometry: a `shader_screen`.
    pub(crate) fullscreen: bool,
    pub(crate) vertex: String,
    pub(crate) pixel: String,
    pub(crate) uniforms: Vec<(String, Type)>,
    pub(crate) textures: Vec<(String, TextureKind)>,
}

/// The game's `Nodes[70]` holds one mesh's node map, uploaded a mesh at a time.
/// The preview's vertices name the skeleton's own nodes, all at once, with the
/// last kept spare.
const H3_NODES: usize = MAX_PREVIEW_BONES + 1;

/// A skinned vertex without weights rides the spare node, which holds still.
/// Direct3D lets a stage write less of a semantic than the next reads, as
/// `cortana.fx`'s `static_sh_vs` does with `COLOR1`, the rest reading 0; GLSL
/// will not link that. The vertex stage writes them all instead.
fn widen_varyings(vertex: &str, pixel: &str) -> String {
    let mut source = vertex.to_owned();
    for line in vertex.lines() {
        let Some((width, name)) = line
            .strip_prefix("out vec")
            .and_then(|rest| rest.split_once(' '))
        else {
            continue;
        };
        let name = name.trim_end_matches(';');
        let Ok(width) = width.parse::<usize>() else {
            continue;
        };
        let wanted = (2..=4).find(|wider| {
            *wider > width
                && pixel
                    .lines()
                    .any(|line| line == format!("in vec{wider} {name};"))
        });
        let Some(wider) = wanted else {
            continue;
        };
        let zeros = ", 0.0".repeat(wider - width);
        source = source.replace(line, &format!("out vec{wider} {name};"));
        let prefix = format!("{name} = ");
        let widened: Vec<String> = source
            .lines()
            .map(|line| {
                match line
                    .trim_start()
                    .strip_prefix(&prefix)
                    .and_then(|value| value.strip_suffix(';'))
                {
                    Some(value) => format!("    {name} = vec{wider}({value}{zeros});"),
                    None => line.to_owned(),
                }
            })
            .collect();
        source = widened.join("\n");
    }
    source
}

pub(crate) fn pass_of(vertex: Translated, pixel: Translated) -> Gen3Pass {
    let mut uniforms = vertex.uniforms;
    // Halo 4 calls them `vs_model_skinning_matrices`, the same shape.
    let nodes = uniforms.iter_mut().find_map(|(name, ty)| match ty {
        Type::Array(_, Some(nodes)) if name == "Nodes" || name == "vs_model_skinning_matrices" => {
            Some(nodes)
        }
        _ => None,
    });
    let spare = nodes.map(|nodes| {
        *nodes = H3_NODES;
        H3_NODES - 1
    });
    for uniform in pixel.uniforms {
        if !uniforms.iter().any(|known| known.0 == uniform.0) {
            uniforms.push(uniform);
        }
    }
    let mut textures = vertex.textures;
    for texture in pixel.textures {
        if !textures.iter().any(|known| known.0 == texture.0) {
            textures.push(texture);
        }
    }
    // Flat in the source, three rows to a node.
    // The albedo pass lays depth that later passes test equal against, from
    // other shaders' arithmetic: Direct3D's `precise` positions, in GLSL terms.
    let invariant = vertex
        .source
        .replacen('\n', "\ninvariant gl_Position;\n", 1);
    let source = widen_varyings(&invariant, &pixel.source)
        .replace("Nodes[210]", &format!("Nodes[{}]", H3_NODES * 3))
        .replace(
            "vs_model_skinning_matrices[210]",
            &format!("vs_model_skinning_matrices[{}]", H3_NODES * 3),
        );
    let source = source.replace(
        "hlsl_p0.node_weights = a_BLENDWEIGHT;",
        "hlsl_p0.node_weights = a_BLENDWEIGHT;
    if (dot(a_BLENDWEIGHT, vec4(1.0)) <= 0.0) {
        hlsl_p0.node_weights = vec4(1.0, 0.0, 0.0, 0.0);
        hlsl_p0.node_indices = uvec4(H3_STILL_NODE);
    }",
    );
    let source = source.replace("H3_STILL_NODE", &format!("{}u", spare.unwrap_or(0)));
    // A vertex's occlusion ratio divides by the light's response to its
    // normal. Level probes keep that well above its floor, but the default
    // probe is lit nearly from one side, so on knobbly geometry a ratio of 100
    // interpolates across pixels that are properly lit. Occlusion only dims.
    let source = source.replace(
        "v_TEXCOORD7 = hlsl_result.prt_ravi_diff;",
        "v_TEXCOORD7 = hlsl_result.prt_ravi_diff;\n    v_TEXCOORD7.x = min(v_TEXCOORD7.x, 1.0);",
    );
    // Reach keeps the binormal's handedness in the position's w, which the
    // preview's three-component positions leave at 1: every mirrored half
    // lit with the wrong binormal. The preview carries the authored
    // binormal, so its side of the normal and tangent gives the sign.
    const REACH_BINORMAL_SCALE: &str =
        "float l_binormal_scale = ((l_vertex.position.w * 2.) - 1.);";
    let source = if source.contains(REACH_BINORMAL_SCALE) {
        // Functions come before the inputs they would read, so `main` works
        // the sign out into a global first.
        let source = source
            .replace(REACH_BINORMAL_SCALE, "float l_binormal_scale = g_reach_binormal_sign;")
            .replacen("void main() {", "void main() {\n    g_reach_binormal_sign = (dot(cross(a_NORMAL, a_TANGENT), a_BINORMAL) < 0.0) ? -1.0 : 1.0;", 1);
        let source = if source.contains("in vec3 a_BINORMAL;") {
            source
        } else {
            source.replacen(
                "in vec3 a_NORMAL;",
                "in vec3 a_NORMAL;\nin vec3 a_BINORMAL;",
                1,
            )
        };
        let (version, rest) = source.split_once('\n').unwrap_or((&source, ""));
        format!("{version}\nfloat g_reach_binormal_sign = 1.0;\n{rest}")
    } else {
        source
    };
    // Halo 4 keeps the binormal's handedness in the tangent's w
    // (`generate_binormal(normal, tangent, vertex.tangent.w)`), which the
    // preview's three-component tangents leave at 1. As for Reach, the
    // authored binormal's side of the normal and tangent gives it.
    const H4_TANGENT: &str = "hlsl_p0.tangent = a_TANGENT;";
    let source = if source.contains(H4_TANGENT) && source.contains("in vec4 a_NORMAL;") {
        let source = source.replace(
            H4_TANGENT,
            "hlsl_p0.tangent = vec4(a_TANGENT.xyz, (dot(cross(a_NORMAL.xyz, a_TANGENT.xyz), a_BINORMAL) < 0.0) ? -1.0 : 1.0);",
        );
        if source.contains("in vec3 a_BINORMAL;") {
            source
        } else {
            source.replacen(
                "in vec4 a_NORMAL;",
                "in vec4 a_NORMAL;\nin vec3 a_BINORMAL;",
                1,
            )
        }
    } else {
        source
    };
    // Halo 4's lit passes `Load` the shadow mask at their pixel. With no level
    // it is white everywhere, which a texel bound smaller than the picture
    // stands for once the read stays inside it.
    let pixel_source = pixel.source.replace(
        "texelFetch(l_shadowMask, l_screen_texcoord.xy, l_screen_texcoord.z)",
        "texelFetch(l_shadowMask, min(l_screen_texcoord.xy, textureSize(l_shadowMask, 0) - 1), 0)",
    );
    // The game's camo reads the frame ahead of its tone curve; the preview's
    // copy of what lies behind has been through it. This is that curve undone,
    // the root of `1.5x - 0.5x^3`.
    let pixel_source = pixel_source.replace(
        "vec3 l_true_scene_color = l_ldr_color.xyz;",
        "vec3 l_true_scene_color = 2.0 * sin(asin(clamp(l_ldr_color.xyz, 0.0, 1.0)) / 3.0);",
    );
    Gen3Pass {
        fullscreen: false,
        vertex: source,
        pixel: pixel_source,
        uniforms,
        textures,
    }
}

/// `global_render_method`'s `blend_mode`, in its option order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gen3Blend {
    Opaque,
    Additive,
    Multiply,
    AlphaBlend,
    DoubleMultiply,
    PreMultiplied,
    Maximum,
    MultiplyAdd,
    AddSrcTimesDstAlpha,
    AddSrcTimesSrcAlpha,
    InvAlphaBlend,
    MotionBlurStatic,
    MotionBlurInhibit,
    AddSrcTimesSrcAlphaAccumAlpha,
}

impl Gen3Blend {
    /// In `BLEND_MODES`' order.
    pub(super) const ALL: [Self; BLEND_MODES.len()] = [
        Self::Opaque,
        Self::Additive,
        Self::Multiply,
        Self::AlphaBlend,
        Self::DoubleMultiply,
        Self::PreMultiplied,
        Self::Maximum,
        Self::MultiplyAdd,
        Self::AddSrcTimesDstAlpha,
        Self::AddSrcTimesSrcAlpha,
        Self::InvAlphaBlend,
        Self::MotionBlurStatic,
        Self::MotionBlurInhibit,
        Self::AddSrcTimesSrcAlphaAccumAlpha,
    ];
}

#[cfg(test)]
mod varying_tests {
    use super::widen_varyings;

    #[test]
    fn a_vertex_stage_writes_as_much_as_the_pixel_stage_reads() {
        let vertex = "out vec3 v_COLOR1;\nout vec2 v_TEXCOORD0;\nvoid main() {\n    v_COLOR1 = a.xyz;\n    v_TEXCOORD0 = b;\n}";
        let pixel = "in vec4 v_COLOR1;\nin vec2 v_TEXCOORD0;";
        let widened = widen_varyings(vertex, pixel);
        assert!(widened.contains("out vec4 v_COLOR1;"), "{widened}");
        assert!(
            widened.contains("v_COLOR1 = vec4(a.xyz, 0.0);"),
            "{widened}"
        );
        assert!(widened.contains("out vec2 v_TEXCOORD0;"));
        assert!(widened.contains("v_TEXCOORD0 = b;"));
        assert_eq!(
            widen_varyings(vertex, "in vec3 v_COLOR1;\nin vec2 v_TEXCOORD0;"),
            vertex
        );
    }
}
