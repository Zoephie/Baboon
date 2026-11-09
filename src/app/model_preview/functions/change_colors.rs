//! Change colours: an object's four channels of permutations, chosen by
//! variant.

use super::super::common::fields::{path_elements, rgb};
use blam_tags::TagFile;

/// One initial permutation of a change color channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChangePermutation {
    pub(crate) weight: f32,
    pub(crate) lower: [f32; 3],
    pub(crate) upper: [f32; 3],
    pub(crate) variant: String,
}

/// An object's four change color channels.
pub(crate) type ChangeColors = [Vec<ChangePermutation>; 4];

/// The object's change colors. Halo 2 nests each group's parent as a struct,
/// so `object` sits at the end of the chain of leading structs.
pub(in crate::app) fn read_change_colors(object: &TagFile) -> ChangeColors {
    let mut holder = object.root();
    let block = loop {
        if holder.field("change colors").is_some() {
            break Some(holder);
        }
        let Some(parent) = holder.fields_all().find_map(|field| field.as_struct()) else {
            break None;
        };
        holder = parent;
    };
    let mut colors = ChangeColors::default();
    let Some(block) = block else {
        return colors;
    };
    for (channel, entry) in path_elements(block, "change colors").take(4).enumerate() {
        // Halo CE's block has another name, and no variants.
        let permutations = ["initial permutations", "permutations"]
            .into_iter()
            .find(|name| entry.field(name).is_some())
            .unwrap_or("initial permutations");
        colors[channel] = path_elements(entry, permutations)
            .map(|permutation| ChangePermutation {
                weight: permutation.read_real("weight").unwrap_or(0.0),
                lower: rgb(permutation, "color lower bound"),
                upper: rgb(permutation, "color upper bound"),
                variant: permutation
                    .read_string_id("variant name")
                    .unwrap_or_default(),
            })
            .collect();
    }
    colors
}

/// One channel's color for a model variant. The game picks by a per-object
/// position; a preview has none, so this takes the permutation at the middle
/// of the weights and the middle of its range. White with nothing allowed, as
/// the game does.
pub(super) fn change_color_for(permutations: &[ChangePermutation], variant: &str) -> [f32; 3] {
    // `object_postprocess` shares out a block with no weights evenly, so
    // there a zero is as likely as any other entry and not ruled out.
    let unweighted = permutations.iter().all(|entry| entry.weight <= 0.0);
    let weight = |entry: &ChangePermutation| if unweighted { 1.0 } else { entry.weight };
    let allowed: Vec<_> = permutations
        .iter()
        .filter(|entry| weight(entry) > 0.0)
        .filter(|entry| entry.variant.is_empty() || entry.variant == variant)
        .collect();
    let total: f32 = allowed.iter().map(|entry| weight(entry)).sum();
    let mut remaining = total / 2.0;
    for entry in allowed {
        remaining -= weight(entry);
        if remaining <= 0.0 {
            return std::array::from_fn(|axis| (entry.lower[axis] + entry.upper[axis]) / 2.0);
        }
    }
    [1.0; 3]
}

pub(crate) fn change_colors_for(colors: &ChangeColors, variant: &str) -> [[f32; 3]; 4] {
    std::array::from_fn(|channel| change_color_for(&colors[channel], variant))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_colors_filter_by_variant_and_fall_back_to_white() {
        let entry = |weight, value: f32, variant: &str| ChangePermutation {
            weight,
            lower: [value; 3],
            upper: [value + 0.25; 3],
            variant: variant.to_owned(),
        };
        let list = [
            entry(1.0, 0.25, "minor"),
            entry(1.0, 0.5, "major"),
            entry(0.0, 0.75, ""),
        ];
        assert_eq!(change_color_for(&list, "minor"), [0.375; 3]);
        assert_eq!(change_color_for(&list, "major"), [0.625; 3]);
        assert_eq!(
            change_color_for(&list, "ultra"),
            [1.0; 3],
            "nothing allowed"
        );
        assert_eq!(change_color_for(&[], "minor"), [1.0; 3]);
        let shared = [
            entry(1.0, 0.0, ""),
            entry(2.0, 0.5, ""),
            entry(1.0, 0.75, ""),
        ];
        assert_eq!(
            change_color_for(&shared, "any"),
            [0.625; 3],
            "the middle of the weights"
        );
    }

    #[test]
    fn a_block_with_no_weights_shares_them_evenly() {
        let entry = |value: f32, variant: &str| ChangePermutation {
            weight: 0.0,
            lower: [value; 3],
            upper: [value; 3],
            variant: variant.to_owned(),
        };
        let list = [
            entry(0.25, "minor"),
            entry(0.5, "major"),
            entry(0.75, "major"),
        ];
        assert_eq!(change_color_for(&list, "minor"), [0.25; 3]);
        assert_eq!(change_color_for(&list, "major"), [0.5; 3]);
        assert_eq!(
            change_color_for(&list, "ultra"),
            [1.0; 3],
            "no entry for this variant"
        );
    }
}
