//! Halo CE's sprite sheets and secondary maps, which effects, particle systems and contrails share.

use super::super::TagReader;
use super::super::common::image::Image;
use super::super::common::view::ViewFrame;
use super::super::materials::decode_image;
use super::shaders::{CeMapAnimation, read_effect_map_animation};
use crate::core::h2_pixel_shader::SamplerKind;
use blam_tags::bitmap::BitmapDefinition;
use blam_tags::{TagFile, TagStruct};
use std::sync::Arc;

/// One sprite of a bitmap's sequence: its rect across the bitmap, the point
/// it hangs from, and the image it is cut from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct CeSpriteFrame {
    pub(in crate::app::model_preview) rect: [f32; 4],
    pub(in crate::app::model_preview) registration: [f32; 2],
    pub(in crate::app::model_preview) image: usize,
}

/// An effect shader's second map: drawn over the sprite where anchored to
/// it or to the screen, or read as depth for a `zsprite`.
#[derive(Debug, Clone)]
pub(crate) struct CeSecondaryMap {
    anchor: Anchor,
    pub(crate) image: Arc<Image>,
    animation: CeMapAnimation,
    /// `zsprite radius scale`, where nothing means one.
    zsprite_scale: f32,
}

/// A second map's `anchor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    WithPrimary,
    WithScreenSpace,
    Zsprite,
}

impl Anchor {
    fn read(holder: TagStruct<'_>) -> Self {
        match holder.read_enum_option("anchor").as_deref() {
            Some("with screen space") => Self::WithScreenSpace,
            Some("zsprite") => Self::Zsprite,
            _ => Self::WithPrimary,
        }
    }
}

/// What a sprite draws its second map with.
#[derive(Debug, Clone)]
pub(crate) struct CeSecondaryDraw {
    pub(crate) map: Arc<CeSecondaryMap>,
    /// 1 over the sprite, 2 over the screen, 3 as depth.
    pub(crate) mode: i32,
    /// `C_TEXTURE1_XFORM`'s two rows.
    pub(crate) rows: [[f32; 4]; 2],
    /// How far toward the camera the map's green moves a zsprite's pixels.
    pub(crate) radius: f32,
}

/// An effect shader's `bitmap` and how it is laid, by the field's name.
pub(in crate::app::model_preview) fn read_secondary_map(
    reader: &TagReader,
    holder: TagStruct<'_>,
    bitmap_field: &str,
) -> Option<Arc<CeSecondaryMap>> {
    let path = holder
        .read_tag_ref_path(bitmap_field)
        .filter(|path| !path.is_empty())?;
    let image = decode_image(reader, &path, SamplerKind::Texture2D, 0)?;
    Some(Arc::new(CeSecondaryMap {
        anchor: Anchor::read(holder),
        image: Arc::new(image),
        animation: read_effect_map_animation(holder),
        zsprite_scale: holder.read_real("zsprite radius scale").unwrap_or(0.0),
    }))
}

impl CeSecondaryMap {
    /// What `rasterizer_transparent_geometry_group_draw` sets for it, with
    /// the `secondary map radius` the draw gives the shader.
    /// Sprites draw with no object's animation, which reads every source as one.
    pub(crate) fn draw(self: &Arc<Self>, time: f32, radius: f32) -> CeSecondaryDraw {
        let scale = if self.zsprite_scale == 0.0 {
            1.0
        } else {
            self.zsprite_scale
        };
        CeSecondaryDraw {
            map: self.clone(),
            mode: match self.anchor {
                Anchor::WithPrimary => 1,
                Anchor::WithScreenSpace => 2,
                Anchor::Zsprite => 3,
            },
            rows: self.animation.rows(time, &[1.0; 4]),
            radius: radius * scale,
        }
    }
}

/// A bitmap's sprites by sequence, each image's size, and the images any
/// sprite is cut from, decoded.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeSpriteSheet {
    pub(super) sizes: Vec<[f32; 2]>,
    pub(super) sequences: Vec<Vec<CeSpriteFrame>>,
    pub(crate) images: Vec<Option<Arc<Image>>>,
}

pub(in crate::app::model_preview) fn read_sprite_sheet(
    reader: &TagReader,
    bitmap_path: &str,
) -> Option<CeSpriteSheet> {
    let bitmap = reader.load(bitmap_path, "bitmap", b"bitm").ok()?;
    let (sizes, sequences) = read_sheet_layout(&bitmap);
    let used: Vec<usize> = sequences
        .iter()
        .flatten()
        .map(|sprite| sprite.image)
        .collect();
    let images = (0..sizes.len())
        .map(|index| {
            used.contains(&index)
                .then(|| decode_image(reader, bitmap_path, SamplerKind::Texture2D, index))
                .flatten()
                .map(Arc::new)
        })
        .collect();
    Some(CeSpriteSheet {
        sizes,
        sequences,
        images,
    })
}

/// A sheet's image sizes and its sprites by sequence, read from the bitmap
/// without decoding it.
pub(in crate::app::model_preview) fn read_sheet_layout(
    bitmap: &TagFile,
) -> (Vec<[f32; 2]>, Vec<Vec<CeSpriteFrame>>) {
    let definition = BitmapDefinition::from_tag(bitmap);
    let sizes: Vec<[f32; 2]> = definition
        .images
        .iter()
        .map(|image| [f32::from(image.width.max(1)), f32::from(image.height.max(1))])
        .collect();
    let sequences: Vec<Vec<CeSpriteFrame>> = definition
        .sequences
        .iter()
        .map(|sequence| {
            sequence
                .sprites
                .iter()
                .map(|sprite| CeSpriteFrame {
                    rect: sprite.rect(),
                    registration: sprite.registration_point,
                    image: sprite.bitmap_index.max(0) as usize,
                })
                .collect()
        })
        .collect();
    (sizes, sequences)
}

impl CeSpriteSheet {
    /// How many sprites a sequence has; none past the end.
    pub(crate) fn count(&self, sequence: i32) -> usize {
        usize::try_from(sequence)
            .ok()
            .and_then(|at| self.sequences.get(at))
            .map_or(0, Vec::len)
    }

    /// `build_sprite` for one sprite, facing the view as mode 0 does: the
    /// corners (place, then u and v) about `center`, `scale` world units to
    /// a pixel of its image, turned by `rotation` and mirrored across.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn corners(
        &self,
        sequence: i32,
        sprite: usize,
        center: [f32; 3],
        rotation: f32,
        scale: f32,
        mirror: bool,
        view: &ViewFrame,
    ) -> Option<(usize, [[f32; 5]; 4])> {
        let frame = usize::try_from(sequence)
            .ok()
            .and_then(|at| self.sequences.get(at))
            .and_then(|frames| frames.get(sprite))?;
        let size = self.sizes.get(frame.image).map_or(1.0, |size| size[0]) * scale;
        let [left, right, top, bottom] = frame.rect;
        let center_uv = [left + frame.registration[0], top + frame.registration[1]];
        let (sine, cosine) = rotation.sin_cos();
        let corner = |u: f32, v: f32| {
            let x0 = u - center_uv[0];
            let y0 = center_uv[1] - v;
            let mut x = x0 * cosine - y0 * sine;
            let y = y0 * cosine + x0 * sine;
            if mirror {
                x = -x;
            }
            let place: [f32; 3] = std::array::from_fn(|axis| {
                center[axis] + (view.right[axis] * x + view.up[axis] * y) * size
            });
            [place[0], place[1], place[2], u, v]
        };
        Some((
            frame.image,
            [
                corner(left, bottom),
                corner(right, bottom),
                corner(left, top),
                corner(right, top),
            ],
        ))
    }
}
