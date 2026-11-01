//! Halo 2's lens flares on objects: `render_submit`'s attachments, then
//! `rasterizer_lens_flares.cpp`. The shape is Halo CE's, in
//! [`super::ce_lights`], but the sums are not: brightness, color and rotation
//! animate, occlusion comes in two sizes, and the pixel shader adds a highlight.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeFlareView, CeSprite, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::{H2DefaultLight, H2Image, ce_color_interpolate, object_input, tag_function_at};
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// `flags` of a `lens_flare`.
const NO_OCCLUSION_TEST: i128 = 2;
const ONLY_FIRST_PERSON: i128 = 4;
const SCALE_BY_MARKER: i128 = 0x40;
/// `flags` of a reflection.
const ALIGN_WITH_SCREEN_CENTER: i128 = 1;
const RADIUS_NOT_SCALED_BY_DISTANCE: i128 = 2;
const RADIUS_SCALED_BY_OCCLUSION: i128 = 4;
const OCCLUDED_BY_SOLID_OBJECTS: i128 = 8;
const IGNORE_LIGHT_COLOR: i128 = 0x10;
const NOT_AFFECTED_BY_INNER_OCCLUSION: i128 = 0x20;

/// `flags` of a `light`.
const NO_ILLUMINATION: i128 = 1;
const LIGHT_ONLY_FIRST_PERSON: i128 = 0x20;
/// `ignore parent object` and `ignore all parents`: `sub_6BA660` leaves the
/// object unlit only with both.
const IGNORES_PARENTS: i128 = 0x2800;
/// `type`: the only one that takes the sphere path.
const SPHERE: i128 = 0;
/// What the brightness of `sub_8ED2A0` comes to, at most.
const BRIGHTEST: f32 = 4.0;
/// `default lightmap setting`: the game makes no such light.
const LIGHTMAPS_ONLY: i128 = 2;
const NO_SPECULAR: i128 = 2;
/// Tool's postprocess takes bounds all under this as unset.
const UNSET: f32 = 0.0001;

/// `occlusion inner radius scale`.
const INNER_SCALES: [f32; 7] = [1.0, 0.5, 0.25, 0.125, 0.0625, 0.03125, 0.015625];

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
struct H2Reflection {
    flags: i128,
    /// Which of [`H2Flare::images`] it draws.
    image: usize,
    position: f32,
    rotation: f32,
    radius: [f32; 2],
    brightness: [f32; 2],
    modulation: f32,
    color: [f32; 3],
}

/// A `lens_flare` tag.
#[derive(Debug, Clone)]
pub(crate) struct H2Flare {
    flags: i128,
    /// Cosines of its `falloff angle` and `cutoff angle`.
    cones: [f32; 2],
    fade: [f32; 2],
    occlusion_radius: f32,
    /// Toward the viewer, along the marker, or none.
    occlusion_offset: i128,
    /// The share of the outer patch a second test takes, if there is one.
    inner_scale: Option<f32>,
    corona_scale: [f32; 2],
    rotation_function: i128,
    rotation_scale: f32,
    reflections: Vec<H2Reflection>,
    brightness: Option<Arc<TagFunction>>,
    color: Option<Arc<TagFunction>>,
    rotation: Option<Arc<TagFunction>>,
    pub(crate) images: Vec<Arc<H2Image>>,
}

/// What `sub_8ED2A0` works a `light` out from, for the flare it submits.
#[derive(Debug, Clone)]
struct H2Light {
    interpolation: i128,
    size: [f32; 2],
    specular: [[f32; 3]; 2],
    diffuse: [[f32; 3]; 2],
    brightness: [f32; 2],
    color_animation: Option<Arc<TagFunction>>,
    brightness_animation: Option<Arc<TagFunction>>,
    radius: f32,
    specular_radius: f32,
}

/// A sphere light at the marker an object attaches it to, lighting the model.
#[derive(Debug, Clone)]
pub(crate) struct H2PointLight {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    /// `primary scale`, a name the object's functions answer to.
    scale_by: String,
    light: H2Light,
}

/// A flare at one of the markers an object attaches it to.
#[derive(Debug, Clone)]
pub(crate) struct H2FlareAt {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    marker_scale: f32,
    /// `primary scale`, a name the object's functions answer to.
    scale_by: String,
    /// `change color`: which of the four, if any.
    change_color: Option<usize>,
    /// The light it belongs to, if it is no attachment of its own.
    light: Option<H2Light>,
    pub(crate) flare: Arc<H2Flare>,
}

/// `object_attachment_scale`: no name is 1, and a name nothing answers to is 0,
/// as `object_get_function_value` leaves it.
fn attachment_value(name: &str, values: Option<&H2FunctionValues>) -> f32 {
    if name.is_empty() {
        return 1.0;
    }
    let known = values.and_then(|values| values.get(name)).copied();
    known.unwrap_or_else(|| object_input(name))
}

fn scale_function(entry: TagStruct<'_>, block: &str) -> Option<Arc<TagFunction>> {
    let element = *elements(entry, block).first()?;
    tag_function_at(&element, "function").map(Arc::new)
}

fn read_h2_flare(source: &TagSource, path: &str) -> Option<H2Flare> {
    let tag = load_referenced_tag_from_source(source, path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let bitmap = root.read_tag_ref_path("bitmap").filter(|bitmap| !bitmap.is_empty())?;
    let mut images = Vec::new();
    let mut indices = Vec::new();
    let mut reflections = Vec::new();
    for reflection in elements(root, "reflections") {
        let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
        let image = indices.iter().position(|known| *known == index).or_else(|| {
            let image = decode_h2_image(source, &bitmap, SamplerKind::Texture2D, index)?;
            images.push(Arc::new(image));
            indices.push(index);
            Some(images.len() - 1)
        })?;
        let radius = reflection.read_real_bounds("radius");
        let brightness = reflection.read_fraction_bounds("brightness");
        let color = reflection.read_rgb("color");
        reflections.push(H2Reflection {
            flags: reflection.read_int_any("flags").unwrap_or(0),
            image,
            position: reflection.read_real("position").unwrap_or(0.0),
            rotation: reflection.read_real("rotation offset").unwrap_or(0.0).to_radians(),
            radius: [radius.lower, radius.upper],
            brightness: [brightness.lower, brightness.upper],
            modulation: reflection.read_real("modulation factor").unwrap_or(0.0),
            color: [color.red, color.green, color.blue],
        });
    }
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0).cos();
    let corona = root.read_vec2("corona scale");
    let inner = root.read_int_any("occlusion inner radius scale").unwrap_or(0).clamp(0, 6) as usize;
    let unset_as_one = |scale: f32| if scale == 0.0 { 1.0 } else { scale };
    Some(H2Flare {
        flags: root.read_int_any("flags").unwrap_or(0),
        cones: [angle("falloff angle"), angle("cutoff angle")],
        fade: [
            root.read_real("near fade distance").unwrap_or(0.0),
            root.read_real("far fade distance").unwrap_or(0.0),
        ],
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        occlusion_offset: root.read_int_any("occlusion offset direction").unwrap_or(0),
        inner_scale: (inner != 0).then_some(INNER_SCALES[inner]),
        corona_scale: [unset_as_one(corona.i), unset_as_one(corona.j)],
        rotation_function: root.read_int_any("rotation function").unwrap_or(0),
        rotation_scale: root.read_real("rotation function scale").unwrap_or(0.0),
        reflections,
        brightness: scale_function(root, "brightness"),
        color: scale_function(root, "color"),
        rotation: scale_function(root, "rotation"),
        images,
    })
}

/// `sub_8EB720`, and what tool's postprocess gives a tag for the game: the
/// light, whether it lights the model on the sphere path, and its flare. `None`
/// for a light the game makes nothing of in a third person view.
fn read_h2_light(source: &TagSource, path: &str) -> Option<(H2Light, bool, Option<Arc<H2Flare>>)> {
    let tag = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = tag.root();
    let flags = root.read_int_any("flags").unwrap_or(0);
    // Nothing else sets the bit `sub_8ED2A0` turns a light on through.
    if flags & (NO_ILLUMINATION | LIGHT_ONLY_FIRST_PERSON) != 0 {
        return None;
    }
    if root.read_int_any("default lightmap setting") == Some(LIGHTMAPS_ONLY) {
        return None;
    }
    let flare = root.read_tag_ref_path("lens flare").filter(|path| !path.is_empty());
    let flare = flare.and_then(|path| read_h2_flare(source, &path)).map(Arc::new);
    let rgb = |name: &str| {
        let color = root.read_rgb(name);
        [color.red, color.green, color.blue]
    };
    let unset = |colors: &[[f32; 3]; 2]| colors.iter().flatten().all(|channel| *channel < UNSET);
    let size = root.read_real_bounds("size modifer");
    let size = if size.lower < UNSET && size.upper < UNSET {
        [1.0, 1.0]
    } else {
        [size.lower, size.upper]
    };
    let mut diffuse = [rgb("diffuse lower bound"), rgb("diffuse upper bound")];
    if unset(&diffuse) {
        diffuse = [[1.0; 3]; 2];
    }
    let mut specular = [rgb("specular lower bound"), rgb("specular upper bound")];
    if flags & NO_SPECULAR != 0 {
        specular = [[0.0; 3]; 2];
    } else if unset(&specular) {
        specular = diffuse;
    }
    let brightness = root.read_real_bounds("brightness bounds");
    let brightness = if brightness.lower == 0.0 && brightness.upper == 0.0 {
        [1.0, 1.0]
    } else {
        [brightness.lower, brightness.upper]
    };
    let radius = root.read_real("radius").unwrap_or(0.0);
    let specular_radius = root.read_real("specular radius").filter(|radius| *radius != 0.0);
    let saturate = |colors: [[f32; 3]; 2]| colors.map(|color| color.map(|c| c.clamp(0.0, 1.0)));
    let light = H2Light {
        interpolation: root.read_int_any("interpolation flags").unwrap_or(0),
        size,
        specular: saturate(specular),
        diffuse: saturate(diffuse),
        brightness,
        color_animation: scale_function(root, "color animation"),
        brightness_animation: scale_function(root, "brightness animation"),
        radius,
        specular_radius: specular_radius.unwrap_or(radius),
    };
    let gel = root.read_tag_ref_path("gel map").is_some_and(|gel| !gel.is_empty());
    let lights_model = root.read_int_any("type") == Some(SPHERE)
        && !gel
        && flags & IGNORES_PARENTS != IGNORES_PARENTS;
    Some((light, lights_model, flare))
}

/// `render_submit_object_attachments`' `lens_flare`s, at every marker of their
/// names, and `light_submit_lens_flares`' lights, which take the first.
pub(super) fn read_h2_flares(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> (Vec<H2FlareAt>, Vec<H2PointLight>) {
    let mut flares = Vec::new();
    let mut point_lights = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return (flares, point_lights);
    };
    let lens_group = u32::from_be_bytes(*b"lens");
    let light_group = u32::from_be_bytes(*b"ligh");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let (light, flare) = if group == lens_group {
            let Some(flare) = read_h2_flare(source, &path).map(Arc::new) else {
                continue;
            };
            (None, flare)
        } else if group == light_group {
            let Some((light, lights_model, flare)) = read_h2_light(source, &path) else {
                continue;
            };
            if lights_model {
                let name = attachment.read_string_id("marker").unwrap_or_default();
                let named = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(&name));
                if let Some(marker) = named {
                    let (place, node) = frame(marker);
                    point_lights.push(H2PointLight {
                        object: owner,
                        node,
                        position: place.position,
                        scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                        light: light.clone(),
                    });
                }
            }
            let Some(flare) = flare else {
                continue;
            };
            (Some(light), flare)
        } else {
            continue;
        };
        // A preview is a third person view.
        if flare.flags & ONLY_FIRST_PERSON != 0 {
            continue;
        }
        let name = attachment.read_string_id("marker").unwrap_or_default();
        let scale_by = attachment.read_string_id("primary scale").unwrap_or_default();
        let change_color = usize::try_from(attachment.read_int_any("change color").unwrap_or(0) - 1);
        let named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name));
        for marker in named.take(if light.is_some() { 1 } else { usize::MAX }) {
            let (place, node) = frame(marker);
            flares.push(H2FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                marker_scale: marker.scale,
                scale_by: scale_by.clone(),
                change_color: change_color.ok(),
                light: light.clone(),
                flare: Arc::clone(&flare),
            });
        }
    }
    (flares, point_lights)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn from_view(view: &CeFlareView, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| point[axis] - view.eye[axis])
}

/// `sub_92D350`: a corona's turn for how the marker sits against the view, in
/// units of `atan2 * scale / pi`. The game's third view axis points back at the
/// viewer's side as `-forward`, as Halo CE's does.
fn corona_rotation(
    function: i128,
    scale: f32,
    to_light: [f32; 3],
    forward: [f32; 3],
    view: &CeFlareView,
) -> f32 {
    let turned = cross(cross(forward, view.right), forward);
    let back = view.forward.map(|axis| -axis);
    let (across, along) = match function {
        1 => (dot(turned, view.forward), dot(forward, view.forward)),
        2 => {
            let behind = forward.map(|axis| -axis);
            (dot(behind, view.right), dot(behind, back))
        }
        3 => (dot(to_light, turned), dot(to_light, forward)),
        4 => (dot(to_light, view.right), dot(to_light, back)),
        _ => return 0.0,
    };
    if across == 0.0 {
        return 0.0;
    }
    across.atan2(-along) * scale / std::f32::consts::PI
}

impl H2Flare {
    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        self.brightness.is_some() || self.color.is_some() || self.rotation.is_some()
    }

    fn tests_occlusion(&self) -> bool {
        self.flags & NO_OCCLUSION_TEST == 0
    }
}

impl H2Light {
    /// `sub_8ED2A0`'s color for `value`, or none while the light is out. Both
    /// bounds' alphas are 1 here, so a change color adds nothing.
    fn diffuse_at(&self, value: f32, time: f32) -> Option<[f32; 3]> {
        let at = |[lower, upper]: [[f32; 3]; 2]| {
            ce_color_interpolate(self.interpolation, lower, upper, value)
        };
        let (mut diffuse, specular) = (at(self.diffuse), at(self.specular));
        if let Some(function) = &self.color_animation {
            let color = function.evaluate_color(time, 0.0);
            let scale = [color.red, color.green, color.blue];
            diffuse = std::array::from_fn(|channel| diffuse[channel] * scale[channel]);
        }
        let diffuse = diffuse.map(|channel| channel.clamp(0.0, 1.0));
        let specular = specular.map(|channel| channel.clamp(0.0, 1.0));
        let size = (1.0 - value) * self.size[0] + value * self.size[1];
        let lit = diffuse.iter().chain(&specular).any(|channel| *channel != 0.0);
        (size > 0.0 && lit).then_some(diffuse)
    }
}

impl H2PointLight {
    /// The function that scales it, which sliders and frames must answer for.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        self.light.color_animation.is_some() || self.light.brightness_animation.is_some()
    }

    /// What `sub_947870` hands the passes this frame, if the light is on. It
    /// splits a bright color across `overbrighten_scale`'s shader variants;
    /// a constant can simply hold it.
    pub(crate) fn lit(
        &self,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
    ) -> Option<H2DefaultLight> {
        let value = attachment_value(&self.scale_by, values);
        let light = &self.light;
        let diffuse = light.diffuse_at(value, time)?;
        let specular =
            ce_color_interpolate(light.interpolation, light.specular[0], light.specular[1], value);
        let [dim, bright] = light.brightness;
        let mut brightness = (1.0 - value) * dim + value * bright;
        if let Some(function) = &light.brightness_animation {
            brightness *= function.evaluate(time, 0.0);
        }
        let brightness = (brightness * 2.0).clamp(0.0, BRIGHTEST);
        let size = (1.0 - value) * light.size[0] + value * light.size[1];
        let radius = (light.radius * size).max(light.specular_radius * size);
        if radius <= UNSET {
            return None;
        }
        let (position, _) = posed_on(self.node, self.position, [1.0, 0.0, 0.0], bones);
        Some(H2DefaultLight {
            position,
            color: diffuse.map(|channel| (channel * brightness).min(BRIGHTEST)),
            specular: specular.map(|channel| (channel * brightness).min(BRIGHTEST)),
            radius,
        })
    }
}

impl H2FlareAt {
    /// The function that scales it, which sliders and frames must answer for.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        let light = self.light.as_ref().is_some_and(|light| light.color_animation.is_some());
        light || self.flare.animates()
    }

    fn posed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        posed_on(self.node, self.position, self.forward, bones)
    }

    /// A light passes `lens_flare_submit` a scale of 1, whatever its marker's.
    fn marker_scaled(&self) -> f32 {
        let by_marker = self.flare.flags & SCALE_BY_MARKER != 0 && self.light.is_none();
        if by_marker { self.marker_scale } else { 1.0 }
    }

    fn value(&self, values: Option<&H2FunctionValues>) -> f32 {
        attachment_value(&self.scale_by, values)
    }

    /// The patches `lens_flare_occlusion_callback` tests: the outer, and an
    /// inner one chained under it. None for a flare that takes no test.
    pub(crate) fn occlusion_patches(
        &self,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
    ) -> Option<(CeSprite, Option<CeSprite>)> {
        let flare = &*self.flare;
        if !flare.tests_occlusion() {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let outer = flare.occlusion_radius * self.marker_scaled();
        let patch = |radius: f32| {
            let offset = match flare.occlusion_offset {
                0 => view.forward.map(|axis| -axis * radius),
                1 => forward.map(|axis| axis * radius * std::f32::consts::SQRT_2),
                _ => [0.0; 3],
            };
            CeSprite {
                center: std::array::from_fn(|axis| position[axis] + offset[axis]),
                extent: [radius; 2],
                rotation: 0.0,
                color: [0.0; 3],
                alpha: 0.0,
                image: 0,
                highlight: None,
                depth_tested: false,
            }
        };
        Some((patch(outer), flare.inner_scale.map(|scale| patch(outer * scale))))
    }

    /// `lens_flare_draw_callback`. `occlusion` is `rasterizer_occlusion_get_result`
    /// with the chained test, and of the outer test alone.
    pub(crate) fn sprites(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        occlusion: [f32; 2],
    ) -> Vec<CeSprite> {
        let flare = &*self.flare;
        let (position, forward) = self.posed(bones);
        let to_light = from_view(view, position);
        let depth = dot(to_light, view.forward);
        // `sub_92DA00` drops it past its far fade.
        if flare.fade[0] != flare.fade[1] && flare.fade[1] <= depth {
            return Vec::new();
        }
        let scaled_by_marker = flare.flags & SCALE_BY_MARKER != 0;
        let value = self.value(values);
        // A marker's scale takes the place of the attachment's, and of its color.
        let interpolation = if scaled_by_marker { 1.0 } else { value };
        let radius_scale = self.marker_scaled();
        let attached = match (scaled_by_marker, self.change_color) {
            (false, Some(index)) => change_colors[index],
            _ => [1.0; 3],
        };
        let external = match &self.light {
            Some(light) => {
                let Some(diffuse) = light.diffuse_at(value, time) else {
                    return Vec::new();
                };
                if scaled_by_marker { [1.0; 3] } else { diffuse }
            }
            None => attached,
        };
        let mut fade = 1.0;
        if flare.fade[0] != flare.fade[1] {
            fade = ((depth - flare.fade[1]) / (flare.fade[0] - flare.fade[1])).clamp(0.0, 1.0);
        }
        let [falloff, cutoff] = flare.cones;
        if falloff != cutoff {
            let slope = 1.0 / (falloff - cutoff);
            fade *= (-cutoff * slope - dot(forward, to_light) * slope).clamp(0.0, 1.0);
        }
        let [chained, outer] = occlusion;
        let mut brightness = fade * chained;
        let not_inner = fade * outer;
        if let Some(function) = &flare.brightness {
            brightness *= function.evaluate(time, 0.0);
        }
        if brightness <= 0.0 {
            return Vec::new();
        }
        let color = flare.color.as_ref().map_or([1.0; 3], |function| {
            let color = function.evaluate_color(time, 0.0);
            [color.red, color.green, color.blue]
        });
        let mut corona = corona_rotation(
            flare.rotation_function,
            flare.rotation_scale,
            to_light,
            forward,
            view,
        );
        if let Some(function) = &flare.rotation {
            corona += function.evaluate(time, 0.0) * std::f32::consts::PI;
        }
        let screen_angle = dot(to_light, view.right).atan2(dot(to_light, view.up));
        let mirror: [f32; 3] =
            std::array::from_fn(|axis| (view.forward[axis] * depth - to_light[axis]) * 2.0);
        let mut sprites = Vec::new();
        for (index, reflection) in flare.reflections.iter().enumerate() {
            let [low, high] = reflection.brightness;
            let scale = lerp(low, high, interpolation).clamp(0.0, 1.0);
            let [small, large] = reflection.radius;
            let mut radius = lerp(small, large, interpolation) * radius_scale;
            let kept = reflection.flags & NOT_AFFECTED_BY_INNER_OCCLUSION != 0;
            let alpha = if kept { not_inner } else { brightness } * scale;
            if alpha <= 0.0 || radius <= 0.0 {
                continue;
            }
            let mut rotation = reflection.rotation;
            let mut stretch = [1.0; 2];
            if index == 0 {
                rotation += corona;
                stretch = flare.corona_scale;
            }
            if reflection.flags & ALIGN_WITH_SCREEN_CENTER != 0 {
                rotation += screen_angle;
            }
            if reflection.flags & RADIUS_SCALED_BY_OCCLUSION != 0 {
                radius *= (chained * 0.5 + 0.5).clamp(0.0, 1.0);
            }
            if reflection.flags & RADIUS_NOT_SCALED_BY_DISTANCE != 0 {
                radius *= depth;
            }
            let tint: [f32; 3] = std::array::from_fn(|channel| {
                let own = color[channel] * reflection.color[channel];
                if reflection.flags & IGNORE_LIGHT_COLOR == 0 { own * external[channel] } else { own }
            });
            sprites.push(CeSprite {
                center: std::array::from_fn(|axis| {
                    position[axis] + mirror[axis] * reflection.position
                }),
                extent: [radius * stretch[0], radius * stretch[1]],
                rotation,
                color: tint,
                alpha,
                image: reflection.image,
                highlight: Some((1.0 - reflection.modulation).clamp(0.0, 1.0)),
                depth_tested: reflection.flags & OCCLUDED_BY_SOLID_OBJECTS != 0,
            });
        }
        sprites
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> CeFlareView {
        CeFlareView {
            eye: [0.0, 0.0, 0.0],
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }

    fn reflection(flags: i128) -> H2Reflection {
        H2Reflection {
            flags,
            image: 0,
            position: 0.0,
            rotation: 0.0,
            radius: [0.1, 0.3],
            brightness: [0.5, 1.0],
            modulation: 0.25,
            color: [1.0, 0.5, 0.25],
        }
    }

    fn flare(flags: i128, reflections: Vec<H2Reflection>) -> H2FlareAt {
        H2FlareAt {
            object: 0,
            node: 0,
            position: [2.0, 0.0, 0.0],
            forward: [-1.0, 0.0, 0.0],
            marker_scale: 3.0,
            scale_by: "power".to_owned(),
            change_color: None,
            light: None,
            flare: Arc::new(H2Flare {
                flags,
                cones: [1.0, 1.0],
                fade: [0.0, 0.0],
                occlusion_radius: 0.2,
                occlusion_offset: 2,
                inner_scale: Some(0.5),
                corona_scale: [2.0, 3.0],
                rotation_function: 0,
                rotation_scale: 0.0,
                reflections,
                brightness: None,
                color: None,
                rotation: None,
                images: Vec::new(),
            }),
        }
    }

    fn values(power: f32) -> H2FunctionValues {
        H2FunctionValues::from([("power".to_owned(), power)])
    }

    #[test]
    fn a_function_runs_a_reflection_between_its_bounds() {
        let at = flare(0, vec![reflection(0)]);
        let sprites = at.sprites(Some(&values(0.5)), &[[1.0; 3]; 4], None, &view(), 0.0, [1.0, 1.0]);
        assert!((sprites[0].alpha - 0.75).abs() < 1e-5);
        assert!((sprites[0].extent[0] - 0.4).abs() < 1e-5, "0.2, then the corona's stretch");
        assert_eq!(sprites[0].highlight, Some(0.75));
    }

    #[test]
    fn scale_by_marker_takes_the_upper_bounds_and_the_markers_scale() {
        let at = flare(SCALE_BY_MARKER, vec![reflection(0)]);
        let sprites = at.sprites(Some(&values(0.0)), &[[1.0; 3]; 4], None, &view(), 0.0, [1.0, 1.0]);
        assert!((sprites[0].alpha - 1.0).abs() < 1e-5);
        assert!((sprites[0].extent[1] - 2.7).abs() < 1e-5);
    }

    #[test]
    fn only_some_reflections_ignore_inner_occlusion() {
        let mut at = flare(
            0,
            vec![reflection(0), reflection(NOT_AFFECTED_BY_INNER_OCCLUSION)],
        );
        at.scale_by.clear();
        let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, [0.0, 0.5]);
        assert_eq!(sprites.len(), 0, "with the chained test all hidden nothing draws");
        let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, [0.25, 0.5]);
        assert!((sprites[0].alpha - 0.25).abs() < 1e-5);
        assert!((sprites[1].alpha - 0.5).abs() < 1e-5, "the outer test alone");
    }

    #[test]
    fn a_change_color_tints_unless_a_reflection_ignores_it() {
        let mut at = flare(0, vec![reflection(0), reflection(IGNORE_LIGHT_COLOR)]);
        at.scale_by.clear();
        at.change_color = Some(1);
        let colors = [[1.0; 3], [0.5, 0.5, 0.5], [1.0; 3], [1.0; 3]];
        let sprites = at.sprites(None, &colors, None, &view(), 0.0, [1.0, 1.0]);
        assert_eq!(sprites[0].color, [0.5, 0.25, 0.125]);
        assert_eq!(sprites[1].color, [1.0, 0.5, 0.25]);
    }

    fn light() -> H2Light {
        H2Light {
            interpolation: 0,
            size: [1.0, 1.0],
            specular: [[0.0; 3]; 2],
            diffuse: [[0.0, 0.0, 0.0], [1.0, 0.5, 0.25]],
            brightness: [1.0, 1.0],
            color_animation: None,
            brightness_animation: None,
            radius: 2.0,
            specular_radius: 3.0,
        }
    }

    #[test]
    fn a_name_nothing_answers_to_is_zero_and_no_name_is_one() {
        let mut at = flare(0, vec![reflection(0)]);
        at.scale_by = "no_such_function".to_owned();
        assert_eq!(at.value(None), 0.0);
        assert_eq!(at.value(Some(&values(0.5))), 0.0);
        at.scale_by.clear();
        assert_eq!(at.value(None), 1.0);
    }

    #[test]
    fn a_light_tints_its_flare_by_its_own_color_and_no_change_color() {
        let mut at = flare(0, vec![reflection(0)]);
        at.light = Some(light());
        at.change_color = Some(0);
        let colors = [[0.0; 3]; 4];
        let sprites = at.sprites(Some(&values(0.5)), &colors, None, &view(), 0.0, [1.0, 1.0]);
        assert_eq!(sprites[0].color, [0.5, 0.125, 0.03125]);
    }

    #[test]
    fn a_light_with_no_color_or_size_is_out() {
        let mut at = flare(0, vec![reflection(0)]);
        at.light = Some(light());
        let none = [[1.0; 3]; 4];
        assert!(at.sprites(Some(&values(0.0)), &none, None, &view(), 0.0, [1.0, 1.0]).is_empty());
        at.light = Some(H2Light { size: [0.0, 0.0], ..light() });
        assert!(at.sprites(Some(&values(1.0)), &none, None, &view(), 0.0, [1.0, 1.0]).is_empty());
    }

    #[test]
    fn a_light_ignores_its_markers_scale() {
        let mut at = flare(SCALE_BY_MARKER, vec![reflection(0)]);
        at.light = Some(light());
        let none = [[1.0; 3]; 4];
        let sprites = at.sprites(Some(&values(0.5)), &none, None, &view(), 0.0, [1.0, 1.0]);
        assert!((sprites[0].extent[1] - 0.9).abs() < 1e-5, "0.3 by the corona's 3");
    }

    fn point(light: H2Light) -> H2PointLight {
        H2PointLight {
            object: 0,
            node: 0,
            position: [1.0, 2.0, 3.0],
            scale_by: "power".to_owned(),
            light,
        }
    }

    #[test]
    fn a_light_doubles_its_brightness_and_reaches_its_larger_radius() {
        let lit = point(light()).lit(Some(&values(1.0)), None, 0.0).unwrap();
        assert_eq!(lit.position, [1.0, 2.0, 3.0]);
        assert_eq!(lit.color, [2.0, 1.0, 0.5]);
        assert_eq!(lit.radius, 3.0);
    }

    #[test]
    fn a_bright_light_stops_at_four() {
        let bright = H2Light { brightness: [3.0, 3.0], ..light() };
        let lit = point(bright).lit(Some(&values(1.0)), None, 0.0).unwrap();
        assert_eq!(lit.color, [4.0, 2.0, 1.0]);
    }

    #[test]
    fn a_light_with_no_function_value_is_out_and_no_name_is_on() {
        assert!(point(light()).lit(None, None, 0.0).is_some(), "`power` rests at 1");
        let mut out = point(light());
        out.scale_by = "no_such_function".to_owned();
        assert!(out.lit(None, None, 0.0).is_none());
        out.scale_by.clear();
        assert!(out.lit(None, None, 0.0).is_some());
    }

    #[test]
    fn a_flare_with_no_test_has_no_patches() {
        let tested = flare(0, vec![reflection(0)]);
        let (outer, inner) = tested.occlusion_patches(None, &view()).unwrap();
        assert_eq!((outer.extent[0], inner.unwrap().extent[0]), (0.2, 0.1));
        assert!(flare(NO_OCCLUSION_TEST, Vec::new()).occlusion_patches(None, &view()).is_none());
    }
}
