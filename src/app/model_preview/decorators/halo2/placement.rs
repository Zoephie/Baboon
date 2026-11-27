//! Halo 2's placements, as its 24-byte record keeps them (`sub_94AE50`,
//! `sub_6B20A0`): what the brush picked, cut to the record's bits.
//!
//! - `internal data 1`: class (3 bits), permutation (6), yaw (4, of a whole
//!   turn), tint (3, between `tint 1` and `tint 2`), scale (4, between the
//!   permutation's least and most), and the ground's normal (4 bits an
//!   axis, `n / 15 · 2 − 1`);
//! - `compressed position`: the place within its 8-unit cell, 10 bits across
//!   each way and 12 up.

use super::super::super::particles::Random;
use super::super::Layout;
use super::Halo2Set;

/// One placement, as its record's bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub class: usize,
    pub permutation: usize,
    /// Sixteenths of a turn.
    pub yaw: u8,
    /// Sevenths of the way from `tint 1` to `tint 2`.
    pub tint: u8,
    /// Fifteenths of the way from the least scale to the most.
    pub scale: u8,
    /// Each axis in fifteenths of the way from −1 to 1.
    pub normal: [u8; 3],
    /// Its 8-unit cell.
    pub cell: [i32; 3],
    /// Its place in the cell: 10, 10 and 12 bits.
    pub place: [u16; 3],
}

/// A cell's edge, in world units.
pub(crate) const CELL: f32 = 8.0;

/// The most each axis of a place counts to.
pub(crate) const PLACE_STEPS: [f32; 3] = [1023.0, 1023.0, 4095.0];

impl Record {
    /// Where the game draws it: its cell's corner and its place within.
    pub(crate) fn position(&self) -> [f32; 3] {
        std::array::from_fn(|axis| {
            self.cell[axis] as f32 * CELL + f32::from(self.place[axis]) * CELL / PLACE_STEPS[axis]
        })
    }

    /// The ground's normal, decoded (`sub_6AFC50`), unnormalized.
    pub(crate) fn normal(&self) -> [f32; 3] {
        self.normal.map(|bits| f32::from(bits) / 15.0 * 2.0 - 1.0)
    }

    pub(crate) fn yaw(&self) -> f32 {
        f32::from(self.yaw) / 15.0 * std::f32::consts::TAU
    }

    /// Its scale between the permutation's `scale` bounds.
    pub(crate) fn scale(&self, [least, most]: [f32; 2]) -> f32 {
        let along = f32::from(self.scale) / 15.0;
        (1.0 - along) * least + most * along
    }
}

/// Flat ground's normal, `(0, 0, 1)`, to the record's four bits an axis.
const FLAT: [u8; 3] = [8, 8, 15];

/// The records `layout` asks for: each permutation picked by its
/// `distribution weight` (evenly where none has one), its yaw, tint and
/// scale at random, stood on flat ground.
pub(in crate::app::model_preview) fn scatter_halo_2(set: &Halo2Set, layout: Layout) -> Vec<Record> {
    let choices: Vec<(usize, usize, f32)> = set
        .classes
        .iter()
        .enumerate()
        .flat_map(|(class, entry)| {
            entry
                .permutations
                .iter()
                .enumerate()
                .map(move |(permutation, entry)| (class, permutation, entry.distribution_weight))
        })
        .collect();
    if choices.is_empty() {
        return Vec::new();
    }
    let total: f32 = choices.iter().map(|(_, _, weight)| weight).sum();
    let mut random = Random(layout.seed);
    let half = layout.size * 0.5;
    (0..layout.count)
        .map(|_| {
            let (class, permutation) = pick(&choices, total, &mut random);
            let spot = [random.range(-half, half), random.range(-half, half), 0.0];
            let cell = spot.map(|value| (value / CELL).floor() as i32);
            let place = std::array::from_fn(|axis| {
                let within = spot[axis] - cell[axis] as f32 * CELL;
                (within / CELL * PLACE_STEPS[axis])
                    .round()
                    .clamp(0.0, PLACE_STEPS[axis]) as u16
            });
            Record {
                class,
                permutation,
                yaw: (random.next() * 16.0).min(15.0) as u8,
                tint: (random.next() * 8.0).min(7.0) as u8,
                scale: (random.next() * 16.0).min(15.0) as u8,
                normal: FLAT,
                cell,
                place,
            }
        })
        .collect()
}

fn pick(choices: &[(usize, usize, f32)], total: f32, random: &mut Random) -> (usize, usize) {
    if total <= 0.0 {
        let (class, permutation, _) =
            choices[(random.next() * choices.len() as f32) as usize % choices.len()];
        return (class, permutation);
    }
    let mut left = random.next() * total;
    for &(class, permutation, weight) in choices {
        if left < weight {
            return (class, permutation);
        }
        left -= weight;
    }
    let &(class, permutation, _) = choices
        .iter()
        .rev()
        .find(|(_, _, weight)| *weight > 0.0)
        .unwrap_or(&choices[0]);
    (class, permutation)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record decodes as the game's `sub_6AFBB0` and `sub_6AFC50` do:
    /// flat ground's normal to its nearest steps, a place back within a
    /// step of where it was put.
    #[test]
    fn a_record_decodes_as_the_game_reads_it() {
        let record = Record {
            class: 0,
            permutation: 0,
            yaw: 15,
            tint: 0,
            scale: 15,
            normal: FLAT,
            cell: [-1, 0, 0],
            place: [1023, 512, 0],
        };
        assert_eq!(record.position(), [0.0, 512.0 * 8.0 / 1023.0, 0.0]);
        let [x, y, z] = record.normal();
        assert!((x - 1.0 / 15.0).abs() < 1e-6 && (y - 1.0 / 15.0).abs() < 1e-6 && z == 1.0);
        assert!(
            (record.yaw() - std::f32::consts::TAU).abs() < 1e-5,
            "the last of sixteen steps is a whole turn"
        );
        assert_eq!(record.scale([0.5, 2.0]), 2.0);
    }
}
