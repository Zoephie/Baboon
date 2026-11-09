//! The texture slots a material binds, and their parameters.


/// The texture roles the preview understands.
///
/// Deliberately the maps that describe the *surface* — its colour and its shape
/// — and not the ones describing how it answers light. Specular, self
/// illumination, fresnel and environment reflection were all tried: without a
/// scene to light against, each needed invented lighting and a strength control
/// to look like anything, which is a great deal of machinery and guesswork for
/// a preview. These four are read straight off the tag and need none of it.
///
/// The order is the order the fragment shader binds them in, and
/// [`MaterialTextures::slots`] is indexed by `as usize`, so these discriminants
/// are load-bearing: 0-3 reach the shader as one `vec4` of flags, 4-5 as the
/// next.
///
/// `Multipurpose` is Halo CE's alone: its model shaders mask the detail map by
/// one of that map's channels (`fx/model_common.h`). No render method or Halo 2
/// shader names it, so it is absent from [`SLOT_PARAMETERS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextureSlot {
    Base,
    Detail,
    Bump,
    BumpDetail,
    AlphaTest,
    Multipurpose,
}

pub(crate) const SLOT_COUNT: usize = 6;

/// Every slot, in binding order.
pub(crate) const ALL_SLOTS: [TextureSlot; SLOT_COUNT] = [
    TextureSlot::Base,
    TextureSlot::Detail,
    TextureSlot::Bump,
    TextureSlot::BumpDetail,
    TextureSlot::AlphaTest,
    TextureSlot::Multipurpose,
];

/// Slot → the Bungie parameter name that carries it.
///
/// These names were checked against 50 shipped shaders across H3EK and HREK
/// rather than assumed: `base_map` appears in 25/25 of each, and every name
/// here is spelled identically in both games, so one table serves both. A
/// shader that does not declare a slot simply leaves it unbound — Reach's
/// `brute_captain_armor` has no `detail_map` at all, which is normal.
///
/// `alpha_test_map` earns its place on correctness rather than looks: 14 of 25
/// shipped Halo 3 shaders use it, and ignoring it draws solid quads where the
/// cutouts belong.
pub(crate) const SLOT_PARAMETERS: [(TextureSlot, &str); 5] = [
    (TextureSlot::Base, "base_map"),
    (TextureSlot::Detail, "detail_map"),
    (TextureSlot::Bump, "bump_map"),
    (TextureSlot::BumpDetail, "bump_detail_map"),
    (TextureSlot::AlphaTest, "alpha_test_map"),
];

/// Longest edge a preview texture is decoded to.
///
/// Shipped base maps are commonly 2048², and a model carries one per slot per
/// material. The preview draws at a few hundred points, so the full mip costs
/// GPU memory and decode time for detail no one can see.
pub(crate) const MAX_TEXTURE_EDGE: u32 = 1024;
