//! Playing an animation: its entry, playback state and the frames it samples.

use super::*;

/// One row of the preview's animation list.
#[derive(Debug, Clone)]
pub(crate) struct PreviewAnimationEntry {
    pub name: String,
    pub frame_count: u16,
    /// A JMA-family label (`jma`, `jmo`, `jmr`, …) for the row.
    pub kind: &'static str,
    /// False when the build kept no payload for this animation (a monolithic
    /// capture's unpaged resource, or a runtime-blend composite) — listed,
    /// but not selectable.
    pub playable: bool,
}

/// Cross-frame playback state, one per previewed document.
pub(crate) struct PreviewAnimationPlayback {
    pub selected: Option<usize>,
    pub playing: bool,
    /// Stop is distinct from pause: it shows the bind pose while retaining
    /// the selected and decoded animation.
    pub stopped: bool,
    pub looped: bool,
    /// Blend between frames. Off, the pose holds each frame whole, which is
    /// how an overlay animation is read frame by frame.
    pub interpolate: bool,
    pub speed: f32,
    /// Seconds into the animation.
    pub time: f32,
    pub pose: Option<std::sync::Arc<PreviewAnimationPose>>,
    /// The animation index a decode worker is running for.
    pub decoding: Option<usize>,
    pub error: Option<String>,
    /// The one-shot guard on the list request for this load.
    pub requested_list: bool,
    /// Case-insensitive substring filter for the animation picker; a graph
    /// can list a thousand animations.
    pub filter: String,
    /// The egui pass the clock last advanced in. Two panes showing the same
    /// tag share this state, and each advanced the clock, so playback ran at
    /// twice the speed.
    pub advanced_in_pass: Option<u64>,
}

impl Default for PreviewAnimationPlayback {
    fn default() -> Self {
        Self {
            selected: None,
            playing: false,
            stopped: false,
            looped: true,
            interpolate: true,
            speed: 1.0,
            time: 0.0,
            pose: None,
            decoding: None,
            error: None,
            requested_list: false,
            filter: String::new(),
            advanced_in_pass: None,
        }
    }
}

/// Where playback stands within `frame_count` frames: past the end of a
/// looping clip it continues from the loop frame, and a clip that doesn't
/// loop holds its last frame. On a whole frame when not interpolating.
pub(in crate::app::model_preview) fn playback_frame_position(playback: &PreviewAnimationPlayback, frame_count: usize) -> f32 {
    let mut position = (playback.time * ANIMATION_FRAME_RATE).max(0.0);
    if playback.looped && frame_count > 1 {
        position = looped_frame_position(playback, position, frame_count);
    } else {
        position = position.min(frame_count.saturating_sub(1) as f32);
    }
    if playback.interpolate { position } else { position.floor() }
}

/// `position` frames into a looping clip of `frame_count` frames. The game
/// plays to the end, then continues from the animation's loop frame, or
/// restarts at frame 0 when that is 0; nothing blends the last frame back
/// into the loop frame.
pub(in crate::app::model_preview) fn looped_frame_position(
    playback: &PreviewAnimationPlayback,
    position: f32,
    frame_count: usize,
) -> f32 {
    let count = frame_count as f32;
    if position < count {
        return position;
    }
    let start = playback
        .pose
        .as_ref()
        .map_or(0, |pose| pose.loop_frame)
        .min(frame_count.saturating_sub(1)) as f32;
    start + (position - start) % (count - start)
}

/// The two frames playback lies between and how far it is from the first to
/// the second. The last frame is held rather than blended into the next
/// loop. `frame_count` must not be 0.
pub(super) fn sampled_frames(playback: &PreviewAnimationPlayback, frame_count: usize) -> (usize, usize, f32) {
    let position = playback_frame_position(playback, frame_count);
    let a = (position.floor() as usize).min(frame_count - 1);
    let b = (a + 1).min(frame_count - 1);
    (a, b, position - a as f32)
}
