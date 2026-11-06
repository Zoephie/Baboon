//! Halo CE's model shaders as draw passes.
//!
//! `shader_model` and a `shader_environment` on a model both draw through
//! `ShaderModel()` in the kit's `fx/model_common.h`. What feeds it follows
//! `rasterizer_model_draw_model_shader_pp` and
//! `rasterizer_model_draw_environment_shader_pp`; the lighting is
//! `default_object_lighting`, what the game falls back on with no level.

use std::sync::{Arc, LazyLock};

use blam_tags::{TagBlock, TagFieldData, TagStruct};

use super::super::dx9::passes::{
    H2Blend, H2BlendEquation, H2BlendFactor, H2Compare, H2Cull, H2Depth, H2DrawPass, H2DrawTexture,
    H2Image, H2_GLSL_VERSION,
};
use super::attachments::{has_flag, has_flag_in_any};
use super::super::dx9::stage::{H2_TEXTURE_ROWS, H2VertexStage};
use super::super::load_referenced_tag_from_source;
use crate::core::ce_pixel_shader::{TranslatedFxShader, translate_fx};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;
use crate::app::model_preview::common::random::{high16_times_inverse_65535, lcg_step};

const COMMON_FILE: &str = "model_common.h";
const GENERIC_FILE: &str = "transparent_generic_shader.psh";
const CHICAGO_FILE: &str = "chicago_shader.psh";
const GLASS_TINT_FILE: &str = "transparent_glass_tint.psh";
const GLASS_FLAT_FILE: &str = "transparent_glass_reflection_flat.psh";
const GLASS_BUMPED_FILE: &str = "transparent_glass_reflection_bumped.psh";
const GLASS_DIFFUSE_FILE: &str = "transparent_glass_diffuse.psh";
const METER_FILE: &str = "transparent_meter.psh";
const PLASMA_FILE: &str = "transparent_plasma.psh";
const CAMO_FILE: &str = "active_camouflage_draw.psh";
const WATER_OPACITY_FILE: &str = "transparent_water_opacity.psh";
const WATER_REFLECTION_FILE: &str = "transparent_water_reflection.psh";
const AMBIENT: [f32; 3] = [0.2, 0.2, 0.2];
const FIRST_LIGHT_DIRECTION: [f32; 3] = [-0.577, -0.577, -0.577];
const FIRST_LIGHT_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
const SECOND_LIGHT_DIRECTION: [f32; 3] = [0.0, 0.0, 1.0];
const SECOND_LIGHT_COLOR: [f32; 3] = [0.4, 0.4, 0.5];
/// `reflection_tint_color`: alpha, then red, green and blue.
const REFLECTION_TINT: [f32; 4] = [0.5, 1.0, 1.0, 1.0];
/// `AlphaTest_Greater7F`: between 0x7F and 0x80.
const ALPHA_REFERENCE: f32 = 0.5;

/// `periodic_functions_initialize`'s seed: every table comes out the same.
const PERIODIC_SEED: u32 = 0x20F3_F660;
const PERIODIC_SAMPLES: usize = 1024;
/// Cycles each table holds, of which `periodic_function_evaluate` steps
/// `PERIODIC_RATE` samples a period: the game's periods run long by this.
const PERIODIC_CYCLES: f32 = 28.0;
const PERIODIC_RATE: f32 = 25.6;
const PERIODIC_ONE: usize = 0;
const PERIODIC_ZERO: usize = 1;
const PERIODIC_SLIDES: [usize; 2] = [6, 7];

struct SeedRandom(u32);

impl SeedRandom {
    fn real(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high16_times_inverse_65535(self.0)
    }
}

/// `periodic_function_build_variable_period_x_table`.
fn variable_period_table(random: &mut SeedRandom) -> Vec<f32> {
    let mut table = Vec::with_capacity(PERIODIC_SAMPLES);
    let mut total = 0.0;
    for index in 0..PERIODIC_SAMPLES {
        table.push(total);
        let index = index as f32;
        let first = random.real();
        let wave_a = (index * 0.025_157_286).cos() + 1.0;
        let scale_a = random.real();
        let wave_b = (index * 0.031_293_21).cos() + 1.0;
        let scale_b = random.real();
        let wave_c = (index * 0.044_792_242).cos() + 1.0;
        let scale_c = random.real();
        total += (first + 1.0) * 0.25 + wave_a * scale_a + scale_c * wave_c + wave_b * scale_b;
    }
    table.iter().map(|value| value / total).collect()
}

/// `periodic_function_build_table`.
fn periodic_table(function: usize, random: &mut SeedRandom) -> Vec<u8> {
    use std::f32::consts::{PI, TAU};
    let x_table = variable_period_table(random);
    let wave = |x: f32| {
        let x = x.fract();
        if x < 0.5 { x * 2.0 } else { 1.0 - (x - 0.5) * 2.0 }
    };
    let values: Vec<f32> = x_table
        .iter()
        .enumerate()
        .map(|(index, varied)| {
            let steady = index as f32 * (PERIODIC_CYCLES / PERIODIC_SAMPLES as f32);
            let varied = varied * PERIODIC_CYCLES;
            match function {
                PERIODIC_ONE => 1.0,
                PERIODIC_ZERO => 0.0,
                2 => (steady * TAU).cos(),
                3 => (varied * TAU).cos(),
                4 => wave(steady),
                5 => wave(varied),
                6 => steady.fract(),
                7 => varied.fract(),
                8 => random.real(),
                // Jitter takes wander's path through the game's table builder.
                9 | 10 => {
                    ((steady * 43.9823).cos() * (steady * PI / 2.0).sin()
                        + (steady * 25.132_742).cos() * (steady * 0.897_597_9).cos())
                        * 0.5
                        + (steady * PI).sin() * (steady * TAU).cos()
                }
                _ => varied.fract() * varied.fract(),
            }
        })
        .collect();
    let lowest = values.iter().copied().fold(f32::MAX, f32::min);
    let highest = values.iter().copied().fold(f32::MIN, f32::max);
    let range = if PERIODIC_SLIDES.contains(&function) { 0.0 } else { highest - lowest };
    values
        .iter()
        .map(|value| {
            let value = if range != 0.0 { (value - lowest) / range } else { *value };
            (value * 255.0).clamp(0.0, 255.0) as u8
        })
        .collect()
}

static PERIODIC_TABLES: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    let mut random = SeedRandom(PERIODIC_SEED);
    (0..12).map(|function| periodic_table(function, &mut random)).collect()
});

/// `periodic_function_evaluate`.
pub(in crate::app::model_preview) fn periodic_evaluate(function: usize, time: f32) -> f32 {
    if function == PERIODIC_ONE {
        return 1.0;
    }
    let Some(table) = PERIODIC_TABLES.get(function) else {
        return 0.0;
    };
    let position = time * PERIODIC_RATE;
    let blend = position.rem_euclid(1.0);
    let index = (position - blend) as i64 as usize & (PERIODIC_SAMPLES - 1);
    let low = f32::from(table[index]) / 255.0;
    let mut high = f32::from(table[(index + 1) & (PERIODIC_SAMPLES - 1)]) / 255.0;
    if !PERIODIC_SLIDES.contains(&function) {
        return low + (high - low) * blend;
    }
    if low > 0.75 && high < 0.25 {
        high += 1.0;
    }
    let value = low + (high - low) * blend;
    if value > 1.0 { value - 1.0 } else { value }
}

/// One axis of a `shader_texture_animation`. The tag's zeros mean a default.
#[derive(Debug, Clone, PartialEq)]
struct TextureWave {
    function: usize,
    period: f32,
    phase: f32,
    scale: f32,
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
    row: usize,
    /// The object output u, v and rotation read, if any.
    sources: [Option<usize>; 3],
    waves: [TextureWave; 3],
    center: [f32; 2],
    scale: [f32; 2],
    offset: [f32; 2],
    rotation: f32,
}

impl CeMapAnimation {
    /// Whether it moves with no object output doing the moving.
    fn moves(&self, outputs: &[f32; 4]) -> bool {
        (0..3).any(|axis| {
            self.waves[axis].moves() && self.sources[axis].is_none_or(|source| outputs[source] != 0.0)
        })
    }

    fn apply(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], time: f32, outputs: &[f32; 4]) {
        let [u, v, r] =
            std::array::from_fn(|axis| self.waves[axis].value(self.sources[axis], time, outputs));
        let (offset_u, offset_v) =
            (self.offset[0] - self.center[0] + u, self.offset[1] - self.center[1] + v);
        let turn = r + self.rotation;
        let (sin, cos) = if turn == 0.0 || turn == 360.0 { (0.0, 1.0) } else { turn.to_radians().sin_cos() };
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
struct CeColorAnimation {
    register: u32,
    function: usize,
    period: f32,
    lower: [f32; 4],
    upper: [f32; 4],
    /// Whether the alpha runs too, or belongs to something else.
    animates_alpha: bool,
    /// The change color channel it is tinted by.
    channel: Option<usize>,
    /// `A-out controls color0 animation`: the object's output for the wave.
    follows_output: bool,
}

impl CeColorAnimation {
    fn moves(&self) -> bool {
        self.function != PERIODIC_ONE && self.function != PERIODIC_ZERO && self.lower != self.upper
    }

    fn apply(&self, value: &mut [f32; 4], time: f32, change_colors: &[[f32; 3]; 4], outputs: &[f32; 4]) {
        let wave = if self.follows_output {
            outputs[0]
        } else {
            periodic_evaluate(self.function, time / self.period)
        };
        let at = |axis: usize| self.lower[axis] + (self.upper[axis] - self.lower[axis]) * wave;
        for axis in 0..3 {
            let tint = self.channel.map_or(1.0, |channel| change_colors[channel][axis]);
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
    maps: Vec<CeMapAnimation>,
    colors: Vec<CeColorAnimation>,
    /// The constant that holds the way back along the view.
    pub(crate) eye_register: Option<u32>,
    /// The object output a `framebuffer fade source` reads.
    fade_source: Option<usize>,
    meter: Option<CeMeter>,
    plasma: Option<CePlasma>,
}

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

/// One of plasma's two noise lookups.
#[derive(Debug, Clone, PartialEq)]
struct CePlasmaNoise {
    period: f32,
    direction: [f32; 3],
    scale: f32,
}

/// What `rasterizer_plasma_energy_draw` works out from its sources.
#[derive(Debug, Clone, PartialEq)]
struct CePlasma {
    intensity_source: Option<usize>,
    intensity_exponent: f32,
    offset_source: Option<usize>,
    offset_exponent: f32,
    offset_amount: f32,
    perpendicular: [f32; 4],
    parallel: [f32; 4],
    primary: CePlasmaNoise,
    secondary: CePlasmaNoise,
}

impl CePlasma {
    fn intensity(&self, outputs: &[f32; 4]) -> f32 {
        self.intensity_source.map_or(1.0, |source| outputs[source].powf(self.intensity_exponent))
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
        rows[7] = [parallel[0], parallel[1], parallel[2], parallel[3] * intensity];
    }
}

/// What `shader_transparent_meter` works out from its sources.
#[derive(Debug, Clone, PartialEq)]
struct CeMeter {
    /// Brightness, flash, value and flash extension.
    sources: [Option<usize>; 4],
    flash: [f32; 3],
    maximum: [f32; 3],
    minimum: [f32; 3],
    background: [f32; 3],
    tint: [f32; 3],
    /// `tint mode-2`: meter and background transparency.
    transparency: Option<[f32; 2]>,
    negative: bool,
}

impl CeMeter {
    /// `c0` to `c5` of the kit's shader. With no source an input is 1.
    fn constants(&self, outputs: &[f32; 4]) -> [[f32; 4]; 6] {
        let input = |index: usize| self.sources[index].map_or(1.0, |source| outputs[source]);
        let (brightness, flash, value, extension) = (input(0), input(1), input(2), input(3));
        let with = |[r, g, b]: [f32; 3], w: f32| [r, g, b, w];
        let lit = self.flash.map(|channel| channel * flash);
        let (first_alpha, tint, tint_alpha, background_alpha) = match self.transparency {
            Some([meter, background]) => {
                (meter, self.tint.map(|channel| channel * brightness), meter, background)
            }
            None => (brightness, self.tint, brightness, 0.0),
        };
        let (variable, sign) = if self.negative { (lit.map(|c| -c), -1.0) } else { (lit, 1.0) };
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

impl CeModelAnimation {
    pub(crate) fn moves(&self) -> bool {
        self.maps.iter().any(|map| map.moves(&[0.0; 4]))
            || self.colors.iter().any(CeColorAnimation::moves)
    }

    /// Whether it moves by itself while `outputs` hold: plasma's noise always
    /// runs, but there is nothing to see of it at no intensity.
    pub(crate) fn moves_with(&self, outputs: &[f32; 4]) -> bool {
        self.plasma.as_ref().is_some_and(|plasma| plasma.intensity(outputs) > 0.0)
    }

    /// Which of the object's outputs this reads.
    pub(crate) fn outputs_read(&self) -> [bool; 4] {
        let mut read = [false; 4];
        let sources = self
            .maps
            .iter()
            .flat_map(|map| map.sources)
            .chain(self.meter.iter().flat_map(|meter| meter.sources))
            .chain(self.plasma.iter().flat_map(|plasma| [plasma.intensity_source, plasma.offset_source]))
            .chain([self.fade_source]);
        sources.flatten().for_each(|source| read[source] = true);
        read[0] |= self.colors.iter().any(|color| color.follows_output);
        read
    }

    /// Whether anything here follows an object output.
    pub(crate) fn reads_outputs(&self) -> bool {
        self.fade_source.is_some()
            || self.meter.is_some()
            || self.plasma.is_some()
            || self.maps.iter().any(|map| map.sources.iter().any(Option::is_some))
            || self.colors.iter().any(|color| color.follows_output)
    }

    pub(crate) fn apply_rows(
        &self,
        rows: &mut [[f32; 4]; H2_TEXTURE_ROWS],
        time: f32,
        outputs: &[f32; 4],
    ) {
        self.maps.iter().for_each(|map| map.apply(rows, time, outputs));
        if let Some(source) = self.fade_source {
            rows[8][0] = outputs[source];
        }
        if let Some(plasma) = &self.plasma {
            plasma.apply(rows, time, outputs);
        }
    }

    /// The color `register` holds, if it is one of these.
    pub(crate) fn constant(
        &self,
        register: u32,
        value: &mut [f32; 4],
        time: f32,
        change_colors: &[[f32; 3]; 4],
        outputs: &[f32; 4],
    ) {
        for color in self.colors.iter().filter(|color| color.register == register) {
            color.apply(value, time, change_colors, outputs);
        }
        if let Some(meter) = &self.meter
            && let Some(constant) = meter.constants(outputs).get(register as usize)
        {
            *value = *constant;
        }
    }
}

/// `model_common.h` translated, or `None` where the kit has no shaders.
pub(in crate::app::model_preview) type CeModelCommon = Arc<TranslatedFxShader>;

/// The kit's `shaders/fx` folder, beside `tags`.
fn read_common(source: &TagSource) -> Option<CeModelCommon> {
    let directory = shader_directory(source)?;
    let read = |name: &str| read_shader_source(&directory, name);
    match translate_fx(COMMON_FILE, H2_GLSL_VERSION, &[], read) {
        Ok(shader) => Some(Arc::new(shader)),
        Err(error) => {
            eprintln!("model preview: {COMMON_FILE}: {error}");
            None
        }
    }
}

/// What a map defaults to, as `rasterizer_set_texture_for_effect`'s usage has
/// it: the shape of `global_rasterizer_data`'s default bitmaps.
fn flat(value: u8) -> Arc<H2Image> {
    Arc::new(H2Image::Flat(super::super::materials::TextureImage {
        rgba: vec![value, value, value, value],
        width: 1,
        height: 1,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    }))
}

const WHITE: u8 = 255;
const GREY: u8 = 128;

fn texture(slot: u32, kind: SamplerKind, image: Option<Arc<H2Image>>) -> H2DrawTexture {
    H2DrawTexture {
        slot,
        kind,
        image,
        normalization: false,
        texaccum: false,
        frame_copy: false,
        falloff: false,
        gel: false,
    }
}

/// A `real_argb_color` of tag fields: brightness, then a color.
struct ReflectionColor {
    brightness: f32,
    color: [f32; 3],
}

/// What `ShaderModel` is called with and fed, whichever group asked.
struct ModelPass<'a> {
    environment: bool,
    detail_mask: i32,
    detail_function: i32,
    detail_before_reflection: bool,
    alpha_test: bool,
    decal: bool,
    two_sided: bool,
    base: Option<Arc<H2Image>>,
    detail: Option<Arc<H2Image>>,
    multipurpose: Option<Arc<H2Image>>,
    reflection: Option<Arc<H2Image>>,
    base_scale: [f32; 2],
    detail_scale: [f32; 2],
    self_illumination: [f32; 4],
    change_color: Option<usize>,
    animation: Option<CeModelAnimation>,
    translucency: f32,
    reflection_distances: [f32; 2],
    perpendicular: ReflectionColor,
    parallel: ReflectionColor,
    common: &'a TranslatedFxShader,
}

fn tinted(color: &ReflectionColor) -> [f32; 4] {
    let [alpha, red, green, blue] = REFLECTION_TINT;
    [
        color.color[0] * red,
        color.color[1] * green,
        color.color[2] * blue,
        color.brightness * alpha,
    ]
}

fn build_passes(pass: ModelPass<'_>) -> Vec<H2DrawPass> {
    let common = pass.common;
    let fragment = common.entry(&format!(
        "ShaderModel(i, {}, {}, {}, {}, false)",
        i32::from(pass.environment),
        pass.detail_mask,
        pass.detail_function,
        pass.detail_before_reflection,
    ));
    let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: !pass.decal };
    let mut draw = H2DrawPass::new(H2VertexStage::CeModel, fragment, depth);
    draw.channels = [true, true, true, false];
    if pass.decal {
        draw.depth_bias = MODEL_DECAL_DEPTH_BIAS;
        draw.blend = Some(H2Blend {
            equation: H2BlendEquation::Add,
            source: H2BlendFactor::SrcAlpha,
            destination: H2BlendFactor::OneMinusSrcAlpha,
            constant: [0; 4],
        });
    }
    draw.textures = vec![
        texture(0, SamplerKind::Texture2D, pass.base.or_else(|| Some(flat(WHITE)))),
        texture(1, SamplerKind::Texture2D, pass.detail.or_else(|| Some(flat(GREY)))),
        texture(2, SamplerKind::Texture2D, pass.multipurpose.or_else(|| Some(flat(WHITE)))),
        texture(3, SamplerKind::Cube, pass.reflection),
    ];
    let mut animation = pass.animation;
    if let (Some(animation), Some(register)) = (&mut animation, common.register("c_self_illumination_color")) {
        animation.colors.iter_mut().for_each(|color| color.register = register);
    }
    if let (Some(channel), Some(register)) = (pass.change_color, common.register("c_primary_change_color")) {
        draw.change_colors = (0..3).map(|component| (register, component, channel)).collect();
    }
    let mut constant = |name: &str, value: [f32; 4]| {
        if let Some(register) = common.register(name) {
            draw.constants.push((register, value));
        }
    };
    constant("c_primary_change_color", [1.0; 4]);
    // With no fog: the sky's share is none and the scene's all.
    constant("c_fog_color_correction_0", [0.0, 0.0, 0.0, 1.0]);
    constant("c_fog_color_correction_E", [0.0; 4]);
    constant("c_fog_color_correction_1", [0.0; 4]);
    constant("c_self_illumination_color", pass.self_illumination);
    constant("c_alpha_ref", [if pass.alpha_test { ALPHA_REFERENCE } else { 0.0 }; 4]);
    constant("c_fog_color", [0.0; 4]);

    let vector = |[x, y, z]: [f32; 3]| [x, y, z, 0.0];
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [pass.base_scale[0], 0.0, 0.0, 0.0];
    rows[1] = [0.0, pass.base_scale[1], 0.0, 0.0];
    rows[2] = [pass.detail_scale[0], pass.detail_scale[1], 0.0, 0.0];
    rows[3] = [pass.translucency, 1.0, 0.0, 0.0];
    rows[4] = vector(AMBIENT);
    rows[5] = vector(FIRST_LIGHT_COLOR);
    rows[6] = vector(SECOND_LIGHT_DIRECTION);
    rows[7] = vector(SECOND_LIGHT_COLOR);
    let [falloff, cutoff] = pass.reflection_distances;
    rows[8] = [cutoff, falloff, 0.0, 0.0];
    draw.texture_rows = rows;
    draw.light_direct = FIRST_LIGHT_DIRECTION;
    draw.ce_animation = animation.map(Arc::new);

    let perpendicular = tinted(&pass.perpendicular);
    let parallel = tinted(&pass.parallel);
    draw.tint_facing = std::array::from_fn(|at| perpendicular[at] - parallel[at]);
    draw.tint_glancing = parallel;
    if !pass.two_sided {
        return vec![draw];
    }
    // The game draws the far side again, with its normals turned to face.
    let mut back = draw.clone();
    draw.cull = H2Cull::Back;
    back.cull = H2Cull::Front;
    back.texture_rows[3][1] = -1.0;
    vec![draw, back]
}

/// A shader's option by name, with a list's position standing for its value.
pub(in crate::app::model_preview) fn option_index(root: TagStruct<'_>, field: &str, options: &[&str]) -> i32 {
    let name = root.read_enum_option(field);
    options.iter().position(|option| name.as_deref() == Some(*option)).unwrap_or(0) as i32
}

const DETAIL_FUNCTIONS: [&str; 3] = ["double/biased multiply", "multiply", "double/biased add"];
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
fn color_channel(root: TagStruct<'_>, field: &str) -> Option<usize> {
    (option_index(root, field, &COLOR_SOURCES) as usize).checked_sub(1)
}

/// The object output a source field reads, with `none` as `None`.
fn function_source(root: TagStruct<'_>, field: &str) -> Option<usize> {
    (option_index(root, field, &FUNCTION_SOURCES) as usize).checked_sub(1)
}

fn nonzero(value: Option<f32>, default: f32) -> f32 {
    value.filter(|value| *value != 0.0).unwrap_or(default)
}

/// An effect shader's second map's animation, as the game reads it: no
/// postprocess fills in its zeros, and it is scaled by one.
pub(in crate::app::model_preview) fn read_effect_map_animation(root: TagStruct<'_>) -> CeMapAnimation {
    let center = root.read_point2d("rotation-animation center");
    CeMapAnimation {
        row: 0,
        sources: [
            function_source(root, "u-animation source"),
            function_source(root, "v-animation source"),
            function_source(root, "rotation-animation source"),
        ],
        waves: [TextureWave::read(root, "u", 0.0), TextureWave::read(root, "v", 0.0), TextureWave::read(root, "rotation", 0.0)],
        center: [center.x, center.y],
        scale: [1.0; 2],
        offset: [0.0; 2],
        rotation: 0.0,
    }
}

impl CeMapAnimation {
    /// The two rows of the transform at a time, for an object's outputs.
    pub(in crate::app::model_preview) fn rows(&self, time: f32, outputs: &[f32; 4]) -> [[f32; 4]; 2] {
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        let at = Self { row: 0, ..self.clone() };
        at.apply(&mut rows, time, outputs);
        [rows[0], rows[1]]
    }
}

fn read_map_animation(
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

fn read_model_animation(root: TagStruct<'_>, scale: [f32; 2], detail_mask: i32) -> CeModelAnimation {
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

/// `sub_859960` in MCC's tag_test: a stage's color function as the kit's
/// chicago shader reads it.
fn chicago_color_table(function: i32) -> [f32; 12] {
    let mut table = [0.0; 12];
    let set = |table: &mut [f32; 12], pairs: &[(usize, f32)]| {
        pairs.iter().for_each(|(index, value)| table[*index] = *value);
    };
    set(&mut table, match function {
        0 => &[(3, 1.0), (10, 1.0)],
        1 => &[(0, 1.0), (3, 1.0), (10, 1.0)],
        2 => &[(0, 1.0), (10, 1.0)],
        3 => &[(0, 1.0), (10, 2.0)],
        4 => &[(0, 1.0), (3, 1.0), (9, 1.0), (8, 1.0), (10, 1.0), (11, 1.0)],
        5 => &[(0, 1.0), (3, 1.0), (5, 1.0), (8, 2.0), (11, -1.0), (10, 1.0)],
        6 => &[(3, 1.0), (5, 1.0), (6, 1.0), (8, 2.0), (11, -1.0), (10, 1.0)],
        7 => &[(0, 1.0), (3, 1.0), (9, 1.0), (11, -1.0), (8, 1.0), (10, 1.0)],
        8 => &[(3, 1.0), (4, 1.0), (9, 1.0), (11, -1.0), (8, 1.0), (10, 1.0)],
        9 => &[(0, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
        10 => &[(4, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
        11 => &[(0, 1.0), (1, 1.0), (6, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
        12 => &[(1, 1.0), (6, 1.0), (4, 1.0), (7, 1.0), (2, 1.0), (11, 1.0), (8, 1.0), (10, 1.0)],
        _ => &[],
    });
    table
}

/// `sub_8597B0`: the same for a stage's alpha function.
fn chicago_alpha_table(function: i32) -> [f32; 10] {
    let mut table = [0.0; 10];
    let set = |table: &mut [f32; 10], pairs: &[(usize, f32)]| {
        pairs.iter().for_each(|(index, value)| table[*index] = *value);
    };
    set(&mut table, match function {
        0 => &[(2, 1.0), (8, 1.0)],
        1 => &[(0, 1.0), (2, 1.0), (8, 1.0)],
        2 => &[(0, 1.0), (8, 1.0)],
        3 => &[(0, 1.0), (8, 2.0)],
        4 => &[(0, 1.0), (2, 1.0), (7, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
        5 => &[(0, 1.0), (2, 1.0), (4, 1.0), (6, 2.0), (9, -1.0), (8, 1.0)],
        6 => &[(2, 1.0), (4, 1.0), (5, 1.0), (6, 2.0), (9, -1.0), (8, 1.0)],
        7 => &[(0, 1.0), (2, 1.0), (7, 1.0), (9, -1.0), (6, 1.0), (8, 1.0)],
        8 => &[(2, 1.0), (3, 1.0), (7, 1.0), (9, -1.0), (6, 1.0), (8, 1.0)],
        9 => &[(0, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
        10 => &[(3, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
        11 => &[(0, 1.0), (1, 1.0), (5, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
        12 => &[(1, 1.0), (5, 1.0), (3, 1.0), (9, 1.0), (6, 1.0), (8, 1.0)],
        _ => &[],
    });
    table
}

/// A Halo CE `framebuffer blend function`, by name. A number the list does
/// not name blends as alpha blend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FramebufferBlend {
    #[default]
    AlphaBlend,
    Multiply,
    DoubleMultiply,
    Add,
    Subtract,
    ComponentMin,
    ComponentMax,
    AlphaMultiplyAdd,
}

impl FramebufferBlend {
    /// In the list's order, which [`Self::shader_number`] keeps.
    const ALL: [Self; 8] = [
        Self::AlphaBlend,
        Self::Multiply,
        Self::DoubleMultiply,
        Self::Add,
        Self::Subtract,
        Self::ComponentMin,
        Self::ComponentMax,
        Self::AlphaMultiplyAdd,
    ];

    /// `holder`'s `framebuffer blend function`.
    pub(in crate::app::model_preview) fn read(holder: TagStruct<'_>) -> Self {
        let held = holder.read_enum_option("framebuffer blend function");
        let at = BLEND_FUNCTIONS.iter().position(|name| held.as_deref() == Some(*name));
        at.map_or(Self::AlphaBlend, |at| Self::ALL[at])
    }

    /// Its place in the list: what the effect shader's `blend` and the
    /// shader passes' stage macros switch on.
    pub(in crate::app::model_preview) fn shader_number(self) -> i32 {
        Self::ALL.iter().position(|blend| *blend == self).unwrap_or(0) as i32
    }

    /// `srcblend_table`, `destblend_table` and `blendop_table`.
    pub(in crate::app::model_preview) fn gl(self) -> H2Blend {
        use H2BlendFactor::{DstColor, One, OneMinusSrcAlpha, SrcAlpha, SrcColor, Zero};
        let (equation, source, destination) = match self {
            Self::Multiply => (H2BlendEquation::Add, DstColor, Zero),
            Self::DoubleMultiply => (H2BlendEquation::Add, DstColor, SrcColor),
            Self::Add => (H2BlendEquation::Add, One, One),
            Self::Subtract => (H2BlendEquation::ReverseSubtract, One, One),
            Self::ComponentMin => (H2BlendEquation::Min, One, One),
            Self::ComponentMax => (H2BlendEquation::Max, One, One),
            Self::AlphaMultiplyAdd => (H2BlendEquation::Add, One, OneMinusSrcAlpha),
            Self::AlphaBlend => (H2BlendEquation::Add, SrcAlpha, OneMinusSrcAlpha),
        };
        H2Blend { equation, source, destination, constant: [0; 4] }
    }
}

const BLEND_FUNCTIONS: [&str; 8] = [
    "alpha blend",
    "multiply",
    "double multiply",
    "add",
    "subtract",
    "component min",
    "component max",
    "alpha-multiply add",
];
const FADE_MODES: [&str; 3] = ["none", "fade when perpendicular", "fade when parallel"];

/// The stage arguments of `S#_CONFIGURATION`, which a stage's enums fill in
/// the tag's order, as `sub_71B7B0` copies them.
fn generic_stage_macro(
    index: usize,
    color_mux: bool,
    alpha_mux: bool,
    values: &[i32; 26],
    fog: bool,
) -> String {
    let list = |range: std::ops::Range<usize>| {
        values[range].iter().map(i32::to_string).collect::<Vec<_>>().join(",")
    };
    format!(
        "{index},{},{color_mux},{},{},{alpha_mux},{},{fog}",
        list(0..8),
        list(8..14),
        list(14..22),
        list(22..26),
    )
}

/// The stage `sub_71B7B0` adds for a blend function's fade. Vertex inputs by
/// fade mode: its color input, then its alpha one.
fn generic_fade_stage(blend: i32, fade: i32, index: usize) -> String {
    const SCRATCH: i32 = 11;
    let (color_input, alpha_input) = match fade {
        1 => (20, 10),
        2 => (10, 20),
        _ => (19, 9),
    };
    let mut values = [0; 26];
    let fade_alpha = |values: &mut [i32; 26]| {
        values[14] = SCRATCH;
        values[16] = alpha_input;
        values[24] = 1;
    };
    let fade_color = |values: &mut [i32; 26]| {
        values[0] = SCRATCH;
        values[2] = color_input;
        values[12] = 1;
    };
    match blend {
        0 => fade_alpha(&mut values),
        1 | 2 | 5 => {
            fade_color(&mut values);
            values[4] = if blend == 2 { 2 } else { 1 };
            values[6] = color_input;
            values[7] = 1;
        }
        7 => {
            fade_alpha(&mut values);
            fade_color(&mut values);
        }
        _ => fade_color(&mut values),
    }
    generic_stage_macro(index, false, false, &values, true)
}

/// A shader's enum values in tag order, with a name to start after.
fn enum_values(stage: TagStruct<'_>, after: &str) -> Vec<i32> {
    let mut started = false;
    stage
        .fields()
        .filter_map(|field| {
            let value = match field.value()? {
                TagFieldData::CharEnum { value, .. } => value as i32,
                TagFieldData::ShortEnum { value, .. } => value as i32,
                TagFieldData::LongEnum { value, .. } => value,
                _ => return None,
            };
            let keep = started;
            started |= field.clean_name() == after;
            keep.then_some(value)
        })
        .collect()
}

/// A `real_argb_color` field as red, green, blue and alpha; the tag stores
/// alpha first.
pub(in crate::app::model_preview) fn argb(root: TagStruct<'_>, name: &str) -> [f32; 4] {
    match root.field(name).and_then(|field| field.value()) {
        Some(TagFieldData::RealArgbColor(color)) => [color.red, color.green, color.blue, color.alpha],
        _ => [0.0; 4],
    }
}

/// Which vertex stage draws the first map.
fn transparent_stage(first_map: i32, screenspace: bool) -> H2VertexStage {
    match first_map {
        1 => H2VertexStage::CeTransparentReflection,
        2 => H2VertexStage::CeTransparentObject,
        3 => H2VertexStage::CeTransparentViewer,
        _ if screenspace => H2VertexStage::CeTransparentScreen,
        _ => H2VertexStage::CeTransparent,
    }
}

/// Read a transparent shader's maps into textures and animations.
fn read_transparent_maps(
    maps: Option<&TagBlock<'_>>,
    cube_first: bool,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> (Vec<H2DrawTexture>, Vec<CeMapAnimation>) {
    let mut textures = Vec::new();
    let mut animations = Vec::new();
    for slot in 0..4usize {
        let cube = slot == 0 && cube_first;
        let element = maps.and_then(|maps| maps.element(slot));
        let kind = if cube { SamplerKind::Cube } else { SamplerKind::Texture2D };
        let Some(element) = element else {
            textures.push(texture(slot as u32, kind, (!cube).then(|| flat(0))));
            continue;
        };
        let image = element
            .read_tag_ref_path("map")
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, kind));
        let has = |flag: &str| has_flag(element, "flags", flag);
        let image = match image.as_deref() {
            Some(H2Image::Flat(flat)) if has("u-clamped") || has("v-clamped") => {
                let mut clamped = flat.clone();
                clamped.repeat_x = !has("u-clamped");
                clamped.repeat_y = !has("v-clamped");
                Some(Arc::new(H2Image::Flat(clamped)))
            }
            _ => image,
        };
        textures.push(texture(slot as u32, kind, image));
        if !cube {
            let scale = [
                nonzero(element.read_real("map u-scale"), 1.0),
                nonzero(element.read_real("map v-scale"), 1.0),
            ];
            let offset = [
                element.read_real("map u-offset").unwrap_or(0.0),
                element.read_real("map v-offset").unwrap_or(0.0),
            ];
            let rotation = element.read_real("map rotation").unwrap_or(0.0);
            animations.push(read_map_animation(element, slot * 2, scale, offset, rotation));
        }
    }
    (textures, animations)
}

fn identity_map_rows() -> [[f32; 4]; H2_TEXTURE_ROWS] {
    std::array::from_fn(|row| match row {
        0 | 2 | 4 | 6 => [1.0, 0.0, 0.0, 0.0],
        1 | 3 | 5 | 7 => [0.0, 1.0, 0.0, 0.0],
        // The `C_TRANSLUCENCY` a fade source would scale.
        8 => [1.0, 0.0, 0.0, 0.0],
        _ => [0.0; 4],
    })
}

/// A `numeric` shader's digits, before they are split across passes.
fn first_map_digits(root: TagStruct<'_>) -> Option<CeNumeric> {
    let block = ["maps", "4 stage maps"]
        .into_iter()
        .find_map(|name| root.field(name).and_then(|field| field.as_block()))?;
    Some(CeNumeric {
        map: block.element(0)?.read_tag_ref_path("map").filter(|path| !path.is_empty())?,
        count: 0,
        limit: root.read_int_any("numeric counter limit").unwrap_or(0) as i32,
        digit: 0,
        place: 0,
        timer: has_flag(root, "extra flags", "numeric countdown timer"),
    })
}

/// `cf_DecalZBiasValue`, as `rasterizer_model_draw_model_shader_pp` sets it.
const MODEL_DECAL_DEPTH_BIAS: f32 = -5.5e-5;
/// `cf_TransparentDecalZBiasValue`, as `rasterizer_transparent_geometry_group_draw`
/// sets it. Neither has a slope term.
const TRANSPARENT_DECAL_DEPTH_BIAS: f32 = -5e-6;

/// A transparent pass's state, whichever group fills its constants.
fn transparent_pass(
    root: TagStruct<'_>,
    first_map: i32,
    flags: &ShaderFlags<'_>,
    fragment: String,
    textures: Vec<H2DrawTexture>,
    constants: Vec<(u32, [f32; 4])>,
    animation: CeModelAnimation,
) -> H2DrawPass {
    let has = |flag: &str| flags.has(flag);
    let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: false };
    let stage = transparent_stage(first_map, has("first map is in screenspace"));
    let mut pass = H2DrawPass::new(stage, fragment, depth);
    pass.channels = [true, true, true, false];
    pass.blend = Some(FramebufferBlend::read(root).gl());
    pass.cull = if has("two-sided") { H2Cull::None } else { H2Cull::Back };
    pass.hidden_by_camo = !has("don't fade active-camouflage");
    pass.depth_bias = decal_bias(flags.has("decal"));
    pass.numeric = has("numeric").then(|| first_map_digits(root)).flatten();
    pass.textures = textures;
    pass.constants = constants;
    pass.texture_rows = identity_map_rows();
    let fade_source = function_source(root, "framebuffer fade source");
    pass.ce_animation = Some(Arc::new(CeModelAnimation { fade_source, ..animation }));
    pass
}

const FIRST_MAP_TYPES: [&str; 4] = [
    "2D map",
    "first map is reflection cube map",
    "first map is object-centered cube map",
    "first map is viewer-centered cube map",
];
const GENERIC_FIRST_MAP_TYPES: [&str; 4] =
    ["2D map", "reflection cube map", "object-centered cube map", "viewer-centered cube map"];

fn read_shader_source(directory: &std::path::Path, name: &str) -> Result<String, String> {
    let path = directory.join(name);
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn shader_directory(source: &TagSource) -> Option<std::path::PathBuf> {
    let TagSource::LooseFolder { root, .. } = source else {
        return None;
    };
    Some(root.parent()?.join("shaders").join("fx"))
}

fn generic_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<H2DrawPass, String> {
    let first_map = option_index(root, "first map type", &GENERIC_FIRST_MAP_TYPES);
    let maps = root.field("maps").and_then(|field| field.as_block());
    let (textures, maps) = read_transparent_maps(maps.as_ref(), first_map != 0, bitmap);

    let mut macros: Vec<(String, String)> = Vec::new();
    let mut constants: Vec<(u32, [f32; 4])> = (0..16).map(|register| (register, [0.0; 4])).collect();
    let mut colors = Vec::new();
    let stages = root.field("stages").and_then(|field| field.as_block());
    let count = stages.as_ref().map_or(0, |stages| stages.len());
    if count == 0 {
        let mut values = [0; 26];
        (values[0], values[2], values[8]) = (5, 1, 1);
        (values[14], values[16], values[22]) = (5, 1, 1);
        macros.push(("S0_CONFIGURATION".to_owned(), generic_stage_macro(0, false, false, &values, false)));
    }
    for index in 0..count {
        let Some(stage) = stages.as_ref().and_then(|stages| stages.element(index)) else {
            continue;
        };
        let enums = enum_values(stage, "color0 animation function");
        let values: [i32; 26] =
            enums.try_into().map_err(|_| "A stage has the wrong number of options.".to_owned())?;
        let has = |flag: &str| has_flag(stage, "flags", flag);
        macros.push((
            format!("S{index}_CONFIGURATION"),
            generic_stage_macro(index, has("color mux"), has("alpha mux"), &values, false),
        ));
        let lower = argb(stage, "color0 animation lower bound");
        let upper = argb(stage, "color0 animation upper bound");
        let period = nonzero(stage.read_real("color0 animation period"), 1.0);
        let function = option_index(stage, "color0 animation function", &PERIODIC_FUNCTIONS) as usize;
        let channel = color_channel(stage, "color0 source");
        constants[index].1 = lower;
        constants[index + 8].1 = argb(stage, "color1");
        colors.push(CeColorAnimation {
            register: index as u32,
            function,
            period,
            lower,
            upper,
            animates_alpha: true,
            channel,
            follows_output: has("A-out controls color0 animation"),
        });
    }
    let blend = FramebufferBlend::read(root).shader_number();
    let fade = option_index(root, "framebuffer fade mode", &FADE_MODES);
    let last = count.max(1);
    macros.push((format!("S{last}_CONFIGURATION"), generic_fade_stage(blend, fade, last)));
    // Fog on and not planar: the last stage fades rather than fogs.
    constants.push((16, [1.0, 0.0, 0.0, 0.0]));

    let mut defines: Vec<(&str, &str)> =
        macros.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
    if first_map != 0 {
        defines.push(("FIRST_MAP_IS_CUBE", "1"));
    }
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(GENERIC_FILE, H2_GLSL_VERSION, &defines, read)?;
    let reference = flags.has("alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation { maps, colors, ..Default::default() };
    Ok(transparent_pass(root, first_map, flags, fragment, textures, constants, animation))
}

fn chicago_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    field: &str,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<H2DrawPass, String> {
    let first_map = option_index(root, "first map type", &FIRST_MAP_TYPES);
    let maps = root.field(field).and_then(|field| field.as_block());
    let (textures, animations) = read_transparent_maps(maps.as_ref(), first_map != 0, bitmap);

    let mut replicate = [0.0; 3];
    let mut functions = [(0, 0); 3];
    let count = maps.as_ref().map_or(0, |maps| maps.len());
    for (index, slot) in functions.iter_mut().enumerate().take(count.saturating_sub(1)) {
        let Some(map) = maps.as_ref().and_then(|maps| maps.element(index)) else {
            continue;
        };
        *slot = (
            map.read_int_any("color function").unwrap_or(0) as i32,
            map.read_int_any("alpha function").unwrap_or(0) as i32,
        );
        replicate[index] = f32::from(u8::from(has_flag(map, "flags", "alpha replicate")));
    }
    let blend = match FramebufferBlend::read(root) {
        FramebufferBlend::AlphaBlend => 0,
        FramebufferBlend::Multiply | FramebufferBlend::ComponentMin => 1,
        FramebufferBlend::DoubleMultiply => 2,
        FramebufferBlend::AlphaMultiplyAdd => 4,
        FramebufferBlend::Add | FramebufferBlend::Subtract | FramebufferBlend::ComponentMax => 3,
    };
    let fade = option_index(root, "framebuffer fade mode", &FADE_MODES);
    let custom = flags.has("custom edition blending");
    let truth = |on: bool| f32::from(u8::from(on));

    let mut constants = vec![(0, [0.0; 4])];
    for (stage, (color, alpha)) in functions.into_iter().enumerate() {
        let (color, alpha) = (chicago_color_table(color), chicago_alpha_table(alpha));
        let first = 1 + 6 * stage as u32;
        for row in 0..3 {
            let at = row as usize * 4;
            constants.push((first + row, std::array::from_fn(|index| color[at + index])));
            constants.push((
                first + 3 + row,
                std::array::from_fn(|index| alpha.get(at + index).copied().unwrap_or(0.0)),
            ));
        }
    }
    constants.push((19, [replicate[0], replicate[1], replicate[2], truth(custom)]));
    constants.push((20, [truth(fade == 0), truth(fade == 1), truth(fade == 2), 0.0]));
    constants.push((21, [truth(blend == 0), truth(blend == 1), truth(blend == 2), truth(blend == 3)]));

    let cube = if first_map != 0 { "1" } else { "0" };
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(CHICAGO_FILE, H2_GLSL_VERSION, &[("USE_CUBE", cube)], read)?;
    let reference = flags.has("alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation { maps: animations, ..Default::default() };
    Ok(transparent_pass(root, first_map, flags, fragment, textures, constants, animation))
}

/// A Halo CE shader's flags by name. Its root holds a `flags` field per
/// section (the shader's own and its kind's) and sometimes `extra flags`,
/// whose options don't repeat, so a name is asked of each.
#[derive(Clone, Copy)]
pub(in crate::app::model_preview) struct ShaderFlags<'a>(TagStruct<'a>);

impl<'a> ShaderFlags<'a> {
    pub(in crate::app::model_preview) fn of(root: TagStruct<'a>) -> Self {
        Self(root)
    }

    /// Whether the option `name` is set in any of the root's flags.
    pub(in crate::app::model_preview) fn has(&self, name: &str) -> bool {
        has_flag_in_any(self.0, "flags", name) || has_flag_in_any(self.0, "extra flags", name)
    }
}

/// A transparent shader's passes: its extra layers, then itself. Empty for
/// any other group, or where the kit has no shaders.
pub(in crate::app::model_preview) fn ce_transparent_passes(
    source: &TagSource,
    root: TagStruct<'_>,
    group: &[u8; 4],
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Vec<H2DrawPass> {
    let Some(directory) = shader_directory(source) else {
        return Vec::new();
    };
    let single = match group {
        b"sgla" => Some(glass_passes(&directory, root, &ShaderFlags::of(root), bitmap)),
        b"smet" => {
            let flags = ShaderFlags::of(root);
            Some(meter_pass(&directory, root, &flags, bitmap).map(|mut pass| {
                pass.depth_bias = decal_bias(flags.has("decal"));
                vec![pass]
            }))
        }
        b"swat" => Some(water_passes(&directory, root, &ShaderFlags::of(root), bitmap)),
        b"spla" => Some(plasma_pass(&directory, root, bitmap).map(|pass| vec![pass])),
        _ => None,
    };
    if let Some(passes) = single {
        return passes.unwrap_or_else(|error| {
            eprintln!("model preview: transparent shader: {error}");
            Vec::new()
        });
    }
    if !matches!(group, b"sotr" | b"schi" | b"scex") {
        return Vec::new();
    }
    let mut passes = Vec::new();
    let layers = root.field("extra layers").and_then(|field| field.as_block());
    for index in 0..layers.as_ref().map_or(0, |layers| layers.len()) {
        let Some(path) = layers
            .as_ref()
            .and_then(|layers| layers.element(index))
            .and_then(|layer| layer.read_tag_ref_path("shader"))
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        for (extension, layer_group) in LAYER_GROUPS {
            if let Ok(tag) = load_referenced_tag_from_source(source, &path, extension, layer_group) {
                passes.extend(ce_transparent_passes(source, tag.root(), layer_group, bitmap));
                break;
            }
        }
    }
    let flags = ShaderFlags::of(root);
    let pass = match group {
        b"sotr" => generic_pass(&directory, root, &flags, bitmap),
        b"schi" => chicago_pass(&directory, root, &flags, "maps", bitmap),
        _ => chicago_pass(&directory, root, &flags, "4 stage maps", bitmap),
    };
    match pass {
        Ok(pass) => passes.push(pass),
        Err(error) => {
            eprintln!("model preview: transparent shader: {error}");
            return Vec::new();
        }
    }
    passes
}

/// A pass of the shaders below: nothing but what differs.
struct SimplePass {
    stage: H2VertexStage,
    fragment: String,
    blend: H2Blend,
    two_sided: bool,
    write_depth: bool,
    channels: [bool; 4],
    textures: Vec<H2DrawTexture>,
    constants: Vec<(u32, [f32; 4])>,
    rows: [[f32; 4]; H2_TEXTURE_ROWS],
}

impl SimplePass {
    fn build(self) -> H2DrawPass {
        let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: self.write_depth };
        let mut pass = H2DrawPass::new(self.stage, self.fragment, depth);
        pass.blend = Some(self.blend);
        pass.cull = if self.two_sided { H2Cull::None } else { H2Cull::Back };
        pass.channels = self.channels;
        pass.textures = self.textures;
        pass.constants = self.constants;
        pass.texture_rows = self.rows;
        pass.light_direct = FIRST_LIGHT_DIRECTION;
        pass
    }
}

fn plain_blend(source: H2BlendFactor, destination: H2BlendFactor) -> H2Blend {
    H2Blend { equation: H2BlendEquation::Add, source, destination, constant: [0; 4] }
}

fn lit_rows(scale: [f32; 4]) -> [[f32; 4]; H2_TEXTURE_ROWS] {
    let vector = |[x, y, z]: [f32; 3]| [x, y, z, 0.0];
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[2] = scale;
    rows[4] = vector(AMBIENT);
    rows[5] = vector(FIRST_LIGHT_COLOR);
    rows[6] = vector(SECOND_LIGHT_DIRECTION);
    rows[7] = vector(SECOND_LIGHT_COLOR);
    rows
}

/// `H2VertexStage::Bumped`'s rows for one map at `scale`.
fn bumped_rows(scale: f32) -> [[f32; 4]; H2_TEXTURE_ROWS] {
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [1.0, 1.0, 0.0, 0.0];
    rows[2] = [scale, scale, 0.0, 0.0];
    rows
}

fn normalization(slot: u32) -> H2DrawTexture {
    let mut cube = texture(slot, SamplerKind::Cube, None);
    cube.normalization = true;
    cube
}

/// A flat normal, as the default vector bitmap has it.
fn flat_normal() -> Arc<H2Image> {
    Arc::new(H2Image::Flat(super::super::materials::TextureImage {
        rgba: vec![128, 128, 255, 255],
        width: 1,
        height: 1,
        repeat_x: true,
        repeat_y: true,
        scale: [1.0, 1.0],
    }))
}

/// `rasterizer_glass_draw_tint`, `_reflection` and `_diffuse`, in that order.
fn glass_passes(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<Vec<H2DrawPass>, String> {
    let read = |name: &str| read_shader_source(directory, name);
    let has = |flag: &str| flags.has(flag);
    let two_sided = has("two-sided");
    let path = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
    let mut map = |name: &str, kind| path(name).and_then(|path| bitmap(&path, kind));
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let scale = |name: &str| nonzero(root.read_real(name), 1.0);
    let mut passes = Vec::new();

    let tint = rgb("background tint color");
    let tint_map = map("background tint map", SamplerKind::Texture2D);
    if tint_map.is_some() || tint != [0.0; 3] {
        let shader = translate_fx(GLASS_TINT_FILE, H2_GLSL_VERSION, &[], read)?;
        let register = shader.register("c_tint_and_intensity").unwrap_or(0);
        let size = scale("background tint map scale");
        passes.push(SimplePass {
            stage: H2VertexStage::CeGlassLit,
            fragment: shader.entry("pixel_main(i)"),
            blend: plain_blend(H2BlendFactor::Zero, H2BlendFactor::SrcColor),
            two_sided,
            write_depth: false,
            channels: [true, true, true, false],
            textures: vec![texture(0, SamplerKind::Texture2D, tint_map.or_else(|| Some(flat(WHITE))))],
            constants: vec![(register, [tint[0], tint[1], tint[2], 1.0])],
            rows: lit_rows([size, size, 0.0, 0.0]),
        }.build());
    }

    let perpendicular = real("perpendicular brightness");
    let parallel = real("parallel brightness");
    let reflection = map("reflection map", SamplerKind::Cube);
    let reflection_type = option_index(root, "reflection type", &["bumped cube-map", "flat cube-map", "dynamic mirror"]);
    if (perpendicular > 0.0 || parallel > 0.0) && reflection.is_some() && reflection_type != 2 {
        let bump = map("bump map", SamplerKind::Texture2D);
        let specular_mask = has("bump map is specular mask");
        let flat_surface = reflection_type != 0 || specular_mask || bump.is_none();
        let file = if flat_surface { GLASS_FLAT_FILE } else { GLASS_BUMPED_FILE };
        let shader = translate_fx(file, H2_GLSL_VERSION, &[], read)?;
        let mut constants = Vec::new();
        let mut constant = |name: &str, value: [f32; 4]| {
            if let Some(register) = shader.register(name) {
                constants.push((register, value));
            }
        };
        let [red, green, blue] = rgb("perpendicular tint color");
        constant("c_view_perpendicular_color", [red, green, blue, perpendicular]);
        let [red, green, blue] = rgb("parallel tint color");
        constant("c_view_parallel_color", [red, green, blue, parallel]);
        let truth = |on: bool| f32::from(u8::from(on));
        constant("params", [1.0, truth(bump.is_none()), truth(specular_mask), 0.0]);
        constant("c_eye_forward", [0.0; 4]);
        let size = real("bump map scale");
        let size = if size == 0.0 { 1.0 } else { size };
        let mut draw = SimplePass {
            stage: if flat_surface { H2VertexStage::CeGlassFlat } else { H2VertexStage::Bumped },
            fragment: shader.entry("pixel_main(i)"),
            blend: plain_blend(H2BlendFactor::SrcAlpha, H2BlendFactor::One),
            two_sided,
            write_depth: false,
            channels: [true, true, true, false],
            textures: vec![
                texture(0, SamplerKind::Texture2D, bump.or_else(|| Some(flat_normal()))),
                normalization(1),
                normalization(2),
                texture(3, SamplerKind::Cube, reflection),
            ],
            constants,
            rows: if flat_surface { lit_rows([size, size, 0.0, 0.0]) } else { bumped_rows(size) },
        }
        .build();
        draw.ce_animation = Some(Arc::new(CeModelAnimation {
            eye_register: shader.register("c_eye_forward"),
            ..Default::default()
        }));
        passes.push(draw);
    }

    let diffuse = map("diffuse map", SamplerKind::Texture2D);
    let detail = map("diffuse detail map", SamplerKind::Texture2D);
    if diffuse.is_some() || detail.is_some() {
        let shader = translate_fx(GLASS_DIFFUSE_FILE, H2_GLSL_VERSION, &[], read)?;
        let (base, fine) = (scale("diffuse map scale"), scale("diffuse detail map scale"));
        passes.push(SimplePass {
            stage: H2VertexStage::CeGlassLit,
            fragment: shader.entry("main_T0_P0(i)"),
            blend: plain_blend(H2BlendFactor::SrcAlpha, H2BlendFactor::OneMinusSrcAlpha),
            two_sided,
            write_depth: false,
            channels: [true, true, true, false],
            textures: vec![
                texture(0, SamplerKind::Texture2D, diffuse.or_else(|| Some(flat(WHITE)))),
                texture(1, SamplerKind::Texture2D, detail.or_else(|| Some(flat(GREY)))),
            ],
            constants: Vec::new(),
            rows: lit_rows([base, base, fine, fine]),
        }.build());
    }
    // A dynamic mirror alone: there is nothing here for it to reflect.
    if passes.is_empty() {
        passes.push(unseen_pass());
    }
    for pass in &mut passes {
        pass.hidden_by_camo = true;
        pass.depth_bias = decal_bias(flags.has("decal"));
    }
    Ok(passes)
}

/// The depth bias of a transparent shader by its `decal` flag.
fn decal_bias(decal: bool) -> f32 {
    if decal { TRANSPARENT_DECAL_DEPTH_BIAS } else { 0.0 }
}

/// The `shader_transparent_meter` case of
/// `rasterizer_transparent_geometry_group_draw`.
fn meter_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<H2DrawPass, String> {
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let has = |flag: &str| flags.has(flag);
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let meter = CeMeter {
        sources: [
            function_source(root, "meter brightness source"),
            function_source(root, "flash brightness source"),
            function_source(root, "value source"),
            function_source(root, "flash-extension source"),
        ],
        flash: rgb("flash color"),
        maximum: rgb("gradient max color"),
        minimum: rgb("gradient min color"),
        background: rgb("background color"),
        tint: rgb("tint color"),
        transparency: has("tint mode-2")
            .then(|| [real("meter transparency"), real("background transparency")]),
        negative: has("flash color is negative"),
    };
    let image = root
        .read_tag_ref_path("map")
        .filter(|path| !path.is_empty())
        .and_then(|path| bitmap(&path, SamplerKind::Texture2D));
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(METER_FILE, H2_GLSL_VERSION, &[], read)?;
    let mut pass = SimplePass {
        stage: H2VertexStage::CeMeter,
        fragment: shader.entry("pixel_main(i)"),
        blend: plain_blend(H2BlendFactor::One, H2BlendFactor::One),
        two_sided: has("two-sided"),
        write_depth: false,
        channels: [true, true, true, false],
        textures: vec![texture(0, SamplerKind::Texture2D, image.or_else(|| Some(flat(WHITE))))],
        constants: (0..6).map(|register| (register, [0.0; 4])).collect(),
        rows: [[0.0; 4]; H2_TEXTURE_ROWS],
    }
    .build();
    pass.ce_animation =
        Some(Arc::new(CeModelAnimation { meter: Some(meter), ..Default::default() }));
    Ok(pass)
}

/// `rasterizer_water_draw`, without its ripples: the game composes those into
/// a target each frame, which this draws flat.
fn water_passes(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &ShaderFlags<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<Vec<H2DrawPass>, String> {
    let read = |name: &str| read_shader_source(directory, name);
    let has = |flag: &str| flags.has(flag);
    let path = |name: &str| root.read_tag_ref_path(name).filter(|path| !path.is_empty());
    let base = path("base map")
        .and_then(|path| bitmap(&path, SamplerKind::Texture2D))
        .or_else(|| Some(flat(WHITE)));
    let reflection = path("reflection map").and_then(|path| bitmap(&path, SamplerKind::Cube));
    let perpendicular = root.read_real("view perpendicular brightness").unwrap_or(0.0);
    let parallel = nonzero(root.read_real("view parallel brightness"), 1.0);
    let modulates_reflection = has("base map alpha modulates reflection");
    let opacity = translate_fx(WATER_OPACITY_FILE, H2_GLSL_VERSION, &[], read)?;
    let constants = vec![(0, [perpendicular; 4]), (1, [parallel; 4]), (2, [0.0; 4])];
    let mut passes = Vec::new();
    let surface = |call: &str, blend, channels, textures, constants| {
        SimplePass {
            stage: H2VertexStage::CeWaterOpacity,
            fragment: opacity.entry(call),
            blend,
            two_sided: true,
            write_depth: false,
            channels,
            textures,
            constants,
            rows: [[0.0; 4]; H2_TEXTURE_ROWS],
        }
        .build()
    };
    if modulates_reflection {
        passes.push(surface(
            "main_T0_P0(i)",
            plain_blend(H2BlendFactor::One, H2BlendFactor::Zero),
            [false, false, false, true],
            vec![texture(0, SamplerKind::Texture2D, base.clone()), normalization(1)],
            constants.clone(),
        ));
    }
    if has("base map color modulates background") {
        passes.push(surface(
            "main_T0_P1(i)",
            plain_blend(H2BlendFactor::Zero, H2BlendFactor::SrcColor),
            [true, true, true, false],
            vec![texture(0, SamplerKind::Texture2D, base), normalization(1)],
            constants,
        ));
    }
    let shader = translate_fx(WATER_REFLECTION_FILE, H2_GLSL_VERSION, &[], read)?;
    let source = if modulates_reflection { H2BlendFactor::DstAlpha } else { H2BlendFactor::One };
    passes.push(
        SimplePass {
            stage: H2VertexStage::Bumped,
            fragment: shader.entry("pixel_main(i)"),
            blend: plain_blend(source, H2BlendFactor::One),
            two_sided: true,
            write_depth: true,
            channels: [true, true, true, false],
            textures: vec![
                texture(0, SamplerKind::Texture2D, Some(flat_normal())),
                texture(3, SamplerKind::Cube, reflection),
            ],
            constants: vec![(0, [1.0, 1.0, 1.0, 0.0]), (1, [0.0; 4])],
            rows: bumped_rows(1.0),
        }
        .build(),
    );
    Ok(passes)
}

/// What `globals` gives every object's active camouflage.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app::model_preview) struct CeCamoSettings {
    fragment: String,
    effect_register: u32,
    screen_register: u32,
    refraction: f32,
    falloff: f32,
    tint: [f32; 3],
    distortion: Option<String>,
}

/// `globals\globals`'s rasterizer data, and the kit's shader for it.
pub(in crate::app::model_preview) fn read_ce_camo_settings(source: &TagSource) -> Option<CeCamoSettings> {
    let directory = shader_directory(source)?;
    let globals = load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
    let root = globals.root();
    let data = root.descend("rasterizer data[0]")?;
    let tint = data.read_rgb("tint color");
    let read = |name: &str| read_shader_source(&directory, name);
    let shader = translate_fx(CAMO_FILE, H2_GLSL_VERSION, &[], read).ok()?;
    let entry = if has_flag(data, "flags", "tint edge density") {
        "main_T0_P1(i)"
    } else {
        "main_T0_P0(i)"
    };
    Some(CeCamoSettings {
        fragment: shader.entry(entry),
        effect_register: shader.register("c_effect_intensity")?,
        screen_register: shader.register("c_split_screen_offset_and_scale")?,
        refraction: data.read_real("refraction amount").unwrap_or(0.0),
        falloff: data.read_real("distance falloff").unwrap_or(0.0),
        tint: [tint.red, tint.green, tint.blue],
        distortion: data
            .read_tag_ref_path("active camouflage distortion")
            .filter(|path| !path.is_empty()),
    })
}

/// `rasterizer_active_camouflage_draw` at full intensity. A cheap copy of the
/// frame behind stands in for its own primary target, as Halo 2's camo does.
pub(in crate::app::model_preview) fn ce_camo_pass(
    settings: &CeCamoSettings,
    two_sided: bool,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> H2DrawPass {
    /// `refraction amount` is in pixels of the half-width the game draws at.
    const HALF_WIDTH: f32 = 320.0;
    let distortion =
        settings.distortion.as_deref().and_then(|path| bitmap(path, SamplerKind::Cube));
    let mut frame = texture(2, SamplerKind::Texture2D, None);
    frame.frame_copy = true;
    let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
    rows[0] = [settings.refraction / HALF_WIDTH, settings.falloff, 0.0, 0.0];
    let [red, green, blue] = settings.tint;
    rows[1] = [red, green, blue, 0.0];
    SimplePass {
        stage: H2VertexStage::CeActiveCamo,
        fragment: settings.fragment.clone(),
        blend: plain_blend(H2BlendFactor::One, H2BlendFactor::Zero),
        two_sided,
        write_depth: true,
        channels: [true, true, true, false],
        textures: vec![texture(0, SamplerKind::Cube, distortion), frame],
        constants: vec![
            (settings.effect_register, [1.0; 4]),
            (settings.screen_register, [0.0, 1.0, 0.0, 1.0]),
        ],
        rows,
    }
    .build()
}

/// `rasterizer_plasma_energy_draw`. All the kit's are shield hits, unseen until
/// `recent shield damage` raises their intensity. `tint color source` is none
/// on all of them, so it is not read.
fn plasma_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Result<H2DrawPass, String> {
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let tinted = |brightness: &str, tint: &str| {
        let color = root.read_rgb(tint);
        [color.red, color.green, color.blue, real(brightness)]
    };
    let noise = |name: &str| {
        let direction = root.read_vec3(&format!("{name} animation direction"));
        CePlasmaNoise {
            period: nonzero(root.read_real(&format!("{name} animation period")), 1.0),
            direction: [direction.i, direction.j, direction.k],
            scale: real(&format!("{name} noise map scale")),
        }
    };
    let plasma = CePlasma {
        intensity_source: function_source(root, "intensity source"),
        intensity_exponent: nonzero(root.read_real("intensity exponent"), 1.0),
        offset_source: function_source(root, "offset source"),
        offset_exponent: nonzero(root.read_real("offset exponent"), 1.0),
        offset_amount: real("offset amount"),
        perpendicular: tinted("perpendicular brightness", "perpendicular tint color"),
        parallel: tinted("parallel brightness", "parallel tint color"),
        primary: noise("primary"),
        secondary: noise("secondary"),
    };
    let mut volume = |slot, name: &str| {
        let image = root
            .read_tag_ref_path(name)
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, SamplerKind::Volume));
        texture(slot, SamplerKind::Volume, image)
    };
    let textures = vec![volume(0, "primary noise map"), volume(1, "secondary noise map")];
    let read = |name: &str| read_shader_source(directory, name);
    let shader = translate_fx(PLASMA_FILE, H2_GLSL_VERSION, &[], read)?;
    let mut pass = SimplePass {
        stage: H2VertexStage::CePlasma,
        fragment: shader.entry("pixel_main(i)"),
        blend: plain_blend(H2BlendFactor::SrcAlpha, H2BlendFactor::One),
        two_sided: true,
        write_depth: false,
        channels: [true, true, true, false],
        textures,
        constants: Vec::new(),
        rows: [[0.0; 4]; H2_TEXTURE_ROWS],
    }
    .build();
    pass.ce_animation =
        Some(Arc::new(CeModelAnimation { plasma: Some(plasma), ..Default::default() }));
    Ok(pass)
}

/// A pass that draws nothing, for a surface with nothing to show a lone model.
fn unseen_pass() -> H2DrawPass {
    let fragment = format!(
        "{H2_GLSL_VERSION}in vec4 v_color0;\nout vec4 out_color;\nvoid main() {{ discard; }}\n"
    );
    let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: false };
    let mut pass = H2DrawPass::new(H2VertexStage::CeMeter, fragment, depth);
    pass.channels = [false; 4];
    pass
}

const LAYER_GROUPS: [(&str, &[u8; 4]); 3] = [
    ("shader_transparent_generic", b"sotr"),
    ("shader_transparent_chicago", b"schi"),
    ("shader_transparent_chicago_extended", b"scex"),
];

/// The passes of a `shader_model` or `shader_environment`, empty for any other
/// group and where the kit has no shaders, which keeps the slots drawing.
pub(in crate::app::model_preview) fn ce_model_passes(
    source: &TagSource,
    root: TagStruct<'_>,
    group: &[u8; 4],
    flags: &ShaderFlags<'_>,
    common: &mut Option<Option<CeModelCommon>>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Vec<H2DrawPass> {
    if !matches!(group, b"soso" | b"senv") {
        return Vec::new();
    }
    let Some(common) = common.get_or_insert_with(|| read_common(source)).clone() else {
        return Vec::new();
    };
    let has = |flag: &str| flags.has(flag);
    let factor = |value: Option<f32>| value.filter(|v| v.is_finite() && *v != 0.0).unwrap_or(1.0);
    let mut map = |name: &str, kind| {
        root.read_tag_ref_path(name)
            .filter(|path| !path.is_empty())
            .and_then(|path| bitmap(&path, kind))
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let pass = if group == b"soso" {
        let detail_mask = option_index(root, "detail mask", &super::super::materials::CE_DETAIL_MASKS);
        let decal = has("alpha-blended decal");
        let scale = factor(root.read_real("detail map scale"));
        let base_scale = [
            factor(root.read_real("map u-scale")),
            factor(root.read_real("map v-scale")),
        ];
        let xbox_order = if has("multipurpose map uses OG Xbox channel order") { 1.0 } else { 0.0 };
        ModelPass {
            environment: false,
            detail_mask,
            detail_function: option_index(root, "detail function", &DETAIL_FUNCTIONS),
            detail_before_reflection: !has("detail after reflection"),
            alpha_test: !decal && !has("not alpha-tested"),
            decal,
            two_sided: has("two-sided"),
            base: map("base map", SamplerKind::Texture2D),
            detail: map("detail map", SamplerKind::Texture2D),
            multipurpose: map("multipurpose map", SamplerKind::Texture2D),
            reflection: map("reflection cube map", SamplerKind::Cube),
            base_scale,
            detail_scale: [scale, scale * factor(root.read_real("detail map v-scale"))],
            self_illumination: [0.0, 0.0, 0.0, xbox_order],
            change_color: color_channel(root, "change color source"),
            animation: Some(read_model_animation(root, base_scale, detail_mask)),
            translucency: real("translucency"),
            reflection_distances: [
                real("reflection falloff distance"),
                real("reflection cutoff distance"),
            ],
            perpendicular: ReflectionColor {
                brightness: real("perpendicular brightness"),
                color: rgb("perpendicular tint color"),
            },
            parallel: ReflectionColor {
                brightness: real("parallel brightness"),
                color: rgb("parallel tint color"),
            },
            common: &common,
        }
    } else {
        let alpha_tested = has("alpha-tested");
        let scale = factor(root.read_real("primary detail map scale"));
        ModelPass {
            environment: true,
            detail_mask: 0,
            detail_function: option_index(root, "detail map function", &DETAIL_FUNCTIONS),
            detail_before_reflection: true,
            alpha_test: alpha_tested,
            decal: false,
            two_sided: false,
            base: map("base map", SamplerKind::Texture2D),
            detail: map("primary detail map", SamplerKind::Texture2D),
            multipurpose: if alpha_tested {
                map("bump map", SamplerKind::Texture2D)
            } else {
                None
            },
            reflection: map("reflection cube map", SamplerKind::Cube),
            base_scale: [1.0; 2],
            detail_scale: [scale; 2],
            self_illumination: [0.0; 4],
            change_color: None,
            animation: None,
            translucency: 0.0,
            reflection_distances: [0.0; 2],
            // The reflection takes its colors from the specular section.
            perpendicular: ReflectionColor {
                brightness: real("perpendicular brightness"),
                color: rgb("perpendicular color"),
            },
            parallel: ReflectionColor {
                brightness: real("parallel brightness"),
                color: rgb("parallel color"),
            },
            common: &common,
        }
    };
    build_passes(pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE_GROUPS: [(&str, &[u8; 4]); 4] = [
        ("shader_transparent_glass", b"sgla"),
        ("shader_transparent_meter", b"smet"),
        ("shader_transparent_water", b"swat"),
        ("shader_transparent_plasma", b"spla"),
    ];

    /// With `BLAM_TEST_HCEEK`: every `ShaderModel` the game can ask for links
    /// with its stage.
    #[test]
    fn every_model_shader_links_with_its_stage() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let common = read_common(&source).expect("the kit's model shader translates");
        let dir = crate::core::test_kits::unique_temp_dir("ce_model_shaders");
        let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
        std::fs::write(&vertex, H2VertexStage::CeModel.source(H2_GLSL_VERSION)).unwrap();
        for (environment, mask, function, before) in (0..2).flat_map(|environment| {
            (0..9).flat_map(move |mask| {
                (0..3).flat_map(move |function| {
                    [true, false].map(move |before| (environment, mask, function, before))
                })
            })
        }) {
            let call = format!("ShaderModel(i, {environment}, {mask}, {function}, {before}, false)");
            std::fs::write(&pixel, common.entry(&call)).unwrap();
            let output = std::process::Command::new("glslangValidator")
                .arg("-l")
                .arg(&vertex)
                .arg(&pixel)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{call} does not link:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    /// With `BLAM_TEST_HCEEK`: the kit's camouflage reads and links.
    #[test]
    fn active_camo_links_with_its_stage() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let settings = read_ce_camo_settings(&source).expect("the kit's camo reads");
        assert!(settings.refraction > 0.0);
        assert!(settings.distortion.is_some());
        let pass = ce_camo_pass(&settings, false, &mut |_, _| None);
        let dir = crate::core::test_kits::unique_temp_dir("ce_active_camo");
        let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
        std::fs::write(&vertex, pass.stage.source(H2_GLSL_VERSION)).unwrap();
        std::fs::write(&pixel, &pass.fragment).unwrap();
        let output = std::process::Command::new("glslangValidator")
            .arg("-l")
            .arg(&vertex)
            .arg(&pixel)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    }

    /// With `BLAM_TEST_HCEEK`: every transparent shader in the kit translates
    /// and links with its stage.
    #[test]
    fn every_transparent_shader_links_with_its_stage() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
            eprintln!("skipping: glslangValidator is not on PATH");
            return;
        }
        let tags = std::path::PathBuf::from(root);
        let source = TagSource::LooseFolder {
            root: tags.clone(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let dir = crate::core::test_kits::unique_temp_dir("ce_transparent_shaders");
        let (vertex, pixel) = (dir.join("pass.vert"), dir.join("pass.frag"));
        let mut folders = vec![tags.clone()];
        let mut drawn = std::collections::HashSet::new();
        let mut shaders = 0;
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                let extension = path.extension().and_then(|name| name.to_str()).unwrap_or("");
                let Some((_, group)) = LAYER_GROUPS
                    .iter()
                    .chain(&SINGLE_GROUPS)
                    .find(|(name, _)| *name == extension)
                else {
                    continue;
                };
                let relative = path.strip_prefix(&tags).unwrap().with_extension("");
                let relative = relative.to_string_lossy().replace('/', "\\");
                let tag = load_referenced_tag_from_source(&source, &relative, extension, group).unwrap();
                shaders += 1;
                let passes = ce_transparent_passes(&source, tag.root(), group, &mut |_, kind| {
                    match kind {
                        SamplerKind::Cube => Some(Arc::new(H2Image::Cube {
                            size: 1,
                            faces: std::array::from_fn(|_| vec![0; 4]),
                        })),
                        SamplerKind::Volume => None,
                        SamplerKind::Texture2D => Some(flat(WHITE)),
                    }
                });
                assert!(!passes.is_empty(), "{} has no passes", path.display());
                for pass in passes {
                    if !drawn.insert((pass.stage, pass.fragment.clone())) {
                        continue;
                    }
                    std::fs::write(&vertex, pass.stage.source(H2_GLSL_VERSION)).unwrap();
                    std::fs::write(&pixel, &pass.fragment).unwrap();
                    let output = std::process::Command::new("glslangValidator")
                        .arg("-l")
                        .arg(&vertex)
                        .arg(&pixel)
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{} does not link:\n{}",
                        path.display(),
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
            }
        }
        assert!(shaders > 100, "only {shaders} shaders");
        eprintln!("{shaders} shaders, {} programs", drawn.len());
    }

    #[test]
    fn periodic_functions_keep_their_shape_and_range() {
        assert_eq!(periodic_evaluate(PERIODIC_ONE, 0.3), 1.0);
        assert_eq!(periodic_evaluate(PERIODIC_ZERO, 0.3), 0.0);
        // Cosine starts at its peak; a slide at nothing, and it wraps.
        assert!(periodic_evaluate(2, 0.0) > 0.99);
        assert!(periodic_evaluate(6, 0.0) < 0.01);
        for function in 2..12 {
            for step in 0..4096 {
                let value = periodic_evaluate(function, step as f32 * 0.037);
                assert!((0.0..=1.0).contains(&value), "{function} gave {value}");
            }
        }
    }

    fn map(waves: [TextureWave; 3], sources: [Option<usize>; 3]) -> CeMapAnimation {
        CeMapAnimation {
            row: 0,
            sources,
            waves,
            center: [0.5, 0.5],
            scale: [2.0, 3.0],
            offset: [0.0; 2],
            rotation: 0.0,
        }
    }

    #[test]
    fn a_full_turn_of_rotation_leaves_the_rows_square() {
        let still = TextureWave { function: PERIODIC_ONE, period: 1.0, phase: 0.0, scale: 360.0 };
        let idle = TextureWave { function: PERIODIC_ZERO, period: 1.0, phase: 0.0, scale: 1.0 };
        let animation = map([idle.clone(), idle, still], [None; 3]);
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        animation.apply(&mut rows, 1.0, &[0.0; 4]);
        assert_eq!(rows[0], [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(rows[1], [0.0, 3.0, 0.0, 0.0]);
        assert!(!animation.moves(&[0.0; 4]));
    }

    #[test]
    fn scrolling_moves_and_an_input_that_rests_does_not() {
        let scroll = TextureWave { function: 6, period: 2.0, phase: 0.0, scale: 1.0 };
        let idle = TextureWave { function: PERIODIC_ONE, period: 1.0, phase: 0.0, scale: 1.0 };
        let mut animation = map([scroll, idle.clone(), idle], [None; 3]);
        assert!(animation.moves(&[0.0; 4]));
        animation.sources[0] = Some(0);
        assert!(!animation.moves(&[0.0; 4]));
        assert!(animation.moves(&[1.0, 0.0, 0.0, 0.0]));
    }

    /// With `BLAM_TEST_HCEEK`: the blink reads `A out`, so eyes with no object
    /// behind them stay open and its output shuts them.
    #[test]
    fn an_eye_shader_reads_its_blink_from_an_object_function() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let tag = load_referenced_tag_from_source(
            &source,
            "characters\\marine\\shaders\\eyes_charles",
            "shader_model",
            b"soso",
        )
        .unwrap();
        let animation = read_map_animation(tag.root(), 0, [1.0; 2], [0.0; 2], 0.0);
        assert_eq!(animation.sources, [None, Some(0), None]);
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        animation.apply(&mut rows, 0.0, &[0.0; 4]);
        assert_eq!(rows[1][3], 0.0);
        animation.apply(&mut rows, 0.0, &[1.0, 0.0, 0.0, 0.0]);
        assert_eq!(rows[1][3], -0.06);
        assert!(animation.moves(&[0.0; 4]));
    }

    #[test]
    fn a_meter_follows_its_sources_and_an_unset_one_reads_one() {
        let meter = CeMeter {
            sources: [Some(0), None, Some(1), None],
            flash: [1.0, 0.5, 0.0],
            maximum: [0.0; 3],
            minimum: [0.0; 3],
            background: [0.0; 3],
            tint: [1.0; 3],
            transparency: None,
            negative: true,
        };
        let constants = meter.constants(&[0.25, 0.75, 0.0, 0.0]);
        assert_eq!(constants[0], [1.0, 0.5, 0.0, 0.25]);
        assert_eq!(constants[2][3], 0.75);
        assert_eq!(constants[1][3], 1.0 / 8.0);
        assert_eq!(constants[5], [-1.0, -0.5, -0.0, -1.0]);
    }

    #[test]
    fn plasma_rests_unseen_and_its_output_brings_it_up() {
        let noise = CePlasmaNoise { period: 2.0, direction: [1.0, 0.0, 0.0], scale: 3.0 };
        let plasma = CePlasma {
            intensity_source: Some(1),
            intensity_exponent: 1.0,
            offset_source: Some(1),
            offset_exponent: 2.0,
            offset_amount: 0.03,
            perpendicular: [1.0; 4],
            parallel: [1.0; 4],
            primary: noise.clone(),
            secondary: noise,
        };
        let animation = CeModelAnimation { plasma: Some(plasma), ..Default::default() };
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        animation.apply_rows(&mut rows, 4.0, &[0.0; 4]);
        assert_eq!(rows[7][3], 0.0);
        assert_eq!(rows[0], [3.0, 0.0, 0.0, 2.0]);
        assert!(!animation.moves_with(&[0.0; 4]));

        animation.apply_rows(&mut rows, 4.0, &[0.0, 0.5, 0.0, 0.0]);
        assert_eq!(rows[7][3], 0.5);
        assert!((rows[0][2] - 0.0075).abs() < 1e-6);
        assert!(animation.moves_with(&[0.0, 0.5, 0.0, 0.0]));
    }

    #[test]
    fn a_pass_says_which_outputs_it_reads() {
        let idle = TextureWave { function: PERIODIC_ONE, period: 1.0, phase: 0.0, scale: 1.0 };
        let map = map([idle.clone(), idle.clone(), idle], [None, Some(2), None]);
        let animation = CeModelAnimation { maps: vec![map], fade_source: Some(1), ..Default::default() };
        assert_eq!(animation.outputs_read(), [false, true, true, false]);
        assert_eq!(CeModelAnimation::default().outputs_read(), [false; 4]);
    }

    #[test]
    fn only_a_decal_shader_takes_the_depth_bias() {
        assert_eq!(decal_bias(true), TRANSPARENT_DECAL_DEPTH_BIAS);
        assert_eq!(decal_bias(false), 0.0);
        // A bias pulls toward the viewer, by what a 24 bit buffer can hold.
        const { assert!(MODEL_DECAL_DEPTH_BIAS < TRANSPARENT_DECAL_DEPTH_BIAS) };
        const { assert!(TRANSPARENT_DECAL_DEPTH_BIAS < 0.0) };
    }

    #[test]
    fn a_numeric_pass_shows_its_digit_of_its_place() {
        let digit = |digit, place| CeNumeric {
            map: String::new(),
            count: 10,
            limit: 60,
            digit,
            place,
            timer: false,
        };
        let digit_at = |value| digit(value, 1);
        // Half of 60 is 30, read off the first output for ten images.
        let outputs = [0.5, 0.0, 0.0, 0.0];
        assert!(digit(0, 0).shows(&outputs));
        assert!(digit(3, 1).shows(&outputs));
        assert!(!digit(3, 0).shows(&outputs));
        assert!(digit(0, 2).shows(&outputs));
        // Eight images read the fourth output instead.
        let octal = CeNumeric { count: 8, ..digit(1, 0) };
        assert!(octal.shows(&[0.0, 0.0, 0.0, 1.0 / 60.0]));
        assert!(!octal.shows(&[1.0 / 60.0, 0.0, 0.0, 0.0]));
        // A countdown ignores the object, and with no script stays at zero.
        let timer = |digit| CeNumeric { timer: true, ..digit_at(digit) };
        assert!(timer(0).shows(&outputs));
        assert!(!timer(3).shows(&outputs));
    }

    #[test]
    fn a_chicago_stage_reads_the_engine_tables() {
        // Add: next map, then the current map through the second product.
        let add = chicago_color_table(4);
        assert_eq!(add[0], 1.0);
        assert_eq!((add[3], add[9], add[10], add[11]), (1.0, 1.0, 1.0, 1.0));
        // Double multiply scales its only product.
        assert_eq!(chicago_color_table(3)[10], 2.0);
        // Subtract next map takes it from the current one.
        assert_eq!(chicago_alpha_table(8)[9], -1.0);
    }

    #[test]
    fn the_fade_stage_follows_the_blend_function() {
        let blend = |index| generic_fade_stage(index, 0, 3);
        // Alpha blend fades the alpha alone: a stage with no color output.
        assert!(blend(0).contains(",0,0,0,0,0,0,0,0,false,0,0,0,0,0,0,11,0,9,0,"), "{}", blend(0));
        // Multiply fades toward white.
        assert!(blend(1).contains(",11,0,19,0,1,0,19,1,false,"), "{}", blend(1));
        // Double multiply fades toward grey.
        assert!(blend(2).contains(",11,0,19,0,2,0,19,1,false,"), "{}", blend(2));
    }
}
