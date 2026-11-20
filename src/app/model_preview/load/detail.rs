//! Halo CE and Halo 2 levels of detail: each permutation names its geometry
//! once per level, and the preview draws the level picked.

use blam_tags::TagFile;
use blam_tags::game::Game;
use blam_tags::render_model::RenderModel;

/// A model's levels of detail, lowest first, and which of them any
/// permutation has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailLevels {
    pub labels: &'static [&'static str],
    /// Per level, whether a permutation has geometry of its own there: not
    /// what it draws a level lower too, as a tag fills a level it lacks.
    pub available: Vec<bool>,
    /// The level drawn: the one picked, else the highest a permutation has.
    pub shown: usize,
}

/// Halo CE's levels, lowest first, as the gbxmodel's fields name them.
const CE_FIELDS: [&str; 5] = ["super low", "low", "medium", "high", "super high"];
const CE_LABELS: [&str; 5] = ["Super Low", "Low", "Medium", "High", "Super High"];

/// Halo 2's levels, lowest first: L1 is seen farthest away, L6 up close.
const H2_FIELDS: [&str; 6] = [
    "L1 section index",
    "L2 section index",
    "L3 section index",
    "L4 section index",
    "L5 section index",
    "L6 section index",
];
const H2_LABELS: [&str; 6] = [
    "L1 Super Low",
    "L2 Low",
    "L3 Medium",
    "L4 High",
    "L5 Super High",
    "L6 Hollywood",
];

/// Point each permutation of `model` at its geometry for the level `picked`
/// (by index, lowest first; `None` for the highest), and say which levels
/// the model has. A permutation without the level picked takes the nearest
/// higher one it has, else the nearest lower, so nothing vanishes. `None`
/// for a game without levels (Halo 3 and later).
pub(super) fn apply_detail_level(
    tag: &TagFile,
    model: &mut RenderModel,
    mesh_count: usize,
    picked: Option<usize>,
) -> Option<DetailLevels> {
    let (fields, labels): (&[&str], &'static [&'static str]) = match Game::of(tag) {
        Game::Halo1 => (&CE_FIELDS, &CE_LABELS),
        Game::Halo2 => (&H2_FIELDS, &H2_LABELS),
        _ => return None,
    };
    let regions = tag.root().field("regions")?.as_block()?;
    // Every permutation's geometry per level, in the tag's order, which the
    // model's regions keep.
    let mut permutations: Vec<Vec<Vec<Option<i16>>>> = Vec::new();
    for region in 0..regions.len() {
        let region = regions.element(region)?;
        let mut levels = Vec::new();
        if let Some(block) = region.field("permutations").and_then(|field| field.as_block()) {
            for permutation in 0..block.len() {
                let Some(permutation) = block.element(permutation) else {
                    continue;
                };
                levels.push(
                    fields
                        .iter()
                        .map(|field| {
                            permutation
                                .read_int_any(field)
                                .filter(|&index| index >= 0 && (index as usize) < mesh_count)
                                .map(|index| index as i16)
                        })
                        .collect(),
                );
            }
        }
        permutations.push(levels);
    }
    let available = own_levels(permutations.iter().flatten(), fields.len());
    let highest = available.iter().rposition(|&has| has)?;
    let shown = picked.filter(|&level| level < fields.len()).unwrap_or(highest);
    for (region, levels) in model.regions.iter_mut().zip(&permutations) {
        for (permutation, levels) in region.permutations.iter_mut().zip(levels) {
            if let Some(index) = level_or_nearest(levels, shown) {
                permutation.mesh_index = index;
                permutation.mesh_count = 1;
            }
        }
    }
    Some(DetailLevels {
        labels,
        available,
        shown,
    })
}

/// Per level of `count`, whether any of `permutations` (each its geometry
/// per level, lowest first) has geometry there other than the nearest
/// higher level it has. A level a tag lacks is filled from above, as
/// [`level_or_nearest`] fills one: the top of a run of one geometry is the
/// level it belongs to.
fn own_levels<'a>(
    permutations: impl Iterator<Item = &'a Vec<Option<i16>>> + Clone,
    count: usize,
) -> Vec<bool> {
    (0..count)
        .map(|level| {
            permutations.clone().any(|levels| {
                let Some(index) = levels.get(level).copied().flatten() else {
                    return false;
                };
                let above = levels[level + 1..].iter().find_map(|index| *index);
                above != Some(index)
            })
        })
        .collect()
}

/// `levels[level]`, else the nearest higher level's, else the nearest lower.
fn level_or_nearest(levels: &[Option<i16>], level: usize) -> Option<i16> {
    let level = level.min(levels.len().saturating_sub(1));
    levels[level..]
        .iter()
        .chain(levels[..level].iter().rev())
        .find_map(|index| *index)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level only repeating the one above it, as a tag fills a level it
    /// lacks, is not one of its own.
    #[test]
    fn a_level_repeating_the_one_above_is_not_its_own() {
        let permutations = [
            vec![Some(0), Some(0), Some(1), Some(1), Some(2)],
            vec![None, Some(3), Some(3), Some(3), Some(3)],
        ];
        assert_eq!(
            own_levels(permutations.iter(), 5),
            [false, true, false, true, true],
        );
        // One permutation's own geometry is enough, whatever the others do.
        let one = [vec![Some(0), Some(0), Some(0)], vec![Some(1), Some(2), Some(2)]];
        assert_eq!(own_levels(one.iter(), 3), [true, false, true]);
    }

    #[test]
    fn a_missing_level_takes_the_nearest_higher_then_lower() {
        let levels = [Some(0), None, Some(2), None, None];
        assert_eq!(level_or_nearest(&levels, 0), Some(0));
        assert_eq!(level_or_nearest(&levels, 1), Some(2));
        assert_eq!(level_or_nearest(&levels, 3), Some(2));
        assert_eq!(level_or_nearest(&[None, None], 0), None);
    }

    /// A shipped Halo CE gbxmodel and Halo 2 render model list their levels,
    /// draw the highest by default, and draw less at the lowest.
    #[test]
    fn shipped_models_draw_the_level_picked() {
        use crate::app::model_preview::harness::kit::{kit_preview_with, kit_root};
        use crate::app::model_preview::load::PreviewLoadSettings;
        use crate::core::game::GameId;
        let cases = [
            (GameId::HaloCe, r"characters\cyborg\cyborg", "gbxmodel", b"mod2", 5),
            (GameId::Halo2, r"objects\characters\brute\brute", "render_model", b"mode", 6),
        ];
        for (game, path, extension, group, count) in cases {
            let Some(root) = kit_root(game) else {
                continue;
            };
            let at = |detail_level| {
                let settings = PreviewLoadSettings {
                    detail_level,
                    ..Default::default()
                };
                kit_preview_with(&root, game, path, extension, group, &settings).preview
            };
            let highest = at(None);
            let levels = highest.detail_levels.clone().expect("levels of detail");
            assert_eq!(levels.labels.len(), count, "{path}");
            let top = levels.available.iter().rposition(|&has| has).unwrap();
            assert_eq!(levels.shown, top, "{path}: the highest by default");
            let lowest = levels.available.iter().position(|&has| has).unwrap();
            assert!(lowest < top, "{path}: one level only: {levels:?}");
            let low = at(Some(lowest));
            assert_eq!(low.detail_levels.as_ref().unwrap().shown, lowest);
            assert!(
                low.indices.len() < highest.indices.len(),
                "{path}: {} indices at {}, {} at {}",
                low.indices.len(),
                levels.labels[lowest],
                highest.indices.len(),
                levels.labels[top],
            );
        }
    }
}
