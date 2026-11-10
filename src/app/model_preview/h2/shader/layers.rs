//! Halo 2's names for the dx9 layers: each layer's `layer` option in a
//! shader template, and the implementation an object draws on it.

use super::super::super::dx9::passes::Dx9Layer;

impl Dx9Layer {
    /// The tag's `layer` option.
    pub(in crate::app) fn tag_name(self) -> &'static str {
        match self {
            Self::Texaccum => TEXACCUM_LAYER,
            Self::LightAlbedo => "light albedo",
            Self::Lightmap | Self::ObjectLightmap => LIGHTMAP_LAYER,
            Self::Prt => "sh prt",
            Self::Environment => "environment map",
            Self::SelfIllumination => "self-illumination",
            Self::Overlay => "overlay",
            Self::LightDiffuse
            | Self::LightDiffuseHigh
            | Self::LightDiffuseLow
            | Self::LightDiffuseVeryLow
            | Self::LightGel
            | Self::LightFrustum => "diffuse",
            Self::LightSpecular => "specular",
            Self::Hologram => "hologram",
            Self::ActiveCamo => "active camo",
            Self::Bloom => "bloom",
            Self::GelMask
            | Self::ShadowCaster
            | Self::ShadowMask
            | Self::ShadowApply
            | Self::ShieldDamaged
            | Self::Overshield
            | Self::HologramBloom => "",
            Self::Transparent => TRANSPARENT_LAYER,
        }
    }
    /// The layer drawing a light of this `diffuse contrast`, by its option's
    /// name, which `rasterizer_get_switch_extern` hands the pass as
    /// `light_diffuse_contrast`.
    pub(crate) fn diffuse_with_contrast(contrast: Option<&str>) -> Self {
        match contrast {
            Some("high") => Self::LightDiffuseHigh,
            Some("low") => Self::LightDiffuseLow,
            Some("very low") => Self::LightDiffuseVeryLow,
            _ => Self::LightDiffuse,
        }
    }
    /// What this layer sets `light_diffuse_contrast` to.
    pub(in crate::app) fn diffuse_contrast(self) -> i32 {
        match self {
            Self::LightDiffuseHigh => 1,
            Self::LightDiffuseLow => 2,
            Self::LightDiffuseVeryLow => 3,
            _ => 0,
        }
    }
    /// An object's implementation on this layer. Only the lightmap layer has
    /// a choice to make; see `rasterizer_shader_get_implementation_index`.
    pub(in crate::app) fn implementation(self) -> usize {
        match self {
            Self::Lightmap => H2_FALLBACK_LIGHTMAP_IMPLEMENTATION,
            Self::ObjectLightmap => H2_OBJECT_LIGHTMAP_IMPLEMENTATION,
            Self::LightDiffuse
            | Self::LightDiffuseHigh
            | Self::LightDiffuseLow
            | Self::LightDiffuseVeryLow => H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION,
            Self::LightGel => H2_GEL_LIGHT_DIFFUSE_IMPLEMENTATION,
            Self::ActiveCamo => H2_FULL_ACTIVE_CAMO_IMPLEMENTATION,
            _ => 0,
        }
    }
}

const TRANSPARENT_LAYER: &str = "transparent";
const LIGHTMAP_LAYER: &str = "lightmap (indirect)";
const TEXACCUM_LAYER: &str = "texaccum";
/// What `render_visible_section_get_lightmap_mode` picks when the level has
/// no lightmaps: fixed lighting, with the default lights doing the rest.
const H2_FALLBACK_LIGHTMAP_IMPLEMENTATION: usize = 3;
/// What it picks for a model in a level with lightmaps. This pass reads the
/// object's lighting and takes its reflection from the object's brightness.
const H2_OBJECT_LIGHTMAP_IMPLEMENTATION: usize = 2;
/// "amount=1 (no blending, requires no opaque render)".
const H2_FULL_ACTIVE_CAMO_IMPLEMENTATION: usize = 1;
/// `render_light_get_diffuse_implementation_index` for a light of type sphere.
const H2_SPHERE_LIGHT_DIFFUSE_IMPLEMENTATION: usize = 1;
/// The same for a frustum with neither specular nor a shadow.
const H2_GEL_LIGHT_DIFFUSE_IMPLEMENTATION: usize = 3;
