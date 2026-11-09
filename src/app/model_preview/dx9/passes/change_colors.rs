//! Change colours: Halo 2's permutations and tints, and Halo CE's colour interpolation.

use super::*;

/// One initial permutation of a change color channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChangePermutation {
    pub(crate) weight: f32,
    pub(crate) lower: [f32; 3],
    pub(crate) upper: [f32; 3],
    pub(crate) variant: String,
}

/// Animations of a tint, with the components they write.
pub(super) type H2AnimatedTint = ([bool; 4], Vec<H2Animation>);

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
    for (channel, entry) in elements(block, "change colors").take(4).enumerate() {
        // Halo CE's block has another name, and no variants.
        let permutations = ["initial permutations", "permutations"]
            .into_iter()
            .find(|name| entry.field(name).is_some())
            .unwrap_or("initial permutations");
        colors[channel] = elements(entry, permutations)
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

pub(super) fn rgb_to_hsv([red, green, blue]: [f32; 3]) -> [f32; 3] {
    let high = red.max(green).max(blue);
    let span = high - red.min(green).min(blue);
    let hue = if span == 0.0 {
        0.0
    } else if high == red {
        ((green - blue) / span).rem_euclid(6.0)
    } else if high == green {
        (blue - red) / span + 2.0
    } else {
        (red - green) / span + 4.0
    };
    [hue / 6.0, if high == 0.0 { 0.0 } else { span / high }, high]
}

pub(super) fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
    let sector = hue.rem_euclid(1.0) * 6.0;
    let whole = sector.floor();
    let part = sector - whole;
    let (low, falling, rising) = (
        value * (1.0 - saturation),
        value * (1.0 - saturation * part),
        value * (1.0 - saturation * (1.0 - part)),
    );
    match whole as u8 {
        0 => [value, rising, low],
        1 => [falling, value, low],
        2 => [low, value, rising],
        3 => [low, falling, value],
        4 => [rising, low, value],
        _ => [value, low, falling],
    }
}

/// `rgb_colors_interpolate` with its HSV flag: the short way round the hue,
/// or the long way with its `...more colors` flag.
pub(super) fn hsv_interpolate(
    lower: [f32; 3],
    upper: [f32; 3],
    u: f32,
    long_way: bool,
) -> [f32; 3] {
    let (mut from, mut to) = (rgb_to_hsv(lower), rgb_to_hsv(upper));
    if ((from[0] - to[0]).abs() > 0.5) != long_way {
        if from[0] >= to[0] {
            to[0] += 1.0;
        } else {
            from[0] += 1.0;
        }
    }
    hsv_to_rgb(std::array::from_fn(|channel| {
        from[channel] * (1.0 - u) + to[channel] * u
    }))
}

/// `object_choose_random_change_colors` for an object at the world's origin,
/// where its hash of the position leaves only the channel. White with no
/// permutation, as an unplaced object has.
pub(super) fn ce_change_color(permutations: &[ChangePermutation], channel: usize) -> [f32; 3] {
    let pick = (channel as f32 * 431.12894).rem_euclid(1.0);
    let u = (channel as f32 * 0.71211).rem_euclid(1.0);
    // Tool bakes the weights to a running share of the whole, shared out
    // evenly where a block has none.
    let unweighted = permutations.iter().all(|entry| entry.weight <= 0.0);
    let weight = |entry: &ChangePermutation| if unweighted { 1.0 } else { entry.weight };
    let total: f32 = permutations.iter().map(weight).sum();
    let mut running = 0.0;
    for entry in permutations {
        running += weight(entry) / total;
        if pick <= running {
            return hsv_interpolate(entry.lower, entry.upper, u, false)
                .map(|value| value.clamp(0.0, 1.0));
        }
    }
    [1.0; 3]
}

/// `rgb_colors_interpolate`'s flags from a tag's two options: whether to
/// blend in HSV, and whether the long way round the hue wheel.
pub(crate) fn ce_color_interpolation(hsv: bool, long_way: bool) -> i128 {
    i128::from(hsv) | i128::from(long_way) << 1
}

/// `rgb_colors_interpolate`: bit 0 of `flags` blends in HSV, bit 1 the long way.
pub(crate) fn ce_color_interpolate(
    flags: i128,
    lower: [f32; 3],
    upper: [f32; 3],
    u: f32,
) -> [f32; 3] {
    if flags & 1 != 0 {
        hsv_interpolate(lower, upper, u, flags & 2 != 0)
    } else {
        std::array::from_fn(|channel| lower[channel] * (1.0 - u) + upper[channel] * u)
    }
}

pub(crate) fn ce_change_colors(colors: &ChangeColors) -> [[f32; 3]; 4] {
    std::array::from_fn(|channel| ce_change_color(&colors[channel], channel))
}
