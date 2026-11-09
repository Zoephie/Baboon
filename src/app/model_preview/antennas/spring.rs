//! Reach's antennas as they move: `antennas_update` (the 360's 0x82C932B0,
//! MCC's 0x140E52C90) springs each antenna's far end toward the line out of
//! its marker, once a game tick, under its point physics. The near end is the
//! marker itself; [`super::Antenna`] draws the curve between.

use super::super::cloth::FIXED_TICK_RATE;
use super::super::common::view::posed_on;
use super::Antenna;
use crate::app::model_preview::common::math::{minus, plus, scaled};
use crate::app::model_preview::common::point_physics::CePointPhysics;

/// The step `antennas_update` hands every antenna, whatever the tick's length:
/// the 360's constant `flt_85251DE4`; MCC's `min(tick, 0.0333)` (0x140E52D1B)
/// comes to the same.
const STEP: f32 = 0.0333;
/// The radius `antenna_update` (the 360's 0x82C93390) gives the far end's
/// `point_physics_update`.
const POINT_RADIUS: f32 = 0.02;
/// How far `antenna_update_attachment` (the 360's 0x82C93898) lets its marker
/// move along any axis in a tick before it carries both ends along.
const TELEPORT: f32 = 1.0;

/// What an antenna's tag gives its spring.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AntennaSpring {
    /// `spring strength coefficient`: how much of the way to its rest the far
    /// end is pulled each tick.
    pub(crate) strength: f32,
    /// `runtime total length`.
    pub(crate) length: f32,
    /// Its `physics`, if it names one.
    pub(crate) physics: Option<CePointPhysics>,
}

/// One antenna's two points as they stand: `point[0]` is where the marker
/// was, `point[1]` the far end.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RunningAntenna {
    /// Where `antenna_update_attachment` last found the marker.
    attach: [f32; 3],
    near: [f32; 3],
    far: [f32; 3],
    velocity: [f32; 3],
}

impl RunningAntenna {
    /// Both ends where the spring rests them. The game starts the far end
    /// straight up from the world's origin and lets it fall into place; that
    /// first transient is not something to reproduce. \[inferred\]
    fn at_rest(marker: [f32; 3], forward: [f32; 3], spring: &AntennaSpring) -> Self {
        Self {
            attach: marker,
            near: marker,
            far: plus(marker, scaled(forward, spring.length)),
            velocity: [0.0; 3],
        }
    }

    /// One tick of `antenna_update`, with the marker at `marker` facing the
    /// unit `forward`.
    fn step(&mut self, marker: [f32; 3], forward: [f32; 3], spring: &AntennaSpring) {
        let moved = minus(marker, self.attach);
        if moved.iter().any(|axis| axis.abs() > TELEPORT) {
            self.near = plus(self.near, moved);
            self.far = plus(self.far, moved);
        }
        self.attach = marker;
        // `point[0]`: the marker, and the rest its far end is pulled to. The
        // game swings `global_up · length` onto the forward, which for a unit
        // forward is this.
        self.near = marker;
        let rest = plus(marker, scaled(forward, spring.length));
        // `point[1]`.
        let mut place = self.far;
        if let Some(physics) = &spring.physics {
            // In air alone: a preview has no structure to collide with.
            physics.step(
                &mut place,
                &mut self.velocity,
                POINT_RADIUS,
                f32::NEG_INFINITY,
                STEP,
            );
        }
        // Held at its length from the near end, then eased toward its rest.
        let out = minus(place, marker);
        let distance = out.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        if distance > 0.0 {
            place = plus(marker, scaled(out, spring.length / distance));
        }
        place = plus(
            scaled(place, 1.0 - spring.strength),
            scaled(rest, spring.strength),
        );
        self.velocity = scaled(minus(place, self.far), 1.0 / STEP);
        self.far = place;
    }
}

/// What a slot keeps of its Reach antennas from frame to frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct AntennaSprings {
    /// When it began, and the last time it was run to.
    start: f32,
    time: Option<f32>,
    /// The ticks run since `start`.
    ticks: u64,
    running: Vec<Option<RunningAntenna>>,
}

impl AntennaSprings {
    /// Run the ticks up to `time`, and hand back where each antenna's far end
    /// is; none for one that is not Reach's.
    pub(crate) fn advance(
        &mut self,
        antennas: &[Antenna],
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) -> Vec<Option<[f32; 3]>> {
        let marker_of = |antenna: &Antenna| antenna.marker(bones);
        if self.running.len() != antennas.len() || self.time.is_none_or(|last| time < last) {
            self.running = antennas
                .iter()
                .map(|antenna| {
                    let spring = antenna.spring()?;
                    let (marker, forward) = marker_of(antenna);
                    Some(RunningAntenna::at_rest(marker, forward, spring))
                })
                .collect();
            (self.start, self.ticks) = (time, 0);
        }
        self.time = Some(time);
        // `game_time > last_update_tick`: at most one step a tick.
        let due = ((time - self.start) * FIXED_TICK_RATE).floor() as u64;
        while self.ticks < due {
            self.ticks += 1;
            for (antenna, running) in antennas.iter().zip(&mut self.running) {
                if let (Some(spring), Some(running)) = (antenna.spring(), running.as_mut()) {
                    let (marker, forward) = marker_of(antenna);
                    running.step(marker, forward, spring);
                }
            }
        }
        self.running
            .iter()
            .map(|running| running.map(|running| running.far))
            .collect()
    }
}

/// A marker's place and unit forward on `node`, where the pose puts them.
pub(in crate::app::model_preview) fn posed_marker(
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    bones: Option<&[[f32; 4]]>,
) -> ([f32; 3], [f32; 3]) {
    let (place, forward) = posed_on(node, position, forward, bones);
    let length = forward.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
    (
        place,
        if length > 0.0 {
            scaled(forward, 1.0 / length)
        } else {
            forward
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `warthog_antenna_steel.point_physics`: steel, a little air friction.
    fn steel() -> CePointPhysics {
        CePointPhysics::new(7.7, false, false, 0.05, 0.0, 0.0)
    }

    fn warthog(physics: Option<CePointPhysics>) -> AntennaSpring {
        AntennaSpring {
            strength: 0.1,
            length: 0.4,
            physics,
        }
    }

    fn settled(spring: &AntennaSpring, forward: [f32; 3], ticks: usize) -> [f32; 3] {
        let mut running = RunningAntenna::at_rest([0.0; 3], forward, spring);
        for _ in 0..ticks {
            running.step([0.0; 3], forward, spring);
        }
        running.far
    }

    /// `sim_antenna.py`'s 600 ticks: an antenna standing up stays straight,
    /// its length taking out what gravity does along it; one lying flat sags
    /// to where the spring holds the weight, some 6 degrees; without its
    /// physics it does not sag at all.
    #[test]
    fn an_antenna_stands_straight_and_one_lying_flat_sags_under_its_weight() {
        let up = settled(&warthog(Some(steel())), [0.0, 0.0, 1.0], 600);
        assert!(
            up[0].abs() < 1e-6 && up[1].abs() < 1e-6 && (up[2] - 0.4).abs() < 1e-5,
            "{up:?}"
        );
        let flat = settled(&warthog(Some(steel())), [1.0, 0.0, 0.0], 600);
        assert!(
            (flat[0] - 0.3976).abs() < 2e-4
                && flat[1].abs() < 1e-6
                && (flat[2] + 0.041).abs() < 5e-4,
            "{flat:?}"
        );
        let weightless = settled(&warthog(None), [1.0, 0.0, 0.0], 600);
        assert_eq!(weightless, [0.4, 0.0, 0.0]);
    }

    /// A marker that moves within a unit drags the far end through its spring;
    /// one that jumps past it takes both ends along.
    #[test]
    fn a_marker_that_jumps_takes_the_antenna_along() {
        let spring = warthog(None);
        let mut running = RunningAntenna::at_rest([0.0; 3], [0.0, 0.0, 1.0], &spring);
        running.step([0.5, 0.0, 0.0], [0.0, 0.0, 1.0], &spring);
        assert!(
            running.far[0] < 0.5 - 1e-3,
            "trailing behind: {:?}",
            running.far
        );
        let mut jumped = RunningAntenna::at_rest([0.0; 3], [0.0, 0.0, 1.0], &spring);
        jumped.step([2.0, 0.0, 0.0], [0.0, 0.0, 1.0], &spring);
        assert!(
            (jumped.far[0] - 2.0).abs() < 1e-6 && (jumped.far[2] - 0.4).abs() < 1e-6,
            "{:?}",
            jumped.far
        );
    }

    /// `antennas_update` runs at most once a tick, at Reach's rate, however
    /// many frames come between.
    #[test]
    fn an_antenna_steps_by_the_tick_whatever_the_frame_rate() {
        let ticks_over_a_second = |frames: usize| {
            let mut field = AntennaSprings::default();
            field.advance(&[], None, 10.0);
            (1..=frames).for_each(|frame| {
                _ = field.advance(&[], None, 10.0 + frame as f32 / frames as f32)
            });
            field.ticks
        };
        assert_eq!(ticks_over_a_second(60), FIXED_TICK_RATE as u64);
        assert_eq!(ticks_over_a_second(240), FIXED_TICK_RATE as u64);
    }
}
