//! The clock frame-advanced simulations step by: how a frame's gap becomes
//! steps, and where a simulation picks up after a stall.

/// How far a frame can trail the last before the gap counts as a stall (a
/// hidden pane, a long load) and is skipped rather than replayed tick by tick.
const MOST_FRAME_GAP: f32 = 1.0;

/// Where a simulation at `now` picks up to reach `time`: no further back than
/// its own `rollback` (what a reset steps through to settle) and one frame
/// gap, so a stall never makes one frame simulate minutes.
pub(in crate::app::model_preview) fn resume_point(now: f32, time: f32, rollback: f32) -> f32 {
    now.max(time - rollback - MOST_FRAME_GAP)
}

/// The longest frame a frame-advanced simulation takes in one step: Halo 3
/// and ODST's two 60 Hz ticks (`game_time_update` caps a frame's `game_dt` at
/// them), the shortest cap of the games.
const MOST_FRAME_STEP: f32 = 1.0 / 30.0;

/// How long each frame of a longer gap is: a reset's pre-roll, which no game
/// has, or a stall `resume_point` let through.
const PRE_ROLL_FRAME: f32 = 1.0 / 60.0;

/// The steps a frame-advanced simulation takes from `last` to `time`. Every
/// game advances its particles, contrails and most widgets once per rendered frame,
/// by that frame's time (`game_frame`, `effects_frame_advance`): a preview
/// frame is one step of its own length, and a longer gap runs as
/// [`PRE_ROLL_FRAME`]s.
pub(in crate::app::model_preview) fn frame_steps(
    last: f32,
    time: f32,
) -> impl Iterator<Item = f32> {
    let gap = time - last;
    let count = if gap <= 0.0 {
        0
    } else if gap <= MOST_FRAME_STEP {
        1
    } else {
        (gap / PRE_ROLL_FRAME).ceil() as usize
    };
    std::iter::repeat_n(gap / count.max(1) as f32, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simulation steps through an ordinary frame and its own reset rollback,
    /// but a stall picks up just short of the frame instead of replaying it.
    #[test]
    fn a_stalled_simulation_skips_rather_than_replays() {
        assert_eq!(resume_point(10.0, 10.02, 0.0), 10.0, "an ordinary frame");
        assert_eq!(resume_point(4.0, 10.0, 6.0), 4.0, "a reset's own rollback");
        let resumed = resume_point(10.0, 610.0, 6.0);
        assert!(
            (603.0..610.0).contains(&resumed),
            "ten minutes behind: {resumed}"
        );
    }

    /// A frame is one step of its own length, up to Halo 3's cap of two ticks; a
    /// longer gap runs as 60 Hz frames that add up to it, and no gap runs none.
    #[test]
    fn a_frame_is_one_step_and_a_longer_gap_is_many() {
        let steps = |last: f32, time: f32| frame_steps(last, time).collect::<Vec<_>>();
        assert_eq!(steps(10.0, 10.0), [] as [f32; 0], "no time");
        assert_eq!(steps(10.0, 9.0), [] as [f32; 0], "time going back");
        assert_eq!(steps(0.0, 0.25 / 30.0), [0.25 / 30.0], "a 120 fps frame");
        assert_eq!(steps(0.0, 1.0 / 30.0), [1.0 / 30.0], "the longest one step");
        let gap = steps(0.0, 1.0);
        assert_eq!(gap.len(), 60, "a second's gap");
        assert!((gap.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(gap.iter().all(|step| (step - 1.0 / 60.0).abs() < 1e-6));
    }
}
