//! Where the preview stands a set's decorators: a patch of flat ground
//! filled the way the game's brush fills one.
//!
//! Reach's `place_randomized_decorator` (360 0x82796b70) and Halo 4's (360
//! 0x83a71e28) give each placement a random hover along the ground's normal,
//! motion in the type's wind range, a spin about the up axis when the type
//! rotates at random, a tilt in its range about a random level axis, a scale
//! in its range, a random mix of its three colours and a ground tint in its
//! range. Halo 3's placement is Sapien's, which no database here holds, and
//! its placements in the kit's levels land within the same ranges. The
//! ground here is flat, so aligning to its normal changes nothing; `align
//! random` and `rotate x axis down` are not applied, their maths unread.

use super::super::particles::Random;
use super::{DecoratorSet, Placement};

/// What the patch is filled with, and how its stand-ins for a level draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Layout {
    /// The patch's edge, in world units, centred on the origin.
    pub size: f32,
    /// How many decorators it tries to stand.
    pub count: usize,
    pub seed: u32,
    /// Whether the game's fade with distance draws (the Distance card).
    pub fade: bool,
    /// How bright the light a level would bake into each decorator is, as a
    /// share of the stand-in's (the Lighting card).
    pub light: f32,
    /// Whether the wind sways what its shader sways (the Wind card).
    pub sway: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            size: 4.0,
            count: 250,
            seed: 1,
            fade: false,
            light: 1.0,
            sway: true,
        }
    }
}

/// Tries per decorator before the patch is called full: a spot too close to
/// another of its type is drawn again.
const TRIES: usize = 30;

/// The placements `layout` asks for: each a type picked by its placement
/// weight (evenly where the types have none), kept `minimum distance between
/// decorators` from the others of its type.
pub(in crate::app::model_preview) fn scatter(set: &DecoratorSet, layout: Layout) -> Vec<Placement> {
    let weights: Vec<f32> = set
        .types
        .iter()
        .map(|kind| kind.placement_weight.unwrap_or(1.0).max(0.0))
        .collect();
    let total: f32 = weights.iter().sum();
    let mut random = Random(layout.seed);
    let mut placements: Vec<Placement> = Vec::with_capacity(layout.count);
    let half = layout.size * 0.5;
    for _ in 0..layout.count {
        let type_index = pick(&weights, total, &mut random);
        let kind = &set.types[type_index];
        let spacing = kind.minimum_distance.max(0.0);
        let spot = (0..TRIES).find_map(|_| {
            let spot = [random.range(-half, half), random.range(-half, half)];
            let clear = placements
                .iter()
                .filter(|other| other.type_index == type_index)
                .all(|other| {
                    let [dx, dy] = [other.position[0] - spot[0], other.position[1] - spot[1]];
                    dx * dx + dy * dy >= spacing * spacing
                });
            clear.then_some(spot)
        });
        let Some([x, y]) = spot else {
            continue;
        };
        let hover = random.range(kind.hover[0], kind.hover[1]);
        let spin = if kind.random_rotation {
            random.range(0.0, std::f32::consts::TAU)
        } else {
            0.0
        };
        let tilt = random.range(kind.tilt[0], kind.tilt[1]).to_radians();
        let tilt_axis = random.range(0.0, std::f32::consts::TAU);
        let rotation = multiply(
            about([tilt_axis.cos(), tilt_axis.sin(), 0.0], tilt),
            about([0.0, 0.0, 1.0], spin),
        );
        let scale = random.range(kind.scale[0], kind.scale[1]);
        let motion = random.range(kind.wind[0], kind.wind[1]);
        let tint = mix(kind.colors, &mut random);
        let ground_tint = random.range(kind.ground_tint[0], kind.ground_tint[1]);
        placements.push(Placement {
            type_index,
            position: [x, y, hover],
            rotation,
            scale,
            motion,
            tint,
            ground_tint,
        });
    }
    placements
}

/// A type by its weight, the first when no type has any.
fn pick(weights: &[f32], total: f32, random: &mut Random) -> usize {
    if total <= 0.0 {
        return 0;
    }
    let mut left = random.next() * total;
    for (index, weight) in weights.iter().enumerate() {
        if left < *weight {
            return index;
        }
        left -= weight;
    }
    weights
        .iter()
        .rposition(|weight| *weight > 0.0)
        .unwrap_or(0)
}

/// A random point between three colours, each mix as likely as another.
fn mix(colors: [[f32; 3]; 3], random: &mut Random) -> [f32; 3] {
    let (mut u, mut v) = (random.next(), random.next());
    if u + v > 1.0 {
        (u, v) = (1.0 - u, 1.0 - v);
    }
    let w = 1.0 - u - v;
    std::array::from_fn(|channel| {
        colors[0][channel] * w + colors[1][channel] * u + colors[2][channel] * v
    })
}

/// The turn by `angle` radians about the unit `axis`, `[x, y, z, w]`.
fn about(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (sin, cos) = (angle * 0.5).sin_cos();
    [axis[0] * sin, axis[1] * sin, axis[2] * sin, cos]
}

/// `a` after `b`: the turn that applies `b`, then `a`.
fn multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `vector` turned by the unit quaternion `rotation`.
pub(super) fn rotate(rotation: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = rotation;
    let q = [x, y, z];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross(q, vector).map(|value| value * 2.0);
    let u = cross(q, t);
    std::array::from_fn(|axis| vector[axis] + w * t[axis] + u[axis])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_about_up_spins_level_axes_and_keeps_up() {
        let quarter = about([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2);
        let turned = rotate(quarter, [1.0, 0.0, 0.0]);
        assert!(
            (turned[0]).abs() < 1e-6 && (turned[1] - 1.0).abs() < 1e-6,
            "{turned:?}"
        );
        let up = rotate(quarter, [0.0, 0.0, 1.0]);
        assert!((up[2] - 1.0).abs() < 1e-6);
        // Applying `b` then `a` is one turn of both.
        let both = multiply(quarter, quarter);
        let half = rotate(both, [1.0, 0.0, 0.0]);
        assert!((half[0] + 1.0).abs() < 1e-5, "{half:?}");
    }

    #[test]
    fn a_mix_of_three_colours_stays_between_them() {
        let mut random = Random(7);
        let colors = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for _ in 0..1000 {
            let mixed = mix(colors, &mut random);
            assert!(mixed.iter().all(|channel| (0.0..=1.0).contains(channel)));
            assert!((mixed.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn types_are_picked_by_weight_and_kept_apart() {
        use super::super::DecoratorType;
        let kind = |weight: f32, spacing: f32| DecoratorType {
            scale: [1.0, 1.0],
            minimum_distance: spacing,
            placement_weight: Some(weight),
            ..Default::default()
        };
        let set = DecoratorSet {
            types: vec![kind(3.0, 0.5), kind(1.0, 0.0), kind(0.0, 0.0)],
            ..Default::default()
        };
        let layout = Layout {
            size: 20.0,
            count: 400,
            seed: 3,
            ..Default::default()
        };
        let placements = scatter(&set, layout);
        let count = |type_index| {
            placements
                .iter()
                .filter(|placement| placement.type_index == type_index)
                .count()
        };
        assert_eq!(count(2), 0, "a type of no weight is never picked");
        assert!(count(0) > 2 * count(1), "{} against {}", count(0), count(1));
        let first: Vec<_> = placements
            .iter()
            .filter(|placement| placement.type_index == 0)
            .collect();
        for (index, a) in first.iter().enumerate() {
            for b in &first[index + 1..] {
                let distance = ((a.position[0] - b.position[0]).powi(2)
                    + (a.position[1] - b.position[1]).powi(2))
                .sqrt();
                assert!(distance >= 0.5, "{distance}");
            }
        }
        assert_eq!(
            scatter(&set, layout),
            placements,
            "a seed lays the same patch every time"
        );
    }
}
