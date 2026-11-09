//! A pass's animated values: their kinds, clocks and rows, and the readers they share.

use super::{DX9_TEXTURE_ROWS, FunctionValues, tag_function_at};
use blam_tags::tag_function::FunctionType;
use blam_tags::{TagFunction, TagStruct};
use std::sync::Arc;

/// Whether a pass draws while the object's outputs hold, for a game's
/// pass that shows only at some values: Halo CE's numeric digits.
pub(crate) trait PassVisibility: std::fmt::Debug + Send + Sync {
    fn shows(&self, outputs: &[f32; 4]) -> bool;
    /// The game's own value, for its reader to look back into.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// What a game's own shader animates on a pass past the template's
/// animations, evaluated at draw: Halo CE's rows and constants.
pub(crate) trait PassAnimation: std::fmt::Debug + Send + Sync {
    /// Whether it looks different from one moment to the next.
    fn moves(&self) -> bool;
    /// Whether it moves by itself while `outputs` hold.
    fn moves_with(&self, outputs: &[f32; 4]) -> bool;
    /// Which of the object's outputs it reads.
    fn outputs_read(&self) -> [bool; 4];
    /// Whether anything here follows an object output.
    fn reads_outputs(&self) -> bool;
    /// The texture rows, reworked for `time` and `outputs`.
    fn apply_rows(&self, rows: &mut [[f32; 4]; DX9_TEXTURE_ROWS], time: f32, outputs: &[f32; 4]);
    /// The value constant `register` holds, if it animates it.
    fn constant(
        &self,
        register: u32,
        value: &mut [f32; 4],
        time: f32,
        change_colors: &[[f32; 3]; 4],
        outputs: &[f32; 4],
    );
    /// The constant that holds the way back along the view.
    fn eye_register(&self) -> Option<u32>;
}

/// What one of a shader parameter's `animation properties` drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dx9AnimationKind {
    ScaleUniform,
    ScaleX,
    ScaleY,
    ScaleZ,
    TranslationX,
    TranslationY,
    TranslationZ,
    Rotation,
    Value,
    Color,
}

/// A parameter function the game reads by time, as in
/// `shader_setup_evaluate_overlay_animation`.
#[derive(Debug, Clone)]
pub(crate) struct Dx9Animation {
    pub(in crate::app::model_preview) kind: Dx9AnimationKind,
    /// Seconds to a cycle. At zero the function reads the time itself.
    pub(in crate::app::model_preview) period: f32,
    /// The object function this reads instead of time, and what it reads
    /// with no such function.
    pub(in crate::app::model_preview) held_input: Option<(String, f32)>,
    /// `range name`: the object function the range reads, and what it reads
    /// with no such function. Unnamed, `object_get_function_value` gives 1.
    pub(in crate::app::model_preview) range_input: Option<(String, f32)>,
    pub(in crate::app::model_preview) function: Arc<TagFunction>,
}

impl PartialEq for Dx9Animation {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.period == other.period
            && self.held_input == other.held_input
            && self.range_input == other.range_input
            && self.function.to_bytes() == other.function.to_bytes()
    }
}

/// When an animation reads, and the object functions it can read from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dx9Clock<'a> {
    pub(crate) time: f32,
    pub(crate) values: Option<&'a FunctionValues>,
}

impl From<f32> for Dx9Clock<'_> {
    fn from(time: f32) -> Self {
        Self { time, values: None }
    }
}

impl Dx9Animation {
    /// `sub_915520`: the named function's value, or with no name the time
    /// over the period, wrapped unless the function is periodic. The game
    /// divides by a period of 0 as well, which leaves it NaN; the preview
    /// reads the time as it is instead.
    pub(super) fn input(&self, clock: Dx9Clock<'_>) -> f32 {
        match &self.held_input {
            Some((name, rest)) => clock
                .values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(*rest),
            None if self.period > 0.0 => {
                let input = clock.time / self.period;
                if self.function.function_type() == FunctionType::Periodic {
                    input
                } else {
                    input.rem_euclid(1.0)
                }
            }
            None => clock.time,
        }
    }

    fn range(&self, clock: Dx9Clock<'_>) -> f32 {
        match &self.range_input {
            Some((name, rest)) => clock
                .values
                .and_then(|values| values.get(name))
                .copied()
                .unwrap_or(*rest),
            None => 1.0,
        }
    }

    fn scalar<'a>(&self, clock: impl Into<Dx9Clock<'a>>) -> f32 {
        let clock = clock.into();
        self.function.evaluate(self.input(clock), self.range(clock))
    }

    fn vector<'a>(&self, clock: impl Into<Dx9Clock<'a>>) -> [f32; 4] {
        let clock = clock.into();
        match self.kind {
            Dx9AnimationKind::Color => {
                let color = self
                    .function
                    .evaluate_color(self.input(clock), self.range(clock));
                [color.red, color.green, color.blue, 1.0]
            }
            _ => [self.scalar(clock); 4],
        }
    }

    /// Whether this shows: a transform always can, but a value or color may
    /// wander by less than a step of the frame.
    pub(super) fn moves(&self) -> bool {
        if self.held_input.is_some() || self.function.as_constant().is_some() {
            return false;
        }
        if self.is_transform() {
            return true;
        }
        const SAMPLES: usize = 16;
        const VISIBLE: f32 = 2.0 / 255.0;
        let span = if self.period > 0.0 { self.period } else { 1.0 };
        let first = self.vector(0.0);
        (1..SAMPLES).any(|sample| {
            let vector = self.vector(span * sample as f32 / SAMPLES as f32);
            vector
                .iter()
                .zip(first)
                .any(|(value, first)| (value - first).abs() >= VISIBLE)
        })
    }

    pub(in crate::app::model_preview) fn is_transform(&self) -> bool {
        !matches!(self.kind, Dx9AnimationKind::Value | Dx9AnimationKind::Color)
    }
}

/// The last value or color of a parameter's animations, as they overwrite
/// one another in the game.
pub(in crate::app::model_preview) fn animated_vector(
    animations: &[Dx9Animation],
    clock: Dx9Clock<'_>,
) -> Option<[f32; 4]> {
    animations
        .iter()
        .rev()
        .find(|animation| !animation.is_transform())
        .map(|last| last.vector(clock))
}

/// The bytes of a `mapping_function`, as the engine's H2 function.
pub(in crate::app::model_preview) fn animation_function(
    property: &TagStruct<'_>,
) -> Option<TagFunction> {
    tag_function_at(property, "function")
}

/// How a pass holds a texture transform in one register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dx9RowForm {
    Row1,
    Row2,
    /// The last of a 3D map's three: `xyzw row3 (3D affine xform)`.
    Row3,
    SimpleXform,
    ScaleXy,
    ScaleZw,
}

/// A texture transform to rebuild each frame.
#[derive(Debug, Clone)]
pub(crate) struct Dx9AnimatedRow {
    pub(in crate::app::model_preview) register: usize,
    pub(in crate::app::model_preview) form: Dx9RowForm,
    pub(in crate::app::model_preview) scale: [f32; 3],
    pub(in crate::app::model_preview) animations: Vec<Dx9Animation>,
}

impl Dx9AnimatedRow {
    /// `sub_6D8B60`, which turns about the middle of the map. Scale reaches
    /// the turn and not the translation, which `sub_9185D0` writes straight
    /// into its register each frame.
    pub(super) fn apply(&self, rows: &mut [[f32; 4]; DX9_TEXTURE_ROWS], clock: Dx9Clock<'_>) {
        let [mut scale_x, mut scale_y, mut scale_z] = self.scale;
        let (mut move_x, mut move_y, mut move_z, mut angle) = (0.0, 0.0, 0.0, 0.0);
        for animation in &self.animations {
            let value = animation.scalar(clock);
            match animation.kind {
                Dx9AnimationKind::ScaleUniform => {
                    (scale_x, scale_y, scale_z) = (value, value, value)
                }
                Dx9AnimationKind::ScaleX => scale_x = value,
                Dx9AnimationKind::ScaleY => scale_y = value,
                Dx9AnimationKind::ScaleZ => scale_z = value,
                Dx9AnimationKind::TranslationX => move_x = value,
                Dx9AnimationKind::TranslationY => move_y = value,
                Dx9AnimationKind::TranslationZ => move_z = value,
                Dx9AnimationKind::Rotation => angle = value,
                Dx9AnimationKind::Value | Dx9AnimationKind::Color => {}
            }
        }
        let (sin, cos): (f32, f32) = f32::sin_cos(angle);
        let offset_x = (sin + 1.0 - cos) * 0.5 * scale_x + move_x;
        let offset_y = (1.0 - cos - sin) * 0.5 * scale_y + move_y;
        let Some(row) = rows.get_mut(self.register) else {
            return;
        };
        match self.form {
            Dx9RowForm::Row1 => *row = [cos * scale_x, -sin * scale_x, 0.0, offset_x],
            Dx9RowForm::Row2 => *row = [sin * scale_y, cos * scale_y, 0.0, offset_y],
            Dx9RowForm::Row3 => *row = [0.0, 0.0, scale_z, move_z],
            Dx9RowForm::SimpleXform => *row = [cos * scale_x, cos * scale_y, offset_x, offset_y],
            Dx9RowForm::ScaleXy => (row[0], row[1]) = (cos * scale_x, cos * scale_y),
            Dx9RowForm::ScaleZw => (row[2], row[3]) = (cos * scale_x, cos * scale_y),
        }
    }
}

/// Animations of a tint, with the components they write.
pub(in crate::app::model_preview) type Dx9AnimatedTint = ([bool; 4], Vec<Dx9Animation>);
