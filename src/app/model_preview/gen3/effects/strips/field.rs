//! The strips' field, stepped a tick at a time.

use super::*;

/// What a slot keeps of its contrails from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct Gen3StripField {
    time: Option<f32>,
    running: Vec<RunningContrail>,
    /// Per strip, one for each particle it follows, with when that began.
    /// Ordered by the particle's id, so they draw in the same order every run.
    pub(super) carried: Vec<BTreeMap<u64, (f32, RunningContrail)>>,
}

impl Gen3StripField {
    /// Run contrails up to `time`, with their locations where `bones` has them.
    pub(crate) fn advance(
        &mut self,
        strips: &[Arc<Gen3StripAt>],
        values: impl Fn(u8) -> Option<FunctionValues>,
        onsets: &mut EffectOnsets,
        tracks: impl Fn(&Arc<Gen3StripAt>) -> Vec<(u64, [f32; 3])>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) {
        if self.running.len() != strips.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..strips.len())
                .map(|at| RunningContrail { random: at as u32 * 977 + 1, ..Default::default() })
                .collect();
            self.carried = vec![BTreeMap::new(); strips.len()];
            self.time = Some(time);
        }
        let dt = time - self.time.unwrap_or(time);
        self.time = Some(time);
        if dt <= 0.0 {
            return;
        }
        for (index, (strip, running)) in strips.iter().zip(&mut self.running).enumerate() {
            let StripKind::Contrail(contrail) = &*strip.kind else {
                continue;
            };
            if strip.carried {
                let live = tracks(strip);
                let carried = &mut self.carried[index];
                for &(id, place) in &live {
                    let (born, follower) = carried.entry(id).or_insert_with(|| {
                        (time, RunningContrail { random: (id as u32).wrapping_mul(977) | 1, previous: place, location: Some(place), ..Default::default() })
                    });
                    if let Some(state) = strip.state(None, time, *born) {
                        follower.location = Some(place);
                        follower.pulse(contrail, &strip.look, &state, place, dt);
                    }
                }
                for (id, (born, follower)) in carried.iter_mut() {
                    let state = strip.state(None, time, *born).unwrap_or_default();
                    follower.update(contrail, &state, dt);
                    if !live.iter().any(|(known, _)| known == id) {
                        follower.location = None;
                    }
                }
                carried.retain(|_, (_, follower)| follower.location.is_some() || !follower.profiles.is_empty());
                continue;
            }
            let at = strip.origin(bones);
            let last = running.location.replace(at);
            if last.is_none() {
                running.previous = at;
            }
            running.speed = last.map_or(0.0, |last| {
                let moved = minus(at, last);
                dot(moved, moved).sqrt() / dt
            });
            let values = values(strip.object);
            let onset = onsets.of(strip.object, &strip.scale_by, values.as_ref(), time);
            let state = strip.state(values.as_ref(), time, onset);
            if let Some(state) = state {
                let state = StripState { speed: running.speed, ..state };
                running.pulse(contrail, &strip.look, &state, at, dt);
                running.update(contrail, &state, dt);
            } else {
                let rest = StripState { game_time: time.rem_euclid(1.0), age: 1.0, seeds: strip.seeds, ..Default::default() };
                running.update(contrail, &rest, dt);
            }
        }
    }

    /// The triangles of a beam or contrail this frame, at the area screen
    /// effects' exposure `stops`.
    pub(crate) fn vertices(
        &self,
        index: usize,
        strip: &Gen3StripAt,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        lighting: &[[f32; 4]; 10],
        time: f32,
        onset: f32,
        stops: f32,
    ) -> Vec<ParticleCorner> {
        match &*strip.kind {
            StripKind::Contrail(contrail) if strip.carried => {
                let followers = self.carried.get(index).into_iter().flatten();
                followers
                    .flat_map(|(_, (born, follower))| {
                        strip.contrail_vertices(contrail, follower, None, view, lighting, time, *born, stops)
                    })
                    .collect()
            }
            StripKind::Beam(beam) => strip.beam_vertices(beam, values, bones, view, time, onset, stops),
            StripKind::Contrail(contrail) => self
                .running
                .get(index)
                .map_or_else(Vec::new, |running| {
                    strip.contrail_vertices(contrail, running, values, view, lighting, time, onset, stops)
                }),
        }
    }
}

attached!(Gen3StripAt);
