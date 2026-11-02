//! Halo 3's attached effects, as `effect_new_looping` runs them on an object at
//! rest: what each event's parts are, and where they go.

use std::collections::HashMap;
use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::h2_functions::H2FunctionValues;
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
    /// Over after one run, as an event's effect is.
    once: bool,
}

impl H3EffectTimes {
    pub(super) fn read(effect: TagStruct<'_>) -> Self {
        let flags = effect.read_flag_names("flags").unwrap_or_default();
        Self {
            events: elements(effect, "events").iter().map(|event| H3EventTime::read(*event)).collect(),
            loop_start: usize::try_from(effect.read_block_index("loop start event")).unwrap_or(0),
            parallel: flags.iter().any(|(_, name)| name == PARALLEL),
            once: false,
        }
    }

    /// `event_update_time`: event `index`'s age at `time`, or none while
    /// something else runs. Parallel events restart once all have finished.
    /// Otherwise one follows another, returning to the loop start.
    pub(super) fn age(&self, index: usize, time: f32) -> Option<f32> {
        let event = *self.events.get(index)?;
        if self.once && !(0.0..self.once()).contains(&time) {
            return None;
        }
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

    /// Seconds since `index` began, ended or not, for what outlives its event.
    pub(super) fn since_begun(&self, index: usize, time: f32) -> Option<f32> {
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events.iter().take(index).map(|event| event.length()).sum()
        };
        let cycle = self.once();
        let time = if self.once || cycle <= 0.0 { time } else { time.rem_euclid(cycle.max(ONE_TICK)) };
        let age = time - before - event.delay;
        (age >= 0.0).then_some(age)
    }

    #[cfg(test)]
    pub(super) fn read_nothing() -> Self {
        Self {
            events: vec![H3EventTime::of([0.0, 0.0], [100.0, 100.0])],
            loop_start: 0,
            parallel: false,
            once: false,
        }
    }

    /// Whether a run that does not loop has finished.
    pub(super) fn is_over(&self, time: f32) -> bool {
        self.once && time >= self.once()
    }

    /// How long a run that does not loop takes.
    pub(super) fn once(&self) -> f32 {
        let lengths = self.events.iter().map(|event| event.length());
        if self.parallel { lengths.fold(0.0, f32::max) } else { lengths.sum() }
    }

    /// A time inside `index`, if it first begins after `from` and by `to`: a
    /// step can be longer than a whole event, and must not pass over it. Half a
    /// tick in, as rounding can put its very start before it.
    pub(super) fn begins_within(&self, index: usize, from: f32, to: f32) -> Option<f32> {
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events.iter().take(index).map(|event| event.length()).sum()
        };
        let begin = before + event.delay;
        (from < begin && begin <= to).then_some(begin + ONE_TICK * 0.5)
    }

    /// Seconds since `index` began, for what outlasts its normalized age. An
    /// effect that only ticks over has been running all along.
    pub(super) fn elapsed(&self, index: usize, time: f32) -> Option<f32> {
        self.age(index, time)?;
        if !self.moves() {
            return Some(time);
        }
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events.iter().take(index).map(|event| event.length()).sum()
        };
        let cycle: f32 = if self.parallel {
            self.events.iter().map(|event| event.length()).fold(0.0, f32::max)
        } else {
            self.events.iter().map(|event| event.length()).sum()
        };
        Some((time.rem_euclid(cycle.max(ONE_TICK)) - before - event.delay).max(0.0))
    }

    /// Whether the event's age or its being on changes as time passes.
    pub(super) fn moves(&self) -> bool {
        self.once || self.events.len() > 1 || self.events.iter().any(|event| event.delay > 0.0 || event.duration > ONE_TICK)
    }
}

/// When each attachment's function last came on: `attachments_update` makes
/// the effect then, so its events run from there. What was on from the first
/// frame has run all along.
#[derive(Debug, Clone, Default)]
pub(crate) struct H3EffectOnsets(HashMap<(u8, String), (bool, f32)>);

impl H3EffectOnsets {
    pub(crate) fn of(&mut self, object: u8, name: &str, values: Option<&H2FunctionValues>, time: f32) -> f32 {
        if name.is_empty() {
            return 0.0;
        }
        let on = values.and_then(|values| values.get(name)).is_some_and(|value| *value > 0.0);
        let seen = self.0.entry((object, name.to_owned())).or_insert((on, 0.0));
        if on && !seen.0 {
            seen.1 = time;
        }
        seen.0 = on;
        if seen.1 > time {
            seen.1 = 0.0;
        }
        seen.1
    }
}

/// An event's part on an attachment's effect.
pub(super) struct H3EffectPart<'a> {
    pub(super) effect: &'a H3ObjectEffect,
    pub(super) part: TagStruct<'a>,
    pub(super) group: u32,
    pub(super) path: String,
    /// The marker its location resolves to, by name.
    pub(super) marker: String,
    /// `secondary location (beams)`, by name.
    pub(super) secondary_marker: Option<String>,
    /// `use parent position but world orientation`, of its effect.
    pub(super) world_oriented: bool,
    /// `never attached to object`: it stays where it began.
    pub(super) unattached: bool,
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
    for object_effect in h3_object_effects(source, object) {
        let Ok(effect) = load_referenced_tag_from_source(source, &object_effect.path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(object_effect.times_of(effect.root()));
        let world_oriented = H3ObjectEffect::world_oriented(effect.root());
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
                let marker = object_effect.marker_of(*location);
                let secondary = usize::try_from(part.read_block_index("secondary location (beams)")).ok();
                let secondary_marker = secondary.and_then(|at| locations.get(at)).map(|at| marker_name(*at, "marker name"));
                let flags = part.read_flag_names("flags").unwrap_or_default();
                let unattached = flags.iter().any(|(_, name)| name.starts_with("never attached to object"));
                let scales = part.read_flag_names("A scales values").unwrap_or_default();
                let scaled = scales.iter().any(|(_, name)| name == PART_SCALE);
                let times = Arc::clone(&times);
                visit(H3EffectPart {
                    effect: &object_effect,
                    part,
                    group,
                    path,
                    marker,
                    secondary_marker,
                    world_oriented,
                    unattached,
                    scaled,
                    times,
                    event: at,
                });
            }
        }
    }
}

impl H3EffectPart<'_> {
    /// Where a marker puts this part: its frame and the node it follows.
    pub(super) fn placed(&self, place: MarkerFrame, node: usize) -> (MarkerFrame, usize) {
        let place = if self.world_oriented { MarkerFrame { axes: WORLD_AXES, ..place } } else { place };
        (place, if self.unattached { NO_NODE } else { node })
    }
}

/// An effect an object runs: an attachment, on while its function is, or one
/// an event plays, which the preview gives a function of its own to start it.
#[derive(Debug, Clone)]
pub(super) struct H3ObjectEffect {
    pub(super) path: String,
    marker: String,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    pub(super) change_color: i128,
    /// Plays through once, as an event's does; attachments and `looping effect`s loop.
    pub(super) once: bool,
}

impl H3ObjectEffect {
    /// `effect_build_locations`' marker for a location.
    pub(super) fn marker_of(&self, location: TagStruct<'_>) -> String {
        let marker = marker_name(location, "marker name");
        if marker == REPLACE { self.marker.clone() } else { marker }
    }

    /// `use parent position but world orientation`.
    pub(super) fn world_oriented(effect: TagStruct<'_>) -> bool {
        let flags = effect.read_flag_names("flags").unwrap_or_default();
        flags.iter().any(|(_, name)| name.starts_with(WORLD_ORIENTATION))
    }

    pub(super) fn times_of(&self, effect: TagStruct<'_>) -> H3EffectTimes {
        H3EffectTimes { once: self.once, ..H3EffectTimes::read(effect) }
    }
}

/// What a location of this name lands on: its markers, or, for what no marker
/// answers, the effect's own point. `build_common_effect_vectors` names those
/// directions at the point an event happened, and `root` is the object's.
pub(super) fn part_markers(
    markers: &[RenderModelPreviewMarker],
    name: &str,
    once: bool,
) -> Vec<RenderModelPreviewMarker> {
    let named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(name));
    let named: Vec<_> = named.take(MOST_MARKERS).cloned().collect();
    if !named.is_empty() {
        return named;
    }
    if !once && !name.eq_ignore_ascii_case("root") {
        return Vec::new();
    }
    vec![RenderModelPreviewMarker {
        name: name.to_owned(),
        node_index: 0,
        position: [0.0; 3],
        axes: axes_of_vector(effect_vector(name)),
        scale: 1.0,
        only_under: None,
    }]
}

/// Where a named vector points, on an object facing along x with nothing under
/// it but the ground. Anything else, such as `root`, is the object's own.
fn effect_vector(name: &str) -> [f32; 3] {
    match name {
        "gravity" => [0.0, 0.0, -1.0],
        "up" | "normal" => [0.0, 0.0, 1.0],
        "backward" => [-1.0, 0.0, 0.0],
        _ => [1.0, 0.0, 0.0],
    }
}

fn axes_of_vector(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = forward;
    if z.abs() > 0.5 {
        [forward, [0.0, 1.0, 0.0], [-z, 0.0, x]]
    } else {
        [forward, [-y, x, 0.0], [0.0, 0.0, 1.0]]
    }
}

/// Every effect `object` and its `.model` name, attachments first.
pub(super) fn h3_object_effects(source: &TagSource, object: &TagFile) -> Vec<H3ObjectEffect> {
    let mut effects = Vec::new();
    let effect_group = u32::from_be_bytes(*b"effe");
    if let Some(block) = object_block(object, "attachments") {
        for attachment in elements(block, "attachments") {
            let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
                continue;
            };
            if group == effect_group && !path.is_empty() {
                effects.push(H3ObjectEffect {
                    path,
                    marker: marker_name(attachment, "marker"),
                    scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                    secondary_by: attachment.read_string_id("secondary scale").unwrap_or_default(),
                    change_color: attachment.read_int_any("change color").unwrap_or(0),
                    once: false,
                });
            }
        }
    }
    let mut named = Vec::new();
    collect_event_effects(object.root(), &mut named);
    let model = object_reference(object, "model")
        .and_then(|path| load_referenced_tag_from_source(source, &path, "model", b"hlmt").ok());
    if let Some(model) = &model {
        collect_event_effects(model.root(), &mut named);
    }
    let mut seen = std::collections::HashSet::new();
    for (field, path) in named {
        if seen.insert(path.clone()) {
            effects.push(H3ObjectEffect {
                scale_by: event_function_name(&field, &path),
                once: !field.contains("looping"),
                path,
                marker: String::new(),
                secondary_by: String::new(),
                change_color: 0,
            });
        }
    }
    with_nested_effects(source, effects)
}

const WORLD_AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// Past any pose, so what holds it stays at rest.
const NO_NODE: usize = usize::MAX / 8;
const WORLD_ORIENTATION: &str = "use parent position but world orientation";

/// Whether a function is one the preview made to start an event's effect.
pub(super) fn is_event_function(name: &str) -> bool {
    name.rsplit_once(": ").is_some_and(|(field, _)| field.contains("effect") || field.contains("detonation"))
}

/// Hand an attached object's event effect to the object it is on, under its
/// name, so that one panel plays them all.
pub(super) fn adopt_event_function(name: &str, function: &mut String, object: &mut u8) {
    if is_event_function(function) {
        *function = format!("{name}: {function}");
        *object = 0;
    }
}

/// How deep an effect's parts may name further effects: the tags go one down.
const MOST_NESTING: usize = 3;

/// Each effect, then those its parts name, at those parts' locations. A part's
/// effect is played by its event, once, under the same function.
fn with_nested_effects(source: &TagSource, mut effects: Vec<H3ObjectEffect>) -> Vec<H3ObjectEffect> {
    let effect_group = u32::from_be_bytes(*b"effe");
    let mut level = 0..effects.len();
    for _ in 0..MOST_NESTING {
        let mut nested = Vec::new();
        for outer in &effects[level.clone()] {
            let Ok(tag) = load_referenced_tag_from_source(source, &outer.path, "effect", b"effe") else {
                continue;
            };
            let locations = elements(tag.root(), "locations");
            let parts = elements(tag.root(), "events").into_iter().flat_map(|event| elements(event, "parts"));
            for part in parts {
                let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                    continue;
                };
                let location = usize::try_from(part.read_block_index("location")).ok();
                let location = location.and_then(|location| locations.get(location));
                if let (true, false, Some(location)) = (group == effect_group, path.is_empty(), location) {
                    let marker = outer.marker_of(*location);
                    nested.push(H3ObjectEffect { path, marker, once: true, ..outer.clone() });
                }
            }
        }
        let start = effects.len();
        effects.extend(nested);
        level = start..effects.len();
        if level.is_empty() {
            break;
        }
    }
    effects
}

/// The name an event's effect goes by among the object's functions.
fn event_function_name(field: &str, path: &str) -> String {
    let stem = path.rsplit('\\').next().unwrap_or(path);
    format!("{field}: {stem}")
}

/// Every effect reference under `holder`, by the field that names it, leaving
/// out `attachments`, which run on their own.
fn collect_event_effects(holder: TagStruct<'_>, found: &mut Vec<(String, String)>) {
    let effect_group = u32::from_be_bytes(*b"effe");
    for field in holder.fields() {
        let name = field.name();
        if name == "attachments" {
            continue;
        }
        if field.field_type() == blam_tags::TagFieldType::TagReference {
            if let Some((group, path)) = holder.read_tag_ref_with_group(name)
                && group == effect_group
                && !path.is_empty()
            {
                found.push((name.to_owned(), path));
            }
        } else if let Some(child) = field.as_struct() {
            collect_event_effects(child, found);
        } else if let Some(block) = field.as_block() {
            block.iter().for_each(|element| collect_event_effects(element, found));
        } else if let Some(array) = field.as_array() {
            array.iter().for_each(|element| collect_event_effects(element, found));
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
        H3EffectTimes { events, loop_start, parallel, once: false }
    }

    #[test]
    fn an_effect_restarts_each_time_its_function_comes_on() {
        let mut onsets = H3EffectOnsets::default();
        let values = |value: f32| H2FunctionValues::from([("on".to_owned(), value)]);
        let of = |onsets: &mut H3EffectOnsets, value, time| onsets.of(0, "on", Some(&values(value)), time);
        assert_eq!(of(&mut onsets, 1.0, 5.0), 0.0, "on from the start");
        assert_eq!(of(&mut onsets, 0.0, 6.0), 0.0);
        assert_eq!(of(&mut onsets, 1.0, 7.0), 7.0);
        assert_eq!(of(&mut onsets, 1.0, 8.0), 7.0);
        assert_eq!(of(&mut onsets, 1.0, 2.0), 0.0, "the clock went back");
        assert_eq!(onsets.of(0, "", None, 9.0), 0.0);
    }

    #[test]
    fn seconds_run_from_an_events_delay_and_a_tick_runs_from_the_start() {
        let two = times(&[([1.0, 1.0], [2.0, 2.0]), ([0.0, 0.0], [3.0, 3.0])], 0, false);
        assert_eq!(two.elapsed(0, 0.5), None);
        assert_eq!(two.elapsed(0, 2.5), Some(1.5));
        assert_eq!(two.elapsed(1, 4.0), Some(1.0));
        let tick = times(&[([0.0, 0.0], [0.0, 0.0])], 0, false);
        assert_eq!(tick.elapsed(0, 7.0), Some(7.0));
    }

    #[test]
    fn a_delayed_tick_of_an_event_is_found_however_late_it_was_played() {
        let delayed = H3EffectTimes { once: true, ..times(&[([0.1, 0.1], [0.0, 0.0])], 0, false) };
        let inside = delayed.begins_within(0, 0.05, 0.11).unwrap();
        assert!(delayed.age(0, inside).is_some());
        // The sum this must not go through: late enough, it rounds down.
        let late = (0..3000).map(|at| 100.0 + at as f32 * 0.1169).any(|onset: f32| (0.1 + onset) - onset < 0.1);
        assert!(late);
    }

    #[test]
    fn a_step_longer_than_an_event_still_sees_it_begin() {
        let two = times(&[([0.0, 0.0], [0.0, 0.0]), ([0.0, 0.0], [0.0, 0.0])], 0, false);
        assert_eq!(two.begins_within(1, 0.0, 0.05), Some(ONE_TICK * 1.5));
        assert_eq!(two.begins_within(1, 0.05, 0.1), None);
        assert_eq!(two.begins_within(0, -0.01, 0.05), Some(ONE_TICK * 0.5));
        assert_eq!(two.begins_within(0, 0.0, 0.05), None, "already begun");
    }

    #[test]
    fn a_named_vector_sits_at_the_object_and_points_its_way() {
        let down = part_markers(&[], "gravity", true);
        assert_eq!(down[0].position, [0.0; 3]);
        assert_eq!(down[0].axes[0], [0.0, 0.0, -1.0]);
        assert_eq!(part_markers(&[], "up", true)[0].axes[0], [0.0, 0.0, 1.0]);
        assert_eq!(part_markers(&[], "backward", true)[0].axes[0], [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_looping_effect_needs_its_marker_unless_it_asks_for_the_root() {
        assert!(part_markers(&[], "fx_vent", false).is_empty());
        assert_eq!(part_markers(&[], "root", false).len(), 1);
    }

    #[test]
    fn a_real_marker_of_that_name_wins() {
        let marker = RenderModelPreviewMarker {
            name: "up".to_owned(),
            node_index: 3,
            position: [1.0, 2.0, 3.0],
            axes: axes_of_vector([1.0, 0.0, 0.0]),
            scale: 1.0,
            only_under: None,
        };
        assert_eq!(part_markers(&[marker], "up", true)[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn an_events_effect_runs_once_and_an_attachment_loops() {
        let looped = times(&[([0.0, 0.0], [2.0, 2.0])], 0, false);
        assert!(looped.age(0, 3.0).is_some());
        let once = H3EffectTimes { once: true, ..times(&[([0.0, 0.0], [2.0, 2.0])], 0, false) };
        assert!(once.age(0, 1.0).is_some());
        assert_eq!(once.age(0, 3.0), None);
        assert!(once.moves());
    }

    #[test]
    fn an_attached_objects_event_effect_moves_to_its_owner_and_a_function_of_its_own_does_not() {
        let (mut function, mut object) = ("firing effect: firing".to_owned(), 3);
        adopt_event_function("gun", &mut function, &mut object);
        assert_eq!((function.as_str(), object), ("gun: firing effect: firing", 0));
        assert!(is_event_function("gun: firing effect: firing"));
        let (mut function, mut object) = ("power".to_owned(), 3);
        adopt_event_function("gun", &mut function, &mut object);
        assert_eq!((function.as_str(), object), ("power", 3));
    }

    #[test]
    fn an_events_effect_is_named_by_its_field_and_tag() {
        assert_eq!(event_function_name("firing effect", "objects\\weapons\\fx\\firing"), "firing effect: firing");
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
