//! Halo 2's stages: chosen by the kit's file names, and their lightmap outputs.

use super::*;

impl Dx9VertexStage {
    /// From a pass's `vertex shader` reference, which ignores case as every
    /// tag reference does.
    pub(in crate::app) fn from_path(path: &str) -> Option<Self> {
        match path
            .rsplit(['\\', '/'])
            .next()?
            .to_ascii_lowercase()
            .as_str()
        {
            "transparent_generic_default" | "transparent_generic_default_sky" => {
                Some(Self::Default)
            }
            "transparent_generic_reflection" | "transparent_generic_reflection_sky" => {
                Some(Self::Reflection)
            }
            "transparent_generic_offset" => Some(Self::Offset),
            "stencil_env_bumped_tangentspace" => Some(Self::Bumped),
            "lightmap_dynamic_tex_detail" => Some(Self::Lightmap),
            "lightmap_dynamic_tex_env" => Some(Self::LightmapEnv),
            "lightmap_dynamic_tex_mark" => Some(Self::LightmapMark),
            "lightmap_dynamic_tex_env_stencil" => Some(Self::LightmapEnvStencil),
            "lightmap_dynamic_tex_mark_stencil" => Some(Self::LightmapMarkStencil),
            "lightmap_dynamic_tex_stencil" => Some(Self::LightmapStencil),
            "lightmap_dynamic_env" => Some(Self::LightmapTarget),
            "lightmap_dynamic" => Some(Self::LightmapTargetEye),
            "hologram" => Some(Self::Hologram),
            "texture_camera" => Some(Self::TextureCamera),
            "water_pool" | "water_pool_static" => Some(Self::Water),
            "transparent_active_camo_viewer_normal" => Some(Self::ActiveCamo),
            "prt" => Some(Self::Prt),
            "prt_lightmap" => Some(Self::PrtDetail),
            "lightmap_dynamic_tex" => Some(Self::LightmapEye),
            "lightmap_dynamic_tex_detail_blend" => Some(Self::LightmapEyeBlend),
            "stencil_texture" => Some(Self::Texaccum),
            "stencil_emissive" => Some(Self::Emissive),
            "stencil_env" => Some(Self::Environment),
            "lightaccum_sphere_diffuse_tex" => Some(Self::LightDiffuse),
            "lightaccum_sphere_diffuse" => Some(Self::LightDiffuseTarget),
            "lightaccum_sphere_diffuse_tex_detail" => Some(Self::LightDiffuseDetail),
            "lightaccum_frustum_no_specular_gel_tex" => Some(Self::LightGel),
            "lightaccum_frustum_no_specular_gel" => Some(Self::LightGelTarget),
            "lightaccum_frustum_no_specular_gel_tex_detail" => Some(Self::LightGelDetail),
            "stencil_light_specular_tangentspace_sphere" => Some(Self::LightSpecular),
            "lightaccum_frustum_tex" => Some(Self::LightFrustum),
            "lightaccum_frustum" => Some(Self::LightFrustumTarget),
            "lightaccum_frustum_tex_detail" => Some(Self::LightFrustumDetail),
            _ => None,
        }
    }

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

    /// Whether texture registers hold a 2D scale and offset, not affine rows.
    pub(in crate::app) fn uses_scale_rows(self) -> bool {
        self.is_lightmap()
            || matches!(
                self,
                Self::Bumped
                    | Self::Emissive
                    | Self::Environment
                    | Self::LightDiffuse
                    | Self::LightDiffuseTarget
                    | Self::LightDiffuseDetail
                    | Self::LightGel
                    | Self::LightGelTarget
                    | Self::LightGelDetail
                    | Self::LightSpecular
                    | Self::LightFrustum
                    | Self::LightFrustumTarget
                    | Self::LightFrustumDetail
                    | Self::ActiveCamo
                    | Self::TextureCamera
                    | Self::Water
                    | Self::Hologram
                    | Self::Prt
                    | Self::PrtDetail
            )
    }

    /// Whether registers also carry lone values rather than only map scales.
    pub(in crate::app) fn takes_row_values(self) -> bool {
        matches!(
            self,
            Self::ActiveCamo | Self::Offset | Self::Prt | Self::PrtDetail
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
