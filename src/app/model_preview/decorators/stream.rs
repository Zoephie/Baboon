//! Halo 2's decorators' and Halo CE's detail objects' placements beside the
//! model's vertices: what their vertex shaders read that the preview's own
//! vertex does not carry.
//!
//! Halo 2's cache builder writes a quad's corners at its placement, each with
//! its offset from it and its axis (`sub_6B1770`: `va_billboard_offset` and
//! `va_billboard_axis`), and the vertex shader turns them to the eye. Halo
//! CE's detail objects keep their place, colour, and frame, type and corner
//! as a second colour (`d3dve_detail_object`). The preview's own vertices
//! hold the corners baked for Solid and Textured; the shaders' `POSITION0`
//! reads the placement from here instead.

use super::super::RenderModelPreview;
use super::super::common::streams::{VertexStream, VertexStreamKind};
use super::super::gl::buffer::slice_bytes;
use eframe::glow;
use eframe::glow::HasContext as _;

/// One vertex of the stream.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DecoratorStreamVertex {
    /// `POSITION0`: a quad's or detail object's placement, else the vertex
    /// where it lies.
    pub origin: [f32; 3],
    /// `COLOR0`: a sprite's, decal's or detail object's colour, as the game
    /// bakes it.
    pub color: [f32; 4],
    /// Halo 2's `TEXCOORD4`, a quad's corner's offset from its placement; Halo
    /// CE's `COLOR1`, a detail object's frame, type and corner over 255.
    pub offset: [f32; 3],
    /// Halo 2's `TEXCOORD5`: an axis-rotating quad's axis.
    pub axis: [f32; 3],
}

/// Where the stream's inputs arrive: the four the slot's vertex array has
/// free past the preview's own seven, short of the PRT streams' 11 to 15.
/// Halo 2's `TEXCOORD4` and Halo CE's `COLOR1` share one.
pub(in crate::app::model_preview) const DECORATOR_STREAM_ATTRIBUTES: [(u32, &str); 5] = [
    (7, "a_POSITION0"),
    (8, "a_COLOR0"),
    (9, "a_TEXCOORD4"),
    (9, "a_COLOR1"),
    (10, "a_TEXCOORD5"),
];

/// The stream, on a Halo 2 decorator set's or Halo CE detail object
/// collection's preview.
pub(in crate::app::model_preview) struct DecoratorStream;

impl VertexStream for DecoratorStream {
    fn kind(&self) -> VertexStreamKind {
        VertexStreamKind::Decorator
    }

    fn present(&self, preview: &RenderModelPreview) -> bool {
        !preview.decorator_stream.is_empty()
    }

    unsafe fn upload(
        &self,
        gl: &glow::Context,
        preview: &RenderModelPreview,
        buffer: glow::NativeBuffer,
    ) {
        unsafe {
            if preview.decorator_stream.is_empty() {
                for (location, _) in DECORATOR_STREAM_ATTRIBUTES {
                    gl.disable_vertex_attrib_array(location);
                }
                return;
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&preview.decorator_stream),
                glow::STATIC_DRAW,
            );
            let stride = std::mem::size_of::<DecoratorStreamVertex>() as i32;
            let [(origin, _), (color, _), (offset, _), _, (axis, _)] = DECORATOR_STREAM_ATTRIBUTES;
            for (location, size, at) in [
                (origin, 3, 0),
                (color, 4, 12),
                (offset, 3, 28),
                (axis, 3, 40),
            ] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, at);
            }
        }
    }

    fn pad(&self, preview: &mut RenderModelPreview, count: usize) {
        if !preview.decorator_stream.is_empty() {
            let padded = preview.decorator_stream.len() + count;
            preview
                .decorator_stream
                .resize(padded, DecoratorStreamVertex::default());
        }
    }
}
