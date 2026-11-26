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

/// The largest side a snapshot drawn whole can have on the GPU the preview
/// draws with, or `None` before it has drawn.
pub(in crate::app::model_preview) fn snapshot_size_limit() -> Option<i32> {
    Some(SIZE_LIMIT.load(Ordering::Relaxed)).filter(|limit| *limit > 0)
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

/// The two renders, the helpers over each, and the matte, read back.
pub(in crate::app::model_preview) unsafe fn draw_snapshot(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
) -> Result<SnapshotImage, String> {
    let [width, height] = request.size;
    let limit = snapshot_size_limit().unwrap_or(i32::MAX);
    if width < 1 || height < 1 {
        return Err("the viewport has no size to snapshot".to_owned());
    }
    if width > limit || height > limit {
        return Err(format!("{width} × {height} is over this GPU's {limit} pixel limit"));
    }
    unsafe {
        let black = SceneTarget::new(gl, width, height, Texels::RGBA16F)?;
        let white = match SceneTarget::new(gl, width, height, Texels::RGBA16F) {
            Ok(white) => white,
            Err(error) => {
                black.delete(gl);
                return Err(error);
            }
        };
        // Black first: its bloom is the model's alone, which white adds as
        // it is (`RenderCore::held_bloom`).
        for (target, background, held_bloom) in [(&black, 0.0, false), (&white, 1.0, true)] {
            renderer.core.held_bloom = held_bloom;
            draw_on(gl, texture_of, renderer, request, target, background);
        }
        renderer.core.held_bloom = false;
        let image = matte(gl, &black, &white);
        black.delete(gl);
        white.delete(gl);
        image
    }
}

/// The frame on `background` (the same grey each channel) into `target`,
/// without the grid or the Material Editor's hover, then the helpers over it.
unsafe fn draw_on(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    renderer: &mut ModelGlRenderer,
    request: &SnapshotRequest,
    target: &SceneTarget,
    background: f32,
) {
    let [width, height] = target.size;
    let mut frame = request.frame.clone();
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
        draw_helpers(gl, texture_of, &request.helpers, request.points);
    }
}

/// egui's meshes over the frame, as its painter draws them: premultiplied,
/// over what is there.
unsafe fn draw_helpers(
    gl: &glow::Context,
    texture_of: &TextureOf<'_>,
    helpers: &[egui::ClippedPrimitive],
    points: [f32; 2],
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
    let request = SnapshotRequest::new(frame.clone(), size, Vec::new(), [size[0] as f32, size[1] as f32]);
    unsafe {
        let target = SceneTarget::new(gl, size[0], size[1], Texels::RGBA16F).expect("a float target");
        renderer.core.held_bloom = held_bloom;
        draw_on(gl, &|_| None, renderer, &request, &target, background);
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
