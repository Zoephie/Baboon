//! Halo CE's simulation clock: its 30 ticks a second, the pre-roll, and the ticks a frame runs.

/// A game tick, which is what `effects_update` steps by.
pub(in crate::app::model_preview) const STEP: f32 = 1.0 / 30.0;

pub(super) const PRE_ROLL: f32 = 4.0;

/// How far short of a tick the frames' time can fall by rounding alone.
const TICK_ROUNDING: f32 = 1e-5;

/// The frames' time not yet run as ticks, as `game_time_update` keeps it.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::app::model_preview) struct CeTicks(f32);

impl CeTicks {
    /// How many ticks a frame `dt` long runs: `game_time_get_elapsed`, which
    /// `game_frame` runs `particles_update` and `widgets_update` by.
    pub(in crate::app::model_preview) fn run(&mut self, dt: f32) -> u32 {
        self.0 += dt;
        let mut ticks = 0;
        // A frame of two half ticks owes a whole one, whatever the rounding.
        while self.0 >= STEP - TICK_ROUNDING {
            self.0 -= STEP;
            ticks += 1;
        }
        ticks
    }
}
