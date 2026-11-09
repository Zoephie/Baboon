//! What each Halo 3 family game's loads do differently: part of its
//! [`GameRules`](super::super::load::GameRules).

use super::super::common::image::ImageTexels;
use super::shaders::{Gen3Clock, Gen3Pass, RenderMethodDialect};
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
}

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
    };
}
