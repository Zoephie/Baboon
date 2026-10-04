//! Reading a World Partition cell into placed meshes.
//! It owns turning a cell's exports into world-space placements; the container
//! format belongs to `blam-tags`, and presentation and export belong elsewhere.
//!
//! A Campaign Evolved level is not a package. `C10` is a persistent level plus
//! ~2,300 `_Generated_` cell packages, each holding a handful of actors, so
//! "the level" is the union of what those cells place. Two kinds of export
//! carry geometry, and they hide their placements in different places:
//!
//! - a `StaticMeshComponent` names its mesh and its transform as ordinary
//!   reflected properties;
//! - an `InstancedStaticMeshComponent` names its mesh the same way but writes
//!   its placements past the property block, where only
//!   [`read_instance_transforms`] can reach them.
//!
//! Both attach into a component hierarchy, so a component's own
//! `RelativeLocation` means nothing until it is composed with its parents'.

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use blam_tags::iostore::asset::level::read_instance_transforms;
use blam_tags::iostore::object::export::ExportBlock;
use blam_tags::iostore::object::native::NativeStruct;
use blam_tags::iostore::object::value::{PropValue, PropertyBlock};
use blam_tags::iostore::package::imports::{ImportSlot, import_slot_of, read_import_slots};

/// How deep an `AttachParent` chain may go before it is treated as a cycle.
const MAX_ATTACH_DEPTH: usize = 32;

/// A row-major transform with the translation in `[12..15]`, which is Unreal's
/// own `FMatrix` convention — and therefore the one the instance array already
/// hands back, so placements from both sources compose the same way.
pub(in crate::app) type WorldMatrix = [f64; 16];

pub(in crate::app) const IDENTITY: WorldMatrix = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// One mesh placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct MeshPlacement {
    /// Index into [`LevelScene::meshes`], so a mesh used a thousand times is
    /// named once — which is also exactly the shape an ASS scene wants.
    pub(in crate::app) mesh: usize,
    pub(in crate::app) world: WorldMatrix,
}

/// What a cell could not contribute, and why.
///
/// Counted rather than silently dropped: a level that quietly loses a third of
/// its props looks like a broken exporter, and the honest answer is that those
/// meshes are not in the cell to be found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct LevelSkips {
    /// The component inherits its mesh from a Blueprint class default, which is
    /// in another package entirely.
    pub(in crate::app) inherited_mesh: usize,
    /// A `StaticMesh` reference that did not resolve to an imported package.
    pub(in crate::app) unresolved_mesh: usize,
    /// An instanced component whose placement array did not parse.
    pub(in crate::app) unreadable_instances: usize,
    /// A cell package that could not be read at all.
    pub(in crate::app) unreadable_cells: usize,
    /// Cells whose reader thread panicked, taking everything it had read
    /// from them with it.
    pub(in crate::app) lost_cells: usize,
}

impl LevelSkips {
    #[cfg(test)]
    pub(in crate::app) fn total(&self) -> usize {
        self.inherited_mesh + self.unresolved_mesh + self.unreadable_instances
    }

    fn absorb(&mut self, other: LevelSkips) {
        self.inherited_mesh += other.inherited_mesh;
        self.unresolved_mesh += other.unresolved_mesh;
        self.unreadable_instances += other.unreadable_instances;
        self.unreadable_cells += other.unreadable_cells;
        self.lost_cells += other.lost_cells;
    }

    /// What was left out of an export and why, for the line that reports it.
    /// `None` when nothing was.
    pub(in crate::app) fn summary(&self) -> Option<String> {
        let parts: Vec<String> = [
            (self.lost_cells, "cell(s) lost to a reader crash"),
            (self.unreadable_cells, "unreadable cell(s)"),
            (self.unreadable_instances, "unreadable instance array(s)"),
            (
                self.inherited_mesh,
                "placement(s) whose mesh is set on a Blueprint",
            ),
            (self.unresolved_mesh, "placement(s) with an unresolved mesh"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("{count} {what}"))
        .collect();
        (!parts.is_empty()).then(|| format!("Left out: {}", parts.join(", ")))
    }
}

/// Every mesh placement gathered from one or more cells.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct LevelScene {
    /// Unique mesh packages, in first-seen order.
    pub(in crate::app) meshes: Vec<String>,
    /// The same names, for lookup. A level places a quarter of a million meshes
    /// drawn from several hundred, and scanning the list for each one is a
    /// hundred million string comparisons for an answer a map gives directly.
    by_mesh: HashMap<String, usize>,
    pub(in crate::app) placements: Vec<MeshPlacement>,
    pub(in crate::app) skipped: LevelSkips,
    /// Cells that were read into this scene.
    pub(in crate::app) cells: usize,
}

impl LevelScene {
    fn mesh_index(&mut self, package: &str) -> usize {
        if let Some(index) = self.by_mesh.get(package) {
            return *index;
        }
        self.meshes.push(package.to_owned());
        let index = self.meshes.len() - 1;
        self.by_mesh.insert(package.to_owned(), index);
        index
    }

    /// Fold another scene's placements into this one, keeping first-seen mesh
    /// order.
    ///
    /// Cells are read in parallel but absorbed in cell order, so the result is
    /// the one a single-threaded read would have produced — mesh indices
    /// included. A scene that depended on which thread finished first would
    /// export differently every run.
    pub(in crate::app) fn absorb(&mut self, other: LevelScene) {
        let remap: Vec<usize> = other
            .meshes
            .iter()
            .map(|package| self.mesh_index(package))
            .collect();
        self.placements
            .extend(other.placements.into_iter().map(|placement| MeshPlacement {
                mesh: remap[placement.mesh],
                world: placement.world,
            }));
        self.skipped.absorb(other.skipped);
        self.cells += other.cells;
    }

    fn place(&mut self, package: &str, world: WorldMatrix) {
        let mesh = self.mesh_index(package);
        self.placements.push(MeshPlacement { mesh, world });
    }
}

/// Multiply two row-vector transforms: the result applies `a`, then `b`.
pub(in crate::app) fn multiply(a: &WorldMatrix, b: &WorldMatrix) -> WorldMatrix {
    let mut out = [0f64; 16];
    for row in 0..4 {
        for column in 0..4 {
            out[row * 4 + column] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum();
        }
    }
    out
}

/// Build the transform a component's location, rotation and scale describe.
///
/// The rotation is an `FRotator` in degrees, stored as (pitch, yaw, roll) —
/// rotations about Y, Z and X respectively, which is not the order the numbers
/// suggest and is the easiest thing here to get quietly wrong.
pub(in crate::app) fn compose(
    translation: [f64; 3],
    rotation_degrees: [f64; 3],
    scale: [f64; 3],
) -> WorldMatrix {
    let (pitch, yaw, roll) = (
        rotation_degrees[0].to_radians(),
        rotation_degrees[1].to_radians(),
        rotation_degrees[2].to_radians(),
    );
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();

    // FRotationMatrix, with each basis row then scaled by that axis's scale.
    let basis = [
        [cp * cy, cp * sy, sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp],
        [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp],
    ];
    let mut matrix = IDENTITY;
    for row in 0..3 {
        for column in 0..3 {
            matrix[row * 4 + column] = basis[row][column] * scale[row];
        }
    }
    matrix[12] = translation[0];
    matrix[13] = translation[1];
    matrix[14] = translation[2];
    matrix
}

fn vec3(value: &PropValue) -> Option<[f64; 3]> {
    match value {
        PropValue::Native(NativeStruct::Vec3d(v)) => Some(*v),
        PropValue::Native(NativeStruct::Vec3f(v)) => Some([v[0] as f64, v[1] as f64, v[2] as f64]),
        _ => None,
    }
}

fn property<'a>(block: &'a PropertyBlock, name: &str) -> Option<&'a PropValue> {
    block
        .entries
        .iter()
        .find(|entry| &*entry.name == name)
        .map(|entry| &entry.value)
}

fn reflected(export: &ChimpExport) -> Option<&PropertyBlock> {
    match &export.decoded {
        Ok(decoded) => match &decoded.block {
            ExportBlock::Reflected(block) => Some(block),
            _ => None,
        },
        Err(_) => None,
    }
}

/// The component's own transform, before its parents are applied.
fn local_transform(block: &PropertyBlock) -> WorldMatrix {
    let translation = property(block, "RelativeLocation")
        .and_then(vec3)
        .unwrap_or([0.0; 3]);
    let rotation = property(block, "RelativeRotation")
        .and_then(vec3)
        .unwrap_or([0.0; 3]);
    let scale = property(block, "RelativeScale3D")
        .and_then(vec3)
        .unwrap_or([1.0; 3]);
    compose(translation, rotation, scale)
}

/// Compose a component's transform with every parent it attaches to.
fn world_transform(exports: &[ChimpExport], index: usize) -> WorldMatrix {
    let mut matrix = IDENTITY;
    let mut at = Some(index);
    let mut seen = HashSet::new();
    let mut depth = 0;
    while let Some(current) = at {
        if depth >= MAX_ATTACH_DEPTH || !seen.insert(current) {
            break;
        }
        depth += 1;
        let Some(block) = exports.get(current).and_then(reflected) else {
            break;
        };
        matrix = multiply(&matrix, &local_transform(block));
        // A positive FPackageIndex is an export in this same package; anything
        // else attaches outside the cell, which cooked components do not do.
        at = match property(block, "AttachParent") {
            Some(PropValue::Object(parent)) if *parent > 0 => Some(*parent as usize - 1),
            _ => None,
        };
    }
    matrix
}

/// The package a component's `StaticMesh` property points at.
fn mesh_package(block: &PropertyBlock, slots: &[ImportSlot]) -> Result<String, ()> {
    let Some(PropValue::Object(index)) = property(block, "StaticMesh") else {
        return Err(());
    };
    let slot = import_slot_of(*index).ok_or(())?;
    match slots.get(slot) {
        Some(ImportSlot::Package(target)) => Ok(target.package.clone()),
        _ => Err(()),
    }
}

/// Read one World Partition cell into `scene`.
///
/// Appends rather than returns, because a level is the union of thousands of
/// cells and callers accumulate them.
pub(in crate::app) fn read_cell_into(cell: &ChimpPackage, scene: &mut LevelScene) {
    let slots = read_import_slots(&cell.header).unwrap_or_default();
    let mut skips = LevelSkips::default();

    for (index, export) in cell.exports.iter().enumerate() {
        let class = export.class.as_deref().unwrap_or_default();
        let instanced = class == "InstancedStaticMeshComponent";
        if class != "StaticMeshComponent" && !instanced {
            continue;
        }
        let Some(block) = reflected(export) else {
            continue;
        };
        let package = match mesh_package(block, &slots) {
            Ok(package) => package,
            // No `StaticMesh` of its own: a Blueprint subobject taking the mesh
            // from its class default, which is not in this package to read.
            Err(()) => {
                if property(block, "StaticMesh").is_some() {
                    skips.unresolved_mesh += 1;
                } else {
                    skips.inherited_mesh += 1;
                }
                continue;
            }
        };
        let component = world_transform(&cell.exports, index);
        if !instanced {
            scene.place(&package, component);
            continue;
        }
        let Ok(decoded) = &export.decoded else {
            skips.unreadable_instances += 1;
            continue;
        };
        match read_instance_transforms(&decoded.tail) {
            Ok(instances) => {
                for instance in instances {
                    // Instance transforms are relative to their component.
                    scene.place(&package, multiply(&instance, &component));
                }
            }
            Err(_) => skips.unreadable_instances += 1,
        }
    }

    scene.skipped.absorb(skips);
    scene.cells += 1;
}

/// A chunk's scene, or — if its reader panicked — a scene that says its
/// cells were lost. A panicked chunk used to come back as an empty scene, and
/// its cells vanished from the export without a word.
fn chunk_scene(cells: usize, joined: std::thread::Result<LevelScene>) -> LevelScene {
    joined.unwrap_or_else(|_| LevelScene {
        cells,
        skipped: LevelSkips {
            lost_cells: cells,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// Read every cell of a level, using the machine rather than one core of it.
///
/// Cells are independent — each is a package decoded on its own — and there are
/// thousands of them, so this is the difference between a quarter of an hour and
/// a couple of minutes. Reading C10 sequentially takes 13 of the 25 minutes a
/// full export costs.
///
/// The work is split into *contiguous* chunks and absorbed in chunk order, so
/// the scene is byte-identical to a sequential read. Handing out cells one at a
/// time would balance better and make mesh order depend on thread scheduling,
/// which would export a different file every run.
pub(in crate::app) fn read_cells(
    world: &World,
    cells: &[String],
    threads: usize,
    progress: &(dyn Fn(usize) + Sync),
) -> LevelScene {
    let threads = threads.clamp(1, 64).min(cells.len().max(1));
    let done = AtomicUsize::new(0);
    let chunk = cells.len().div_ceil(threads.max(1));
    let mut parts: Vec<LevelScene> = std::thread::scope(|scope| {
        let handles: Vec<_> = cells
            .chunks(chunk.max(1))
            .map(|slice| {
                let done = &done;
                let handle = scope.spawn(move || {
                    let mut scene = LevelScene::default();
                    for cell in slice {
                        if let Ok(document) = crate::app::chimp::load_chimp_package(world, cell) {
                            read_cell_into(&document, &mut scene);
                        } else {
                            // Still counted: the caller asked for this many
                            // cells and a progress bar that stalls on the
                            // unreadable ones is lying about where it is.
                            scene.cells += 1;
                            scene.skipped.unreadable_cells += 1;
                        }
                        let seen = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if seen % 32 == 0 {
                            progress(seen);
                        }
                    }
                    scene
                });
                (slice.len(), handle)
            })
            .collect();
        handles
            .into_iter()
            .map(|(cells, handle)| chunk_scene(cells, handle.join()))
            .collect()
    });
    progress(cells.len());

    // Merging into the first part rather than an empty scene keeps the largest
    // chunk's placements where they are instead of copying them.
    let mut scene = if parts.is_empty() {
        LevelScene::default()
    } else {
        parts.remove(0)
    };
    for part in parts {
        scene.absorb(part);
    }
    scene
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scaling_probe_tests;

#[cfg(test)]
mod real_data_tests;
