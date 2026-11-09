//! The dx9 shader layers a pass draws in, and Halo 2's template layer names and implementations.

/// A layer the preview draws, in the order the game's `render_scene` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dx9Layer {
    Texaccum,
    /// A late fill of the texaccum target, drawn to it per light in the game,
    /// for shaders whose color changes rather than sitting in a base map.
    LightAlbedo,
    /// The fallback for a scene with no lightmaps, which adds the default
    /// lights on top.
    Lightmap,
    /// The object path in a level with lightmaps; see `H2_OBJECT_LIGHTMAP_IMPLEMENTATION`.
    ObjectLightmap,
    /// Precomputed radiance transfer, for the shaders with no lightmap pass.
    Prt,
    Environment,
    SelfIllumination,
    Overlay,
    LightDiffuse,
    /// `LightDiffuse` for a light of another `diffuse contrast`: `high`.
    LightDiffuseHigh,
    /// `low`.
    LightDiffuseLow,
    /// `very low`.
    LightDiffuseVeryLow,
    /// A frustum light's gel pass, drawn once per light.
    LightGel,
    /// A frustum light with specular: its diffuse pass's first implementation.
    LightFrustum,
    /// What `shadow_apply_combined`'s third implementation leaves in the frame's
    /// alpha for `LightFrustum`.
    GelMask,
    LightSpecular,
    Hologram,
    /// Takes over from every other opaque layer while camo is on.
    ActiveCamo,
    Transparent,
    /// A model's `shield damaged shader`, over all of it.
    ShieldDamaged,
    /// A model's `overshield shader`, over all of it.
    Overshield,
    /// A model's `hologram shader`'s bloom passes, over all of it while its
    /// `hologram control function` is above nothing.
    HologramBloom,
    /// A mask written to the frame's alpha, for the glow drawn from it.
    Bloom,
    /// Not the game's: see `h2_shadow_passes`.
    ShadowCaster,
    ShadowMask,
    /// Halo 3's `shadow_apply`, darkening what the lit pass drew.
    ShadowApply,
}

pub(crate) const DX9_LAYER_COUNT: usize = 26;

impl Dx9Layer {
    pub(crate) const ALL: [Self; DX9_LAYER_COUNT] = [
        Self::Texaccum,
        Self::LightAlbedo,
        Self::Lightmap,
        Self::ObjectLightmap,
        Self::Prt,
        Self::Environment,
        Self::SelfIllumination,
        Self::Overlay,
        Self::LightDiffuse,
        Self::LightDiffuseHigh,
        Self::LightDiffuseLow,
        Self::LightDiffuseVeryLow,
        Self::LightGel,
        Self::LightFrustum,
        Self::GelMask,
        Self::LightSpecular,
        Self::Hologram,
        Self::ActiveCamo,
        Self::Transparent,
        Self::ShieldDamaged,
        Self::Overshield,
        Self::HologramBloom,
        Self::Bloom,
        Self::ShadowCaster,
        Self::ShadowMask,
        Self::ShadowApply,
    ];

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
