//! Halo 2's reading of a pass's vertex shader: the stage the kit's file
//! names, and how its texture registers are laid out.

use super::super::super::dx9::stage::Dx9VertexStage;

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
}
