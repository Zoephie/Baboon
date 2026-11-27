//! The placements packed as the decorator shader reads them on PC
//! (`decorators.hlsl`, `DX_VERSION == 11`), 16 bytes each.
//!
//! Halo 3's and ODST's ([`InstanceLayout::Halo3`]):
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
//!
//! Reach's ([`InstanceLayout::Reach`]), as its `compress_placement` (360
//! 0x82ff0868) and `light_placement` write them:
//!
//! - `POSITION1`, one `uint`: the position within the block's bounds, 11, 11
//!   and 10 bits (`UnpackUHEND3N`);
//! - `COLOR2`, four `UINT8` read `.wzyx`: the sun's visibility (255 is in
//!   full sun), a spare byte, the motion byte and the type index;
//! - `NORMAL1`, four `SNORM8` read `.wzyx`: the rotation, scaled as Halo 3's;
//! - `COLOR1`, four `UNORM8`: the baked light, as Halo 3's.
//!
//! Halo 4's ([`InstanceLayout::Halo4`]) are Reach's, but for the rotation's
//! scale and the light. Its `compress_placement` (360 0x82df2500) scales the
//! rotation by `√scale / √5`, clamping the scale to 5, so the vertex
//! compression's constant scale is 5 where Halo 3's and Reach's is 2; and its
//! light is `RGBk`, `rgb · k · 31.875` (`UnpackRGBk`).

use super::Placement;

/// One packed instance's size.
pub(crate) const INSTANCE_BYTES: usize = 16;

/// Which game's shader the instances are packed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstanceLayout {
    Halo3,
    Reach,
    Halo4,
}

impl InstanceLayout {
    /// The vertex compression's constant scale, which a rotation of `|q|² =
    /// scale / this` multiplies back: the largest scale a byte quaternion
    /// carries.
    pub(crate) fn vertex_scale(self) -> f32 {
        match self {
            Self::Halo3 | Self::Reach => 2.0,
            Self::Halo4 => 5.0,
        }
    }
}

/// A placement in full sun, as Reach's sun term reads it (`> 1` lights it).
const FULL_SUN: u8 = 255;

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
    layout: InstanceLayout,
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
    // Halo 3 scales whole steps of a `uint16`; Reach unpacks to 0..1 first.
    let steps = match layout {
        InstanceLayout::Halo3 => 65_535.0,
        InstanceLayout::Reach | InstanceLayout::Halo4 => 1.0,
    };
    let scale: [f32; 3] =
        std::array::from_fn(|axis| ((high[axis] - low[axis]) / steps).max(f32::MIN_POSITIVE));
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
            let motion = (placement.motion.clamp(0.0, 1.0) * 255.0).round() as u8;
            let type_byte = type_index.min(255) as u8;
            let most = layout.vertex_scale();
            let length = (placement.scale.clamp(0.0, most) / most).sqrt();
            let rotation = placement.rotation.map(|component| component * length);
            let along = |axis: usize| (placement.position[axis] - low[axis]) / scale[axis];
            match layout {
                InstanceLayout::Halo3 => {
                    for axis in 0..3 {
                        let steps = along(axis).round().clamp(0.0, 65_535.0) as u16;
                        packed.bytes.extend_from_slice(&steps.to_le_bytes());
                    }
                    let shifted = u16::from(type_byte) << 8 | u16::from(motion);
                    packed.bytes.extend_from_slice(&shifted.to_le_bytes());
                    for component in rotation {
                        packed.bytes.push(unorm8((component + 1.0) * 0.5));
                    }
                }
                InstanceLayout::Reach | InstanceLayout::Halo4 => {
                    let bits = |axis: usize, most: f32| {
                        (along(axis).clamp(0.0, 1.0) * most).round() as u32
                    };
                    let position = bits(0, 2047.0) | bits(1, 2047.0) << 11 | bits(2, 1023.0) << 22;
                    packed.bytes.extend_from_slice(&position.to_le_bytes());
                    packed
                        .bytes
                        .extend_from_slice(&[FULL_SUN, 0, motion, type_byte]);
                    let [x, y, z, w] = rotation;
                    packed.bytes.extend([w, z, y, x].map(snorm8));
                }
            }
            let light = light(placement);
            packed.bytes.extend_from_slice(&match layout {
                InstanceLayout::Halo3 | InstanceLayout::Reach => rgbe(light),
                InstanceLayout::Halo4 => rgbk(light),
            });
        }
        packed
            .types
            .push((first, packed.bytes.len() / INSTANCE_BYTES - first));
    }
    packed
}

/// `color` as `PackRGBk` writes it: `k`, the brightest channel rounded up
/// to an eighth, over 31.875, and the channels over `k`.
fn rgbk(color: [f32; 3]) -> [u8; 4] {
    let brightest = color.iter().copied().fold(0.0_f32, f32::max);
    let k = (brightest * 8.0 + 1.0).floor() / 8.0;
    let [r, g, b] = color.map(|channel| unorm8(channel / k));
    [r, g, b, unorm8(k / 31.875)]
}

fn snorm8(value: f32) -> u8 {
    (value.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8
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

    /// What the shader reads of one packed instance: its position, type,
    /// motion, rotation (scaled) and light, and Reach's sun byte.
    struct Read {
        position: [f32; 3],
        type_index: usize,
        motion: f32,
        quaternion: [f32; 4],
        light: [f32; 3],
        sun: f32,
    }

    fn unpack(layout: InstanceLayout, packed: &PackedInstances, index: usize) -> Read {
        let bytes = &packed.bytes[index * INSTANCE_BYTES..][..INSTANCE_BYTES];
        let unit = match layout {
            InstanceLayout::Halo3 | InstanceLayout::Reach => {
                (bytes[15] as f32 / 255.0 * 63.75 - 31.75).exp2()
            }
            // `UnpackRGBk`.
            InstanceLayout::Halo4 => bytes[15] as f32 / 255.0 * 31.875,
        };
        let light = std::array::from_fn(|at| bytes[12 + at] as f32 / 255.0 * unit);
        let place = |unpacked: [f32; 3]| {
            std::array::from_fn(|axis| unpacked[axis] * packed.scale[axis] + packed.offset[axis])
        };
        match layout {
            InstanceLayout::Halo3 => {
                let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as f32;
                let shifted = word(6) / 256.0;
                let type_index = shifted.floor();
                Read {
                    position: place([word(0), word(2), word(4)]),
                    type_index: type_index as usize,
                    motion: shifted - type_index,
                    quaternion: std::array::from_fn(|at| bytes[8 + at] as f32 / 255.0 * 2.0 - 1.0),
                    light,
                    sun: 0.0,
                }
            }
            InstanceLayout::Reach | InstanceLayout::Halo4 => {
                // `UnpackUHEND3N`, then `.wzyx` of the aux bytes and the rotation.
                let input = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let unpacked = [
                    (input & 0x7ff) as f32 / 2047.0,
                    ((input >> 11) & 0x7ff) as f32 / 2047.0,
                    ((input >> 22) & 0x3ff) as f32 / 1023.0,
                ];
                let aux = [bytes[7], bytes[6], bytes[5], bytes[4]].map(f32::from);
                let snorm = |byte: u8| (byte as i8 as f32 / 127.0).max(-1.0);
                Read {
                    position: place(unpacked),
                    type_index: aux[0] as usize,
                    motion: aux[1] / 256.0,
                    quaternion: [bytes[11], bytes[10], bytes[9], bytes[8]].map(snorm),
                    light,
                    sun: aux[3],
                }
            }
        }
    }

    fn placement(
        type_index: usize,
        position: [f32; 3],
        turn: f32,
        scale: f32,
        motion: f32,
    ) -> Placement {
        let (sin, cos) = (turn * 0.5).sin_cos();
        Placement {
            type_index,
            position,
            rotation: [0.0, 0.0, sin, cos],
            scale,
            motion,
            tint: [1.0; 3],
            ground_tint: 0.0,
        }
    }

    /// Each game's shader reads a placement back as it was laid out: Halo 3
    /// to a 16-bit step of the patch, Reach to an 11-bit one (10 up).
    #[test]
    fn a_placement_reads_back_as_each_shader_unpacks_it() {
        let placements = [
            placement(1, [-2.0, 1.5, 0.25], 0.6, 0.8, 0.6),
            placement(0, [3.0, -1.0, 0.0], 0.0, 1.5, 0.2),
        ];
        for (layout, tolerance) in [
            (InstanceLayout::Halo3, 1e-3),
            (InstanceLayout::Reach, 5.0 / 2047.0),
        ] {
            let packed = pack(
                layout,
                &placements,
                |placement| [0.2, 0.5, 3.0].map(|channel| channel * placement.scale),
                2,
            );
            assert_eq!(packed.types, vec![(0, 1), (1, 1)], "grouped by type");
            let read = unpack(layout, &packed, 1);
            assert_eq!(read.type_index, 1, "{layout:?}");
            assert!(
                read.position
                    .iter()
                    .zip(placements[0].position)
                    .all(|(a, b)| (a - b).abs() < tolerance),
                "{layout:?} {:?}",
                read.position
            );
            assert!(
                (read.motion - 153.0 / 256.0).abs() < 1e-6,
                "the motion byte over 256"
            );
            let length_squared: f32 = read.quaternion.iter().map(|q| q * q).sum();
            assert!(
                (length_squared - 0.4).abs() < 0.02,
                "{layout:?}: |q|² is half the scale: {length_squared}"
            );
            assert!(
                (read.quaternion[2] / read.quaternion[3] - (0.3_f32).tan()).abs() < 0.02,
                "{layout:?}: the turn"
            );
            // One scale for all three: each channel is exact to a step of the
            // brightest's, which the next quarter power of two above it sets
            // (RGBE), or the next eighth (RGBk).
            let expected = [0.16, 0.4, 2.4];
            let step = match layout {
                InstanceLayout::Halo4 => {
                    ((2.4_f32 * 8.0 + 1.0).floor() / 8.0) / 255.0 + 2.4 / 255.0
                }
                _ => ((2.4_f32.log2() * 4.0).ceil() / 4.0).exp2() / 255.0,
            };
            assert!(
                read.light
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() <= step),
                "{:?}",
                read.light
            );
            let sun = if layout == InstanceLayout::Reach {
                255.0
            } else {
                0.0
            };
            assert_eq!(read.sun, sun, "Reach's placements stand in full sun");
        }
    }

    #[test]
    fn scale_is_clamped_to_what_a_byte_quaternion_carries() {
        for layout in [InstanceLayout::Halo3, InstanceLayout::Reach] {
            let packed = pack(
                layout,
                &[placement(0, [0.0; 3], 0.0, 5.0, 0.0)],
                |_| [1.0; 3],
                1,
            );
            let read = unpack(layout, &packed, 0);
            assert!(
                (read.quaternion[3] - 1.0).abs() < 0.01,
                "{layout:?} {:?}",
                read.quaternion
            );
        }
    }
}
