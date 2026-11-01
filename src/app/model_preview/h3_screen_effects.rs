//! Halo 3's area screen effects: what an effect does to the whole picture while
//! the camera is near it. `screen_effect_sample` folds all of them into one set
//! of settings, which the final composite applies before the tone curve.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::elements;
use super::ce_lights::{CeFlareView, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::tag_function_at;
use super::h3_effects::{H3EffectOnsets, H3EffectTimes, for_each_attached_part, part_markers};
use super::*;
use crate::core::source::TagSource;

/// `build_saturation_matrix`'s weights.
const LUMINANCE: [f32; 3] = [0.212_656, 0.715_158, 0.072_185_6];
/// `set_hue_saturaton_and_color_filters`' `scale`.
const CONTRAST_SCALE: f32 = 10.0;

type Matrix = [[f32; 4]; 4];

/// `s_screen_effect_settings`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H3ScreenSettings {
    exposure_boost: f32,
    hue_left: f32,
    hue_right: f32,
    saturate: f32,
    desaturate: f32,
    contrast_enhance: f32,
    gamma_enhance: f32,
    gamma_reduce: f32,
    color_filter: [f32; 3],
    color_floor: [f32; 3],
}

impl Default for H3ScreenSettings {
    /// `set_defaults`.
    fn default() -> Self {
        Self {
            exposure_boost: 0.0,
            hue_left: 0.0,
            hue_right: 0.0,
            saturate: 0.0,
            desaturate: 0.0,
            contrast_enhance: 0.0,
            gamma_enhance: 0.0,
            gamma_reduce: 0.0,
            color_filter: [1.0; 3],
            color_floor: [0.0; 3],
        }
    }
}

impl H3ScreenSettings {
    /// `accumulate_settings`: the strongest of each, not their sum.
    fn accumulate(&mut self, other: &Self, falloff: f32) {
        let falloff = falloff.clamp(0.0, 1.0);
        let most = |mine: &mut f32, theirs: f32| *mine = mine.max(falloff * theirs);
        most(&mut self.exposure_boost, other.exposure_boost);
        most(&mut self.hue_left, other.hue_left);
        most(&mut self.hue_right, other.hue_right);
        most(&mut self.saturate, other.saturate);
        most(&mut self.desaturate, other.desaturate);
        most(&mut self.contrast_enhance, other.contrast_enhance);
        most(&mut self.gamma_enhance, other.gamma_enhance);
        most(&mut self.gamma_reduce, other.gamma_reduce);
        for channel in 0..3 {
            let toward = falloff * other.color_filter[channel] + (1.0 - falloff);
            self.color_filter[channel] = self.color_filter[channel].min(toward);
            most(&mut self.color_floor[channel], other.color_floor[channel]);
        }
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
    lifetime: f32,
    outside_radius: bool,
    distance_falloff: Option<TagFunction>,
    time_falloff: Option<TagFunction>,
    angle_falloff: Option<TagFunction>,
}

impl H3ScreenDefinition {
    /// `evaluate_falloff`.
    fn falloff(&self, distance: f32, angle: f32, time: f32) -> f32 {
        if !self.outside_radius && distance > self.radius {
            return 0.0;
        }
        let at = |function: &Option<TagFunction>, input: f32| {
            function.as_ref().map_or(1.0, |function| function.evaluate(input.clamp(0.0, 1.0), 0.0))
        };
        let time_input = if self.lifetime > f32::EPSILON { time / self.lifetime } else { time };
        at(&self.distance_falloff, distance / self.radius.max(f32::EPSILON))
            * at(&self.time_falloff, time_input)
            * at(&self.angle_falloff, angle / std::f32::consts::PI)
    }
}

fn read_definition(element: TagStruct<'_>) -> Option<H3ScreenDefinition> {
    let flags = element.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    // The preview's camera is a third person's, on nothing.
    if flagged("first person only") {
        return None;
    }
    let real = |name: &str| element.read_real(name).unwrap_or(0.0);
    let color = |name: &str| {
        let color = element.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let falloff = |name: &str| element.descend(name).and_then(|holder| tag_function_at(&holder, "Mapping"));
    Some(H3ScreenDefinition {
        settings: H3ScreenSettings {
            exposure_boost: real("exposure boost"),
            hue_left: real("hue left"),
            hue_right: real("hue right"),
            saturate: real("saturation"),
            desaturate: real("desaturation"),
            contrast_enhance: real("contrast enhance"),
            gamma_enhance: real("gamma enhance"),
            gamma_reduce: real("gamma reduce"),
            color_filter: color("color filter"),
            color_floor: color("color floor"),
        },
        radius: real("maximum distance"),
        lifetime: real("lifetime"),
        outside_radius: flagged("allow effect outside radius"),
        distance_falloff: falloff("distance falloff"),
        time_falloff: falloff("time falloff"),
        angle_falloff: falloff("angle falloff"),
    })
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
        let read = elements(tag.root(), "screen effects").into_iter().filter_map(read_definition);
        let definitions = Arc::new(read.collect::<Vec<_>>());
        if definitions.is_empty() {
            return;
        }
        for marker in part_markers(markers, &part.marker, part.effect.once) {
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

/// `screen_effect_sample`: what every running effect comes to from this camera.
pub(crate) fn h3_screen_settings<'a>(
    effects: &[H3ScreenEffectAt],
    values: impl Fn(u8) -> Option<&'a H2FunctionValues>,
    onsets: &mut H3EffectOnsets,
    bones: Option<&[[f32; 4]]>,
    view: &CeFlareView,
    time: f32,
) -> H3ScreenSettings {
    let mut settings = H3ScreenSettings::default();
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
            if definition.lifetime <= 0.0 || age <= definition.lifetime {
                let falloff = definition.falloff(distance, angle, age);
                if falloff > 0.0 {
                    settings.accumulate(&definition.settings, falloff);
                }
            }
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(matrix: &Matrix, color: [f32; 3]) -> [f32; 3] {
        let color = [color[0], color[1], color[2], 1.0];
        std::array::from_fn(|column| (0..4).map(|row| color[row] * matrix[row][column]).sum())
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-3)
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
        settings.accumulate(&strong, 0.5);
        settings.accumulate(&H3ScreenSettings { desaturate: 0.2, ..Default::default() }, 1.0);
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
        let definition = H3ScreenDefinition {
            settings: H3ScreenSettings::default(),
            radius: 10.0,
            lifetime: 0.0,
            outside_radius: false,
            distance_falloff: None,
            time_falloff: None,
            angle_falloff: None,
        };
        assert_eq!(definition.falloff(5.0, 0.0, 0.0), 1.0);
        assert_eq!(definition.falloff(11.0, 0.0, 0.0), 0.0);
        assert_eq!(H3ScreenDefinition { outside_radius: true, ..definition }.falloff(11.0, 0.0, 0.0), 1.0);
    }
}
