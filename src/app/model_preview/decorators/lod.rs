//! Reach's and Halo 4's detail levels: up to four render models a set swaps
//! between with distance, cross-faded through a screen-door stipple in the
//! stencil, the nearest thinned by decimation.
//!
//! The game picks a level per cull block, a patch of the set's decorators
//! (`render_decorators`), and draws it through `c_decorator_lod_renderer`.
//! Reach and Halo 4 measure the distance and the cross-fade differently; both
//! come from their 360 tag builds, checked against MCC's `reach_tag_test` and
//! `halo4_tag_test`. See `docs/decorator-preview-plan.md`.

use super::Placement;
use crate::core::game::GameId;
use eframe::glow;

/// How a game picks and fades its levels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LodRule {
    #[default]
    Reach,
    Halo4,
}

impl LodRule {
    pub(crate) fn of(game: Option<GameId>) -> Option<Self> {
        match game {
            Some(GameId::HaloReach) => Some(Self::Reach),
            Some(GameId::Halo4) => Some(Self::Halo4),
            _ => None,
        }
    }
}

/// One of the four `Base` to `Lod4` slots, after the postprocess: the model it
/// draws (an index into the set's levels, the base 0) and its cross-fade to
/// the next.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Slot {
    pub model: Option<usize>,
    pub start: f32,
    pub end: f32,
    /// `scale0` to `scale3` and `offset0` to `offset3`: `distance · scale +
    /// offset` is 1 at `start` and 0 at `end`.
    pub scale: f32,
    pub offset: f32,
}

/// `decimation start` and `end`, and Halo 4's own hidden decimation fields.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Decimation {
    /// `Use decimation method for random structured decorators`.
    pub on: bool,
    /// From `decimation start` and `end`: 1 at the start, 0 at the end.
    pub scale: f32,
    pub offset: f32,
    /// Halo 4's `decimation speed`, `decimation start distance` and
    /// `decimate to`, which its postprocess leaves as the tag has them.
    pub speed: f32,
    pub start_distance: f32,
    pub decimate_to: f32,
}

/// A set's levels and how the game picks between them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DetailLevels {
    pub rule: LodRule,
    pub slots: [Slot; 4],
    /// `max valid lod`: the last slot the postprocess filled.
    pub last: usize,
    pub decimation: Decimation,
    /// `early cull`, which only Halo 4 reads.
    pub early_cull: f32,
    /// `cull block size`, clamped as the game's build clamps it.
    pub block_size: f32,
    /// The rasterizer globals' default performance throttles' first
    /// element's `decorator fade dist scale`: 0 draws no decorators at all.
    pub throttle: f32,
    /// Each level's render model, nearest first.
    pub models: Vec<String>,
}

/// `cull block size`'s range (`[0.5-100]world units`), which the game's
/// build clamps it to.
pub(crate) const BLOCK_SIZE: [f32; 2] = [0.5, 100.0];

/// `s_decorator_set::postprocess`'s levels: `models` says which of the four
/// slots name a render model that loads, `ranges` each slot's `start point`
/// and `end point`. The valid slots are moved down together, each taking its
/// range; then each end at or short of the one before becomes that end + 8,
/// each start short of the end before or past its own end becomes a quarter
/// of the way from the end before to its own, and each range becomes a scale
/// and offset. `None` where no slot is valid.
pub(crate) fn postprocess(models: [bool; 4], ranges: [[f32; 2]; 4]) -> Option<([Slot; 4], usize)> {
    // Which slot's model each slot now holds, by its own first index.
    let mut holds: [Option<usize>; 4] = std::array::from_fn(|slot| models[slot].then_some(slot));
    let mut slots = ranges.map(|[start, end]| Slot {
        start,
        end,
        ..Default::default()
    });
    let mut compacted = [None; 4];
    let mut last: Option<usize> = None;
    for slot in 0..4 {
        if !models[slot] {
            continue;
        }
        let at = last.map_or(0, |last| last + 1);
        last = Some(at);
        compacted[slot] = Some(at);
        if at != slot {
            holds[at] = holds[slot];
            slots[at] = slots[slot];
        }
    }
    let last = last?;
    for (slot, held) in slots.iter_mut().zip(holds) {
        slot.model = held.and_then(|first| compacted[first]);
    }
    let mut end_before = 0.0;
    for slot in &mut slots[..=last] {
        if slot.end <= end_before {
            slot.end = end_before + 8.0;
        }
        end_before = slot.end;
    }
    let mut end_before = 0.0;
    for slot in &mut slots[..=last] {
        if slot.start < end_before || slot.start > slot.end {
            slot.start = (end_before + 3.0 * slot.end) * 0.25;
        }
        end_before = slot.end;
    }
    for slot in &mut slots[..=last] {
        slot.scale = -1.0 / (slot.end - slot.start).max(0.0001);
        slot.offset = 1.0 - slot.start * slot.scale;
    }
    slots[last].end = slots[last].end.max(0.0);
    Some((slots, last))
}

/// `decimation start` and `end` as the postprocess leaves them: a scale and
/// offset that is 1 at the start and 0 at the end.
pub(crate) fn decimation_terms(start: f32, end: f32) -> (f32, f32) {
    let scale = -1.0 / (end - start).max(0.0001);
    (scale, -end * scale)
}

/// A cull block: the placements the game culls and picks a level for
/// together, and the sphere it measures them by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Block {
    pub center: [f32; 3],
    pub radius: f32,
}

/// Each placement's block, and the blocks: a grid of `size` on the ground,
/// each cell's sphere around its placements. The game's own blocks come
/// from a level's build; the preview grids its patch the same size.
pub(crate) fn blocks(placements: &[Placement], size: f32) -> (Vec<usize>, Vec<Block>) {
    let size = size.clamp(BLOCK_SIZE[0], BLOCK_SIZE[1]);
    let mut cells: std::collections::BTreeMap<(i64, i64), usize> = Default::default();
    let mut bounds: Vec<([f32; 3], [f32; 3])> = Vec::new();
    let of: Vec<usize> = placements
        .iter()
        .map(|placement| {
            let [x, y, _] = placement.position;
            let cell = ((x / size).floor() as i64, (y / size).floor() as i64);
            let index = *cells.entry(cell).or_insert_with(|| {
                bounds.push(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]));
                bounds.len() - 1
            });
            let (low, high) = &mut bounds[index];
            for axis in 0..3 {
                low[axis] = low[axis].min(placement.position[axis]);
                high[axis] = high[axis].max(placement.position[axis]);
            }
            index
        })
        .collect();
    let blocks = bounds
        .into_iter()
        .map(|(low, high)| {
            let center = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
            let radius = (0..3)
                .map(|axis| (high[axis] - low[axis]) * 0.5)
                .map(|half| half * half)
                .sum::<f32>()
                .sqrt();
            Block { center, radius }
        })
        .collect();
    (of, blocks)
}

/// One draw of a block's level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LevelDraw {
    pub slot: usize,
    /// How much of it the stipple lets through, 0 to 1.
    pub fade: f32,
    /// The stipple's inverse, which the level after a fading one draws with.
    pub inverse: bool,
    /// How much of each strip draws, and how much of it is whole: the rest
    /// fades out through the alpha test (`instance_data`). `[1, 1]` without
    /// decimation.
    pub keep: [f32; 2],
    /// Reach's plain draw, which skips a level the stipple would hide
    /// whole; its decimated draw and Halo 4's draw regardless.
    pub skippable: bool,
}

/// `rasterizer_camera_get_fov_pixel_linear_scale` normalised: the game's
/// distances are scaled by `1.7186729 · tan(fov / 2)`, 1 at a 60° view.
const FOV_NORMAL: f32 = 1.718_672_9;

/// The levels the game draws for `block` from `eye`, with `fov` the view's
/// `tan(fov / 2)`, nearest first.
pub(crate) fn choose(
    levels: &DetailLevels,
    block: &Block,
    eye: [f32; 3],
    fov: f32,
) -> Vec<LevelDraw> {
    let distance = (0..3)
        .map(|axis| eye[axis] - block.center[axis])
        .map(|delta| delta * delta)
        .sum::<f32>()
        .sqrt();
    match levels.rule {
        LodRule::Reach => choose_reach(levels, block, distance, fov),
        LodRule::Halo4 => choose_halo_4(levels, block, distance, fov),
    }
}

/// Reach: the distance past the block's sphere, scaled by the view and the
/// throttle; culled where the last level has faded; the first level not yet
/// faded, and the next under it while it fades.
fn choose_reach(levels: &DetailLevels, block: &Block, distance: f32, fov: f32) -> Vec<LevelDraw> {
    if levels.throttle == 0.0 {
        return Vec::new();
    }
    let distance = (distance - block.radius).max(0.0) * FOV_NORMAL * fov / levels.throttle;
    let along = |slot: &Slot| distance * slot.scale + slot.offset;
    if along(&levels.slots[levels.last]) <= 0.0 {
        return Vec::new();
    }
    let Some(slot) = (0..=levels.last).find(|&slot| along(&levels.slots[slot]) > 0.0) else {
        return Vec::new();
    };
    let fade = along(&levels.slots[slot]).min(1.0);
    let decimation = levels.decimation;
    let mut draws = vec![if slot == 0 && decimation.on {
        let test = 1.0 / fov.sqrt();
        let kept = |distance: f32| {
            let x = (distance * decimation.scale + decimation.offset).max(0.0);
            test * x * x * x * x
        };
        let keep = kept(distance).min(1.0);
        LevelDraw {
            slot,
            fade,
            inverse: false,
            keep: [keep, kept(distance + 3.0).min(keep)],
            skippable: false,
        }
    } else {
        LevelDraw {
            slot,
            fade,
            inverse: false,
            keep: [1.0; 2],
            skippable: true,
        }
    }];
    if slot < levels.last && fade < 1.0 {
        draws.push(LevelDraw {
            slot: slot + 1,
            fade,
            inverse: true,
            keep: [1.0; 2],
            skippable: true,
        });
    }
    draws
}

/// Halo 4: the distance to the block's centre, scaled by the throttle over
/// the view; culled past the last slot's end; the first slot the eye is
/// short of the end of, and the next under it once the eye is past its
/// start. Slot 0 with no end draws whole.
fn choose_halo_4(levels: &DetailLevels, block: &Block, distance: f32, fov: f32) -> Vec<LevelDraw> {
    let scale = levels.throttle / (FOV_NORMAL * fov).max(0.785);
    let cull = match levels.slots[3].end * scale {
        0.0 => f32::MAX,
        cull => cull,
    };
    if distance > (block.radius + cull) * (1.0 - levels.early_cull) {
        return Vec::new();
    }
    let decimation = levels.decimation;
    let test = 1.0 / fov.sqrt();
    let kept = |distance: f32| {
        (distance.max(1e-10).powf(-0.667 * decimation.speed) * test).clamp(0.0, 1.0)
    };
    let past = distance - decimation.start_distance * scale;
    let keep = kept(past).max(decimation.decimate_to / 100.0).min(1.0);
    let keep = [keep, kept(past + 10.0).min(keep)];
    let whole = |slot: usize| LevelDraw {
        slot,
        fade: 1.0,
        inverse: false,
        keep: [1.0; 2],
        skippable: false,
    };
    for slot in 0..4 {
        let end = levels.slots[slot].end * scale;
        if end == 0.0 && slot == 0 {
            return vec![whole(0)];
        }
        if end <= distance {
            continue;
        }
        let start = levels.slots[slot].start * scale;
        let fade = 1.0 - ((distance - start) / (end - start)).clamp(0.0, 1.0);
        let mut draws = vec![LevelDraw {
            fade,
            keep: if slot == 0 && decimation.on {
                keep
            } else {
                [1.0; 2]
            },
            ..whole(slot)
        }];
        if distance > start && slot < 3 {
            draws.push(LevelDraw {
                fade,
                inverse: true,
                ..whole(slot + 1)
            });
        }
        return draws;
    }
    Vec::new()
}

/// A stencil test against the stipple pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StencilTest {
    /// A GL comparison: the reference against the pattern's value.
    pub function: u32,
    pub reference: i32,
    pub mask: u32,
}

/// `rasterizer_stipple_set_fade_byte` for `fade`: the test, and whether a
/// skippable draw draws at all (Reach skips a level the stipple hides
/// whole). Reach's pattern holds 0 to 3, tested `ref > value` against
/// `ref = (3b + 253) / 254` of the fade byte `b`; Halo 4's holds 0 to 15,
/// tested `ref ≥ value` against `b >> 4`. The inverse is `ref ≤ value`.
pub(crate) fn stipple(rule: LodRule, fade: f32, inverse: bool) -> (StencilTest, bool) {
    let byte = (fade * 255.0).min(255.0) as i32;
    let draws = if inverse { byte < 255 } else { byte > 0 };
    let test = match rule {
        LodRule::Reach => StencilTest {
            function: if inverse { glow::LEQUAL } else { glow::GREATER },
            reference: ((3 * byte + 253) / 254) & 7,
            mask: 7,
        },
        LodRule::Halo4 => StencilTest {
            function: if inverse { glow::LEQUAL } else { glow::GEQUAL },
            reference: (byte >> 4) & 15,
            mask: 15,
        },
    };
    (test, draws)
}

/// What the shader's `instance_data` holds for `keep` of a strip of `count`
/// vertices: the count, how many draw, how many of those are whole, and the
/// rate the rest fade out at. With none of it decimated, every vertex is
/// whole.
pub(crate) fn instance_data(count: usize, keep: [f32; 2]) -> (usize, [f32; 4]) {
    let count_f = count as f32;
    let kept = (keep[0] * count_f) as usize;
    let whole = keep[1] * count_f;
    let rate = 0.5 / (kept as f32 - whole).max(1.0);
    (kept, [count_f, kept as f32, whole, rate])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(rule: LodRule, ranges: [[f32; 2]; 4], models: [bool; 4]) -> DetailLevels {
        let (slots, last) = postprocess(models, ranges).unwrap();
        DetailLevels {
            rule,
            slots,
            last,
            throttle: 1.0,
            block_size: 10.0,
            ..Default::default()
        }
    }

    /// The postprocess's repairs: an end at or short of the last becomes it
    /// + 8, a start out of range a quarter of the way from the last end.
    #[test]
    fn the_postprocess_repairs_each_range() {
        let (slots, last) = postprocess(
            [true, true, true, false],
            [[3.0, 5.0], [0.0, 0.0], [50.0, 40.0], [0.0; 2]],
        )
        .unwrap();
        assert_eq!(last, 2);
        assert_eq!(
            (slots[1].start, slots[1].end),
            ((5.0 + 3.0 * 13.0) * 0.25, 13.0)
        );
        assert_eq!(
            (slots[2].start, slots[2].end),
            ((13.0 + 3.0 * 40.0) * 0.25, 40.0)
        );
        let at = |slot: &Slot, distance: f32| distance * slot.scale + slot.offset;
        assert!((at(&slots[0], 3.0) - 1.0).abs() < 1e-5 && at(&slots[0], 5.0).abs() < 1e-5);
        assert_eq!(slots[3].model, None);
    }

    /// A slot with no model is skipped: the next valid one moves down with
    /// its range, and a slot left past them keeps the model it named.
    #[test]
    fn the_postprocess_moves_valid_slots_down() {
        let (slots, last) = postprocess(
            [false, true, false, true],
            [[0.0; 2], [2.0, 4.0], [0.0; 2], [6.0, 9.0]],
        )
        .unwrap();
        assert_eq!(last, 1);
        assert_eq!(
            (slots[0].model, slots[0].start, slots[0].end),
            (Some(0), 2.0, 4.0)
        );
        assert_eq!(
            (slots[1].model, slots[1].start, slots[1].end),
            (Some(1), 6.0, 9.0)
        );
        assert_eq!(
            slots[3].model,
            Some(1),
            "Lod4's model is the second valid one"
        );
        assert!(postprocess([false; 4], [[0.0; 2]; 4]).is_none());
    }

    /// Reach: whole up close, cross-fading into the next level between its
    /// start and end, and gone past the last level's end.
    #[test]
    fn reach_cross_fades_between_levels() {
        let set = levels(
            LodRule::Reach,
            [[3.0, 5.0], [30.0, 40.0], [0.0; 2], [0.0; 2]],
            [true, true, false, false],
        );
        let block = Block {
            center: [0.0; 3],
            radius: 0.0,
        };
        let fov = 1.0 / FOV_NORMAL;
        let near = choose(&set, &block, [1.0, 0.0, 0.0], fov);
        assert_eq!(near.len(), 1);
        assert_eq!((near[0].slot, near[0].fade), (0, 1.0));
        let crossing = choose(&set, &block, [4.0, 0.0, 0.0], fov);
        assert_eq!(
            crossing
                .iter()
                .map(|draw| (draw.slot, draw.inverse))
                .collect::<Vec<_>>(),
            [(0, false), (1, true)]
        );
        assert!((crossing[0].fade - 0.5).abs() < 1e-4);
        assert_eq!(choose(&set, &block, [20.0, 0.0, 0.0], fov)[0].slot, 1);
        assert!(choose(&set, &block, [41.0, 0.0, 0.0], fov).is_empty());
        let off = DetailLevels {
            throttle: 0.0,
            ..set
        };
        assert!(
            choose(&off, &block, [1.0, 0.0, 0.0], fov).is_empty(),
            "a throttle of 0 draws none"
        );
    }

    /// Reach measures from the block's sphere, Halo 4 from its centre.
    #[test]
    fn reach_measures_past_the_sphere() {
        let reach = levels(
            LodRule::Reach,
            [[3.0, 5.0], [0.0; 2], [0.0; 2], [0.0; 2]],
            [true, false, false, false],
        );
        let block = Block {
            center: [0.0; 3],
            radius: 10.0,
        };
        let fov = 1.0 / FOV_NORMAL;
        assert_eq!(choose(&reach, &block, [12.0, 0.0, 0.0], fov)[0].fade, 1.0);
        let halo_4 = DetailLevels {
            rule: LodRule::Halo4,
            ..reach
        };
        assert!(
            choose(&halo_4, &block, [12.0, 0.0, 0.0], fov).is_empty(),
            "12 from the centre is past the base's end"
        );
    }

    /// Halo 4: slot 0 with no end draws whole at any distance.
    #[test]
    fn halo_4_draws_an_unranged_base_whole() {
        let set = DetailLevels {
            rule: LodRule::Halo4,
            throttle: 1.0,
            ..Default::default()
        };
        let block = Block {
            center: [0.0; 3],
            radius: 1.0,
        };
        let draws = choose(&set, &block, [500.0, 0.0, 0.0], 1.0 / FOV_NORMAL);
        assert_eq!(
            draws,
            [LevelDraw {
                slot: 0,
                fade: 1.0,
                inverse: false,
                keep: [1.0; 2],
                skippable: false
            }]
        );
    }

    /// Reach's decimation keeps the whole strip at its start, thins it to
    /// nothing by its end, and keeps less a few units further on.
    #[test]
    fn reach_decimation_thins_with_distance() {
        let mut set = levels(
            LodRule::Reach,
            [[30.0, 40.0], [0.0; 2], [0.0; 2], [0.0; 2]],
            [true, false, false, false],
        );
        let (scale, offset) = decimation_terms(2.0, 20.0);
        set.decimation = Decimation {
            on: true,
            scale,
            offset,
            ..Default::default()
        };
        let block = Block {
            center: [0.0; 3],
            radius: 0.0,
        };
        let fov = 1.0 / FOV_NORMAL;
        let keep = |distance: f32| choose(&set, &block, [distance, 0.0, 0.0], fov)[0].keep;
        assert_eq!(keep(1.0)[0], 1.0);
        let [kept, whole] = keep(10.0);
        assert!(kept > 0.0 && kept < 1.0 && whole < kept, "{kept} {whole}");
        assert_eq!(keep(25.0)[0], 0.0);
    }

    /// The stipple's reference steps with the fade, and the inverse covers
    /// what the plain test leaves.
    #[test]
    fn the_stipple_and_its_inverse_cover_the_pattern() {
        for (rule, values) in [(LodRule::Reach, 0..4), (LodRule::Halo4, 0..16)] {
            for fade in [0.1, 0.3, 0.5, 0.8] {
                let (plain, _) = stipple(rule, fade, false);
                let (inverse, _) = stipple(rule, fade, true);
                let passes = |test: StencilTest, value: i32| match test.function {
                    glow::GREATER => test.reference > value,
                    glow::GEQUAL => test.reference >= value,
                    _ => test.reference <= value,
                };
                let covered = values
                    .clone()
                    .filter(|&value| passes(plain, value) || passes(inverse, value))
                    .count();
                assert_eq!(covered, values.len(), "{rule:?} {fade}");
            }
        }
        assert!(!stipple(LodRule::Reach, 0.0, false).1 && !stipple(LodRule::Reach, 1.0, true).1);
        assert_eq!(stipple(LodRule::Reach, 1.0, false).0.reference, 4);
    }

    /// `instance_data` with nothing decimated leaves every vertex whole.
    #[test]
    fn instance_data_without_decimation_is_whole() {
        assert_eq!(
            instance_data(222, [1.0, 1.0]),
            (222, [222.0, 222.0, 222.0, 0.5])
        );
        let (kept, data) = instance_data(100, [0.5, 0.25]);
        assert_eq!((kept, data), (50, [100.0, 50.0, 25.0, 0.5 / 25.0]));
    }

    /// Blocks grid the ground by the block size, each with a sphere around
    /// its placements.
    #[test]
    fn blocks_grid_the_ground() {
        let at = |x: f32, y: f32| Placement {
            type_index: 0,
            position: [x, y, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            motion: 0.0,
            tint: [1.0; 3],
            ground_tint: 0.0,
        };
        let (of, blocks) = blocks(&[at(1.0, 1.0), at(3.0, 1.0), at(12.0, 1.0)], 10.0);
        assert_eq!(of, [0, 0, 1]);
        assert_eq!(
            blocks[0],
            Block {
                center: [2.0, 1.0, 0.0],
                radius: 1.0
            }
        );
        assert_eq!(blocks[1].radius, 0.0);
    }
}
