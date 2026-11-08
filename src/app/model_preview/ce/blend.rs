//! Halo CE's framebuffer blend modes, which effects, particle systems, contrails and widgets draw with.

use blam_tags::TagStruct;
use super::super::dx9::passes::{
    H2Blend, H2BlendEquation, H2BlendFactor,
};

/// A Halo CE `framebuffer blend function`, by name. A number the list does
/// not name blends as alpha blend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FramebufferBlend {
    #[default]
    AlphaBlend,
    Multiply,
    DoubleMultiply,
    Add,
    Subtract,
    ComponentMin,
    ComponentMax,
    AlphaMultiplyAdd,
}

impl FramebufferBlend {
    /// In the list's order, which [`Self::shader_number`] keeps.
    const ALL: [Self; 8] = [
        Self::AlphaBlend,
        Self::Multiply,
        Self::DoubleMultiply,
        Self::Add,
        Self::Subtract,
        Self::ComponentMin,
        Self::ComponentMax,
        Self::AlphaMultiplyAdd,
    ];

    /// `holder`'s `framebuffer blend function`.
    pub(in crate::app::model_preview) fn read(holder: TagStruct<'_>) -> Self {
        let held = holder.read_enum_option("framebuffer blend function");
        let at = BLEND_FUNCTIONS.iter().position(|name| held.as_deref() == Some(*name));
        at.map_or(Self::AlphaBlend, |at| Self::ALL[at])
    }

    /// Its place in the list: what the effect shader's `blend` and the
    /// shader passes' stage macros switch on.
    pub(in crate::app::model_preview) fn shader_number(self) -> i32 {
        Self::ALL.iter().position(|blend| *blend == self).unwrap_or(0) as i32
    }

    /// `srcblend_table`, `destblend_table` and `blendop_table`.
    pub(in crate::app::model_preview) fn gl(self) -> H2Blend {
        use H2BlendFactor::{DstColor, One, OneMinusSrcAlpha, SrcAlpha, SrcColor, Zero};
        let (equation, source, destination) = match self {
            Self::Multiply => (H2BlendEquation::Add, DstColor, Zero),
            Self::DoubleMultiply => (H2BlendEquation::Add, DstColor, SrcColor),
            Self::Add => (H2BlendEquation::Add, One, One),
            Self::Subtract => (H2BlendEquation::ReverseSubtract, One, One),
            Self::ComponentMin => (H2BlendEquation::Min, One, One),
            Self::ComponentMax => (H2BlendEquation::Max, One, One),
            Self::AlphaMultiplyAdd => (H2BlendEquation::Add, One, OneMinusSrcAlpha),
            Self::AlphaBlend => (H2BlendEquation::Add, SrcAlpha, OneMinusSrcAlpha),
        };
        H2Blend { equation, source, destination, constant: [0; 4] }
    }
}

const BLEND_FUNCTIONS: [&str; 8] = [
    "alpha blend",
    "multiply",
    "double multiply",
    "add",
    "subtract",
    "component min",
    "component max",
    "alpha-multiply add",
];

pub(super) const FADE_MODES: [&str; 3] = ["none", "fade when perpendicular", "fade when parallel"];
