//! A dx9 pass that draws one digit of a number, by an object output.

/// A `numeric` shader: its first map holds digits, and which one draws comes
/// from an object output, as `rasterizer_dx9_transparent_chicago_draw` has it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CeNumeric {
    /// The first map, whose images are the digits.
    pub(in crate::app::model_preview) map: String,
    /// How many digits it holds, the base of the count.
    pub(in crate::app::model_preview) count: usize,
    /// `numeric counter limit`.
    pub(in crate::app::model_preview) limit: i32,
    /// The digit of this pass.
    pub(in crate::app::model_preview) digit: usize,
    /// The material's permutation: which place of the number it shows.
    pub(in crate::app::model_preview) place: usize,
    /// `numeric countdown timer`: it reads a clock only scripts set, not the
    /// object. A preview has no script, so that stays at its zero.
    pub(in crate::app::model_preview) timer: bool,
}

impl CeNumeric {
    fn number(&self, outputs: &[f32; 4]) -> i32 {
        // Eight images mean a different output from the usual ten.
        let output = outputs[if self.count == 8 { 3 } else { 0 }];
        ((output * self.limit as f32 + 0.5).floor() as i32).clamp(0, self.limit)
    }

    pub(crate) fn shows(&self, outputs: &[f32; 4]) -> bool {
        if self.timer {
            return self.digit == 0;
        }
        let base = self.count.max(1) as i32;
        let mut number = self.number(outputs);
        for _ in 0..self.place {
            number /= base;
        }
        (number % base) as usize == self.digit
    }
}
