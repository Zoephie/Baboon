//! A decoded map of any shape a pass samples, what its texels mean to the
//! card, and how it is sampled: shared by every game's drawing.

use super::super::materials::TextureImage;
use blam_tags::bitmap::BitmapFormat;
use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

/// A decoded map of any shape a pass samples.
#[derive(Debug, Clone)]
pub(crate) enum Image {
    Flat(TextureImage),
    /// Six RGBA faces of `size` texels a side, in +X -X +Y -Y +Z -Z order.
    Cube {
        size: usize,
        faces: [Vec<u8>; 6],
    },
    Volume {
        width: usize,
        height: usize,
        depth: usize,
        rgba: Vec<u8>,
    },
}

impl Image {
    /// Whether its pixels fill its size, which is how much GL reads.
    pub(crate) fn whole(&self) -> bool {
        use super::super::materials::rgba_fills;
        match self {
            Self::Flat(image) => image.whole(),
            Self::Cube { size, faces } => {
                faces.iter().all(|face| rgba_fills(face, &[*size, *size]))
            }
            Self::Volume {
                width,
                height,
                depth,
                rgba,
            } => rgba_fills(rgba, &[*width, *height, *depth]),
        }
    }
}

/// A texture's address and filter modes, from the bake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sampler {
    pub(crate) address: [BitmapAddressMode; 2],
    pub(crate) filter: BitmapFilterMode,
}

/// What a texel means to the card, from the bitmap's format and curve as
/// `bitmap_format_to_hardware_format_pc_unchecked` chooses its DXGI format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageTexels {
    Linear,
    /// The card decodes the `sRGB` curve as it samples.
    Srgb,
    /// `_SNORM`: the shaders read -1..1 directly, and DirectX 11's
    /// `BUMP_CONVERT` does nothing to them.
    Signed,
}

impl ImageTexels {
    pub(crate) fn of(format: BitmapFormat, srgb_curve: bool) -> Self {
        match format {
            BitmapFormat::DxnSnorm
            | BitmapFormat::V8u8
            | BitmapFormat::Q8w8v8u8
            | BitmapFormat::V16u16 => Self::Signed,
            _ if srgb_curve => Self::Srgb,
            _ => Self::Linear,
        }
    }

    /// Halo 4's MCC upload (`halo4_tag_test` 0x1408D93B0, sRGB views
    /// 0x1408D9510). Only the five formats with an sRGB twin can read
    /// sRGB, when the curve is xRGB, sRGB or unknown; `dxn` is BC5_SNORM.
    pub(crate) fn of_halo4(format: BitmapFormat, curve: i128) -> Self {
        match format {
            BitmapFormat::Dxn
            | BitmapFormat::DxnSnorm
            | BitmapFormat::V8u8
            | BitmapFormat::Q8w8v8u8
            | BitmapFormat::V16u16
            | BitmapFormat::Signedr16g16b16a16 => Self::Signed,
            BitmapFormat::X8r8g8b8
            | BitmapFormat::A8r8g8b8
            | BitmapFormat::Dxt1
            | BitmapFormat::Dxt3
            | BitmapFormat::Dxt5
                if matches!(curve, 0 | 1 | 5) =>
            {
                Self::Srgb
            }
            _ => Self::Linear,
        }
    }

    /// `bitmap_format_is_default_srgb`, for a bitmap whose curve is unknown.
    pub(crate) fn default_srgb(format: BitmapFormat) -> bool {
        !matches!(
            format,
            BitmapFormat::SoftwareRgbfp32
                | BitmapFormat::V8u8
                | BitmapFormat::Abgrfp32
                | BitmapFormat::Abgrfp16
                | BitmapFormat::Q8w8v8u8
                | BitmapFormat::A2r10g10b10
                | BitmapFormat::A16b16g16r16
                | BitmapFormat::V16u16
                | BitmapFormat::Dxn
                | BitmapFormat::DxnSnorm
                | BitmapFormat::Ctx1
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image short of its size is not whole: a cube is square, so a face
    /// as wide as the image but shorter would be read past its end.
    #[test]
    fn an_image_short_of_its_size_is_not_whole() {
        let face = |side: usize| vec![0u8; side * side * 4];
        let cube = |last: Vec<u8>| Image::Cube {
            size: 4,
            faces: [face(4), face(4), face(4), face(4), face(4), last],
        };
        assert!(cube(face(4)).whole());
        assert!(!cube(vec![0; 4 * 2 * 4]).whole());
        let volume = |bytes: usize| Image::Volume {
            width: 2,
            height: 2,
            depth: 3,
            rgba: vec![0; bytes],
        };
        assert!(volume(48).whole());
        assert!(!volume(47).whole());
        let huge = Image::Volume {
            width: usize::MAX,
            height: 2,
            depth: 1,
            rgba: Vec::new(),
        };
        assert!(
            !huge.whole(),
            "a size past the address space is never whole"
        );
    }
}
