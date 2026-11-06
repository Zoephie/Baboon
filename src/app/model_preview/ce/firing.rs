//! Halo CE: firing a weapon's trigger from the preview panel, as
//! `weapon_trigger_fire` does to the weapon. A shot plays one of the
//! trigger's firing effects and lights the inputs `weapon_export_function_values`
//! gives the weapon's functions: its rate of fire, the ejection port, the
//! illumination, heat, and `firing` for the tick it fires and the next.
//!
//! Each tick `weapon_update` cools the heat, then runs each trigger: the
//! ejection port and illumination fall by the rates tool bakes (`sub_4A5930`:
//! a tick over the recovery time, or the whole of it in a tick when the time is
//! under 0.0001), and a shot then sets them back to 1, so the shot's own tick
//! ends at 1. The preview holds the weapon as the game holds an actor's (no
//! player owns it): heat stops at the overheated threshold, so it never
//! overheats, and it never ages. The rate of fire is the panel's scale A,
//! which is also what the firing effect is played at, falling by the
//! trigger's deceleration once it stops.

use blam_tags::{TagFile, TagStruct};

use super::super::common::fields::{elements, has_flag, object_block};
use super::functions::TICKS_PER_SECOND;
use super::particles::{CeEffectAt, Random, STEP};

/// Below this a recovery time is none: tool bakes a whole tick's fall.
const LEAST_RECOVERY_TIME: f32 = 0.0001;

/// A trigger, as firing it reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CeTrigger {
    /// `+240`: how far the illumination falls in a tick, if a shot lights it.
    illumination_fall: Option<f32>,
    /// `+244`: how far the ejection port closes in a tick, if a shot opens
    /// it (not when it opens on its chamber animation's key frame).
    ejection_fall: Option<f32>,
    /// `+252`: how far the rate of fire falls in a tick.
    rate_fall: f32,
    heat_per_round: f32,
    /// `random firing effects`: picked at random, not in turn.
    random: bool,
    /// Its firing effects: the shot count bounds, and the effect's index
    /// among the preview's, if it has one to play.
    effects: Vec<([i16; 2], Option<usize>)>,
}

/// A weapon whose triggers the panel can fire.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CeWeaponFiring {
    /// 0 the opened weapon, 1 the one the opened unit holds.
    pub(crate) object: u8,
    pub(crate) triggers: Vec<CeTrigger>,
    overheated_threshold: f32,
    /// How far the heat falls in a tick.
    heat_fall: f32,
    heat_illumination: f32,
}

/// Tool's bake of a time into what falls in a tick.
fn fall_per_tick(seconds: f32) -> f32 {
    if seconds <= LEAST_RECOVERY_TIME { 1.0 } else { STEP / seconds }
}

/// The triggers of `weapon`, if it is one, with each firing effect found
/// among `effects`, the object's event effects, by its path.
pub(in crate::app::model_preview) fn read_ce_weapon_firing(weapon: &TagFile, object: u8, effects: &[CeEffectAt]) -> Option<CeWeaponFiring> {
    let holder = object_block(weapon, "triggers")?;
    let effect_index = |path: &str| {
        effects
            .iter()
            .position(|effect| effect.object == object && effect.event.as_ref().is_some_and(|event| event.path == path))
    };
    let triggers: Vec<CeTrigger> = elements(holder, "triggers")
        .into_iter()
        .map(|trigger| read_trigger(trigger, &effect_index))
        .collect();
    if triggers.is_empty() {
        return None;
    }
    let real = |name: &str| holder.read_real(name).unwrap_or(0.0);
    Some(CeWeaponFiring {
        object,
        triggers,
        overheated_threshold: real("overheated threshold"),
        heat_fall: real("heat loss per second") * STEP,
        heat_illumination: real("heat illumination"),
    })
}

fn read_trigger(trigger: TagStruct<'_>, effect_index: &impl Fn(&str) -> Option<usize>) -> CeTrigger {
    let real = |name: &str| trigger.read_real(name).unwrap_or(0.0);
    let short = |entry: TagStruct<'_>, name: &str| entry.read_int_any(name).unwrap_or(0) as i16;
    let recovery = |name: &str| Some(real(name)).filter(|seconds| *seconds > 0.0).map(fall_per_tick);
    let effects = elements(trigger, "firing effects")
        .into_iter()
        .map(|entry| {
            let bounds = [short(entry, "shot count lower bound"), short(entry, "shot count upper bound")];
            let path = entry.read_tag_ref_path("firing effect").filter(|path| !path.is_empty());
            (bounds, path.and_then(|path| effect_index(&path)))
        })
        .collect();
    CeTrigger {
        illumination_fall: recovery("illumination recovery time"),
        ejection_fall: recovery("ejection port recovery time").filter(|_| !has_flag(trigger, "flags", "ejects during chamber")),
        // The first of the trigger's two: the rate of fire's, not the error's.
        rate_fall: fall_per_tick(real("deceleration time")),
        heat_per_round: real("heat generated per round"),
        random: has_flag(trigger, "flags", "random firing effects"),
        effects,
    }
}

/// A trigger's state between shots: `weapon_trigger_fire`'s choice of firing
/// effect, and its last shot.
#[derive(Debug, Clone, Copy, Default)]
struct TriggerShots {
    /// Firing effects used since all last were (`+616`).
    used: u16,
    /// The firing effect in use (`+618`), and its shots left (`+620`).
    current: i16,
    left: i16,
    /// The tick of its last shot, and the rate of fire it had.
    last: Option<(i64, f32)>,
}

/// What the panel's shots have done to one weapon.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeWeaponShots {
    triggers: Vec<TriggerShots>,
    /// The heat at the tick of the last shot.
    heat: Option<(i64, f32)>,
    /// The seed of the random choices.
    random: Random,
}

/// Shots fired from the panel, by model and object.
pub(crate) type CeShots = std::collections::HashMap<(u64, u8), CeWeaponShots>;

/// The tick `time` falls in.
fn tick_of(time: f32) -> i64 {
    (time * TICKS_PER_SECOND).floor() as i64
}

impl CeWeaponShots {
    /// `weapon_trigger_fire` for `trigger` at `time`, at the rate of fire
    /// `rate`: the firing effect it plays, by index among the preview's.
    pub(crate) fn fire(&mut self, weapon: &CeWeaponFiring, trigger: usize, rate: f32, time: f32) -> Option<usize> {
        let definition = weapon.triggers.get(trigger)?;
        self.triggers.resize(weapon.triggers.len(), TriggerShots::default());
        let now = tick_of(time);
        // The heat cools to this tick, then the shot adds its own, held as
        // an actor's weapon is, under the overheated threshold.
        let heat = self.heat_at(weapon, now) + definition.heat_per_round;
        self.heat = Some((now, if heat > weapon.overheated_threshold { weapon.overheated_threshold } else { heat.min(1.0) }));
        let state = &mut self.triggers[trigger];
        state.last = Some((now, rate));
        let count = definition.effects.len() as i32;
        if count == 0 {
            return None;
        }
        if state.left <= 0 {
            let start = state.current;
            let mut index = if definition.random { self.random.bits() as i32 % count } else { i32::from(start) };
            let full = ((1u32 << count) - 1) as u16;
            loop {
                if state.used == full {
                    state.used = 0;
                }
                loop {
                    index = (index + 1) % count;
                    if state.used & (1 << index) == 0 {
                        break;
                    }
                }
                state.used |= 1 << index;
                state.current = index as i16;
                let [lower, upper] = definition.effects[index as usize].0;
                // `seed_random_range`: from the lower bound, short of the upper.
                let span = i32::from(upper) - i32::from(lower);
                state.left = ((span * self.random.bits() as i32) >> 16) as i16 + lower;
                if state.left > 0 || state.current == start {
                    break;
                }
            }
        }
        state.left -= 1;
        definition.effects.get(state.current as usize)?.1
    }

    /// The heat at tick `now`: the last shot's, cooled by every tick since.
    /// A shot after `now`, from before the preview's clock wrapped, is long
    /// cold.
    fn heat_at(&self, weapon: &CeWeaponFiring, now: i64) -> f32 {
        self.heat
            .filter(|(tick, _)| *tick <= now)
            .map_or(0.0, |(tick, heat)| (heat - weapon.heat_fall * (now - tick) as f32).max(0.0))
    }

    /// What `weapon_export_function_values` gives the input `name` at
    /// `time`, if the shots have made it more than nothing.
    pub(crate) fn input(&self, weapon: &CeWeaponFiring, name: &str, time: f32) -> Option<f32> {
        let now = tick_of(time);
        // Since the trigger's last shot, in ticks, and its rate then.
        let since = |trigger: usize| {
            let (tick, rate) = self.triggers.get(trigger)?.last.filter(|(tick, _)| *tick <= now)?;
            Some(((now - tick) as f32, rate))
        };
        let falling = |trigger: usize, fall: Option<f32>| -> f32 {
            match (since(trigger), fall) {
                (Some((ticks, _)), Some(fall)) => (1.0 - ticks * fall).max(0.0),
                _ => 0.0,
            }
        };
        let rate = |trigger: usize| -> f32 {
            let definition = weapon.triggers.get(trigger);
            match (since(trigger), definition) {
                (Some((ticks, rate)), Some(definition)) => (rate - ticks * definition.rate_fall).max(0.0),
                _ => 0.0,
            }
        };
        // `+720` is the weapon's: either trigger's shot counts.
        let fired_lately = (0..self.triggers.len()).filter_map(since).any(|(ticks, _)| ticks <= 1.0);
        let fall = |trigger: usize, pick: fn(&CeTrigger) -> Option<f32>| weapon.triggers.get(trigger).and_then(pick);
        let value = match name {
            "heat" => self.heat_at(weapon, now),
            "primary rate of fire" | "primary firing on" => rate(0),
            "secondary rate of fire" | "secondary firing on" => rate(1),
            "primary firing" => if fired_lately { rate(0) } else { 0.0 },
            "secondary firing" => if fired_lately { rate(1) } else { 0.0 },
            "primary ejection port" => falling(0, fall(0, |trigger| trigger.ejection_fall)),
            "secondary ejection port" => falling(1, fall(1, |trigger| trigger.ejection_fall)),
            "illumination" => (0..weapon.triggers.len())
                .map(|trigger| falling(trigger, fall(trigger, |trigger| trigger.illumination_fall)))
                .fold(self.heat_at(weapon, now) * weapon.heat_illumination, f32::max),
            _ => return None,
        };
        (value > 0.0).then_some(value)
    }

    /// Whether anything a shot did still changes with time.
    pub(crate) fn moving(&self, weapon: &CeWeaponFiring, time: f32) -> bool {
        const MOVING: [&str; 6] =
            ["heat", "illumination", "primary rate of fire", "secondary rate of fire", "primary ejection port", "secondary ejection port"];
        MOVING.iter().any(|name| self.input(weapon, name, time).is_some())
    }
}

/// What the panel's shots give `object`'s inputs at `time`, by name.
pub(in crate::app::model_preview) fn fired_input<'a>(
    firing: &'a [CeWeaponFiring],
    shots: &'a CeShots,
    geometry: u64,
    object: u8,
    time: f32,
) -> impl Fn(&str) -> Option<f32> + 'a {
    let weapon = firing.iter().find(|weapon| weapon.object == object);
    let fired = shots.get(&(geometry, object));
    move |name| fired?.input(weapon?, name, time)
}

/// Whether any shot fired on `geometry` still changes its weapon's inputs.
pub(in crate::app::model_preview) fn shots_moving(firing: &[CeWeaponFiring], shots: &CeShots, geometry: u64, time: f32) -> bool {
    firing.iter().any(|weapon| shots.get(&(geometry, weapon.object)).is_some_and(|fired| fired.moving(weapon, time)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle(effects: Vec<([i16; 2], Option<usize>)>, random: bool) -> CeWeaponFiring {
        CeWeaponFiring {
            object: 1,
            triggers: vec![CeTrigger {
                illumination_fall: Some(fall_per_tick(0.1)),
                ejection_fall: Some(fall_per_tick(0.2)),
                rate_fall: fall_per_tick(0.5),
                heat_per_round: 0.0,
                random,
                effects,
            }],
            overheated_threshold: 1.0,
            heat_fall: 0.0,
            heat_illumination: 0.0,
        }
    }

    /// The shot's tick ends with the illumination at 1; it falls a tick over
    /// its recovery time each tick after, and `firing` lasts the shot's tick
    /// and the next.
    #[test]
    fn a_shot_lights_the_weapon_and_lets_it_fall() {
        let weapon = rifle(vec![([1, 1], Some(7))], false);
        let mut shots = CeWeaponShots::default();
        assert_eq!(shots.input(&weapon, "illumination", 10.0), None, "unfired, nothing");
        assert_eq!(shots.fire(&weapon, 0, 1.0, 10.0), Some(7));
        let at = |ticks: f32| 10.0 + ticks * STEP;
        assert_eq!(shots.input(&weapon, "illumination", at(0.5)), Some(1.0));
        let after_one = shots.input(&weapon, "illumination", at(1.5)).unwrap();
        assert!((after_one - (1.0 - 1.0 / 3.0)).abs() < 1e-5, "{after_one}");
        assert_eq!(shots.input(&weapon, "illumination", at(3.5)), None, "dark in three ticks");
        assert_eq!(shots.input(&weapon, "primary firing", at(1.5)), Some(1.0 - fall_per_tick(0.5)));
        assert_eq!(shots.input(&weapon, "primary firing", at(2.5)), None);
        assert!(shots.input(&weapon, "primary ejection port", at(3.5)).is_some(), "the port closes slower");
    }

    /// In turn, the first pick steps on from the first entry, as the game
    /// starts its index at 0 and moves before it picks, and an entry is used
    /// for its shot count before the next.
    #[test]
    fn firing_effects_are_used_in_turn_for_their_shot_counts() {
        let weapon = rifle(vec![([1, 1], Some(0)), ([2, 2], Some(1)), ([1, 1], Some(2))], false);
        let mut shots = CeWeaponShots::default();
        let picks: Vec<_> = (0..6).map(|shot| shots.fire(&weapon, 0, 1.0, shot as f32).unwrap()).collect();
        assert_eq!(picks, [1, 1, 2, 0, 1, 1]);
    }

    /// Heat adds up shot by shot, cools between, and an actor's weapon stops
    /// at the overheated threshold.
    #[test]
    fn heat_stops_at_the_overheated_threshold() {
        let mut weapon = rifle(vec![([1, 1], None)], false);
        weapon.triggers[0].heat_per_round = 0.3;
        weapon.overheated_threshold = 0.8;
        weapon.heat_fall = 0.01;
        let mut shots = CeWeaponShots::default();
        shots.fire(&weapon, 0, 1.0, 0.0);
        shots.fire(&weapon, 0, 1.0, 10.0 * STEP);
        let heat = shots.input(&weapon, "heat", 10.5 * STEP).unwrap();
        assert!((heat - (0.3 - 0.1 + 0.3)).abs() < 1e-5, "{heat}");
        for shot in 11..20 {
            shots.fire(&weapon, 0, 1.0, shot as f32 * STEP);
        }
        assert_eq!(shots.input(&weapon, "heat", 19.5 * STEP), Some(0.8));
    }
}
