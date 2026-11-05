//! Halo 3's area screen effects: what an effect does to the whole picture while
//! the camera is near it. `screen_effect_sample` folds all of them into one set
//! of settings, which the final composite applies before the tone curve.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce::attachments::elements;
use super::ce::lights::{CeFlareView, posed_on};
use super::h2::functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{H3EffectOnsets, H3EffectTimes, for_each_attached_part};
use super::*;
use super::renderer::REACH_SCREEN_NOISE_BITMAP;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// `build_saturation_matrix`'s weights.
const LUMINANCE: [f32; 3] = [0.212_656, 0.715_158, 0.072_185_6];
/// `set_hue_saturaton_and_color_filters`' `scale`.
const CONTRAST_SCALE: f32 = 10.0;

/// `render_postprocess_blur`'s `flt`s, as ODST has them.
const BLUR_OFFSET: f32 = -0.5;
const BLUR_FLOOR: f32 = 0.35;
const BLUR_BLEND: f32 = 300.0;
/// `shader_count`'s limit in `s_screen_effect_shader_sample_result`.
const MOST_SCREEN_SHADERS: usize = 8;

/// What `evaluate_falloff` gives a function for its range.
const FALLOFF_RANGE: f32 = 0.5;

type Matrix = [[f32; 4]; 4];

/// `s_screen_effect_settings`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H3ScreenSettings {
    exposure_boost: f32,
    /// Reach's `exposure deboost`, stops taken off as `boost` adds them.
    exposure_deboost: f32,
    hue_left: f32,
    hue_right: f32,
    saturate: f32,
    desaturate: f32,
    contrast_enhance: f32,
    gamma_enhance: f32,
    gamma_reduce: f32,
    /// Reach's `bright noise` and `dark noise`, for the final composite.
    bright_noise: f32,
    dark_noise: f32,
    color_filter: [f32; 3],
    color_floor: [f32; 3],
    /// ODST's `horizontal blur` and `vertical blur`.
    blur: [f32; 2],
    /// ODST's `fov in` and `fov out`, in stops of the view's tangent.
    fov: [f32; 2],
    /// ODST's `vision mode`.
    vision: f32,
    /// ODST's `motion suck`: on a definition its amount, and on what is sampled
    /// the way the view seems to have moved.
    suck: f32,
    suck_toward: [f32; 3],
}

impl Default for H3ScreenSettings {
    /// `set_defaults`.
    fn default() -> Self {
        Self {
            exposure_boost: 0.0,
            exposure_deboost: 0.0,
            hue_left: 0.0,
            hue_right: 0.0,
            saturate: 0.0,
            desaturate: 0.0,
            contrast_enhance: 0.0,
            gamma_enhance: 0.0,
            gamma_reduce: 0.0,
            bright_noise: 0.0,
            dark_noise: 0.0,
            color_filter: [1.0; 3],
            color_floor: [0.0; 3],
            blur: [0.0; 2],
            fov: [0.0; 2],
            vision: 0.0,
            suck: 0.0,
            suck_toward: [0.0; 3],
        }
    }
}

impl H3ScreenSettings {
    /// `accumulate_settings`: the strongest of each, not their sum.
    fn accumulate(&mut self, other: &Self, falloff: f32, toward: [f32; 3]) {
        let falloff = falloff.clamp(0.0, 1.0);
        let most = |mine: &mut f32, theirs: f32| *mine = mine.max(falloff * theirs);
        most(&mut self.exposure_boost, other.exposure_boost);
        most(&mut self.exposure_deboost, other.exposure_deboost);
        most(&mut self.hue_left, other.hue_left);
        most(&mut self.hue_right, other.hue_right);
        most(&mut self.saturate, other.saturate);
        most(&mut self.desaturate, other.desaturate);
        most(&mut self.contrast_enhance, other.contrast_enhance);
        most(&mut self.gamma_enhance, other.gamma_enhance);
        most(&mut self.gamma_reduce, other.gamma_reduce);
        most(&mut self.bright_noise, other.bright_noise);
        most(&mut self.dark_noise, other.dark_noise);
        for channel in 0..3 {
            let toward = falloff * other.color_filter[channel] + (1.0 - falloff);
            self.color_filter[channel] = self.color_filter[channel].min(toward);
            most(&mut self.color_floor[channel], other.color_floor[channel]);
        }
        for axis in 0..2 {
            most(&mut self.blur[axis], other.blur[axis]);
            most(&mut self.fov[axis], other.fov[axis]);
        }
        most(&mut self.vision, other.vision);
        // Unlike all the rest, this one adds up.
        let length = toward.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        let unit = if length >= 0.0001 { toward.map(|axis| axis / length) } else { toward };
        for axis in 0..3 {
            self.suck_toward[axis] += unit[axis] * other.suck * falloff;
        }
    }

    pub(crate) fn vision(&self) -> f32 {
        self.vision
    }

    /// `distortion_apply`'s motion, if any.
    pub(crate) fn suck(&self) -> Option<[f32; 3]> {
        (self.suck_toward != [0.0; 3]).then_some(self.suck_toward)
    }

    /// `c_player_view::render_postprocess_blur`: the blurred copy's spread by
    /// axis, and how much of it covers the picture. None with neither blur on.
    pub(crate) fn blur(&self) -> Option<([f32; 2], f32)> {
        let [horizontal, vertical] = self.blur;
        if horizontal <= 0.0 && vertical <= 0.0 {
            return None;
        }
        let spread = self.blur.map(|blur| (blur + BLUR_OFFSET).max(BLUR_FLOOR));
        Some((spread, (horizontal.max(vertical) * BLUR_BLEND / 255.0).clamp(0.0, 1.0)))
    }

    /// How much wider the view is than the preview's lens, by
    /// `screen_effect_apply_field_of_view_scale`.
    pub(crate) fn view_scale(&self, half_lens: f32) -> f32 {
        let [fov_in, fov_out] = self.fov;
        if (fov_out - fov_in).abs() <= 0.0001 {
            return 1.0;
        }
        let half = (2f32.powf(fov_out - fov_in) * half_lens.tan()).atan();
        let whole = (half * 2.0).clamp(1f32.to_radians(), 150f32.to_radians());
        (whole * 0.5).tan() / half_lens.tan()
    }

    /// `set_hue_saturaton_and_color_filters`, for a color on the left of it.
    pub(crate) fn color_matrix(&self) -> Matrix {
        if *self == Self::default() {
            return identity();
        }
        let hue = (self.hue_right - self.hue_left).rem_euclid(360.0);
        let saturation = (self.saturate - self.desaturate + 1.0).clamp(-10.0, 10.0);
        let contrast = self.contrast_enhance * self.contrast_enhance;
        let [red, green, blue] = self.color_filter;
        let gain = 1.0 + CONTRAST_SCALE * contrast;
        let mut filters = scaling([red * gain, green * gain, blue * gain]);
        filters[3] = [
            -self.color_floor[0] - contrast,
            -self.color_floor[1] - contrast,
            -self.color_floor[2] - contrast,
            1.0,
        ];
        product(product(saturation_matrix(saturation), filters), hue_matrix(hue))
    }

    /// `postprocess_player_view`'s contrast.
    pub(crate) fn gamma(&self) -> f32 {
        (1.0 + self.gamma_enhance - self.gamma_reduce).clamp(0.01, 10.0)
    }

    /// How much the picture brightens, by stops of exposure.
    pub(crate) fn exposure(&self) -> f32 {
        2f32.powf(self.exposure_boost)
    }

    /// Reach's `c_player_view::render`: the stops it hands
    /// `setup_camera_fx_parameters`, boost less deboost.
    pub(crate) fn stops(&self) -> f32 {
        self.exposure_boost - self.exposure_deboost
    }

    /// Reach's bright and dark noise, for `apply_noise`.
    pub(crate) fn noise(&self) -> [f32; 2] {
        [self.bright_noise, self.dark_noise]
    }
}

fn identity() -> Matrix {
    scaling([1.0; 3])
}

fn scaling([x, y, z]: [f32; 3]) -> Matrix {
    [[x, 0.0, 0.0, 0.0], [0.0, y, 0.0, 0.0], [0.0, 0.0, z, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn product(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|row| std::array::from_fn(|column| (0..4).map(|at| a[row][at] * b[at][column]).sum()))
}

/// `build_saturation_matrix`.
fn saturation_matrix(saturation: f32) -> Matrix {
    let rest = 1.0 - saturation;
    let [red, green, blue] = LUMINANCE.map(|weight| rest * weight);
    [
        [red + saturation, red, red, 0.0],
        [green, green + saturation, green, 0.0],
        [blue, blue, blue + saturation, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn x_rotation(sin: f32, cos: f32) -> Matrix {
    [[1.0, 0.0, 0.0, 0.0], [0.0, cos, sin, 0.0], [0.0, -sin, cos, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn y_rotation(sin: f32, cos: f32) -> Matrix {
    [[cos, 0.0, -sin, 0.0], [0.0, 1.0, 0.0, 0.0], [sin, 0.0, cos, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn z_rotation(sin: f32, cos: f32) -> Matrix {
    [[cos, sin, 0.0, 0.0], [-sin, cos, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

fn z_shear(x: f32, y: f32) -> Matrix {
    [[1.0, 0.0, x, 0.0], [0.0, 1.0, y, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
}

/// `build_hue_matrix`: a turn about the grey axis that keeps luminance.
fn hue_matrix(hue: f32) -> Matrix {
    let root = 2f32.sqrt().recip();
    let (y_sin, y_cos) = (-(3f32.sqrt().recip()), 2f32.sqrt() / 3f32.sqrt());
    let tilted = product(x_rotation(root, root), y_rotation(y_sin, y_cos));
    let [x, y, z]: [f32; 3] = std::array::from_fn(|axis| (0..3).map(|at| LUMINANCE[at] * tilted[at][axis]).sum());
    let (shear_x, shear_y) = (x / z, y / z);
    let (sin, cos) = hue.to_radians().sin_cos();
    let mut result = product(tilted, z_shear(shear_x, shear_y));
    result = product(result, z_rotation(sin, cos));
    result = product(result, z_shear(-shear_x, -shear_y));
    result = product(result, y_rotation(-y_sin, y_cos));
    product(result, x_rotation(-root, root))
}

/// `s_single_screen_effect_definition`.
#[derive(Debug)]
struct H3ScreenDefinition {
    settings: H3ScreenSettings,
    radius: f32,
    /// ODST's. `postprocess` has it follow the last when they play in turn.
    delay: f32,
    /// Reach's `screen_effect_is_visible` waits past even a delay of 0.
    strict_delay: bool,
    lifetime: f32,
    outside_radius: bool,
    /// ODST's `disable camera falloffs`: neither distance nor angle counts.
    no_camera_falloffs: bool,
    distance_falloff: Option<TagFunction>,
    time_falloff: Option<TagFunction>,
    angle_falloff: Option<TagFunction>,
    object_falloff: Option<H3ObjectFalloff>,
    /// Its tag was last saved before `object falloff` existed.
    predates_object_falloff: bool,
    /// ODST's `shader effect`, with its `render layer` and `sort order`.
    shader: Option<(String, ScreenLayer, i32)>,
}

/// `render layer`: where `render_screen_shaders` draws a `shader_screen`.
/// Pre-UI and post-UI draw last, either side of the UI; Reach's
/// pre-transparents draws in `c_player_view::render`, after the static
/// lighting and before the lights, the transparents and the postprocess (the
/// 360's 0x8302A358 called with layer 2). Its accumulate sorts by `sort order`
/// alone; each draw takes its own layer's, so sorting by layer first keeps
/// each layer's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScreenLayer {
    PreUi,
    PostUi,
    PreTransparents,
}

impl ScreenLayer {
    fn read(root: TagStruct<'_>) -> Self {
        match root.read_enum_option("render layer").as_deref() {
            Some("post-UI") => Self::PostUi,
            Some("pre-transparents") => Self::PreTransparents,
            _ => Self::PreUi,
        }
    }
}

/// A `shader_screen` to draw this frame, at this much of its effect.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H3ScreenShader {
    pub(crate) path: String,
    pub(crate) alpha: f32,
    layer: ScreenLayer,
    order: i32,
}

impl H3ScreenShader {
    /// Drawn at the end of the frame. Nothing the preview loads uses Reach's
    /// pre-transparents layer (only night vision and the sniper's zoom, from
    /// a unit's or weapon's `screen effects`), so it has no pass of its own
    /// and is left out rather than drawn over everything.
    pub(crate) fn at_frame_end(&self) -> bool {
        self.layer != ScreenLayer::PreTransparents
    }
}

/// `s_screen_effect_shader_sample_result::accumulate_settings`: ahead of the
/// first that sorts at or past it, and no more than the game keeps.
fn insert_shader(shaders: &mut Vec<H3ScreenShader>, shader: H3ScreenShader) {
    if shaders.len() >= MOST_SCREEN_SHADERS {
        return;
    }
    let before = shaders.iter().position(|known| (known.layer, known.order) >= (shader.layer, shader.order));
    shaders.insert(before.unwrap_or(shaders.len()), shader);
}

/// ODST's `screen_effect_scalar_object_function_struct`.
#[derive(Debug)]
struct H3ObjectFalloff {
    input: String,
    range: String,
    mapping: TagFunction,
}

impl H3ScreenDefinition {
    /// `screen_effect_is_active`, for a viewer of what it hangs on.
    /// Reach's (`is_visible`, the 360's 0x83033AB0) is strict on the delay
    /// with no exception for none.
    fn active(&self, age: f32) -> bool {
        let begun = (self.delay == 0.0 && !self.strict_delay) || age > self.delay;
        self.lifetime <= 0.0 || (begun && age <= self.delay + self.lifetime)
    }

    /// `evaluate_falloff`.
    fn falloff(&self, distance: f32, angle: f32, time: f32, values: Option<&H2FunctionValues>) -> f32 {
        if !self.outside_radius && !self.no_camera_falloffs && distance > self.radius {
            return 0.0;
        }
        let at = |function: &Option<TagFunction>, input: f32| {
            function.as_ref().map_or(1.0, |function| function.evaluate(input.clamp(0.0, 1.0), FALLOFF_RANGE))
        };
        let time_input = if self.lifetime > f32::EPSILON { (time - self.delay) / self.lifetime } else { time };
        let camera = if self.no_camera_falloffs {
            1.0
        } else {
            at(&self.distance_falloff, distance / self.radius.max(f32::EPSILON))
                * at(&self.angle_falloff, angle / std::f32::consts::PI)
        };
        let object = self.object_falloff.as_ref().map_or(1.0, |falloff| {
            let read = |name: &str| values.and_then(|values| values.get(name)).copied().unwrap_or(0.0);
            // With no input named, it reads how far through its life it is.
            let input = if falloff.input.is_empty() { time_input } else { read(&falloff.input) };
            let range = if falloff.range.is_empty() { FALLOFF_RANGE } else { read(&falloff.range) };
            falloff.mapping.evaluate(input.clamp(0.0, 1.0), range)
        });
        camera * at(&self.time_falloff, time_input) * object
    }
}

/// Whether an effect shows to a third person viewing what it hangs on: ODST's
/// options say `first person` and `unattached` and carry the old name behind,
/// and any of the three narrows it to those it names.
fn shows_in_third_person(flagged: impl Fn(&str) -> bool) -> bool {
    !(flagged("unattached") || flagged("first person") || flagged("third person")) || flagged("third person")
}

/// Reach's `is_visible` (the 360's 0x83033AB0) for a viewer who is nobody's
/// unit: `render_object_is_third_person` (0x82752FC0) and its first person
/// twin hold only for the viewer's own unit, so an effect naming a person
/// shows only if it also names `unattached`.
fn shows_to_no_unit(flagged: impl Fn(&str) -> bool) -> bool {
    !(flagged("unattached") || flagged("first person") || flagged("third person")) || flagged("unattached")
}

fn read_screen_shader(source: &TagSource, element: TagStruct<'_>) -> Option<(String, ScreenLayer, i32)> {
    let path = element.read_tag_ref_path("shader effect").filter(|path| !path.is_empty())?;
    let tag = load_referenced_tag_from_source(source, &path, "shader_screen", b"rmss").ok()?;
    let root = tag.root();
    let layer = ScreenLayer::read(root);
    let order = root.read_int_any("sort order").unwrap_or(0) as i32;
    Some((path, layer, order))
}

fn read_definition(source: &TagSource, element: TagStruct<'_>, delay: f32) -> Option<H3ScreenDefinition> {
    let flags = element.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let reach = source.game() == Some(GameId::HaloReach);
    // The preview's camera is a third person's, on nothing.
    let shows = if reach { shows_to_no_unit(flagged) } else { shows_in_third_person(flagged) };
    if flagged("debug disable") || !shows {
        return None;
    }
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let reach_real = |name: &str| if reach { real(name) } else { 0.0 };
    let color = |name: &str| {
        let color = element.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let falloff = |name: &str| element.descend(name).and_then(|holder| tag_function_at(&holder, "Mapping"));
    let object_falloff = element.descend("object falloff").and_then(|holder| {
        Some(H3ObjectFalloff {
            input: holder.read_string_id("Input Variable").unwrap_or_default(),
            range: holder.read_string_id("Range Variable").unwrap_or_default(),
            mapping: tag_function_at(&holder, "Mapping")?,
        })
    });
    Some(H3ScreenDefinition {
        settings: H3ScreenSettings {
            exposure_boost: real("exposure boost"),
            exposure_deboost: reach_real("exposure deboost"),
            hue_left: real("hue left"),
            hue_right: real("hue right"),
            saturate: real("saturation"),
            desaturate: real("desaturation"),
            contrast_enhance: real("contrast enhance"),
            gamma_enhance: real("gamma enhance"),
            gamma_reduce: real("gamma reduce"),
            bright_noise: reach_real("bright noise"),
            dark_noise: reach_real("dark noise"),
            color_filter: color("color filter"),
            color_floor: color("color floor"),
            blur: [real("horizontal blur"), real("vertical blur")],
            fov: [real("fov in"), real("fov out")],
            vision: real("vision mode"),
            suck: real("motion suck"),
            suck_toward: [0.0; 3],
        },
        radius: real("maximum distance"),
        delay,
        strict_delay: reach,
        lifetime: real("lifetime"),
        outside_radius: flagged("allow effect outside radius"),
        no_camera_falloffs: flagged("disable camera falloffs"),
        distance_falloff: falloff("distance falloff"),
        time_falloff: falloff("time falloff"),
        angle_falloff: falloff("angle falloff"),
        object_falloff,
        predates_object_falloff: element.descend("object falloff").is_none(),
        shader: read_screen_shader(source, element),
    })
}

/// `s_area_screen_effect_definition::postprocess`: played in turn, each starts
/// as the last ends, whatever its own `delay` says.
fn read_definitions(source: &TagSource, root: TagStruct<'_>) -> Vec<H3ScreenDefinition> {
    let flags = root.read_flag_names("global flags").unwrap_or_default();
    let in_turn = flags.iter().any(|(_, name)| name.starts_with("play sequentially"));
    let mut start = 0.0;
    let mut definitions = Vec::new();
    for element in elements(root, "screen effects") {
        let lifetime = element.read_real("lifetime").unwrap_or(0.0);
        let delay = if in_turn { start } else { element.read_real("delay").unwrap_or(0.0) };
        start += lifetime;
        definitions.extend(read_definition(source, element, delay));
    }
    definitions
}

/// `screen_effect_evaluate_falloff`: on a live object ODST multiplies by a
/// function that nothing fills in for a tag older than the field, and one with
/// no data is 0. `on_new` only helps elements made since. Halo 3 has no such
/// function, and what is on no object never reads it.
fn shows_on_object(odst: bool, predates_object_falloff: bool, unattached: bool) -> bool {
    !odst || !predates_object_falloff || unattached
}

/// An `area_screen_effect` on an effect of an object.
#[derive(Debug, Clone)]
pub(crate) struct H3ScreenEffectAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    scale_by: String,
    times: Arc<H3EffectTimes>,
    event: usize,
    definitions: Arc<Vec<H3ScreenDefinition>>,
}

impl H3ScreenEffectAt {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// The object functions its object falloffs read.
    pub(crate) fn turns_on_vision(&self) -> bool {
        self.definitions.iter().any(|definition| definition.settings.vision > 0.0)
    }

    pub(crate) fn sucks(&self) -> bool {
        self.definitions.iter().any(|definition| definition.settings.suck != 0.0)
    }

    /// Whether any of it adds Reach's bright or dark noise.
    pub(crate) fn adds_noise(&self) -> bool {
        self.definitions.iter().any(|definition| definition.settings.noise().iter().any(|amount| *amount > 0.0))
    }

    pub(crate) fn object_inputs(&self) -> impl Iterator<Item = &str> {
        let falloffs = self.definitions.iter().filter_map(|definition| definition.object_falloff.as_ref());
        falloffs.flat_map(|falloff| [falloff.input.as_str(), falloff.range.as_str()]).filter(|name| !name.is_empty())
    }
}

pub(super) fn read_h3_screen_effects(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3ScreenEffectAt> {
    let mut effects = Vec::new();
    let group = u32::from_be_bytes(*b"sefc");
    for_each_attached_part(source, object, |part| {
        if part.group != group {
            return;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, "area_screen_effect", b"sefc") else {
            return;
        };
        let odst = source.game() == Some(GameId::Halo3Odst);
        let mut definitions = read_definitions(source, tag.root());
        definitions.retain(|definition| {
            shows_on_object(odst, definition.predates_object_falloff, part.unattached)
        });
        let definitions = Arc::new(definitions);
        if definitions.is_empty() {
            return;
        }
        for marker in &part.effect.locations(markers, &part.marker, &frame) {
            let (place, node) = frame(marker);
            effects.push(H3ScreenEffectAt {
                object: owner,
                node,
                position: place.position,
                scale_by: part.effect.scale_by.clone(),
                times: Arc::clone(&part.times),
                event: part.event,
                definitions: Arc::clone(&definitions),
            });
        }
    });
    effects
}

/// `rasterizer_globals`' default bitmap that Reach's final composite reads
/// its bright and dark noise from (sampler 7 of MCC `sub_140841DE0`).
pub(super) fn read_reach_screen_noise(source: &TagSource) -> Option<super::materials::H3Bitmap> {
    let globals =
        load_referenced_tag_from_source(source, "globals\\rasterizer_globals", "rasterizer_globals", b"rasg").ok()?;
    let defaults = elements(globals.root(), "default bitmaps");
    let noise = defaults.get(REACH_SCREEN_NOISE_BITMAP)?;
    let path = noise.read_tag_ref_path("default bitmaps").filter(|path| !path.is_empty())?;
    super::materials::read_h3_bitmap(source, &path, 0, SamplerKind::Texture2D)
}

/// Every `shader_screen` these can draw, as materials of no batch, which gives
/// them the textures and passes of any other.
pub(super) fn add_screen_shader_materials(preview: &mut RenderModelPreview) {
    let group = u32::from_be_bytes(*b"rmss");
    let wanted: Vec<String> = preview
        .h3_screen_effects
        .iter()
        .flat_map(|effect| effect.definitions.iter())
        .filter_map(|definition| definition.shader.as_ref().map(|(path, ..)| path.clone()))
        .collect();
    for shader_path in wanted {
        let present = |material: &RenderModelPreviewMaterial| material.shader_group == group && material.shader_path == shader_path;
        if !preview.materials.iter().any(present) {
            preview.materials.push(RenderModelPreviewMaterial { shader_path, shader_group: group, ..Default::default() });
        }
    }
}

/// `screen_effect_sample`: what every running effect comes to from this camera.
pub(crate) fn h3_screen_settings<'a>(
    effects: &[H3ScreenEffectAt],
    values: impl Fn(u8) -> Option<&'a H2FunctionValues>,
    onsets: &mut H3EffectOnsets,
    bones: Option<&[[f32; 4]]>,
    view: &CeFlareView,
    time: f32,
) -> (H3ScreenSettings, Vec<H3ScreenShader>) {
    let mut settings = H3ScreenSettings::default();
    let mut shaders = Vec::new();
    for effect in effects {
        let scope = values(effect.object);
        let on = effect.scale_by.is_empty()
            || scope.and_then(|values| values.get(&effect.scale_by)).is_some_and(|value| *value > 0.0);
        let onset = onsets.of(effect.object, &effect.scale_by, scope, time);
        // It outlives its event: nothing ends it but its own lifetime.
        let Some(age) = effect.times.since_begun(effect.event, time - onset).filter(|_| on) else {
            continue;
        };
        let (place, _) = posed_on(effect.node, effect.position, [1.0, 0.0, 0.0], bones);
        let to_effect: [f32; 3] = std::array::from_fn(|axis| place[axis] - view.eye[axis]);
        let distance = (0..3).map(|axis| to_effect[axis] * to_effect[axis]).sum::<f32>().sqrt();
        let along = (0..3).map(|axis| to_effect[axis] * view.forward[axis]).sum::<f32>();
        let angle = (along / distance.max(f32::EPSILON)).clamp(-1.0, 1.0).acos();
        for definition in effect.definitions.iter() {
            if definition.active(age) {
                let falloff = definition.falloff(distance, angle, age, scope);
                if falloff > 0.0 {
                    settings.accumulate(&definition.settings, falloff, to_effect);
                    if let Some((path, layer, order)) = &definition.shader {
                        let shader = H3ScreenShader { path: path.clone(), alpha: falloff, layer: *layer, order: *order };
                        insert_shader(&mut shaders, shader);
                    }
                }
            }
        }
    }
    (settings, shaders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_odst_loses_what_is_older_than_object_falloff_and_only_on_an_object() {
        assert!(!shows_on_object(true, true, false));
        assert!(shows_on_object(true, true, true), "on no object it is never read");
        assert!(shows_on_object(true, false, false), "a newer tag has its function");
        assert!(shows_on_object(false, true, false), "Halo 3 has no such function");
    }

    #[test]
    fn motion_suck_adds_up_along_the_way_to_each_effect() {
        let mut settings = H3ScreenSettings::default();
        assert_eq!(settings.suck(), None, "nothing moved");
        let sucks = H3ScreenSettings { suck: 0.5, ..Default::default() };
        settings.accumulate(&sucks, 0.5, [0.0, 0.0, 4.0]);
        settings.accumulate(&sucks, 1.0, [3.0, 0.0, 0.0]);
        assert_eq!(settings.suck(), Some([0.5, 0.0, 0.25]));
        settings.accumulate(&H3ScreenSettings::default(), 1.0, [3.0, 0.0, 0.0]);
        assert_eq!(settings.suck(), Some([0.5, 0.0, 0.25]), "no suck adds none");
    }

    fn apply(matrix: &Matrix, color: [f32; 3]) -> [f32; 3] {
        let color = [color[0], color[1], color[2], 1.0];
        std::array::from_fn(|column| (0..4).map(|row| color[row] * matrix[row][column]).sum())
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-3)
    }

    fn timed(delay: f32, lifetime: f32) -> H3ScreenDefinition {
        H3ScreenDefinition {
            settings: H3ScreenSettings::default(),
            radius: 1.0,
            delay,
            strict_delay: false,
            lifetime,
            outside_radius: false,
            no_camera_falloffs: false,
            distance_falloff: None,
            time_falloff: None,
            angle_falloff: None,
            object_falloff: None,
            predates_object_falloff: false,
            shader: None,
        }
    }

    #[test]
    fn a_blur_spreads_by_axis_and_covers_by_the_stronger() {
        assert_eq!(H3ScreenSettings::default().blur(), None);
        let (spread, cover) = H3ScreenSettings { blur: [2.0, 0.0], ..Default::default() }.blur().unwrap();
        assert_eq!(spread, [1.5, BLUR_FLOOR]);
        assert_eq!(cover, 1.0);
        let (_, cover) = H3ScreenSettings { blur: [0.0, 0.17], ..Default::default() }.blur().unwrap();
        assert!((cover - 0.2).abs() < 1e-5);
    }

    #[test]
    fn the_view_widens_by_stops_up_to_its_limit() {
        let half = 30f32.to_radians();
        assert_eq!(H3ScreenSettings::default().view_scale(half), 1.0);
        let one_stop = H3ScreenSettings { fov: [0.0, 1.0], ..Default::default() }.view_scale(half);
        assert!((one_stop - 2.0).abs() < 1e-4, "{one_stop}");
        let narrower = H3ScreenSettings { fov: [1.0, 0.0], ..Default::default() }.view_scale(half);
        assert!((narrower - 0.5).abs() < 1e-4, "{narrower}");
        let most = H3ScreenSettings { fov: [0.0, 9.0], ..Default::default() }.view_scale(half);
        assert!((most - 75f32.to_radians().tan() / half.tan()).abs() < 1e-3, "{most}");
    }

    #[test]
    fn screen_shaders_sort_by_layer_then_order_and_stop_at_the_limit() {
        let shader = |name: &str, layer, order| H3ScreenShader { path: name.to_owned(), alpha: 1.0, layer, order };
        let mut shaders = Vec::new();
        insert_shader(&mut shaders, shader("post", ScreenLayer::PostUi, 0));
        insert_shader(&mut shaders, shader("b", ScreenLayer::PreUi, 5));
        insert_shader(&mut shaders, shader("a", ScreenLayer::PreUi, 2));
        insert_shader(&mut shaders, shader("c", ScreenLayer::PreUi, 5));
        let names: Vec<_> = shaders.iter().map(|shader| shader.path.as_str()).collect();
        assert_eq!(names, ["a", "c", "b", "post"], "the later of equals goes first");
        (0..9).for_each(|at| insert_shader(&mut shaders, shader("more", ScreenLayer::PostUi, at)));
        assert_eq!(shaders.len(), MOST_SCREEN_SHADERS);
    }

    #[test]
    fn a_delayed_effect_runs_for_its_lifetime_after_its_delay() {
        let definition = timed(2.0, 1.0);
        assert!(!definition.active(1.0));
        assert!(!definition.active(2.0));
        assert!(definition.active(2.5));
        assert!(definition.active(3.0));
        assert!(!definition.active(3.5));
        assert!(timed(0.0, 1.0).active(0.0));
        assert!(timed(5.0, 0.0).active(0.0), "with no lifetime it ignores its delay");
    }

    #[test]
    fn camera_falloffs_can_be_turned_off() {
        let mut definition = timed(0.0, 0.0);
        assert_eq!(definition.falloff(2.0, 0.0, 0.0, None), 0.0);
        definition.no_camera_falloffs = true;
        assert_eq!(definition.falloff(2.0, 0.0, 0.0, None), 1.0);
    }

    #[test]
    fn a_third_person_sees_what_names_no_person_or_names_them() {
        let with = |names: &'static [&'static str]| shows_in_third_person(move |wanted| names.iter().any(|name| name.starts_with(wanted)));
        assert!(with(&[]));
        assert!(with(&["allow effect outside radius"]));
        assert!(with(&["third person only"]));
        assert!(with(&["third person{third person only}"]));
        assert!(with(&["first person{first person only}", "third person{third person only}"]));
        assert!(!with(&["first person only"]));
        assert!(!with(&["first person{first person only}"]));
        assert!(!with(&["unattached"]));
    }

    /// Reach's viewer is nobody's unit, so a person named shows only with
    /// `unattached`: the Spartans' body damage (0x78) does not; Halo 3's and
    /// ODST's rule keeps it.
    #[test]
    fn a_reach_effect_naming_a_person_shows_to_no_unit_only_unattached() {
        let names = |names: &'static [&'static str]| move |wanted: &str| names.iter().any(|name| name.starts_with(wanted));
        let body_damage: &[&str] = &[
            "first person{first person only}",
            "third person{third person only}",
            "disable camera falloffs{cinematic}",
            "only affects attached object",
        ];
        assert!(!shows_to_no_unit(names(body_damage)));
        assert!(shows_in_third_person(names(body_damage)), "ODST's rule shows it");
        assert!(shows_to_no_unit(names(&[])));
        assert!(shows_to_no_unit(names(&["unattached", "third person{third person only}"])));
        assert!(shows_to_no_unit(names(&["unattached"])));
        assert!(!shows_in_third_person(names(&["unattached"])), "where ODST's does not");
    }

    /// Reach's `is_visible` waits past a delay of 0 too.
    #[test]
    fn a_reach_effect_is_strict_on_its_delay() {
        let reach = H3ScreenDefinition { strict_delay: true, ..timed(0.0, 1.0) };
        assert!(!reach.active(0.0));
        assert!(reach.active(0.01));
        assert!(timed(0.0, 1.0).active(0.0), "ODST's starts at once");
        assert!(H3ScreenDefinition { strict_delay: true, ..timed(5.0, 0.0) }.active(0.0), "no lifetime ignores it");
    }

    /// `c_player_view::render`'s stops: the strongest boost less the
    /// strongest deboost; the noises take the strongest too.
    #[test]
    fn reachs_stops_are_boost_less_deboost_and_noise_takes_the_strongest() {
        let mut settings = H3ScreenSettings::default();
        assert_eq!(settings.stops(), 0.0);
        let boost = H3ScreenSettings { exposure_boost: 2.0, bright_noise: 0.4, ..Default::default() };
        let deboost = H3ScreenSettings { exposure_deboost: 3.0, dark_noise: 0.8, bright_noise: 0.1, ..Default::default() };
        settings.accumulate(&boost, 1.0, [0.0; 3]);
        settings.accumulate(&deboost, 0.5, [0.0; 3]);
        assert_eq!(settings.stops(), 2.0 - 1.5);
        assert_eq!(settings.noise(), [0.4, 0.4]);
        assert_eq!(settings.exposure(), 4.0, "Halo 3's composite reads the boost alone");
    }

    /// With `BLAM_TEST_HREK`: a `render layer` reads by its option's name,
    /// and only a pre-transparents shader stays out of the frame's end.
    #[test]
    fn kit_reach_screen_shaders_read_their_layer() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::HaloReach);
        for (path, layer) in [
            ("fx\\screen_fx\\camera_fx\\dead_static_overlay", ScreenLayer::PostUi),
            ("objects\\characters\\spartans\\fx\\night_vision\\night_vision", ScreenLayer::PreTransparents),
        ] {
            let tag = load_referenced_tag_from_source(&source, path, "shader_screen", b"rmss").expect(path);
            assert_eq!(ScreenLayer::read(tag.root()), layer, "{path}");
            let shader = H3ScreenShader { path: path.to_owned(), alpha: 1.0, layer, order: 0 };
            assert_eq!(shader.at_frame_end(), layer != ScreenLayer::PreTransparents, "{path}");
        }
    }

    /// With `BLAM_TEST_HREK`: the Spartans' body damage effect is not seen
    /// by the preview's camera, and the composite's noise map reads.
    #[test]
    fn kit_reach_hides_body_damage_and_reads_its_noise_map() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            eprintln!("skipping: BLAM_TEST_HREK is not set");
            return;
        };
        let source = super::super::h3_particles::loose_kit(&root, GameId::HaloReach);
        let path = "objects\\characters\\spartans\\fx\\damage_feedback\\body_damage_screen_distortion";
        let tag = load_referenced_tag_from_source(&source, path, "area_screen_effect", b"sefc").expect(path);
        assert!(!elements(tag.root(), "screen effects").is_empty());
        assert!(read_definitions(&source, tag.root()).is_empty());
        let (image, ..) = read_reach_screen_noise(&source).expect("monochrome_random");
        assert!(image.whole());
    }

    #[test]
    fn nothing_on_leaves_the_picture_alone() {
        let settings = H3ScreenSettings::default();
        assert_eq!(settings.color_matrix(), identity());
        assert_eq!(settings.gamma(), 1.0);
        assert_eq!(settings.exposure(), 1.0);
    }

    #[test]
    fn full_desaturation_leaves_only_luminance() {
        let settings = H3ScreenSettings { desaturate: 1.0, ..Default::default() };
        let grey = apply(&settings.color_matrix(), [1.0, 0.0, 0.0]);
        assert!(close(grey, [LUMINANCE[0]; 3]), "{grey:?}");
    }

    #[test]
    fn a_turn_of_hue_keeps_grey_and_moves_red() {
        let settings = H3ScreenSettings { hue_right: 120.0, ..Default::default() };
        let matrix = settings.color_matrix();
        assert!(close(apply(&matrix, [0.5; 3]), [0.5; 3]));
        assert!(!close(apply(&matrix, [1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_full_turn_of_hue_comes_back() {
        assert!(close(apply(&hue_matrix(0.0), [0.2, 0.5, 0.9]), [0.2, 0.5, 0.9]));
    }

    #[test]
    fn the_strongest_wins_and_a_filter_fades_out_with_its_falloff() {
        let mut settings = H3ScreenSettings::default();
        let strong = H3ScreenSettings { desaturate: 0.8, color_filter: [0.0, 1.0, 1.0], ..Default::default() };
        settings.accumulate(&strong, 0.5, [0.0; 3]);
        settings.accumulate(&H3ScreenSettings { desaturate: 0.2, ..Default::default() }, 1.0, [0.0; 3]);
        assert_eq!(settings.desaturate, 0.4);
        assert_eq!(settings.color_filter[0], 0.5);
        assert_eq!(settings.color_filter[1], 1.0);
    }

    #[test]
    fn gamma_stays_within_its_limits() {
        assert_eq!(H3ScreenSettings { gamma_reduce: 5.0, ..Default::default() }.gamma(), 0.01);
        assert_eq!(H3ScreenSettings { gamma_enhance: 20.0, ..Default::default() }.gamma(), 10.0);
    }

    #[test]
    fn past_its_radius_an_effect_is_gone_unless_it_says_otherwise() {
        let definition = H3ScreenDefinition { radius: 10.0, ..timed(0.0, 0.0) };
        assert_eq!(definition.falloff(5.0, 0.0, 0.0, None), 1.0);
        assert_eq!(definition.falloff(11.0, 0.0, 0.0, None), 0.0);
        assert_eq!(H3ScreenDefinition { outside_radius: true, ..definition }.falloff(11.0, 0.0, 0.0, None), 1.0);
    }
}
