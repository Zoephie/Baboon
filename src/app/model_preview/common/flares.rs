//! What a Halo CE or Halo 2 flare hands the sprite pass: where it tests its
//! occlusion from, and the quad it draws. Gen3's occlusion offset is its
//! own (`gen3::effects::flares`), with a different default.

use blam_tags::TagStruct;

/// A flare's `occlusion offset direction`: where it tests from, beside its
/// marker. A number with no name tests from the marker, as `none` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum OcclusionOffset {
    TowardViewer,
    MarkerForward,
    None,
}

impl OcclusionOffset {
    pub(in crate::app::model_preview) fn read(root: TagStruct<'_>) -> Self {
        match root
            .read_enum_option("occlusion offset direction")
            .as_deref()
        {
            Some("toward viewer") => Self::TowardViewer,
            Some("marker forward") => Self::MarkerForward,
            _ => Self::None,
        }
    }
}

/// One quad, ready to place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlareSprite {
    pub(crate) center: [f32; 3],
    /// Half its width and height, in world units.
    pub(crate) extent: [f32; 2],
    /// Radians, about the view axis.
    pub(crate) rotation: f32,
    pub(crate) color: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) image: usize,
    /// Halo 2: how strongly the map's brightest parts run to white.
    pub(crate) highlight: Option<f32>,
    /// Halo 3: how the map's own color adds, and the power that takes it to.
    pub(crate) tint: Option<[f32; 2]>,
    /// Halo 2: whether the world's depth hides it.
    pub(crate) depth_tested: bool,
}
