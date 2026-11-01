//! Halo 3's attached effects, as `effect_new_looping` runs them on an object at
//! rest: what each event's parts are, and where they go.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::*;
use crate::core::source::TagSource;

/// `effect_build_locations`: a location of this name takes the attachment's marker.
const REPLACE: &str = "replace";
const ONLY_FIRST_PERSON: &str = "only in first person";
const ONLY_NONVIOLENT: &str = "nonviolent mode only";
const ANYWHERE: &str = "any environment";
const AIR: &str = "air only";
/// `_effect_part_type_scale_bit`, by its option's name.
const PART_SCALE: &str = "type-specific scale";
const DISABLED: &str = "disabled for debugging";
/// `k_maximum_effect_location_markers`.
pub(super) const MOST_MARKERS: usize = 16;

/// `effect_compute_event_runtime_data`'s shortest duration, a tick.
const ONE_TICK: f32 = 1.0 / 60.0;
const PARALLEL: &str = "run events in parallel";

/// When an event's delay and duration run, in seconds. A bound the game rolls
/// takes its middle, so that a preview holds still from one load to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
struct H3EventTime {
    delay: f32,
    duration: f32,
}

impl H3EventTime {
    fn read(event: TagStruct<'_>) -> Self {
        let delay = event.read_real_bounds("delay bounds");
        let duration = event.read_real_bounds("duration bounds");
        Self::of([delay.lower, delay.upper], [duration.lower, duration.upper])
    }

    /// The bake: a duration that can be 0 is a tick, which also flattens a
    /// random one starting at 0.
    fn of(delay: [f32; 2], duration: [f32; 2]) -> Self {
        let baked = duration[0] <= 0.0 && duration[1] >= 0.0;
        Self {
            delay: (delay[0] + delay[1]) * 0.5,
            duration: if baked { ONE_TICK } else { (duration[0] + duration[1]) * 0.5 },
        }
    }

    fn length(self) -> f32 {
        self.delay + self.duration
    }

    /// `effect_get_event_status`, `time` into this: the normalized age while
    /// in its duration.
    fn age(self, time: f32) -> Option<f32> {
        if time < self.delay {
            return None;
        }
        // A tick's worth of duration restarts on the tick, and is seen at 0.
        if self.duration <= ONE_TICK {
            return Some(0.0);
        }
        Some(((time - self.delay) / self.duration).min(1.0))
    }
}

/// An effect's events, as `effect_new_looping` runs them: always looping.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct H3EffectTimes {
    events: Vec<H3EventTime>,
    loop_start: usize,
    parallel: bool,
}

impl H3EffectTimes {
    pub(super) fn read(effect: TagStruct<'_>) -> Self {
        let flags = effect.read_flag_names("flags").unwrap_or_default();
        Self {
            events: elements(effect, "events").iter().map(|event| H3EventTime::read(*event)).collect(),
            loop_start: usize::try_from(effect.read_block_index("loop start event")).unwrap_or(0),
            parallel: flags.iter().any(|(_, name)| name == PARALLEL),
        }
    }

    /// `event_update_time`: event `index`'s age at `time`, or none while
    /// something else runs. Parallel events restart once all have finished.
    /// Otherwise one follows another, returning to the loop start.
    pub(super) fn age(&self, index: usize, time: f32) -> Option<f32> {
        let event = *self.events.get(index)?;
        if self.parallel {
            let cycle = self.events.iter().map(|event| event.length()).fold(0.0, f32::max);
            let time = time.rem_euclid(cycle.max(ONE_TICK));
            return (time < event.length()).then(|| event.age(time)).flatten();
        }
        let first: f32 = self.events.iter().map(|event| event.length()).sum();
        let looped = self.events.get(self.loop_start..).unwrap_or(&self.events);
        let start = if time < first {
            0
        } else {
            self.events.len() - looped.len()
        };
        let cycle: f32 = looped.iter().map(|event| event.length()).sum();
        let mut left = if time < first { time } else { (time - first).rem_euclid(cycle.max(ONE_TICK)) };
        for (at, candidate) in self.events.iter().enumerate().skip(start) {
            if left < candidate.length() {
                return (at == index).then(|| candidate.age(left)).flatten();
            }
            left -= candidate.length();
        }
        None
    }

    /// Whether the event's age or its being on changes as time passes.
    pub(super) fn moves(&self) -> bool {
        self.events.len() > 1 || self.events.iter().any(|event| event.delay > 0.0 || event.duration > ONE_TICK)
    }
}

/// An event's part on an attachment's effect.
pub(super) struct H3EffectPart<'a> {
    pub(super) attachment: TagStruct<'a>,
    pub(super) part: TagStruct<'a>,
    pub(super) group: u32,
    pub(super) path: String,
    /// The marker its location resolves to, by name.
    pub(super) marker: String,
    /// `A scales values` takes in the attachment's scale.
    pub(super) scaled: bool,
    pub(super) times: Arc<H3EffectTimes>,
    /// Which of the effect's events it belongs to.
    pub(super) event: usize,
}

/// Whether `event_generate_parts` makes a part on an object in third person:
/// out of the water, and not asked to be nonviolent.
fn part_runs(part: TagStruct<'_>) -> bool {
    part_runs_by(part, "create in", "violence mode")
}

/// `part_runs`, for parts and particle systems, which name these two apart.
pub(super) fn part_runs_by(part: TagStruct<'_>, environment: &str, violence: &str) -> bool {
    let named = |field: &str| part.read_enum_name(field).unwrap_or_default();
    let environment = named(environment);
    let disabled = part.read_flag_names("flags").unwrap_or_default();
    matches!(environment.as_str(), "" | ANYWHERE | AIR)
        && named(violence) != ONLY_NONVIOLENT
        && named("camera mode") != ONLY_FIRST_PERSON
        && !disabled.iter().any(|(_, name)| name.starts_with(DISABLED))
}

/// Every part of every effect `object` attaches, with its effect held open.
pub(super) fn for_each_attached_part(
    source: &TagSource,
    object: &TagFile,
    mut visit: impl for<'a> FnMut(H3EffectPart<'a>),
) {
    let Some(block) = object_block(object, "attachments") else {
        return;
    };
    let effect_group = u32::from_be_bytes(*b"effe");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != effect_group || path.is_empty() {
            continue;
        }
        let Ok(effect) = load_referenced_tag_from_source(source, &path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(H3EffectTimes::read(effect.root()));
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for part in elements(event, "parts") {
                let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                    continue;
                };
                let location = usize::try_from(part.read_block_index("location")).ok();
                let location = location.and_then(|location| locations.get(location));
                let (false, Some(location), true) = (path.is_empty(), location, part_runs(part)) else {
                    continue;
                };
                let mut marker = marker_name(*location, "marker name");
                if marker == REPLACE {
                    marker = marker_name(attachment, "marker");
                }
                let scales = part.read_flag_names("A scales values").unwrap_or_default();
                let scaled = scales.iter().any(|(_, name)| name == PART_SCALE);
                let times = Arc::clone(&times);
                visit(H3EffectPart { attachment, part, group, path, marker, scaled, times, event: at });
            }
        }
    }
}

pub(super) fn marker_name(holder: TagStruct<'_>, field: &str) -> String {
    holder.read_string_id(field).or_else(|| holder.read_string(field)).unwrap_or_default()
}

/// `matrix4x3_rotation_from_angles` at no roll: forward, left and up.
fn part_rotation(yaw: f32, pitch: f32) -> [[f32; 3]; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    [
        [cos_pitch * cos_yaw, sin_yaw, sin_pitch * cos_yaw],
        [-cos_pitch * sin_yaw, cos_yaw, -sin_pitch * sin_yaw],
        [-sin_pitch, 0.0, cos_pitch],
    ]
}

/// `effect_get_part_to_node`: a part's own frame within its location's.
pub(super) fn part_frame(location: &MarkerFrame, offset: [f32; 3], yaw: f32, pitch: f32) -> MarkerFrame {
    let of_location = |vector: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| (0..3).map(|at| location.axes[at][axis] * vector[at]).sum())
    };
    let moved = of_location(offset);
    MarkerFrame {
        axes: part_rotation(yaw, pitch).map(of_location),
        position: std::array::from_fn(|axis| location.position[axis] + moved[axis]),
        node: location.node,
    }
}

/// The offset and turn `part_frame` wants, from the tag.
pub(super) fn read_part_placement(part: TagStruct<'_>) -> ([f32; 3], f32, f32) {
    let offset = part.read_point3d("relative offset");
    let angles = part.read_euler2d("relative orientation (yaw, pitch)");
    ([offset.x, offset.y, offset.z], angles.yaw, angles.pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location() -> MarkerFrame {
        // Facing +Y, its left along -X.
        MarkerFrame {
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            position: [1.0, 2.0, 3.0],
            node: 4,
        }
    }

    fn times(events: &[([f32; 2], [f32; 2])], loop_start: usize, parallel: bool) -> H3EffectTimes {
        let events = events.iter().map(|(delay, duration)| H3EventTime::of(*delay, *duration)).collect();
        H3EffectTimes { events, loop_start, parallel }
    }

    #[test]
    fn a_duration_that_can_be_zero_bakes_to_a_tick_and_stays_at_age_zero() {
        let effect = times(&[([0.0; 2], [0.0; 2])], 0, false);
        assert_eq!(effect.events[0].duration, ONE_TICK);
        for time in [0.0, 0.001, 7.3] {
            assert_eq!(effect.age(0, time), Some(0.0));
        }
        assert!(!effect.moves());
        assert_eq!(H3EventTime::of([0.0; 2], [0.0, 3.0]).duration, ONE_TICK);
        assert_eq!(H3EventTime::of([0.0; 2], [1.0, 3.0]).duration, 2.0);
    }

    #[test]
    fn an_event_waits_out_its_delay_then_ages_across_its_duration() {
        let effect = times(&[([1.0; 2], [2.0; 2])], 0, false);
        assert_eq!(effect.age(0, 0.5), None);
        assert_eq!(effect.age(0, 2.0), Some(0.5));
        assert_eq!(effect.age(0, 3.5), None, "looped round into its delay");
        assert_eq!(effect.age(0, 5.0), Some(0.5));
        assert!(effect.moves());
    }

    #[test]
    fn events_follow_each_other_and_loop_from_the_loop_start() {
        let effect = times(&[([0.0; 2], [1.0; 2]), ([0.0; 2], [2.0; 2])], 1, false);
        assert_eq!(effect.age(0, 0.5), Some(0.5));
        assert_eq!(effect.age(1, 0.5), None);
        assert_eq!(effect.age(1, 2.0), Some(0.5));
        assert_eq!(effect.age(0, 3.5), None, "the first event never comes round again");
        assert_eq!(effect.age(1, 3.5), Some(0.25));
    }

    #[test]
    fn parallel_events_run_together_and_restart_when_the_longest_is_done() {
        let effect = times(&[([0.0; 2], [1.0; 2]), ([0.0; 2], [2.0; 2])], 0, true);
        assert_eq!(effect.age(0, 0.5), Some(0.5));
        assert_eq!(effect.age(1, 0.5), Some(0.25));
        assert_eq!(effect.age(0, 1.5), None);
        assert_eq!(effect.age(1, 1.5), Some(0.75));
        assert_eq!(effect.age(0, 2.5), Some(0.5));
    }

    #[test]
    fn a_part_with_no_placement_is_its_location() {
        let frame = part_frame(&location(), [0.0; 3], 0.0, 0.0);
        assert_eq!(frame.position, [1.0, 2.0, 3.0]);
        assert_eq!(frame.axes, location().axes);
        assert_eq!(frame.node, 4);
    }

    #[test]
    fn an_offset_runs_along_the_locations_axes_not_the_worlds() {
        let frame = part_frame(&location(), [0.5, 0.0, 0.0], 0.0, 0.0);
        assert_eq!(frame.position, [1.0, 2.5, 3.0]);
        let frame = part_frame(&location(), [0.0, 0.5, 0.0], 0.0, 0.0);
        assert_eq!(frame.position, [0.5, 2.0, 3.0]);
    }

    #[test]
    fn yaw_turns_toward_left_and_pitch_toward_up() {
        let quarter = std::f32::consts::FRAC_PI_2;
        let [forward, _, _] = part_rotation(quarter, 0.0);
        assert!((forward[1] - 1.0).abs() < 1e-6 && forward[0].abs() < 1e-6);
        let [forward, _, up] = part_rotation(0.0, quarter);
        assert!((forward[2] - 1.0).abs() < 1e-6 && forward[0].abs() < 1e-6);
        assert!((up[0] + 1.0).abs() < 1e-6);
    }
}
