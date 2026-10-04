//! Splitting a level's placements into segments an importer can actually open.
//! It owns the spatial division and its budgets; reading cells belongs to
//! [`super::level`], and writing files belongs to [`super::level_export`].
//!
//! A whole level does not import. Measured against Blender: 56 million
//! triangles across 14,049 placements opens, and 109.7 million across 296,399
//! does not. Two different things run out, so a segment is bounded by both — a
//! dense stand of foliage can hold a placement for every leaf while reusing a
//! handful of meshes, and a segment budgeted only on geometry would sail past
//! the object count that is the actual ceiling there.
//!
//! The division is spatial, and has to be. Cells arrive in the order the pak
//! lists them, which carries no geometric meaning at all: 27 cells read that way
//! spanned 404 x 421 x 279 m of a level that is 797 x 1137 x 448 m in total.
//! Segments are therefore cut from placement positions, by repeatedly halving
//! the widest axis at the median until both budgets are met, which follows the
//! level's own density rather than imposing a grid on it — a grid would put
//! thousands of segments across empty terrain and still overflow inside a
//! building.
//!
//! Geometry is counted per *distinct mesh*, not per placement. Every segment
//! references one shared prototype library, so what a segment costs to open is
//! the meshes it names once each, however many times it places them.

use std::collections::HashSet;

/// How far the median split will recurse before a segment is accepted as-is.
///
/// Each level halves the placements, so this is far past what any real level
/// reaches; it exists so that placements which cannot be separated — a thousand
/// props at one position — end a recursion rather than continuing it.
const MAX_SPLIT_DEPTH: usize = 24;

/// What a segment may hold before it is split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct SegmentBudget {
    /// Triangles across the distinct meshes the segment uses.
    pub(in crate::app) triangles: usize,
    pub(in crate::app) placements: usize,
}

impl Default for SegmentBudget {
    /// The measured import ceiling, with room beneath it.
    ///
    /// 30 million triangles is a size Blender was shown to open. 50,000
    /// placements is a deliberate guess rather than a measurement: 14,049 is
    /// known to work and 296,399 is known not to, and nothing in between has
    /// been tried.
    fn default() -> Self {
        Self {
            triangles: 30_000_000,
            placements: 50_000,
        }
    }
}

/// One segment: which placements it holds, and what they cost.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::app) struct Segment {
    /// Indices into `LevelScene::placements`.
    pub(in crate::app) placements: Vec<usize>,
    /// Indices into `LevelScene::meshes`, each counted once.
    pub(in crate::app) meshes: Vec<usize>,
    pub(in crate::app) triangles: usize,
    /// Set when the segment breaks a budget that no further split can fix —
    /// a single mesh larger than the whole allowance, or placements that share
    /// one position. Reported rather than hidden, because the file will be
    /// harder to open than the budget promised.
    pub(in crate::app) over_budget: bool,
}

/// A placement reduced to what the split needs: where it is, and what it uses.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct PlacedMesh {
    pub(in crate::app) mesh: usize,
    pub(in crate::app) position: [f64; 3],
}

/// Divide placements into segments that each fit the budget.
///
/// `mesh_triangles` is indexed by mesh, so the caller decodes each mesh once and
/// the split costs nothing beyond arithmetic.
pub(in crate::app) fn segment(
    placements: &[PlacedMesh],
    mesh_triangles: &[usize],
    budget: SegmentBudget,
) -> Vec<Segment> {
    if placements.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let all: Vec<usize> = (0..placements.len()).collect();
    divide(placements, mesh_triangles, budget, all, 0, &mut segments);
    segments
}

fn divide(
    placements: &[PlacedMesh],
    mesh_triangles: &[usize],
    budget: SegmentBudget,
    indices: Vec<usize>,
    depth: usize,
    out: &mut Vec<Segment>,
) {
    let segment = weigh(placements, mesh_triangles, indices);
    let fits =
        segment.placements.len() <= budget.placements && segment.triangles <= budget.triangles;
    if fits || segment.placements.len() <= 1 || depth >= MAX_SPLIT_DEPTH {
        out.push(Segment {
            over_budget: !fits,
            ..segment
        });
        return;
    }

    match halve(placements, &segment.placements) {
        Some((left, right)) => {
            divide(placements, mesh_triangles, budget, left, depth + 1, out);
            divide(placements, mesh_triangles, budget, right, depth + 1, out);
        }
        // Every placement sits at the same point, so no cut separates them.
        None => out.push(Segment {
            over_budget: true,
            ..segment
        }),
    }
}

/// What a set of placements costs: its distinct meshes and their triangles.
fn weigh(placements: &[PlacedMesh], mesh_triangles: &[usize], indices: Vec<usize>) -> Segment {
    let mut seen = HashSet::new();
    let mut meshes = Vec::new();
    let mut triangles = 0;
    for &index in &indices {
        let mesh = placements[index].mesh;
        if seen.insert(mesh) {
            meshes.push(mesh);
            triangles += mesh_triangles.get(mesh).copied().unwrap_or(0);
        }
    }
    meshes.sort_unstable();
    Segment {
        placements: indices,
        meshes,
        triangles,
        over_budget: false,
    }
}

/// Split at the median of the widest axis, or `None` if the placements occupy a
/// single point and no cut divides them.
fn halve(placements: &[PlacedMesh], indices: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for &index in indices {
        for axis in 0..3 {
            min[axis] = min[axis].min(placements[index].position[axis]);
            max[axis] = max[axis].max(placements[index].position[axis]);
        }
    }
    let axis = (0..3)
        .max_by(|&a, &b| (max[a] - min[a]).total_cmp(&(max[b] - min[b])))
        .expect("three axes");
    if !(max[axis] - min[axis]).is_finite() || max[axis] - min[axis] <= 0.0 {
        return None;
    }

    let mut sorted = indices.to_vec();
    sorted.sort_by(|&a, &b| placements[a].position[axis].total_cmp(&placements[b].position[axis]));
    let middle = sorted.len() / 2;
    let right = sorted.split_off(middle);
    // A median that lands on a run of identical coordinates can leave one side
    // empty; the bounds check above means some cut exists, so this cannot both
    // be empty and be reached.
    if sorted.is_empty() || right.is_empty() {
        return None;
    }
    Some((sorted, right))
}

#[cfg(test)]
mod tests;
