//! Running cloth and its field, stepped a tick at a time.

use super::super::resume_point;
use super::links::{ClothPin, rough_sqrt};
use super::read::{ClothAt, reach_binormal};
use super::rules::{
    ClothGame, ClothWind, ERROR_MAX, MOST_HELD_LINKS, MOST_ITERATIONS, NEARLY_ZERO,
    REACH_CENTER_JUMP, REACH_CREATION_RELAXATIONS,
};
use super::wind::{H2_MOST_PASSES, H2Wind, H3Wind, ReachWind};
use super::{cross, dot, minus, posed_on, unit_or_unchanged};

/// A cloth's points as they stand.
#[derive(Debug, Clone, Default)]
pub(super) struct RunningCloth {
    pub(super) places: Vec<[f32; 3]>,
    pub(super) last: Vec<[f32; 3]>,
    pub(super) normals: Vec<[f32; 3]>,
    /// Reach's: where the object's bounding sphere is now, and where
    /// `cloth_enforce_attachments` last saw it.
    pub(super) center: [f32; 3],
    pub(super) last_center: [f32; 3],
}

impl RunningCloth {
    /// `cloth_reset_flag_to_default_positions`. Halo 2 and 3 move the sheet up
    /// to its first pin; Reach's (the 360's 0x82C90BF8) leaves it at the
    /// world's origin, for `pin` to carry to the object.
    pub(super) fn at_rest(cloth: &ClothAt, pins: &[[f32; 3]], center: [f32; 3]) -> Self {
        let shift = if cloth.game.rules().rests_at_first_pin {
            pins.first().copied().unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        let places: Vec<[f32; 3]> = cloth
            .places
            .iter()
            .map(|place| std::array::from_fn(|axis| place[axis] + shift[axis]))
            .collect();
        Self {
            last: places.clone(),
            normals: vec![[0.0, 1.0, 0.0]; places.len()],
            places,
            center,
            last_center: [0.0; 3],
        }
    }

    /// `cloth_enforce_attachments`. Reach's first carries the whole sheet
    /// with the object's bounding sphere, where that has jumped.
    pub(super) fn pin(&mut self, cloth: &ClothAt, pins: &[[f32; 3]]) {
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
    pub(super) fn relax(&mut self, cloth: &ClothAt, pins: &[[f32; 3]]) -> usize {
        let rules = cloth.game.rules();
        let (most_iterations, most_passes) = (rules.most_iterations, rules.most_passes);
        let count = cloth.iterations.clamp(1, most_iterations);
        let held = |vertex: usize| cloth.pins.iter().any(|pin| pin.vertex == vertex);
        let mut passes = 0;
        let mut done = 0;
        loop {
            self.pin(cloth, pins);
            let mut worst = 0.0_f32;
            for link in cloth
                .links
                .iter()
                .filter(|link| held(link.ends[0]) || held(link.ends[1]))
                .take(MOST_HELD_LINKS)
            {
                let [a, b] = link.ends;
                let rod = minus(self.places[b], self.places[a]);
                let length = dot(rod, rod).sqrt();
                let delta = length - link.length;
                let share = rod.map(|axis| axis * delta * 0.5 / length.max(NEARLY_ZERO));
                if !held(a) {
                    let times = if held(b) { 2.0 } else { 1.0 };
                    self.places[a] =
                        std::array::from_fn(|axis| self.places[a][axis] + share[axis] * times);
                }
                if !held(b) {
                    let times = if held(a) { 2.0 } else { 1.0 };
                    self.places[b] =
                        std::array::from_fn(|axis| self.places[b][axis] - share[axis] * times);
                }
                worst = worst.max(delta.abs());
            }
            for link in &cloth.links {
                let [a, b] = link.ends;
                let rod = minus(self.places[b], self.places[a]);
                let length = dot(rod, rod).sqrt();
                let share =
                    rod.map(|axis| axis * (length - link.length) * 0.5 / length.max(NEARLY_ZERO));
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
    pub(super) fn relax_h2(&mut self, cloth: &ClothAt, pins: &[[f32; 3]]) {
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
                    self.places[a] =
                        std::array::from_fn(|axis| self.places[a][axis] - share[axis] * times);
                }
                if !held(b) {
                    let times = if held(a) { 2.0 } else { 1.0 };
                    self.places[b] =
                        std::array::from_fn(|axis| self.places[b][axis] + share[axis] * times);
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
    pub(super) fn collide(&mut self, spheres: &[([f32; 3], f32)]) {
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
    pub(super) fn integrate(&mut self, cloth: &ClothAt, wind: [f32; 3]) {
        let wind = wind.map(|axis| axis * cloth.wind_scale);
        for vertex in 0..self.places.len() {
            if cloth.pins.iter().any(|pin| pin.vertex == vertex) {
                continue;
            }
            let (place, last, normal) =
                (self.places[vertex], self.last[vertex], self.normals[vertex]);
            let into = dot(wind, normal);
            let along: [f32; 3] = std::array::from_fn(|axis| wind[axis] - normal[axis] * into);
            let blown: [f32; 3] =
                std::array::from_fn(|axis| wind[axis] - along[axis] * cloth.flappiness);
            let next: [f32; 3] = std::array::from_fn(|axis| {
                let gravity = if axis == 2 { cloth.gravity } else { 0.0 };
                place[axis] * (2.0 - cloth.drag) - last[axis] * (1.0 - cloth.drag)
                    + gravity
                    + blown[axis]
            });
            self.last[vertex] = place;
            self.places[vertex] = next;
        }
    }

    /// `cloth_calculate_normals`.
    pub(super) fn calculate_normals(&mut self, cloth: &ClothAt) {
        // Halo 2 adds to what the normal was, so each carries a little of the last tick.
        let carry = cloth.game.rules().normals_carry_over;
        let mut sums = if carry {
            self.normals.clone()
        } else {
            vec![[0.0; 3]; self.places.len()]
        };
        for triangle in cloth.indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
            let normal = unit_or_unchanged(cross(
                minus(self.places[b], self.places[a]),
                minus(self.places[c], self.places[a]),
            ));
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
pub(crate) struct ClothField {
    wind: H3Wind,
    h2_wind: H2Wind,
    reach_wind: ReachWind,
    time: Option<f32>,
    pub(super) ticks: f32,
    /// The frames drawn, each a call of `advance`: Reach's wind moves once a
    /// frame.
    frames: u64,
    running: Vec<RunningCloth>,
}

/// A cloth vertex as a step leaves it: its place, its normal, and, where the
/// game derives one from the normal, its binormal.
pub(crate) type MovedClothVertex = ([f32; 3], [f32; 3], Option<[f32; 3]>);

impl ClothField {
    /// A new sheet. Reach's `cloth_new_attachment` (the 360's 0x82C8E2F0)
    /// resets it, enforces its attachments, which carry it to the object's
    /// bounding sphere, and relaxes it five times.
    fn created(cloth: &ClothAt, pins: &[[f32; 3]], center: [f32; 3]) -> RunningCloth {
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
        cloths: &[ClothAt],
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) -> Vec<(usize, Vec<MovedClothVertex>)> {
        let pins_of = |cloth: &ClothAt| -> Vec<[f32; 3]> {
            let posed = |pin: &ClothPin| posed_on(pin.node, pin.position, [1.0, 0.0, 0.0], bones).0;
            cloth.pins.iter().map(posed).collect()
        };
        let center_of =
            |cloth: &ClothAt| posed_on(cloth.center.0, cloth.center.1, [1.0, 0.0, 0.0], bones).0;
        self.frames += 1;
        if self.running.len() != cloths.len() || self.time.is_none_or(|last| time < last) {
            self.running = cloths
                .iter()
                .map(|cloth| Self::created(cloth, &pins_of(cloth), center_of(cloth)))
                .collect();
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
                    .map(|sphere| {
                        (
                            posed_on(sphere.node, sphere.position, [1.0, 0.0, 0.0], bones).0,
                            sphere.radius,
                        )
                    })
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
                        let binormal = cloth
                            .game
                            .rules()
                            .derives_binormal
                            .then(|| reach_binormal(normal));
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
