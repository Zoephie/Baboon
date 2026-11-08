//! Halo 2's lens flares on objects: `render_submit`'s attachments, then
//! `rasterizer_lens_flares.cpp`. The shape is Halo CE's, in
//! [`super::super::ce::lights`], but the sums are not: brightness, color and rotation
//! animate, occlusion comes in two sizes, and the pixel shader adds a highlight.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use crate::app::model_preview::common::marker::MarkerFrame;
use super::super::common::fields::{elements, function_slot_of, has_flag, has_flag_in_any, object_block};
use crate::app::model_preview::common::flares::FlareSprite;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::super::effects::{EffectTimes, for_each_attached_part, part_markers};
use blam_tags::tag_function::h2::transition_evaluate;
use super::super::dx9::passes::{H2DefaultLight, H2Layer, ce_color_interpolate};
use super::super::common::image::H2Image;
use super::super::functions::object_input;
use super::super::common::named_function::tag_function_at;
use super::super::materials::decode_h2_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{UNSET, cross, dot, lerp};
use crate::app::model_preview::common::random::lcg_step;
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::flares::OcclusionOffset;
use crate::app::model_preview::dx9::passes::ce_color_interpolation;

/// A `lens_flare`'s `flags`, by name.
#[derive(Debug, Clone, Copy, Default)]
struct FlareFlags {
    no_occlusion_test: bool,
    only_first_person: bool,
    scale_by_marker: bool,
}

/// A reflection's `flags`, by name.
#[derive(Debug, Clone, Copy, Default)]
struct ReflectionFlags {
    align_with_screen_center: bool,
    radius_not_scaled_by_distance: bool,
    radius_scaled_by_occlusion: bool,
    occluded_by_solid_objects: bool,
    ignore_light_color: bool,
    not_affected_by_inner_occlusion: bool,
}
/// `sub_946960`'s divisor for the near plane's edge, `dword_EB9D38`.
const NEAR_EDGE_DIVISOR: f32 = 40.0;
/// What `sub_946960` leaves between the apex and the near plane's edge.
const NEAR_EDGE_BIAS: f32 = 0.0078125;
/// The bounds tool's postprocess holds `field of view` to, in degrees.
const FIELD_OF_VIEW: [f32; 2] = [1.0, 120.0];
/// What the brightness of `sub_8ED2A0` comes to, at most.
const BRIGHTEST: f32 = 4.0;
/// `occlusion inner radius scale`.
const INNER_SCALES: [f32; 7] = [1.0, 0.5, 0.25, 0.125, 0.0625, 0.03125, 0.015625];

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
struct H2Reflection {
    flags: ReflectionFlags,
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
    flags: FlareFlags,
    /// Cosines of its `falloff angle` and `cutoff angle`.
    cones: [f32; 2],
    fade: [f32; 2],
    occlusion_radius: f32,
    occlusion_offset: OcclusionOffset,
    /// The share of the outer patch a second test takes, if there is one.
    inner_scale: Option<f32>,
    corona_scale: [f32; 2],
    rotation_function: i128,
    rotation_scale: f32,
    reflections: Vec<H2Reflection>,
    brightness: Option<Arc<TagFunction>>,
    color: Option<Arc<TagFunction>>,
    rotation: Option<Arc<TagFunction>>,
    /// The animation's `synchronized`: every instance runs on the same clock.
    /// Without it, `lens_flare_draw_callback` shifts each one's.
    synchronized: bool,
    /// `falloff function`: how an effect's flare dims over its event.
    falloff: u8,
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
    /// The diffuse pass's variant its `diffuse contrast` picks.
    diffuse_layer: H2Layer,
    radius: f32,
    specular_radius: f32,
    /// `duration`: how long it lasts when an effect makes it, or 0 for ever.
    effect_duration: f32,
    /// The second `falloff function`: how it dims over that duration.
    effect_falloff: u8,
}

/// A light an effect's event makes, `sub_8EBD10`: it stands from when its
/// event begins. One with a `duration` dims by its falloff, `sub_8ED2A0`, and
/// is deleted after it, `sub_8EE2E0`; without, nothing deletes it.
#[derive(Debug, Clone)]
pub(crate) struct H2EffectLight {
    times: Arc<EffectTimes>,
    event: usize,
    /// `A scales values` takes the type-specific bit: `sub_515FC0` scales it
    /// by the effect's A scale, which the attachment's function gives.
    scaled: bool,
}

impl H2EffectLight {
    pub(crate) fn new(times: Arc<EffectTimes>, event: usize, scaled: bool) -> Self {
        Self { times, event, scaled }
    }

    /// The scale `sub_8ED2A0` reads, or none while the light does not exist.
    /// Without a duration it takes the upper colors and brightness whatever
    /// the scale, and only its size follows; that differs only when the effect
    /// scales it below 1.
    fn value(&self, light: &H2Light, on: f32, time: f32, onset: f32) -> Option<f32> {
        if on <= 0.0 {
            return None;
        }
        let since = self.times.since_begun(self.event, time - onset)?;
        let scale = if self.scaled { on } else { 1.0 };
        if light.effect_duration <= 0.0 {
            return Some(scale);
        }
        if since > light.effect_duration {
            return None;
        }
        let faded = transition_evaluate(light.effect_falloff, since / light.effect_duration);
        Some(scale * (1.0 - faded))
    }
}

/// An event's `lens_flare` or `light_volume` part on an attachment's effect,
/// which `sub_515280` draws at each of its location's markers while its event
/// runs (`render_submit_special_model_groups`), not `effect_generate_part`.
#[derive(Debug, Clone)]
pub(crate) struct H2EffectPart {
    times: Arc<EffectTimes>,
    event: usize,
    /// `A scales values` and `B scales values` take the type-specific bit:
    /// the effect's A and B scales brighten it.
    scaled: [bool; 2],
    /// The effect's B scale, a name the object's functions answer to.
    secondary_by: String,
}

/// How `sub_515280` lights a part this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2PartLight {
    /// The A and B scales it takes.
    pub(crate) brightness: f32,
    /// `1 - age / duration`, for an event longer than a tick.
    pub(crate) fade: f32,
    /// Whether its event has a duration; one of none neither fades nor falls off.
    pub(crate) lasts: bool,
}

/// `sub_515280` fades a part over an event longer than this, or at its end.
const SHORTEST_FADING_EVENT: f32 = 0.033_433_337;

impl H2EffectPart {
    pub(in crate::app::model_preview) fn new(part: &super::super::effects::EffectPart<'_>) -> Self {
        Self {
            times: Arc::clone(&part.times),
            event: part.event,
            scaled: [part.scaled, part.b_scaled],
            secondary_by: part.effect.secondary_by.clone(),
        }
    }

    /// How it is lit at `time`, with its effect's A scale `primary` names; none
    /// while the effect is off or another of its events runs.
    pub(crate) fn light(
        &self,
        primary: &str,
        values: Option<&FunctionValues>,
        time: f32,
        onset: f32,
    ) -> Option<H2PartLight> {
        let on = attachment_value(primary, values);
        if on <= 0.0 {
            return None;
        }
        let age = self.times.age(self.event, time - onset)?;
        let duration = self.times.duration(self.event)?;
        let secondary = attachment_value(&self.secondary_by, values);
        let [a, b] = self.scaled;
        let brightness = if a { on } else { 1.0 } * if b { secondary } else { 1.0 };
        let fade = if duration > SHORTEST_FADING_EVENT { (1.0 - age).clamp(0.0, 1.0) } else { 1.0 };
        Some(H2PartLight { brightness, fade, lasts: duration != 0.0 })
    }
}

/// What makes a flare that is no attachment of its own: a light an effect
/// makes, or an effect's `lens_flare` part.
#[derive(Debug, Clone)]
enum H2FlareEffect {
    Light(H2EffectLight),
    Part(H2EffectPart),
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
    effect: Option<H2EffectLight>,
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
}

/// A frustum's shape.
#[derive(Debug, Clone)]
struct H2Frustum {
    near_width: f32,
    /// `height stretch` by the gel's aspect.
    stretch: f32,
    half_view: f32,
    falloff: f32,
    cutoff: f32,
    gel: Option<Arc<H2Image>>,
}

/// A frustum light at its marker, which only a gel pass draws.
#[derive(Debug, Clone)]
pub(crate) struct H2GelLight {
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    /// `primary scale`, a name the object's functions answer to.
    scale_by: String,
    light: H2Light,
    frustum: H2Frustum,
    effect: Option<H2EffectLight>,
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
}

/// The vectors `lightaccum_frustum_no_specular_gel` reads as c33 to c36, in a
/// light's frame of `sub_946960`. All but `apex` take the vector to the light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct H2GelRows {
    pub(crate) apex: [f32; 3],
    pub(crate) rows: [[f32; 4]; 4],
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
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    /// `primary scale`, a name the object's functions answer to.
    scale_by: String,
    /// `change color`: which of the four, if any.
    change_color: Option<usize>,
    /// The light it belongs to, if it is no attachment of its own.
    light: Option<H2Light>,
    effect: Option<H2FlareEffect>,
    pub(crate) flare: Arc<H2Flare>,
}

impl H2FlareAt {
    /// `lens_flare_draw_callback` for an unsynchronized flare: seconds ahead,
    /// the first byte of a hash over 256 and the second whole. The game hashes
    /// the address of the flare's draw record; the preview hashes where the
    /// flare sits, which is as fixed for it and differs between flares.
    fn unsynchronized_offset(&self) -> f32 {
        let seed = self.position.iter().fold(self.node as u32, |seed, axis| {
            seed.rotate_left(5) ^ axis.to_bits()
        });
        let hashed = lcg_step(seed).to_le_bytes();
        f32::from(hashed[0]) / 256.0 + f32::from(hashed[1])
    }
}

/// The second of two fields that share a name, by its place in the struct.
fn second_field_int(root: TagStruct<'_>, name: &str) -> Option<i128> {
    let field = root.fields().filter(|field| field.name() == name).nth(1)?;
    root.read_int_any(&format!("{name}#{}", field.ordinal()))
}

/// What a light's colors and size interpolate by: its attachment's function,
/// or for an effect's light, its clock. None while that light does not exist.
fn light_value(
    light: &H2Light,
    effect: Option<&H2EffectLight>,
    scale_by: &str,
    values: Option<&FunctionValues>,
    time: f32,
    onset: f32,
) -> Option<f32> {
    let on = attachment_value(scale_by, values);
    match effect {
        Some(effect) => effect.value(light, on, time, onset),
        None => Some(on),
    }
}

/// What an attachment's `scale by` reads: full with none named, and the
/// idle object's input for a name nothing gave a value.
pub(in crate::app::model_preview) fn attachment_value(name: &str, values: Option<&FunctionValues>) -> f32 {
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

fn read_h2_flare(reader: &TagReader, path: &str) -> Option<H2Flare> {
    let tag = reader.load(path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let bitmap = root.read_tag_ref_path("bitmap").filter(|bitmap| !bitmap.is_empty())?;
    let mut images = Vec::new();
    let mut indices = Vec::new();
    let mut reflections = Vec::new();
    for reflection in elements(root, "reflections") {
        let index = reflection.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
        let image = indices.iter().position(|known| *known == index).or_else(|| {
            let image = decode_h2_image(reader, &bitmap, SamplerKind::Texture2D, index)?;
            images.push(Arc::new(image));
            indices.push(index);
            Some(images.len() - 1)
        })?;
        let radius = reflection.read_real_bounds("radius");
        let brightness = reflection.read_fraction_bounds("brightness");
        let color = reflection.read_rgb("color");
        reflections.push(H2Reflection {
            flags: {
                let set = |option: &str| has_flag(reflection, "flags", option);
                ReflectionFlags {
                    align_with_screen_center: set("align rotation with screen center"),
                    radius_not_scaled_by_distance: set("radius NOT scaled by distance"),
                    radius_scaled_by_occlusion: set("radius scaled by occlusion factor"),
                    occluded_by_solid_objects: set("occluded by solid objects"),
                    ignore_light_color: set("ignore light color"),
                    not_affected_by_inner_occlusion: set("not affected by inner occlusion"),
                }
            },
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
        flags: FlareFlags {
            no_occlusion_test: has_flag(root, "flags", "no occlusion test"),
            only_first_person: has_flag(root, "flags", "only render in first person"),
            scale_by_marker: has_flag(root, "flags", "scale by marker"),
        },
        cones: [angle("falloff angle"), angle("cutoff angle")],
        fade: [
            root.read_real("near fade distance").unwrap_or(0.0),
            root.read_real("far fade distance").unwrap_or(0.0),
        ],
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        occlusion_offset: OcclusionOffset::read(root),
        inner_scale: (inner != 0).then_some(INNER_SCALES[inner]),
        corona_scale: [unset_as_one(corona.i), unset_as_one(corona.j)],
        rotation_function: root.read_int_any("rotation function").unwrap_or(0),
        rotation_scale: root.read_real("rotation function scale").unwrap_or(0.0),
        reflections,
        brightness: scale_function(root, "brightness"),
        color: scale_function(root, "color"),
        rotation: scale_function(root, "rotation"),
        // Its second `flags` field holds it.
        synchronized: has_flag_in_any(root, "flags", "synchronized"),
        falloff: root.read_int_any("falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
        images,
    })
}

/// Whether a `light` can ever be on in a third person view: nothing else sets
/// the bit `sub_8ED2A0` turns a light on through.
pub(in crate::app::model_preview) fn light_can_shine(root: TagStruct<'_>) -> bool {
    let flag = |option: &str| has_flag(root, "flags", option);
    // `default lightmap setting`'s `lightmaps only`: the game makes no such light.
    !flag("no illumination")
        && !flag("only render in first person")
        && !root.field("default lightmap setting").is_some_and(|field| field.enum_is("lightmaps only"))
}

/// `sub_8EB720`, and what tool's postprocess gives a tag for the game: the
/// light, whether it lights the model on the sphere path, and its flare. `None`
/// for a light the game makes nothing of in a third person view.
fn read_h2_light(
    reader: &TagReader,
    path: &str,
) -> Option<(H2Light, bool, Option<H2Frustum>, Option<Arc<H2Flare>>)> {
    let tag = reader.load(path, "light", b"ligh").ok()?;
    let root = tag.root();
    let flag = |option: &str| has_flag(root, "flags", option);
    if !light_can_shine(root) {
        return None;
    }
    let flare = root.read_tag_ref_path("lens flare").filter(|path| !path.is_empty());
    let flare = flare.and_then(|path| read_h2_flare(reader, &path)).map(Arc::new);
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
    if flag("no specular") {
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
        interpolation: ce_color_interpolation(
            has_flag(root, "interpolation flags", "blend in hsv"),
            has_flag(root, "interpolation flags", "...more colors"),
        ),
        size,
        specular: saturate(specular),
        diffuse: saturate(diffuse),
        brightness,
        color_animation: scale_function(root, "color animation"),
        brightness_animation: scale_function(root, "brightness animation"),
        diffuse_layer: H2Layer::diffuse_with_contrast(root.read_enum_option("diffuse contrast").as_deref()),
        radius,
        specular_radius: specular_radius.unwrap_or(radius),
        effect_duration: root.read_real("duration").unwrap_or(0.0),
        effect_falloff: second_field_int(root, "falloff function").and_then(|index| u8::try_from(index).ok()).unwrap_or(0),
    };
    // `sub_6BA660` leaves the object unlit only with both.
    let lit = !(flag("ignore parent object") && flag("ignore all parents"));
    // Implementation 1 has no gel to read, so a sphere's own never shows.
    // A sphere is the only type that takes the sphere path.
    let kind = |option: &str| root.field("type").is_some_and(|field| field.enum_is(option));
    let lights_model = kind("sphere") && lit;
    let frustum = lit.then(|| read_h2_frustum(reader, root)).flatten();
    Some((light, lights_model, frustum, flare))
}

/// The shape `sub_8EB100` and `sub_8E84F0` read from a frustum `light`, with
/// tool's postprocess applied. A shadow's pass needs a map of the level from
/// the light, which a preview has none of, so it lights without one.
fn read_h2_frustum(reader: &TagReader, root: TagStruct<'_>) -> Option<H2Frustum> {
    let is = |field: &str, option: &str| root.field(field).is_some_and(|field| field.enum_is(option));
    // Orthogonal lights assert in `sub_946960`, so these are the frustums;
    // tool's postprocess makes `directional` of a frustum's default falloff.
    let (projective, pyramid) = (is("type", "projective"), is("type", "pyramid"));
    let falloff = is("falloff geometry", "default") || is("falloff geometry", "directional");
    if !(projective || pyramid) || !falloff {
        return None;
    }
    let gel = root.read_tag_ref_path("gel map").filter(|gel| !gel.is_empty());
    let gel = gel
        .and_then(|gel| decode_h2_image(reader, &gel, SamplerKind::Texture2D, 0))
        .map(Arc::new);
    let aspect = gel.as_deref().map_or(1.0, |image| match image {
        H2Image::Flat(image) => image.height as f32 / image.width as f32,
        _ => 1.0,
    });
    let stretch = root.read_real("height stretch").filter(|stretch| *stretch != 0.0).unwrap_or(1.0);
    let [narrowest, widest] = FIELD_OF_VIEW;
    let view = root.read_real("field of view").unwrap_or(0.0).clamp(narrowest, widest);
    Some(H2Frustum {
        near_width: if pyramid { 0.0 } else { root.read_real("near width").unwrap_or(0.0) },
        stretch: stretch * aspect,
        half_view: view.to_radians() * 0.5,
        falloff: root.read_real("falloff distance").unwrap_or(0.0),
        cutoff: root.read_real("cutoff distance").unwrap_or(0.0),
        gel,
    })
}

/// Where one light goes, and what drives it.
struct H2LightPlace {
    place: MarkerFrame,
    node: usize,
    marker_scale: f32,
    only_under: Option<(String, String)>,
    scale_by: String,
    change_color: Option<usize>,
    effect: Option<H2EffectLight>,
}

/// What a light puts on the object: a sphere's light, a frustum's gel pass,
/// and the flare `light_submit_lens_flares` submits for it.
fn place_h2_light(
    reader: &TagReader,
    path: &str,
    owner: u8,
    at: H2LightPlace,
    found: &mut (Vec<H2FlareAt>, Vec<H2PointLight>, Vec<H2GelLight>),
) {
    let Some((light, lights_model, frustum, flare)) = read_h2_light(reader, path) else {
        return;
    };
    let (place, node) = (at.place, at.node);
    if let Some(frustum) = frustum {
        found.2.push(H2GelLight {
            object: owner,
            node,
            position: place.position,
            forward: place.axes[0],
            up: place.axes[2],
            scale_by: at.scale_by.clone(),
            light: light.clone(),
            frustum,
            effect: at.effect.clone(),
            only_under: at.only_under.clone(),
        });
    }
    if lights_model {
        found.1.push(H2PointLight {
            object: owner,
            node,
            position: place.position,
            scale_by: at.scale_by.clone(),
            light: light.clone(),
            effect: at.effect.clone(),
            only_under: at.only_under.clone(),
        });
    }
    // A preview is a third person view.
    let Some(flare) = flare.filter(|flare| !flare.flags.only_first_person) else {
        return;
    };
    found.0.push(H2FlareAt {
        object: owner,
        node,
        position: place.position,
        forward: place.axes[0],
        marker_scale: at.marker_scale,
        only_under: at.only_under,
        scale_by: at.scale_by,
        change_color: at.change_color,
        light: Some(light),
        effect: at.effect.map(H2FlareEffect::Light),
        flare,
    });
}

/// `render_submit_object_attachments`' `lens_flare`s, at every marker of their
/// names, and `light_submit_lens_flares`' lights, which take the first. Then
/// the lights its effects' events make, `effect_generate_part`'s only light,
/// and their `lens_flare` parts, which `sub_515280` submits at every marker
/// of their location: scale 1, tinted by the effect's color (its object's
/// change color, or white).
pub(in crate::app::model_preview) fn read_h2_flares(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> (Vec<H2FlareAt>, Vec<H2PointLight>, Vec<H2GelLight>) {
    let mut found = (Vec::new(), Vec::new(), Vec::new());
    let lens_group = u32::from_be_bytes(*b"lens");
    let light_group = u32::from_be_bytes(*b"ligh");
    let attachments = object_block(object, "attachments").map(|block| elements(block, "attachments"));
    for attachment in attachments.unwrap_or_default() {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let name = attachment.read_string_id("marker").unwrap_or_default();
        let scale_by = attachment.read_string_id("primary scale").unwrap_or_default();
        let change_color = function_slot_of(attachment, "change color");
        let mut named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name));
        if group == light_group {
            let Some(marker) = named.next() else {
                continue;
            };
            let (place, node) = frame(marker);
            let at = H2LightPlace {
                place,
                node,
                marker_scale: marker.scale,
                only_under: marker.only_under.clone(),
                scale_by,
                change_color,
                effect: None,
            };
            place_h2_light(reader, &path, owner, at, &mut found);
            continue;
        }
        if group != lens_group {
            continue;
        }
        let Some(flare) = read_h2_flare(reader, &path).map(Arc::new) else {
            continue;
        };
        if flare.flags.only_first_person {
            continue;
        }
        for marker in named {
            let (place, node) = frame(marker);
            found.0.push(H2FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                marker_scale: marker.scale,
                only_under: marker.only_under.clone(),
                scale_by: scale_by.clone(),
                change_color,
                light: None,
                effect: None,
                flare: Arc::clone(&flare),
            });
        }
    }
    for_each_attached_part(reader, object, |part| {
        if part.group == lens_group {
            let third_person = |flare: &H2Flare| !flare.flags.only_first_person;
            let Some(flare) = read_h2_flare(reader, &part.path).filter(third_person) else {
                return;
            };
            let flare = Arc::new(flare);
            let change_color = part.effect.change_color;
            for marker in part_markers(markers, &part.marker, part.effect.once) {
                let (place, node) = frame(&marker);
                let (place, node) = part.placed(place, node);
                found.0.push(H2FlareAt {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    marker_scale: 1.0,
                    only_under: marker.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    change_color,
                    light: None,
                    effect: Some(H2FlareEffect::Part(H2EffectPart::new(&part))),
                    flare: Arc::clone(&flare),
                });
            }
            return;
        }
        if part.group != light_group {
            return;
        }
        // `sub_8EBD10` takes the location's node, or none if not attached.
        let Some(marker) = part_markers(markers, &part.marker, part.effect.once).into_iter().next() else {
            return;
        };
        let (place, node) = frame(&marker);
        let (place, node) = part.placed(place, node);
        let at = H2LightPlace {
            place,
            node,
            marker_scale: 1.0,
            only_under: marker.only_under.clone(),
            scale_by: part.effect.scale_by.clone(),
            change_color: None,
            effect: Some(H2EffectLight::new(Arc::clone(&part.times), part.event, part.scaled)),
        };
        place_h2_light(reader, &part.path, owner, at, &mut found);
    });
    found
}




fn from_view(view: &ViewFrame, point: [f32; 3]) -> [f32; 3] {
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
    view: &ViewFrame,
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
        !self.flags.no_occlusion_test
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

    /// `sub_8ED2A0`'s brightness for `value`.
    fn shining(&self, value: f32, time: f32) -> f32 {
        let [dim, bright] = self.brightness;
        let mut brightness = (1.0 - value) * dim + value * bright;
        if let Some(function) = &self.brightness_animation {
            brightness *= function.evaluate(time, 0.0);
        }
        (brightness * 2.0).clamp(0.0, BRIGHTEST)
    }
}

impl H2PointLight {
    /// The layer its diffuse draws on.
    pub(crate) fn diffuse_layer(&self) -> H2Layer {
        self.light.diffuse_layer
    }

    /// The function that scales it, which sliders and frames must answer for.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with nothing but time.
    pub(crate) fn animates(&self) -> bool {
        self.light.color_animation.is_some() || self.light.brightness_animation.is_some()
    }

    /// What `sub_947870` hands the passes this frame, if the light is on.
    /// `onset` is when its function last came on, for an effect's light.
    pub(crate) fn lit(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<H2DefaultLight> {
        let value = light_value(&self.light, self.effect.as_ref(), &self.scale_by, values, time, onset)?;
        let light = &self.light;
        let diffuse = light.diffuse_at(value, time)?;
        let specular =
            ce_color_interpolate(light.interpolation, light.specular[0], light.specular[1], value);
        let brightness = light.shining(value, time);
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

impl H2Frustum {
    /// `sub_8E84F0` and `sub_946960` at `size`, for a marker's `position`.
    /// `falloff geometry` directional; gel animation rests at no offset.
    fn rows(&self, position: [f32; 3], forward: [f32; 3], up: [f32; 3], size: f32) -> H2GelRows {
        let tan_x = self.half_view.tan();
        let tan_y = tan_x * self.stretch;
        let (falloff, cutoff) = (self.falloff * size, self.cutoff * size);
        let behind = self.near_width / tan_x * 0.5;
        let apex = std::array::from_fn(|axis| position[axis] - forward[axis] * behind);
        let left = cross(forward, up);
        let scale = -1.0 / (cutoff - falloff);
        let across = |axis: usize| (forward[axis] - left[axis] / tan_x) * scale * 0.5;
        let along = |axis: usize| (forward[axis] + up[axis] / tan_y) * scale * 0.5;
        let reach = forward.map(|axis| axis * scale);
        let edge = (cutoff - falloff) * 0.5 / NEAR_EDGE_DIVISOR;
        H2GelRows {
            apex,
            rows: [
                [across(0), across(1), across(2), 0.0],
                [along(0), along(1), along(2), 0.0],
                [reach[0], reach[1], reach[2], (behind + falloff) * scale],
                [
                    reach[0] * edge,
                    reach[1] * edge,
                    reach[2] * edge,
                    edge * scale * (behind + NEAR_EDGE_BIAS) + 0.5,
                ],
            ],
        }
    }
}

impl H2GelLight {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    pub(crate) fn animates(&self) -> bool {
        self.light.brightness_animation.is_some() || self.light.color_animation.is_some()
    }

    pub(crate) fn gel(&self) -> Option<&Arc<H2Image>> {
        self.frustum.gel.as_ref()
    }

    /// The light for a gel pass, and its frustum there. None while it's out.
    pub(crate) fn lit(
        &self,
        values: Option<&FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        time: f32,
        onset: f32,
    ) -> Option<(H2DefaultLight, H2GelRows)> {
        let value = light_value(&self.light, self.effect.as_ref(), &self.scale_by, values, time, onset)?;
        let light = &self.light;
        let diffuse = light.diffuse_at(value, time)?;
        let size = (1.0 - value) * light.size[0] + value * light.size[1];
        let brightness = light.shining(value, time);
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let rows = self.frustum.rows(position, forward, up, size);
        let color = diffuse.map(|channel| (channel * brightness).min(BRIGHTEST));
        let specular = ce_color_interpolate(light.interpolation, light.specular[0], light.specular[1], value);
        let specular = specular.map(|channel| (channel * brightness).min(BRIGHTEST));
        Some((H2DefaultLight { position: rows.apex, color, specular, radius: 1.0 }, rows))
    }
}

/// `rasterizer_render_light`'s flag 1: whether a frustum takes implementation 0.
pub(crate) fn has_specular(light: &H2DefaultLight) -> bool {
    light.specular.iter().any(|channel| *channel > 0.01)
}

/// `sub_946960`: a color past 1 goes to the pass halved or quartered, for a
/// variant that scales its output back up, as passes saturate on the way. The
/// preview keeps the first variant: the same pass drawn again is the same sum.
pub(crate) fn overbrighten(light: &H2DefaultLight) -> (H2DefaultLight, usize) {
    let brightest = light.color.iter().chain(&light.specular).copied().fold(0.0, f32::max);
    let (scale, draws) = match brightest {
        value if value <= 1.0 => (1.0, 1),
        value if value <= 2.0 => (0.5, 2),
        _ => (0.25, 4),
    };
    let scaled = |color: [f32; 3]| color.map(|channel| (channel * scale).clamp(0.0, 1.0));
    let dimmed = H2DefaultLight {
        color: scaled(light.color),
        specular: scaled(light.specular),
        ..*light
    };
    (dimmed, draws)
}

impl H2FlareAt {
    #[cfg(test)]
    pub(crate) fn effect_part(&self) -> Option<&H2EffectPart> {
        match &self.effect {
            Some(H2FlareEffect::Part(part)) => Some(part),
            _ => None,
        }
    }



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
        let by_marker = self.flare.flags.scale_by_marker && self.light.is_none();
        if by_marker { self.marker_scale } else { 1.0 }
    }

    fn value(&self, values: Option<&FunctionValues>, time: f32, onset: f32) -> Option<f32> {
        match (&self.effect, &self.light) {
            (Some(H2FlareEffect::Light(effect)), Some(light)) => {
                effect.value(light, attachment_value(&self.scale_by, values), time, onset)
            }
            // `sub_515280`: dimmed by the flare's falloff over its event.
            (Some(H2FlareEffect::Part(part)), _) => {
                let lit = part.light(&self.scale_by, values, time, onset)?;
                let dimmed = || 1.0 - transition_evaluate(self.flare.falloff, 1.0 - lit.fade);
                let falloff = if lit.lasts { dimmed() } else { 1.0 };
                Some(lit.brightness * falloff)
            }
            _ => Some(attachment_value(&self.scale_by, values)),
        }
    }

    /// The patches `lens_flare_occlusion_callback` tests: the outer, and an
    /// inner one chained under it. None for a flare that takes no test.
    pub(crate) fn occlusion_patches(
        &self,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
    ) -> Option<(FlareSprite, Option<FlareSprite>)> {
        let flare = &*self.flare;
        if !flare.tests_occlusion() {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let outer = flare.occlusion_radius * self.marker_scaled();
        let patch = |radius: f32| {
            let offset = match flare.occlusion_offset {
                OcclusionOffset::TowardViewer => view.forward.map(|axis| -axis * radius),
                OcclusionOffset::MarkerForward => forward.map(|axis| axis * radius * std::f32::consts::SQRT_2),
                OcclusionOffset::None => [0.0; 3],
            };
            FlareSprite {
                center: std::array::from_fn(|axis| position[axis] + offset[axis]),
                extent: [radius; 2],
                rotation: 0.0,
                color: [0.0; 3],
                alpha: 0.0,
                image: 0,
                highlight: None,
                tint: None,
                depth_tested: false,
            }
        };
        Some((patch(outer), flare.inner_scale.map(|scale| patch(outer * scale))))
    }

    /// `lens_flare_draw_callback`. `occlusion` is `rasterizer_occlusion_get_result`
    /// with the chained test, and of the outer test alone.
    pub(crate) fn sprites(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
        occlusion: [f32; 2],
    ) -> Vec<FlareSprite> {
        let flare = &*self.flare;
        let (position, forward) = self.posed(bones);
        let to_light = from_view(view, position);
        let depth = dot(to_light, view.forward);
        // `sub_92DA00` drops it past its far fade.
        if flare.fade[0] != flare.fade[1] && flare.fade[1] <= depth {
            return Vec::new();
        }
        let scaled_by_marker = flare.flags.scale_by_marker;
        let Some(value) = self.value(values, time, onset) else {
            return Vec::new();
        };
        // A marker's scale takes the place of the attachment's, and of its color.
        let interpolation = if scaled_by_marker { 1.0 } else { value };
        let radius_scale = self.marker_scaled();
        let attached = match (scaled_by_marker, self.change_color) {
            (false, Some(index)) => change_colors.get(index).copied().unwrap_or([1.0; 3]),
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
        let time = if flare.synchronized { time } else { time + self.unsynchronized_offset() };
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
            let kept = reflection.flags.not_affected_by_inner_occlusion;
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
            if reflection.flags.align_with_screen_center {
                rotation += screen_angle;
            }
            if reflection.flags.radius_scaled_by_occlusion {
                radius *= (chained * 0.5 + 0.5).clamp(0.0, 1.0);
            }
            if reflection.flags.radius_not_scaled_by_distance {
                radius *= depth;
            }
            let tint: [f32; 3] = std::array::from_fn(|channel| {
                let own = color[channel] * reflection.color[channel];
                if !reflection.flags.ignore_light_color { own * external[channel] } else { own }
            });
            sprites.push(FlareSprite {
                center: std::array::from_fn(|axis| {
                    position[axis] + mirror[axis] * reflection.position
                }),
                extent: [radius * stretch[0], radius * stretch[1]],
                rotation,
                color: tint,
                alpha,
                image: reflection.image,
                highlight: Some((1.0 - reflection.modulation).clamp(0.0, 1.0)),
                tint: None,
                depth_tested: reflection.flags.occluded_by_solid_objects,
            });
        }
        sprites
    }
}

attached!(H2PointLight, H2GelLight, H2FlareAt);

#[cfg(test)]
mod tests {
    use super::*;

    /// What every kind of attachment reads for its `scale by`: full with no
    /// name, the object's value, and with none an idle object's input.
    #[test]
    fn an_attachment_scale_reads_the_object_then_its_idle_input() {
        let values = FunctionValues::from([("throttle".to_owned(), 0.25)]);
        assert_eq!(attachment_value("", None), 1.0);
        assert_eq!(attachment_value("throttle", Some(&values)), 0.25);
        assert_eq!(attachment_value("alive", None), 1.0, "nothing to ask: the idle object is alive");
        assert_eq!(attachment_value("boost", None), 0.0);
    }

    #[test]
    fn a_frustum_takes_the_combined_pass_only_with_specular_to_show() {
        let light = |specular| H2DefaultLight { position: [0.0; 3], color: [1.0; 3], specular, radius: 1.0 };
        assert!(has_specular(&light([0.0, 0.5, 0.0])));
        assert!(!has_specular(&light([0.01; 3])), "the game's limit is past one hundredth");
        assert!(!has_specular(&light([0.0; 3])));
    }

    fn view() -> ViewFrame {
        ViewFrame {
            eye: [0.0, 0.0, 0.0],
            forward: [1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }

    fn reflection(flags: ReflectionFlags) -> H2Reflection {
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

    fn flare(flags: FlareFlags, reflections: Vec<H2Reflection>) -> H2FlareAt {
        H2FlareAt {
            object: 0,
            node: 0,
            position: [2.0, 0.0, 0.0],
            forward: [-1.0, 0.0, 0.0],
            marker_scale: 3.0,
            only_under: None,
            scale_by: "power".to_owned(),
            change_color: None,
            light: None,
            effect: None,
            flare: Arc::new(H2Flare {
                flags,
                cones: [1.0, 1.0],
                fade: [0.0, 0.0],
                occlusion_radius: 0.2,
                occlusion_offset: OcclusionOffset::None,
                inner_scale: Some(0.5),
                corona_scale: [2.0, 3.0],
                rotation_function: 0,
                rotation_scale: 0.0,
                reflections,
                brightness: None,
                color: None,
                rotation: None,
                synchronized: true,
                falloff: 0,
                images: Vec::new(),
            }),
        }
    }

    fn values(power: f32) -> FunctionValues {
        FunctionValues::from([("power".to_owned(), power)])
    }

    #[test]
    fn a_function_runs_a_reflection_between_its_bounds() {
        let at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
        let sprites = at.sprites(Some(&values(0.5)), &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [1.0, 1.0]);
        assert!((sprites[0].alpha - 0.75).abs() < 1e-5);
        assert!((sprites[0].extent[0] - 0.4).abs() < 1e-5, "0.2, then the corona's stretch");
        assert_eq!(sprites[0].highlight, Some(0.75));
    }

    #[test]
    fn scale_by_marker_takes_the_upper_bounds_and_the_markers_scale() {
        let at = flare(FlareFlags { scale_by_marker: true, ..Default::default() }, vec![reflection(ReflectionFlags::default())]);
        let sprites = at.sprites(Some(&values(0.0)), &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [1.0, 1.0]);
        assert!((sprites[0].alpha - 1.0).abs() < 1e-5);
        assert!((sprites[0].extent[1] - 2.7).abs() < 1e-5);
    }

    #[test]
    fn only_some_reflections_ignore_inner_occlusion() {
        let kept = ReflectionFlags { not_affected_by_inner_occlusion: true, ..Default::default() };
        let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default()), reflection(kept)]);
        at.scale_by.clear();
        let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [0.0, 0.5]);
        assert_eq!(sprites.len(), 0, "with the chained test all hidden nothing draws");
        let sprites = at.sprites(None, &[[1.0; 3]; 4], None, &view(), 0.0, 0.0, [0.25, 0.5]);
        assert!((sprites[0].alpha - 0.25).abs() < 1e-5);
        assert!((sprites[1].alpha - 0.5).abs() < 1e-5, "the outer test alone");
    }

    #[test]
    fn a_change_color_tints_unless_a_reflection_ignores_it() {
        let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default()), reflection(ReflectionFlags { ignore_light_color: true, ..Default::default() })]);
        at.scale_by.clear();
        at.change_color = Some(1);
        let colors = [[1.0; 3], [0.5, 0.5, 0.5], [1.0; 3], [1.0; 3]];
        let sprites = at.sprites(None, &colors, None, &view(), 0.0, 0.0, [1.0, 1.0]);
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
            diffuse_layer: H2Layer::LightDiffuse,
            radius: 2.0,
            specular_radius: 3.0,
            effect_duration: 0.0,
            effect_falloff: 0,
        }
    }

    #[test]
    fn a_name_nothing_answers_to_is_zero_and_no_name_is_one() {
        let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
        at.scale_by = "no_such_function".to_owned();
        assert_eq!(at.value(None, 0.0, 0.0), Some(0.0));
        assert_eq!(at.value(Some(&values(0.5)), 0.0, 0.0), Some(0.0));
        at.scale_by.clear();
        assert_eq!(at.value(None, 0.0, 0.0), Some(1.0));
    }

    #[test]
    fn a_light_tints_its_flare_by_its_own_color_and_no_change_color() {
        let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
        at.light = Some(light());
        at.change_color = Some(0);
        let colors = [[0.0; 3]; 4];
        let sprites = at.sprites(Some(&values(0.5)), &colors, None, &view(), 0.0, 0.0, [1.0, 1.0]);
        assert_eq!(sprites[0].color, [0.5, 0.125, 0.03125]);
    }

    #[test]
    fn a_light_with_no_color_or_size_is_out() {
        let mut at = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
        at.light = Some(light());
        let none = [[1.0; 3]; 4];
        assert!(at.sprites(Some(&values(0.0)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]).is_empty());
        at.light = Some(H2Light { size: [0.0, 0.0], ..light() });
        assert!(at.sprites(Some(&values(1.0)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]).is_empty());
    }

    #[test]
    fn a_light_ignores_its_markers_scale() {
        let mut at = flare(FlareFlags { scale_by_marker: true, ..Default::default() }, vec![reflection(ReflectionFlags::default())]);
        at.light = Some(light());
        let none = [[1.0; 3]; 4];
        let sprites = at.sprites(Some(&values(0.5)), &none, None, &view(), 0.0, 0.0, [1.0, 1.0]);
        assert!((sprites[0].extent[1] - 0.9).abs() < 1e-5, "0.3 by the corona's 3");
    }

    fn point(light: H2Light) -> H2PointLight {
        H2PointLight {
            object: 0,
            node: 0,
            position: [1.0, 2.0, 3.0],
            scale_by: "power".to_owned(),
            light,
            effect: None,
            only_under: None,
        }
    }

    #[test]
    fn an_effects_light_dims_over_its_duration_and_is_gone_after_it() {
        let times = Arc::new(EffectTimes::read_nothing());
        let effect = H2EffectLight::new(times, 0, false);
        let lasting = H2Light { effect_duration: 0.5, ..light() };
        assert_eq!(effect.value(&lasting, 1.0, 0.0, 0.0), Some(1.0));
        let half = effect.value(&lasting, 1.0, 0.25, 0.0).unwrap();
        assert!((half - 0.5).abs() < 0.01, "linear falloff halfway: {half}");
        assert_eq!(effect.value(&lasting, 1.0, 0.6, 0.0), None);
        assert_eq!(effect.value(&lasting, 1.0, 0.5, 0.5), Some(1.0), "runs from its onset");
        assert_eq!(effect.value(&lasting, 0.0, 0.1, 0.0), None, "nothing while its function is off");
        assert_eq!(effect.value(&light(), 1.0, 50.0, 0.0), Some(1.0), "no duration lasts");
        let scaled = H2EffectLight::new(Arc::new(EffectTimes::read_nothing()), 0, true);
        assert_eq!(scaled.value(&light(), 0.25, 1.0, 0.0), Some(0.25));
    }

    #[test]
    fn a_light_doubles_its_brightness_and_reaches_its_larger_radius() {
        let lit = point(light()).lit(Some(&values(1.0)), None, 0.0, 0.0).unwrap();
        assert_eq!(lit.position, [1.0, 2.0, 3.0]);
        assert_eq!(lit.color, [2.0, 1.0, 0.5]);
        assert_eq!(lit.radius, 3.0);
    }

    #[test]
    fn a_bright_light_stops_at_four() {
        let bright = H2Light { brightness: [3.0, 3.0], ..light() };
        let lit = point(bright).lit(Some(&values(1.0)), None, 0.0, 0.0).unwrap();
        assert_eq!(lit.color, [4.0, 2.0, 1.0]);
    }

    #[test]
    fn a_light_with_no_function_value_is_out_and_no_name_is_on() {
        assert!(point(light()).lit(None, None, 0.0, 0.0).is_some(), "`power` rests at 1");
        let mut out = point(light());
        out.scale_by = "no_such_function".to_owned();
        assert!(out.lit(None, None, 0.0, 0.0).is_none());
        out.scale_by.clear();
        assert!(out.lit(None, None, 0.0, 0.0).is_some());
    }

    #[test]
    fn a_bright_light_is_scaled_down_and_drawn_again() {
        let light = |color| H2DefaultLight { position: [0.0; 3], color, specular: [0.0; 3], radius: 1.0 };
        let (dim, draws) = overbrighten(&light([0.5, 1.0, 0.25]));
        assert_eq!((dim.color, draws), ([0.5, 1.0, 0.25], 1));
        let (dim, draws) = overbrighten(&light([2.0, 1.0, 0.5]));
        assert_eq!((dim.color, draws), ([1.0, 0.5, 0.25], 2));
        let (dim, draws) = overbrighten(&light([4.0, 2.0, 1.0]));
        assert_eq!((dim.color, draws), ([1.0, 0.5, 0.25], 4));
        let brighter_specular = H2DefaultLight { specular: [3.0; 3], ..light([1.0; 3]) };
        assert_eq!(overbrighten(&brighter_specular).1, 4, "the brighter of the two decides");
    }


    #[test]
    fn a_flare_with_no_test_has_no_patches() {
        let tested = flare(FlareFlags::default(), vec![reflection(ReflectionFlags::default())]);
        let (outer, inner) = tested.occlusion_patches(None, &view()).unwrap();
        assert_eq!((outer.extent[0], inner.unwrap().extent[0]), (0.2, 0.1));
        assert!(flare(FlareFlags { no_occlusion_test: true, ..Default::default() }, Vec::new()).occlusion_patches(None, &view()).is_none());
    }

    fn gel_frustum() -> H2Frustum {
        H2Frustum {
            near_width: 0.6,
            stretch: 0.8,
            half_view: 65f32.to_radians() * 0.5,
            falloff: 3.0,
            cutoff: 6.0,
            gel: None,
        }
    }

    /// What `lightaccum_frustum_no_specular_gel` gives a point: gel u, v,
    /// falloff and near plane colors, before they clamp.
    fn gel_at(rows: &H2GelRows, point: [f32; 3]) -> [f32; 4] {
        let to_light: [f32; 3] = std::array::from_fn(|axis| rows.apex[axis] - point[axis]);
        let row = |index: usize| dot(to_light, std::array::from_fn(|axis| rows.rows[index][axis]));
        let w = row(2);
        [
            (row(0) + rows.rows[0][3]) / w,
            (row(1) + rows.rows[1][3]) / w,
            row(2) + rows.rows[2][3],
            row(3) + rows.rows[3][3],
        ]
    }

    #[test]
    fn a_frustum_projects_its_gel_from_a_point_behind_the_near_plane() {
        let frustum = gel_frustum();
        let (forward, up) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let rows = frustum.rows([2.0, 0.0, 0.0], forward, up, 1.0);
        let behind = 0.3 / frustum.half_view.tan();
        assert!((rows.apex[0] - (2.0 - behind)).abs() < 1e-5);
        let ahead = |distance: f32| [2.0 - behind + distance, 0.0, 0.0];
        // The axis lands in the middle of the gel, at every distance.
        for distance in [1.0, 4.0] {
            let [u, v, ..] = gel_at(&rows, ahead(distance));
            assert!((u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5);
        }
        // The frustum's edges land on the gel's. `sub_8E84F0` crosses forward
        // into up, so u climbs toward +Y on a +X, +Z light.
        let tan_x = frustum.half_view.tan();
        let left = [2.0 - behind + 4.0, 4.0 * tan_x, 0.0];
        let right = [2.0 - behind + 4.0, -4.0 * tan_x, 0.0];
        assert!((gel_at(&rows, left)[0] - 1.0).abs() < 1e-4);
        assert!(gel_at(&rows, right)[0].abs() < 1e-4);
        let top = [2.0 - behind + 4.0, 0.0, 4.0 * tan_x * frustum.stretch];
        assert!((gel_at(&rows, top)[1] - 1.0).abs() < 1e-4);
        // Dark from the far end, full up to the falloff distance.
        assert!(gel_at(&rows, ahead(behind + 3.0))[2].abs() < 1e-4);
        assert!((gel_at(&rows, ahead(behind + 6.0))[2] - 1.0).abs() < 1e-4);
        // The near edge takes a surface past it and leaves one behind it.
        assert!(gel_at(&rows, ahead(behind + 0.1))[3] > 0.5);
        assert!(gel_at(&rows, ahead(behind - 0.1))[3] < 0.5);
    }

    /// `sub_515280`: a part fades out over its event, and takes the A and B
    /// scales only where its flags ask; with its effect off it is not drawn.
    #[test]
    fn an_effect_part_fades_over_its_event_and_takes_the_scales_it_asks_for() {
        let part = |scaled: [bool; 2]| H2EffectPart {
            times: Arc::new(EffectTimes::read_nothing()),
            event: 0,
            scaled,
            secondary_by: "power".to_owned(),
        };
        let values: FunctionValues = [("power".to_owned(), 0.5), ("on".to_owned(), 0.8)].into_iter().collect();
        let lit = |scaled, time| part(scaled).light("on", Some(&values), time, 0.0);
        assert_eq!(lit([false; 2], 0.0), Some(H2PartLight { brightness: 1.0, fade: 1.0, lasts: true }));
        assert_eq!(lit([true, false], 50.0).map(|lit| (lit.brightness, lit.fade)), Some((0.8, 0.5)));
        assert_eq!(lit([true, true], 0.0).map(|lit| lit.brightness), Some(0.4));
        let off: FunctionValues = [("on".to_owned(), 0.0)].into_iter().collect();
        assert_eq!(part([false; 2]).light("on", Some(&off), 0.0, 0.0), None, "its effect is off");
    }
}
