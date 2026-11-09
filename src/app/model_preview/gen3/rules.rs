//! What each Halo 3 family game's loads do differently: part of its
//! [`GameRules`](super::super::load::GameRules).

use super::super::TagReader;
use super::super::common::image::ImageTexels;
use super::effects::light_volumes::Gen3VolumeDefinition;
use super::shaders::{Gen3Clock, Gen3Pass, RenderMethodDialect};
use blam_tags::TagStruct;
use blam_tags::bitmap::BitmapFormat;

/// One game's gen3 choices, flag by flag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gen3Rules {
    /// Whose tool lays out a render method's sources.
    pub(crate) dialect: RenderMethodDialect,
    /// Whether a render method takes its definition's version update and
    /// its reference shader before it resolves, as Reach's tool does.
    pub(crate) reference_shaders: bool,
    /// The clock a material's animated parameters run by, and a screen
    /// shader's.
    pub(crate) clock: Gen3Clock,
    pub(crate) screen_clock: Gen3Clock,
    /// Reach's `select_mesh_entry_point_for_pass` (MCC `sub_14084C910`)
    /// lights what blends in a single pass, by the mesh's policy: one probe,
    /// or the ambient transfer.
    pub(crate) single_pass_entries: bool,
    /// Whether an opaque material compiles its `shadow_generate` pass.
    pub(crate) shadow_generate: bool,
    /// The active camo pass as the preview draws it, where the game's writes
    /// back what it samples (Reach's), which the preview's copy holds
    /// encoded. `None` keeps the pass as compiled.
    pub(crate) camo_pass: Option<fn(&Gen3Pass) -> Option<Gen3Pass>>,
    /// Whether each pass binds the material's environment maps as Reach's do.
    pub(crate) environment_maps: bool,
    /// How a bitmap's format and curve upload, where the game has its own
    /// table (Halo 4's); `None` goes by the format and the curve's sRGB.
    pub(crate) texels: Option<fn(BitmapFormat, i128) -> ImageTexels>,
    /// Whether `y8` and `ay8` upload as R8_UNORM, read in red alone: Halo 4's
    /// (`halo4_tag_test` 0x141B7E080), whose DirectX 11 views have no swizzle.
    pub(crate) luminance_as_red: bool,
    /// Reach's screen effects: shown by `is_visible` (the 360's 0x83033AB0)
    /// to a viewer who is nobody's unit, with exposure deboost and bright and
    /// dark noise, and strict on their delay.
    pub(crate) screen_effect_terms: bool,
    /// ODST's `screen_effect_evaluate_falloff`: on a live object, an effect
    /// older than its `object falloff` field multiplies by a function nothing
    /// fills in, so does not show.
    pub(crate) screen_object_falloff: bool,
    /// Reach's beams and contrails: their sort bias, clock, exposure and step
    /// fade, and for a beam `effects__beam.hlsl`'s `sample_diffuse`
    /// (l.373–458), one texcoord, a `particle_alpha` of 1 and no palette
    /// shift, fading by its range.
    pub(crate) strip_terms: bool,
    /// Reach's light volume postprocess: circular albedo, depth fade, LOD and
    /// precompiled profiles.
    pub(crate) light_volume_terms: bool,
    /// A light volume's definition read the game's own way: Halo 4's, through
    /// its `actual material`.
    pub(crate) light_volume_reader: Option<LightVolumeReader>,
}

/// A light volume system's definition read from its element.
pub(crate) type LightVolumeReader = fn(&TagReader, TagStruct<'_>) -> Option<Gen3VolumeDefinition>;

impl Gen3Rules {
    /// Halo 3's, which the others start from.
    pub(crate) const DEFAULT: Self = Self {
        dialect: RenderMethodDialect::Halo3,
        reference_shaders: false,
        clock: Gen3Clock::Halo3,
        screen_clock: Gen3Clock::Halo3,
        single_pass_entries: false,
        shadow_generate: false,
        camo_pass: None,
        environment_maps: false,
        texels: None,
        luminance_as_red: false,
        screen_effect_terms: false,
        screen_object_falloff: false,
        strip_terms: false,
        light_volume_terms: false,
        light_volume_reader: None,
    };
}
