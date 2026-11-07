//! Halo 3's cloth: a sheet of points kept apart by links, pinned to markers and
//! blown by the wind. The tag holds only the sheet's size; `tool.exe` works the
//! rest out, so it is worked out here the same way.

use blam_tags::{TagFile, TagStruct};

use super::attachment::MarkerFrame;
use super::common::fields::{elements, has_flag, object_block};
use super::common::view::posed_on;
use super::effects::marker_name;
use super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, dot, minus, unit_or_unchanged};
use crate::app::model_preview::common::random::{high_bits, lcg_step};

/// `k_maximum_vertices_per_cloth`.
const MOST_VERTICES: usize = 121;
/// The same, in Halo 2.
const H2_MOST_VERTICES: usize = 128;
/// `k_maximum_attachments_per_cloth`.
const MOST_ATTACHMENTS: usize = 6;
/// Reach's: `cloth_setup_attachments` (the 360's 0x82C8E450) fetches up to
/// `NUMBEROF(marker)`, ten, of the attachment's markers.
const REACH_MOST_ATTACHMENTS: usize = 10;
/// `k_maximum_collision_spheres_per_cloth`.
const MOST_SPHERES: usize = 8;
/// `cloth_constrain_positions_relaxation`'s, past the tag's own limit because
/// a moving object adds to them; a preview's stands still.
const MOST_ITERATIONS: i32 = 15;
/// Its passes, a sheet that keeps coming apart included.
const MOST_PASSES: usize = 16;
/// Reach's (the 360's 0x82C8EE80, MCC's 0x140D96750): at most 9 iterations,
/// and the tenth pass ends the loop (`if (pass++ == 9) i = N`).
const REACH_MOST_ITERATIONS: i32 = 9;
const REACH_MOST_PASSES: usize = 10;
/// How far Reach's `cloth_enforce_attachments` (the 360's 0x82C8F9B0) lets
/// the object's bounding sphere move before it carries the whole sheet along.
const REACH_CENTER_JUMP: f32 = 0.4;
/// The relaxations Reach's `cloth_new_attachment` (the 360's 0x82C8E2F0)
/// settles a new sheet with.
const REACH_CREATION_RELAXATIONS: usize = 5;
const ERROR_MAX: f32 = 2.0;
/// How many links it keeps of those that touch a pin.
const MOST_HELD_LINKS: usize = 1210;
/// The shortest a link is measured at, and how near a sphere's centre a point
/// has no way out but `global_left3d`.
const NEARLY_ZERO: f32 = 0.0001;

/// Which game's cloth a sheet is: they share the sheet and its links, and
/// differ in their limits, wind, ticks and how they start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClothGame {
    Halo2,
    /// Halo 3 and ODST.
    Halo3,
    Reach,
}

/// Reach's game ticks a second, for its cloth and its antennas. \[inferred\]
/// The rate comes from `game_options +12` (the 360's
/// `game_time_initialize_for_new_map`, 0x82D7A998), which was not read for
/// MCC. 30 is the 360's, and the antennas' fixed step, 0.0333 a tick, is a
/// 30th of a second.
pub(crate) const REACH_TICK_RATE: f32 = 30.0;

/// Where the games' cloth differs, decision by decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClothRules {
    /// The game's ticks a second: `cloth_update` runs each cloth at most once
    /// a tick (`last_update_tick`), whatever the frame's time. Halo 2 ticks at
    /// 30; Halo 3 and ODST at their native rate, the 60 of `get_vblank_rate`;
    /// Reach at [`REACH_TICK_RATE`].
    pub(crate) tick_rate: f32,
    /// The most vertices a sheet keeps (`cloth_generate_flag_grid`).
    most_vertices: usize,
    /// Reach's `create_cloth_grid_data` spans the texture across each side by
    /// its own step, where the others span a square across the longer side.
    spans_texture_per_side: bool,
    /// The most attachment markers a sheet pins to: Reach takes ten to the
    /// others' six.
    most_attachments: usize,
    /// Whether the sheet's last vertex pins to the secondary marker. Reach's
    /// would pin it to a slot that is never filled.
    pins_secondary: bool,
    /// Whether the tag's properties are held to `tool.exe`'s ranges. Halo 2's
    /// tool limits none of them.
    clamps_properties: bool,
    /// How `cloth_render_opaque` frames each side.
    sheet_basis: ClothSheetBasis,
    /// Whether the preview's region for a sheet keeps the tag's `.cloth`.
    region_keeps_extension: bool,
    /// `cloth_reset_flag_to_default_positions`: Halo 2 and 3 move the sheet up
    /// to its first pin; Reach's leaves it at the world's origin.
    rests_at_first_pin: bool,
    /// Reach's `cloth_enforce_attachments` first carries the whole sheet with
    /// the object's bounding sphere, where that has jumped.
    follows_center: bool,
    /// `cloth_constrain_positions_relaxation`'s limits: the iterations a pass
    /// runs, and the passes before it gives up.
    most_iterations: i32,
    most_passes: usize,
    /// Reach's `cloth_new_attachment` pins a new sheet and relaxes it five
    /// times.
    settles_new_sheet: bool,
    /// Whose wind blows the sheets.
    wind: ClothWind,
    /// Whether the sheet relaxes as Halo 2's does.
    relaxes_as_halo2: bool,
    /// Halo 2 adds to what the normal was, so each carries a little of the
    /// last tick, and an unset one points up; the others start from nothing
    /// and point along x.
    normals_carry_over: bool,
    /// The sign of the back side's normal: Halo 2 turns it over, Halo 3 and
    /// Reach keep the one normal.
    back_normal_sign: f32,
    /// Whether each vertex's binormal follows its normal, as Reach's does.
    derives_binormal: bool,
}

/// How a cloth's two sides are framed in the model's vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClothSheetBasis {
    /// Halo 2's `cloth_render` turns the whole frame over with the side.
    TurnsOver,
    /// Halo 3's binormal turns over, as loaded.
    BinormalTurns,
    /// Both sides one frame, from the rest normal, `global_left3d`.
    RestNormal,
}

/// Whose wind blows a slot's cloth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClothWind {
    Halo2,
    /// Halo 3's wind clock counts the ticks the wind was authored at, 30 a second.
    Halo3,
    /// Reach's moves as each cloth's verlet asks, on its own clock.
    PerCloth,
}

impl ClothGame {
    /// The game's cloth rules.
    pub(crate) const fn rules(self) -> ClothRules {
        let reach = matches!(self, ClothGame::Reach);
        let halo2 = matches!(self, ClothGame::Halo2);
        ClothRules {
            tick_rate: match self {
                ClothGame::Halo2 => 30.0,
                ClothGame::Halo3 => 60.0,
                ClothGame::Reach => REACH_TICK_RATE,
            },
            most_vertices: if halo2 { H2_MOST_VERTICES } else { MOST_VERTICES },
            spans_texture_per_side: reach,
            most_attachments: if reach { REACH_MOST_ATTACHMENTS } else { MOST_ATTACHMENTS },
            pins_secondary: !reach,
            clamps_properties: !halo2,
            sheet_basis: match self {
                ClothGame::Halo2 => ClothSheetBasis::TurnsOver,
                ClothGame::Halo3 => ClothSheetBasis::BinormalTurns,
                ClothGame::Reach => ClothSheetBasis::RestNormal,
            },
            region_keeps_extension: reach,
            rests_at_first_pin: !reach,
            follows_center: reach,
            most_iterations: if reach { REACH_MOST_ITERATIONS } else { MOST_ITERATIONS },
            most_passes: if reach { REACH_MOST_PASSES } else { MOST_PASSES },
            settles_new_sheet: reach,
            wind: match self {
                ClothGame::Halo2 => ClothWind::Halo2,
                ClothGame::Halo3 => ClothWind::Halo3,
                ClothGame::Reach => ClothWind::PerCloth,
            },
            relaxes_as_halo2: halo2,
            normals_carry_over: halo2,
            back_normal_sign: if halo2 { -1.0 } else { 1.0 },
            derives_binormal: reach,
        }
    }
}

/// The game's ticks a second; see [`ClothRules::tick_rate`].
fn tick_rate(game: ClothGame) -> f32 {
    game.rules().tick_rate
}

/// `cloth_update_positions_verlet`'s gravity for one tick at `rate`.
fn gravity_per_tick(rate: f32) -> f32 {
    -9.78 * 0.328_084 / rate / 32.0
}

/// `cloth_update_wind` in Halo 2: its limit from straight up, and the ticks per
/// second that its clock counts in.
const H2_WIND_POLE_ANGLE: f32 = 0.349_065_9;
const H2_WIND_CLOCK: f32 = 30.0;
/// Halo 2 has no limit on a sheet that keeps coming apart; this one is ours.
const H2_MOST_PASSES: usize = 64;

/// `real_vector3d_exclude_from_pole_by_angle`'s and the wind's own.
const WIND_POLE_ANGLE: f32 = 20.0 * std::f32::consts::PI / 180.0;





fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let between = minus(a, b);
    dot(between, between).sqrt()
}

/// `rotate_vector_about_axis`.
pub(crate) fn turned_about(vector: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let (sin, cos) = angle.sin_cos();
    let along = dot(axis, vector) * (1.0 - cos);
    let across = cross(axis, vector);
    std::array::from_fn(|at| vector[at] * cos + across[at] * sin + axis[at] * along)
}

/// `cloth_update_wind`: one wind for everything, wandering by itself. It reads
/// nothing of a level, so a preview blows as a map does.
#[derive(Debug, Clone)]
pub(crate) struct H3Wind {
    direction: [f32; 3],
    scale: f32,
    random: u32,
}

impl Default for H3Wind {
    fn default() -> Self {
        Self { direction: [0.0; 3], scale: 0.0, random: 1 }
    }
}

/// The random generator the winds draw on: the seed's next step, and the
/// upper half of it.
fn next_random(seed: &mut u32) -> f32 {
    *seed = lcg_step(*seed);
    high_bits(*seed) as f32
}

/// `random_direction3d`, from two of `next`'s draws in `0..1`.
fn random_direction(mut next: impl FnMut() -> f32) -> [f32; 3] {
    let z = next() * 2.0 - 1.0;
    let (sin, cos) = (next() * std::f32::consts::TAU).sin_cos();
    let flat = (1.0 - z * z).max(0.0).sqrt();
    [flat * cos, flat * sin, z]
}

impl H3Wind {
    fn next(&mut self) -> f32 {
        next_random(&mut self.random) / 65_536.0
    }

    fn random_direction(&mut self) -> [f32; 3] {
        random_direction(|| self.next())
    }

    /// What the wind accelerates by this tick, `tick` counting from the start
    /// in the 30-a-second ticks `cloth_update_wind` measures its clock in.
    pub(crate) fn step(&mut self, tick: f32) -> [f32; 3] {
        let t = tick * 0.3;
        if self.direction == [0.0; 3] {
            self.direction = self.random_direction();
        }
        let angle = self.next() * (t * 0.005).cos() * (t * 0.005).cos() * 0.15;
        let about = self.random_direction();
        self.direction = turned_about(self.direction, about, angle);
        self.direction = away_from_pole([0.0, 0.0, 1.0], WIND_POLE_ANGLE, self.direction);
        let slow = (t * 0.0001).cos();
        let drift = ((t * 0.005).sin() * 0.01 * slow + (t * 0.001).cos() * (t * 0.0006).cos() * 0.01 * slow) * 0.4;
        self.scale = (self.scale + drift).clamp(-0.1, 0.1);
        let strength = (self.scale.abs() + 0.0075) * 0.4 * 0.2;
        self.direction.map(|axis| axis * strength)
    }
}

/// Halo 2's `cloth_update_wind`, whose one wind wanders by other sums.
#[derive(Debug, Clone)]
struct H2Wind {
    direction: [f32; 3],
    scale: f32,
    random: u32,
}

impl Default for H2Wind {
    fn default() -> Self {
        Self { direction: [0.0; 3], scale: 0.1, random: 1 }
    }
}

impl H2Wind {
    fn next(&mut self) -> f32 {
        next_random(&mut self.random) * 1.525_902_2e-5
    }

    fn random_direction(&mut self) -> [f32; 3] {
        random_direction(|| self.next())
    }

    /// What the wind accelerates by this tick, before the cloth's own scale.
    fn step(&mut self, tick: f32) -> [f32; 3] {
        let t = tick * H2_WIND_CLOCK;
        let turn = (t * 0.005).cos();
        let angle = self.next() * turn * turn * 0.15;
        if self.direction == [0.0; 3] {
            self.direction = self.random_direction();
        }
        let about = self.random_direction();
        self.direction = held_from_straight_up(turned_about(self.direction, about, angle), H2_WIND_POLE_ANGLE);
        let slow = (t * 0.0001).cos();
        let drift = (t * 0.001).cos() * (t * 0.0006).cos() * 0.01 * slow + (t * 0.005).sin() * 0.01 * slow;
        self.scale = (self.scale + drift * 0.3).clamp(-0.1, 0.1);
        let strength = (self.scale.abs() + 0.0075) * 0.4 * 0.2;
        self.direction.map(|axis| axis * strength)
    }
}

/// Reach's `cloth_update_wind` (the 360's 0x82C902A0, MCC's 0x140D98EF0):
/// one wind for every cloth, which moves on only when a new frame is drawn.
#[derive(Debug, Clone)]
struct ReachWind {
    direction: [f32; 3],
    scale: f32,
    random: u32,
    /// `c_render_globals::get_frame_index()` when it last moved.
    last_frame: u64,
}

impl Default for ReachWind {
    fn default() -> Self {
        Self { direction: [0.0; 3], scale: 0.0, random: 1, last_frame: 0 }
    }
}

/// Reach's least and greatest wind, either way.
const REACH_WIND_LEAST: f32 = 0.05;
const REACH_WIND_MOST: f32 = 0.2;

impl ReachWind {
    /// `real_random`: the upper half of the seed over 65535.
    fn next(&mut self) -> f32 {
        next_random(&mut self.random) * (1.0 / 65_535.0)
    }

    /// What the wind accelerates by, before the cloth's own scale. It moves
    /// only on a `frame` past the last it moved on; `t` is its clock,
    /// `game_time · game_tick_rate + (datum index & 0xFF) · 0.75`, ticks and
    /// rate together as both binaries have it.
    fn step(&mut self, frame: u64, t: f32) -> [f32; 3] {
        // Drawn once, and again only if it came out too short to turn.
        if self.direction == [0.0; 3] {
            let direction = random_direction(|| self.next());
            if dot(direction, direction).sqrt() >= NEARLY_ZERO {
                self.direction = unit_or_unchanged(direction);
            }
        }
        if frame > self.last_frame {
            self.last_frame = frame;
            let turn = (t * 0.005).cos();
            let angle = self.next() * turn * turn * 0.15;
            // `random_direction3d` is taken to draw as Halo 3's does. [inferred]
            let about = random_direction(|| self.next());
            self.direction = turned_about(self.direction, about, angle);
            self.direction = away_from_pole([0.0, 0.0, 1.0], WIND_POLE_ANGLE, self.direction);
            let slow = (t * 0.0001).cos();
            let drift = 0.01 * (t * 0.005).sin() * slow + 0.01 * (t * 0.0006).cos() * (t * 0.001).cos() * slow;
            self.scale += drift;
            // Never stiller than its least: nothing at all counts as the other way.
            self.scale = if self.scale <= 0.0 {
                self.scale.min(-REACH_WIND_LEAST)
            } else {
                self.scale.max(REACH_WIND_LEAST)
            };
            self.scale = self.scale.clamp(-REACH_WIND_MOST, REACH_WIND_MOST);
        }
        self.direction.map(|axis| axis * self.scale.abs())
    }
}

/// `wind_exclude_from_pole`: `z` is cut off, and the rest shares what remains.
fn held_from_straight_up(vector: [f32; 3], angle: f32) -> [f32; 3] {
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
fn rough_sqrt(value: f32) -> f32 {
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
fn flag_grid(size: [usize; 2], spacing: [f32; 2], game: ClothGame) -> (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<usize>) {
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
        [((wide - 1).max(1) as f32).recip(), ((high - 1).max(1) as f32).recip()]
    } else {
        [shared; 2]
    };
    for row in 0..high {
        for column in 0..wide {
            places.push([column as f32 * spacing[0], 0.0, -(row as f32) * spacing[1]]);
            uvs.push([column as f32 * across, (high - 1) as f32 * down - row as f32 * down]);
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
struct ClothLink {
    ends: [usize; 2],
    length: f32,
}

fn find_link(links: &[ClothLink], a: usize, b: usize) -> Option<usize> {
    links.iter().position(|link| link.ends == [a, b] || link.ends == [b, a])
}

/// `cloth_add_link`.
fn add_link(links: &mut Vec<ClothLink>, places: &[[f32; 3]], a: usize, b: usize) {
    links.push(ClothLink { ends: [a, b], length: distance(places[a], places[b]) });
}

/// `cloth_add_link_if_short`: what stiffens the sheet, up to `longest_rod`
/// of the link it grew from.
fn add_short_link(links: &mut Vec<ClothLink>, places: &[[f32; 3]], longest: f32, a: usize, b: usize, scale: f32) {
    if a != b && find_link(links, a, b).is_none() && longest >= distance(places[a], places[b]) * scale {
        add_link(links, places, a, b);
    }
}

/// `cloth_generate_links`: an edge first met is a link, and one met again links
/// what lies across it. Then every link stiffens each of its two ends.
fn generate_links(places: &[[f32; 3]], indices: &[usize], longest: f32) -> Vec<ClothLink> {
    let mut links = Vec::new();
    for triangle in indices.chunks_exact(3) {
        for edge in 0..3 {
            let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
            let Some(found) = find_link(&links, a, b) else {
                add_link(&mut links, places, a, b);
                continue;
            };
            let across = triangle[(edge + 2) % 3];
            let ClothLink { ends: [first, second], length } = links[found];
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
        let ClothLink { ends: [a, b], length } = links[at];
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
struct ClothPin {
    vertex: usize,
    node: usize,
    position: [f32; 3],
}

/// A marker that the sheet stays out of.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ClothSphere {
    node: usize,
    position: [f32; 3],
    radius: f32,
}

/// `cloth_setup_attachments`: the markers in order along an axis, first to the
/// sheet's first corner and last to the far one of its first column. Reach
/// takes ten markers to the others' six, and pins no secondary: it would pin
/// the last vertex to a marker slot that is never filled.
fn cloth_pins(
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
            ClothPin { vertex, node: *node, position: *position }
        })
        .collect();
    if let (true, Some((node, position))) = (pins.len() < most, secondary) {
        pins.push(ClothPin { vertex: vertex_count - 1, node, position });
    }
    pins
}

/// A `cloth` on an object, and where its mesh lies in the model's vertices.
#[derive(Debug, Clone)]
pub(crate) struct H3ClothAt {
    places: Vec<[f32; 3]>,
    indices: Vec<usize>,
    links: Vec<ClothLink>,
    pins: Vec<ClothPin>,
    spheres: Vec<ClothSphere>,
    iterations: i32,
    gravity: f32,
    drag: f32,
    wind_scale: f32,
    flappiness: f32,
    /// The sheet's two sides in the model's vertices, each `places` long.
    first_vertex: usize,
    game: ClothGame,
    /// Reach's: the middle of the object's bounding sphere, and the node it
    /// is posed on, which `cloth_enforce_attachments` follows.
    center: (usize, [f32; 3]),
}

impl H3ClothAt {
    pub(crate) fn halo2(&self) -> bool {
        self.game == ClothGame::Halo2
    }

    #[cfg(test)]
    pub(crate) fn game(&self) -> ClothGame {
        self.game
    }

    /// Each pin's vertex, in the order the markers sort.
    #[cfg(test)]
    pub(crate) fn pinned_vertices(&self) -> Vec<usize> {
        self.pins.iter().map(|pin| pin.vertex).collect()
    }
}

fn cloth_tags(object: &TagFile) -> Vec<String> {
    let group = u32::from_be_bytes(*b"clwd");
    let mut paths = Vec::new();
    let widgets = object_block(object, "widgets").map_or(Vec::new(), |holder| elements(holder, "widgets"));
    let attachments = object_block(object, "attachments").map_or(Vec::new(), |holder| elements(holder, "attachments"));
    for entry in widgets.iter().chain(&attachments) {
        if let Some((found, path)) = entry.read_tag_ref_with_group("type")
            && found == group
            && !path.is_empty()
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

fn read_cloth_tag(
    root: TagStruct<'_>,
    markers: &[RenderModelPreviewMarker],
    frame: &impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
    game: ClothGame,
    center: [f32; 3],
) -> Option<(H3ClothAt, Vec<[f32; 2]>, Option<(String, String)>)> {
    let clamps = game.rules().clamps_properties;
    // The ranges are `tool.exe`'s postprocess, which tags on disk have not been through.
    let int = |name: &str| root.read_int_any(name).unwrap_or(0).clamp(2, 32) as usize;
    let size = [int("grid x dimension"), int("grid y dimension")];
    let spacing = [root.read_real("grid spacing x")?, root.read_real("grid spacing y")?];
    let (places, uvs, indices) = flag_grid(size, spacing, game);
    if places.is_empty() {
        return None;
    }
    let properties = root.descend("properties")?;
    // Halo 2's tool limits none of these.
    let real = |name: &str, low: f32, high: f32| {
        let value = properties.read_real(name).unwrap_or(0.0);
        if clamps { value.clamp(low, high) } else { value }
    };
    // The bounding sphere's middle, carried as a marker of the object's own.
    let middle = RenderModelPreviewMarker { position: center, scale: 1.0, ..Default::default() };
    let (middle, middle_node) = frame(&middle);
    let placed = |name: &str| -> Vec<(usize, [f32; 3])> {
        let named = markers.iter().filter(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name));
        named
            .map(|marker| {
                let (place, node) = frame(marker);
                (node, place.position)
            })
            .collect()
    };
    let attach_name = marker_name(root, "marker attachment name");
    let attach = placed(&attach_name);
    if attach.is_empty() {
        return None;
    }
    // The game finds markers only on shown permutations, so a sheet hung on
    // one goes with it.
    let under = markers
        .iter()
        .find(|marker| marker.name.eq_ignore_ascii_case(&attach_name))
        .and_then(|marker| marker.only_under.clone());
    let secondary = placed(&marker_name(root, "secondary axis attachment name")).first().copied();
    let high = places.len() / size[0].max(1);
    let spheres = elements(root, "collision spheres")
        .into_iter()
        .take(MOST_SPHERES)
        .filter_map(|sphere| {
            let (node, position) = *placed(&marker_name(sphere, "object marker name")).first()?;
            Some(ClothSphere { node, position, radius: sphere.read_real("radius")? })
        })
        .collect();
    let links = generate_links(&places, &indices, real("longest_rod", 1.0, 10.0));
    let top = has_flag(root, "flags", "Uses grid attach top");
    let pins = cloth_pins(attach, secondary, [size[0], high], places.len(), top, game);
    let cloth = H3ClothAt {
        places,
        indices,
        links,
        pins,
        spheres,
        iterations: properties.read_int_any("Number iterations").unwrap_or(1).clamp(1, 8) as i32,
        gravity: real("weight", -10.0, 10.0) * gravity_per_tick(tick_rate(game)),
        drag: real("drag", 0.0, 0.5),
        wind_scale: real("wind_scale", 0.0, 3.0),
        flappiness: properties.read_real("wind_flappiness_scale").unwrap_or(0.0),
        first_vertex: 0,
        game,
        center: (middle_node, middle.position),
    };
    Some((cloth, uvs, under))
}

/// Reach's binormal for a cloth vertex: `deform.hlsl_include` takes
/// `cross(normal, tangent)`, its sign from the position's w, which
/// `cloth_render_opaque` (the 360's 0x82C8D6A0) writes as 1; the tangent is
/// `global_forward3d`.
fn reach_binormal(normal: [f32; 3]) -> [f32; 3] {
    cross(normal, [1.0, 0.0, 0.0])
}

/// `cloth_render_opaque`'s mesh: both sides, to be written over each frame.
fn sheet_vertices(uvs: &[[f32; 2]], game: ClothGame) -> Vec<RenderModelPreviewVertex> {
    [1.0, -1.0]
        .iter()
        .flat_map(|side| {
            let (tangent, binormal) = match game.rules().sheet_basis {
                ClothSheetBasis::TurnsOver => ([1.0 * side, 0.0, 0.0], [0.0, 0.0, 1.0 * side]),
                ClothSheetBasis::BinormalTurns => ([1.0, 0.0, 0.0], [0.0, *side, 0.0]),
                ClothSheetBasis::RestNormal => ([1.0, 0.0, 0.0], reach_binormal([0.0, 1.0, 0.0])),
            };
            uvs.iter().map(move |uv| RenderModelPreviewVertex {
                texcoord: *uv,
                tangent,
                binormal,
                ..Default::default()
            })
        })
        .collect()
}

/// The region the preview lists a sheet under, which no game has: the tag's
/// name. Reach's keeps its `.cloth`, since its models' variants name regions
/// the render model lacks, and one such (the flag's `flag`) would hide a
/// sheet of the same name.
fn cloth_region_name(path: &str, game: ClothGame) -> String {
    let name = path.rsplit('\\').next().unwrap_or(path);
    if game.rules().region_keeps_extension {
        format!("{name}.cloth")
    } else {
        name.to_owned()
    }
}

/// An object's cloth, added to its model as a mesh of its own, drawn with the
/// tag's shader from the first active permutation of a region of its own.
/// `center` is the middle of the object's bounding sphere, in its own space,
/// where Reach starts a sheet; the other games start one at its first pin.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_cloth(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
    preview: &mut RenderModelPreview,
    game: ClothGame,
    center: [f32; 3],
) {
    for path in cloth_tags(object) {
        let Ok(tag) = load_referenced_tag_from_source(source, &path, "cloth", b"clwd") else {
            continue;
        };
        let root = tag.root();
        let Some((shader_group, shader_path)) = root.read_tag_ref_with_group("Shader") else {
            continue;
        };
        let Some((mut cloth, uvs, under)) = read_cloth_tag(root, markers, &frame, game, center) else {
            continue;
        };
        cloth.first_vertex = preview.vertices.len();
        let base = cloth.first_vertex as u32;
        let count = cloth.places.len() as u32;
        let index_start = preview.indices.len() as u32;
        preview.vertices.extend(sheet_vertices(&uvs, game));
        // The sides face apart, as `cloth_render_opaque`'s cull modes have it.
        preview.indices.extend(cloth.indices.iter().map(|index| *index as u32 + base));
        preview.indices.extend(cloth.indices.chunks_exact(3).rev().flat_map(|triangle| {
            [triangle[2], triangle[1], triangle[0]].map(|index| index as u32 + base + count)
        }));
        let name = cloth_region_name(&path, game);
        preview.materials.push(RenderModelPreviewMaterial {
            shader_path,
            shader_group,
            object: owner,
            ..Default::default()
        });
        // On the opened object it follows the region of the marker it hangs
        // from; an attached object's regions are renamed when it is merged.
        let (name, permutation) = match under.filter(|_| owner == 0) {
            Some(under) => under,
            None => {
                let permutation = "cloth".to_owned();
                preview.regions.push(RenderModelPreviewRegion { name: name.clone(), permutations: vec![permutation.clone()] });
                (name, permutation)
            }
        };
        preview.batches.push(RenderModelPreviewBatch {
            region_name: name,
            permutation_name: permutation,
            material_index: (preview.materials.len() - 1) as u16,
            index_start,
            index_count: cloth.indices.len() as u32 * 2,
            flat_color: None,
            layer: ModelPreviewLayer::Render,
            sort_position: None,
        });
        // Either game's PRT attributes stay parallel to the vertices.
        if let Some(prt) = preview.h2.prt.as_mut() {
            prt.pad(cloth.places.len() * 2);
        }
        if let Some(prt) = preview.gen3.prt.as_mut() {
            prt.pad(cloth.places.len() * 2);
        }
        preview.cloth.push(cloth);
    }
}

/// A cloth's points as they stand.
#[derive(Debug, Clone, Default)]
struct RunningCloth {
    places: Vec<[f32; 3]>,
    last: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    /// Reach's: where the object's bounding sphere is now, and where
    /// `cloth_enforce_attachments` last saw it.
    center: [f32; 3],
    last_center: [f32; 3],
}

impl RunningCloth {
    /// `cloth_reset_flag_to_default_positions`. Halo 2 and 3 move the sheet up
    /// to its first pin; Reach's (the 360's 0x82C90BF8) leaves it at the
    /// world's origin, for `pin` to carry to the object.
    fn at_rest(cloth: &H3ClothAt, pins: &[[f32; 3]], center: [f32; 3]) -> Self {
        let shift = if cloth.game.rules().rests_at_first_pin {
            pins.first().copied().unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        let places: Vec<[f32; 3]> =
            cloth.places.iter().map(|place| std::array::from_fn(|axis| place[axis] + shift[axis])).collect();
        Self { last: places.clone(), normals: vec![[0.0, 1.0, 0.0]; places.len()], places, center, last_center: [0.0; 3] }
    }

    /// `cloth_enforce_attachments`. Reach's first carries the whole sheet
    /// with the object's bounding sphere, where that has jumped.
    fn pin(&mut self, cloth: &H3ClothAt, pins: &[[f32; 3]]) {
        if cloth.game.rules().follows_center {
            let jump = minus(self.center, self.last_center);
            if dot(jump, jump) > REACH_CENTER_JUMP * REACH_CENTER_JUMP {
                for place in &mut self.places {
                    *place = std::array::from_fn(|axis| place[axis] + jump[axis]);
                }
                self.last.clone_from(&self.places);
            }
            self.last_center = self.center;
        }
        for (pin, place) in cloth.pins.iter().zip(pins) {
            self.places[pin.vertex] = *place;
        }
    }

    /// `cloth_constrain_positions_relaxation`. A sheet that has come apart goes
    /// back to rest and has that pass over, until the sixteenth ends it (in
    /// Reach, the tenth). Gives back how many passes it took.
    fn relax(&mut self, cloth: &H3ClothAt, pins: &[[f32; 3]]) -> usize {
        let rules = cloth.game.rules();
        let (most_iterations, most_passes) = (rules.most_iterations, rules.most_passes);
        let count = cloth.iterations.clamp(1, most_iterations);
        let held = |vertex: usize| cloth.pins.iter().any(|pin| pin.vertex == vertex);
        let mut passes = 0;
        let mut done = 0;
        loop {
            self.pin(cloth, pins);
            let mut worst = 0.0_f32;
            for link in cloth.links.iter().filter(|link| held(link.ends[0]) || held(link.ends[1])).take(MOST_HELD_LINKS) {
                let [a, b] = link.ends;
                let rod = minus(self.places[b], self.places[a]);
                let length = dot(rod, rod).sqrt();
                let delta = length - link.length;
                let share = rod.map(|axis| axis * delta * 0.5 / length.max(NEARLY_ZERO));
                if !held(a) {
                    let times = if held(b) { 2.0 } else { 1.0 };
                    self.places[a] = std::array::from_fn(|axis| self.places[a][axis] + share[axis] * times);
                }
                if !held(b) {
                    let times = if held(a) { 2.0 } else { 1.0 };
                    self.places[b] = std::array::from_fn(|axis| self.places[b][axis] - share[axis] * times);
                }
                worst = worst.max(delta.abs());
            }
            for link in &cloth.links {
                let [a, b] = link.ends;
                let rod = minus(self.places[b], self.places[a]);
                let length = dot(rod, rod).sqrt();
                let share = rod.map(|axis| axis * (length - link.length) * 0.5 / length.max(NEARLY_ZERO));
                self.places[a] = std::array::from_fn(|axis| self.places[a][axis] + share[axis]);
                self.places[b] = std::array::from_fn(|axis| self.places[b][axis] - share[axis]);
            }
            if worst > ERROR_MAX {
                *self = Self::at_rest(cloth, pins, self.center);
                done -= 1;
            }
            self.pin(cloth, pins);
            passes += 1;
            done = if passes == most_passes { count } else { done } + 1;
            if done >= count {
                return passes;
            }
        }
    }

    /// Halo 2's `cloth_constrain_positions`: one pass over every link, by a
    /// rule of its own, with a held end leaving all the work to the other.
    fn relax_h2(&mut self, cloth: &H3ClothAt, pins: &[[f32; 3]]) {
        let held = |vertex: usize| cloth.pins.iter().any(|pin| pin.vertex == vertex);
        let count = cloth.iterations.clamp(1, MOST_ITERATIONS);
        let (mut done, mut passes) = (0, 0);
        self.pin(cloth, pins);
        while done < count && passes < H2_MOST_PASSES {
            for link in &cloth.links {
                let [a, b] = link.ends;
                let rod = minus(self.places[b], self.places[a]);
                let rest = link.length * link.length;
                let length = rough_sqrt(dot(rod, rod)).max(NEARLY_ZERO);
                let pull = (rest / (length * length + rest) - 0.5) * length;
                let share = rod.map(|axis| axis / length * pull);
                if !held(a) {
                    let times = if held(b) { 2.0 } else { 1.0 };
                    self.places[a] = std::array::from_fn(|axis| self.places[a][axis] - share[axis] * times);
                }
                if !held(b) {
                    let times = if held(a) { 2.0 } else { 1.0 };
                    self.places[b] = std::array::from_fn(|axis| self.places[b][axis] + share[axis] * times);
                }
                if pull.abs() > ERROR_MAX {
                    *self = Self::at_rest(cloth, pins, self.center);
                    self.pin(cloth, pins);
                    done -= 1;
                }
            }
            self.pin(cloth, pins);
            done += 1;
            passes += 1;
        }
    }

    /// `cloth_collide`: every point, pins too, out to each sphere's skin.
    fn collide(&mut self, spheres: &[([f32; 3], f32)]) {
        for place in &mut self.places {
            for &(center, radius) in spheres {
                let out = minus(*place, center);
                let squared = dot(out, out);
                if squared <= NEARLY_ZERO {
                    *place = [center[0], center[1] + radius, center[2]];
                } else if radius * radius > squared {
                    let scale = radius / squared.sqrt();
                    *place = std::array::from_fn(|axis| out[axis] * scale + center[axis]);
                }
            }
        }
    }

    /// `cloth_update_positions_verlet`.
    fn integrate(&mut self, cloth: &H3ClothAt, wind: [f32; 3]) {
        let wind = wind.map(|axis| axis * cloth.wind_scale);
        for vertex in 0..self.places.len() {
            if cloth.pins.iter().any(|pin| pin.vertex == vertex) {
                continue;
            }
            let (place, last, normal) = (self.places[vertex], self.last[vertex], self.normals[vertex]);
            let into = dot(wind, normal);
            let along: [f32; 3] = std::array::from_fn(|axis| wind[axis] - normal[axis] * into);
            let blown: [f32; 3] = std::array::from_fn(|axis| wind[axis] - along[axis] * cloth.flappiness);
            let next: [f32; 3] = std::array::from_fn(|axis| {
                let gravity = if axis == 2 { cloth.gravity } else { 0.0 };
                place[axis] * (2.0 - cloth.drag) - last[axis] * (1.0 - cloth.drag) + gravity + blown[axis]
            });
            self.last[vertex] = place;
            self.places[vertex] = next;
        }
    }

    /// `cloth_calculate_normals`.
    fn calculate_normals(&mut self, cloth: &H3ClothAt) {
        // Halo 2 adds to what the normal was, so each carries a little of the last tick.
        let carry = cloth.game.rules().normals_carry_over;
        let mut sums = if carry { self.normals.clone() } else { vec![[0.0; 3]; self.places.len()] };
        for triangle in cloth.indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
            let normal = unit_or_unchanged(cross(minus(self.places[b], self.places[a]), minus(self.places[c], self.places[a])));
            for corner in [a, b, c] {
                sums[corner] = std::array::from_fn(|axis| sums[corner][axis] + normal[axis]);
            }
        }
        for (normal, sum) in self.normals.iter_mut().zip(sums) {
            *normal = if sum != [0.0; 3] {
                unit_or_unchanged(sum)
            } else if carry {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
        }
    }
}

/// What a slot keeps of its cloth from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3ClothField {
    wind: H3Wind,
    h2_wind: H2Wind,
    reach_wind: ReachWind,
    time: Option<f32>,
    ticks: f32,
    /// The frames drawn, each a call of `advance`: Reach's wind moves once a
    /// frame.
    frames: u64,
    running: Vec<RunningCloth>,
}

/// A cloth vertex as a step leaves it: its place, its normal, and, where the
/// game derives one from the normal, its binormal.
pub(crate) type MovedClothVertex = ([f32; 3], [f32; 3], Option<[f32; 3]>);

impl H3ClothField {
    /// A new sheet. Reach's `cloth_new_attachment` (the 360's 0x82C8E2F0)
    /// resets it, enforces its attachments, which carry it to the object's
    /// bounding sphere, and relaxes it five times.
    fn created(cloth: &H3ClothAt, pins: &[[f32; 3]], center: [f32; 3]) -> RunningCloth {
        let mut running = RunningCloth::at_rest(cloth, pins, center);
        if cloth.game.rules().settles_new_sheet {
            running.pin(cloth, pins);
            for _ in 0..REACH_CREATION_RELAXATIONS {
                running.relax(cloth, pins);
            }
        }
        running
    }

    /// Run the ticks up to `time`, and hand back what each cloth's vertices
    /// have become: where they start, and each side's in turn.
    pub(crate) fn advance(
        &mut self,
        cloths: &[H3ClothAt],
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) -> Vec<(usize, Vec<MovedClothVertex>)> {
        let pins_of = |cloth: &H3ClothAt| -> Vec<[f32; 3]> {
            let posed = |pin: &ClothPin| posed_on(pin.node, pin.position, [1.0, 0.0, 0.0], bones).0;
            cloth.pins.iter().map(posed).collect()
        };
        let center_of = |cloth: &H3ClothAt| posed_on(cloth.center.0, cloth.center.1, [1.0, 0.0, 0.0], bones).0;
        self.frames += 1;
        if self.running.len() != cloths.len() || self.time.is_none_or(|last| time < last) {
            self.running = cloths.iter().map(|cloth| Self::created(cloth, &pins_of(cloth), center_of(cloth))).collect();
            // It settles before it is seen.
            self.time = Some(time - SETTLE);
        }
        let mut now = resume_point(self.time.unwrap_or(time), time, SETTLE);
        let game = cloths.first().map_or(ClothGame::Halo3, |cloth| cloth.game);
        let rules = game.rules();
        let rate = rules.tick_rate;
        let tick = 1.0 / rate;
        while now + tick <= time {
            now += tick;
            self.ticks += 1.0;
            let shared = match rules.wind {
                ClothWind::Halo2 => Some(self.h2_wind.step(self.ticks)),
                ClothWind::Halo3 => Some(self.wind.step(self.ticks * 30.0 * tick)),
                ClothWind::PerCloth => None,
            };
            for (index, (cloth, running)) in cloths.iter().zip(&mut self.running).enumerate() {
                // A datum's index is taken to be its place in the slot's list. [inferred]
                let clock = self.ticks * rate + (index & 0xFF) as f32 * 0.75;
                let wind = match shared {
                    Some(wind) => wind,
                    None => self.reach_wind.step(self.frames, clock),
                };
                let pins = pins_of(cloth);
                let spheres: Vec<([f32; 3], f32)> = cloth
                    .spheres
                    .iter()
                    .map(|sphere| (posed_on(sphere.node, sphere.position, [1.0, 0.0, 0.0], bones).0, sphere.radius))
                    .collect();
                running.center = center_of(cloth);
                if cloth.game.rules().relaxes_as_halo2 {
                    running.relax_h2(cloth, &pins);
                } else {
                    running.relax(cloth, &pins);
                }
                running.collide(&spheres);
                running.integrate(cloth, wind);
                running.calculate_normals(cloth);
            }
        }
        self.time = Some(now);
        cloths
            .iter()
            .zip(&self.running)
            .map(|(cloth, running)| {
                // In Halo 3 and Reach both sides keep the one normal: only
                // Halo 3's binormal turns over, as loaded.
                let back = cloth.game.rules().back_normal_sign;
                let side = |sign: f32| {
                    let places = running.places.iter().zip(&running.normals);
                    places.map(move |(place, normal)| {
                        let normal = normal.map(|axis| axis * sign);
                        let binormal = cloth.game.rules().derives_binormal.then(|| reach_binormal(normal));
                        (*place, normal, binormal)
                    })
                };
                (cloth.first_vertex, side(1.0).chain(side(back)).collect())
            })
            .collect()
    }
}

/// How long a new cloth runs before it shows, in seconds.
const SETTLE: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_hangs_down_from_its_first_row() {
        let (places, uvs, indices) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3);
        assert_eq!(places, [[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -0.25], [0.5, 0.0, -0.25], [1.0, 0.0, -0.25]]);
        assert_eq!(uvs[0], [0.0, 0.5], "a square of texture spans the longer side");
        assert_eq!(uvs[5], [1.0, 0.0]);
        assert_eq!(indices[..6], [0, 3, 1, 1, 3, 4]);
        assert_eq!(indices.len(), 12);
    }

    #[test]
    fn a_grid_past_the_limit_gives_way_on_its_longer_side() {
        let (places, ..) = flag_grid([20, 20], [1.0, 1.0], ClothGame::Halo3);
        assert_eq!(places.len(), 120);
        assert!(flag_grid([1, 5], [1.0, 1.0], ClothGame::Halo3).0.is_empty());
    }

    #[test]
    fn every_edge_is_a_link_once_and_each_keeps_its_length() {
        let (places, _, indices) = flag_grid([3, 3], [1.0, 1.0], ClothGame::Halo3);
        let links = generate_links(&places, &indices, 0.5);
        let pairs: std::collections::BTreeSet<_> = links.iter().map(|link| (link.ends[0].min(link.ends[1]), link.ends[0].max(link.ends[1]))).collect();
        assert_eq!(pairs.len(), links.len(), "no link twice");
        assert!(find_link(&links, 0, 3).is_some() && find_link(&links, 3, 1).is_some(), "a side and a diagonal");
        for link in &links {
            assert!((link.length - distance(places[link.ends[0]], places[link.ends[1]])).abs() < 1e-6);
        }
    }

    #[test]
    fn a_longer_rod_allows_links_across_more_of_the_sheet() {
        let (places, _, indices) = flag_grid([4, 4], [1.0, 1.0], ClothGame::Halo3);
        let short = generate_links(&places, &indices, 0.5).len();
        let long = generate_links(&places, &indices, 2.0).len();
        assert!(long > short, "{long} against {short}");
    }

    #[test]
    fn pins_take_the_markers_in_order_from_corner_to_corner() {
        let markers = vec![(1, [0.0, 0.0, 2.0]), (2, [0.0, 0.0, 0.0]), (3, [0.0, 0.0, 1.0])];
        let pins = cloth_pins(markers, Some((4, [9.0; 3])), [8, 8], 64, false, ClothGame::Halo3);
        assert_eq!(pins.iter().map(|pin| pin.vertex).collect::<Vec<_>>(), [0, 24, 56, 63]);
        assert_eq!(pins[0].node, 2, "the lowest first");
        assert_eq!(pins[3].node, 4, "the secondary takes the last vertex");
    }

    fn hung() -> (H3ClothAt, RunningCloth, Vec<[f32; 3]>) {
        let (places, _, indices) = flag_grid([4, 4], [0.1, 0.1], ClothGame::Halo3);
        let links = generate_links(&places, &indices, 2.0);
        let pins = cloth_pins(vec![(0, [0.0, 0.0, 1.0]), (0, [0.0, 0.0, 0.7])], None, [4, 4], 16, false, ClothGame::Halo3);
        let cloth = H3ClothAt {
            places,
            indices,
            links,
            pins,
            spheres: Vec::new(),
            iterations: 5,
            gravity: 0.5 * gravity_per_tick(tick_rate(ClothGame::Halo3)),
            drag: 0.01,
            wind_scale: 0.15,
            flappiness: 0.25,
            first_vertex: 0,
            game: ClothGame::Halo3,
            center: (0, [0.0; 3]),
        };
        let targets: Vec<[f32; 3]> = cloth.pins.iter().map(|pin| pin.position).collect();
        let running = RunningCloth::at_rest(&cloth, &targets, [0.0; 3]);
        (cloth, running, targets)
    }

    /// `cloth_update` runs a cloth at most once a tick, so a faster frame
    /// rate adds no steps, and Halo 3 ticks twice as often as Halo 2 and
    /// Reach.
    #[test]
    fn cloth_steps_by_its_games_ticks_whatever_the_frame_rate() {
        let ticks_over_a_second = |game: ClothGame, frames: usize| {
            let (cloth, _, _) = hung();
            let cloths = [H3ClothAt { game, ..cloth }];
            let mut field = H3ClothField::default();
            field.advance(&cloths, None, 10.0);
            let settled = field.ticks;
            (1..=frames).for_each(|frame| _ = field.advance(&cloths, None, 10.0 + frame as f32 / frames as f32));
            field.ticks - settled
        };
        assert_eq!(ticks_over_a_second(ClothGame::Halo3, 60), 60.0);
        assert_eq!(ticks_over_a_second(ClothGame::Halo3, 240), 60.0, "frames between ticks add none");
        assert_eq!(ticks_over_a_second(ClothGame::Halo2, 60), 30.0, "Halo 2 ticks at 30");
        assert_eq!(ticks_over_a_second(ClothGame::Reach, 60), REACH_TICK_RATE, "Reach at its own rate");
    }

    #[test]
    fn a_sheet_that_comes_apart_has_its_pass_over_up_to_the_sixteenth() {
        let (mut cloth, mut running, pins) = hung();
        assert_eq!(running.relax(&cloth, &pins), 5, "a sound sheet takes what the tag asks");
        cloth.iterations = 40;
        assert_eq!(running.relax(&cloth, &pins), 15, "and no more than the limit");
        cloth.iterations = 5;
        let held = cloth.pins[0].vertex;
        let link = cloth.links.iter_mut().find(|link| link.ends.contains(&held)).unwrap();
        link.length += 100.0;
        assert_eq!(running.relax(&cloth, &pins), 16, "one that never settles stops all the same");
    }

    #[test]
    fn spheres_push_points_to_their_skin_and_leave_the_rest() {
        let center = [1.0, 2.0, 3.0];
        let mut running = RunningCloth {
            places: vec![[1.0, 2.0, 3.5], [1.0, 2.0, 5.5], [1.0, 2.0, 3.005]],
            ..RunningCloth::default()
        };
        running.collide(&[(center, 1.0)]);
        assert_eq!(running.places[0], [1.0, 2.0, 4.0], "inside goes out along its own line");
        assert_eq!(running.places[1], [1.0, 2.0, 5.5], "outside stays");
        assert_eq!(running.places[2], [1.0, 3.0, 3.0], "at the centre it goes to the left");
    }

    #[test]
    fn a_sheet_stays_on_its_pins_and_near_its_size() {
        let (cloth, mut running, pins) = hung();
        let mut wind = H3Wind::default();
        for tick in 0..300 {
            let blown = wind.step(tick as f32);
            running.relax(&cloth, &pins);
            running.integrate(&cloth, blown);
            running.calculate_normals(&cloth);
        }
        assert_eq!(running.places[cloth.pins[0].vertex], pins[0]);
        for link in &cloth.links {
            let length = distance(running.places[link.ends[0]], running.places[link.ends[1]]);
            assert!(length.is_finite() && length < link.length * 1.5, "{length} against {}", link.length);
        }
    }

    #[test]
    fn gravity_pulls_what_is_not_pinned_and_wind_stirs_it() {
        let settled = |gravity: f32| {
            let (mut cloth, mut running, pins) = hung();
            cloth.gravity = gravity;
            for _ in 0..120 {
                running.relax(&cloth, &pins);
                running.integrate(&cloth, [0.0; 3]);
                running.calculate_normals(&cloth);
            }
            running.places[15]
        };
        let (weightless, weighted) = (settled(0.0), settled(gravity_per_tick(tick_rate(ClothGame::Halo3))));
        assert!(weighted[2] < weightless[2] - 1e-4, "{weighted:?} against {weightless:?}");
        assert!(weighted[1].abs() < 1e-3, "nothing pushes it out of its plane");
        let (cloth, mut blown, pins) = hung();
        let mut wind = H3Wind::default();
        for tick in 0..120 {
            let push = wind.step(tick as f32);
            blown.relax(&cloth, &pins);
            blown.integrate(&cloth, push);
            blown.calculate_normals(&cloth);
        }
        assert!(blown.places[15][1].abs() > 1e-4, "{:?}", blown.places[15]);
    }

    #[test]
    fn halo_2_allows_the_larger_sheet() {
        assert_eq!(flag_grid([16, 8], [1.0, 1.0], ClothGame::Halo2).0.len(), 128);
        assert_eq!(flag_grid([16, 8], [1.0, 1.0], ClothGame::Halo3).0.len(), 120);
    }

    #[test]
    fn the_rough_root_is_near_the_true_one() {
        for value in [0.01_f32, 0.25, 1.0, 4.0, 100.0] {
            assert!((rough_sqrt(value) / value.sqrt() - 1.0).abs() < 0.07, "{value}");
        }
        assert_ne!(rough_sqrt(2.0), 2.0_f32.sqrt(), "and is not the true one");
    }

    #[test]
    fn halo_2_links_pull_a_stretched_sheet_back_with_the_pin_held() {
        let (mut cloth, _, pins) = hung();
        cloth.game = ClothGame::Halo2;
        cloth.pins.truncate(1);
        let pins = &pins[..1];
        let mut running = RunningCloth::at_rest(&cloth, pins, [0.0; 3]);
        running.places[15][1] += 0.3;
        for _ in 0..200 {
            running.relax_h2(&cloth, pins);
        }
        assert_eq!(running.places[cloth.pins[0].vertex], pins[0]);
        for link in &cloth.links {
            let length = distance(running.places[link.ends[0]], running.places[link.ends[1]]);
            assert!((length - link.length).abs() < 0.05, "{length} against {}", link.length);
        }
    }

    #[test]
    fn a_halo_2_link_at_length_does_not_pull() {
        let (mut cloth, mut running, pins) = hung();
        cloth.game = ClothGame::Halo2;
        cloth.pins.clear();
        let before = running.places.clone();
        running.relax_h2(&cloth, &pins);
        for (place, was) in running.places.iter().zip(&before) {
            assert!(distance(*place, *was) < 0.02, "the rough root leaves only a little");
        }
    }

    #[test]
    fn halo_2_normals_keep_some_of_the_last_tick() {
        let (mut cloth, mut running, _) = hung();
        let fresh = |running: &RunningCloth| running.normals[5];
        running.normals = vec![[1.0, 0.0, 0.0]; running.places.len()];
        running.calculate_normals(&cloth);
        let halo3 = fresh(&running);
        cloth.game = ClothGame::Halo2;
        running.normals = vec![[1.0, 0.0, 0.0]; running.places.len()];
        running.calculate_normals(&cloth);
        assert_ne!(fresh(&running), halo3);
    }

    #[test]
    fn the_halo_2_wind_stays_a_unit_away_from_straight_up() {
        let limit = H2_WIND_POLE_ANGLE.cos();
        let held = held_from_straight_up([0.0, 0.0, 1.0], H2_WIND_POLE_ANGLE);
        assert!((held[2] - limit).abs() < 1e-6);
        assert!((dot(held, held) - 1.0).abs() < 1e-5);
        assert!(held[0] > 0.0, "with nothing level to keep, it goes to the front");
        let clear = unit_or_unchanged([1.0, 0.0, 0.2]);
        let kept = held_from_straight_up(clear, H2_WIND_POLE_ANGLE);
        assert!(distance(kept, clear) < 1e-5, "{kept:?}");
    }

    #[test]
    fn the_halo_2_wind_stays_within_its_strength() {
        let mut wind = H2Wind::default();
        for tick in 0..5000 {
            let push = wind.step(tick as f32);
            assert!(dot(push, push).sqrt() <= (0.1 + 0.0075) * 0.4 * 0.2 + 1e-5);
        }
    }

    #[test]
    fn the_wind_stays_out_of_the_straight_up_cone() {
        let up = [0.0, 0.0, 1.0];
        let moved = away_from_pole(up, 0.5, unit_or_unchanged([0.01, 0.0, 1.0]));
        assert!(dot(moved, up).acos() >= 0.499, "{moved:?}");
        let clear = unit_or_unchanged([1.0, 0.0, 0.2]);
        assert_eq!(away_from_pole(up, 0.5, clear), clear);
    }

    #[test]
    fn the_wind_never_blows_harder_than_its_limit() {
        let mut wind = H3Wind::default();
        for tick in 0..5000 {
            let push = wind.step(tick as f32);
            assert!(dot(push, push).sqrt() <= (0.1 + 0.0075) * 0.4 * 0.2 + 1e-5);
        }
    }

    /// Reach's `create_cloth_grid_data` spans the texture across each side by
    /// its own step; Halo 3's one square spans the longer side.
    #[test]
    fn a_reach_grid_spans_its_texture_across_each_side() {
        let (places, uvs, indices) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Reach);
        assert_eq!(places, flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3).0, "the same sheet");
        assert_eq!(indices, flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3).2);
        assert_eq!(uvs, [[0.0, 1.0], [0.5, 1.0], [1.0, 1.0], [0.0, 0.0], [0.5, 0.0], [1.0, 0.0]]);
        let (_, halo3, _) = flag_grid([3, 2], [0.5, 0.25], ClothGame::Halo3);
        assert_eq!(halo3[0], [0.0, 0.5], "Halo 3 keeps its square");
        let (_, flag, _) = flag_grid([10, 10], [0.03, 0.04], ClothGame::Reach);
        assert_eq!((flag[0], flag[99]), ([0.0, 1.0], [1.0, 0.0]), "the kit's flag, corner to corner");
    }

    /// Reach lists a sheet as its tag's whole name, apart from any region a
    /// model's variant names; the others as the tag's name.
    #[test]
    fn a_reach_sheet_is_listed_by_its_tags_whole_name() {
        let path = "objects\\weapons\\multiplayer\\flag\\flag";
        assert_eq!(cloth_region_name(path, ClothGame::Reach), "flag.cloth");
        assert_eq!(cloth_region_name(path, ClothGame::Halo3), "flag");
        assert_eq!(cloth_region_name(path, ClothGame::Halo2), "flag");
    }

    /// Reach's flag has ten `flag_attach` markers up its pole, every one a pin
    /// down the first column from the lowest; Halo 3 keeps six, and only it
    /// pins a secondary.
    #[test]
    fn reach_pins_all_ten_markers_and_no_secondary() {
        let markers: Vec<_> = (0..10).rev().map(|at| (at, [0.0, 0.0, 0.26 + 0.04 * at as f32])).collect();
        let secondary = Some((11, [9.0; 3]));
        let reach = cloth_pins(markers.clone(), secondary, [10, 10], 100, false, ClothGame::Reach);
        assert_eq!(reach.iter().map(|pin| pin.vertex).collect::<Vec<_>>(), [0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
        assert_eq!(reach[0].node, 0, "row 0 on the lowest marker");
        let halo3 = cloth_pins(markers, secondary, [10, 10], 100, false, ClothGame::Halo3);
        assert_eq!(halo3.len(), 6);
        assert_eq!(halo3.last().map(|pin| pin.vertex), Some(90), "the sixth ends the column");
        let fewer: Vec<_> = (0..3).map(|at| (at, [0.0, 0.0, at as f32])).collect();
        assert_eq!(cloth_pins(fewer.clone(), secondary, [10, 10], 100, false, ClothGame::Reach).len(), 3);
        let halo3 = cloth_pins(fewer, secondary, [10, 10], 100, false, ClothGame::Halo3);
        assert_eq!(halo3.last().map(|pin| (pin.vertex, pin.node)), Some((99, 11)), "Halo 3's secondary");
    }

    /// Reach relaxes at most 9 times, and its tenth pass ends a sheet that
    /// keeps coming apart; Halo 3's limits are 15 and 16.
    #[test]
    fn a_reach_sheet_that_comes_apart_has_its_pass_over_up_to_the_tenth() {
        let (cloth, mut running, pins) = hung();
        let mut cloth = H3ClothAt { game: ClothGame::Reach, ..cloth };
        assert_eq!(running.relax(&cloth, &pins), 5, "a sound sheet takes what the tag asks");
        cloth.iterations = 40;
        assert_eq!(running.relax(&cloth, &pins), 9);
        cloth.iterations = 5;
        let held = cloth.pins[0].vertex;
        let link = cloth.links.iter_mut().find(|link| link.ends.contains(&held)).unwrap();
        link.length += 100.0;
        assert_eq!(running.relax(&cloth, &pins), 10);
    }

    /// Reach's reset leaves the sheet at the world's origin, and enforcing its
    /// attachments carries it to the object's bounding sphere once that is
    /// more than 0.4 away; Halo 3 moves it up to its first pin.
    #[test]
    fn a_reach_sheet_starts_at_its_objects_bounding_sphere() {
        let (cloth, _, pins) = hung();
        let reach = H3ClothAt { game: ClothGame::Reach, ..cloth.clone() };
        let start = |cloth: &H3ClothAt, center: [f32; 3]| {
            let mut running = RunningCloth::at_rest(cloth, &pins, center);
            running.pin(cloth, &pins);
            running
        };
        let far = start(&reach, [0.0, 0.0, 2.0]);
        assert_eq!(far.places[15], [cloth.places[15][0], 0.0, cloth.places[15][2] + 2.0]);
        assert_eq!(far.last[15], far.places[15], "its last places go with it");
        assert_eq!(start(&reach, [0.0, 0.0, 0.3]).places[15], cloth.places[15], "a sphere within 0.4 leaves it");
        let halo3 = start(&cloth, [0.0, 0.0, 2.0]);
        assert_eq!(halo3.places[15], [cloth.places[15][0], 0.0, cloth.places[15][2] + pins[0][2]], "at the first pin");
        // Once there, it is carried only by a jump.
        let mut moved = far.clone();
        moved.center = [0.0, 0.0, 2.3];
        moved.pin(&reach, &pins);
        assert_eq!(moved.places[15], far.places[15]);
    }

    /// Reach's wind is never stiller than 0.05 nor stronger than 0.2, more
    /// than Halo 3's ever blows.
    #[test]
    fn the_reach_wind_stays_between_its_least_and_most() {
        let mut wind = ReachWind::default();
        for frame in 1..5000 {
            let push = wind.step(frame, frame as f32 * REACH_TICK_RATE);
            let strength = dot(push, push).sqrt();
            assert!((REACH_WIND_LEAST - 1e-5..=REACH_WIND_MOST + 1e-5).contains(&strength), "{strength}");
            assert!(strength > (0.1 + 0.0075) * 0.4 * 0.2, "past Halo 3's");
        }
    }

    /// It moves once a drawn frame, however many ticks that frame runs.
    #[test]
    fn the_reach_wind_moves_only_on_a_new_frame() {
        let mut wind = ReachWind::default();
        let first = wind.step(1, 0.0);
        assert_eq!(wind.step(1, 30.0), first, "the same frame");
        assert_ne!(wind.step(2, 60.0), first);
    }

    /// Reach derives a vertex's binormal from its normal, `cross(n, x)`, each
    /// step, on both sides; Halo 3 keeps the one it was loaded with.
    #[test]
    fn a_reach_sheet_turns_its_binormal_with_its_normal() {
        let (cloth, _, _) = hung();
        let reach = [H3ClothAt { game: ClothGame::Reach, ..cloth.clone() }];
        let written = H3ClothField::default().advance(&reach, None, 10.0);
        let (_, vertices) = &written[0];
        assert_eq!(vertices.len(), cloth.places.len() * 2);
        for (_, normal, binormal) in vertices {
            assert_eq!(*binormal, Some(cross(*normal, [1.0, 0.0, 0.0])));
        }
        assert_eq!(sheet_vertices(&[[0.0; 2]], ClothGame::Reach)[1].binormal, [0.0, 0.0, -1.0], "from the rest normal");
        let written = H3ClothField::default().advance(&[cloth], None, 10.0);
        assert!(written[0].1.iter().all(|(_, _, binormal)| binormal.is_none()));
    }
}
