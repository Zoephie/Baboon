//! A tracer evaluated for a frame.

use super::definition::TracerPath;
use super::profile::{H4TracerDraw, ProfileState, pack_profile, percentile_step, profile_count};
use super::properties::{RANDOM_U_OFFSET, RANDOM_V_OFFSET, TracerState};
use super::read::H4TracerAt;
use super::{FunctionValues, length, posed_on};

/// The per-tracer part of a frame: its states, and the seconds its system
/// has run.
pub(super) struct TracerFrame {
    pub(super) state: TracerState,
    pub(super) since: f32,
    /// Its event is running and its effect on: it lays profiles down.
    pub(super) live: bool,
}

impl H4TracerAt {
    pub(crate) fn scale_names(&self) -> [&str; 2] {
        [&self.scale_by, &self.secondary_by]
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn sort_layer(&self) -> i32 {
        self.definition.sort_layer
    }

    /// World units nearer the camera it sorts.
    pub(crate) fn sort_bias(&self) -> f32 {
        self.definition.sort_bias
    }

    /// Its location this frame, and its forward.
    pub(super) fn location(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        let forward = match self.path {
            TracerPath::Trailing { forward } => forward,
            TracerPath::PointToPoint { .. } => [1.0, 0.0, 0.0],
        };
        posed_on(self.start.0, self.start.1, forward, bones)
    }

    /// Its sort position: `Tracer::Submit` hands over its location's.
    pub(crate) fn origin(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        self.location(bones).0
    }

    /// `TracerStates::UpdateStates` (X 0x82991E10) at `time`, from `at`.
    pub(super) fn frame(
        &self,
        values: Option<&FunctionValues>,
        at: [f32; 3],
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> TracerFrame {
        let read = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        let since = time - onset;
        let effect_a = read(&self.scale_by);
        let age = self.times.age(self.event, since);
        let from_eye: [f32; 3] = std::array::from_fn(|axis| at[axis] - eye[axis]);
        TracerFrame {
            state: TracerState {
                game_time: time,
                tracer_seeds: self.seeds,
                system_age: age.unwrap_or(1.0),
                lod: self.lod.at(length(from_eye)),
                effect_a,
                effect_b: read(&self.secondary_by),
                ..Default::default()
            },
            since,
            live: effect_a > 0.0 && age.is_some(),
        }
    }

    /// `SetMaterialState`'s overall state around `states`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_of(
        &self,
        state: &TracerState,
        since: f32,
        [step, capped, offset]: [f32; 3],
        origin: [f32; 3],
        direction: [f32; 3],
        states: Vec<[u32; 16]>,
    ) -> H4TracerDraw {
        let tracer = &*self.definition;
        let random = |flag: u32, seed: f32| {
            if tracer.appearance & flag != 0 {
                seed
            } else {
                0.0
            }
        };
        H4TracerDraw {
            shape: tracer.shape as f32,
            ngon_sides: tracer.ngon_sides as f32,
            appearance: tracer.appearance as f32,
            percentile_step: step,
            capped_length: capped,
            offset,
            game_time: since.max(0.0),
            uv_tiling: tracer.uv_tiling,
            uv_scroll: tracer.uv_scroll,
            uv_offset: [
                random(RANDOM_U_OFFSET, self.seeds[0]),
                random(RANDOM_V_OFFSET, self.seeds[1]),
            ],
            origin,
            direction,
            fade: tracer.fade,
            inputs: std::array::from_fn(|index| {
                if index == 1 {
                    0.0
                } else {
                    state.of(index as u8)
                }
            }),
            strips: tracer.strips(),
            states,
        }
    }

    /// Whether it trails its location, and so runs in an [`H4TracerField`].
    pub(crate) fn trails(&self) -> bool {
        matches!(self.path, TracerPath::Trailing { .. })
    }

    /// A point-to-point tracer's frame at `time`, or none while its event is
    /// not running, its effect is off or it can make no profiles.
    pub(crate) fn draw(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        eye: [f32; 3],
        time: f32,
        onset: f32,
    ) -> Option<H4TracerDraw> {
        let TracerPath::PointToPoint { end } = self.path else {
            return None;
        };
        let tracer = &*self.definition;
        if tracer.most_profiles <= 0 {
            return None;
        }
        let start = self.origin(bones);
        let TracerFrame {
            mut state,
            since,
            live,
        } = self.frame(values, start, eye, time, onset);
        if !live {
            return None;
        }
        let end = posed_on(end.0, end.1, [1.0, 0.0, 0.0], bones).0;
        // `SetStatelessPositionAndLength`: a beam of no length runs forward.
        let span: [f32; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
        let cap = length(span);
        let direction = if cap > 0.0 {
            span.map(|axis| axis / cap)
        } else {
            [1.0, 0.0, 0.0]
        };
        // `Tracer::FrameAdvance`: the length held to the gap between the two.
        let length = tracer.length.scalar(&state);
        let capped = if cap == 0.0 || cap > length {
            length
        } else {
            cap
        };
        state.length = capped;
        let offset = tracer.offset.scalar(&state);
        let density = tracer.density.scalar(&state);
        let count = profile_count(capped, density);
        let step = percentile_step(capped, density);
        // `tracer_stateless_setup.fx`: no age, velocity or random, the
        // initial colour white at an exponent of 1, the initial alpha 1.
        let states = (0..count)
            .map(|index| {
                let percentile = if index == count - 1 {
                    1.0
                } else {
                    index as f32 * step
                };
                state.percentile = percentile;
                let mut profile = ProfileState {
                    position: std::array::from_fn(|axis| {
                        start[axis] + direction[axis] * (offset + capped * percentile)
                    }),
                    initial_alpha: 1.0,
                    percentile,
                    length: percentile * capped,
                    initial_color: [1.0; 4],
                    ..Default::default()
                };
                tracer.apply_properties(&state, &mut profile);
                pack_profile(&profile)
            })
            .collect();
        Some(self.draw_of(
            &state,
            since,
            [step, capped, offset],
            start,
            direction,
            states,
        ))
    }
}
