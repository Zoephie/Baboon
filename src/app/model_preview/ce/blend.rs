//! Halo CE's framebuffer blend modes, which effects, particle systems, contrails and widgets draw with.

use super::super::dx9::passes::{Dx9Blend, Dx9BlendEquation, Dx9BlendFactor};
use blam_tags::TagStruct;

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
        let at = BLEND_FUNCTIONS
            .iter()
            .position(|name| held.as_deref() == Some(*name));
        at.map_or(Self::AlphaBlend, |at| Self::ALL[at])
    }

    /// Its place in the list: what the effect shader's `blend` and the
    /// shader passes' stage macros switch on.
    pub(in crate::app::model_preview) fn shader_number(self) -> i32 {
        Self::ALL
            .iter()
            .position(|blend| *blend == self)
            .unwrap_or(0) as i32
    }

    /// `srcblend_table`, `destblend_table` and `blendop_table`.
    pub(in crate::app::model_preview) fn gl(self) -> Dx9Blend {
        use Dx9BlendFactor::{DstColor, One, OneMinusSrcAlpha, SrcAlpha, SrcColor, Zero};
        let (equation, source, destination) = match self {
            Self::Multiply => (Dx9BlendEquation::Add, DstColor, Zero),
            Self::DoubleMultiply => (Dx9BlendEquation::Add, DstColor, SrcColor),
            Self::Add => (Dx9BlendEquation::Add, One, One),
            Self::Subtract => (Dx9BlendEquation::ReverseSubtract, One, One),
            Self::ComponentMin => (Dx9BlendEquation::Min, One, One),
            Self::ComponentMax => (Dx9BlendEquation::Max, One, One),
            Self::AlphaMultiplyAdd => (Dx9BlendEquation::Add, One, OneMinusSrcAlpha),
            Self::AlphaBlend => (Dx9BlendEquation::Add, SrcAlpha, OneMinusSrcAlpha),
        };
        Dx9Blend {
            equation,
            source,
            destination,
            constant: [0; 4],
        }
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
