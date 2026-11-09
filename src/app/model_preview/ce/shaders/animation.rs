//! A shader's per-frame animation, which the dx9 pass carries and evaluates at draw: maps, colours, meters, plasma, and their readers.

use super::base::option_index;
use super::periodic::{PERIODIC_ONE, PERIODIC_ZERO, periodic_evaluate};
use super::{H2_TEXTURE_ROWS, PassAnimation};
use blam_tags::TagStruct;

/// One axis of a `shader_texture_animation`. The tag's zeros mean a default.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TextureWave {
    pub(super) function: usize,
    pub(super) period: f32,
    pub(super) phase: f32,
    pub(super) scale: f32,
}

impl TextureWave {
    fn read(root: TagStruct<'_>, axis: &str, default_scale: f32) -> Self {
        let name = |field: &str| format!("{axis}-animation {field}");
        Self {
            function: option_index(root, &name("function"), &PERIODIC_FUNCTIONS) as usize,
            period: Some(root.read_real(&name("period")).unwrap_or(0.0))
                .filter(|period| *period != 0.0)
                .unwrap_or(1.0),
            phase: root.read_real(&name("phase")).unwrap_or(0.0),
            scale: Some(root.read_real(&name("scale")).unwrap_or(0.0))
                .filter(|scale| *scale != 0.0)
                .unwrap_or(default_scale),
        }
    }

    /// With the object output a wave reads, or 1 with none.
    fn value(&self, source: Option<usize>, time: f32, outputs: &[f32; 4]) -> f32 {
        let input = source.map_or(1.0, |source| outputs[source]);
        periodic_evaluate(self.function, (self.phase + time) / self.period) * self.scale * input
    }

    fn moves(&self) -> bool {
        self.function != PERIODIC_ONE && self.function != PERIODIC_ZERO
    }
}

/// A map's transform as `shader_texture_animation_evaluate` works it out.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct CeMapAnimation {
    /// The first of its two texture rows.
    pub(super) row: usize,
    /// The object output u, v and rotation read, if any.
    pub(super) sources: [Option<usize>; 3],
    pub(super) waves: [TextureWave; 3],
    pub(super) center: [f32; 2],
    pub(super) scale: [f32; 2],
    pub(super) offset: [f32; 2],
    pub(super) rotation: f32,
}

impl CeMapAnimation {
    /// Whether it moves with no object output doing the moving.
    pub(super) fn moves(&self, outputs: &[f32; 4]) -> bool {
        (0..3).any(|axis| {
            self.waves[axis].moves()
                && self.sources[axis].is_none_or(|source| outputs[source] != 0.0)
        })
    }

    pub(super) fn apply(
        &self,
        rows: &mut [[f32; 4]; H2_TEXTURE_ROWS],
        time: f32,
        outputs: &[f32; 4],
    ) {
        let [u, v, r] =
            std::array::from_fn(|axis| self.waves[axis].value(self.sources[axis], time, outputs));
        let (offset_u, offset_v) = (
            self.offset[0] - self.center[0] + u,
            self.offset[1] - self.center[1] + v,
        );
        let turn = r + self.rotation;
        let (sin, cos) = if turn == 0.0 || turn == 360.0 {
            (0.0, 1.0)
        } else {
            turn.to_radians().sin_cos()
        };
        rows[self.row] = [
            cos * self.scale[0],
            -sin * self.scale[1],
            0.0,
            cos * offset_u - sin * offset_v + self.center[0],
        ];
        rows[self.row + 1] = [
            sin * self.scale[0],
            cos * self.scale[1],
            0.0,
            cos * offset_v + sin * offset_u + self.center[1],
        ];
    }
}

/// A constant color a shader runs between two bounds.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CeColorAnimation {
    pub(super) register: u32,
    pub(super) function: usize,
    pub(super) period: f32,
    pub(super) lower: [f32; 4],
    pub(super) upper: [f32; 4],
    /// Whether the alpha runs too, or belongs to something else.
    pub(super) animates_alpha: bool,
    /// The change color channel it is tinted by.
    pub(super) channel: Option<usize>,
    /// `A-out controls color0 animation`: the object's output for the wave.
    pub(super) follows_output: bool,
}

impl CeColorAnimation {
    fn moves(&self) -> bool {
        self.function != PERIODIC_ONE && self.function != PERIODIC_ZERO && self.lower != self.upper
    }

    fn apply(
        &self,
        value: &mut [f32; 4],
        time: f32,
        change_colors: &[[f32; 3]; 4],
        outputs: &[f32; 4],
    ) {
        let wave = if self.follows_output {
            outputs[0]
        } else {
            periodic_evaluate(self.function, time / self.period)
        };
        let at = |axis: usize| self.lower[axis] + (self.upper[axis] - self.lower[axis]) * wave;
        for axis in 0..3 {
            let tint = self
                .channel
                .map_or(1.0, |channel| change_colors[channel][axis]);
            value[axis] = at(axis) * tint;
        }
        if self.animates_alpha {
            value[3] = at(3);
        }
    }
}

/// What a Halo CE shader works out each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CeModelAnimation {
    pub(super) maps: Vec<CeMapAnimation>,
    pub(super) colors: Vec<CeColorAnimation>,
    /// The constant that holds the way back along the view.
    pub(crate) eye_register: Option<u32>,
    /// The object output a `framebuffer fade source` reads.
    pub(super) fade_source: Option<usize>,
    pub(super) meter: Option<CeMeter>,
    pub(super) plasma: Option<CePlasma>,
}

/// One of plasma's two noise lookups.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CePlasmaNoise {
    pub(super) period: f32,
    pub(super) direction: [f32; 3],
    pub(super) scale: f32,
}

/// What `rasterizer_plasma_energy_draw` works out from its sources.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CePlasma {
    pub(super) intensity_source: Option<usize>,
    pub(super) intensity_exponent: f32,
    pub(super) offset_source: Option<usize>,
    pub(super) offset_exponent: f32,
    pub(super) offset_amount: f32,
    pub(super) perpendicular: [f32; 4],
    pub(super) parallel: [f32; 4],
    pub(super) primary: CePlasmaNoise,
    pub(super) secondary: CePlasmaNoise,
}

impl CePlasma {
    fn intensity(&self, outputs: &[f32; 4]) -> f32 {
        self.intensity_source
            .map_or(1.0, |source| outputs[source].powf(self.intensity_exponent))
    }

    fn apply(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], time: f32, outputs: &[f32; 4]) {
        const LEAST_OFFSET: f32 = 0.0005;
        let offset = self.offset_source.map_or(0.0, |source| {
            outputs[source].powf(self.offset_exponent) * self.offset_amount
        });
        let offset = if offset < LEAST_OFFSET { 0.0 } else { offset };
        for (first, noise) in [(0, &self.primary), (3, &self.secondary)] {
            let moved = time / noise.period;
            for axis in 0..3 {
                let mut row = [0.0; 4];
                row[axis] = noise.scale;
                row[3] = noise.direction[axis] * moved;
                rows[first + axis] = row;
            }
        }
        // The game keeps the push in a spare slot of this row, and its vertex
        // shader's dot with position then picks it up as well.
        rows[0][2] = offset;
        let intensity = self.intensity(outputs);
        let [red, green, blue, alpha] = self.perpendicular;
        let parallel = self.parallel;
        rows[6] = [
            red - parallel[0],
            green - parallel[1],
            blue - parallel[2],
            (alpha - parallel[3]) * intensity,
        ];
        rows[7] = [
            parallel[0],
            parallel[1],
            parallel[2],
            parallel[3] * intensity,
        ];
    }
}

/// What `shader_transparent_meter` works out from its sources.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CeMeter {
    /// Brightness, flash, value and flash extension.
    pub(super) sources: [Option<usize>; 4],
    pub(super) flash: [f32; 3],
    pub(super) maximum: [f32; 3],
    pub(super) minimum: [f32; 3],
    pub(super) background: [f32; 3],
    pub(super) tint: [f32; 3],
    /// `tint mode-2`: meter and background transparency.
    pub(super) transparency: Option<[f32; 2]>,
    pub(super) negative: bool,
}

impl CeMeter {
    /// `c0` to `c5` of the kit's shader. With no source an input is 1.
    pub(super) fn constants(&self, outputs: &[f32; 4]) -> [[f32; 4]; 6] {
        let input = |index: usize| self.sources[index].map_or(1.0, |source| outputs[source]);
        let (brightness, flash, value, extension) = (input(0), input(1), input(2), input(3));
        let with = |[r, g, b]: [f32; 3], w: f32| [r, g, b, w];
        let lit = self.flash.map(|channel| channel * flash);
        let (first_alpha, tint, tint_alpha, background_alpha) = match self.transparency {
            Some([meter, background]) => (
                meter,
                self.tint.map(|channel| channel * brightness),
                meter,
                background,
            ),
            None => (brightness, self.tint, brightness, 0.0),
        };
        let (variable, sign) = if self.negative {
            (lit.map(|c| -c), -1.0)
        } else {
            (lit, 1.0)
        };
        [
            with(lit, first_alpha),
            with(self.maximum, 1.0 / (extension * 8.0f32).max(1.0)),
            with(self.minimum, value),
            with(self.background, background_alpha),
            with(tint, tint_alpha),
            with(variable, sign),
        ]
    }
}

impl PassAnimation for CeModelAnimation {
    fn moves(&self) -> bool {
        self.maps.iter().any(|map| map.moves(&[0.0; 4]))
            || self.colors.iter().any(CeColorAnimation::moves)
    }

    /// Whether it moves by itself while `outputs` hold: plasma's noise always
    /// runs, but there is nothing to see of it at no intensity.
    fn moves_with(&self, outputs: &[f32; 4]) -> bool {
        self.plasma
            .as_ref()
            .is_some_and(|plasma| plasma.intensity(outputs) > 0.0)
    }

    /// Which of the object's outputs this reads.
    fn outputs_read(&self) -> [bool; 4] {
        let mut read = [false; 4];
        let sources = self
            .maps
            .iter()
            .flat_map(|map| map.sources)
            .chain(self.meter.iter().flat_map(|meter| meter.sources))
            .chain(
                self.plasma
                    .iter()
                    .flat_map(|plasma| [plasma.intensity_source, plasma.offset_source]),
            )
            .chain([self.fade_source]);
        sources.flatten().for_each(|source| read[source] = true);
        read[0] |= self.colors.iter().any(|color| color.follows_output);
        read
    }

    /// Whether anything here follows an object output.
    fn reads_outputs(&self) -> bool {
        self.fade_source.is_some()
            || self.meter.is_some()
            || self.plasma.is_some()
            || self
                .maps
                .iter()
                .any(|map| map.sources.iter().any(Option::is_some))
            || self.colors.iter().any(|color| color.follows_output)
    }

    fn apply_rows(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], time: f32, outputs: &[f32; 4]) {
        self.maps
            .iter()
            .for_each(|map| map.apply(rows, time, outputs));
        if let Some(source) = self.fade_source {
            rows[8][0] = outputs[source];
        }
        if let Some(plasma) = &self.plasma {
            plasma.apply(rows, time, outputs);
        }
    }

    /// The color `register` holds, if it is one of these.
    fn constant(
        &self,
        register: u32,
        value: &mut [f32; 4],
        time: f32,
        change_colors: &[[f32; 3]; 4],
        outputs: &[f32; 4],
    ) {
        for color in self
            .colors
            .iter()
            .filter(|color| color.register == register)
        {
            color.apply(value, time, change_colors, outputs);
        }
        if let Some(meter) = &self.meter
            && let Some(constant) = meter.constants(outputs).get(register as usize)
        {
            *value = *constant;
        }
    }

    fn eye_register(&self) -> Option<u32> {
        self.eye_register
    }
}

pub(super) const DETAIL_FUNCTIONS: [&str; 3] =
    ["double/biased multiply", "multiply", "double/biased add"];

const COLOR_SOURCES: [&str; 5] = ["none", "A", "B", "C", "D"];

/// An object function's outputs, as animation and fade fields name them.
const FUNCTION_SOURCES: [&str; 5] = ["none", "A out", "B out", "C out", "D out"];

pub(in crate::app::model_preview) const PERIODIC_FUNCTIONS: [&str; 12] = [
    "one",
    "zero",
    "cosine",
    "cosine (variable period)",
    "diagonal wave",
    "diagonal wave (variable period)",
    "slide",
    "slide (variable period)",
    "noise",
    "jitter",
    "wander",
    "spark",
];

/// A color source's change color channel, with `none` as `None`.
pub(super) fn color_channel(root: TagStruct<'_>, field: &str) -> Option<usize> {
    (option_index(root, field, &COLOR_SOURCES) as usize).checked_sub(1)
}

/// The object output a source field reads, with `none` as `None`.
pub(super) fn function_source(root: TagStruct<'_>, field: &str) -> Option<usize> {
    (option_index(root, field, &FUNCTION_SOURCES) as usize).checked_sub(1)
}

pub(super) fn nonzero(value: Option<f32>, default: f32) -> f32 {
    value.filter(|value| *value != 0.0).unwrap_or(default)
}

/// An effect shader's second map's animation, as the game reads it: no
/// postprocess fills in its zeros, and it is scaled by one.
pub(in crate::app::model_preview) fn read_effect_map_animation(
    root: TagStruct<'_>,
) -> CeMapAnimation {
    let center = root.read_point2d("rotation-animation center");
    CeMapAnimation {
        row: 0,
        sources: [
            function_source(root, "u-animation source"),
            function_source(root, "v-animation source"),
            function_source(root, "rotation-animation source"),
        ],
        waves: [
            TextureWave::read(root, "u", 0.0),
            TextureWave::read(root, "v", 0.0),
            TextureWave::read(root, "rotation", 0.0),
        ],
        center: [center.x, center.y],
        scale: [1.0; 2],
        offset: [0.0; 2],
        rotation: 0.0,
    }
}

impl CeMapAnimation {
    /// The two rows of the transform at a time, for an object's outputs.
    pub(in crate::app::model_preview) fn rows(
        &self,
        time: f32,
        outputs: &[f32; 4],
    ) -> [[f32; 4]; 2] {
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        let at = Self {
            row: 0,
            ..self.clone()
        };
        at.apply(&mut rows, time, outputs);
        [rows[0], rows[1]]
    }
}

pub(super) fn read_map_animation(
    root: TagStruct<'_>,
    row: usize,
    scale: [f32; 2],
    offset: [f32; 2],
    rotation: f32,
) -> CeMapAnimation {
    let center = root.read_point2d("rotation-animation center");
    CeMapAnimation {
        row,
        sources: [
            function_source(root, "u-animation source"),
            function_source(root, "v-animation source"),
            function_source(root, "rotation-animation source"),
        ],
        waves: [
            TextureWave::read(root, "u", 1.0),
            TextureWave::read(root, "v", 1.0),
            TextureWave::read(root, "rotation", 360.0),
        ],
        center: [center.x, center.y],
        scale,
        offset,
        rotation,
    }
}

pub(super) fn read_model_animation(
    root: TagStruct<'_>,
    scale: [f32; 2],
    detail_mask: i32,
) -> CeModelAnimation {
    let upper = root.read_rgb("animation color upper bound");
    let lower = root.read_rgb("animation color lower bound");
    CeModelAnimation {
        maps: vec![read_map_animation(root, 0, scale, [0.0; 2], 0.0)],
        // The game only sets this color with no detail mask.
        eye_register: None,
        fade_source: None,
        meter: None,
        plasma: None,
        colors: (detail_mask == 0)
            .then(|| CeColorAnimation {
                register: 0,
                function: option_index(root, "animation function", &PERIODIC_FUNCTIONS) as usize,
                period: nonzero(root.read_real("animation period"), 1.0),
                lower: [lower.red, lower.green, lower.blue, 0.0],
                upper: [upper.red, upper.green, upper.blue, 0.0],
                animates_alpha: false,
                channel: color_channel(root, "color source"),
                follows_output: false,
            })
            .into_iter()
            .collect(),
    }
}

pub(super) fn identity_map_rows() -> [[f32; 4]; H2_TEXTURE_ROWS] {
    std::array::from_fn(|row| match row {
        0 | 2 | 4 | 6 => [1.0, 0.0, 0.0, 0.0],
        1 | 3 | 5 | 7 => [0.0, 1.0, 0.0, 0.0],
        // The `C_TRANSLUCENCY` a fade source would scale.
        8 => [1.0, 0.0, 0.0, 0.0],
        _ => [0.0; 4],
    })
}
