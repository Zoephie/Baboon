//! Colour maths the engines share: `rgb_colors_interpolate` and its HSV
//! conversions.

pub(in crate::app::model_preview) fn rgb_to_hsv([red, green, blue]: [f32; 3]) -> [f32; 3] {
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

pub(in crate::app::model_preview) fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> [f32; 3] {
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
pub(in crate::app::model_preview) fn hsv_interpolate(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ce_colors_blend_the_short_way_round_the_hue() {
        // Red and a red just shy of the wrap meet near red, not across cyan.
        let mixed = hsv_interpolate([1.0, 0.0, 0.0], [1.0, 0.0, 0.2], 0.5, false);
        assert!(mixed[0] > 0.99 && mixed[1] < 0.01, "{mixed:?}");
        // Unlike an RGB blend, yellow keeps its brightness between red and green.
        let middle = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, false);
        assert!(
            (middle[0] - 1.0).abs() < 1e-5 && (middle[1] - 1.0).abs() < 1e-5,
            "{middle:?}"
        );
        // The long way from red to green goes through blue.
        let long = hsv_interpolate([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, true);
        assert!(long[2] > 0.99 && long[1] < 0.01, "{long:?}");
        // A color survives the trip.
        let color = [0.34, 0.4, 0.15];
        let back = hsv_to_rgb(rgb_to_hsv(color));
        assert!(
            color.iter().zip(back).all(|(a, b)| (a - b).abs() < 1e-5),
            "{back:?}"
        );
    }
}
