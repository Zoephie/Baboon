//! The snapshot's drawing (`model_preview::snapshot`): the frame on screen,
//! drawn again into half-float targets on black and on white, the viewport's
//! egui helpers over each, and the two matted on the GPU into the image.
//!
//! It draws with the viewport's own renderer, through the same
//! [`ModelGlRenderer::paint`], so nothing is drawn by a second path. Only
//! where the frame lands differs, and the format of the frame-sized targets
//! that hold the background (`RenderCore::frame_texels`), so the white
//! render doesn't clamp where light is added to it.

use super::{CachedModelGlRenderer, MODEL_GL_RENDERER, ModelGlRenderer, cached_renderer};
use crate::app::model_preview::gl::scope::PostScope;
use crate::app::model_preview::gl::target::{ColorTarget, Sampling, SceneTarget, Texels};
use crate::app::model_preview::snapshot::{SnapshotImage, SnapshotRequest};
use eframe::egui;
use eframe::glow::{self, HasContext as _};
use std::sync::atomic::{AtomicI32, Ordering};

const MATTE_FRAGMENT: &str = include_str!("shaders/snapshot_matte.frag");
const HELPERS_VERTEX: &str = include_str!("shaders/snapshot_helpers.vert");
const HELPERS_FRAGMENT: &str = include_str!("shaders/snapshot_helpers.frag");

/// The largest side a snapshot's targets can have on this GPU, once a
/// context has been drawn in; 0 before.
static SIZE_LIMIT: AtomicI32 = AtomicI32::new(0);

/// The largest tile a snapshot draws: its float targets, two renders and the
/// game's own come to about a gigabyte at this size, which any GPU that can
/// draw the preview holds; a larger image is drawn in more tiles.
const TILE_LIMIT: i32 = 4096;

/// The largest side a snapshot draws at once on the GPU the preview draws
/// with (the GPU's own limit, or [`TILE_LIMIT`]), or `None` before it has
/// drawn. A larger snapshot is drawn in tiles.
pub(in crate::app::model_preview) fn snapshot_size_limit() -> Option<i32> {
    Some(SIZE_LIMIT.load(Ordering::Relaxed)).filter(|limit| *limit > 0).map(|limit| limit.min(TILE_LIMIT))
}

/// Note the largest texture, renderbuffer and viewport `gl` takes.
pub(super) fn note_size_limit(gl: &glow::Context) {
    unsafe {
        let mut viewport = [0; 2];
        gl.get_parameter_i32_slice(glow::MAX_VIEWPORT_DIMS, &mut viewport);
        let limit = gl
            .get_parameter_i32(glow::MAX_TEXTURE_SIZE)
            .min(gl.get_parameter_i32(glow::MAX_RENDERBUFFER_SIZE))
            .min(viewport[0])
            .min(viewport[1]);
        SIZE_LIMIT.store(limit, Ordering::Relaxed);
    }
}

/// Draw `request` and leave its result in it: called from the viewport's
/// paint callback, before the viewport's own frame, so the simulations are
/// where the frame on screen left them.
pub(in crate::app::model_preview) fn paint_snapshot(
    painter: &eframe::egui_glow::Painter,
    request: &SnapshotRequest,
) {
    let gl = painter.gl();
    let result = MODEL_GL_RENDERER.with(|cell| {
        let mut cached = cell.borrow_mut();
        cached_renderer(&mut cached, gl);
        let Some(CachedModelGlRenderer { renderer: Ok(renderer), .. }) = cached.as_mut() else {
            return Err("the preview has no renderer to draw the snapshot with".to_owned());
        };
        let scope = unsafe { PostScope::capture(gl) };
        let texture_of = |id| painter.texture(id);
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            draw_snapshot(gl, &texture_of, renderer, request)
        }));
        unsafe {
            renderer.core.frame_texels = Texels::RGBA8;
            renderer.core.held_bloom = false;
            scope.restore(gl);
            gl.use_program(None);
            gl.bind_vertex_array(None);
        }
        drawn.unwrap_or_else(|panic| {
            Err(format!("drawing the snapshot crashed: {}", crate::app::shell::worker::panic_detail(panic.as_ref())))
        })
    });
    request.set_result(result);
}

/// egui's texture for an id: the painter's, in the app.
pub(in crate::app::model_preview) type TextureOf<'a> = dyn Fn(egui::TextureId) -> Option<glow::NativeTexture> + 'a;

/// The snapshot: its bloom built once from the whole picture at the
/// viewport's size, then each tile (one, unless the image is larger than the
/// GPU draws at once) drawn on black and white, its helpers over it, matted,
/// and put in its place.
pub(in crate::app::model_preview) unsafe fn draw_snapshot(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
) -> Result<SnapshotImage, String> {
    let limit = snapshot_size_limit().unwrap_or(TILE_LIMIT);
    unsafe { draw_snapshot_in_tiles(gl, texture_of, renderer, request, limit) }
}

/// [`draw_snapshot`], with tiles no larger than `limit` a side.
pub(in crate::app::model_preview) unsafe fn draw_snapshot_in_tiles(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
    limit: i32,
) -> Result<SnapshotImage, String> {
    let [width, height] = request.size;
    if width < 1 || height < 1 {
        return Err("the viewport has no size to snapshot".to_owned());
    }
    let margin = ((32.0 * request.scale).ceil() as i32).min(limit / 8);
    let tiles = snapshot_tiles(request.size, limit, margin)?;
    let core = &mut renderer.core;
    core.tile_view_scale = 1.0;
    unsafe {
        // The bloom, from the whole picture at the viewport's size on black:
        // the viewport's own, which every render after adds.
        if request.frame.game_shaders {
            let on_screen = request.size.map(|side| ((side as f32 / request.scale).round() as i32).clamp(1, limit));
            let target = SceneTarget::new(gl, on_screen[0], on_screen[1], Texels::RGBA16F)?;
            renderer.core.held_bloom = false;
            draw_on(gl, texture_of, renderer, request, &request.frame, &target, 0.0, WHOLE_PICTURE);
            target.delete(gl);
        }
        renderer.core.held_bloom = true;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        for tile in &tiles {
            let drawn = draw_tile(gl, texture_of, renderer, request, tile);
            let tile_image = match drawn {
                Ok(image) => image,
                Err(error) => {
                    reset_tiling(&mut renderer.core);
                    return Err(error);
                }
            };
            place_tile(&mut rgba, request.size, tile, &tile_image);
        }
        let view_scale = renderer.core.tile_view_scale;
        reset_tiling(&mut renderer.core);
        let mut image = SnapshotImage { width: width as usize, height: height as usize, rgba };
        if tiles.len() > 1 && (view_scale - 1.0).abs() > 1e-4 {
            image = crate::app::model_preview::snapshot::zoom_about_centre(&image, view_scale);
        }
        Ok(image)
    }
}

/// What tiling changed, back as a plain draw leaves it.
fn reset_tiling(core: &mut super::RenderCore) {
    core.held_bloom = false;
    core.bloom_window = super::WHOLE_WINDOW;
}

/// The whole picture, as a window of clip space.
const WHOLE_PICTURE: [f32; 4] = [-1.0, -1.0, 1.0, 1.0];

/// One tile of a snapshot, in pixels of the whole image from its bottom-left
/// corner (as GL counts): what is drawn, margin and all, and the inside of
/// it that goes into the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app::model_preview) struct SnapshotTile {
    /// `[x, y, width, height]`.
    pub(in crate::app::model_preview) drawn: [i32; 4],
    pub(in crate::app::model_preview) kept: [i32; 4],
}

/// The tiles that cover an image of `size`, none wider or taller than
/// `limit`: one, the whole image, when it fits; else a grid whose insides
/// meet edge to edge, each drawn with `margin` more on every side within the
/// image, so blurs across their edges come out as across the whole.
pub(in crate::app::model_preview) fn snapshot_tiles(
    size: [i32; 2],
    limit: i32,
    margin: i32,
) -> Result<Vec<SnapshotTile>, String> {
    let [width, height] = size;
    if width <= limit && height <= limit {
        let whole = [0, 0, width, height];
        return Ok(vec![SnapshotTile { drawn: whole, kept: whole }]);
    }
    let inside = limit - 2 * margin;
    if inside < 64 {
        return Err(format!("{width} × {height} is too large to draw in tiles of {limit} pixels"));
    }
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < height {
        let kept_height = inside.min(height - y);
        let mut x = 0;
        while x < width {
            let kept_width = inside.min(width - x);
            let (left, bottom) = ((x - margin).max(0), (y - margin).max(0));
            let (right, top) = ((x + kept_width + margin).min(width), (y + kept_height + margin).min(height));
            tiles.push(SnapshotTile {
                drawn: [left, bottom, right - left, top - bottom],
                kept: [x, y, kept_width, kept_height],
            });
            x += kept_width;
        }
        y += kept_height;
    }
    Ok(tiles)
}

/// One tile, drawn through its window of the picture on black and white, and
/// matted.
unsafe fn draw_tile(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
    tile: &SnapshotTile,
) -> Result<SnapshotImage, String> {
    let [width, height] = request.size.map(|side| side as f32);
    let [x, y, w, h] = tile.drawn;
    let window = [
        2.0 * x as f32 / width - 1.0,
        2.0 * y as f32 / height - 1.0,
        2.0 * (x + w) as f32 / width - 1.0,
        2.0 * (y + h) as f32 / height - 1.0,
    ];
    let mut frame = request.frame.clone();
    frame.camera = frame.camera.windowed(window);
    renderer.core.bloom_window = [x as f32 / width, y as f32 / height, w as f32 / width, h as f32 / height];
    unsafe {
        let black = SceneTarget::new(gl, w, h, Texels::RGBA16F)?;
        let white = match SceneTarget::new(gl, w, h, Texels::RGBA16F) {
            Ok(white) => white,
            Err(error) => {
                black.delete(gl);
                return Err(error);
            }
        };
        for (target, background) in [(&black, 0.0), (&white, 1.0)] {
            draw_on(gl, texture_of, renderer, request, &frame, target, background, window);
        }
        let image = matte(gl, &black, &white);
        black.delete(gl);
        white.delete(gl);
        image
    }
}

/// `tile`'s kept inside, from `image` (the tile drawn, top row first), into
/// `rgba`, the whole image's, top row first.
fn place_tile(rgba: &mut [u8], size: [i32; 2], tile: &SnapshotTile, image: &SnapshotImage) {
    let (width, height) = (size[0] as usize, size[1] as usize);
    let [drawn_x, drawn_y, _, drawn_h] = tile.drawn.map(|edge| edge as usize);
    let [kept_x, kept_y, kept_w, kept_h] = tile.kept.map(|edge| edge as usize);
    let drawn_top = drawn_y + drawn_h;
    for row in 0..kept_h {
        // Rows counted from the top: the whole image's, and the tile's.
        let from_bottom = kept_y + kept_h - 1 - row;
        let whole_row = height - 1 - from_bottom;
        let tile_row = drawn_top - 1 - from_bottom;
        let from = (tile_row * image.width + (kept_x - drawn_x)) * 4;
        let to = (whole_row * width + kept_x) * 4;
        rgba[to..to + kept_w * 4].copy_from_slice(&image.rgba[from..from + kept_w * 4]);
    }
}

/// `frame` on `background` (the same grey each channel) into `target`,
/// without the grid or the Material Editor's hover, then the helpers over it,
/// seen through `window` of the picture.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_on(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
    frame: &crate::app::model_preview::frame::ModelGpuFrame,
    target: &SceneTarget,
    background: f32,
    window: [f32; 4],
) {
    let [width, height] = target.size;
    let mut frame = frame.clone();
    frame.background = [background; 3];
    frame.show_grid = false;
    frame.highlight.clear();
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
        gl.viewport(0, 0, width, height);
        gl.disable(glow::SCISSOR_TEST);
        gl.color_mask(true, true, true, true);
        gl.depth_mask(true);
        // Alpha 0, as the scene target is cleared in the viewport's frame:
        // the passes that read destination alpha read none of it as cover.
        gl.clear_color(background, background, background, 0.0);
        gl.clear_depth_f32(1.0);
        gl.clear_stencil(0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        renderer.core.frame_texels = Texels::RGBA16F;
        renderer.paint(gl, &frame);
        renderer.core.frame_texels = Texels::RGBA8;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
        gl.viewport(0, 0, width, height);
        gl.disable(glow::SCISSOR_TEST);
        draw_helpers(gl, texture_of, &request.helpers, request.points, window);
    }
}

/// egui's meshes over the frame, as its painter draws them: premultiplied,
/// over what is there.
unsafe fn draw_helpers(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    helpers: &[egui::ClippedPrimitive],
    points: [f32; 2],
    window: [f32; 4],
) {
    let meshes: Vec<&egui::epaint::Mesh> = helpers
        .iter()
        .filter_map(|primitive| match &primitive.primitive {
            egui::epaint::Primitive::Mesh(mesh) if !mesh.indices.is_empty() => Some(mesh),
            _ => None,
        })
        .collect();
    if meshes.is_empty() {
        return;
    }
    unsafe {
        let stages = [(glow::VERTEX_SHADER, HELPERS_VERTEX), (glow::FRAGMENT_SHADER, HELPERS_FRAGMENT)];
        let program = match crate::app::model_preview::gl::link::link(gl, &stages, &[]) {
            Ok(program) => program,
            Err(log) => {
                eprintln!("model preview: snapshot helpers shader: {log}");
                return;
            }
        };
        let array = gl.create_vertex_array().expect("a vertex array");
        let vertices = gl.create_buffer().expect("a buffer");
        let indices = gl.create_buffer().expect("a buffer");
        gl.use_program(Some(program));
        gl.uniform_2_f32(gl.get_uniform_location(program, "screen_points").as_ref(), points[0], points[1]);
        let [x0, y0, x1, y1] = window;
        let (scale, offset) = ([2.0 / (x1 - x0), 2.0 / (y1 - y0)], [-(x1 + x0) / (x1 - x0), -(y1 + y0) / (y1 - y0)]);
        gl.uniform_4_f32(gl.get_uniform_location(program, "window").as_ref(), scale[0], scale[1], offset[0], offset[1]);
        gl.uniform_1_i32(gl.get_uniform_location(program, "image").as_ref(), 0);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_vertex_array(Some(array));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices));
        let stride = std::mem::size_of::<egui::epaint::Vertex>() as i32;
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
        gl.enable_vertex_attrib_array(2);
        gl.vertex_attrib_pointer_f32(2, 4, glow::UNSIGNED_BYTE, true, stride, 16);
        for mesh in meshes {
            let Some(texture) = texture_of(mesh.texture_id) else { continue };
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                crate::app::model_preview::gl::buffer::slice_bytes(&mesh.vertices),
                glow::STREAM_DRAW,
            );
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                crate::app::model_preview::gl::buffer::slice_bytes(&mesh.indices),
                glow::STREAM_DRAW,
            );
            gl.draw_elements(glow::TRIANGLES, mesh.indices.len() as i32, glow::UNSIGNED_INT, 0);
        }
        gl.bind_vertex_array(None);
        gl.delete_vertex_array(array);
        gl.delete_buffer(vertices);
        gl.delete_buffer(indices);
        gl.delete_program(program);
        gl.disable(glow::BLEND);
    }
}

/// The two renders matted into straight RGBA bytes, top row first.
unsafe fn matte(gl: &glow::Context, black: &SceneTarget, white: &SceneTarget) -> Result<SnapshotImage, String> {
    let [width, height] = black.size;
    unsafe {
        let program = crate::app::model_preview::gl::screen::screen_program(gl, MATTE_FRAGMENT)
            .ok_or("the snapshot's matte shader did not build")?;
        let target = ColorTarget::new(gl, [width, height], Texels::RGBA8, Sampling::NEAREST_CLAMPED)
            .ok_or("the snapshot's image target was not made")?;
        let array = gl.create_vertex_array().expect("a vertex array");
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
        gl.viewport(0, 0, width, height);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.disable(glow::DEPTH_TEST);
        gl.use_program(Some(program));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(black.color));
        gl.uniform_1_i32(gl.get_uniform_location(program, "black").as_ref(), 0);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(white.color));
        gl.uniform_1_i32(gl.get_uniform_location(program, "white").as_ref(), 1);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_vertex_array(Some(array));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        gl.read_pixels(
            0,
            0,
            width,
            height,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut rgba)),
        );
        gl.bind_vertex_array(None);
        gl.delete_vertex_array(array);
        target.delete(gl);
        Ok(SnapshotImage { width: width as usize, height: height as usize, rgba })
    }
}

/// The frame drawn as a snapshot draws it, on `background`, read back as
/// floats (RGBA, bottom row first), for a test of the matte's premise; with
/// `held_bloom`, adding the bloom the draw before built.
#[cfg(test)]
pub(in crate::app::model_preview) unsafe fn draw_floats(
    gl: &glow::Context,
    renderer: &mut ModelGlRenderer,
    frame: &crate::app::model_preview::frame::ModelGpuFrame,
    size: [i32; 2],
    background: f32,
    held_bloom: bool,
) -> Vec<f32> {
    let request = SnapshotRequest::new(frame.clone(), size, Vec::new(), [size[0] as f32, size[1] as f32], 1.0);
    unsafe {
        let target = SceneTarget::new(gl, size[0], size[1], Texels::RGBA16F).expect("a float target");
        renderer.core.held_bloom = held_bloom;
        draw_on(gl, &|_| None, renderer, &request, frame, &target, background, WHOLE_PICTURE);
        renderer.core.held_bloom = false;
        let mut floats = vec![0f32; size[0] as usize * size[1] as usize * 4];
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
        gl.read_pixels(
            0,
            0,
            size[0],
            size[1],
            glow::RGBA,
            glow::FLOAT,
            glow::PixelPackData::Slice(Some(bytemuck_floats(&mut floats))),
        );
        target.delete(gl);
        floats
    }
}

#[cfg(test)]
fn bytemuck_floats(floats: &mut [f32]) -> &mut [u8] {
    // SAFETY: f32 has no invalid bit patterns, and u8 no alignment.
    unsafe { std::slice::from_raw_parts_mut(floats.as_mut_ptr().cast::<u8>(), std::mem::size_of_val(floats)) }
}

#[cfg(test)]
mod tests {
    use super::snapshot_tiles;

    /// An image that fits is one tile, the whole of it. One that doesn't is
    /// tiled: the kept insides cover every pixel once, each drawn tile holds
    /// its inside with the margin round it (less at the image's edges), and
    /// none is larger than the limit.
    #[test]
    fn tiles_cover_the_image_once_within_the_limit() {
        let one = snapshot_tiles([1000, 700], 1024, 64).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].drawn, [0, 0, 1000, 700]);
        let (size, limit, margin) = ([3000, 2000], 1024, 64);
        let tiles = snapshot_tiles(size, limit, margin).unwrap();
        assert!(tiles.len() > 1);
        let mut covered = vec![0u8; (size[0] * size[1]) as usize];
        for tile in &tiles {
            let [dx, dy, dw, dh] = tile.drawn;
            let [kx, ky, kw, kh] = tile.kept;
            assert!(dw <= limit && dh <= limit, "{tile:?} within the limit");
            assert!(dx <= kx && dy <= ky && dx + dw >= kx + kw && dy + dh >= ky + kh, "{tile:?} holds its inside");
            assert!(kx - dx == margin.min(kx) && ky - dy == margin.min(ky), "{tile:?} margin, or the image's edge");
            for y in ky..ky + kh {
                for x in kx..kx + kw {
                    covered[(y * size[0] + x) as usize] += 1;
                }
            }
        }
        assert!(covered.iter().all(|&count| count == 1), "every pixel kept once");
        assert!(snapshot_tiles([5000, 5000], 150, 64).is_err(), "no room inside the margins");
    }
}
