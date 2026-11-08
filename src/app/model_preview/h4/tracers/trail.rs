//! Running tracers and their trails, stepped a tick at a time.

use super::*;

/// `TracerGpu::k_profilesPerRow`.
const PROFILES_PER_ROW: usize = 16;

/// `Tracer::k_maxProfilesSpawnedPerFrame`, `…Constrained`, and the count
/// past which the constrained one holds (`k_profileSpawnConstraintThreshold`).
const SPAWNED_PER_FRAME: usize = 10;

const SPAWNED_PER_FRAME_CONSTRAINED: usize = 2;

const SPAWN_CONSTRAINT_THRESHOLD: usize = 25;

/// `tracerAppearanceFlags`: `profile opacity from scale a`.
const OPACITY_FROM_SCALE_A: u32 = 1 << 2;

/// What `Tracer::Extend` and `PlaceProfilesToPoint` take for no distance.
const PLACE_EPSILON: f32 = 1e-4;

/// A row of the card's profiles: they are freed together once the longest
/// lifespan among them has run (`TracerGpu::FrameAdvance`).
#[derive(Debug, Clone, Default)]
pub(super) struct TrailRow {
    pub(super) slots: Vec<ProfileState>,
    timer: f32,
}

/// A trailing tracer's `Tracer` and `TracerGpu` between frames.
#[derive(Debug, Clone, Default)]
pub(super) struct RunningTracer {
    /// Oldest first, all but the newest full.
    pub(super) rows: Vec<TrailRow>,
    /// The last real profile's place, and whether there is one yet.
    pub(super) last_placed: [f32; 3],
    pub(super) placed_real: bool,
    /// The last real profile's length along the trail (`Tracer+60`).
    pub(super) length: f32,
    /// The newest slot holds the temporary leading profile.
    pub(super) leading_temporary: bool,
    pub(super) location: Option<[f32; 3]>,
    pub(super) speed: f32,
    pub(super) random: u32,
}

impl RunningTracer {
    fn next_seed(&mut self) -> f32 {
        self.random = lcg_step(self.random);
        high16_over_65535(self.random)
    }

    pub(super) fn count(&self) -> usize {
        self.rows.iter().map(|row| row.slots.len()).sum()
    }

    /// `QueueProfile` / `AllocateProfile` (X 0x82BA4A78): a temporary
    /// profile last time is written over; else the newest row takes it, or a
    /// new one does. No more than a strip holds.
    fn queue(&mut self, profile: ProfileState, temporary: bool) {
        let timer = (1.0 - profile.age) * profile.lifespan;
        if self.leading_temporary
            && let Some(row) = self.rows.last_mut()
            && let Some(slot) = row.slots.last_mut()
        {
            *slot = profile;
            row.timer = row.timer.max(timer);
            self.leading_temporary = temporary;
            return;
        }
        if self.count() >= MOST_PROFILES {
            return;
        }
        if self
            .rows
            .last()
            .is_none_or(|row| row.slots.len() >= PROFILES_PER_ROW)
        {
            self.rows.push(TrailRow::default());
        }
        let row = self.rows.last_mut().expect("a row");
        row.slots.push(profile);
        row.timer = row.timer.max(timer);
        self.leading_temporary = temporary;
    }

    /// `CreateProfile` and `InitializeProfile` (X 0x829C2A98, 0x82B9B070):
    /// `distance` on from the last real profile.
    fn create(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        temporary: bool,
        direction: [f32; 3],
        distance: f32,
    ) {
        // The temporary leading profile is reused, its seeds and all.
        let reused = self
            .leading_temporary
            .then(|| self.rows.last().and_then(|row| row.slots.last()))
            .flatten();
        let random = match reused {
            Some(profile) => profile.random,
            None => std::array::from_fn(|_| self.next_seed()),
        };
        let profile_state = TracerState {
            profile_seeds: random,
            ..*state
        };
        let position =
            std::array::from_fn(|axis| self.last_placed[axis] + direction[axis] * distance);
        let length = if self.placed_real {
            self.length + distance
        } else {
            0.0
        };
        let opacity_from_a = tracer.appearance & OPACITY_FROM_SCALE_A != 0;
        let profile = ProfileState {
            position,
            random,
            length,
            lifespan: tracer.lifespan.scalar(&profile_state),
            initial_alpha: if opacity_from_a { state.effect_a } else { 1.0 },
            // No lightmap: white at an exponent of 0.
            initial_color: [1.0, 1.0, 1.0, 0.0],
            ..Default::default()
        };
        if !temporary {
            self.placed_real = true;
            self.last_placed = position;
            self.length = length;
        }
        self.queue(profile, temporary);
    }

    /// `Tracer::Extend` (X 0x829C2840): an origin fade gets profiles at
    /// both its ends; with none, the first stretch gets a tiny one.
    fn extend(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        temporary: bool,
        direction: [f32; 3],
        by: f32,
    ) {
        let mut current = self.length;
        let last = current + by;
        if let Some([cutoff, range]) = tracer.origin_fade {
            for boundary in [cutoff, cutoff + range] {
                if current + PLACE_EPSILON < boundary && boundary < last - PLACE_EPSILON {
                    self.create(tracer, state, false, direction, boundary - current);
                    current = boundary;
                }
            }
        } else if current < PLACE_EPSILON && last > PLACE_EPSILON {
            self.create(tracer, state, false, direction, 2.0 * PLACE_EPSILON);
        }
        self.create(tracer, state, temporary, direction, last - current);
    }

    /// `PlaceProfilesToPoint` (X 0x82995628): profiles a density apart up to
    /// `at`, no more than the frame's allowance, and a temporary one at `at`
    /// itself while the effect runs.
    pub(super) fn place_to(
        &mut self,
        tracer: &TracerDefinition,
        state: &TracerState,
        at: [f32; 3],
        spawned: &mut usize,
    ) {
        let moved: [f32; 3] = std::array::from_fn(|axis| at[axis] - self.last_placed[axis]);
        let distance = length(moved);
        if self.placed_real && distance <= PLACE_EPSILON {
            return;
        }
        let direction = if distance > 0.0 {
            moved.map(|axis| axis / distance)
        } else {
            [0.0; 3]
        };
        let wanted = distance * tracer.density.scalar(state);
        let allowance = if *spawned < SPAWN_CONSTRAINT_THRESHOLD {
            SPAWNED_PER_FRAME
        } else {
            SPAWNED_PER_FRAME_CONSTRAINED
        };
        let whole = if wanted.is_finite() {
            (wanted.max(0.0) as usize).min(allowance)
        } else {
            allowance
        };
        *spawned += whole;
        let steps = wanted.min(whole as f32);
        let mut remaining = distance;
        let mut left = steps;
        while left >= 1.0 {
            let step = distance / steps;
            self.extend(tracer, state, false, direction, step);
            remaining -= step;
            left -= 1.0;
        }
        if !self.placed_real || remaining > PLACE_EPSILON {
            self.extend(tracer, state, true, direction, remaining);
        }
    }

    /// `TracerGpu::FrameAdvance`'s spawned branch: rows past their time go;
    /// the rest age, move and take their properties (`tracer_update.fx`),
    /// each by its buffer slot's place.
    pub(super) fn update(&mut self, tracer: &TracerDefinition, state: &TracerState, dt: f32) {
        self.rows.retain(|row| row.timer >= 0.0);
        if self.rows.is_empty() {
            self.leading_temporary = false;
        }
        let count = self.count();
        let step = percentile_step(self.length, tracer.density.scalar(state));
        for (at, row) in self.rows.iter_mut().enumerate() {
            row.timer -= dt;
            for (within, profile) in row.slots.iter_mut().enumerate() {
                if profile.age >= 1.0 {
                    continue;
                }
                let before = TracerState {
                    profile_age: profile.age,
                    percentile: profile.percentile,
                    profile_seeds: profile.random,
                    ..*state
                };
                profile.age += dt / profile.lifespan;
                let acceleration = tracer.acceleration.vector(&before);
                let moving = profile.position.iter_mut().zip(&mut profile.velocity);
                for ((place, velocity), push) in moving.zip(acceleration) {
                    *place += *velocity * dt;
                    *velocity += push * dt;
                }
                tracer.apply_properties(&before, profile);
                let index = at * PROFILES_PER_ROW + within;
                profile.percentile = if index + 1 == count {
                    1.0
                } else {
                    index as f32 * step
                };
            }
        }
    }
}

/// What a slot keeps of its trailing tracers from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct H4TracerField {
    time: Option<f32>,
    running: Vec<RunningTracer>,
}

impl H4TracerField {
    /// Run the trailing tracers up to `time`, their locations where `bones`
    /// has them: `PulseAll`, then each one's update.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn advance(
        &mut self,
        tracers: &[H4TracerAt],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut EffectOnsets,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
    ) {
        if self.running.len() != tracers.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..tracers.len())
                .map(|at| RunningTracer {
                    random: (at as u32).wrapping_mul(977) | 1,
                    ..Default::default()
                })
                .collect();
            self.time = Some(time);
        }
        let dt = time - self.time.unwrap_or(time);
        self.time = Some(time);
        if dt <= 0.0 {
            return;
        }
        // `PulseAll` sets the frame's allowance going once for every tracer.
        let mut spawned = 0;
        for (tracer, running) in tracers.iter().zip(&mut self.running) {
            if !tracer.trails() {
                continue;
            }
            let (at, _) = tracer.location(bones);
            let last = running.location.replace(at);
            if last.is_none() {
                running.last_placed = at;
            }
            running.speed = last.map_or(0.0, |last| {
                length(std::array::from_fn(|axis| last[axis] - at[axis])) / dt
            });
            let values = values(tracer.object);
            let onset = onsets.of(tracer.object, &tracer.scale_by, values.as_ref(), time);
            let TracerFrame {
                mut state, live, ..
            } = tracer.frame(values.as_ref(), at, eye, time, onset);
            state.speed = running.speed;
            state.length = running.length;
            if live {
                running.place_to(&tracer.definition, &state, at, &mut spawned);
            }
            running.update(&tracer.definition, &state, dt);
        }
    }

    /// The trailing tracer at `index`'s frame, or none while it has one
    /// profile or fewer (`TracerGpu::Render`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        index: usize,
        tracer: &H4TracerAt,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<H4TracerDraw> {
        let running = self.running.get(index)?;
        if running.count() <= 1 {
            return None;
        }
        let (origin, forward) = tracer.location(bones);
        let TracerFrame {
            mut state, since, ..
        } = tracer.frame(values, origin, eye, time, onset);
        state.speed = running.speed;
        state.length = running.length;
        let definition = &*tracer.definition;
        let step = percentile_step(running.length, definition.density.scalar(&state));
        let offset = definition.offset.scalar(&state);
        let states = running
            .rows
            .iter()
            .flat_map(|row| &row.slots)
            .map(pack_profile)
            .collect();
        let direction = {
            let length = length(forward).max(1e-6);
            forward.map(|axis| axis / length)
        };
        Some(tracer.draw_of(
            &state,
            since,
            [step, running.length, offset],
            origin,
            direction,
            states,
        ))
    }
}

attached!(H4TracerAt);
