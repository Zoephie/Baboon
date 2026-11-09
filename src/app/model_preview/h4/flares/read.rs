//! Halo 4's lens flares: their flags, curves and reflections, read off the tag and placed on an object.

use super::*;

pub(super) const SMALLEST: f32 = 1e-4;

/// `lens_flares_render`'s least visible share that draws.
pub(super) const MINIMUM_VISIBLE: f32 = 0.05;

/// The fewest samples `lens_flares_render` counts a patch as, seen or expected.
pub(super) const FEWEST_SAMPLES: f32 = 10.0;

/// Width plus height, in pixels, past which a patch tests its inner share alone.
pub(super) const OCCLUSION_PIXELS: f32 = 600.0;

/// The `lens_flare` `flags` the preview reads.
#[derive(Debug, Clone, Copy)]
pub(super) struct H4FlareFlags {
    pub(super) rotate_occlusion_box: bool,
    pub(super) no_occlusion_test: bool,
    pub(super) first_person_only: bool,
    pub(super) simple_occlusion_box: bool,
    pub(super) no_opacity_feedback: bool,
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
pub(super) struct H4ReflectionFlags {
    pub(super) rotate_from_center: bool,
    pub(super) radius_by_distance: bool,
    pub(super) radius_by_occlusion: bool,
    pub(super) ignore_external_color: bool,
    pub(super) lock_x: bool,
    pub(super) lock_y: bool,
    pub(super) proximity_input: bool,
    pub(super) mirror: bool,
    pub(super) flip_u: bool,
    pub(super) flip_v: bool,
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
pub(super) struct H4FlareCurve {
    pub(super) input: String,
    pub(super) function: Arc<TagFunction>,
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
pub(super) struct H4Reflection {
    pub(super) flags: H4ReflectionFlags,
    /// Into the flare's images.
    pub(super) image: usize,
    /// `lens_flare_get_bitmap_aspect_ratio`.
    pub(super) aspect: [f32; 2],
    /// Degrees.
    pub(super) rotation_offset: f32,
    pub(super) axis_offset: f32,
    pub(super) offset_bounds: [f32; 2],
    pub(super) radius: Arc<TagFunction>,
    pub(super) scale: [Arc<TagFunction>; 2],
    pub(super) brightness: Arc<TagFunction>,
    pub(super) color: H4ReflectionColor,
    pub(super) modulation: f32,
    pub(super) tint_power: f32,
}

#[derive(Debug, Clone)]
pub(super) enum H4ReflectionColor {
    Curve(Arc<TagFunction>),
    /// A Reach-era tag's constant, where it has no curve.
    Constant([f32; 3]),
}

/// A `lens_flare` tag, by field name: the kit holds two reflection layouts.
#[derive(Debug, Clone)]
pub(crate) struct H4LensFlare {
    pub(super) flags: H4FlareFlags,
    /// Radians.
    pub(super) falloff: f32,
    pub(super) cutoff: f32,
    pub(super) near_fade: [f32; 2],
    /// `near fade distance`, `far fade distance`.
    pub(super) far_fade: [f32; 2],
    pub(super) occlusion_reflection: usize,
    pub(super) occlusion_offset: f32,
    pub(super) offset_direction: OcclusionOffset,
    pub(super) inner_scale: f32,
    pub(super) falloff_function: u8,
    pub(super) rotation_function: i128,
    pub(super) rotation_scale: f32,
    pub(super) time_brightness: Option<Arc<TagFunction>>,
    pub(super) age_brightness: Option<Arc<TagFunction>>,
    pub(super) time_rotation: Option<Arc<TagFunction>>,
    pub(super) age_rotation: Option<Arc<TagFunction>>,
    pub(super) time_color: Option<H4FlareCurve>,
    pub(super) age_color: Option<H4FlareCurve>,
    pub(super) reflections: Vec<H4Reflection>,
    /// Each reflection's bitmap image, by `image`.
    pub(crate) images: Vec<Gen3Bitmap>,
}

fn aspect_of(image: &Image) -> [f32; 2] {
    let (width, height) = match image {
        Image::Flat(image) => (image.width as f32, image.height as f32),
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

pub(crate) fn read_h4_lens_flare(reader: &TagReader, path: &str) -> Option<H4LensFlare> {
    let tag = reader.load(path, "lens_flare", b"lens").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let flags = H4FlareFlags::read(root);
    let bitmap = root
        .read_tag_ref_path("bitmap")
        .filter(|path| !path.is_empty());
    let mut images: Vec<(String, i16, Gen3Bitmap)> = Vec::new();
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
                let Some(read) = read_h3_bitmap(reader, &path, index, SamplerKind::Texture2D)
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
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    pub(super) forward: [f32; 3],
    pub(super) only_under: Option<(String, String)>,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    pub(super) change_color: Option<usize>,
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
pub(super) fn corona_rotation(
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

/// An object's attached `lens_flare`s, at every marker of each name; none
/// that draw only in first person.
pub(crate) fn read_h4_flares(
    reader: &TagReader,
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
                let flare = read_h4_lens_flare(reader, &path).map(Arc::new);
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

attached!(H4FlareAt);
