//! Where the preview stands a collection's detail objects: on Sapien's
//! lattice, as its brush (`sub_426830`) paints them.
//!
//! Sapien snaps the brush to a lattice of 255 steps an 8-unit cell
//! (`sub_424710`), keeps a point by the brush's density, and gives each kept
//! one the type chosen in its dialog and a frame of that type's sequence at
//! random (`HIWORD(lcg) % sprites`). The instance keeps four bits of the
//! frame, and the game draws `frame % count` of the sequence.

use super::super::super::particles::Random;
use super::super::Layout;
use super::HaloCeCollection;
use super::shading::detail_object_color;

/// The lattice's step: an 8-unit cell in 255 steps.
pub(crate) const LATTICE: f32 = 8.0 / 255.0;

/// One detail object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Instance {
    pub type_index: usize,
    /// Of its type's sequence.
    pub frame: usize,
    pub position: [f32; 3],
    /// As Sapien bakes it (`detail_object_color`).
    pub color: [f32; 3],
}

/// The instances `layout` asks for: each on a lattice point of the patch no
/// other has taken, its type at random (Sapien paints one type at a time;
/// the preview mixes them), its frame at random.
pub(in crate::app::model_preview) fn scatter_halo_ce(
    collection: &HaloCeCollection,
    layout: Layout,
) -> Vec<Instance> {
    let mut random = Random(layout.seed);
    let steps = ((layout.size / LATTICE) as i64).max(1);
    let half = steps / 2;
    let points = (steps * steps) as usize;
    let mut taken = std::collections::HashSet::new();
    let mut instances = Vec::with_capacity(layout.count.min(points));
    while instances.len() < layout.count.min(points) {
        let point = [
            (random.next() * steps as f32) as i64 - half,
            (random.next() * steps as f32) as i64 - half,
        ];
        if !taken.insert(point) {
            continue;
        }
        let type_index = ((random.next() * collection.types.len() as f32) as usize)
            .min(collection.types.len() - 1);
        let sprites = collection
            .plate
            .sequences
            .get(collection.types[type_index].sequence)
            .map_or(0, Vec::len);
        let frame = if sprites == 0 {
            0
        } else {
            // What the instance keeps of Sapien's pick, then the game's own.
            ((random.bits() as usize % sprites) & 0xF) % sprites
        };
        // Sapien's one draw of its colour's place between least and most.
        let color = detail_object_color(&collection.types[type_index], random.next());
        instances.push(Instance {
            type_index,
            frame,
            color,
            position: [
                point[0] as f32 * LATTICE,
                point[1] as f32 * LATTICE,
                collection.z_offset,
            ],
        });
    }
    instances
}
