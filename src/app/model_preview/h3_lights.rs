//! Halo 3's lights on objects: `objects\lights.cpp` works one out, and
//! `c_lights_view::submit_visibility_and_render` draws it through each
//! shader's `dynamic_light` entry.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::h3_effects::adopt_event_function;
use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeFlareView, CeSprite, posed_on};
use super::h3_effects::{
    H3EffectTimes, for_each_attached_part, marker_name, part_frame, part_markers, read_part_placement,
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
    /// `use parent position but world orientation`: its axes do not turn.
    world_oriented: bool,
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
    /// Its `Lens Flare`, which `light_submit_lens_flares` draws at it.
    flare: Option<Arc<H3Flare>>,
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
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
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

    /// `light_update`'s `state.color` and `state.current_scale` this frame,
    /// or none while the light is out.
    pub(crate) fn state(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        time: f32,
        onset: f32,
    ) -> Option<([f32; 3], f32)> {
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
        let mut scale = 1.0;
        if !self.scale_by.is_empty() {
            let value = values.and_then(|values| values.get(&self.scale_by)).copied().unwrap_or(0.0);
            scale = self.by_effect.map_or(value, |effect| effect.of(value));
            intensity *= scale;
        }
        let tint = self.change_color.and_then(|channel| change_colors.get(channel)).copied();
        let [red, green, blue] = tint.unwrap_or([1.0; 3]);
        let color = [color.red * red, color.green * green, color.blue * blue].map(|channel| channel * intensity);
        if color.iter().all(|channel| *channel <= 0.0) {
            return None;
        }
        Some((color, scale))
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
        let (color, _) = self.state(values, change_colors, time, onset)?;
        let (position, mut forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, mut up) = posed_on(self.node, self.position, self.up, bones);
        if self.world_oriented {
            (forward, up) = (self.forward, self.up);
        }
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
        flare: root
            .read_tag_ref_path("Lens Flare")
            .filter(|path| !path.is_empty())
            .and_then(|path| read_h3_flare(source, &path))
            .map(Arc::new),
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
    flare: Option<Arc<H3Flare>>,
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
                           scale_by: String,
                           change_color: i128,
                           by_effect: Option<H3EffectScale>,
                           event: Option<(Arc<H3EffectTimes>, usize)>,
                           world_oriented: bool| H3Light {
        event,
        world_oriented,
        destroy_after: tag.destroy_after,
        object: owner,
        node,
        position: place.position,
        forward: place.axes[0],
        up: place.axes[2],
        only_under,
        scale_by,
        by_effect,
        // The tag's first option is none; the channels follow.
        change_color: usize::try_from(change_color - 1).ok(),
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
        flare: tag.flare,
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
        let scale_by = attachment.read_string_id("primary scale").unwrap_or_default();
        let change_color = attachment.read_int_any("change color").unwrap_or(0);
        lights.push(place_light(tag, place, node, None, scale_by, change_color, None, None, false));
    }
    for_each_attached_part(source, object, |part| {
        if part.group != light_group {
            return;
        }
        let (offset, yaw, pitch) = read_part_placement(part.part);
        for marker in &part.effect.locations(markers, &part.marker, &frame) {
            let Some(tag) = read_h3_light(source, &part.path) else {
                return;
            };
            let (place, node) = frame(marker);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            let by_effect = Some(H3EffectScale { scaled: part.scaled });
            let only_under = marker.only_under.clone();
            let event = Some((Arc::clone(&part.times), part.event));
            let (scale_by, change_color) = (part.effect.scale_by.clone(), part.effect.change_color);
            lights.push(place_light(tag, place, node, only_under, scale_by, change_color, by_effect, event, part.world_oriented));
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

    fn at(&self, input: f32, values: Option<&H2FunctionValues>) -> [f32; 3] {
        let driven = values.filter(|_| !self.input.is_empty()).and_then(|values| values.get(&self.input)).copied();
        let input = driven.unwrap_or_else(|| self.function.evaluate(input, 0.0));
        let color = self.function.evaluate_color(input, 1.0);
        [color.red, color.green, color.blue]
    }
}

/// `lens_flares_evaluate_corona_rotation_function`.
fn corona_rotation(function: i128, scale: f32, position: [f32; 3], forward: [f32; 3], view: &CeFlareView) -> f32 {
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
    event: Option<(Arc<H3EffectTimes>, usize)>,
    flare: Arc<H3Flare>,
    /// A light's own flare: the light, whose state colors and scales it.
    light: Option<Arc<H3Light>>,
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
    let reflection_flags = reflection.read_flag_names("flags").unwrap_or_default();
    let reflected = |wanted: &str| reflection_flags.iter().any(|(_, name)| name.starts_with(wanted));
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
        tests_occlusion: flags & NO_OCCLUSION_TEST == 0,
        falloff: root.read_int_any("falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
        occlusion_offset: fade("occlusion offset distance"),
        occlusion_toward_eye: root.read_int_any("occlusion offset direction") == Some(OFFSET_TOWARD_EYE),
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
        rotation_function: match root.read_enum_name("rotation function").as_deref() {
            Some("rotation A") => 1,
            Some("rotation B") => 2,
            Some("rotation-translation") => 3,
            Some("translation") => 4,
            _ => 0,
        },
        rotation_scale: root.read_real("rotation function scale").unwrap_or(0.0),
        rotates_from_center: reflected("rotate from center of screen"),
        radius_by_distance: reflected("radius scaled by distance"),
        radius_by_occlusion: reflected("radius scaled by occlusion factor"),
        ignores_external_color: reflected("ignore external color"),
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
                           found: Vec<RenderModelPreviewMarker>,
                           scale_by: String,
                           secondary_by: String,
                           by_effect: Option<H3EffectScale>,
                           event: Option<(Arc<H3EffectTimes>, usize)>,
                           placed: &dyn Fn(MarkerFrame, usize) -> (MarkerFrame, usize)| {
        let Some(flare) = read_h3_flare(source, path).map(Arc::new) else {
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
    for_each_attached_part(source, object, |part| {
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
pub(super) fn light_flares(lights: &[H3Light]) -> Vec<H3FlareAt> {
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
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

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
        values: Option<&H2FunctionValues>,
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
    fn scale(&self, values: Option<&H2FunctionValues>, time: f32) -> f32 {
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
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        pixels: &dyn Fn([f32; 3], [f32; 2]) -> f32,
    ) -> Option<CeSprite> {
        let flare = &*self.flare;
        let (scale, _) = self.scale_and_tint(light, values, time);
        if !flare.tests_occlusion || scale <= 0.0 {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let toward_eye = view.forward.map(|axis| -axis);
        let offset = if flare.occlusion_toward_eye { toward_eye } else { forward };
        let radius = self.radius(scale);
        let across = pixels(position, [radius * flare.aspect[0], radius * flare.aspect[1]]);
        let radius = radius * flare.inner.share(across);
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
    /// `lens_flare_render_internal`. Only reflection 0 ever draws. `light` is
    /// its light's `state`, for a light's own flare.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sprite(
        &self,
        light: Option<([f32; 3], f32)>,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        occlusion: f32,
        image: usize,
    ) -> Option<CeSprite> {
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
        Some(CeSprite {
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
        let source = |var: &str, game| {
            let root = std::env::var(var).ok()?;
            Some(TagSource::LooseFolder {
                root: root.into(),
                game: Some(game),
                definitions_root: crate::core::bundled::locate_definitions_root(),
            })
        };
        if let Some(reach) = source("BLAM_TEST_HREK", crate::core::game::GameId::HaloReach) {
            let path = "objects\\props\\human\\unsc\\light_fixture_telescoping\\fx\\wide_flare";
            let flare = read_h3_flare(&reach, path).unwrap();
            assert!(flare.curves.is_some());
            assert!(matches!(flare.inner, FlareInner::Fraction(_)), "{:?}", flare.inner);
        }
        if let Some(halo3) = source("BLAM_TEST_H3EK", crate::core::game::GameId::Halo3) {
            let path = "objects\\vehicles\\warthog\\fx\\running\\headlights";
            let flare = read_h3_flare(&halo3, path).unwrap();
            assert!(flare.curves.is_none());
            assert!(matches!(flare.inner, FlareInner::Halved(_)), "{:?}", flare.inner);
        }
    }

    #[test]
    fn a_coronas_rotation_follows_the_eye_only_when_it_has_a_function() {
        let view = CeFlareView { eye: [0.0; 3], forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0] };
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
