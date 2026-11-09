//! Decoding a gen3 bitmap tag into an image, with its stored mips.

use super::super::super::{ResolveCaches, TagFile, TagReader, TextureImage, decode_image};
use super::{Image, ImageTexels, SamplerKind, elements};
use std::sync::Arc;

/// A bitmap for a pass, and whether `curve` has the card decode it.
pub(in crate::app::model_preview) fn gen3_image(
    reader: &TagReader,
    path: &str,
    index: i16,
    kind: SamplerKind,
    caches: &mut ResolveCaches,
) -> Option<Gen3Bitmap> {
    let key = (path.to_owned(), index, kind as usize);
    if let Some(found) = caches.h3_images.get(&key) {
        return found.clone();
    }
    let found = read_gen3_bitmap(reader, path, index, kind);
    caches.h3_images.insert(key, found.clone());
    found
}

/// One image of a bitmap, what its texels mean, and its stored mips.
pub(in crate::app::model_preview) type Gen3Bitmap = (Arc<Image>, ImageTexels, Arc<Vec<Image>>);

/// Levels past the top, as the tag stores them. None when `image` was not
/// decoded at the tag's own size, since these would not continue it.
fn gen3_stored_mips(tag: &TagFile, index: usize, image: &Image) -> Option<Vec<Image>> {
    let bitmap = blam_tags::bitmap::Bitmap::new(tag).ok()?;
    let stored = bitmap.image(index)?;
    let (width, height) = (stored.width(), stored.height());
    let side = |size: u32, level: u32| (size >> level).max(1) as usize;
    let levels = 1..stored.mipmap_levels();
    match image {
        Image::Flat(top) if top.width == width as usize && top.height == height as usize => levels
            .map(|level| {
                let data =
                    crate::app::editor::build_bitmap_preview(tag, index, level as usize).ok()?;
                let (width, height) = (side(width, level), side(height, level));
                (data.width as usize == width && data.height as usize == height).then_some(
                    Image::Flat(TextureImage {
                        rgba: data.rgba,
                        width,
                        height,
                        repeat_x: top.repeat_x,
                        repeat_y: top.repeat_y,
                        scale: top.scale,
                    }),
                )
            })
            .collect(),
        Image::Cube { size, .. } if *size == width as usize => {
            let format = stored.format().ok()?;
            levels
                .map(|level| {
                    let edge = side(width, level);
                    let face = |face: u32| {
                        let data = stored.cube_face_level_bytes(level, face).ok()?;
                        let palette = bitmap.p8_palette();
                        blam_tags::bitmap::decode::decode_to_rgba8(
                            format,
                            edge as u32,
                            edge as u32,
                            data,
                            palette,
                        )
                        .ok()
                    };
                    let faces = [face(0)?, face(1)?, face(2)?, face(3)?, face(4)?, face(5)?];
                    Some(Image::Cube { size: edge, faces })
                })
                .collect()
        }
        _ => None,
    }
}

/// What an `a8` texel samples as: MCC's Halo 3 (`halo3.dll` 0x180801D30) and
/// Reach (`haloreach.dll` 0x180A32D70) upload it as `DXGI_FORMAT_A8_UNORM`,
/// which reads black with its alpha. Reach's `color_black` change-color map is
/// one, and two_change_color reads its red and green.
fn alpha_only(image: &mut Image) {
    let blacken = |rgba: &mut Vec<u8>| {
        rgba.chunks_exact_mut(4)
            .for_each(|texel| texel[..3].fill(0))
    };
    match image {
        Image::Flat(flat) => blacken(&mut flat.rgba),
        Image::Cube { faces, .. } => faces.iter_mut().for_each(blacken),
        Image::Volume { rgba, .. } => blacken(rgba),
    }
}

/// What an R8_UNORM texel samples as: (r, 0, 0, 1).
fn red_only(image: &mut Image) {
    let keep_red = |rgba: &mut Vec<u8>| {
        rgba.chunks_exact_mut(4).for_each(|texel| {
            texel[1] = 0;
            texel[2] = 0;
            texel[3] = 255;
        })
    };
    match image {
        Image::Flat(flat) => keep_red(&mut flat.rgba),
        Image::Cube { faces, .. } => faces.iter_mut().for_each(keep_red),
        Image::Volume { rgba, .. } => keep_red(rgba),
    }
}

pub(in crate::app::model_preview) fn read_gen3_bitmap(
    reader: &TagReader,
    path: &str,
    index: i16,
    kind: SamplerKind,
) -> Option<Gen3Bitmap> {
    let tag = reader.load(path, "bitmap", b"bitm").ok();
    // `render_method_submit`: an index past the last image takes the last.
    let images = tag
        .as_ref()
        .map_or(1, |tag| elements(tag.root(), "bitmaps*").len());
    let index = (index.max(0) as usize).min(images.saturating_sub(1)) as i16;
    let image = decode_image(reader, path, kind, index as usize);
    image.map(|mut image| {
        // `c_rasterizer_texture_ref::allocate`: xRGB and sRGB curves.
        let curve = tag.as_ref().and_then(|tag| {
            elements(tag.root(), "bitmaps*")
                .get(index.max(0) as usize)?
                .read_int_any("curve")
        });
        let format = tag.as_ref().and_then(|tag| {
            blam_tags::bitmap::Bitmap::new(tag)
                .ok()?
                .image(index.max(0) as usize)?
                .format()
                .ok()
        });
        let own_texels = reader.rules().gen3.texels;
        let texels = format.map_or(ImageTexels::Linear, |format| {
            if let Some(texels) = own_texels {
                return texels(format, curve.unwrap_or(0));
            }
            let srgb = if matches!(curve, None | Some(0)) {
                ImageTexels::default_srgb(format)
            } else {
                matches!(curve, Some(1 | 5))
            };
            ImageTexels::of(format, srgb)
        });
        let mut mips = tag
            .as_ref()
            .and_then(|tag| gen3_stored_mips(tag, index.max(0) as usize, &image))
            .unwrap_or_default();
        if format == Some(blam_tags::bitmap::BitmapFormat::A8) {
            std::iter::once(&mut image)
                .chain(&mut mips)
                .for_each(alpha_only);
        }
        if reader.rules().gen3.luminance_as_red
            && matches!(
                format,
                Some(blam_tags::bitmap::BitmapFormat::Y8 | blam_tags::bitmap::BitmapFormat::Ay8)
            )
        {
            std::iter::once(&mut image)
                .chain(&mut mips)
                .for_each(red_only);
        }
        (Arc::new(image), texels, Arc::new(mips))
    })
}
