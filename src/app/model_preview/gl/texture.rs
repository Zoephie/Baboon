//! Texture uploads every pipeline shares: a decoded map in any of its shapes,
//! with or without the bake's sampler, and the placeholder, black and solid
//! maps a pass binds where it has none.

use std::borrow::Cow;

use blam_tags::render_method::{BitmapAddressMode, BitmapFilterMode};

use eframe::glow::{self, HasContext as _};

use super::super::common::image::{Image, ImageTexels, Sampler};
use super::super::*;

/// The texture bound to a slot whose material has none: one opaque white
/// texel, never sampled for its value (the slot's `have` flag is zero), only
/// there so the unit holds a complete texture. Complete means a non-mipmap
/// minification filter. GL's default filter expects mipmaps, and with one
/// level it would leave this texture as incomplete as texture 0.
pub(in crate::app::model_preview) unsafe fn upload_placeholder_texture(
    gl: &glow::Context,
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&[255, 255, 255, 255])),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

pub(in crate::app::model_preview) unsafe fn upload_image(
    gl: &glow::Context,
    image: &Image,
) -> Option<glow::NativeTexture> {
    if !image.whole() {
        return None;
    }
    let texture_parameters = |target| unsafe {
        gl.generate_mipmap(target);
        gl.tex_parameter_i32(
            target,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    };
    unsafe {
        match image {
            Image::Flat(image) => upload_texture(gl, image),
            Image::Cube { size, faces } => {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
                for (face, rgba) in faces.iter().enumerate() {
                    gl.tex_image_2d(
                        glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                        0,
                        glow::RGBA8 as i32,
                        *size as i32,
                        *size as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(rgba)),
                    );
                }
                texture_parameters(glow::TEXTURE_CUBE_MAP);
                for wrap in [
                    glow::TEXTURE_WRAP_S,
                    glow::TEXTURE_WRAP_T,
                    glow::TEXTURE_WRAP_R,
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, wrap, glow::CLAMP_TO_EDGE as i32);
                }
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
                Some(texture)
            }
            Image::Volume {
                width,
                height,
                depth,
                rgba,
            } => {
                let texture = gl.create_texture().ok()?;
                gl.bind_texture(glow::TEXTURE_3D, Some(texture));
                gl.tex_image_3d(
                    glow::TEXTURE_3D,
                    0,
                    glow::RGBA8 as i32,
                    *width as i32,
                    *height as i32,
                    *depth as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(rgba)),
                );
                texture_parameters(glow::TEXTURE_3D);
                gl.bind_texture(glow::TEXTURE_3D, None);
                Some(texture)
            }
        }
    }
}

/// A 1x1 black 2D texture: the bloom a pass reads when none was drawn.
pub(in crate::app::model_preview) unsafe fn upload_black_flat(
    gl: &glow::Context,
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        let black = glow::PixelUnpackData::Slice(Some(&[0, 0, 0, 255]));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            black,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

/// A 1x1 black cube, for passes whose cube map is not read: a reflection
/// that adds nothing.
pub(in crate::app::model_preview) unsafe fn upload_black_cube(
    gl: &glow::Context,
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
        for face in 0..6 {
            gl.tex_image_2d(
                glow::TEXTURE_CUBE_MAP_POSITIVE_X + face,
                0,
                glow::RGBA8 as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&[0, 0, 0, 255])),
            );
        }
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_CUBE_MAP, name, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_CUBE_MAP, None);
        Some(texture)
    }
}

/// A 1x1x1 texture of one color for `target`, a volume or an array of 2D
/// layers: a stand-in for a map that is not read.
pub(in crate::app::model_preview) unsafe fn upload_solid_volume(
    gl: &glow::Context,
    target: u32,
    color: [u8; 4],
) -> Option<glow::NativeTexture> {
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(target, Some(texture));
        gl.tex_image_3d(
            target,
            0,
            glow::RGBA8 as i32,
            1,
            1,
            1,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&color)),
        );
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        ] {
            gl.tex_parameter_i32(target, name, value as i32);
        }
        gl.bind_texture(target, None);
        Some(texture)
    }
}

/// Upload one decoded texture, with mipmaps and the wrap modes the shader
/// authored.
///
/// Mipmaps matter here beyond quality: a detail map tiling twenty times across
/// a model aliases into noise without them, which is exactly where a preview
/// looks worst.
pub(in crate::app::model_preview) unsafe fn upload_texture(
    gl: &glow::Context,
    image: &TextureImage,
) -> Option<glow::NativeTexture> {
    if image.width == 0 || image.height == 0 || !image.whole() {
        return None;
    }
    unsafe {
        let texture = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            image.width as i32,
            image.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&image.rgba)),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        let wrap = |repeat: bool| {
            if repeat {
                glow::REPEAT as i32
            } else {
                glow::CLAMP_TO_EDGE as i32
            }
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(image.repeat_x));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(image.repeat_y));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(texture)
    }
}

/// `d3d11_sampler_state_cache::get`: the border is transparent black, and
/// anisotropic filters are trilinear with a limit.
pub(in crate::app::model_preview) unsafe fn gen3_apply_sampler(
    gl: &glow::Context,
    target: u32,
    sampler: Sampler,
) {
    const TEXTURE_MAX_ANISOTROPY: u32 = 0x84FE;
    let address = |mode| match mode {
        BitmapAddressMode::Clamp => glow::CLAMP_TO_EDGE,
        BitmapAddressMode::Mirror => glow::MIRRORED_REPEAT,
        BitmapAddressMode::BlackBorder => glow::CLAMP_TO_BORDER,
        _ => glow::REPEAT,
    } as i32;
    let (minify, magnify, anisotropy) = match sampler.filter {
        BitmapFilterMode::Point => (glow::NEAREST_MIPMAP_NEAREST, glow::NEAREST, 1.0),
        BitmapFilterMode::Bilinear | BitmapFilterMode::LightprobeTextureArray => {
            (glow::LINEAR_MIPMAP_NEAREST, glow::LINEAR, 1.0)
        }
        BitmapFilterMode::Anisotropic2Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 2.0),
        BitmapFilterMode::Anisotropic3Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 3.0),
        BitmapFilterMode::Anisotropic4Expensive => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 4.0),
        _ => (glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR, 1.0),
    };
    unsafe {
        gl.tex_parameter_i32(target, glow::TEXTURE_MIN_FILTER, minify as i32);
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, magnify as i32);
        gl.tex_parameter_f32(target, TEXTURE_MAX_ANISOTROPY, anisotropy);
        if target != glow::TEXTURE_CUBE_MAP {
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_S, address(sampler.address[0]));
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_T, address(sampler.address[1]));
            gl.tex_parameter_i32(target, glow::TEXTURE_WRAP_R, address(sampler.address[0]));
            gl.tex_parameter_f32_slice(target, glow::TEXTURE_BORDER_COLOR, &[0.0; 4]);
        }
    }
}

/// The engine decodes signed texels biased by 128; the card wants them back.
pub(in crate::app::model_preview) fn signed_bytes(
    rgba: &[u8],
    texels: ImageTexels,
) -> Cow<'_, [u8]> {
    match texels {
        ImageTexels::Signed => rgba.iter().map(|byte| byte.wrapping_sub(128)).collect(),
        _ => Cow::Borrowed(rgba),
    }
}

/// `upload_image` with an sRGB format, for the curve the card decodes.
pub(in crate::app::model_preview) unsafe fn upload_image_linear(
    gl: &glow::Context,
    image: &Image,
    texels: ImageTexels,
    mips: &[Image],
    sampler: Option<Sampler>,
) -> Option<glow::NativeTexture> {
    // GL reads as much as the size says, whatever the slice holds.
    if !image.whole() {
        return None;
    }
    let mips = if mips.iter().all(Image::whole) {
        mips
    } else {
        &[]
    };
    let (format, kind) = match texels {
        ImageTexels::Linear => (glow::RGBA8, glow::UNSIGNED_BYTE),
        ImageTexels::Srgb => (glow::SRGB8_ALPHA8, glow::UNSIGNED_BYTE),
        ImageTexels::Signed => (glow::RGBA8_SNORM, glow::BYTE),
    };
    let format = format as i32;
    let finish = |target| unsafe {
        gl.generate_mipmap(target);
        gl.tex_parameter_i32(
            target,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl.tex_parameter_i32(target, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    };
    unsafe {
        let texture = gl.create_texture().ok()?;
        match image {
            Image::Flat(image) => {
                if image.width == 0 || image.height == 0 {
                    gl.delete_texture(texture);
                    return None;
                }
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    format,
                    image.width as i32,
                    image.height as i32,
                    0,
                    glow::RGBA,
                    kind,
                    glow::PixelUnpackData::Slice(Some(&signed_bytes(&image.rgba, texels))),
                );
                finish(glow::TEXTURE_2D);
                for (level, mip) in mips.iter().enumerate() {
                    if let Image::Flat(mip) = mip {
                        let rgba = signed_bytes(&mip.rgba, texels);
                        let (width, height) = (mip.width as i32, mip.height as i32);
                        let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                        gl.tex_image_2d(
                            glow::TEXTURE_2D,
                            level as i32 + 1,
                            format,
                            width,
                            height,
                            0,
                            glow::RGBA,
                            kind,
                            pixels,
                        );
                    }
                }
                if !mips.is_empty() {
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MAX_LEVEL,
                        mips.len() as i32,
                    );
                }
                let wrap = |repeat: bool| if repeat { glow::REPEAT } else { glow::CLAMP_TO_EDGE } as i32;
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap(image.repeat_x));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap(image.repeat_y));
                if let Some(sampler) = sampler {
                    gen3_apply_sampler(gl, glow::TEXTURE_2D, sampler);
                }
            }
            Image::Cube { size, faces } => {
                gl.bind_texture(glow::TEXTURE_CUBE_MAP, Some(texture));
                for (face, rgba) in faces.iter().enumerate() {
                    gl.tex_image_2d(
                        glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32,
                        0,
                        format,
                        *size as i32,
                        *size as i32,
                        0,
                        glow::RGBA,
                        kind,
                        glow::PixelUnpackData::Slice(Some(&signed_bytes(rgba, texels))),
                    );
                }
                finish(glow::TEXTURE_CUBE_MAP);
                for (level, mip) in mips.iter().enumerate() {
                    if let Image::Cube { size, faces } = mip {
                        for (face, rgba) in faces.iter().enumerate() {
                            let rgba = signed_bytes(rgba, texels);
                            let target = glow::TEXTURE_CUBE_MAP_POSITIVE_X + face as u32;
                            let side = *size as i32;
                            let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                            gl.tex_image_2d(
                                target,
                                level as i32 + 1,
                                format,
                                side,
                                side,
                                0,
                                glow::RGBA,
                                kind,
                                pixels,
                            );
                        }
                    }
                }
                if !mips.is_empty() {
                    gl.tex_parameter_i32(
                        glow::TEXTURE_CUBE_MAP,
                        glow::TEXTURE_MAX_LEVEL,
                        mips.len() as i32,
                    );
                }
                if let Some(sampler) = sampler {
                    gen3_apply_sampler(gl, glow::TEXTURE_CUBE_MAP, sampler);
                }
            }
            Image::Volume {
                width,
                height,
                depth,
                rgba,
            } => {
                gl.bind_texture(glow::TEXTURE_3D, Some(texture));
                gl.tex_image_3d(
                    glow::TEXTURE_3D,
                    0,
                    format,
                    *width as i32,
                    *height as i32,
                    *depth as i32,
                    0,
                    glow::RGBA,
                    kind,
                    glow::PixelUnpackData::Slice(Some(&signed_bytes(rgba, texels))),
                );
                finish(glow::TEXTURE_3D);
                if let Some(sampler) = sampler {
                    gen3_apply_sampler(gl, glow::TEXTURE_3D, sampler);
                }
            }
        }
        Some(texture)
    }
}
