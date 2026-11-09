//! A running contrail and its profiles.

use super::*;

/// `c_contrail_profile`, and what its update keeps.
#[derive(Debug, Clone, Copy)]
pub(super) struct ContrailProfile {
    pub(super) place: [f32; 3],
    velocity: [f32; 3],
    pub(super) age: f32,
    lifespan: f32,
    pub(super) length: f32,
    pub(super) seeds: [f32; PROFILE_SEEDS],
    pub(super) alpha: f32,
    pub(super) temporary: bool,
}

/// `c_contrail`.
#[derive(Debug, Clone, Default)]
pub(super) struct RunningContrail {
    /// Oldest first.
    pub(super) profiles: Vec<ContrailProfile>,
    pub(super) accumulator: f32,
    pub(super) previous: [f32; 3],
    pub(super) length: f32,
    pub(super) initialized: bool,
    pub(super) location: Option<[f32; 3]>,
    pub(super) speed: f32,
    pub(super) random: u32,
}

impl RunningContrail {
    fn next_seed(&mut self) -> f32 {
        self.random = lcg_step(self.random);
        high24_over_2_24(self.random)
    }

    /// `create_profile`.
    fn create(
        &mut self,
        contrail: &ContrailDefinition,
        state: &StripState,
        temporary: bool,
        direction: [f32; 3],
        distance: f32,
    ) {
        if self
            .profiles
            .last()
            .is_some_and(|profile| profile.temporary)
        {
            self.profiles.pop();
        }
        if self.profiles.len() >= MOST_PROFILES {
            return;
        }
        let seeds: [f32; PROFILE_SEEDS] = std::array::from_fn(|_| self.next_seed());
        let place = std::array::from_fn(|axis| self.previous[axis] + direction[axis] * distance);
        let length = if self.initialized {
            distance + self.length
        } else {
            0.0
        };
        let lifespan = contrail.lifespan.scalar(&StripState {
            profile_seeds: seeds,
            ..*state
        });
        self.profiles.push(ContrailProfile {
            place,
            velocity: [0.0; 3],
            age: 0.0,
            lifespan,
            length,
            seeds,
            alpha: if contrail.opacity_from_a {
                state.effect_a
            } else {
                1.0
            },
            temporary,
        });
        if !temporary {
            self.initialized = true;
            self.previous = place;
            self.length = length;
        }
    }

    /// `extend`: origin fades need profiles at their ends.
    fn extend(
        &mut self,
        contrail: &ContrailDefinition,
        look: &StripLook,
        state: &StripState,
        temporary: bool,
        direction: [f32; 3],
        by: f32,
    ) {
        let mut current = self.length;
        let last = self.length + by;
        if let (true, Some([inverse, cutoff])) = (self.initialized, look.origin_fade) {
            for boundary in [cutoff, cutoff + inverse.recip()] {
                if boundary > current + EPSILON && boundary < last - EPSILON {
                    self.create(contrail, state, false, direction, boundary - current);
                    current = boundary;
                }
            }
        }
        self.create(contrail, state, temporary, direction, last - current);
    }

    /// `c_contrail::pulse`.
    pub(super) fn pulse(
        &mut self,
        contrail: &ContrailDefinition,
        look: &StripLook,
        state: &StripState,
        at: [f32; 3],
        dt: f32,
    ) {
        let moved = minus(at, self.previous);
        let total = dot(moved, moved).sqrt();
        // `normalize3d`: what is too short to have a direction has no length.
        let (total, direction) = if total < EPSILON {
            (0.0, [0.0; 3])
        } else {
            (total, moved.map(|axis| axis / total))
        };
        // `event_initialize_contrail_system` (Halo 3 X360 0x829F9BA0, Reach's
        // by name) pulses a new system once with no time and its flag set,
        // which `c_contrail::pulse` turns into a permanent profile where it
        // starts; `pulse_all` then pulses with the flag clear.
        if !self.initialized {
            self.extend(contrail, look, state, false, direction, total);
            return;
        }
        if total <= EPSILON {
            return;
        }
        let mut remaining = total;
        let rate = contrail.emission_rate.scalar(state);
        let rate = if look.reach {
            rate.min(REACH_MOST_EMISSION_RATE)
        } else {
            rate
        };
        self.accumulator += rate * dt;
        if self.accumulator >= 1.0 {
            let each = total / self.accumulator;
            while self.accumulator >= 1.0 {
                self.extend(contrail, look, state, false, direction, each);
                remaining -= each;
                self.accumulator -= 1.0;
            }
        }
        if remaining > EPSILON {
            self.extend(contrail, look, state, true, direction, remaining);
        }
    }

    /// `contrail_main`.
    pub(super) fn update(&mut self, contrail: &ContrailDefinition, state: &StripState, dt: f32) {
        self.profiles.retain_mut(|profile| {
            let state = StripState {
                profile_age: profile.age,
                profile_seeds: profile.seeds,
                ..*state
            };
            profile.age += dt / profile.lifespan.max(1e-4);
            if profile.age >= 1.0 {
                return false;
            }
            let acceleration = contrail.acceleration.vector(&state);
            for axis in 0..3 {
                profile.place[axis] += profile.velocity[axis] * dt;
                profile.velocity[axis] += acceleration[axis] * dt;
            }
            true
        });
    }
}
