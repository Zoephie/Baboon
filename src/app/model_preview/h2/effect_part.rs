//! The effect parts Halo 2's lights and light volumes share: their timing and the lights they bring.

use std::sync::Arc;
use super::super::functions::FunctionValues;
use super::super::effects::EffectTimes;
use blam_tags::tag_function::h2::transition_evaluate;
use crate::app::model_preview::h2::lights::{H2Light, attachment_value};

/// A light an effect's event makes, `sub_8EBD10`: it stands from when its
/// event begins. One with a `duration` dims by its falloff, `sub_8ED2A0`, and
/// is deleted after it, `sub_8EE2E0`; without, nothing deletes it.
#[derive(Debug, Clone)]
pub(crate) struct H2EffectLight {
    times: Arc<EffectTimes>,
    event: usize,
    /// `A scales values` takes the type-specific bit: `sub_515FC0` scales it
    /// by the effect's A scale, which the attachment's function gives.
    scaled: bool,
}

impl H2EffectLight {
    pub(crate) fn new(times: Arc<EffectTimes>, event: usize, scaled: bool) -> Self {
        Self { times, event, scaled }
    }

    /// The scale `sub_8ED2A0` reads, or none while the light does not exist.
    /// Without a duration it takes the upper colors and brightness whatever
    /// the scale, and only its size follows; that differs only when the effect
    /// scales it below 1.
    pub(super) fn value(&self, light: &H2Light, on: f32, time: f32, onset: f32) -> Option<f32> {
        if on <= 0.0 {
            return None;
        }
        let since = self.times.since_begun(self.event, time - onset)?;
        let scale = if self.scaled { on } else { 1.0 };
        if light.effect_duration <= 0.0 {
            return Some(scale);
        }
        if since > light.effect_duration {
            return None;
        }
        let faded = transition_evaluate(light.effect_falloff, since / light.effect_duration);
        Some(scale * (1.0 - faded))
    }
}

/// An event's `lens_flare` or `light_volume` part on an attachment's effect,
/// which `sub_515280` draws at each of its location's markers while its event
/// runs (`render_submit_special_model_groups`), not `effect_generate_part`.
#[derive(Debug, Clone)]
pub(crate) struct H2EffectPart {
    times: Arc<EffectTimes>,
    event: usize,
    /// `A scales values` and `B scales values` take the type-specific bit:
    /// the effect's A and B scales brighten it.
    scaled: [bool; 2],
    /// The effect's B scale, a name the object's functions answer to.
    secondary_by: String,
}

/// How `sub_515280` lights a part this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2PartLight {
    /// The A and B scales it takes.
    pub(crate) brightness: f32,
    /// `1 - age / duration`, for an event longer than a tick.
    pub(crate) fade: f32,
    /// Whether its event has a duration; one of none neither fades nor falls off.
    pub(crate) lasts: bool,
}

/// `sub_515280` fades a part over an event longer than this, or at its end.
const SHORTEST_FADING_EVENT: f32 = 0.033_433_337;

impl H2EffectPart {
    pub(in crate::app::model_preview) fn new(part: &super::super::effects::EffectPart<'_>) -> Self {
        Self {
            times: Arc::clone(&part.times),
            event: part.event,
            scaled: [part.scaled, part.b_scaled],
            secondary_by: part.effect.secondary_by.clone(),
        }
    }

    /// How it is lit at `time`, with its effect's A scale `primary` names; none
    /// while the effect is off or another of its events runs.
    pub(crate) fn light(
        &self,
        primary: &str,
        values: Option<&FunctionValues>,
        time: f32,
        onset: f32,
    ) -> Option<H2PartLight> {
        let on = attachment_value(primary, values);
        if on <= 0.0 {
            return None;
        }
        let age = self.times.age(self.event, time - onset)?;
        let duration = self.times.duration(self.event)?;
        let secondary = attachment_value(&self.secondary_by, values);
        let [a, b] = self.scaled;
        let brightness = if a { on } else { 1.0 } * if b { secondary } else { 1.0 };
        let fade = if duration > SHORTEST_FADING_EVENT { (1.0 - age).clamp(0.0, 1.0) } else { 1.0 };
        Some(H2PartLight { brightness, fade, lasts: duration != 0.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sub_515280`: a part fades out over its event, and takes the A and B
    /// scales only where its flags ask; with its effect off it is not drawn.
    #[test]
    fn an_effect_part_fades_over_its_event_and_takes_the_scales_it_asks_for() {
        let part = |scaled: [bool; 2]| H2EffectPart {
            times: Arc::new(EffectTimes::read_nothing()),
            event: 0,
            scaled,
            secondary_by: "power".to_owned(),
        };
        let values: FunctionValues = [("power".to_owned(), 0.5), ("on".to_owned(), 0.8)].into_iter().collect();
        let lit = |scaled, time| part(scaled).light("on", Some(&values), time, 0.0);
        assert_eq!(lit([false; 2], 0.0), Some(H2PartLight { brightness: 1.0, fade: 1.0, lasts: true }));
        assert_eq!(lit([true, false], 50.0).map(|lit| (lit.brightness, lit.fade)), Some((0.8, 0.5)));
        assert_eq!(lit([true, true], 0.0).map(|lit| lit.brightness), Some(0.4));
        let off: FunctionValues = [("on".to_owned(), 0.0)].into_iter().collect();
        assert_eq!(part([false; 2]).light("on", Some(&off), 0.0, 0.0), None, "its effect is off");
    }
}
