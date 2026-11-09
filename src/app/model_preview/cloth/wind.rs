//! The cloth winds of Halo 2, Halo 3 and Reach.

use super::*;

/// `cloth_update_wind` in Halo 2: its limit from straight up, and the ticks per
/// second that its clock counts in.
pub(super) const H2_WIND_POLE_ANGLE: f32 = 0.349_065_9;

const H2_WIND_CLOCK: f32 = 30.0;

/// Halo 2 has no limit on a sheet that keeps coming apart; this one is ours.
pub(super) const H2_MOST_PASSES: usize = 64;

/// `real_vector3d_exclude_from_pole_by_angle`'s and the wind's own.
const WIND_POLE_ANGLE: f32 = 20.0 * std::f32::consts::PI / 180.0;

pub(super) fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
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
        Self {
            direction: [0.0; 3],
            scale: 0.0,
            random: 1,
        }
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
        let drift = ((t * 0.005).sin() * 0.01 * slow
            + (t * 0.001).cos() * (t * 0.0006).cos() * 0.01 * slow)
            * 0.4;
        self.scale = (self.scale + drift).clamp(-0.1, 0.1);
        let strength = (self.scale.abs() + 0.0075) * 0.4 * 0.2;
        self.direction.map(|axis| axis * strength)
    }
}

/// Halo 2's `cloth_update_wind`, whose one wind wanders by other sums.
#[derive(Debug, Clone)]
pub(super) struct H2Wind {
    direction: [f32; 3],
    scale: f32,
    random: u32,
}

impl Default for H2Wind {
    fn default() -> Self {
        Self {
            direction: [0.0; 3],
            scale: 0.1,
            random: 1,
        }
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
    pub(super) fn step(&mut self, tick: f32) -> [f32; 3] {
        let t = tick * H2_WIND_CLOCK;
        let turn = (t * 0.005).cos();
        let angle = self.next() * turn * turn * 0.15;
        if self.direction == [0.0; 3] {
            self.direction = self.random_direction();
        }
        let about = self.random_direction();
        self.direction = held_from_straight_up(
            turned_about(self.direction, about, angle),
            H2_WIND_POLE_ANGLE,
        );
        let slow = (t * 0.0001).cos();
        let drift =
            (t * 0.001).cos() * (t * 0.0006).cos() * 0.01 * slow + (t * 0.005).sin() * 0.01 * slow;
        self.scale = (self.scale + drift * 0.3).clamp(-0.1, 0.1);
        let strength = (self.scale.abs() + 0.0075) * 0.4 * 0.2;
        self.direction.map(|axis| axis * strength)
    }
}

/// Reach's `cloth_update_wind` (the 360's 0x82C902A0, MCC's 0x140D98EF0):
/// one wind for every cloth, which moves on only when a new frame is drawn.
#[derive(Debug, Clone)]
pub(super) struct ReachWind {
    direction: [f32; 3],
    scale: f32,
    random: u32,
    /// `c_render_globals::get_frame_index()` when it last moved.
    last_frame: u64,
}

impl Default for ReachWind {
    fn default() -> Self {
        Self {
            direction: [0.0; 3],
            scale: 0.0,
            random: 1,
            last_frame: 0,
        }
    }
}

/// Reach's least and greatest wind, either way.
pub(super) const REACH_WIND_LEAST: f32 = 0.05;

pub(super) const REACH_WIND_MOST: f32 = 0.2;

impl ReachWind {
    /// `real_random`: the upper half of the seed over 65535.
    fn next(&mut self) -> f32 {
        next_random(&mut self.random) * (1.0 / 65_535.0)
    }

    /// What the wind accelerates by, before the cloth's own scale. It moves
    /// only on a `frame` past the last it moved on; `t` is its clock,
    /// `game_time · game_tick_rate + (datum index & 0xFF) · 0.75`, ticks and
    /// rate together as both binaries have it.
    pub(super) fn step(&mut self, frame: u64, t: f32) -> [f32; 3] {
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
            let drift = 0.01 * (t * 0.005).sin() * slow
                + 0.01 * (t * 0.0006).cos() * (t * 0.001).cos() * slow;
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
