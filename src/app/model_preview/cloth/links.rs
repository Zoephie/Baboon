//! A sheet's grid, links, pins and spheres.

use super::*;

/// `wind_exclude_from_pole`: `z` is cut off, and the rest shares what remains.
pub(super) fn held_from_straight_up(vector: [f32; 3], angle: f32) -> [f32; 3] {
    let limit = angle.cos();
    let z = vector[2].clamp(-limit, limit);
    let flat = (vector[0] * vector[0] + vector[1] * vector[1]).sqrt();
    let [x, y] = if flat < 0.0001 || (vector[0].abs() < 0.01 && vector[1].abs() < 0.01) {
        [1.0, 0.0]
    } else {
        [vector[0] / flat, vector[1] / flat]
    };
    let across = (1.0 - z * z).max(0.0).sqrt();
    [x * across, y * across, z]
}

/// The bit trick that `cloth_constrain_positions` takes square roots by.
pub(super) fn rough_sqrt(value: f32) -> f32 {
    f32::from_bits((value.to_bits() >> 1) + 532_676_608)
}

/// `real_vector3d_exclude_from_pole_by_angle`.
pub(crate) fn away_from_pole(pole: [f32; 3], angle: f32, vector: [f32; 3]) -> [f32; 3] {
    let length = dot(vector, vector).sqrt();
    let (pole, toward) = (unit_or_unchanged(pole), unit_or_unchanged(vector));
    let between = dot(pole, toward).clamp(-1.0, 1.0).acos();
    if angle <= f32::EPSILON || between >= angle {
        return vector;
    }
    let axis = unit_or_unchanged(cross(pole, toward));
    unit_or_unchanged(turned_about(toward, axis, angle - between)).map(|axis| axis * length)
}

/// `cloth_generate_flag_grid`: rows hang down from the top, and a square of
/// texture spans the longer side. Reach's `create_cloth_grid_data` (the 360's
/// 0x82C916E8, MCC's 0x140D994E0) spans the texture across each side by its
/// own step instead: `u = j / (wide - 1)`, `v = 1 - i / (high - 1)`.
pub(super) fn flag_grid(
    size: [usize; 2],
    spacing: [f32; 2],
    game: ClothGame,
) -> (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<usize>) {
    let rules = game.rules();
    let most = rules.most_vertices;
    let [mut wide, mut high] = size;
    if wide * high > most {
        if high > wide {
            high = most / wide;
        } else {
            wide = most / high;
        }
    }
    let (mut places, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    if wide < 2 || high < 2 {
        return (places, uvs, indices);
    }
    let shared = ((wide - 1).max(high - 1) as f32).recip();
    let [across, down] = if rules.spans_texture_per_side {
        [
            ((wide - 1).max(1) as f32).recip(),
            ((high - 1).max(1) as f32).recip(),
        ]
    } else {
        [shared; 2]
    };
    for row in 0..high {
        for column in 0..wide {
            places.push([column as f32 * spacing[0], 0.0, -(row as f32) * spacing[1]]);
            uvs.push([
                column as f32 * across,
                (high - 1) as f32 * down - row as f32 * down,
            ]);
        }
    }
    for row in 0..high - 1 {
        for column in 0..wide - 1 {
            let at = column + wide * row;
            indices.extend([at, at + wide, at + 1, at + 1, at + wide, at + wide + 1]);
        }
    }
    (places, uvs, indices)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ClothLink {
    pub(super) ends: [usize; 2],
    pub(super) length: f32,
}

pub(super) fn find_link(links: &[ClothLink], a: usize, b: usize) -> Option<usize> {
    links
        .iter()
        .position(|link| link.ends == [a, b] || link.ends == [b, a])
}

/// `cloth_add_link`.
fn add_link(links: &mut Vec<ClothLink>, places: &[[f32; 3]], a: usize, b: usize) {
    links.push(ClothLink {
        ends: [a, b],
        length: distance(places[a], places[b]),
    });
}

/// `cloth_add_link_if_short`: what stiffens the sheet, up to `longest_rod`
/// of the link it grew from.
fn add_short_link(
    links: &mut Vec<ClothLink>,
    places: &[[f32; 3]],
    longest: f32,
    a: usize,
    b: usize,
    scale: f32,
) {
    if a != b
        && find_link(links, a, b).is_none()
        && longest >= distance(places[a], places[b]) * scale
    {
        add_link(links, places, a, b);
    }
}

/// `cloth_generate_links`: an edge first met is a link, and one met again links
/// what lies across it. Then every link stiffens each of its two ends.
pub(super) fn generate_links(
    places: &[[f32; 3]],
    indices: &[usize],
    longest: f32,
) -> Vec<ClothLink> {
    let mut links = Vec::new();
    for triangle in indices.chunks_exact(3) {
        for edge in 0..3 {
            let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
            let Some(found) = find_link(&links, a, b) else {
                add_link(&mut links, places, a, b);
                continue;
            };
            let across = triangle[(edge + 2) % 3];
            let ClothLink {
                ends: [first, second],
                length,
            } = links[found];
            let other = indices.chunks_exact(3).find_map(|triangle| {
                (0..3).find_map(|at| {
                    let (near, far) = (triangle[at], triangle[(at + 1) % 3]);
                    let opposite = triangle[(at + 2) % 3];
                    let same = (near == first && far == second) || (near == second && far == first);
                    (same && opposite != across).then_some(opposite)
                })
            });
            if let Some(other) = other {
                add_short_link(&mut links, places, longest, across, other, length.recip());
            }
        }
    }
    for at in 0..links.len() {
        let ClothLink {
            ends: [a, b],
            length,
        } = links[at];
        for (from, not) in [(a, b), (b, a)] {
            for triangle in indices.chunks_exact(3) {
                if triangle.contains(&from) && !triangle.contains(&not) {
                    for corner in triangle {
                        add_short_link(&mut links, places, longest, from, *corner, length.recip());
                    }
                }
            }
        }
    }
    links
}

/// What pins the sheet: a vertex to a marker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ClothPin {
    pub(super) vertex: usize,
    pub(super) node: usize,
    pub(super) position: [f32; 3],
}

/// A marker that the sheet stays out of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ClothSphere {
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) radius: f32,
}

/// `cloth_setup_attachments`: the markers in order along an axis, first to the
/// sheet's first corner and last to the far one of its first column. Reach
/// takes ten markers to the others' six, and pins no secondary: it would pin
/// the last vertex to a marker slot that is never filled.
pub(super) fn cloth_pins(
    mut markers: Vec<(usize, [f32; 3])>,
    secondary: Option<(usize, [f32; 3])>,
    size: [usize; 2],
    vertex_count: usize,
    top: bool,
    game: ClothGame,
) -> Vec<ClothPin> {
    let rules = game.rules();
    let most = rules.most_attachments;
    // The game's result is undefined, and no kit tag sets one. [inferred]
    let secondary = secondary.filter(|_| rules.pins_secondary);
    markers.truncate(most);
    let axis = if top { 0 } else { 2 };
    markers.sort_by(|a, b| a.1[axis].total_cmp(&b.1[axis]));
    let count = markers.len();
    let [wide, high] = size;
    let increment = high as f32 / count.max(1) as f32;
    let mut pins: Vec<ClothPin> = markers
        .iter()
        .enumerate()
        .map(|(at, (node, position))| {
            let vertex = if at == 0 {
                0
            } else if at == count - 1 {
                (high - 1) * wide
            } else {
                (increment * at as f32).ceil() as usize * wide
            };
            ClothPin {
                vertex,
                node: *node,
                position: *position,
            }
        })
        .collect();
    if let (true, Some((node, position))) = (pins.len() < most, secondary) {
        pins.push(ClothPin {
            vertex: vertex_count - 1,
            node,
            position,
        });
    }
    pins
}
