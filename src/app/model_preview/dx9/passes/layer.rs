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
    /// Not the game's: see `dx9_shadow_passes`.
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
}
