//! Every dx9 vertex stage, Halo 2's and Halo CE's, and the texture rows they share.

/// Passes keep values up to register 9 of the vertex shader's bank.
pub(in crate::app) const H2_TEXTURE_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum Dx9VertexStage {
    /// All four texture coordinates through their transforms.
    Default,
    /// `tc0` is the reflection vector. The sky variant only pins the result
    /// to the far plane, which a lone model in the preview does not need.
    Reflection,
    /// The surface pushed along its normal, with 3D coordinates from position.
    Offset,
    /// Tangent space in `tc1..3`, the vector to the eye in their `w`.
    Bumped,
    /// An object's lightmap pass. Every variant has its base coordinates in
    /// `tc0`, the bump map's in `tc2`, the direct light in tangent space in
    /// `tc3`, and the eye and indirect light packed into the two colors. They
    /// differ in the rest.
    Lightmap,
    /// `tc1` is the reflection vector.
    LightmapEnv,
    /// Adds a mask at `tc4`.
    LightmapMark,
    /// Reflection in `tc1`, detail at `tc4`, two masks at `tc5` and `tc6`.
    LightmapEnvStencil,
    /// Masks at `tc4` and `tc5`.
    LightmapMarkStencil,
    /// `tc1` is the tangent-space eye vector, otherwise as `LightmapEnvStencil`.
    LightmapStencil,
    /// Reads the texaccum target back: `tc0` is the pixel's screen position
    /// and `tc1` the reflection vector.
    LightmapTarget,
    /// `LightmapTarget` with the tangent-space eye vector in `tc1`.
    LightmapTargetEye,
    /// `tc1` is the tangent-space eye vector, and only base and bump are read.
    LightmapEye,
    /// `LightmapEye` with a chained map at `tc4` and a third at `tc5`.
    LightmapEyeBlend,
    /// A bump at `tc0`, and in `tc1` and `tc2` the rows that turn it into a
    /// bent screen position to read the frame behind at.
    ActiveCamo,
    /// A surface looking down at the frame behind it, by world position. The
    /// game ripples this by registers no tag holds, so it stands still here.
    Water,
    /// The model as its key light sees it, for depth alone.
    ShadowCaster,
    /// The model in view, with its place on the shadow map in `tc0`.
    ShadowMask,
    /// One map at `tc0`.
    TextureCamera,
    /// The frame behind it at `tc0`, and three maps laid out across the view.
    Hologram,
    /// Precomputed radiance transfer, reading the texaccum target at `tc0`.
    Prt,
    /// `Prt` with its own base at `tc0` and a detail map at `tc4`.
    PrtDetail,
    /// Fills the texaccum target: base, detail and two masks.
    Texaccum,
    /// Self-illumination: four coordinates off one base transform.
    Emissive,
    /// A reflection without a bump basis: the eye in `tc2`, reflection in `tc3`.
    Environment,
    /// A sphere light's diffuse pass: the light in tangent space in `tc1`, and
    /// the position within its falloff volume in `tc3`.
    LightDiffuse,
    /// `LightDiffuse` reading its albedo from the texaccum target.
    LightDiffuseTarget,
    /// `LightDiffuse` with a detail map at `tc4`.
    LightDiffuseDetail,
    /// A frustum light's diffuse pass: the light in tangent space in `tc1`, the
    /// gel's projection in `tc3`, its falloff in one color and near plane in
    /// the other.
    LightGel,
    /// `LightGel` reading its albedo from the texaccum target.
    LightGelTarget,
    /// `LightGel` with a detail map at `tc4`.
    LightGelDetail,
    /// A sphere light's specular pass: light and eye in tangent space.
    LightSpecular,
    /// A frustum light's diffuse and specular together, through the frame's
    /// alpha: light and eye in tangent space, the falloff in a color.
    LightFrustum,
    /// `LightFrustum` reading its albedo from the texaccum target.
    LightFrustumTarget,
    /// `LightFrustum` with a detail map at `tc4`.
    LightFrustumDetail,
    /// Halo CE's `model.vsh` for a lone object: the two distant lights and
    /// ambient of `CeModelRows`, and the object's point lights, with no fog.
    CeModel,
    /// Halo CE's `transparent_generic`: four maps through their transforms,
    /// the fades in the colors.
    CeTransparent,
    /// With the first map a cube, by the reflection.
    CeTransparentReflection,
    /// By the normal.
    CeTransparentObject,
    /// By the vector to the eye.
    CeTransparentViewer,
    /// With the first map laid across the view.
    CeTransparentScreen,
    /// Halo CE's glass: two maps by scale, lit as `CeModel` is.
    CeGlassLit,
    /// A reflection with no bump map to bend it.
    CeGlassFlat,
    /// One map, as it lies.
    CeMeter,
    /// Water's base map, and the eye in tangent space.
    CeWaterOpacity,
    /// Two 3D noise lookups by position, pushed out along the normal.
    CePlasma,
    /// `model_active_camouflage.vsh`: the view-space normal, and how far the
    /// surface bends the frame behind it at this distance.
    CeActiveCamo,
}
