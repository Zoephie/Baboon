//! A decoded map of any shape a pass samples, shared by every game's
//! drawing.

use super::super::materials::TextureImage;

/// A decoded map of any shape a pass samples.
#[derive(Debug, Clone)]
pub(crate) enum H2Image {
    Flat(TextureImage),
    /// Six RGBA faces of `size` texels a side, in +X -X +Y -Y +Z -Z order.
    Cube { size: usize, faces: [Vec<u8>; 6] },
    Volume { width: usize, height: usize, depth: usize, rgba: Vec<u8> },
}

impl H2Image {
    /// Whether its pixels fill its size, which is how much GL reads.
    pub(crate) fn whole(&self) -> bool {
        use super::super::materials::rgba_fills;
        match self {
            Self::Flat(image) => image.whole(),
            Self::Cube { size, faces } => faces.iter().all(|face| rgba_fills(face, &[*size, *size])),
            Self::Volume { width, height, depth, rgba } => rgba_fills(rgba, &[*width, *height, *depth]),
        }
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
        let cube = |last: Vec<u8>| H2Image::Cube { size: 4, faces: [face(4), face(4), face(4), face(4), face(4), last] };
        assert!(cube(face(4)).whole());
        assert!(!cube(vec![0; 4 * 2 * 4]).whole());
        let volume = |bytes: usize| H2Image::Volume { width: 2, height: 2, depth: 3, rgba: vec![0; bytes] };
        assert!(volume(48).whole());
        assert!(!volume(47).whole());
        let huge = H2Image::Volume { width: usize::MAX, height: 2, depth: 1, rgba: Vec::new() };
        assert!(!huge.whole(), "a size past the address space is never whole");
    }
}
