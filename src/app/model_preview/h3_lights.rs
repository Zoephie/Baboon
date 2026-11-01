//! Halo 3's lights on objects: `objects\lights.cpp` works one out, and
//! `c_lights_view::submit_visibility_and_render` draws it through each
//! shader's `dynamic_light` entry.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeFlareView, CeSprite, posed_on};
use super::h3_effects::{
    H3EffectTimes, MOST_MARKERS, for_each_attached_part, marker_name, part_frame, read_part_placement,
};
use super::h2_functions::H2FunctionValues;
use blam_tags::tag_function::transition_function_evaluate;

use super::h2_passes::{H2Image, tag_function_at};
use super::renderer::H3_EXPOSURE as EXPOSURE;
use super::materials::{H3Bitmap, read_h3_bitmap};
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// `flags` of a `light`.
const ALLOW_SHADOWS_AND_GELS: i128 = 1;
const FIRST_PERSON_ONLY: i128 = 4;
/// `type`.
const FRUSTUM: i128 = 1;
/// What `add_simple_light_to_draw_list` falls back on.
const FALLBACK_DISTANCE: f32 = 5.0;
const FALLBACK_CONE: f32 = 0.5;
/// `light_update`'s range until something names one.
const DEFAULT_RANGE: f32 = 0.5;
const SMALLEST: f32 = 1e-6;

#[derive(Debug, Clone)]
struct H3Channel {
    input: String,
    range: String,
    function: Arc<TagFunction>,
}

impl H3Channel {
    fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        let holder = root.descend(name)?;
        Some(Self {
            input: holder.read_string_id("Input Variable").unwrap_or_default(),
            range: holder.read_string_id("Range Variable").unwrap_or_default(),
            function: Arc::new(tag_function_at(&holder, "Mapping")?),
        })
    }

    /// `light_update`: seconds alive, unless an object function is named.
    fn sample(&self, values: Option<&H2FunctionValues>, time: f32) -> (f32, f32) {
        let named = |name: &str| values.and_then(|values| values.get(name)).copied().unwrap_or(0.0);
        let input = if self.input.is_empty() { time } else { named(&self.input) };
        let range = if self.range.is_empty() { DEFAULT_RANGE } else { named(&self.range) };
        (input, range)
    }
}

/// A `light` at the marker an object attaches it to.
#[derive(Debug, Clone)]
pub(crate) struct H3Light {
    /// When its effect's event runs, for a light an attached effect makes.
    event: Option<(Arc<H3EffectTimes>, usize)>,
    /// `destroy light after`, seconds from then.
    destroy_after: f32,
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    scale_by: String,
    /// Set for a light an attached effect makes.
    by_effect: Option<H3EffectScale>,
    /// `light_update` multiplies in the object's change color.
    change_color: Option<usize>,
    frustum: bool,
    maximum_distance: f32,
    near_width: f32,
    height_scale: f32,
    /// Radians.
    field_of_view: f32,
    diffusion: f32,
    smoothness: f32,
    spherical: f32,
    color: H3Channel,
    intensity: H3Channel,
    pub(crate) gel: Option<Arc<H2Image>>,
}

/// What a frame's pass reads of a light.
pub(crate) struct H3LitLight {
    /// `s_simple_light`, as `simple_lights` holds it.
    pub(crate) rows: [[f32; 4]; 5],
    /// `Shadow_Projection`, a plain column matrix.
    pub(crate) projection: [f32; 16],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `c_lights_view::initialize_simple_light`.
fn simple_light(
    position: [f32; 3],
    direction: [f32; 3],
    color: [f32; 3],
    size: f32,
    max_distance: f32,
    cone: f32,
    smoothness: f32,
    spherical: f32,
) -> [[f32; 4]; 5] {
    let max_distance = if max_distance > 0.0 { max_distance } else { FALLBACK_DISTANCE };
    let cone = if cone > 0.0 { cone } else { FALLBACK_CONE };
    let smoothness = if smoothness > 0.0 { smoothness } else { 1.0 };
    let squared = max_distance * max_distance;
    let cos_edge = (cone * 0.5).cos();
    let cone_scale = (1.0 - spherical).powf(1.0 / smoothness) / (1.0 - cos_edge);
    let (cone_scale, cone_offset) =
        if cone < SMALLEST { (0.0, 1.0) } else { (cone_scale, -cone_scale * cos_edge) };
    let color = color.map(|channel| channel / size.max(SMALLEST));
    [
        [position[0], position[1], position[2], size],
        [-direction[0], -direction[1], -direction[2], spherical],
        [color[0], color[1], color[2], smoothness],
        [size * (size + squared) / squared, cone_scale, -size / squared, cone_offset],
        [squared, 0.0, 0.0, 0.0],
    ]
}

impl H3Light {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    /// Follow the regions `merge_attached` renames for what `name` brings.
    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        [&self.color.input, &self.color.range, &self.intensity.input, &self.intensity.range]
            .into_iter()
            .chain([&self.scale_by])
            .map(String::as_str)
    }

    pub(crate) fn animates(&self) -> bool {
        [&self.color, &self.intensity]
            .iter()
            .any(|channel| channel.input.is_empty() && channel.function.as_constant().is_none())
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// The light this frame, or none while it is out.
    pub(crate) fn lit(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<H3LitLight> {
        if let Some((times, event)) = &self.event {
            let elapsed = times.elapsed(*event, time - onset)?;
            if self.destroy_after > 0.0 && elapsed >= self.destroy_after {
                return None;
            }
        }
        let (input, range) = self.color.sample(values, time);
        let color = self.color.function.evaluate_color(input, range);
        let (input, range) = self.intensity.sample(values, time);
        let mut intensity = self.intensity.function.evaluate(input, range);
        if !self.scale_by.is_empty() {
            let scale = values.and_then(|values| values.get(&self.scale_by)).copied().unwrap_or(0.0);
            intensity *= self.by_effect.map_or(scale, |effect| effect.of(scale));
        }
        let tint = self.change_color.and_then(|channel| change_colors.get(channel)).copied();
        let [red, green, blue] = tint.unwrap_or([1.0; 3]);
        let color = [color.red * red, color.green * green, color.blue * blue].map(|channel| channel * intensity);
        if color.iter().all(|channel| *channel <= 0.0) {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let (cone, near) = if self.frustum { (self.field_of_view, self.near_width) } else { (0.0, 0.0) };
        // A sphere has no cone: `cone_angle_radians` reads as the fallback.
        let cone_angle = if self.frustum { cone } else { std::f32::consts::TAU };
        let tangent = (cone.max(SMALLEST) * 0.5).tan();
        let behind = if self.frustum { near * 0.5 / tangent } else { 0.0 };
        let origin: [f32; 3] = std::array::from_fn(|axis| position[axis] - forward[axis] * behind);
        let rows = simple_light(
            origin,
            forward,
            color,
            self.diffusion,
            self.maximum_distance,
            cone_angle,
            self.smoothness,
            self.spherical,
        );
        // `setup_shadow_camera_and_projection`'s axes, into a gel's 0 to 1.
        let across = cross(forward, up);
        let along = cross(across, forward);
        let aspect = if self.height_scale > 0.0 { self.height_scale } else { 1.0 };
        let x: [f32; 3] = std::array::from_fn(|axis| across[axis] * 0.5 / tangent + forward[axis] * 0.5);
        let y: [f32; 3] =
            std::array::from_fn(|axis| along[axis] * 0.5 / (tangent * aspect) + forward[axis] * 0.5);
        let to_origin = |row: [f32; 3]| -dot(row, origin);
        let mut projection = [0.0; 16];
        for axis in 0..3 {
            projection[axis * 4] = x[axis];
            projection[axis * 4 + 1] = y[axis];
            projection[axis * 4 + 3] = forward[axis];
        }
        projection[12] = to_origin(x);
        projection[13] = to_origin(y);
        projection[15] = to_origin(forward);
        Some(H3LitLight { rows, projection })
    }
}

fn read_h3_light(source: &TagSource, path: &str) -> Option<H3LightTag> {
    let tag = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = tag.root();
    let flags = root.read_int_any("flags").unwrap_or(0);
    if flags & FIRST_PERSON_ONLY != 0 {
        return None;
    }
    let gel = root.read_tag_ref_path("gel bitmap").filter(|gel| !gel.is_empty());
    let gel = gel
        .filter(|_| flags & ALLOW_SHADOWS_AND_GELS != 0)
        .and_then(|gel| decode_h2_image(source, &gel, SamplerKind::Texture2D, 0))
        .map(Arc::new);
    Some(H3LightTag {
        frustum: root.read_int_any("type") == Some(FRUSTUM),
        destroy_after: root.read_real("destroy light after").unwrap_or(0.0),
        maximum_distance: root.read_real("maximum distance").unwrap_or(0.0),
        near_width: root.read_real("frustum near width").unwrap_or(0.0),
        height_scale: root.read_real("frustum height scale").unwrap_or(1.0),
        field_of_view: root.read_real("frustum field of view").unwrap_or(0.0).to_radians(),
        diffusion: root.read_real("distance diffusion").unwrap_or(0.0),
        smoothness: root.read_real("angular smoothness").unwrap_or(0.0),
        spherical: root.read_real("percent spherical").unwrap_or(0.0),
        color: H3Channel::read(root, "color")?,
        intensity: H3Channel::read(root, "intensity")?,
        gel,
    })
}

/// A `light`, before a marker places it.
struct H3LightTag {
    destroy_after: f32,
    frustum: bool,
    maximum_distance: f32,
    near_width: f32,
    height_scale: f32,
    field_of_view: f32,
    diffusion: f32,
    smoothness: f32,
    spherical: f32,
    color: H3Channel,
    intensity: H3Channel,
    gel: Option<Arc<H2Image>>,
}

/// An object's attached `light`s, at the first marker of each name, and those
/// its attached effects make, `light_new_unattached` at each of a location's.
pub(super) fn read_h3_lights(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H3Light> {
    let mut lights = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return lights;
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    let place_light = |tag: H3LightTag,
                           place: MarkerFrame,
                           node: usize,
                           only_under: Option<(String, String)>,
                           attachment: TagStruct<'_>,
                           by_effect: Option<H3EffectScale>,
                           event: Option<(Arc<H3EffectTimes>, usize)>| H3Light {
        event,
        destroy_after: tag.destroy_after,
        object: owner,
        node,
        position: place.position,
        forward: place.axes[0],
        up: place.axes[2],
        only_under,
        scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
        by_effect,
        // The tag's first option is none; the channels follow.
        change_color: usize::try_from(attachment.read_int_any("change color").unwrap_or(0) - 1).ok(),
        frustum: tag.frustum,
        maximum_distance: tag.maximum_distance,
        near_width: tag.near_width,
        height_scale: tag.height_scale,
        field_of_view: tag.field_of_view,
        diffusion: tag.diffusion,
        smoothness: tag.smoothness,
        spherical: tag.spherical,
        color: tag.color,
        intensity: tag.intensity,
        gel: tag.gel,
    };
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != light_group || path.is_empty() {
            continue;
        }
        let Some(tag) = read_h3_light(source, &path) else {
            continue;
        };
        let name = marker_name(attachment, "marker");
        let Some(marker) = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(&name)) else {
            continue;
        };
        let (place, node) = frame(marker);
        lights.push(place_light(tag, place, node, None, attachment, None, None));
    }
    for_each_attached_part(source, object, |part| {
        if part.group != light_group {
            return;
        }
        let (offset, yaw, pitch) = read_part_placement(part.part);
        let named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&part.marker));
        for marker in named.take(MOST_MARKERS) {
            let Some(tag) = read_h3_light(source, &part.path) else {
                return;
            };
            let (place, node) = frame(marker);
            let place = part_frame(&place, offset, yaw, pitch);
            let by_effect = Some(H3EffectScale { scaled: part.scaled });
            let only_under = marker.only_under.clone();
            let event = Some((Arc::clone(&part.times), part.event));
            lights.push(place_light(tag, place, node, only_under, part.attachment, by_effect, event));
        }
    });
    lights
}

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
    occlusion_toward_eye: bool,
    inner_scale: i128,
    rotation_offset: f32,
    radius: [f32; 2],
    brightness: [f32; 2],
    color: [f32; 3],
    modulation: f32,
    tint_power: f32,
    aspect: [f32; 2],
    time_brightness: Option<Arc<TagFunction>>,
    time_rotation: Option<Arc<TagFunction>>,
    bitmap: H3Bitmap,
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
    /// Set for a flare an attached effect draws.
    by_effect: Option<H3EffectScale>,
    /// When its effect's event runs, for a flare an attached effect draws.
    event: Option<(Arc<H3EffectTimes>, usize)>,
    flare: Arc<H3Flare>,
}

/// `effect_generate_lens_flares`' use of the attachment's scale.
#[derive(Debug, Clone, Copy)]
struct H3EffectScale {
    /// The part's `A scales values` takes in its type's scale, as otherwise the
    /// flare burns at full while the effect runs.
    scaled: bool,
}

const NO_OCCLUSION_TEST: i128 = 2;
const ONLY_FIRST_PERSON_FLARE: i128 = 4;
/// `occlusion offset direction` toward the eye.
const OFFSET_TOWARD_EYE: i128 = 1;

fn block_function(root: TagStruct<'_>, block: &str) -> Option<Arc<TagFunction>> {
    let first = elements(root, block).into_iter().next()?;
    tag_function_at(&first, "function").map(Arc::new)
}

fn read_h3_flare(source: &TagSource, path: &str) -> Option<H3Flare> {
    let tag = load_referenced_tag_from_source(source, path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let flags = root.read_int_any("flags").unwrap_or(0);
    if flags & ONLY_FIRST_PERSON_FLARE != 0 {
        return None;
    }
    let reflection = *elements(root, "reflections").first()?;
    let bitmap = root.read_tag_ref_path("bitmap").filter(|bitmap| !bitmap.is_empty());
    let override_map = reflection.read_tag_ref_path("bitmapOverride").filter(|bitmap| !bitmap.is_empty());
    let bitmap = override_map.or(bitmap)?;
    let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
    let bitmap = read_h3_bitmap(source, &bitmap, index as i16, SamplerKind::Texture2D)?;
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
    let color = reflection.read_rgb("color");
    let fade = |name: &str| root.read_real(name).unwrap_or(0.0);
    Some(H3Flare {
        cones: [fade("falloff angle"), fade("cutoff angle")],
        near_fade: [fade("near fade begin distance"), fade("near fade end distance")],
        far_fade: [fade("near fade distance"), fade("far fade distance")],
        tests_occlusion: flags & NO_OCCLUSION_TEST == 0,
        falloff: root.read_int_any("falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
        occlusion_offset: fade("occlusion offset distance"),
        occlusion_toward_eye: root.read_int_any("occlusion offset direction") == Some(OFFSET_TOWARD_EYE),
        inner_scale: root.read_int_any("occlusion inner radius scale").unwrap_or(0),
        rotation_offset: reflection.read_real("rotation offset").unwrap_or(0.0).to_radians(),
        radius: [radius.lower, radius.upper],
        brightness: [brightness.lower, brightness.upper],
        color: [color.red, color.green, color.blue],
        modulation: reflection.read_real("modulation factor").unwrap_or(0.0),
        tint_power: reflection.read_real("tint power").filter(|power| *power > SMALLEST).unwrap_or(4.0),
        aspect,
        time_brightness: block_function(root, "time brightness"),
        time_rotation: block_function(root, "time rotation"),
        bitmap,
    })
}

/// An object's attached `lens_flare`s, at every marker of each name.
pub(super) fn read_h3_flares(
    source: &TagSource,
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
                           name: &str,
                           attachment: TagStruct<'_>,
                           by_effect: Option<H3EffectScale>,
                           event: Option<(Arc<H3EffectTimes>, usize)>| {
        let Some(flare) = read_h3_flare(source, path).map(Arc::new) else {
            return;
        };
        for marker in markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(name)) {
            let (place, node) = frame(marker);
            flares.push(H3FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                only_under: marker.only_under.clone(),
                scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                by_effect,
                event: event.clone(),
                flare: Arc::clone(&flare),
            });
        }
    };
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group == flare_group && !path.is_empty() {
            let name = marker_name(attachment, "marker");
            place_flare(&path, &name, attachment, None, None);
        }
    }
    for_each_attached_part(source, object, |part| {
        if part.group == flare_group {
            let by_effect = Some(H3EffectScale { scaled: part.scaled });
            let event = Some((Arc::clone(&part.times), part.event));
            place_flare(&part.path, &part.marker, part.attachment, by_effect, event);
        }
    });
    flares
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
    fn of(self, value: f32) -> f32 {
        match (value > 0.0, self.scaled) {
            (false, _) => 0.0,
            (true, true) => value,
            (true, false) => 1.0,
        }
    }
}

impl H3FlareAt {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    /// Follow the regions `merge_attached` renames for what `name` brings.
    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
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
    fn scale(&self, values: Option<&H2FunctionValues>, time: f32) -> f32 {
        // `effect_generate_lens_flares`: out with its event, fading as it ages.
        let aged = match &self.event {
            Some((times, event)) => match times.age(*event, time) {
                Some(age) => 1.0 - transition_function_evaluate(self.flare.falloff, age),
                None => return 0.0,
            },
            None => 1.0,
        };
        if self.scale_by.is_empty() {
            return aged;
        }
        let value = values.and_then(|values| values.get(&self.scale_by)).copied().unwrap_or(0.0);
        aged * self.by_effect.map_or(value, |scale| scale.of(value))
    }

    fn radius(&self, scale: f32) -> f32 {
        let [lower, upper] = self.flare.radius;
        // `lens_flares_create` takes the marker matrix's scale, which is the
        // object's. The marker's own `scale` is its `radius`, and many are 0.
        (upper - lower) * scale + lower
    }

    /// `lens_flare_render_occlusion`'s patch, or none for a flare without one.
    pub(crate) fn occlusion_patch(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
    ) -> Option<CeSprite> {
        let flare = &*self.flare;
        let scale = self.scale(values, time);
        if !flare.tests_occlusion || scale <= 0.0 {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let toward_eye = view.forward.map(|axis| -axis);
        let offset = if flare.occlusion_toward_eye { toward_eye } else { forward };
        let radius = self.radius(scale) / 2f32.powi(flare.inner_scale as i32);
        Some(CeSprite {
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
    /// `lens_flare_render_internal`. Only reflection 0 ever draws.
    pub(crate) fn sprite(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        occlusion: f32,
        image: usize,
    ) -> Option<CeSprite> {
        let flare = &*self.flare;
        let scale = self.scale(values, time);
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
        if let Some(function) = &flare.time_brightness {
            corona *= function.evaluate(time, 0.0);
        }
        let [lower, upper] = flare.brightness;
        let brightness = ((upper - lower) * scale + lower).clamp(0.0, 1.0) * corona;
        let radius = self.radius(scale);
        if brightness <= 0.0 || radius <= 0.0 {
            return None;
        }
        let mut rotation = flare.rotation_offset;
        if let Some(function) = &flare.time_rotation {
            rotation += std::f32::consts::PI * function.evaluate(time, 0.0);
        }
        Some(CeSprite {
            center: position,
            extent: [radius * flare.aspect[0], radius * flare.aspect[1]],
            rotation,
            color: flare.color,
            // `lens_flare.hlsl` lights itself by `ILLUM_EXPOSURE`.
            alpha: brightness * EXPOSURE,
            image,
            highlight: None,
            tint: Some([flare.modulation, flare.tint_power]),
            depth_tested: false,
        })
    }
}

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
    fn an_effects_flare_burns_whole_while_its_function_is_on() {
        let whole = H3EffectScale { scaled: false };
        assert_eq!(whole.of(0.0), 0.0);
        assert_eq!(whole.of(0.25), 1.0);
        let scaled = H3EffectScale { scaled: true };
        assert_eq!(scaled.of(0.0), 0.0);
        assert_eq!(scaled.of(0.25), 0.25);
    }

    #[test]
    fn a_light_reaches_zero_at_its_maximum_distance() {
        let rows = simple_light([0.0; 3], [1.0, 0.0, 0.0], [1.0; 3], 2.0, 10.0, 1.0, 1.0, 0.0);
        let [_, _, _, [scale, _, offset, _], _] = rows;
        let at = |distance: f32| (1.0 / (2.0 + distance * distance)) * scale + offset;
        assert!(at(10.0).abs() < 1e-5);
        assert!((at(0.0) - (0.5 * scale + offset)).abs() < 1e-5);
    }

    #[test]
    fn a_cone_closes_at_half_its_angle() {
        let rows = simple_light([0.0; 3], [1.0, 0.0, 0.0], [1.0; 3], 1.0, 5.0, 1.0, 2.0, 0.0);
        let [_, _, _, [_, scale, _, offset], _] = rows;
        let at = |angle: f32| angle.cos() * scale + offset;
        assert!(at(0.5).abs() < 1e-5, "the edge is dark");
        assert!((at(0.0) - 1.0).abs() < 1e-4, "the axis is full");
    }

    #[test]
    fn a_light_with_no_distance_takes_the_game_s_fallback() {
        let rows = simple_light([0.0; 3], [1.0, 0.0, 0.0], [1.0; 3], 1.0, 0.0, 1.0, 1.0, 0.0);
        assert_eq!(rows[4][0], FALLBACK_DISTANCE * FALLBACK_DISTANCE);
    }

    #[test]
    fn the_stored_direction_points_back_at_the_light() {
        let rows = simple_light([0.0; 3], [0.0, 0.0, 1.0], [1.0; 3], 1.0, 5.0, 1.0, 1.0, 0.25);
        assert_eq!(rows[1], [0.0, 0.0, -1.0, 0.25]);
    }
}
