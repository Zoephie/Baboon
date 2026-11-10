//! A material's animated values and the clocks that drive them.

use super::FunctionValues;
use blam_tags::tag_function::FunctionType;

/// Which part of a constant an animation writes, as `update_constants` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gen3Animated {
    First,
    Second,
    ScaleUniform,
    Third,
    Fourth,
    Color,
    /// Which of a bitmap's images binds.
    Frame,
}

/// One of a shader's `animated parameters`.
#[derive(Debug)]
pub(crate) struct Gen3Animation {
    pub(crate) uniform: String,
    pub(crate) writes: Gen3Animated,
    pub(crate) function: blam_tags::TagFunction,
    pub(crate) input: String,
    pub(crate) range: String,
    pub(crate) period: f32,
    /// Which input names run on the clock, by game and shader.
    pub(crate) clock: Gen3Clock,
}

/// How `evaluate_function` reads an animated parameter's input when it names
/// the clock rather than an object function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Gen3Clock {
    /// Halo 3 (`evaluate_function`, March 2007 build): `unique_id` is time
    /// shifted by the object's index, so like objects run out of step.
    #[default]
    IndexShift,
    /// ODST (`sub_140946C30`): `unique_id` shifted by the object's index
    /// times 1.37317.
    ScaledIndexShift,
    /// An ODST screen shader (`sub_14096D0E0`): `time` and `render_time` are
    /// the clock as well.
    ScaledIndexScreen,
    /// Reach (MCC `sub_140816D50`, the 360's `evaluate_function`
    /// 0x826d8ce0): `unique_id` is the object's index times 1.37317, which
    /// only a periodic function adds to the clock; any other holds it still.
    PeriodicIndexShift,
}

impl Gen3Clock {
    /// Seconds `unique_id` runs ahead for the object of `owner`, or none if
    /// `input` is not a clock here. The game uses the object's datum index;
    /// the preview's owner (0 the opened object) stands in for it.
    fn phase(self, input: &str, owner: u8) -> Option<f32> {
        let owner = f32::from(owner);
        match (self, input) {
            (Self::IndexShift, "unique_id") => Some(owner),
            (
                Self::ScaledIndexShift | Self::ScaledIndexScreen | Self::PeriodicIndexShift,
                "unique_id",
            ) => Some(owner * 1.37317),
            (Self::ScaledIndexScreen, "time" | "render_time") => Some(0.0),
            _ => None,
        }
    }
}

impl Gen3Animation {
    /// `evaluate_function`: an object's named values, or else time around a
    /// period, which only a periodic function runs past 1.
    pub(super) fn sample(
        &self,
        time: f32,
        owner: u8,
        values: Option<&FunctionValues>,
    ) -> (f32, f32) {
        // `object_get_function_value`: no name at all reads full, so a parameter
        // with an input and no range runs on its function's upper curve.
        let named = |name: &str| {
            if name.is_empty() {
                return 1.0;
            }
            values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(0.0)
        };
        let phase = self.clock.phase(&self.input, owner);
        if phase.is_none() && (!self.input.is_empty() || !self.range.is_empty()) {
            return (named(&self.input), named(&self.range));
        }
        let periodic = self.function.function_type() == FunctionType::Periodic;
        let input = match (self.clock, phase) {
            (Gen3Clock::PeriodicIndexShift, Some(phase))
                if self.input == "unique_id" && !periodic =>
            {
                phase
            }
            _ => (time + phase.unwrap_or(0.0)) / self.period,
        };
        if periodic {
            (input, 0.0)
        } else {
            (input.rem_euclid(1.0), 0.0)
        }
    }

    /// Write this into `value`, the constant as it stands.
    pub(crate) fn apply(
        &self,
        value: &mut [f32; 4],
        time: f32,
        owner: u8,
        values: Option<&FunctionValues>,
    ) {
        let (input, range) = self.sample(time, owner, values);
        if self.writes == Gen3Animated::Color {
            let color = self.function.evaluate_color(input, range);
            value[..3].copy_from_slice(&[color.red, color.green, color.blue]);
            return;
        }
        let output = self.function.evaluate(input, range);
        match self.writes {
            Gen3Animated::ScaleUniform => value[..2].fill(output),
            Gen3Animated::First => value[0] = output,
            Gen3Animated::Second => value[1] = output,
            Gen3Animated::Third => value[2] = output,
            _ => value[3] = output,
        }
    }

    /// `render_method_submit_volatile_per_node`'s choice among `count` images.
    pub(crate) fn frame(
        &self,
        count: usize,
        time: f32,
        owner: u8,
        values: Option<&FunctionValues>,
    ) -> usize {
        let (input, range) = self.sample(time, owner, values);
        let frame = (self.function.evaluate(input, range) % count as f32).floor() as isize;
        frame.clamp(0, count as isize - 1) as usize
    }

    /// Whether this moves with no one touching the object.
    pub(crate) fn moves(&self) -> bool {
        self.input.is_empty() && self.range.is_empty() && self.function.as_constant().is_none()
    }
}
