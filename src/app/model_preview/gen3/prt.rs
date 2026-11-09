//! Halo 3's baked occlusion: each mesh's `PRT vertex type` picks an entry point
//! and `per_mesh_prt_data` holds its coefficients, a vertex at a time.
//!
//! Reach keeps only the ambient term. Its tool (`sub_140B14EB0`) derives each
//! mesh's `lighting policy` from that type: ambient lights by
//! `static_prt_ambient`, linear and quadratic are "no longer supported" and
//! light by `static_sh`. The client (`sub_1404019F0`) clamps the gray to 0..1.

use super::super::RenderModelPreview;
use super::super::common::streams::{VertexStream, VertexStreamKind};
use super::super::gl::buffer::slice_bytes;
use blam_tags::TagFile;
use eframe::glow::{self, HasContext as _};

use super::super::common::fields::elements;
use super::super::{RenderModel, preview_mesh_order};
use blam_tags::render_model::RenderMesh;

/// The most any vertex carries: quadratic's nine.
pub(crate) const COEFFICIENTS: usize = 9;

const CHANNELS: usize = 3;

/// How much of the transfer a mesh baked, as `_transfer_prt_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gen3PrtKind {
    None,
    Ambient,
    Linear,
    Quadratic,
}

impl Gen3PrtKind {
    fn from_tag(value: i128) -> Self {
        match value {
            1 => Self::Ambient,
            2 => Self::Linear,
            3 => Self::Quadratic,
            _ => Self::None,
        }
    }

    fn coefficients(self) -> usize {
        match self {
            Self::None => 0,
            Self::Ambient => 1,
            Self::Linear => 4,
            Self::Quadratic => COEFFICIENTS,
        }
    }
}

/// A model's coefficients, parallel to the preview's vertices.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Gen3Prt {
    pub(crate) vertices: Vec<[f32; COEFFICIENTS]>,
    /// Where each mesh's vertices end, and what it baked.
    meshes: Vec<(u32, Gen3PrtKind)>,
}

impl Gen3Prt {
    /// Stay parallel to vertices that have none of this.
    pub(crate) fn pad(&mut self, count: usize) {
        self.vertices
            .resize(self.vertices.len() + count, [0.0; COEFFICIENTS]);
    }

    /// Take on an attached object's own, after everything so far.
    pub(crate) fn append(&mut self, other: &Gen3Prt) {
        let base = self.vertices.len() as u32;
        // What was padded on has no mesh, and must not read as `other`'s first.
        self.meshes.push((base, Gen3PrtKind::None));
        self.vertices.extend_from_slice(&other.vertices);
        self.meshes
            .extend(other.meshes.iter().map(|(end, kind)| (end + base, *kind)));
    }

    /// What the mesh holding `vertex` baked.
    pub(crate) fn kind_at(&self, vertex: u32) -> Gen3PrtKind {
        let found = self.meshes.iter().find(|(end, _)| vertex < *end);
        found.map_or(Gen3PrtKind::None, |(_, kind)| *kind)
    }
}

/// `render geometry`'s `per_mesh_prt_data`, or none if any mesh's data does not
/// hold exactly a vertex's worth for each of its vertices.
pub(in crate::app) fn read_gen3_prt(
    tag: &TagFile,
    model: &RenderModel,
    meshes: &[RenderMesh],
) -> Option<Gen3Prt> {
    let geometry = tag.root().descend("render geometry")?;
    let tag_meshes = elements(geometry, "meshes");
    let data = elements(geometry, "per_mesh_prt_data*");
    if tag_meshes.len() != meshes.len() || data.len() != meshes.len() {
        return None;
    }
    // The tags on disk leave the policy at 0; only its field marks the rules.
    let reach = tag_meshes
        .first()
        .is_some_and(|mesh| mesh.field("lighting policy").is_some());
    let mut per_mesh = Vec::with_capacity(meshes.len());
    for ((mesh, tag_mesh), element) in meshes.iter().zip(&tag_meshes).zip(&data) {
        let kind = Gen3PrtKind::from_tag(tag_mesh.read_int_any("PRT vertex type").unwrap_or(0));
        let kind = if reach && kind != Gen3PrtKind::Ambient {
            Gen3PrtKind::None
        } else {
            kind
        };
        let bytes = element
            .field("mesh pca data")
            .and_then(|field| field.as_data())
            .unwrap_or(&[]);
        let mut prt = Gen3Prt::default();
        prt.pad(mesh.vertices.len());
        // Every coefficient of red, then green, then blue; the shaders read
        // one gray channel.
        let each = kind.coefficients() * CHANNELS * size_of::<f32>();
        let fits = each > 0 && bytes.len() == each * mesh.vertices.len();
        if fits {
            let count = kind.coefficients();
            for (vertex, chunk) in prt.vertices.iter_mut().zip(bytes.chunks_exact(each)) {
                let word = |at: usize| {
                    f32::from_le_bytes([chunk[at], chunk[at + 1], chunk[at + 2], chunk[at + 3]])
                };
                for (coefficient, value) in vertex.iter_mut().take(count).enumerate() {
                    let sum: f32 = (0..CHANNELS)
                        .map(|channel| word((channel * count + coefficient) * 4))
                        .sum();
                    *value = sum / CHANNELS as f32;
                    if reach {
                        *value = value.clamp(0.0, 1.0);
                    }
                }
            }
            // The transfer lies in the same frame as the mesh's own normals,
            // rigid or skinned; the shader turns the light for any pose.
            for vertex in &mut prt.vertices {
                // `static_prt_linear_vs` unpacks its four from a byte stream.
                if kind == Gen3PrtKind::Linear {
                    vertex[..4]
                        .iter_mut()
                        .for_each(|value| *value = (*value + 1.0) * 0.5);
                }
            }
        }
        let kind = if fits { kind } else { Gen3PrtKind::None };
        per_mesh.push((prt.vertices, kind));
    }
    let order = preview_mesh_order(model, meshes.len());
    Some(in_preview_order(
        &per_mesh,
        order.iter().map(|(_, _, mesh)| *mesh),
    ))
}

/// Each mesh's coefficients laid out as the preview lays out its vertices.
fn in_preview_order(
    per_mesh: &[(Vec<[f32; COEFFICIENTS]>, Gen3PrtKind)],
    order: impl Iterator<Item = usize>,
) -> Gen3Prt {
    let mut prt = Gen3Prt::default();
    for (vertices, kind) in order.filter_map(|mesh| per_mesh.get(mesh)) {
        prt.vertices.extend_from_slice(vertices);
        prt.meshes.push((prt.vertices.len() as u32, *kind));
    }
    prt
}

/// The last five coefficients' functions as `n * M * n`, in the game's order
/// and signs: what `sh_inverse_rotate_45678_monochrome` turns the light by.
/// Each has a squared norm of 2.
fn second_order_basis() -> [[[f32; 3]; 3]; 5] {
    let root = 3.0_f32.sqrt().recip();
    [
        [[0.0, -1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        [[root, 0.0, 0.0], [0.0, root, 0.0], [0.0, 0.0, -2.0 * root]],
        [[0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]],
    ]
}

/// What carries the last five coefficients through a rotation, the row and
/// column of each function being its inner product with the turned other.
pub(in crate::app::model_preview) fn second_order_rotation(
    rotation: [[f32; 3]; 3],
) -> [[f32; 5]; 5] {
    let product = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] {
        std::array::from_fn(|row| {
            std::array::from_fn(|column| (0..3).map(|at| a[row][at] * b[at][column]).sum())
        })
    };
    let transpose = std::array::from_fn(|row| std::array::from_fn(|column| rotation[column][row]));
    let basis = second_order_basis();
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            let turned = product(product(rotation, basis[column]), transpose);
            let both = product(basis[row], turned);
            (both[0][0] + both[1][1] + both[2][2]) / 2.0
        })
    })
}

/// The gen3 shaders' baked PRT coefficients, beside the model's vertices.
pub(in crate::app::model_preview) struct Gen3PrtStream;

impl VertexStream for Gen3PrtStream {
    fn kind(&self) -> VertexStreamKind {
        VertexStreamKind::Gen3Prt
    }

    fn present(&self, preview: &RenderModelPreview) -> bool {
        preview.gen3.prt.is_some()
    }

    unsafe fn upload(
        &self,
        gl: &glow::Context,
        preview: &RenderModelPreview,
        buffer: glow::NativeBuffer,
    ) {
        unsafe {
            match &preview.gen3.prt {
                Some(prt) => {
                    gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                    gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        slice_bytes(&prt.vertices),
                        glow::STATIC_DRAW,
                    );
                    let stride = std::mem::size_of::<[f32; COEFFICIENTS]>() as i32;
                    for (index, attribute) in
                        super::pipeline::PRT_ATTRIBUTES.into_iter().enumerate()
                    {
                        gl.enable_vertex_attrib_array(attribute);
                        // The first holds a fourth for the linear shader, in the
                        // second's first place.
                        let size = if index == 0 { 4 } else { 3 };
                        gl.vertex_attrib_pointer_f32(
                            attribute,
                            size,
                            glow::FLOAT,
                            false,
                            stride,
                            index as i32 * 12,
                        );
                    }
                }
                None => {
                    for attribute in super::pipeline::PRT_ATTRIBUTES {
                        gl.disable_vertex_attrib_array(attribute);
                    }
                }
            }
        }
    }

    fn pad(&self, preview: &mut RenderModelPreview, count: usize) {
        if let Some(prt) = preview.gen3.prt.as_mut() {
            prt.pad(count);
        }
    }

    /// An attached object's own coefficients where it has them for every
    /// vertex; else padding.
    fn append_attached(
        &self,
        preview: &mut RenderModelPreview,
        attached: &RenderModelPreview,
        count: usize,
    ) {
        if let Some(prt) = preview.gen3.prt.as_mut() {
            match attached
                .gen3
                .prt
                .as_ref()
                .filter(|own| own.vertices.len() == count)
            {
                Some(own) => prt.append(own),
                None => prt.pad(count),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluate(coefficients: [f32; 5], n: [f32; 3]) -> f32 {
        let term = |m: &[[f32; 3]; 3]| -> f32 {
            (0..3)
                .map(|i| (0..3).map(|j| n[i] * m[i][j] * n[j]).sum::<f32>())
                .sum()
        };
        coefficients
            .iter()
            .zip(&second_order_basis())
            .map(|(c, m)| c * term(m))
            .sum()
    }

    fn turned(matrix: &[[f32; 5]; 5], coefficients: [f32; 5]) -> [f32; 5] {
        std::array::from_fn(|row| {
            matrix[row]
                .iter()
                .zip(&coefficients)
                .map(|(a, b)| a * b)
                .sum()
        })
    }

    #[test]
    fn a_quarter_turn_about_z_negates_xy_and_swaps_yz_with_xz() {
        let quarter = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let result = turned(&second_order_rotation(quarter), [1.0, 2.0, 3.0, 4.0, 5.0]);
        for (got, want) in result.iter().zip([-1.0, 4.0, 3.0, -2.0, -5.0]) {
            assert!((got - want).abs() < 1e-5, "{result:?}");
        }
    }

    #[test]
    fn a_turned_transfer_at_a_turned_direction_keeps_its_value() {
        let (sin, cos) = 1.1_f32.sin_cos();
        let about_x = [[1.0, 0.0, 0.0], [0.0, cos, -sin], [0.0, sin, cos]];
        let about_y = [[cos, 0.0, sin], [0.0, 1.0, 0.0], [-sin, 0.0, cos]];
        let stored = [0.3, -0.7, 0.2, 0.9, -0.4];
        let n = [0.48, 0.6, 0.64];
        let at = |r: [[f32; 3]; 3]| -> [f32; 3] {
            std::array::from_fn(|i| (0..3).map(|j| r[i][j] * n[j]).sum())
        };
        let result = turned(&second_order_rotation(about_x), stored);
        assert!((evaluate(result, at(about_x)) - evaluate(stored, n)).abs() < 1e-5);
        assert!((evaluate(result, at(about_y)) - evaluate(stored, n)).abs() > 0.01);
        assert!((evaluate(stored, at(about_x)) - evaluate(stored, n)).abs() > 0.01);
    }

    #[test]
    fn a_vertex_finds_its_mesh_and_past_the_end_there_is_none() {
        let prt = Gen3Prt {
            vertices: Vec::new(),
            meshes: vec![(4, Gen3PrtKind::Ambient), (9, Gen3PrtKind::Quadratic)],
        };
        assert_eq!(prt.kind_at(3), Gen3PrtKind::Ambient);
        assert_eq!(prt.kind_at(4), Gen3PrtKind::Quadratic);
        assert_eq!(prt.kind_at(9), Gen3PrtKind::None);
    }

    #[test]
    fn coefficients_follow_the_previews_order_not_the_tags() {
        let vertex = |value: f32| [value; COEFFICIENTS];
        let per_mesh = [
            (vec![vertex(0.0), vertex(0.5)], Gen3PrtKind::Ambient),
            (vec![vertex(1.0)], Gen3PrtKind::Quadratic),
        ];
        let prt = in_preview_order(&per_mesh, [1, 0].into_iter());
        assert_eq!(prt.vertices[0][0], 1.0);
        assert_eq!(prt.vertices[2][0], 0.5);
        assert_eq!(prt.kind_at(0), Gen3PrtKind::Quadratic);
        assert_eq!(prt.kind_at(1), Gen3PrtKind::Ambient);
    }

    #[test]
    fn a_mesh_two_permutations_share_appears_twice() {
        let per_mesh = [(vec![[0.0; COEFFICIENTS]], Gen3PrtKind::Ambient)];
        assert_eq!(
            in_preview_order(&per_mesh, [0, 0].into_iter())
                .vertices
                .len(),
            2
        );
    }

    #[test]
    fn an_attached_object_keeps_its_transfer_and_padding_stays_empty() {
        let mut prt = in_preview_order(
            &[(vec![[0.25; COEFFICIENTS]; 2], Gen3PrtKind::Linear)],
            [0].into_iter(),
        );
        prt.pad(3);
        let other = in_preview_order(
            &[(vec![[0.5; COEFFICIENTS]; 4], Gen3PrtKind::Ambient)],
            [0].into_iter(),
        );
        prt.append(&other);
        assert_eq!(prt.vertices.len(), 9);
        assert_eq!(prt.vertices[5][0], 0.5);
        assert_eq!(prt.kind_at(1), Gen3PrtKind::Linear);
        assert_eq!(prt.kind_at(4), Gen3PrtKind::None, "padding");
        assert_eq!(prt.kind_at(5), Gen3PrtKind::Ambient);
        assert_eq!(prt.kind_at(9), Gen3PrtKind::None);
    }

    #[test]
    fn padding_keeps_parallel() {
        let mut prt = Gen3Prt::default();
        prt.pad(3);
        assert_eq!(prt.vertices.len(), 3);
    }
}
