//! Halo CE's model shaders as draw passes.
//!
//! `shader_model` and a `shader_environment` on a model both draw through
//! `ShaderModel()` in the kit's `fx/model_common.h`. What feeds it follows
//! `rasterizer_model_draw_model_shader_pp` and
//! `rasterizer_model_draw_environment_shader_pp`; the lighting is
//! `default_object_lighting`, what the game falls back on with no level.

use std::sync::{Arc, LazyLock};

use blam_tags::{TagBlock, TagFieldData, TagStruct};

use super::h2_passes::{
    H2Blend, H2BlendEquation, H2BlendFactor, H2Compare, H2Cull, H2Depth, H2DrawPass, H2DrawTexture,
    H2Image, H2_GLSL_VERSION,
};
use super::h2_stage::{H2_TEXTURE_ROWS, H2VertexStage};
use super::load_referenced_tag_from_source;
use crate::core::ce_pixel_shader::{TranslatedFxShader, translate_fx};
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

const COMMON_FILE: &str = "model_common.h";
const GENERIC_FILE: &str = "transparent_generic_shader.psh";
const CHICAGO_FILE: &str = "chicago_shader.psh";
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
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 16) as f32 * 0.000_015_259_022
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
fn periodic_evaluate(function: usize, time: f32) -> f32 {
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

    /// With the object input a function reads, or 1 with none. A lone model's
    /// are all 0, as a resting object's.
    fn value(&self, source: bool, time: f32) -> f32 {
        let input = if source { 0.0 } else { 1.0 };
        periodic_evaluate(self.function, (self.phase + time) / self.period) * self.scale * input
    }

    fn moves(&self, source: bool) -> bool {
        !source && self.function != PERIODIC_ONE && self.function != PERIODIC_ZERO
    }
}

/// A map's transform as `shader_texture_animation_evaluate` works it out.
#[derive(Debug, Clone, PartialEq)]
struct CeMapAnimation {
    /// The first of its two texture rows.
    row: usize,
    /// Whether u, v and rotation read an object input.
    sources: [bool; 3],
    waves: [TextureWave; 3],
    center: [f32; 2],
    scale: [f32; 2],
    offset: [f32; 2],
    rotation: f32,
}

impl CeMapAnimation {
    fn moves(&self) -> bool {
        (0..3).any(|axis| self.waves[axis].moves(self.sources[axis]))
    }

    fn apply(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], time: f32) {
        let [u, v, r] = std::array::from_fn(|axis| self.waves[axis].value(self.sources[axis], time));
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
}

impl CeColorAnimation {
    fn moves(&self) -> bool {
        self.function != PERIODIC_ONE && self.function != PERIODIC_ZERO && self.lower != self.upper
    }

    fn apply(&self, value: &mut [f32; 4], time: f32, change_colors: &[[f32; 3]; 4]) {
        let wave = periodic_evaluate(self.function, time / self.period);
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
}

impl CeModelAnimation {
    pub(crate) fn moves(&self) -> bool {
        self.maps.iter().any(CeMapAnimation::moves) || self.colors.iter().any(CeColorAnimation::moves)
    }

    pub(crate) fn apply_rows(&self, rows: &mut [[f32; 4]; H2_TEXTURE_ROWS], time: f32) {
        self.maps.iter().for_each(|map| map.apply(rows, time));
    }

    /// The color `register` holds, if it is one of these.
    pub(crate) fn constant(
        &self,
        register: u32,
        value: &mut [f32; 4],
        time: f32,
        change_colors: &[[f32; 3]; 4],
    ) {
        for color in self.colors.iter().filter(|color| color.register == register) {
            color.apply(value, time, change_colors);
        }
    }
}

/// `model_common.h` translated, or `None` where the kit has no shaders.
pub(super) type CeModelCommon = Arc<TranslatedFxShader>;

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
    Arc::new(H2Image::Flat(super::materials::TextureImage {
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
fn option_index(root: TagStruct<'_>, field: &str, options: &[&str]) -> i32 {
    let name = root.read_enum_name(field);
    options.iter().position(|option| name.as_deref() == Some(*option)).unwrap_or(0) as i32
}

const DETAIL_FUNCTIONS: [&str; 3] = ["double/biased multiply", "multiply", "double/biased add"];
const COLOR_SOURCES: [&str; 5] = ["none", "A", "B", "C", "D"];
/// An object function's outputs, as animation and fade fields name them.
const FUNCTION_SOURCES: [&str; 5] = ["none", "A out", "B out", "C out", "D out"];
const PERIODIC_FUNCTIONS: [&str; 12] = [
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

fn nonzero(value: Option<f32>, default: f32) -> f32 {
    value.filter(|value| *value != 0.0).unwrap_or(default)
}

fn read_map_animation(
    root: TagStruct<'_>,
    row: usize,
    scale: [f32; 2],
    offset: [f32; 2],
    rotation: f32,
) -> CeMapAnimation {
    let source = |field: &str| option_index(root, field, &FUNCTION_SOURCES) != 0;
    let center = root.read_point2d("rotation-animation center");
    CeMapAnimation {
        row,
        sources: [
            source("u-animation source"),
            source("v-animation source"),
            source("rotation-animation source"),
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
        colors: (detail_mask == 0)
            .then(|| CeColorAnimation {
                register: 0,
                function: option_index(root, "animation function", &PERIODIC_FUNCTIONS) as usize,
                period: nonzero(root.read_real("animation period"), 1.0),
                lower: [lower.red, lower.green, lower.blue, 0.0],
                upper: [upper.red, upper.green, upper.blue, 0.0],
                animates_alpha: false,
                channel: color_channel(root, "color source"),
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

/// `srcblend_table`, `destblend_table` and `blendop_table`.
fn framebuffer_blend(function: i32) -> H2Blend {
    use H2BlendFactor::{DstColor, One, OneMinusSrcAlpha, SrcAlpha, SrcColor, Zero};
    let (equation, source, destination) = match function {
        1 => (H2BlendEquation::Add, DstColor, Zero),
        2 => (H2BlendEquation::Add, DstColor, SrcColor),
        3 => (H2BlendEquation::Add, One, One),
        4 => (H2BlendEquation::ReverseSubtract, One, One),
        5 => (H2BlendEquation::Min, One, One),
        6 => (H2BlendEquation::Max, One, One),
        7 => (H2BlendEquation::Add, One, OneMinusSrcAlpha),
        _ => (H2BlendEquation::Add, SrcAlpha, OneMinusSrcAlpha),
    };
    H2Blend { equation, source, destination, constant: [0; 4] }
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
                TagFieldData::LongEnum { value, .. } => value as i32,
                _ => return None,
            };
            let keep = started;
            started |= field.clean_name() == after;
            keep.then_some(value)
        })
        .collect()
}

/// The `color1` of a stage, then its animated `color0`, alpha first as stored.
fn argb(root: TagStruct<'_>, name: &str) -> [f32; 4] {
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
        let names = element.read_flag_names("flags").unwrap_or_default();
        let has = |flag: &str| names.iter().any(|(_, name)| name == flag);
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

/// A transparent pass's state, whichever group fills its constants.
fn transparent_pass(
    root: TagStruct<'_>,
    first_map: i32,
    flags: &[String],
    fragment: String,
    textures: Vec<H2DrawTexture>,
    constants: Vec<(u32, [f32; 4])>,
    animation: CeModelAnimation,
) -> H2DrawPass {
    let has = |flag: &str| flags.iter().any(|name| name == flag);
    let depth = H2Depth { compare: Some(H2Compare::LessOrEqual), write: false };
    let stage = transparent_stage(first_map, has("first map is in screenspace"));
    let mut pass = H2DrawPass::new(stage, fragment, depth);
    pass.channels = [true, true, true, false];
    pass.blend =
        Some(framebuffer_blend(option_index(root, "framebuffer blend function", &BLEND_FUNCTIONS)));
    pass.cull = if has("two-sided") { H2Cull::None } else { H2Cull::Back };
    pass.textures = textures;
    pass.constants = constants;
    pass.texture_rows = identity_map_rows();
    // A fade source reads an object function, which rests at 0 on a lone
    // model: damage and shield overlays stay out of sight, as on a whole one.
    if option_index(root, "framebuffer fade source", &FUNCTION_SOURCES) != 0 {
        pass.texture_rows[8][0] = 0.0;
    }
    pass.ce_animation = Some(Arc::new(animation));
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
    flags: &[String],
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
        let names = stage.read_flag_names("flags").unwrap_or_default();
        let has = |flag: &str| names.iter().any(|(_, name)| name == flag);
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
        });
    }
    let blend = option_index(root, "framebuffer blend function", &BLEND_FUNCTIONS);
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
    let reference = flags.iter().any(|name| name == "alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation { maps, colors };
    Ok(transparent_pass(root, first_map, flags, fragment, textures, constants, animation))
}

fn chicago_pass(
    directory: &std::path::Path,
    root: TagStruct<'_>,
    flags: &[String],
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
        let names = map.read_flag_names("flags").unwrap_or_default();
        replicate[index] = f32::from(u8::from(names.iter().any(|(_, name)| name == "alpha replicate")));
    }
    let blend = match option_index(root, "framebuffer blend function", &BLEND_FUNCTIONS) {
        0 => 0,
        1 | 5 => 1,
        2 => 2,
        7 => 4,
        _ => 3,
    };
    let fade = option_index(root, "framebuffer fade mode", &FADE_MODES);
    let custom = flags.iter().any(|name| name == "custom edition blending");
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
    let reference = flags.iter().any(|name| name == "alpha-tested").then_some(ALPHA_REFERENCE);
    let fragment = shader.entry_tested("pixel_main(i)", reference);
    let animation = CeModelAnimation { maps: animations, colors: Vec::new() };
    Ok(transparent_pass(root, first_map, flags, fragment, textures, constants, animation))
}

fn every_flag(root: TagStruct<'_>) -> Vec<String> {
    root.fields()
        .filter(|field| field.clean_name() == "flags" || field.clean_name() == "extra flags")
        .filter_map(|field| match field.value()? {
            TagFieldData::ByteFlags { names, .. }
            | TagFieldData::WordFlags { names, .. }
            | TagFieldData::LongFlags { names, .. } => Some(names),
            _ => None,
        })
        .flatten()
        .map(|(_, name)| name)
        .collect()
}

/// A transparent shader's passes: its extra layers, then itself. Empty for
/// any other group, or where the kit has no shaders.
pub(super) fn ce_transparent_passes(
    source: &TagSource,
    root: TagStruct<'_>,
    group: &[u8; 4],
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Vec<H2DrawPass> {
    if !matches!(group, b"sotr" | b"schi" | b"scex") {
        return Vec::new();
    }
    let Some(directory) = shader_directory(source) else {
        return Vec::new();
    };
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
    let flags = every_flag(root);
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

const LAYER_GROUPS: [(&str, &[u8; 4]); 3] = [
    ("shader_transparent_generic", b"sotr"),
    ("shader_transparent_chicago", b"schi"),
    ("shader_transparent_chicago_extended", b"scex"),
];

/// The passes of a `shader_model` or `shader_environment`, empty for any other
/// group and where the kit has no shaders, which keeps the slots drawing.
pub(super) fn ce_model_passes(
    source: &TagSource,
    root: TagStruct<'_>,
    group: &[u8; 4],
    flags: &[String],
    common: &mut Option<Option<CeModelCommon>>,
    bitmap: &mut impl FnMut(&str, SamplerKind) -> Option<Arc<H2Image>>,
) -> Vec<H2DrawPass> {
    if !matches!(group, b"soso" | b"senv") {
        return Vec::new();
    }
    let Some(common) = common.get_or_insert_with(|| read_common(source)).clone() else {
        return Vec::new();
    };
    let has = |flag: &str| flags.iter().any(|name| name == flag);
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
        let detail_mask = option_index(root, "detail mask", &super::materials::CE_DETAIL_MASKS);
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
                let Some((_, group)) = LAYER_GROUPS.iter().find(|(name, _)| *name == extension) else {
                    continue;
                };
                let relative = path.strip_prefix(&tags).unwrap().with_extension("");
                let relative = relative.to_string_lossy().replace('/', "\\");
                let tag = load_referenced_tag_from_source(&source, &relative, extension, group).unwrap();
                shaders += 1;
                let passes = ce_transparent_passes(&source, tag.root(), group, &mut |_, _| None);
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

    fn map(waves: [TextureWave; 3], sources: [bool; 3]) -> CeMapAnimation {
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
        let animation = map([idle.clone(), idle, still], [false; 3]);
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        animation.apply(&mut rows, 1.0);
        assert_eq!(rows[0], [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(rows[1], [0.0, 3.0, 0.0, 0.0]);
        assert!(!animation.moves());
    }

    #[test]
    fn scrolling_moves_and_an_input_that_rests_does_not() {
        let scroll = TextureWave { function: 6, period: 2.0, phase: 0.0, scale: 1.0 };
        let idle = TextureWave { function: PERIODIC_ONE, period: 1.0, phase: 0.0, scale: 1.0 };
        let mut animation = map([scroll, idle.clone(), idle], [false; 3]);
        assert!(animation.moves());
        animation.sources[0] = true;
        assert!(!animation.moves());
    }

    /// With `BLAM_TEST_HCEEK`: the blink reads `A out`, so a lone model's eyes
    /// stay open, and the jitter beside it still moves them.
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
        assert_eq!(animation.sources, [false, true, false]);
        let mut rows = [[0.0; 4]; H2_TEXTURE_ROWS];
        animation.apply(&mut rows, 0.0);
        assert_eq!(rows[1][3], 0.0);
        assert!(animation.moves());
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
