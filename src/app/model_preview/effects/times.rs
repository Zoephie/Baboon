//! When an effect's events and parts run: their times and the onsets the preview keeps.

use super::{FunctionValues, elements, has_flag};
use blam_tags::TagStruct;

/// `effect_build_locations`: a location of this name takes the attachment's marker.
pub(super) const REPLACE: &str = "replace";

pub(super) const ONLY_FIRST_PERSON: &str = "only in first person";

pub(super) const ONLY_NONVIOLENT: &str = "nonviolent mode only";

pub(super) const ANYWHERE: &str = "any environment";

pub(super) const AIR: &str = "air only";

/// `_effect_part_type_scale_bit`, by its option's name.
pub(super) const PART_SCALE: &str = "type-specific scale";

pub(super) const DISABLED: &str = "disabled for debugging";

/// `k_maximum_effect_location_markers`.
pub(in crate::app::model_preview) const MOST_MARKERS: usize = 16;

/// `effect_compute_event_runtime_data`'s shortest duration, a tick.
pub(super) const ONE_TICK: f32 = 1.0 / 60.0;

const PARALLEL: &str = "run events in parallel";

/// When an event's delay and duration run, in seconds. A bound the game rolls
/// takes its middle, so that a preview holds still from one load to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EventTime {
    delay: f32,
    pub(super) duration: f32,
}

impl EventTime {
    fn read(event: TagStruct<'_>, baked: bool) -> Self {
        let delay = event.read_real_bounds("delay bounds");
        let duration = event.read_real_bounds("duration bounds");
        let (delay, duration) = ([delay.lower, delay.upper], [duration.lower, duration.upper]);
        if baked {
            Self::of(delay, duration)
        } else {
            Self::unbaked(delay, duration)
        }
    }

    /// The bake: a duration that can be 0 is a tick, which also flattens a
    /// random one starting at 0.
    pub(super) fn of(delay: [f32; 2], duration: [f32; 2]) -> Self {
        let baked = duration[0] <= 0.0 && duration[1] >= 0.0;
        Self {
            delay: (delay[0] + delay[1]) * 0.5,
            duration: if baked {
                ONE_TICK
            } else {
                (duration[0] + duration[1]) * 0.5
            },
        }
    }

    /// Reach's `fix_events` (the 360's 0x830844c8) is an empty loop: a
    /// duration rolls as authored, and one of 0 ends in the update it began,
    /// the tick it stands for here.
    pub(super) fn unbaked(delay: [f32; 2], duration: [f32; 2]) -> Self {
        let duration = (duration[0] + duration[1]) * 0.5;
        Self {
            delay: (delay[0] + delay[1]) * 0.5,
            duration: if duration > 0.0 { duration } else { ONE_TICK },
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
pub(in crate::app::model_preview) struct EffectTimes {
    pub(super) events: Vec<EventTime>,
    pub(super) loop_start: usize,
    pub(super) parallel: bool,
    /// Over after one run, as an event's effect is.
    pub(super) once: bool,
}

impl EffectTimes {
    pub(in crate::app::model_preview) fn read(effect: TagStruct<'_>) -> Self {
        // Reach's effect gathers its looping sound into `looping sounds`, which
        // Halo 3's and ODST's lack; only Reach's postprocess leaves durations
        // as authored.
        let baked = effect.field("looping sounds").is_none();
        Self {
            events: elements(effect, "events")
                .iter()
                .map(|event| EventTime::read(*event, baked))
                .collect(),
            loop_start: usize::try_from(effect.read_block_index("loop start event")).unwrap_or(0),
            parallel: has_flag(effect, "flags", PARALLEL),
            once: false,
        }
    }

    /// `event_update_time`: event `index`'s age at `time`, or none while
    /// something else runs. Parallel events restart once all have finished.
    /// Otherwise one follows another, returning to the loop start.
    pub(in crate::app::model_preview) fn age(&self, index: usize, time: f32) -> Option<f32> {
        let event = *self.events.get(index)?;
        if self.once && !(0.0..self.once()).contains(&time) {
            return None;
        }
        if self.parallel {
            let cycle = self
                .events
                .iter()
                .map(|event| event.length())
                .fold(0.0, f32::max);
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
        let mut left = if time < first {
            time
        } else {
            (time - first).rem_euclid(cycle.max(ONE_TICK))
        };
        for (at, candidate) in self.events.iter().enumerate().skip(start) {
            if left < candidate.length() {
                return (at == index).then(|| candidate.age(left)).flatten();
            }
            left -= candidate.length();
        }
        None
    }

    /// Event `index`'s duration in seconds, with the bake of one that can be 0
    /// (a tick) given back as the 0 it stands for.
    pub(in crate::app::model_preview) fn duration(&self, index: usize) -> Option<f32> {
        let event = self.events.get(index)?;
        Some(if event.duration <= ONE_TICK {
            0.0
        } else {
            event.duration
        })
    }

    /// Seconds since `index` began, ended or not, for what outlives its event.
    pub(in crate::app::model_preview) fn since_begun(
        &self,
        index: usize,
        time: f32,
    ) -> Option<f32> {
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events
                .iter()
                .take(index)
                .map(|event| event.length())
                .sum()
        };
        let cycle = self.once();
        let time = if self.once || cycle <= 0.0 {
            time
        } else {
            time.rem_euclid(cycle.max(ONE_TICK))
        };
        let age = time - before - event.delay;
        (age >= 0.0).then_some(age)
    }

    /// A run that never ends of itself, for what something else times.
    pub(in crate::app::model_preview) fn lasting() -> Self {
        Self {
            events: vec![EventTime {
                delay: 0.0,
                duration: f32::MAX,
            }],
            loop_start: 0,
            parallel: false,
            once: false,
        }
    }

    /// Events of these `(delay, duration)` bounds, one after another, as
    /// Reach's postprocess leaves them (`reach`) or Halo 3's.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn of_events(
        events: &[([f32; 2], [f32; 2])],
        reach: bool,
    ) -> Self {
        let time = |&(delay, duration): &([f32; 2], [f32; 2])| {
            if reach {
                EventTime::unbaked(delay, duration)
            } else {
                EventTime::of(delay, duration)
            }
        };
        Self {
            events: events.iter().map(time).collect(),
            loop_start: 0,
            parallel: false,
            once: false,
        }
    }

    #[cfg(test)]
    pub(in crate::app::model_preview) fn read_nothing() -> Self {
        Self {
            events: vec![EventTime::of([0.0, 0.0], [100.0, 100.0])],
            loop_start: 0,
            parallel: false,
            once: false,
        }
    }

    /// Whether a run that does not loop has finished.
    pub(in crate::app::model_preview) fn is_over(&self, time: f32) -> bool {
        self.once && time >= self.once()
    }

    /// How long a run that does not loop takes.
    pub(in crate::app::model_preview) fn once(&self) -> f32 {
        let lengths = self.events.iter().map(|event| event.length());
        if self.parallel {
            lengths.fold(0.0, f32::max)
        } else {
            lengths.sum()
        }
    }

    /// A time inside `index`, if it first begins after `from` and by `to`: a
    /// step can be longer than a whole event, and must not pass over it. Half a
    /// tick in, as rounding can put its very start before it.
    pub(in crate::app::model_preview) fn begins_within(
        &self,
        index: usize,
        from: f32,
        to: f32,
    ) -> Option<f32> {
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events
                .iter()
                .take(index)
                .map(|event| event.length())
                .sum()
        };
        let begin = before + event.delay;
        (from < begin && begin <= to).then_some(begin + ONE_TICK * 0.5)
    }

    /// Seconds since `index` began, for what outlasts its normalized age. An
    /// effect that only ticks over has been running all along.
    pub(in crate::app::model_preview) fn elapsed(&self, index: usize, time: f32) -> Option<f32> {
        self.age(index, time)?;
        if !self.moves() {
            return Some(time);
        }
        let event = *self.events.get(index)?;
        let before: f32 = if self.parallel {
            0.0
        } else {
            self.events
                .iter()
                .take(index)
                .map(|event| event.length())
                .sum()
        };
        let cycle: f32 = if self.parallel {
            self.events
                .iter()
                .map(|event| event.length())
                .fold(0.0, f32::max)
        } else {
            self.events.iter().map(|event| event.length()).sum()
        };
        Some((time.rem_euclid(cycle.max(ONE_TICK)) - before - event.delay).max(0.0))
    }

    /// Whether the event's age or its being on changes as time passes.
    pub(in crate::app::model_preview) fn moves(&self) -> bool {
        self.once
            || self.events.len() > 1
            || self
                .events
                .iter()
                .any(|event| event.delay > 0.0 || event.duration > ONE_TICK)
    }
}

/// When each attachment's function last came on: `attachments_update` makes
/// the effect then, so its events run from there. What was on from the first
/// frame has run all along.
#[derive(Debug, Clone, Default)]
pub(crate) struct EffectOnsets(std::collections::BTreeMap<(u8, String), (bool, f32)>);

impl EffectOnsets {
    pub(crate) fn of(
        &mut self,
        object: u8,
        name: &str,
        values: Option<&FunctionValues>,
        time: f32,
    ) -> f32 {
        if name.is_empty() {
            return 0.0;
        }
        let on = values
            .and_then(|values| values.get(name))
            .is_some_and(|value| *value > 0.0);
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
