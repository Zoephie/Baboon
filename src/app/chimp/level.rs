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
}

impl LevelSkips {
    pub(in crate::app) fn total(&self) -> usize {
        self.inherited_mesh + self.unresolved_mesh + self.unreadable_instances
    }

    fn absorb(&mut self, other: LevelSkips) {
        self.inherited_mesh += other.inherited_mesh;
        self.unresolved_mesh += other.unresolved_mesh;
        self.unreadable_instances += other.unreadable_instances;
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
            out[row * 4 + column] = (0..4)
                .map(|k| a[row * 4 + k] * b[k * 4 + column])
                .sum();
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
        PropValue::Native(NativeStruct::Vec3f(v)) => {
            Some([v[0] as f64, v[1] as f64, v[2] as f64])
        }
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
                scope.spawn(move || {
                    let mut scene = LevelScene::default();
                    for cell in slice {
                        if let Ok(document) = crate::app::chimp::load_chimp_package(world, cell) {
                            read_cell_into(&document, &mut scene);
                        } else {
                            // Still counted: the caller asked for this many
                            // cells and a progress bar that stalls on the
                            // unreadable ones is lying about where it is.
                            scene.cells += 1;
                        }
                        let seen = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if seen % 32 == 0 {
                            progress(seen);
                        }
                    }
                    scene
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
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
mod tests {
    use super::*;

    fn translation_of(matrix: &WorldMatrix) -> [f64; 3] {
        [matrix[12], matrix[13], matrix[14]]
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn an_unrotated_component_places_where_it_says() {
        let matrix = compose([-38869.7, 19286.6, 12838.1], [0.0; 3], [3.0; 3]);
        assert!(close(translation_of(&matrix), [-38869.7, 19286.6, 12838.1]));
        // Scale lands on the basis rows, not the translation.
        assert!((matrix[0] - 3.0).abs() < 1e-9);
        assert!((matrix[5] - 3.0).abs() < 1e-9);
        assert!((matrix[10] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_yaw_turns_x_towards_y() {
        // Yaw is the second component of an FRotator, not the first: getting
        // that wrong rotates a level about the wrong axis and still looks
        // plausible in isolation.
        let matrix = compose([0.0; 3], [0.0, 90.0, 0.0], [1.0; 3]);
        assert!((matrix[0]).abs() < 1e-9);
        assert!((matrix[1] - 1.0).abs() < 1e-9);
        assert!((matrix[4] + 1.0).abs() < 1e-9);
        assert!((matrix[5]).abs() < 1e-9);
    }

    #[test]
    fn a_child_is_placed_through_its_parent() {
        // Applying the child first and then the parent is what puts an attached
        // component in the right place; the other order rotates the parent's
        // offset by the child's rotation.
        let parent = compose([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], [1.0; 3]);
        let child = compose([10.0, 0.0, 0.0], [0.0; 3], [1.0; 3]);
        let world = multiply(&child, &parent);
        assert!(close(translation_of(&world), [100.0, 10.0, 0.0]));
    }

    #[test]
    fn scale_multiplies_down_the_chain() {
        let parent = compose([0.0; 3], [0.0; 3], [2.0; 3]);
        let child = compose([5.0, 0.0, 0.0], [0.0; 3], [3.0; 3]);
        let world = multiply(&child, &parent);
        assert!(close(translation_of(&world), [10.0, 0.0, 0.0]));
        assert!((world[0] - 6.0).abs() < 1e-9);
    }

    #[test]
    fn identity_is_neutral() {
        let matrix = compose([1.0, 2.0, 3.0], [10.0, 20.0, 30.0], [1.5; 3]);
        assert_eq!(multiply(&matrix, &IDENTITY), matrix);
        assert_eq!(multiply(&IDENTITY, &matrix), matrix);
    }

    #[test]
    fn a_scene_names_each_mesh_once() {
        let mut scene = LevelScene::default();
        scene.place("/Game/SM_Tree", IDENTITY);
        scene.place("/Game/SM_Rock", IDENTITY);
        scene.place("/Game/SM_Tree", compose([5.0, 0.0, 0.0], [0.0; 3], [1.0; 3]));
        assert_eq!(scene.meshes, ["/Game/SM_Tree", "/Game/SM_Rock"]);
        assert_eq!(scene.placements.len(), 3);
        assert_eq!(scene.placements[2].mesh, 0);
    }

    #[test]
    fn skips_are_counted_by_reason() {
        let mut skips = LevelSkips::default();
        skips.absorb(LevelSkips {
            inherited_mesh: 2,
            unresolved_mesh: 1,
            unreadable_instances: 3,
        });
        skips.absorb(LevelSkips {
            inherited_mesh: 1,
            ..LevelSkips::default()
        });
        assert_eq!(skips.inherited_mesh, 3);
        assert_eq!(skips.total(), 7);
    }
}

#[cfg(test)]
mod scaling_probe {
    use super::*;

    /// Split one cell's cost into the steps that make it up.
    ///
    /// Reading a cell costs 329ms and the bytes arrive in under a millisecond,
    /// so the time is in decoding — but decoding is a header parse, a class
    /// lookup per export and a property decode per export, and which of those
    /// it is decides whether anything can be done about it.
    #[test]
    #[ignore]
    fn probe_cell_decode_breakdown() {
        use blam_tags::iostore::object::archive::ExportContext;
        use blam_tags::iostore::object::export::read_export_in;
        use blam_tags::iostore::package::builder::read_payloads;
        use blam_tags::iostore::package::zen::FZenPackageHeader;
        use std::io::Cursor;
        use std::time::Instant;

        let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
        let step: usize = std::env::var("BABOON_SCALE_STEP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8);
        let usmap = crate::app::chimp::load_chimp_usmap(None).expect("usmap");
        let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
        let cells: Vec<String> = world
            .packages()
            .iter()
            .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
            .map(|record| record.name.clone())
            .step_by(step)
            .collect();

        let (mut read, mut parse, mut classes, mut decode) = (0.0, 0.0, 0.0, 0.0);
        let (mut exports_seen, mut per_cell) = (0usize, Vec::new());
        for cell in &cells {
            let cell_started = Instant::now();
            let started = Instant::now();
            let Ok(bytes) = world.read_package(cell) else {
                continue;
            };
            read += started.elapsed().as_secs_f64();

            let started = Instant::now();
            let Ok(header) = FZenPackageHeader::deserialize(
                &mut Cursor::new(&bytes),
                None,
                blam_tags::iostore::world::CE_TOC_VERSION,
                blam_tags::iostore::world::CE_HEADER_VERSION,
                None,
            ) else {
                continue;
            };
            let Ok(payloads) = read_payloads(&header, &bytes) else {
                continue;
            };
            let names = header.name_map.copy_raw_names();
            parse += started.elapsed().as_secs_f64();

            let started = Instant::now();
            let resolved: Vec<Option<String>> = header
                .export_map
                .iter()
                .map(|entry| world.class_key(&header, entry.class_index))
                .collect();
            classes += started.elapsed().as_secs_f64();

            let started = Instant::now();
            let resolver = world.resolver(&header, &bytes, &names);
            let bulk: Vec<(i64, i64)> = header
                .bulk_data
                .iter()
                .map(|entry| (entry.serial_offset, entry.serial_size))
                .collect();
            let context = ExportContext {
                bulk_data: &bulk,
                resolver: Some(&resolver),
            };
            for ((entry, payload), class) in
                header.export_map.iter().zip(&payloads).zip(&resolved)
            {
                exports_seen += 1;
                if let Some(class) = class.as_deref() {
                    let _ = read_export_in(
                        payload,
                        &names,
                        world.usmap(),
                        class,
                        entry.object_flags,
                        &context,
                    );
                }
            }
            decode += started.elapsed().as_secs_f64();
            per_cell.push(cell_started.elapsed().as_secs_f64());
        }

        per_cell.sort_by(f64::total_cmp);
        let total: f64 = per_cell.iter().sum();
        let at = |q: f64| per_cell[((per_cell.len() as f64 - 1.0) * q) as usize];
        eprintln!(
            "{} cells, {exports_seen} exports, {total:.1}s\n\
             \x20 read bytes    {read:6.1}s\n\
             \x20 parse header  {parse:6.1}s\n\
             \x20 resolve class {classes:6.1}s\n\
             \x20 decode props  {decode:6.1}s\n\
             \x20 per cell: median {:.0}ms  p90 {:.0}ms  p99 {:.0}ms  max {:.0}ms",
            cells.len(),
            at(0.5) * 1000.0,
            at(0.9) * 1000.0,
            at(0.99) * 1000.0,
            per_cell.last().copied().unwrap_or(0.0) * 1000.0,
        );
    }

    /// Find out what a cell read actually costs, and whether threads help.
    ///
    /// Parallelising the walk gained 13% on sixteen threads, which is not what
    /// independent work does — so this splits the cost into decompressing the
    /// package and decoding its exports, and times the walk at several thread
    /// counts, rather than guessing which one is the wall.
    #[test]
    #[ignore]
    fn probe_cell_read_scaling() {
        let root = std::env::var("BABOON_PROBE_PAKS").expect("BABOON_PROBE_PAKS");
        let step: usize = std::env::var("BABOON_SCALE_STEP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8);
        let usmap = crate::app::chimp::load_chimp_usmap(None).expect("usmap");
        let world = World::open(std::path::Path::new(&root), usmap).expect("mount");
        let cells: Vec<String> = world
            .packages()
            .iter()
            .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
            .map(|record| record.name.clone())
            .step_by(step)
            .collect();
        eprintln!("{} cells", cells.len());

        let started = std::time::Instant::now();
        let mut bytes = 0usize;
        for cell in &cells {
            if let Ok(data) = world.read_package(cell) {
                bytes += data.len();
            }
        }
        let read = started.elapsed().as_secs_f64();
        eprintln!(
            "  read_package only:      {read:.1}s  ({:.1} MiB, {:.1} MiB/s)",
            bytes as f64 / (1024.0 * 1024.0),
            bytes as f64 / (1024.0 * 1024.0) / read
        );

        let started = std::time::Instant::now();
        for cell in &cells {
            let _ = crate::app::chimp::load_chimp_document(&world, cell);
        }
        eprintln!(
            "  load_chimp_document:    {:.1}s",
            started.elapsed().as_secs_f64()
        );

        for threads in [1usize, 2, 4, 8, 16] {
            let started = std::time::Instant::now();
            let scene = read_cells(&world, &cells, threads, &|_| {});
            eprintln!(
                "  read_cells x{threads:<2}          {:.1}s  ({} placements)",
                started.elapsed().as_secs_f64(),
                scene.placements.len()
            );
        }
    }
}

#[cfg(test)]
mod real_data_tests {
    use super::*;

    /// Read part of a real Campaign Evolved level and check what comes out is a
    /// coherent world rather than a plausible-looking pile of numbers.
    ///
    /// Skips unless `BABOON_PROBE_PAKS` points at an install, in the same way
    /// the editing-kit fixture tests skip: the layout this reads was measured
    /// from shipped data, so shipped data is the only thing that can tell us it
    /// still holds.
    #[test]
    fn a_real_level_reads_as_a_coherent_world() {
        let Ok(root) = std::env::var("BABOON_PROBE_PAKS") else {
            eprintln!("skipping: set BABOON_PROBE_PAKS to a Campaign Evolved Paks folder");
            return;
        };
        let usmap = crate::app::chimp::load_chimp_usmap(None).expect("bundled usmap");
        let world = World::open(std::path::Path::new(&root), usmap).expect("mount the install");
        let cells: Vec<String> = world
            .packages()
            .iter()
            .filter(|record| record.name.to_lowercase().contains("/c10/_generated_/"))
            .map(|record| record.name.clone())
            .collect();
        if cells.is_empty() {
            eprintln!("skipping: this install has no C10 cells");
            return;
        }

        let mut scene = LevelScene::default();
        for cell in cells.iter().step_by(11) {
            if let Ok(document) = crate::app::chimp::load_chimp_package(&world, cell) {
                read_cell_into(&document, &mut scene);
            }
        }

        assert!(scene.cells > 0, "no cell could be read");
        assert!(!scene.placements.is_empty(), "the level placed nothing");
        assert!(
            scene.meshes.len() < scene.placements.len(),
            "a level reuses its meshes; {} meshes for {} placements means instancing was lost",
            scene.meshes.len(),
            scene.placements.len()
        );
        // Skipping some is expected - Blueprint-inherited meshes are not in the
        // cell - but losing a large share of the level would mean the reading is
        // wrong rather than the data being awkward.
        assert!(
            scene.skipped.total() * 20 < scene.placements.len(),
            "skipped {} of {} placements",
            scene.skipped.total(),
            scene.placements.len()
        );
        for placement in &scene.placements {
            assert!(
                placement.world.iter().all(|value| value.is_finite()),
                "a placement is not a finite transform"
            );
            assert!(
                (placement.world[15] - 1.0).abs() < 1e-6,
                "a placement is not affine"
            );
        }
        let extent = scene.placements.iter().fold(
            ([f64::MAX; 3], [f64::MIN; 3]),
            |(mut min, mut max), placement| {
                for axis in 0..3 {
                    min[axis] = min[axis].min(placement.world[12 + axis]);
                    max[axis] = max[axis].max(placement.world[12 + axis]);
                }
                (min, max)
            },
        );
        // Centimetres: a Halo level is hundreds of metres across, not microns
        // and not light years.
        for axis in 0..3 {
            let span = extent.1[axis] - extent.0[axis];
            assert!(
                (100.0..5_000_000.0).contains(&span),
                "axis {axis} spans {span} cm, which is not a level"
            );
        }
        eprintln!(
            "read {} cells: {} meshes, {} placements, {} skipped",
            scene.cells,
            scene.meshes.len(),
            scene.placements.len(),
            scene.skipped.total()
        );
    }
}
