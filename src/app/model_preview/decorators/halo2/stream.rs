//! Halo 2 decorators' placements beside the model's vertices: what their
//! vertex shaders read that the preview's own vertex does not carry.
//!
//! The cache builder writes a quad's corners at its placement, each with its
//! offset from it and its axis (`sub_6B1770`: `va_billboard_offset` and
//! `va_billboard_axis`), and the vertex shader turns them to the eye. The
//! preview's own vertices hold the corners baked for Solid and Textured; the
//! decorator's `POSITION0` reads the placement from here instead.

use super::super::super::RenderModelPreview;
use super::super::super::common::streams::{VertexStream, VertexStreamKind};
use super::super::super::gl::buffer::slice_bytes;
use eframe::glow;
use eframe::glow::HasContext as _;

/// One vertex of the stream.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Halo2DecoratorVertex {
    /// `POSITION0`: a quad's placement, else the vertex where it lies.
    pub origin: [f32; 3],
    /// `COLOR0`: a sprite's or decal's colour, as the cache builder bakes it.
    pub color: [f32; 4],
    /// `TEXCOORD4`: a quad's corner's offset from its placement.
    pub offset: [f32; 3],
    /// `TEXCOORD5`: an axis-rotating quad's axis.
    pub axis: [f32; 3],
}

/// Where the stream's inputs arrive: the four the slot's vertex array has
/// free past the preview's own seven, short of the PRT streams' 11 to 15.
pub(in crate::app::model_preview) const HALO_2_STREAM_ATTRIBUTES: [(u32, &str); 4] = [
    (7, "a_POSITION0"),
    (8, "a_COLOR0"),
    (9, "a_TEXCOORD4"),
    (10, "a_TEXCOORD5"),
];

/// The stream, on a Halo 2 decorator set's preview.
pub(in crate::app::model_preview) struct Halo2DecoratorStream;

impl VertexStream for Halo2DecoratorStream {
    fn kind(&self) -> VertexStreamKind {
        VertexStreamKind::Halo2Decorator
    }

    fn present(&self, preview: &RenderModelPreview) -> bool {
        !preview.h2.decorator_vertices.is_empty()
    }

    unsafe fn upload(
        &self,
        gl: &glow::Context,
        preview: &RenderModelPreview,
        buffer: glow::NativeBuffer,
    ) {
        unsafe {
            if preview.h2.decorator_vertices.is_empty() {
                for (location, _) in HALO_2_STREAM_ATTRIBUTES {
                    gl.disable_vertex_attrib_array(location);
                }
                return;
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                slice_bytes(&preview.h2.decorator_vertices),
                glow::STATIC_DRAW,
            );
            let stride = std::mem::size_of::<Halo2DecoratorVertex>() as i32;
            let [(origin, _), (color, _), (offset, _), (axis, _)] = HALO_2_STREAM_ATTRIBUTES;
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
        if !preview.h2.decorator_vertices.is_empty() {
            let padded = preview.h2.decorator_vertices.len() + count;
            preview
                .h2
                .decorator_vertices
                .resize(padded, Halo2DecoratorVertex::default());
        }
    }
}

/// The constants Halo 2's decorator vertex shaders read (`float4 c[208]`)
/// for `frame`'s view: `c0` to `c3` the view and projection, row by row as
/// `dp3(position, c[i]) + c[i].w` takes them; `c4` to `c7` the eye's right,
/// up and forward and its place (`EYE_RIGHT`, `EYE_UP`, `EYE_FORWARD`); `c15.y`
/// the least a model's light falls to, 0 (`sub_936850`); `c53` the fade,
/// `SCALE = clamp(c53.y − distance · c53.x, c53.z, c53.w)`, held at 1 as no
/// fade; and `c203` to `c207` a model's own, set per placement.
pub(in crate::app::model_preview) fn halo_2_vertex_constants(
    frame: &super::super::super::ModelGpuFrame,
) -> Vec<[f32; 4]> {
    use super::super::super::gen3::pipeline::view_projection;
    use super::super::super::{eye_position, unrotate_view_vector};
    let mut constants = vec![[0.0; 4]; HALO_2_CONSTANTS];
    let columns = view_projection(&frame.camera);
    for (row, constant) in constants.iter_mut().take(4).enumerate() {
        *constant = std::array::from_fn(|column| columns[column * 4 + row]);
    }
    let (yaw, pitch) = (frame.camera.yaw, frame.camera.pitch);
    let axis = |vector| {
        let [x, y, z] = unrotate_view_vector(yaw, pitch, vector);
        [x, y, z, 0.0]
    };
    constants[4] = axis([1.0, 0.0, 0.0]);
    constants[5] = axis([0.0, 0.0, 1.0]);
    constants[6] = axis([0.0, 1.0, 0.0]);
    let [x, y, z] = eye_position(&frame.camera);
    constants[7] = [x, y, z, 1.0];
    constants[FADE] = [0.0, 1.0, 0.0, 1.0];
    constants
}

/// `float4 c[96*2+16]`.
const HALO_2_CONSTANTS: usize = 208;

/// `c53`, the fade.
const FADE: usize = 53;

/// `c203`, the first of a model's own five.
pub(in crate::app::model_preview) const HALO_2_RIGID_CONSTANTS: usize = 203;
