//! Halo 3's, ODST's and Reach's lens flares: a `lens_flare` tag's first
//! reflection, read onto a marker, a light or an effect, and how
//! `lens_flare_render_internal` occludes and colors it. Halo 4 shares the
//! occlusion offset and rotation function readers.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::common::fields::{elements, has_flag, object_block};
use super::super::ce::lights::FlareSprite;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::effects::{
    EffectTimes, for_each_attached_part, marker_name, part_markers,
};
use super::super::functions::FunctionValues;
use blam_tags::tag_function::transition_function_evaluate;

use super::super::common::image::H2Image;
use super::super::common::named_function::tag_function_at;
use super::pipeline::EXPOSURE;
use super::super::gen3::materials::{H3Bitmap, read_h3_bitmap};
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{cross, dot, unit_by_reciprocal};
use crate::app::model_preview::common::attached::attached;

use super::super::h3::lights::{H3Light, SMALLEST};

/// A `lens_flare`'s first reflection, the only one H3 draws.
#[derive(Debug, Clone)]
pub(crate) struct H3Flare {
    cones: [f32; 2],
    near_fade: [f32; 2],
    far_fade: [f32; 2],
    tests_occlusion: bool,
    /// `falloff function`, a transition's table index as it stands: the tag
    /// labels its enum in reverse, so 4 reads "very early" and is very late.
    falloff: u8,
    occlusion_offset: f32,
    occlusion_offset_direction: OcclusionOffset,
    inner: FlareInner,
    /// Reach's reflection curves; none for Halo 3's bounds.
    curves: Option<ReachFlareCurves>,
    rotation_offset: f32,
    radius: [f32; 2],
    brightness: [f32; 2],
    color: [f32; 3],
    modulation: f32,
    tint_power: f32,
    aspect: [f32; 2],
    time_brightness: Option<Arc<TagFunction>>,
    age_brightness: Option<Arc<TagFunction>>,
    time_rotation: Option<Arc<TagFunction>>,
    age_rotation: Option<Arc<TagFunction>>,
    time_color: Option<H3FlareColor>,
    age_color: Option<H3FlareColor>,
    /// `lens_flares_evaluate_corona_rotation_function`'s, and its scale in radians.
    rotation_function: i128,
    rotation_scale: f32,
    rotates_from_center: bool,
    radius_by_distance: bool,
    radius_by_occlusion: bool,
    /// `ignore external color`: a light's color does not tint it.
    ignores_external_color: bool,
    bitmap: H3Bitmap,
}

/// How much of a flare its occlusion patch covers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FlareInner {
    /// Halo 3's `occlusion inner radius scale`: an enum, the radius halved
    /// that many times.
    Halved(i128),
    /// Reach's: the fraction of the corona to occlude against, which
    /// `lens_flare_render_internal` (the 360's 0x826a8ce0) eases back toward
    /// the whole corona as it shrinks on screen.
    Fraction(f32),
}

impl FlareInner {
    /// The patch's share of `radius`, for a flare `pixels` wide plus high.
    fn share(self, pixels: f32) -> f32 {
        match self {
            Self::Halved(times) => 1.0 / 2f32.powi(times as i32),
            Self::Fraction(inner) => {
                let size = (pixels / REACH_OCCLUSION_PIXELS).clamp(0.0, 1.0);
                inner + (1.0 - inner) * (1.0 - size)
            }
        }
    }
}

/// The width plus height, in pixels, at which a Reach flare occludes against
/// its inner fraction alone.
const REACH_OCCLUSION_PIXELS: f32 = 600.0;

/// Reach's reflection `radius curve` and `brightness curve`: each a function
/// of the flare's scale, where Halo 3 took the scale across bounds
/// (`lens_flare_evaluate_parameters`, the 360's 0x826a9978).
#[derive(Debug, Clone)]
struct ReachFlareCurves {
    radius: Arc<TagFunction>,
    brightness: Arc<TagFunction>,
}

/// `s_editing_function`: a color curve, and what can drive it off the object.
#[derive(Debug, Clone)]
struct H3FlareColor {
    function: Arc<TagFunction>,
    input: String,
}

impl H3FlareColor {
    fn read(root: TagStruct<'_>, block: &str) -> Option<Self> {
        let first = elements(root, block).into_iter().next()?;
        Some(Self {
            function: Arc::new(tag_function_at(&first, "function")?),
            input: first.read_string_id("input name").or_else(|| first.read_string_id("Input Variable")).unwrap_or_default(),
        })
    }

    fn at(&self, input: f32, values: Option<&FunctionValues>) -> [f32; 3] {
        let driven = values.filter(|_| !self.input.is_empty()).and_then(|values| values.get(&self.input)).copied();
        let input = driven.unwrap_or_else(|| self.function.evaluate(input, 0.0));
        let color = self.function.evaluate_color(input, 1.0);
        [color.red, color.green, color.blue]
    }
}

/// `lens_flares_evaluate_corona_rotation_function`.
fn corona_rotation(function: i128, scale: f32, position: [f32; 3], forward: [f32; 3], view: &ViewFrame) -> f32 {
    let across = cross(cross(forward, view.forward), forward);
    let to_flare: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
    let back = forward.map(|axis| -axis);
    let (x, y) = match function {
        1 => (dot(view.forward, across), -dot(view.forward, forward)),
        2 => (dot(back, view.forward), -dot(back, view.up)),
        3 => (dot(to_flare, across), -dot(to_flare, forward)),
        4 => (dot(to_flare, view.forward), -dot(to_flare, view.up)),
        _ => return 0.0,
    };
    if x == 0.0 { 0.0 } else { x.atan2(y) * scale / std::f32::consts::PI }
}

/// A flare at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H3FlareAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    only_under: Option<(String, String)>,
    scale_by: String,
    /// Reach's attachment `secondary scale`, which multiplies the primary.
    secondary_by: String,
    /// Set for a flare an attached effect draws.
    by_effect: Option<H3EffectScale>,
    /// When its effect's event runs, for a flare an attached effect draws.
    event: Option<(Arc<EffectTimes>, usize)>,
    flare: Arc<H3Flare>,
    /// A light's own flare: the light, whose state colors and scales it.
    light: Option<Arc<H3Light>>,
}

/// `effect_generate_lens_flares`' use of the attachment's scale.
#[derive(Debug, Clone, Copy)]
pub(in crate::app::model_preview) struct H3EffectScale {
    /// The part's `A scales values` takes in its type's scale, as otherwise the
    /// flare burns at full while the effect runs.
    pub(in crate::app::model_preview) scaled: bool,
}

/// `occlusion offset direction`: where the occlusion quad moves
/// `occlusion offset distance` to (`lens_flare_render_internal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app::model_preview) enum OcclusionOffset {
    /// Along the flare-to-eye line.
    TowardViewer,
    MarkerForward,
    None,
}

impl OcclusionOffset {
    pub(in crate::app::model_preview) fn read(root: TagStruct<'_>) -> Self {
        match root.read_enum_option("occlusion offset direction").as_deref() {
            Some("marker forward") => Self::MarkerForward,
            Some("none") => Self::None,
            _ => Self::TowardViewer,
        }
    }
}

/// `rotation function`, as `lens_flares_evaluate_corona_rotation_function`
/// numbers it: 0 for none.
pub(in crate::app::model_preview) fn rotation_function(root: TagStruct<'_>) -> i128 {
    match root.read_enum_option("rotation function").as_deref() {
        Some("rotation A") => 1,
        Some("rotation B") => 2,
        Some("rotation-translation") => 3,
        Some("translation") => 4,
        _ => 0,
    }
}

fn block_function(root: TagStruct<'_>, block: &str) -> Option<Arc<TagFunction>> {
    let first = elements(root, block).into_iter().next()?;
    tag_function_at(&first, "function").map(Arc::new)
}

pub(in crate::app::model_preview) fn read_h3_flare(reader: &TagReader, path: &str) -> Option<H3Flare> {
    let tag = reader.load(path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    if has_flag(root, "flags", "only render in first person") {
        return None;
    }
    let reflection = *elements(root, "reflections").first()?;
    let reflected = |wanted: &str| has_flag(reflection, "flags", wanted);
    let bitmap = root.read_tag_ref_path("bitmap").filter(|bitmap| !bitmap.is_empty());
    let override_map = reflection.read_tag_ref_path("bitmapOverride").filter(|bitmap| !bitmap.is_empty());
    let bitmap = override_map.or(bitmap)?;
    let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
    let bitmap = read_h3_bitmap(reader, &bitmap, index as i16, SamplerKind::Texture2D)?;
    let (width, height) = match &*bitmap.0 {
        H2Image::Flat(image) => (image.width as f32, image.height as f32),
        _ => (1.0, 1.0),
    };
    // `lens_flare_get_bitmap_aspect_ratio`.
    let aspect = if width > height {
        [1.0, (width / height).floor()]
    } else if height > width {
        [(height / width).floor(), 1.0]
    } else {
        [1.0, 1.0]
    };
    let radius = reflection.read_real_bounds("radius");
    let brightness = reflection.read_fraction_bounds("brightness");
    let curve = |name: &str| reflection.descend(name).and_then(|curve| tag_function_at(&curve, "function")).map(Arc::new);
    let curves = curve("radius curve").zip(curve("brightness curve")).map(|(radius, brightness)| ReachFlareCurves { radius, brightness });
    // Reach's inner scale is a real fraction; Halo 3's an enum.
    let inner = match curves {
        Some(_) => FlareInner::Fraction(root.read_real("occlusion inner radius scale").unwrap_or(1.0)),
        None => FlareInner::Halved(root.read_int_any("occlusion inner radius scale").unwrap_or(0)),
    };
    let color = reflection.read_rgb("color");
    let fade = |name: &str| root.read_real(name).unwrap_or(0.0);
    Some(H3Flare {
        cones: [fade("falloff angle"), fade("cutoff angle")],
        near_fade: [fade("near fade begin distance"), fade("near fade end distance")],
        far_fade: [fade("near fade distance"), fade("far fade distance")],
        tests_occlusion: !has_flag(root, "flags", "no occlusion test"),
        falloff: root.read_int_any("falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
        occlusion_offset: fade("occlusion offset distance"),
        occlusion_offset_direction: OcclusionOffset::read(root),
        inner,
        curves,
        rotation_offset: reflection.read_real("rotation offset").unwrap_or(0.0).to_radians(),
        radius: [radius.lower, radius.upper],
        brightness: [brightness.lower, brightness.upper],
        color: [color.red, color.green, color.blue],
        modulation: reflection.read_real("modulation factor").unwrap_or(0.0),
        tint_power: reflection.read_real("tint power").filter(|power| *power > SMALLEST).unwrap_or(4.0),
        aspect,
        time_brightness: block_function(root, "time brightness"),
        age_brightness: block_function(root, "age brightness"),
        time_rotation: block_function(root, "time rotation"),
        age_rotation: block_function(root, "age rotation"),
        time_color: H3FlareColor::read(root, "time color"),
        age_color: H3FlareColor::read(root, "age color"),
        rotation_function: rotation_function(root),
        rotation_scale: root.read_real("rotation function scale").unwrap_or(0.0),
        rotates_from_center: reflected("rotate from center of screen"),
        radius_by_distance: reflected("radius scaled by distance"),
        radius_by_occlusion: reflected("radius scaled by occlusion factor"),
        ignores_external_color: reflected("ignore external color"),
        bitmap,
    })
}

/// An object's attached `lens_flare`s, at every marker of each name.
pub(in crate::app::model_preview) fn read_h3_flares(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3FlareAt> {
    let mut flares = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return flares;
    };
    let flare_group = u32::from_be_bytes(*b"lens");
    let mut place_flare = |path: &str,
                           found: Vec<RenderModelPreviewMarker>,
                           scale_by: String,
                           secondary_by: String,
                           by_effect: Option<H3EffectScale>,
                           event: Option<(Arc<EffectTimes>, usize)>,
                           placed: &dyn Fn(MarkerFrame, usize) -> (MarkerFrame, usize)| {
        let Some(flare) = read_h3_flare(reader, path).map(Arc::new) else {
            return;
        };
        for marker in &found {
            let (place, node) = frame(marker);
            let (place, node) = placed(place, node);
            flares.push(H3FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                only_under: marker.only_under.clone(),
                scale_by: scale_by.clone(),
                secondary_by: if flare.curves.is_some() { secondary_by.clone() } else { String::new() },
                by_effect,
                event: event.clone(),
                flare: Arc::clone(&flare),
                light: None,
            });
        }
    };
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group == flare_group && !path.is_empty() {
            let name = marker_name(attachment, "marker");
            let scale_by = attachment.read_string_id("primary scale").unwrap_or_default();
            // Reach's `render_object_attachments` (0x8274f128) scales by both.
            let secondary_by = attachment.read_string_id("secondary scale").unwrap_or_default();
            place_flare(&path, part_markers(markers, &name, false), scale_by, secondary_by, None, None, &|place, node| (place, node));
        }
    }
    for_each_attached_part(reader, object, |part| {
        if part.group == flare_group {
            let by_effect = Some(H3EffectScale { scaled: part.scaled });
            let event = Some((Arc::clone(&part.times), part.event));
            let placed = |place, node| part.placed(place, node);
            let scale_by = part.effect.scale_by.clone();
            let found = part.effect.locations(markers, &part.marker, &frame);
            place_flare(&part.path, found, scale_by, String::new(), by_effect, event, &placed);
        }
    });
    flares
}

/// `light_submit_lens_flares`: each light with a `Lens Flare` draws it where
/// the light is, by the light's color and scale. Take `lights` once they are
/// final, renamed for what they hang on.
pub(in crate::app::model_preview) fn light_flares(lights: &[H3Light]) -> Vec<H3FlareAt> {
    let flare_of = |light: &H3Light| {
        Some(H3FlareAt {
            object: light.object,
            node: light.node,
            position: light.position,
            forward: light.forward,
            only_under: light.only_under.clone(),
            scale_by: light.scale_by.clone(),
            secondary_by: String::new(),
            by_effect: None,
            event: None,
            flare: Arc::clone(light.flare.as_ref()?),
            light: Some(Arc::new(light.clone())),
        })
    };
    lights.iter().filter_map(flare_of).collect()
}

/// `k_minimum_visible_lens_flare`.
const MINIMUM_VISIBLE: f32 = 0.05;

/// What `lens_flares_render` and `sub_180296A74` make of the share of a flare's
/// patch that passed: too little draws nothing, the rest takes the falloff.
fn occlusion_fraction(falloff: u8, tested: bool, visible: f32) -> f32 {
    if tested && visible <= MINIMUM_VISIBLE {
        return 0.0;
    }
    transition_function_evaluate(falloff, visible)
}

impl H3EffectScale {
    /// `attachments_update` runs the effect only while the function is on.
    pub(in crate::app::model_preview) fn of(self, value: f32) -> f32 {
        match (value > 0.0, self.scaled) {
            (false, _) => 0.0,
            (true, true) => value,
            (true, false) => 1.0,
        }
    }
}

impl H3FlareAt {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// For a light's own flare, the light.
    pub(crate) fn light(&self) -> Option<&H3Light> {
        self.light.as_deref()
    }

    /// What it burns by, and the color tinting it: for a light's own flare,
    /// the light's `state` (its scale, and its color brought down to at most 1
    /// with the excess taken into the scale, as `lens_flares_create_internal`
    /// does); for any other, its scale and white.
    fn scale_and_tint(
        &self,
        light: Option<([f32; 3], f32)>,
        values: Option<&FunctionValues>,
        time: f32,
    ) -> (f32, [f32; 3]) {
        let Some((color, scale)) = light else {
            return (self.scale(values, time), [1.0; 3]);
        };
        let most = color.into_iter().fold(0.0, f32::max);
        if most > 1.0 { (scale * most, color.map(|channel| channel / most)) } else { (scale, color) }
    }

    /// Whether its event's clock runs from when `scale_name` came on.
    pub(crate) fn event_time(&self, time: f32, onset: f32) -> f32 {
        if self.event.is_some() { time - onset } else { time }
    }

    pub(crate) fn bitmap(&self) -> &H3Bitmap {
        &self.flare.bitmap
    }

    pub(crate) fn animates(&self) -> bool {
        let moving = |function: &Option<Arc<TagFunction>>| {
            function.as_ref().is_some_and(|function| function.as_constant().is_none())
        };
        moving(&self.flare.time_brightness)
            || moving(&self.flare.time_rotation)
            || self.event.as_ref().is_some_and(|(times, _)| times.moves())
    }

    /// The object's `primary scale`, which also runs the flare's bounds.
    fn scale(&self, values: Option<&FunctionValues>, time: f32) -> f32 {
        // `effect_generate_lens_flares`: out with its event, fading as it ages.
        let aged = match &self.event {
            Some((times, event)) => match times.age(*event, time) {
                Some(age) => 1.0 - transition_function_evaluate(self.flare.falloff, age),
                None => return 0.0,
            },
            None => 1.0,
        };
        let named = |name: &str| values.and_then(|values| values.get(name)).copied().unwrap_or(0.0);
        let secondary = if self.secondary_by.is_empty() { 1.0 } else { named(&self.secondary_by) };
        if self.scale_by.is_empty() {
            return aged * secondary;
        }
        let value = named(&self.scale_by);
        aged * self.by_effect.map_or(value, |scale| scale.of(value)) * secondary
    }

    fn radius(&self, scale: f32) -> f32 {
        // Reach: the marker's matrix scale (the object's, 1) times the curve.
        if let Some(curves) = &self.flare.curves {
            return curves.radius.evaluate(scale, 0.0);
        }
        let [lower, upper] = self.flare.radius;
        // `lens_flares_create` takes the marker matrix's scale, which is the
        // object's. The marker's own `scale` is its `radius`, and many are 0.
        (upper - lower) * scale + lower
    }

    /// `lens_flare_render_occlusion`'s patch, or none for a flare without one.
    /// `light` is its light's `state`, for a light's own flare. `pixels` is
    /// how many pixels wide plus high a flare of a radius is at a point.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn occlusion_patch(
        &self,
        light: Option<([f32; 3], f32)>,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        pixels: &dyn Fn([f32; 3], [f32; 2]) -> f32,
    ) -> Option<FlareSprite> {
        let flare = &*self.flare;
        let (scale, _) = self.scale_and_tint(light, values, time);
        if !flare.tests_occlusion || scale <= 0.0 {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let offset = match flare.occlusion_offset_direction {
            OcclusionOffset::TowardViewer => unit_by_reciprocal(std::array::from_fn(|axis| view.eye[axis] - position[axis])),
            OcclusionOffset::MarkerForward => forward,
            OcclusionOffset::None => [0.0; 3],
        };
        let radius = self.radius(scale);
        let across = pixels(position, [radius * flare.aspect[0], radius * flare.aspect[1]]);
        let radius = radius * flare.inner.share(across);
        Some(FlareSprite {
            center: std::array::from_fn(|axis| position[axis] + offset[axis] * flare.occlusion_offset),
            extent: [radius; 2],
            rotation: 0.0,
            color: [0.0; 3],
            alpha: 0.0,
            image: 0,
            highlight: None,
            tint: None,
            depth_tested: false,
        })
    }

    /// `lens_flare_evaluate_parameters` and the screen-facing half of
    /// `lens_flare_render_internal`. Only reflection 0 ever draws. `light` is
    /// its light's `state`, for a light's own flare.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sprite(
        &self,
        light: Option<([f32; 3], f32)>,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        occlusion: f32,
        image: usize,
    ) -> Option<FlareSprite> {
        let flare = &*self.flare;
        let (scale, tint) = self.scale_and_tint(light, values, time);
        if scale <= 0.0 || occlusion <= 0.0 {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let to_flare: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
        let distance = dot(view.forward, to_flare);
        if distance <= 0.0 {
            return None;
        }
        let mut corona = occlusion_fraction(flare.falloff, flare.tests_occlusion, occlusion);
        let [far_begin, far_end] = flare.far_fade;
        if far_begin != far_end {
            corona *= ((distance - far_end) / (far_begin - far_end)).clamp(0.0, 1.0);
        }
        let [near_begin, near_end] = flare.near_fade;
        if near_begin != near_end && distance < near_end {
            corona *= ((distance - near_begin) / (near_end - near_begin)).clamp(0.0, 1.0);
        }
        let [falloff, cutoff] = flare.cones;
        if falloff != cutoff {
            let length = dot(to_flare, to_flare).sqrt().max(SMALLEST);
            let to_eye = to_flare.map(|axis| -axis / length);
            let angle = dot(forward, to_eye).clamp(-1.0, 1.0).acos();
            if angle > falloff {
                corona *= 1.0 - ((angle - falloff) / (cutoff - falloff)).clamp(0.0, 1.0);
            }
        }
        let age = self.event.as_ref().and_then(|(times, event)| times.age(*event, time)).unwrap_or(0.0);
        if let Some(function) = &flare.time_brightness {
            corona *= function.evaluate(time, 0.0);
        }
        if let Some(function) = &flare.age_brightness {
            corona *= function.evaluate(age, 0.0);
        }
        let brightness = match &flare.curves {
            Some(curves) => curves.brightness.evaluate(scale, 0.0) * corona,
            None => {
                let [lower, upper] = flare.brightness;
                ((upper - lower) * scale + lower).clamp(0.0, 1.0) * corona
            }
        };
        let mut radius = self.radius(scale);
        if flare.radius_by_occlusion {
            radius *= ((1.0 + occlusion) * 0.5).clamp(0.0, 1.0);
        }
        if flare.radius_by_distance {
            radius *= distance;
        }
        if brightness <= 0.0 || radius <= 0.0 {
            return None;
        }
        let spin = |function: &Option<Arc<TagFunction>>, input: f32| {
            function.as_ref().map_or(0.0, |function| std::f32::consts::PI * function.evaluate(input, 0.0))
        };
        let mut rotation = flare.rotation_offset
            + corona_rotation(flare.rotation_function, flare.rotation_scale, position, forward, view)
            + spin(&flare.time_rotation, time)
            + spin(&flare.age_rotation, age);
        if flare.rotates_from_center {
            let left = view.right.map(|axis| -axis);
            rotation += dot(to_flare, view.forward).atan2(dot(to_flare, left));
        }
        let mut color = flare.color;
        if !flare.ignores_external_color {
            (0..3).for_each(|channel| color[channel] *= tint[channel]);
        }
        let curves = [(&flare.time_color, time), (&flare.age_color, age)];
        for (curve, input) in curves {
            if let Some(curve) = curve {
                let tint = curve.at(input, values);
                (0..3).for_each(|channel| color[channel] *= tint[channel]);
            }
        }
        Some(FlareSprite {
            center: position,
            extent: [radius * flare.aspect[0], radius * flare.aspect[1]],
            rotation,
            color,
            // `lens_flare.hlsl` lights itself by `ILLUM_EXPOSURE`.
            alpha: brightness * EXPOSURE,
            image,
            highlight: None,
            tint: Some([flare.modulation, flare.tint_power]),
            depth_tested: false,
        })
    }
}

attached!(H3FlareAt);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_hidden_flare_follows_its_falloff_table() {
        // `0x1808A0E70`'s row 4 in `halo3.dll`: 10 of 255 at entry 460.
        let hidden = occlusion_fraction(4, true, 460.0 / 1023.0);
        assert!((hidden - 10.0 / 255.0).abs() < 0.01, "{hidden}");
        assert_eq!(occlusion_fraction(4, true, 1.0), 1.0);
        assert_eq!(occlusion_fraction(0, true, 0.5), 0.5, "linear takes no table");
        assert_eq!(occlusion_fraction(0, true, MINIMUM_VISIBLE), 0.0, "too little to draw");
        assert_eq!(occlusion_fraction(0, false, 0.02), 0.02, "only a tested share has a floor");
    }

    #[test]
    fn a_reach_flare_occludes_against_its_inner_share_only_once_it_is_large() {
        let reach = FlareInner::Fraction(0.25);
        assert_eq!(reach.share(0.0), 1.0, "a speck tests its whole self");
        assert_eq!(reach.share(300.0), 0.625);
        assert_eq!(reach.share(600.0), 0.25);
        assert_eq!(reach.share(5000.0), 0.25);
        assert_eq!(FlareInner::Halved(2).share(5000.0), 0.25, "Halo 3's ignores its size");
        assert_eq!(FlareInner::Halved(2).share(0.0), 0.25);
    }

    /// `BLAM_TEST_HREK` and `BLAM_TEST_H3EK`: the kits' tags folders.
    #[test]
    fn reach_flares_read_their_curves_and_halo_3s_their_bounds() {
        use crate::app::model_preview::harness::kit::kit_source;
        if let Some(reach) = kit_source(crate::core::game::GameId::HaloReach) {
            let path = "objects\\props\\human\\unsc\\light_fixture_telescoping\\fx\\wide_flare";
            let flare = read_h3_flare(&crate::app::model_preview::load::TagReader::new(&reach), path).unwrap();
            assert!(flare.curves.is_some());
            assert!(matches!(flare.inner, FlareInner::Fraction(_)), "{:?}", flare.inner);
        }
        if let Some(halo3) = kit_source(crate::core::game::GameId::Halo3) {
            let path = "objects\\vehicles\\warthog\\fx\\running\\headlights";
            let flare = read_h3_flare(&crate::app::model_preview::load::TagReader::new(&halo3), path).unwrap();
            assert!(flare.curves.is_none());
            assert!(matches!(flare.inner, FlareInner::Halved(_)), "{:?}", flare.inner);
        }
    }

    /// `BLAM_TEST_H3EK` and `BLAM_TEST_HREK`: each `occlusion offset
    /// direction` reads by its option's name, as `lens_flare_render_internal`
    /// uses it: toward the viewer, along the marker, or not moved. 0 is toward
    /// the viewer.
    #[test]
    fn flares_read_their_occlusion_offset_direction() {
        use crate::core::game::GameId;
        let kits = [
            (
                GameId::Halo3,
                [
                    "fx\\cinematics\\010la_jungle_intro\\01\\hatch",
                    "levels\\multi\\s3d_sky_bridgenew\\shaders\\lighting\\flare_projector_illum",
                    "levels\\multi\\s3d_avalanche\\sky\\shaders\\flare_sun_ava_01",
                ],
            ),
            (
                GameId::HaloReach,
                [
                    "cinematics\\070lk_credits\\fx\\slipspace_anamorphic",
                    "objects\\vehicles\\human\\warthog\\fx\\running\\headlights",
                    "levels\\dlc\\cex_beaver_creek\\fx\\ca_beaver_rainbow",
                ],
            ),
        ];
        let directions = [OcclusionOffset::TowardViewer, OcclusionOffset::MarkerForward, OcclusionOffset::None];
        for (game, paths) in kits {
            let Some(source) = crate::app::model_preview::harness::kit::kit_source(game) else {
                continue;
            };
            for (path, direction) in paths.into_iter().zip(directions) {
                let flare = read_h3_flare(&crate::app::model_preview::load::TagReader::new(&source), path).unwrap_or_else(|| panic!("{path}"));
                assert_eq!(flare.occlusion_offset_direction, direction, "{path}");
            }
        }
    }

    #[test]
    fn a_coronas_rotation_follows_the_eye_only_when_it_has_a_function() {
        let view = ViewFrame { eye: [0.0; 3], forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0] };
        assert_eq!(corona_rotation(0, 1.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], &view), 0.0);
        let turned = corona_rotation(4, std::f32::consts::PI, [1.0, 0.0, 1.0], [0.0, 1.0, 0.0], &view);
        assert!((turned - (1.0f32).atan2(-1.0)).abs() < 1e-5, "{turned}");
    }

    #[test]
    fn an_effects_flare_burns_whole_while_its_function_is_on() {
        let whole = H3EffectScale { scaled: false };
        assert_eq!(whole.of(0.0), 0.0);
        assert_eq!(whole.of(0.25), 1.0);
        let scaled = H3EffectScale { scaled: true };
        assert_eq!(scaled.of(0.0), 0.0);
        assert_eq!(scaled.of(0.25), 0.25);
    }
}
