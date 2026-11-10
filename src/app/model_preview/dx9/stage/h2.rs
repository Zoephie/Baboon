//! Halo 2's stages: chosen by the kit's file names, and their lightmap outputs.

use super::kind::Dx9VertexStage;

impl Dx9VertexStage {
    /// Whether this is one of the lightmap family, whose texture registers
    /// hold a 2D scale and offset rather than affine rows.
    pub(in crate::app) fn is_lightmap(self) -> bool {
        matches!(
            self,
            Self::Lightmap
                | Self::LightmapEnv
                | Self::LightmapMark
                | Self::LightmapEnvStencil
                | Self::LightmapMarkStencil
                | Self::LightmapStencil
                | Self::LightmapTarget
                | Self::LightmapTargetEye
                | Self::LightmapEye
                | Self::LightmapEyeBlend
                | Self::Texaccum
        )
    }

    /// What differs between the lightmap variants.
    pub(super) fn lightmap_outputs(self) -> &'static str {
        const CHAINED: &str = "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;";
        match self {
            Self::LightmapEnv => {
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;"
            }
            Self::LightmapMark => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc4.xy = base * u_texture_rows[3].xy;"
            ),
            Self::LightmapEnvStencil => concat!(
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc6.xy = base * u_texture_rows[3].zw;"
            ),
            Self::LightmapMarkStencil => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc4.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].zw;"
            ),
            Self::LightmapTarget => concat!(
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);\n    ",
                "v_tc1.xyz = normal * dot(normal, to_eye) * u_reflection_scale - to_eye;"
            ),
            Self::LightmapTargetEye => concat!(
                "v_tc0 = vec4((gl_Position.xy + gl_Position.w) * 0.5, gl_Position.ww);\n    ",
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);"
            ),
            Self::LightmapEye => "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);",
            Self::LightmapEyeBlend => concat!(
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;"
            ),
            Self::Texaccum => concat!(
                "v_tc1.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc2.xy = base * u_texture_rows[2].xy;\n    ",
                "v_tc3 = vec4(base * u_texture_rows[2].zw, 0.0, 1.0);"
            ),
            Self::LightmapStencil => concat!(
                "v_tc1.xyz = tangent_space(to_eye, tangent, binormal, normal);\n    ",
                "v_tc4.xy = base * u_texture_rows[1].xy + u_texture_rows[1].zw;\n    ",
                "v_tc5.xy = base * u_texture_rows[3].xy;\n    ",
                "v_tc6.xy = base * u_texture_rows[3].zw;"
            ),
            _ => CHAINED,
        }
    }
}
