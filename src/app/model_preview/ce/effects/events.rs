//! An effect's event state machine: which event runs, for how long, and with what inputs.

use super::*;

/// The panel's plays of the objects' event effects: how many times each of
/// the preview's effects has been played, by index (zero for those that are
/// not events), and the scales `effect_new_from_object` is given.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeEventPlays {
    pub(crate) counts: Vec<u32>,
    /// Scale A and scale B.
    pub(crate) scales: [f32; 2],
}

/// An effect particle's `distribution function`: how its count is let go
/// over the event. A field with no option is `start`; one whose number has
/// no name takes the last, as the game's switch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Distribution {
    Start,
    End,
    Constant,
    Buildup,
    Falloff,
    BuildupAndFalloff,
}

impl Distribution {
    pub(super) fn read(entry: TagStruct<'_>) -> Self {
        let Some(field) = entry.field("distribution function") else {
            return Self::Start;
        };
        match entry.read_enum_option("distribution function").as_deref() {
            Some("start") => Self::Start,
            Some("end") => Self::End,
            Some("constant") => Self::Constant,
            Some("buildup") => Self::Buildup,
            Some("falloff") => Self::Falloff,
            _ if field.value().is_none() => Self::Start,
            _ => Self::BuildupAndFalloff,
        }
    }
}

/// `effect_evaluate_function_integral`: the share of an event's particles
/// let go by `fraction` of it.
pub(super) fn integral(function: Distribution, fraction: f32) -> f32 {
    if fraction == -1.0 {
        return 0.0;
    }
    match function {
        Distribution::Start => 1.0,
        Distribution::End => {
            if fraction >= 1.0 {
                1.0
            } else {
                0.0
            }
        }
        Distribution::Constant => fraction,
        Distribution::Buildup => fraction * fraction,
        Distribution::Falloff => (2.0 - fraction) * fraction,
        Distribution::BuildupAndFalloff => (3.0 - 2.0 * fraction) * fraction * fraction,
    }
}

/// `effect_set_event`.
pub(super) fn set_event(running: &mut RunningEffect, def: &CeEffectDef, event: usize, random: &mut CeRandom) {
    if let Some(found) = def.events.get(event) {
        running.event = event;
        running.time = 0.0;
        running.in_event = false;
        running.duration = random.range(found.delay[0], found.delay[1]);
    }
}

/// `effect_update` for an effect `effect_new_looping` made.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_effect(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    inputs: &CeObjectInputs,
    made: Made<'_>,
    random: &mut CeRandom,
    bones: Option<&[[f32; 4]]>,
    dt: f32,
) {
    if let Some(spawn) = effect.spawn {
        // Made once, as the particle said: it runs its events and is done.
        [running.scale_a, running.scale_b] = spawn.scales;
        running.color = spawn.color;
    } else {
        attached_inputs(running, effect, inputs, random);
    }
    run_events(index, running, effect, made, random, bones, dt);
}

/// What an attached effect takes of its object each tick: on while its
/// primary scale is, restarting when it has run out, stopping when it goes
/// off.
fn attached_inputs(running: &mut RunningEffect, effect: &CeEffectAt, inputs: &CeObjectInputs, random: &mut CeRandom) {
    let def = &effect.def;
    // `object_get_function_value`: none is on at one.
    let (scale_a, on) = match effect.primary {
        Some(output) => (inputs.outputs[output], inputs.active[output]),
        None => (1.0, true),
    };
    running.scale_a = scale_a;
    if running.deleted {
        return;
    }
    if !on && def.deleted_when_off {
        // The object's slot for it is cleared too, and `attachments_new`
        // runs only as the object is made: it never comes back. What it
        // made lives on.
        running.finished = true;
        running.deleted = true;
        return;
    }
    if on {
        if running.finished {
            // `effect_update` (360 and MCC) clears only "finished": an effect
            // that once stopped by going on past its loop stays marked as
            // stopping, and its function going off never stops it again.
            running.finished = false;
            set_event(running, def, 0, random);
        }
    } else if !running.finished && !running.stopping {
        // `effect_stop`: on to what follows the loop, or done.
        match def.loop_stop.map(|stop| stop + 1).filter(|next| *next < def.events.len()) {
            Some(next) => {
                set_event(running, def, next, random);
                running.stopping = true;
            }
            None => running.finished = true,
        }
    }
    running.scale_b = effect.secondary.map_or(1.0, |output| inputs.outputs[output]);
    running.color = effect.change_color.map_or([1.0; 3], |channel| inputs.change_colors[channel]);
}

/// The second half of [`update_effect`]: the effect's events through the tick.
#[allow(clippy::too_many_arguments)]
fn run_events(
    index: usize,
    running: &mut RunningEffect,
    effect: &CeEffectAt,
    made: Made<'_>,
    random: &mut CeRandom,
    bones: Option<&[[f32; 4]]>,
    dt: f32,
) {
    let Made { particles, lights, systems, system_random, system_room, ground, tick } = made;
    let def = &effect.def;
    let mut dt = dt;
    for _ in 0..MOST_EVENTS_A_TICK {
        if dt < 0.0 || running.finished {
            break;
        }
        let ended;
        if running.duration - running.time > dt {
            running.time += dt;
            ended = false;
            dt = -1.0;
        } else {
            dt -= running.duration - running.time;
            running.time = running.duration;
            ended = true;
        }
        if running.in_event {
            generate_particles(index, running, effect, particles, random, bones);
            if ended {
                // `effect_update` loops back only for a looping effect, one
                // `effect_new_looping` made (flag 2): an attachment.
                let looping = effect.spawn.is_none();
                let mut next = match (def.loop_start, def.loop_stop) {
                    (Some(start), Some(stop)) if looping && running.event == stop && !running.stopping => start,
                    _ => running.event + 1,
                };
                while next < def.events.len() && random.next() < def.events[next].skip {
                    next += 1;
                }
                if next >= def.events.len() {
                    running.finished = true;
                    return;
                }
                set_event(running, def, next, random);
            }
        } else if ended {
            let event = &def.events[running.event];
            running.time = 0.0;
            running.in_event = true;
            running.last_fraction = -1.0;
            running.duration = random.range(event.duration[0], event.duration[1]);
            running.counts = event
                .particles
                .iter()
                .map(|entry| {
                    let [lower, delta] = scaled(entry.count, running, entry.scales_a.count, entry.scales_b.count);
                    event_particle_count(lower + random.range(0.0, delta))
                })
                .collect();
            generate_parts(index, running, effect, lights, tick);
            generate_systems(index, running, effect, systems, random, system_random, system_room, bones, ground);
        }
    }
}

/// `effect_update` keeps an event's particle count in a byte, cast from the
/// rolled count: truncated toward zero, then only its low eight bits, so 300
/// makes 44 and a count past 255 can make none.
pub(super) fn event_particle_count(rolled: f32) -> f32 {
    f32::from(rolled as i32 as u8)
}
