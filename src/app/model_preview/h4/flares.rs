//! Halo 4's lens flares on objects: each `lens_flare` an object attaches,
//! tested against the depth by a world-space patch (`lens_flare_render_occlusion`)
//! and then drawn one screen-facing quad per reflection through
//! `rasterizer_globals`' explicit shader 46 `lens_flare` (`lens_flare_render_internal`,
//! X360 0x83AE8330, MCC 0x1408CF390). See `docs/h4-preview-research/lights.md` §4.

use std::f32::consts::PI;
use std::sync::Arc;

use blam_tags::tag_function::transition_function_evaluate;
use blam_tags::{TagFile, TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::common::fields::{elements, has_flag};
use super::super::h3::lights::{OcclusionOffset, rotation_function};
use super::super::ce::lights::posed_on;
use super::super::functions::FunctionValues;
use super::super::dx9::passes::{H2Image, tag_function_at};
use super::super::gen3::effects::part_markers;
use super::super::materials::{H3Bitmap, read_h3_bitmap};
use super::super::reach::lights::object_value;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, dot, unit_unless_under_micro};
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::common::attached::{ObjectAttachment, object_attachments};

const SMALLEST: f32 = 1e-4;
/// `lens_flares_render`'s least visible share that draws.
const MINIMUM_VISIBLE: f32 = 0.05;
/// The fewest samples `lens_flares_render` counts a patch as, seen or expected.
const FEWEST_SAMPLES: f32 = 10.0;
/// Width plus height, in pixels, past which a patch tests its inner share alone.
const OCCLUSION_PIXELS: f32 = 600.0;

/// The `lens_flare` `flags` the preview reads.
#[derive(Debug, Clone, Copy)]
struct H4FlareFlags {
    rotate_occlusion_box: bool,
    no_occlusion_test: bool,
    first_person_only: bool,
    simple_occlusion_box: bool,
    no_opacity_feedback: bool,
}

impl H4FlareFlags {
    fn read(root: TagStruct<'_>) -> Self {
        let flagged = |wanted: &str| has_flag(root, "flags", wanted);
        Self {
            rotate_occlusion_box: flagged("rotate occlusion testing box with lens flare"),
            no_occlusion_test: flagged("no occlusion test"),
            first_person_only: flagged("only render in first person"),
            simple_occlusion_box: flagged("use simple occlusion box test"),
            no_opacity_feedback: flagged("no reflection opacity feedback"),
        }
    }
}

/// A reflection's `flags` the preview reads.
#[derive(Debug, Clone, Copy)]
struct H4ReflectionFlags {
    rotate_from_center: bool,
    radius_by_distance: bool,
    radius_by_occlusion: bool,
    ignore_external_color: bool,
    lock_x: bool,
    lock_y: bool,
    proximity_input: bool,
    mirror: bool,
    flip_u: bool,
    flip_v: bool,
}

impl H4ReflectionFlags {
    fn read(reflection: TagStruct<'_>) -> Self {
        let flagged = |wanted: &str| has_flag(reflection, "flags", wanted);
        Self {
            rotate_from_center: flagged("rotate from center of screen"),
            radius_by_distance: flagged("radius scaled by distance"),
            radius_by_occlusion: flagged("radius scaled by occlusion factor"),
            ignore_external_color: flagged("ignore external color"),
            lock_x: flagged("lock to flare X"),
            lock_y: flagged("lock to flare Y"),
            proximity_input: flagged("proximity to center as function input"),
            mirror: flagged("mirror across flare"),
            flip_u: flagged("flip U coordinate"),
            flip_v: flagged("flip V coordinate"),
        }
    }
}

/// A curve driven by game time or the flare's age, with the object function
/// that drives it instead when it names one (`s_editing_function`).
#[derive(Debug, Clone)]
struct H4FlareCurve {
    input: String,
    function: Arc<TagFunction>,
}

impl H4FlareCurve {
    fn read(root: TagStruct<'_>, block: &str) -> Option<Self> {
        let first = elements(root, block).into_iter().next()?;
        Some(Self {
            input: first
                .read_string_id("input name")
                .or_else(|| first.read_string_id("Input Variable"))
                .unwrap_or_default(),
            function: Arc::new(tag_function_at(&first, "function")?),
        })
    }
}

/// One of a flare's `reflections` (148 bytes at run time).
#[derive(Debug, Clone)]
struct H4Reflection {
    flags: H4ReflectionFlags,
    /// Into the flare's images.
    image: usize,
    /// `lens_flare_get_bitmap_aspect_ratio`.
    aspect: [f32; 2],
    /// Degrees.
    rotation_offset: f32,
    axis_offset: f32,
    offset_bounds: [f32; 2],
    radius: Arc<TagFunction>,
    scale: [Arc<TagFunction>; 2],
    brightness: Arc<TagFunction>,
    color: H4ReflectionColor,
    modulation: f32,
    tint_power: f32,
}

#[derive(Debug, Clone)]
enum H4ReflectionColor {
    Curve(Arc<TagFunction>),
    /// A Reach-era tag's constant, where it has no curve.
    Constant([f32; 3]),
}

/// A `lens_flare` tag, by field name: the kit holds two reflection layouts.
#[derive(Debug, Clone)]
pub(crate) struct H4LensFlare {
    flags: H4FlareFlags,
    /// Radians.
    falloff: f32,
    cutoff: f32,
    near_fade: [f32; 2],
    /// `near fade distance`, `far fade distance`.
    far_fade: [f32; 2],
    occlusion_reflection: usize,
    occlusion_offset: f32,
    offset_direction: OcclusionOffset,
    inner_scale: f32,
    falloff_function: u8,
    rotation_function: i128,
    rotation_scale: f32,
    time_brightness: Option<Arc<TagFunction>>,
    age_brightness: Option<Arc<TagFunction>>,
    time_rotation: Option<Arc<TagFunction>>,
    age_rotation: Option<Arc<TagFunction>>,
    time_color: Option<H4FlareCurve>,
    age_color: Option<H4FlareCurve>,
    reflections: Vec<H4Reflection>,
    /// Each reflection's bitmap image, by `image`.
    pub(crate) images: Vec<H3Bitmap>,
}

fn aspect_of(image: &H2Image) -> [f32; 2] {
    let (width, height) = match image {
        H2Image::Flat(image) => (image.width as f32, image.height as f32),
        _ => (1.0, 1.0),
    };
    if width > height {
        [1.0, (width / height).floor()]
    } else if height > width {
        [(height / width).floor(), 1.0]
    } else {
        [1.0, 1.0]
    }
}

fn block_function(root: TagStruct<'_>, block: &str) -> Option<Arc<TagFunction>> {
    let first = elements(root, block).into_iter().next()?;
    tag_function_at(&first, "function").map(Arc::new)
}

pub(crate) fn read_h4_lens_flare(source: &TagSource, path: &str) -> Option<H4LensFlare> {
    let tag = load_referenced_tag_from_source(source, path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let flags = H4FlareFlags::read(root);
    let bitmap = root
        .read_tag_ref_path("bitmap")
        .filter(|path| !path.is_empty());
    let mut images: Vec<(String, i16, H3Bitmap)> = Vec::new();
    let mut reflections = Vec::new();
    for reflection in elements(root, "reflections") {
        let curve = |name: &str| {
            reflection
                .descend(name)
                .and_then(|holder| tag_function_at(&holder, "function"))
                .map(Arc::new)
        };
        let override_map = reflection
            .read_tag_ref_path("bitmap override")
            .filter(|path| !path.is_empty());
        let Some(path) = override_map.or_else(|| bitmap.clone()) else {
            continue;
        };
        let index = reflection.read_int_any("bitmap index").unwrap_or(0) as i16;
        let image = match images
            .iter()
            .position(|(known, at, _)| *known == path && *at == index)
        {
            Some(image) => image,
            None => {
                let Some(read) = read_h3_bitmap(source, &path, index, SamplerKind::Texture2D)
                else {
                    continue;
                };
                images.push((path, index, read));
                images.len() - 1
            }
        };
        let (Some(radius), Some(scale_x), Some(scale_y), Some(brightness)) = (
            curve("radius curve"),
            curve("scale curve X"),
            curve("scale curve Y"),
            curve("brightness curve"),
        ) else {
            continue;
        };
        let color = match curve("color curve") {
            Some(curve) => H4ReflectionColor::Curve(curve),
            None => {
                let color = reflection.read_rgb("color");
                H4ReflectionColor::Constant([color.red, color.green, color.blue])
            }
        };
        let bounds = reflection.read_real_bounds("offset bounds");
        reflections.push(H4Reflection {
            flags: H4ReflectionFlags::read(reflection),
            image,
            aspect: aspect_of(&images[image].2.0),
            rotation_offset: reflection.read_real("rotation offset").unwrap_or(0.0),
            axis_offset: reflection.read_real("axis offset").unwrap_or(0.0),
            offset_bounds: [bounds.lower, bounds.upper],
            radius,
            scale: [scale_x, scale_y],
            brightness,
            color,
            modulation: reflection.read_real("modulation factor").unwrap_or(0.0),
            tint_power: reflection.read_real("tint power").unwrap_or(0.0),
        });
    }
    if reflections.is_empty() {
        return None;
    }
    Some(H4LensFlare {
        flags,
        falloff: real("falloff angle"),
        cutoff: real("cutoff angle"),
        near_fade: [
            real("near fade begin distance"),
            real("near fade end distance"),
        ],
        far_fade: [real("near fade distance"), real("far fade distance")],
        occlusion_reflection: root
            .read_int_any("occlusion reflection index")
            .unwrap_or(0)
            .max(0) as usize,
        occlusion_offset: real("occlusion offset distance"),
        offset_direction: OcclusionOffset::read(root),
        inner_scale: real("occlusion inner radius scale"),
        falloff_function: root
            .read_int_any("falloff function")
            .and_then(|at| u8::try_from(at).ok())
            .unwrap_or(0),
        rotation_function: rotation_function(root),
        rotation_scale: real("rotation function scale"),
        time_brightness: block_function(root, "time brightness"),
        age_brightness: block_function(root, "age brightness"),
        time_rotation: block_function(root, "time rotation"),
        age_rotation: block_function(root, "age rotation"),
        time_color: H4FlareCurve::read(root, "time color"),
        age_color: H4FlareCurve::read(root, "age color"),
        reflections,
        images: images.into_iter().map(|(_, _, image)| image).collect(),
    })
}

/// A flare at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H4FlareAt {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    change_color: Option<usize>,
    pub(crate) flare: Arc<H4LensFlare>,
}

/// The camera, as the flares read it.
pub(crate) struct H4FlareView<'a> {
    pub(crate) eye: [f32; 3],
    pub(crate) forward: [f32; 3],
    pub(crate) right: [f32; 3],
    pub(crate) up: [f32; 3],
    pub(crate) viewport: [f32; 2],
    /// A point's normalized device x and y, and the projection's x and y
    /// scale there (`P[0][0]`, `P[1][1]` for a point at its depth), or none
    /// behind the eye.
    pub(crate) project: &'a dyn Fn([f32; 3]) -> Option<([f32; 2], [f32; 2])>,
}

/// `lens_flare_render_occlusion`'s world quad, and how many samples it would
/// pass seen whole.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H4OcclusionPatch {
    pub(crate) corners: [[f32; 3]; 4],
    pub(crate) expected: f32,
}

/// One reflection's draw: explicit 46's constants.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H4FlareDraw {
    pub(crate) image: usize,
    /// `centerRotation`, `flareScale`, `flareOrigin_offsetBounds`,
    /// `transformedAxes`.
    pub(crate) center_rotation: [f32; 4],
    pub(crate) scale: [f32; 4],
    pub(crate) origin_bounds: [f32; 4],
    pub(crate) axes: [f32; 4],
    pub(crate) mirror: bool,
    /// `modulationFactor`, `tintColor`.
    pub(crate) modulation: [f32; 4],
    pub(crate) tint: [f32; 4],
    pub(crate) flip: [bool; 2],
}


/// `lens_flares_evaluate_corona_rotation_function` (X360 0x83AEA388).
fn corona_rotation(
    function: i128,
    scale: f32,
    position: [f32; 3],
    forward: [f32; 3],
    view: &H4FlareView<'_>,
) -> f32 {
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
    if x == 0.0 {
        0.0
    } else {
        x.atan2(y) * scale / PI
    }
}



/// `LensFlareEvaluateParameters`' result (`s_lens_flare_render_parameters`).
#[derive(Debug, Clone, PartialEq)]
struct H4FlareParameters {
    brightness: f32,
    rotation: f32,
    radius: f32,
    scale: [f32; 2],
    position: [f32; 3],
    color: [f32; 3],
}

impl H4FlareAt {
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        let flare = &self.flare;
        let curves = [&flare.time_color, &flare.age_color]
            .into_iter()
            .flatten()
            .map(|curve| curve.input.as_str());
        [self.scale_by.as_str(), self.secondary_by.as_str()]
            .into_iter()
            .chain(curves)
    }

    pub(crate) fn animates(&self) -> bool {
        let flare = &self.flare;
        let moving = |function: &Option<Arc<TagFunction>>| {
            function
                .as_ref()
                .is_some_and(|function| function.as_constant().is_none())
        };
        moving(&flare.time_brightness)
            || moving(&flare.time_rotation)
            || flare.time_color.as_ref().is_some_and(|curve| {
                curve.input.is_empty() && curve.function.as_constant().is_none()
            })
    }

    /// `render_object_attachments`' `lens` and `lens_flares_create_internal`
    /// (X360 0x83AE7FC8): the scale both attachment functions give it, and
    /// the tint, brought down to at most 1 with the excess taken into the
    /// scale. None while the scale is off.
    fn created(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
    ) -> Option<(f32, [f32; 3])> {
        let mut scale =
            object_value(values, &self.scale_by) * object_value(values, &self.secondary_by);
        if scale <= 0.0 {
            return None;
        }
        let mut tint = self
            .change_color
            .and_then(|channel| change_colors.get(channel))
            .copied()
            .unwrap_or([1.0; 3]);
        let most = tint.iter().copied().fold(0.0, f32::max);
        if most > 1.0 {
            tint = tint.map(|channel| channel / most);
            scale *= most;
        }
        // `pixel32`: the tint is stored as bytes.
        Some((
            scale,
            tint.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() / 255.0),
        ))
    }

    /// `LensFlareEvaluateParameters` (X360 0x83AE9730) for `reflection`, at
    /// visibility `opacity`, the flare's centre at `center` on screen.
    #[allow(clippy::too_many_arguments)]
    fn parameters(
        &self,
        reflection: &H4Reflection,
        scale: f32,
        tint: [f32; 3],
        position: [f32; 3],
        forward: [f32; 3],
        center: [f32; 2],
        opacity: f32,
        values: Option<&FunctionValues>,
        view: &H4FlareView<'_>,
        time: f32,
    ) -> Option<H4FlareParameters> {
        let flare = &*self.flare;
        let to_flare: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
        let depth = dot(view.forward, to_flare);
        let to_center: [f32; 3] =
            std::array::from_fn(|axis| 2.0 * (view.forward[axis] * depth - to_flare[axis]));
        let to_eye = unit_unless_under_micro(to_flare.map(|axis| -axis));
        let mut brightness = 1.0;
        let [near, far] = flare.far_fade;
        if near != far {
            brightness *= ((depth - far) / (near - far)).clamp(0.0, 1.0);
        }
        let [begin, end] = flare.near_fade;
        if begin != end && depth < end {
            brightness *= ((depth - begin) / (end - begin)).clamp(0.0, 1.0);
        }
        if flare.cutoff > SMALLEST {
            let angle = dot(forward, to_eye).clamp(-1.0, 1.0).acos();
            if angle > flare.cutoff {
                brightness = 0.0;
            } else if flare.cutoff > flare.falloff {
                brightness *= 1.0
                    - ((angle - flare.falloff) / (flare.cutoff - flare.falloff)).clamp(0.0, 1.0);
            }
        }
        brightness *= opacity;
        // `CurrentDissolveAmount`: game time. An unsynchronized flare's own
        // offset (a checksum of its slot) is not reproduced.
        let clock = time;
        let age = 0.0;
        if let Some(function) = &flare.time_brightness {
            brightness *= function.evaluate(clock, 0.0);
        }
        if let Some(function) = &flare.age_brightness {
            brightness *= function.evaluate(age, 0.0);
        }
        if brightness <= 0.0 {
            return None;
        }
        let mut corona = corona_rotation(
            flare.rotation_function,
            flare.rotation_scale,
            position,
            forward,
            view,
        );
        let left = view.right.map(|axis| -axis);
        // [inferred, as Halo 3's: the projection's +56, +68 and +80 are its
        // forward, left and up.]
        let screen_angle = dot(to_flare, view.forward).atan2(dot(to_flare, left));
        let mut tint3 = [1.0; 3];
        for (curve, input) in [(&flare.time_color, clock), (&flare.age_color, age)] {
            if let Some(curve) = curve {
                let mut at = curve.function.evaluate(input, 0.0);
                if !curve.input.is_empty() {
                    at = object_value(values, &curve.input);
                }
                let color = curve.function.evaluate_color(at, 1.0);
                tint3 = [
                    tint3[0] * color.red,
                    tint3[1] * color.green,
                    tint3[2] * color.blue,
                ];
            }
        }
        if let Some(function) = &flare.time_rotation {
            corona += PI * function.evaluate(clock, 0.0);
        }
        if let Some(function) = &flare.age_rotation {
            corona += PI * function.evaluate(age, 0.0);
        }
        let mut input = scale;
        if reflection.flags.proximity_input {
            let from_center = (center[0] * center[0] + center[1] * center[1]).sqrt();
            input *= ((std::f32::consts::SQRT_2 - from_center) / std::f32::consts::SQRT_2)
                .clamp(0.0, 1.0);
        }
        let reflected = reflection.brightness.evaluate(input, 0.0) * brightness;
        // The marker matrix's scale: the object's, 1.
        let mut radius = reflection.radius.evaluate(input, 0.0);
        let mut color = match &reflection.color {
            H4ReflectionColor::Curve(curve) => {
                let color = curve.evaluate_color(input, 0.0);
                [color.red, color.green, color.blue]
            }
            H4ReflectionColor::Constant(color) => *color,
        };
        let scale = [
            reflection.scale[0].evaluate(input, 0.0),
            reflection.scale[1].evaluate(input, 0.0),
        ];
        if reflected <= 0.0 || radius <= 0.0 {
            return None;
        }
        let mut rotation = reflection.rotation_offset.to_radians() + corona;
        color = std::array::from_fn(|channel| color[channel] * tint3[channel]);
        if !reflection.flags.ignore_external_color {
            color = std::array::from_fn(|channel| color[channel] * tint[channel]);
        }
        if reflection.flags.rotate_from_center {
            rotation += screen_angle;
        }
        if reflection.flags.radius_by_occlusion {
            radius *= (opacity * 0.5 + 0.5).clamp(0.0, 1.0);
        }
        if reflection.flags.radius_by_distance {
            radius *= depth;
        }
        Some(H4FlareParameters {
            brightness: reflected,
            rotation,
            radius,
            scale,
            position: std::array::from_fn(|axis| {
                position[axis] + to_center[axis] * reflection.axis_offset
            }),
            color,
        })
    }

    fn placed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        (position, unit_unless_under_micro(forward))
    }

    /// `lens_flare_render_occlusion` (X360 0x83AE82E0): the occlusion
    /// reflection's quad facing the eye, sized by the inner share, pushed
    /// along the offset direction; none for a flare that tests nothing or
    /// shows nothing to test.
    pub(crate) fn occlusion_patch(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &H4FlareView<'_>,
        time: f32,
    ) -> Option<H4OcclusionPatch> {
        let flare = &*self.flare;
        if flare.flags.no_occlusion_test || flare.flags.first_person_only {
            return None;
        }
        let (scale, tint) = self.created(values, change_colors)?;
        let (position, forward) = self.placed(bones);
        let (center, _) = (view.project)(position)?;
        let depth = dot(
            view.forward,
            std::array::from_fn(|axis| position[axis] - view.eye[axis]),
        );
        let opacity = transition_function_evaluate(flare.falloff_function, 1.0);
        // Past the last, the last.
        let reflection = flare.reflections.get(flare.occlusion_reflection).or(flare.reflections.last())?;
        let parameters = self.parameters(
            reflection, scale, tint, position, forward, center, opacity, values, view, time,
        )?;
        // `LensFlareProjectBillboard` at radius 1: the half size in pixels,
        // before the projection's own scale.
        let half = [
            0.5 * view.viewport[0] / depth,
            0.5 * view.viewport[1] / depth,
        ];
        let across = ((2.0 * half[0] + 2.0 * half[1]) / OCCLUSION_PIXELS).clamp(0.0, 1.0);
        let share = if flare.flags.simple_occlusion_box {
            flare.inner_scale
        } else {
            flare.inner_scale + (1.0 - flare.inner_scale) * (1.0 - across)
        };
        let radius = parameters.radius * share;
        let half = [
            (half[0] * radius).min(view.viewport[0] / 2.0),
            (half[1] * radius).min(view.viewport[1] / 2.0),
        ];
        let expected = (2.5 * 2.0 * half[0] * 2.0 * half[1]).max(1.0);
        // The quad's own frame: facing the eye, upright on the screen, turned
        // by the flare's rotation when it asks.
        let (sine, cosine) = if flare.flags.rotate_occlusion_box {
            parameters.rotation.sin_cos()
        } else {
            (0.0, 1.0)
        };
        let left = view.right.map(|axis| -axis);
        let toward_viewer = unit_unless_under_micro(std::array::from_fn(|axis| view.eye[axis] - position[axis]));
        let offset = match flare.offset_direction {
            OcclusionOffset::TowardViewer => toward_viewer,
            OcclusionOffset::MarkerForward => forward,
            OcclusionOffset::None => [0.0; 3],
        };
        let corners =
            [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]].map(|[x, y]: [f32; 2]| {
                let (x, y) = (x * radius, y * radius);
                let (x, y) = (x * cosine - y * sine, x * sine + y * cosine);
                std::array::from_fn(|axis| {
                    position[axis]
                        + left[axis] * x
                        + view.up[axis] * y
                        + offset[axis] * flare.occlusion_offset
                })
            });
        Some(H4OcclusionPatch { corners, expected })
    }

    /// `lens_flares_render`' share of the patch seen, from the samples that
    /// passed; 1 for a flare that tests nothing.
    pub(crate) fn visibility(&self, passed: Option<(f32, f32)>) -> f32 {
        if self.flare.flags.no_occlusion_test {
            return 1.0;
        }
        let Some((passed, expected)) = passed else {
            return 0.0;
        };
        let passed = if passed > 0.0 && passed < FEWEST_SAMPLES {
            FEWEST_SAMPLES
        } else {
            passed
        };
        passed / expected.max(FEWEST_SAMPLES)
    }

    /// `lens_flare_render_internal`'s reflections at `visibility`, none below
    /// the least visible share.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draws(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &H4FlareView<'_>,
        time: f32,
        visibility: f32,
    ) -> Vec<H4FlareDraw> {
        let flare = &*self.flare;
        if visibility <= MINIMUM_VISIBLE || flare.flags.first_person_only {
            return Vec::new();
        }
        let Some((scale, tint)) = self.created(values, change_colors) else {
            return Vec::new();
        };
        let (position, forward) = self.placed(bones);
        let mut draws = Vec::new();
        let mut opacity = visibility;
        for reflection in &flare.reflections {
            // Each reflection eases the last one's opacity, unless the flare
            // turns that feedback off.
            let from = if flare.flags.no_opacity_feedback {
                visibility
            } else {
                opacity
            };
            opacity = transition_function_evaluate(flare.falloff_function, from);
            if opacity <= SMALLEST {
                continue;
            }
            let Some((center, _)) = (view.project)(position) else {
                continue;
            };
            let Some(parameters) = self.parameters(
                reflection, scale, tint, position, forward, center, opacity, values, view, time,
            ) else {
                continue;
            };
            let Some((at, focal)) = (view.project)(parameters.position) else {
                continue;
            };
            let depth = dot(
                view.forward,
                std::array::from_fn(|axis| parameters.position[axis] - view.eye[axis]),
            );
            let size = [
                reflection.aspect[0] * parameters.scale[0] * parameters.radius / depth,
                reflection.aspect[1] * parameters.scale[1] * parameters.radius / depth,
            ];
            let x = if reflection.flags.lock_x {
                center[0]
            } else {
                at[0]
            };
            let y = if reflection.flags.lock_y {
                center[1]
            } else {
                at[1]
            };
            let [lower, upper] = reflection.offset_bounds;
            let [r, g, b] = parameters.color;
            draws.push(H4FlareDraw {
                image: reflection.image,
                center_rotation: [x, y, parameters.rotation, 0.0],
                scale: [size[0], size[1], focal[0], focal[1]],
                origin_bounds: [center[0], center[1], lower, upper],
                // World-space axes (flag 0x800) are not read.
                axes: [1.0, 0.0, 0.0, 1.0],
                mirror: reflection.flags.mirror,
                modulation: [
                    1.0 - reflection.modulation,
                    reflection.tint_power,
                    parameters.brightness,
                    1.0 - reflection.modulation,
                ],
                tint: [r, g, b, 1.0],
                flip: [
                    reflection.flags.flip_u,
                    reflection.flags.flip_v,
                ],
            });
        }
        draws
    }
}

/// An object's attached `lens_flare`s, at every marker of each name; none
/// that draw only in first person.
pub(crate) fn read_h4_flares(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H4FlareAt> {
    let mut flares = Vec::new();
    let lens_group = u32::from_be_bytes(*b"lens");
    let mut read: Vec<(String, Option<Arc<H4LensFlare>>)> = Vec::new();
    for attachment in object_attachments(object, &[lens_group]) {
        let ObjectAttachment { path, marker: name, scale_by, secondary_by, change_color, .. } = attachment;
        let flare = match read.iter().find(|(known, _)| *known == path) {
            Some((_, flare)) => flare.clone(),
            None => {
                let flare = read_h4_lens_flare(source, &path).map(Arc::new);
                read.push((path.clone(), flare.clone()));
                flare
            }
        };
        let Some(flare) = flare.filter(|flare| !flare.flags.first_person_only) else {
            continue;
        };
        for marker in part_markers(markers, &name, false) {
            let (place, node) = frame(&marker);
            flares.push(H4FlareAt {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                only_under: marker.only_under.clone(),
                scale_by: scale_by.clone(),
                secondary_by: secondary_by.clone(),
                change_color,
                flare: Arc::clone(&flare),
            });
        }
    }
    flares
}

/// `rasterizer_globals`' explicit shader 46, `lens_flare`, whose one
/// technique's `screen` pass draws every reflection.
const LENS_FLARE: usize = 46;

pub(crate) fn h4_flare_pass(source: &TagSource) -> Result<super::super::gen3::shaders::H3Pass, String> {
    use crate::core::hlsl::emit::Stage;
    let globals = load_referenced_tag_from_source(
        source,
        "globals\\rasterizer_globals",
        "rasterizer_globals",
        b"rasg",
    )
    .map_err(|error| error.to_string())?;
    let explicit = elements(globals.root(), "explicit shaders");
    let entry = explicit.get(LENS_FLARE).ok_or("no explicit shader 46")?;
    let path = |field: &str| {
        entry
            .read_tag_ref_path(field)
            .filter(|path| !path.is_empty())
            .ok_or(format!("no {field}"))
    };
    let program = |path: &str| -> Result<_, String> {
        super::shaders::H4ShaderSource::read(source, path)?
            .program(&super::shaders::H4_DEFINES)
    };
    let vertex = super::shaders::translate_technique(
        &program(&path("explicit vertex shader")?)?,
        0,
        "screen",
        Stage::Vertex,
    )?;
    let pixel = super::shaders::translate_technique(
        &program(&path("explicit pixel shader")?)?,
        0,
        "screen",
        Stage::Pixel,
    )?;
    Ok(super::super::gen3::shaders::pass_of(vertex, pixel))
}

attached!(H4FlareAt);

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> Option<TagSource> {
        let root = std::path::PathBuf::from(crate::core::test_kits::tag_path("halo4_mcc", ""));
        if !root.join("objects").is_dir() {
            eprintln!("skipped: set BLAM_TEST_H4EK to a Halo 4 kit's tags folder");
            return None;
        }
        Some(TagSource::LooseFolder {
            root,
            game: Some(crate::core::game::GameId::Halo4),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        })
    }

    fn sniper_eye(source: &TagSource) -> H4FlareAt {
        let flare = read_h4_lens_flare(source, "objects\\characters\\storm_jackal\\fx\\sniper_eye")
            .expect("the sniper eye flare");
        H4FlareAt {
            object: 0,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            only_under: None,
            scale_by: "alive".to_owned(),
            secondary_by: String::new(),
            change_color: None,
            flare: Arc::new(flare),
        }
    }

    /// A pinhole at `eye` looking down -x toward the origin, 60 degrees
    /// across a square viewport.
    fn project(eye: [f32; 3]) -> impl Fn([f32; 3]) -> Option<([f32; 2], [f32; 2])> {
        move |point: [f32; 3]| {
            let depth = eye[0] - point[0];
            let focal = 1.0 / (30f32.to_radians()).tan();
            (depth > 0.0).then(|| {
                (
                    [
                        -(point[1] - eye[1]) * focal / depth,
                        (point[2] - eye[2]) * focal / depth,
                    ],
                    [focal; 2],
                )
            })
        }
    }

    fn view<'a>(
        eye: [f32; 3],
        forward: [f32; 3],
        project: &'a dyn Fn([f32; 3]) -> Option<([f32; 2], [f32; 2])>,
    ) -> H4FlareView<'a> {
        H4FlareView {
            eye,
            forward,
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            viewport: [800.0, 800.0],
            project,
        }
    }

    fn alive() -> FunctionValues {
        [("alive".to_owned(), 1.0)].into_iter().collect()
    }

    /// The kit's sniper eye: two reflections, the second on its own
    /// anamorphic bitmap, read by name from the H4 reflection layout.
    #[test]
    fn the_sniper_eye_reads_both_reflections_and_their_bitmaps() {
        let Some(source) = kit() else { return };
        let at = sniper_eye(&source);
        assert_eq!(at.flare.reflections.len(), 2);
        assert_eq!(at.flare.images.len(), 2);
        assert_eq!(at.flare.reflections[1].image, 1);
        assert!((at.flare.cutoff - 25f32.to_radians()).abs() < 1e-5);
        assert!(at.flare.reflections[0].flags.proximity_input);
    }

    /// Seen from in front it draws both reflections, centred where the eye
    /// is; from behind its cone puts it out, and so does a dead Jackal.
    #[test]
    fn the_sniper_eye_shows_within_its_cone_while_alive() {
        let Some(source) = kit() else { return };
        let at = sniper_eye(&source);
        let colors = [[1.0; 3]; 4];
        let front = project([2.0, 0.0, 0.0]);
        let ahead = view([2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], &front);
        let draws = at.draws(Some(&alive()), &colors, None, &ahead, 0.0, 1.0);
        assert_eq!(draws.len(), 2, "{draws:?}");
        assert_eq!(&draws[0].center_rotation[..2], &[0.0, 0.0]);
        assert!(draws[0].modulation[2] > 0.0, "its brightness");
        assert!(
            at.draws(Some(&alive()), &colors, None, &ahead, 0.0, MINIMUM_VISIBLE)
                .is_empty(),
            "too little seen"
        );
        assert!(
            at.draws(None, &colors, None, &ahead, 0.0, 1.0).is_empty(),
            "no scale, no flare"
        );
        let back = project([-2.0, 0.0, 0.0]);
        let behind = view([-2.0, 0.0, 0.0], [1.0, 0.0, 0.0], &back);
        assert!(
            at.draws(Some(&alive()), &colors, None, &behind, 0.0, 1.0)
                .is_empty()
        );
    }

    /// `use simple occlusion box test`: the patch is the inner share of the
    /// occlusion reflection's radius whatever its size, and expects 2.5 times
    /// its area in samples.
    #[test]
    fn a_simple_box_occludes_against_its_inner_share() {
        let Some(source) = kit() else { return };
        let at = sniper_eye(&source);
        assert!(at.flare.flags.simple_occlusion_box);
        let front = project([2.0, 0.0, 0.0]);
        let ahead = view([2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], &front);
        let patch = at
            .occlusion_patch(Some(&alive()), &[[1.0; 3]; 4], None, &ahead, 0.0)
            .expect("a patch");
        let half = (patch.corners[2][1] - patch.corners[0][1]).abs() / 2.0;
        let reflection = &at.flare.reflections[0];
        let radius = reflection.radius.evaluate(1.0, 0.0) * at.flare.inner_scale;
        // The occlusion reflection's radius, before its occlusion scaling at
        // full opacity, times the inner share.
        assert!((half - radius).abs() < 1e-4, "{half} {radius}");
        let pixels = 400.0 / 2.0 * half;
        assert!(
            (patch.expected - (2.5 * 4.0 * pixels * pixels).max(1.0)).abs() < 1e-2,
            "{} {pixels}",
            patch.expected
        );
    }

    #[test]
    fn visibility_counts_a_glimpse_as_ten_samples_and_expects_at_least_ten() {
        let Some(source) = kit() else { return };
        let at = sniper_eye(&source);
        assert_eq!(at.visibility(Some((5.0, 100.0))), 0.1);
        assert_eq!(at.visibility(Some((0.0, 100.0))), 0.0);
        assert_eq!(at.visibility(Some((20.0, 3.0))), 2.0);
        assert_eq!(at.visibility(None), 0.0, "no patch, nothing seen");
        let mut untested = (*at.flare).clone();
        untested.flags.no_occlusion_test = true;
        let untested = H4FlareAt {
            flare: Arc::new(untested),
            ..at
        };
        assert_eq!(untested.visibility(None), 1.0);
    }
}
