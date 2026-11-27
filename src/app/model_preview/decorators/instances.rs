//! The placements packed as Halo 3's and ODST's decorator shader reads them
//! on PC (`decorators.hlsl`, `DX_VERSION == 11`), 16 bytes each:
//!
//! - `TEXCOORD1`, four `uint16`: the position within the block's bounds
//!   (`instance_compression_scale` and `_offset`), and in `w` the type index
//!   times 256 plus the motion byte, which the shader splits back apart;
//! - `TEXCOORD2`, four `UNORM8` read as `* 2 - 1`: the rotation, `{x, y, z,
//!   w}`, scaled by the square root of half the placement's scale. The
//!   shader turns a point by `q p q*`, which scales it by `|q|²` too, and the
//!   vertex compression's constant scale of 2 doubles it back;
//! - `TEXCOORD3`, four `UNORM8`: the baked light, `rgb · 2^(a · 63.75 −
//!   31.75)`.

use super::Placement;

/// One packed instance's size.
pub(crate) const INSTANCE_BYTES: usize = 16;

/// The largest scale a quaternion of `UNORM8`s carries: `|q|² = scale / 2`
/// must stay within 1. Reach's `compress_placement` clamps to it too.
const LARGEST_SCALE: f32 = 2.0;

/// The placements packed, grouped by type.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PackedInstances {
    pub bytes: Vec<u8>,
    /// `instance_compression_offset` and `_scale`.
    pub offset: [f32; 3],
    pub scale: [f32; 3],
    /// Each type's first instance and count.
    pub types: Vec<(usize, usize)>,
}

/// `placements`, each lit by `light` (its baked colour), packed in order of
/// type, for `type_count` types.
pub(crate) fn pack(
    placements: &[Placement],
    light: impl Fn(&Placement) -> [f32; 3],
    type_count: usize,
) -> PackedInstances {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for placement in placements {
        for axis in 0..3 {
            low[axis] = low[axis].min(placement.position[axis]);
            high[axis] = high[axis].max(placement.position[axis]);
        }
    }
    if placements.is_empty() {
        (low, high) = ([0.0; 3], [0.0; 3]);
    }
    let scale: [f32; 3] =
        std::array::from_fn(|axis| ((high[axis] - low[axis]) / 65_535.0).max(f32::MIN_POSITIVE));
    let mut packed = PackedInstances {
        bytes: Vec::with_capacity(placements.len() * INSTANCE_BYTES),
        offset: low,
        scale,
        types: Vec::with_capacity(type_count),
    };
    for type_index in 0..type_count {
        let first = packed.bytes.len() / INSTANCE_BYTES;
        for placement in placements
            .iter()
            .filter(|placement| placement.type_index == type_index)
        {
            for axis in 0..3 {
                let steps = ((placement.position[axis] - low[axis]) / scale[axis])
                    .round()
                    .clamp(0.0, 65_535.0) as u16;
                packed.bytes.extend_from_slice(&steps.to_le_bytes());
            }
            let motion = (placement.motion.clamp(0.0, 1.0) * 255.0).round() as u16;
            let shifted = (type_index.min(255) as u16) << 8 | motion;
            packed.bytes.extend_from_slice(&shifted.to_le_bytes());
            let length = (placement.scale.clamp(0.0, LARGEST_SCALE) * 0.5).sqrt();
            for component in placement.rotation {
                packed.bytes.push(unorm8((component * length + 1.0) * 0.5));
            }
            packed.bytes.extend_from_slice(&rgbe(light(placement)));
        }
        packed
            .types
            .push((first, packed.bytes.len() / INSTANCE_BYTES - first));
    }
    packed
}

fn unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `color` as the shader's `rgb · 2^(a · 63.75 − 31.75)`, `a` the fourth
/// byte over 255: the least exponent that holds the brightest channel.
fn rgbe(color: [f32; 3]) -> [u8; 4] {
    let brightest = color.iter().copied().fold(0.0_f32, f32::max);
    if brightest <= 0.0 {
        return [0; 4];
    }
    let exponent = ((brightest.log2() + 31.75) * 4.0).ceil().clamp(0.0, 255.0);
    let unit = (exponent / 4.0 - 31.75).exp2();
    let [r, g, b] = color.map(|channel| unorm8(channel / unit));
    [r, g, b, exponent as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's reads of one packed instance: its position, type, motion,
    /// rotation (scaled) and light.
    fn unpack(
        packed: &PackedInstances,
        index: usize,
    ) -> ([f32; 3], usize, f32, [f32; 4], [f32; 3]) {
        let bytes = &packed.bytes[index * INSTANCE_BYTES..][..INSTANCE_BYTES];
        let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as f32;
        let position =
            std::array::from_fn(|axis| word(axis * 2) * packed.scale[axis] + packed.offset[axis]);
        let shifted = word(6) / 256.0;
        let type_index = shifted.floor();
        let quaternion = std::array::from_fn(|at| bytes[8 + at] as f32 / 255.0 * 2.0 - 1.0);
        let unit = (bytes[15] as f32 / 255.0 * 63.75 - 31.75).exp2();
        let light = std::array::from_fn(|at| bytes[12 + at] as f32 / 255.0 * unit);
        (
            position,
            type_index as usize,
            shifted - type_index,
            quaternion,
            light,
        )
    }

    #[test]
    fn a_placement_reads_back_as_the_shader_unpacks_it() {
        let (sin, cos) = (0.3_f32).sin_cos();
        let placements = [
            Placement {
                type_index: 1,
                position: [-2.0, 1.5, 0.25],
                rotation: [0.0, 0.0, sin, cos],
                scale: 0.8,
                motion: 0.6,
                tint: [1.0; 3],
                ground_tint: 0.0,
            },
            Placement {
                type_index: 0,
                position: [3.0, -1.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: 1.5,
                motion: 0.2,
                tint: [1.0; 3],
                ground_tint: 0.0,
            },
        ];
        let packed = pack(
            &placements,
            |placement| [0.2, 0.5, 3.0].map(|channel| channel * placement.scale),
            2,
        );
        assert_eq!(packed.types, vec![(0, 1), (1, 1)], "grouped by type");
        let (position, type_index, motion, quaternion, light) = unpack(&packed, 1);
        assert_eq!(type_index, 1);
        assert!(
            position
                .iter()
                .zip(placements[0].position)
                .all(|(a, b)| (a - b).abs() < 1e-3),
            "{position:?}"
        );
        assert!(
            (motion - 153.0 / 256.0).abs() < 1e-6,
            "the shader reads the motion byte over 256"
        );
        let length_squared: f32 = quaternion.iter().map(|q| q * q).sum();
        assert!(
            (length_squared - 0.4).abs() < 0.02,
            "|q|² is half the scale: {length_squared}"
        );
        // One exponent for all three: each channel is exact to a step of the
        // brightest's, which the next quarter power of two above it sets.
        let expected = [0.16, 0.4, 2.4];
        let step = ((2.4_f32.log2() * 4.0).ceil() / 4.0).exp2() / 255.0;
        assert!(
            light
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() <= step),
            "{light:?}"
        );
    }

    #[test]
    fn scale_is_clamped_to_what_a_byte_quaternion_carries() {
        let placement = Placement {
            type_index: 0,
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 5.0,
            motion: 0.0,
            tint: [1.0; 3],
            ground_tint: 0.0,
        };
        let packed = pack(&[placement], |_| [1.0; 3], 1);
        let (_, _, _, quaternion, _) = unpack(&packed, 0);
        assert!((quaternion[3] - 1.0).abs() < 0.01, "{quaternion:?}");
    }
}
