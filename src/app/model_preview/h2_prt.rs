//! Halo 2 precomputed radiance transfer: what the `sh prt` layer needs that a
//! tag only holds in raw form. The game turns the render_model's `PRT info`
//! into per-vertex data and per-cluster constants at load; see
//! `prt_project_cluster_basis` and `prt_setup_draw`.

use std::collections::HashMap;

use blam_tags::{RenderMesh, RenderModel, TagFile, TagStruct};

use super::renderer::preview_mesh_order;

/// `info->m_sh_order` is asserted to be 3 in the game.
const SH_ORDER: i128 = 3;
const SH_COEFFICIENTS: usize = 9;
/// Red, green and blue coefficients, for the mean and for each PCA vector.
const CLUSTER_VECTOR: usize = SH_COEFFICIENTS * 3;
/// `pca data offset` counts floats: a cluster id and four weights a vertex.
const VERTEX_FLOATS: usize = 5;
/// The vertex stage takes exactly four PCA vectors, in one `vec4` each.
const PCA_VECTORS: usize = 4;
/// Mean, then red, green and blue weights.
#[cfg(test)]
const CLUSTER_REGISTERS: usize = 4;
/// The size of the vertex stage's constant array: ten clusters on each of the
/// scarab's three LODs, the most in the kit. Beside the bone palette, twice
/// this is more than a driver links.
pub(crate) const H2_PRT_REGISTERS: usize = 120;

/// What a model's `sh prt` passes draw with.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct H2Prt {
    /// Parallel to the preview's vertices: cluster, then four PCA weights.
    /// Clusters count across LODs, which each have their own basis.
    pub(crate) vertices: Vec<[f32; VERTEX_FLOATS]>,
    pub(crate) clusters: Vec<[f32; 4]>,
    pub(crate) light: [f32; 3],
    /// Every LOD's clusters in order, for lighting them again.
    basis: Vec<f32>,
}

impl H2Prt {
    /// Stay parallel to vertices that have none of this.
    pub(crate) fn pad(&mut self, count: usize) {
        self.vertices.resize(self.vertices.len() + count, [0.0; VERTEX_FLOATS]);
    }

    /// The constants of every cluster under `lights` and an ambient color.
    pub(crate) fn clusters_lit_by(&self, lights: &[H2PrtLight], ambient: [f32; 3]) -> Vec<[f32; 4]> {
        let mut coefficients = sh_project(lights);
        // A uniform light holds only the first band, at root(4 pi) a unit.
        let band = 2.0 * std::f32::consts::PI.sqrt();
        for (channel, ambient) in coefficients.iter_mut().zip(ambient) {
            channel[0] += ambient * band;
        }
        cluster_constants(&self.basis, &coefficients)
    }
}

/// One light of what stands in for a level's light probes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2PrtLight {
    pub(crate) direction: [f32; 3],
    pub(crate) color: [f32; 3],
}

/// `sh_evaluate_direction`, for order 3.
fn sh_evaluate(direction: [f32; 3]) -> [f32; SH_COEFFICIENTS] {
    let [x, y, z] = direction;
    let inverse_root_pi = 1.0 / std::f32::consts::PI.sqrt();
    let root_3 = 3f32.sqrt() * inverse_root_pi;
    let root_15 = 15f32.sqrt() * inverse_root_pi;
    let root_5 = 5f32.sqrt() * inverse_root_pi;
    [
        inverse_root_pi * 0.5,
        root_3 * y * -0.5,
        root_3 * z * 0.5,
        root_3 * x * -0.5,
        root_15 * y * x * 0.5,
        root_15 * y * z * -0.5,
        root_5 * (z * z * 3.0 - 1.0) * 0.25,
        root_15 * z * x * -0.5,
        root_15 * (x * x - y * y) * 0.25,
    ]
}

/// `sh_project_directional`, for order 3, summed over `lights`.
fn sh_project(lights: &[H2PrtLight]) -> [[f32; SH_COEFFICIENTS]; 3] {
    let mut channels = [[0.0; SH_COEFFICIENTS]; 3];
    for light in lights {
        let shape = sh_evaluate(light.direction);
        for (channel, color) in channels.iter_mut().zip(light.color) {
            for (coefficient, value) in channel.iter_mut().zip(shape) {
                *coefficient += value * color * (std::f32::consts::PI / 1.0625);
            }
        }
    }
    channels
}

fn sh_dot(basis: &[f32], light: &[f32; SH_COEFFICIENTS]) -> f32 {
    basis.iter().zip(light).map(|(basis, light)| basis * light).sum()
}

/// `prt_project_cluster_basis`: a cluster's mean, then its red, green and blue
/// PCA weights, one register each.
fn cluster_constants(basis: &[f32], light: &[[f32; SH_COEFFICIENTS]; 3]) -> Vec<[f32; 4]> {
    let cluster_size = CLUSTER_VECTOR * (PCA_VECTORS + 1);
    let channel = |vector: usize, channel: usize| {
        move |cluster: &[f32]| {
            let start = vector * CLUSTER_VECTOR + channel * SH_COEFFICIENTS;
            sh_dot(&cluster[start..start + SH_COEFFICIENTS], &light[channel])
        }
    };
    let mut registers = Vec::new();
    for cluster in basis.chunks_exact(cluster_size) {
        registers.push([
            channel(0, 0)(cluster),
            channel(0, 1)(cluster),
            channel(0, 2)(cluster),
            0.0,
        ]);
        for color in 0..3 {
            registers.push(std::array::from_fn(|vector| channel(vector + 1, color)(cluster)));
        }
    }
    registers
}

fn reals(info: &TagStruct<'_>, block: &str, field: &str) -> Option<Vec<f32>> {
    let block = info.field(block)?.as_block()?;
    (0..block.len()).map(|index| block.element(index)?.read_real(field)).collect()
}

/// Read a Halo 2 render_model's PRT data, walking `meshes` in the order the
/// preview appended them. `None` for a model without any, and for one whose
/// data does not fit what the vertex stage takes.
pub(in crate::app) fn read_h2_prt(
    tag: &TagFile,
    model: &RenderModel,
    meshes: &[RenderMesh],
    lights: &[H2PrtLight],
) -> Option<H2Prt> {
    let root = tag.root();
    let info = root.field("PRT info")?.as_block()?;
    let info = info.element(0)?;
    if info.read_int_any("SH Order*")? != SH_ORDER
        || info.read_int_any("pca vectors per cluster*")? != PCA_VECTORS as i128
    {
        return None;
    }
    let cluster_count = usize::try_from(info.read_int_any("num of clusters*")?).ok()?;
    let basis = reals(&info, "cluster basis*", "basis data")?;
    let raw = reals(&info, "raw_pca_data*", "raw pca data")?;
    let light_coefficients = sh_project(lights);
    let cluster_floats = cluster_count * CLUSTER_VECTOR * (PCA_VECTORS + 1);

    let mut clusters = Vec::new();
    let mut all_basis = Vec::new();
    // Section index to where its data starts and the first cluster of its LOD.
    let mut sections = HashMap::new();
    let lods = info.field("lod info*")?.as_block()?;
    for lod_index in 0..lods.len() {
        let lod = lods.element(lod_index)?;
        let offset = usize::try_from(lod.read_int_any("cluster offset*")?).ok()?;
        let first_cluster = lod_index * cluster_count;
        let lod_basis = basis.get(offset..offset + cluster_floats)?;
        all_basis.extend_from_slice(lod_basis);
        clusters.extend(cluster_constants(lod_basis, &light_coefficients));
        let block = lod.field("section info*")?.as_block()?;
        for index in 0..block.len() {
            let section = block.element(index)?;
            sections.insert(
                usize::try_from(section.read_int_any("section index*")?).ok()?,
                (usize::try_from(section.read_int_any("pca data offset*")?).ok()?, first_cluster),
            );
        }
    }
    if clusters.len() > H2_PRT_REGISTERS {
        return None;
    }

    let mut vertices = Vec::new();
    for (_, _, mesh_index) in preview_mesh_order(model, meshes.len()) {
        let section = sections.get(&mesh_index);
        for vertex in 0..meshes[mesh_index].vertices.len() {
            let floats = section.and_then(|(offset, first_cluster)| {
                let start = offset + vertex * VERTEX_FLOATS;
                let data = raw.get(start..start + VERTEX_FLOATS)?;
                Some([data[0] + *first_cluster as f32, data[1], data[2], data[3], data[4]])
            });
            vertices.push(floats.unwrap_or_default());
        }
    }
    Some(H2Prt { vertices, clusters, light: lights.first()?.direction, basis: all_basis })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_matches_the_constant_band_and_the_signs_the_game_uses() {
        let up = sh_evaluate([0.0, 0.0, 1.0]);
        assert!((up[0] - 0.282_095).abs() < 1e-5);
        assert!((up[2] - 0.488_603).abs() < 1e-5);
        assert_eq!((up[1], up[3]), (0.0, 0.0));
        assert!((up[6] - 0.630_783).abs() < 1e-5);
        let right = sh_evaluate([1.0, 0.0, 0.0]);
        assert!((right[3] + 0.488_603).abs() < 1e-5);
    }

    #[test]
    fn relighting_keeps_what_each_light_adds_apart() {
        let mut basis = vec![0.0; CLUSTER_VECTOR * (PCA_VECTORS + 1)];
        basis[0] = 1.0;
        let prt = H2Prt { basis, ..H2Prt::default() };
        let light = H2PrtLight { direction: [0.0, 0.0, 1.0], color: [1.0; 3] };
        let lit = prt.clusters_lit_by(&[light], [0.0; 3])[0][0];
        let ambient = prt.clusters_lit_by(&[], [0.5; 3])[0][0];
        assert!(lit > 0.0);
        assert!((ambient - std::f32::consts::PI.sqrt()).abs() < 1e-5);
        let both = prt.clusters_lit_by(&[light], [0.5; 3])[0][0];
        assert!((both - lit - ambient).abs() < 1e-5);
    }

    #[test]
    fn a_cluster_holds_its_mean_then_a_weight_register_per_channel() {
        let mut basis = vec![0.0; CLUSTER_VECTOR * (PCA_VECTORS + 1)];
        let light = [[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 3];
        // Mean red, green and blue.
        basis[0] = 1.0;
        basis[SH_COEFFICIENTS] = 2.0;
        basis[SH_COEFFICIENTS * 2] = 3.0;
        // Red and blue of the second PCA vector.
        basis[CLUSTER_VECTOR * 2] = 4.0;
        basis[CLUSTER_VECTOR * 2 + SH_COEFFICIENTS * 2] = 5.0;
        let registers = cluster_constants(&basis, &light);
        assert_eq!(registers.len(), CLUSTER_REGISTERS);
        assert_eq!(registers[0], [1.0, 2.0, 3.0, 0.0]);
        assert_eq!(registers[1], [0.0, 4.0, 0.0, 0.0]);
        assert_eq!(registers[2], [0.0; 4]);
        assert_eq!(registers[3], [0.0, 5.0, 0.0, 0.0]);
    }
}
