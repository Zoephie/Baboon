//! Halo CE's lens flares: what `attachments_new` hangs on an object as a
//! `light`, and the sprites `rasterizer_lens_flares_draw` makes of one.

use super::attachment::MarkerFrame;
use super::ce_attachments::{attached_marker_frame, elements, object_block};
use super::ce_passes::{argb, periodic_evaluate};
use super::h2_passes::{H2Image, ce_color_interpolate};
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use blam_tags::TagFieldData;

/// One segment of an antenna: a bitmap sprite stretched along it.
#[derive(Debug, Clone)]
pub(crate) struct CeAntennaSegment {
    length: f32,
    color: [f32; 4],
    /// Left, right, top and bottom of the sprite, across the bitmap.
    rect: [f32; 4],
    /// The sprite's registration point.
    registration: [f32; 2],
    /// World units per width of the bitmap: what `antenna_new` works out so
    /// the sprite runs the length of the segment.
    scale: f32,
}

/// A `widget` that hangs a line of sprites from a marker.
#[derive(Debug, Clone)]
pub(crate) struct CeAntenna {
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    segments: Vec<CeAntennaSegment>,
    /// `antenna_render_proper`'s alpha for every sprite; see
    /// [`ANTENNA_FADE_PIXELS`].
    fade: f32,
    pub(crate) image: Arc<H2Image>,
}

/// What `antenna_render_proper` measures an antenna's `falloff pixels` and
/// `cutoff pixels` against: a constant (`flt_A2AFDC` in MCC tag_test), not
/// the antenna's size on screen, so its fade,
/// `(100 - cutoff) / (falloff - cutoff)`, is the same at any distance.
const ANTENNA_FADE_PIXELS: f32 = 100.0;

/// One antenna sprite's corners, in strip order, with texture coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CeAntennaQuad {
    pub(crate) corners: [[f32; 5]; 4],
    pub(crate) color: [f32; 4],
}

/// A point and direction on `node`, where the pose puts them.
pub(super) fn posed_on(node: usize, position: [f32; 3], direction: [f32; 3], bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
    let Some(rows) = bones.and_then(|bones| bones.get(node * 3..node * 3 + 3)) else {
        return (position, direction);
    };
    let [x, y, z] = position;
    let [i, j, k] = direction;
    let place = |row: &[f32; 4]| row[0] * x + row[1] * y + row[2] * z + row[3];
    let turn = |row: &[f32; 4]| row[0] * i + row[1] * j + row[2] * k;
    ([place(&rows[0]), place(&rows[1]), place(&rows[2])], [turn(&rows[0]), turn(&rows[1]), turn(&rows[2])])
}

impl CeAntenna {
    /// `antenna_render_proper`, for an antenna at rest: its vertices lie
    /// straight out of the marker, which is where `antenna_update`'s spring
    /// pulls them, without the point physics that sway them.
    pub(crate) fn quads(&self, bones: Option<&[[f32; 4]]>, view: &CeFlareView) -> Vec<CeAntennaQuad> {
        let (mut origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let length = dot(forward, forward).sqrt().max(0.0001);
        let forward = forward.map(|axis| axis / length);
        if self.fade <= 0.0 {
            return Vec::new();
        }
        let mut quads = Vec::with_capacity(self.segments.len());
        for segment in &self.segments {
            let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - origin[axis]);
            let across = [
                forward[1] * to_eye[2] - forward[2] * to_eye[1],
                forward[2] * to_eye[0] - forward[0] * to_eye[2],
                forward[0] * to_eye[1] - forward[1] * to_eye[0],
            ];
            let width = dot(across, across).sqrt().max(0.0001);
            let across = across.map(|axis| axis / width);
            let [left, right, top, bottom] = segment.rect;
            let [x, y] = segment.registration;
            let corner = |u: f32, v: f32| {
                let along = (u - (x + left)) * segment.scale;
                let side = (y + top - v) * segment.scale;
                let place: [f32; 3] =
                    std::array::from_fn(|axis| origin[axis] + forward[axis] * along + across[axis] * side);
                [place[0], place[1], place[2], u, v]
            };
            let [red, green, blue, alpha] = segment.color;
            quads.push(CeAntennaQuad {
                corners: [corner(left, bottom), corner(right, bottom), corner(left, top), corner(right, top)],
                color: [red, green, blue, alpha * self.fade],
            });
            origin = std::array::from_fn(|axis| origin[axis] + forward[axis] * segment.length);
        }
        quads
    }
}

/// The first sprite of an antenna vertex's sequence, as `antenna_new` and
/// `get_vertex_scale_by_distance` size it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct AntennaSprite {
    /// Left, right, top and bottom, across the bitmap.
    pub(super) rect: [f32; 4],
    pub(super) registration: [f32; 2],
    /// The image of the bitmap it is cut from.
    pub(super) index: usize,
    /// What of the bitmap's width a sprite runs along.
    pub(super) covered: f32,
    /// That image's width, in texels.
    pub(super) width: f32,
}

/// What Halo CE, 2 and 3 take off a sprite's pixels besides its spacing.
pub(super) const ANTENNA_TRIM: f32 = 1.0;
/// Reach's: `get_vertex_scale_by_distance` (the 360's 0x82C93A88, its
/// constant at 0x82C93BFC) takes half a pixel.
pub(super) const REACH_ANTENNA_TRIM: f32 = 0.5;

/// `antenna_new`'s scale for a sprite, or none where a bitmap has no such
/// sprite to draw: its `pixels`, `width · (right - left) - 2 · spacing -
/// trim`, must be more than none.
pub(super) fn antenna_sprite(bitmap: &TagFile, sequence: usize, trim: f32) -> Option<AntennaSprite> {
    let root = bitmap.root();
    let sequences = elements(root, "sequences");
    let sprite = *elements(*sequences.get(sequence)?, "sprites").first()?;
    let read = |name: &str| sprite.read_real(name).unwrap_or(0.0);
    let index = sprite.read_int_any("bitmap index")?.max(0) as usize;
    let width = elements(root, "bitmaps").get(index)?.read_int_any("width")? as f32;
    let spacing = root.read_int_any("sprite spacing").unwrap_or(0) as f32;
    let rect = [read("left"), read("right"), read("top"), read("bottom")];
    let registration = sprite.read_point2d("registration point");
    let pixels = (rect[1] - rect[0]) * width - spacing * 2.0 - trim;
    let registration = [registration.x, registration.y];
    (pixels > 0.0).then_some(AntennaSprite { rect, registration, index, covered: pixels / width, width })
}

/// `object_get_marker_by_name` as a widget reads it: the marker, or, when the
/// object has none by that name (or the name is empty), the object's origin,
/// where the game still draws it. The origin is known only for the opened
/// object (`owner` 0); a held object's widget without its marker is left out.
pub(super) fn marker_or_origin(
    markers: &[RenderModelPreviewMarker],
    name: &str,
    owner: u8,
) -> Option<(usize, [f32; 3], [f32; 3])> {
    let found = markers.iter().find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name));
    match found {
        Some(marker) => Some((marker.node_index.max(0) as usize, marker.position, marker.axes[0])),
        None => (owner == 0).then_some((0, [0.0; 3], [1.0, 0.0, 0.0])),
    }
}

fn read_antenna(source: &TagSource, path: &str, owner: u8, markers: &[RenderModelPreviewMarker]) -> Option<CeAntenna> {
    let tag = load_referenced_tag_from_source(source, path, "antenna", b"ant!").ok()?;
    let root = tag.root();
    let name = root.read_string("attachment marker name").unwrap_or_default();
    let (node, position, forward) = marker_or_origin(markers, &name, owner)?;
    let bitmap_path = root.read_tag_ref_path("bitmaps").filter(|path| !path.is_empty())?;
    let bitmap = load_referenced_tag_from_source(source, &bitmap_path, "bitmap", b"bitm").ok()?;
    let mut image_index = None;
    let mut segments = Vec::new();
    for vertex in elements(root, "vertices") {
        let sequence = vertex.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
        let AntennaSprite { rect, registration, index, covered, .. } = antenna_sprite(&bitmap, sequence, ANTENNA_TRIM)?;
        image_index.get_or_insert(index);
        let length = vertex.read_real("length").unwrap_or(0.0);
        segments.push(CeAntennaSegment {
            length,
            color: argb(vertex, "color"),
            rect,
            registration,
            scale: length / covered,
        });
    }
    let image = decode_h2_image(source, &bitmap_path, SamplerKind::Texture2D, image_index?)?;
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let (falloff, cutoff) = (real("falloff pixels"), real("cutoff pixels"));
    let fade = (ANTENNA_FADE_PIXELS - cutoff) / (falloff - cutoff);
    Some(CeAntenna {
        node,
        position,
        forward,
        segments,
        // The game's clamp, `>= 0 ? min(1) : 0`, which sends NaN to 0.
        fade: if fade >= 0.0 { fade.min(1.0) } else { 0.0 },
        image: Arc::new(image),
    })
}

/// The antennas among `object`'s widgets.
pub(super) fn read_ce_antennas(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeAntenna> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let antenna_group = u32::from_be_bytes(*b"ant!");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(group, _)| *group == antenna_group)
        .filter_map(|(_, path)| read_antenna(source, &path, owner, markers))
        .collect()
}

/// Colors of a reflection that all read zero take the light's instead.
const NO_COLOR: [f32; 4] = [0.0; 4];

/// `reflection.flags`, as `rasterizer_lens_flares_draw` reads them.
const ALIGN_WITH_SCREEN_CENTER: i128 = 1;
const RADIUS_NOT_SCALED_BY_DISTANCE: i128 = 2;
const RADIUS_SCALED_BY_OCCLUSION: i128 = 4;

/// Below this tool's light postprocess takes a value for unset.
const UNSET: f32 = 0.0001;

/// A `light`'s `flags`.
const LIGHT_DYNAMIC: i128 = 1;
const LIGHT_SPARES_OWNER: i128 = 4;

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
pub(crate) struct CeReflection {
    flags: i128,
    /// Which of [`CeFlare::images`] it draws.
    pub(crate) image: usize,
    position: f32,
    rotation: f32,
    radius: [f32; 2],
    brightness: [f32; 2],
    brightness_scaled_by: usize,
    tint: [f32; 4],
    lower: [f32; 4],
    upper: [f32; 4],
    /// Its second `flags`, for [`ce_color_interpolate`].
    interpolation: i128,
    animation_function: usize,
    animation_period: f32,
    animation_phase: f32,
}

/// A `lens_flare` tag.
#[derive(Debug, Clone)]
pub(crate) struct CeFlare {
    /// `runtime cosine falloff angle` and `cutoff`.
    cones: [f32; 2],
    fade: [f32; 2],
    corona_scale: [f32; 2],
    /// `rotation function`, and its scale in radians.
    rotation_function: i128,
    rotation_scale: f32,
    /// How big a patch `rasterizer_lens_flares_submit_occlusion_tests` tests.
    pub(crate) occlusion_radius: f32,
    /// `occlusion offset direction`: toward the viewer, along the marker, none.
    occlusion_offset: i128,
    reflections: Vec<CeReflection>,
    pub(crate) images: Vec<Arc<H2Image>>,
}

/// A light on an object, at one of the markers of its name.
#[derive(Debug, Clone)]
pub(crate) struct CeLight {
    /// Which object's functions and change colors drive it; 0 is the opened.
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    /// `A out` to `D out`, or none, which reads 1.
    primary: Option<usize>,
    change_color: Option<usize>,
    lower: [f32; 4],
    upper: [f32; 4],
    /// `interpolation flags`, for [`ce_color_interpolate`].
    interpolation: i128,
    /// `lights_preprocess_scene` submits it to light models.
    dynamic: bool,
    /// `find_point_lights_for_object_in_cluster` leaves its own object out.
    spares_owner: bool,
    radius: f32,
    radius_bounds: [f32; 2],
    /// Cosines of its `falloff angle` and `cutoff angle`, or none in all ways.
    cone: Option<[f32; 2]>,
    pub(crate) flare: Option<Arc<CeFlare>>,
    /// A light an effect made: the value its age gives it, in place of a
    /// function's, and a color of the bounds alone.
    fixed: Option<f32>,
}

/// One quad, ready to place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CeSprite {
    pub(crate) center: [f32; 3],
    /// Half its width and height, in world units.
    pub(crate) extent: [f32; 2],
    /// Radians, about the view axis.
    pub(crate) rotation: f32,
    pub(crate) color: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) image: usize,
    /// Halo 2: how strongly the map's brightest parts run to white.
    pub(crate) highlight: Option<f32>,
    /// Halo 3: how the map's own color adds, and the power that takes it to.
    pub(crate) tint: Option<[f32; 2]>,
    /// Halo 2: whether the world's depth hides it.
    pub(crate) depth_tested: bool,
}

/// Where the preview's camera sits, as the flare math wants it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CeFlareView {
    pub(crate) eye: [f32; 3],
    pub(crate) forward: [f32; 3],
    pub(crate) right: [f32; 3],
    pub(crate) up: [f32; 3],
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

fn rgb_of(color: [f32; 4]) -> [f32; 3] {
    [color[0], color[1], color[2]]
}

/// The second `flags` of a tag struct that has two. A flare's reflections keep
/// their first for how they draw and this for how their color blends.
fn second_flags(holder: TagStruct<'_>) -> i128 {
    holder.fields_all().filter(|field| field.name() == "flags").nth(1).map_or(0, |field| {
        match field.value() {
            Some(TagFieldData::WordFlags { value, .. }) => i128::from(value),
            _ => 0,
        }
    })
}

/// `lens_flare_evaluate_corona_rotation_function`: how far round a corona
/// turns as a share of a whole turn, or none for `none`.
fn corona_rotation(function: i128, forward: [f32; 3], to_light: [f32; 3], view: &CeFlareView) -> f32 {
    let back = view.forward.map(|axis| -axis);
    let along = cross(cross(forward, view.right), forward);
    let (sine, cosine) = match function {
        1 => (dot(along, view.forward), -dot(forward, view.forward)),
        2 => (-dot(view.right, forward), dot(back, forward)),
        3 => (dot(along, to_light), -dot(forward, to_light)),
        4 => (dot(view.right, to_light), -dot(back, to_light)),
        _ => return 0.0,
    };
    if sine == 0.0 { 0.0 } else { sine.atan2(cosine) / std::f32::consts::TAU }
}

/// `rgb_colors_interpolate_and_scale`: the bounds' alphas say how much of the
/// color the change color tints.
fn light_color(flags: i128, lower: [f32; 4], upper: [f32; 4], change: [f32; 3], value: f32) -> [f32; 3] {
    let color = ce_color_interpolate(flags, rgb_of(lower), rgb_of(upper), value);
    if lower[3] <= 0.0001 && upper[3] <= 0.0001 {
        return std::array::from_fn(|channel| color[channel] * change[channel]);
    }
    let own = lerp(lower[3], upper[3], value);
    std::array::from_fn(|channel| color[channel] * own + change[channel] * (1.0 - own))
}

impl CeLight {
    /// What `object_get_function_value` gives it.
    fn value(&self, outputs: &[f32; 4]) -> f32 {
        self.fixed.unwrap_or_else(|| self.primary.map_or(1.0, |index| outputs[index]))
    }

    /// The patch tested for how much of the flare the model hides, if it
    /// shows at all: `rasterizer_lens_flares_submit_occlusion_tests`.
    pub(crate) fn occlusion_patch(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
    ) -> Option<CeSprite> {
        let flare = self.flare.as_deref()?;
        if self.color(self.value(outputs), change_colors) == [0.0; 3] {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let radius = flare.occlusion_radius;
        let offset = match flare.occlusion_offset {
            0 => view.forward.map(|axis| -axis * radius),
            1 => forward.map(|axis| axis * radius * std::f32::consts::SQRT_2),
            _ => [0.0; 3],
        };
        Some(CeSprite {
            center: std::array::from_fn(|axis| position[axis] + offset[axis]),
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

    fn color(&self, value: f32, change_colors: &[[f32; 3]; 4]) -> [f32; 3] {
        // `rgb_colors_interpolate`, for a light `light_new_unattached` made.
        if self.fixed.is_some() {
            return ce_color_interpolate(self.interpolation, rgb_of(self.lower), rgb_of(self.upper), value);
        }
        let change = self.change_color.map_or([1.0; 3], |index| change_colors[index]);
        light_color(self.interpolation, self.lower, self.upper, change, value)
    }

    /// `rasterizer_set_model_lighting_point_light`, in the form
    /// [`CE_POINT_LIGHT_ROWS`] takes.
    fn point_rows(&self, outputs: &[f32; 4], change_colors: &[[f32; 3]; 4], bones: Option<&[[f32; 4]]>) -> Option<([[f32; 4]; 3], f32)> {
        if !self.dynamic {
            return None;
        }
        let value = self.value(outputs);
        let color = self.color(value, change_colors);
        let radius = self.radius * lerp(self.radius_bounds[0], self.radius_bounds[1], value);
        if color == [0.0; 3] || radius <= 0.0 {
            return None;
        }
        let (position, forward) = self.posed(bones);
        let (a, b) = match self.cone {
            Some([falloff, cutoff]) if falloff != cutoff => {
                let a = 1.0 / (falloff - cutoff);
                (a, -cutoff * a)
            }
            _ => (0.0, 1.0),
        };
        let [x, y, z] = position;
        let [i, j, k] = forward;
        let [red, green, blue] = color;
        Some(([[x, y, z, 1.0 / (radius * radius)], [i, j, k, a], [red, green, blue, b]], radius))
    }

    /// `rasterizer_lens_flares_draw`, for one light of `lights_preprocess_scene`.
    pub(crate) fn sprites(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
        occlusion: f32,
    ) -> Vec<CeSprite> {
        let Some(flare) = self.flare.as_deref() else {
            return Vec::new();
        };
        let value = self.value(outputs);
        let color = self.color(value, change_colors);
        if color == [0.0; 3] {
            return Vec::new();
        }
        let (position, forward) = self.posed(bones);
        let to_light: [f32; 3] = std::array::from_fn(|axis| position[axis] - view.eye[axis]);
        let distance = dot(to_light, view.forward);
        let length = dot(to_light, to_light).sqrt().max(0.0001);
        let toward = to_light.map(|value| value / length);
        // The axis runs through the light and the screen's center.
        let mirror: [f32; 3] =
            std::array::from_fn(|axis| (view.forward[axis] * distance - to_light[axis]) * 2.0);
        let fade = if flare.fade[1] > 0.0 {
            ((distance - flare.fade[1]) / (flare.fade[0] - flare.fade[1])).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let [falloff, cutoff] = flare.cones;
        let slope = if falloff == cutoff { 0.0 } else { 1.0 / (falloff - cutoff) };
        let offset = -cutoff * slope;
        let scales = [
            1.0,
            (offset - dot(forward, view.forward) * slope).clamp(0.0, 1.0),
            (offset - dot(forward, toward) * slope).clamp(0.0, 1.0),
            (dot(toward, view.forward) * slope + offset).clamp(0.0, 1.0),
        ];
        let screen_angle = dot(to_light, view.right).atan2(dot(to_light, view.up));
        let corona = corona_rotation(flare.rotation_function, forward, to_light, view);
        let mut brightness = fade * occlusion;
        let mut sprites = Vec::new();
        for (index, reflection) in flare.reflections.iter().enumerate() {
            let scale = scales.get(reflection.brightness_scaled_by).copied().unwrap_or(1.0);
            let alpha = lerp(reflection.brightness[0], reflection.brightness[1], value) * scale * brightness;
            // The first reflection dims the rest.
            if index == 0 {
                brightness = alpha;
            }
            if alpha <= 0.0 {
                continue;
            }
            let mut radius = lerp(reflection.radius[0], reflection.radius[1], value);
            if reflection.flags & RADIUS_SCALED_BY_OCCLUSION != 0 {
                radius *= (occlusion + 1.0) * 0.5;
            }
            if reflection.flags & RADIUS_NOT_SCALED_BY_DISTANCE != 0 {
                radius *= distance;
            }
            let mut rotation = reflection.rotation.to_radians();
            if index == 0 {
                rotation += corona * flare.rotation_scale;
            }
            if reflection.flags & ALIGN_WITH_SCREEN_CENTER != 0 {
                rotation += screen_angle;
            }
            let stretch = if index == 0 { flare.corona_scale } else { [1.0; 2] };
            sprites.push(CeSprite {
                center: std::array::from_fn(|axis| position[axis] + mirror[axis] * reflection.position),
                extent: [radius * stretch[0], radius * stretch[1]],
                rotation,
                color: reflection.tinted(color, time),
                alpha,
                image: reflection.image,
                highlight: None,
                tint: None,
                depth_tested: false,
            });
        }
        sprites
    }

    fn posed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        posed_on(self.node, self.position, self.forward, bones)
    }
}

impl CeReflection {
    /// The light's color, unless the reflection sets its own. An animation
    /// function past `one` and `zero` runs it between its bounds.
    fn tinted(&self, light: [f32; 3], time: f32) -> [f32; 3] {
        if self.tint == NO_COLOR && self.lower == NO_COLOR && self.upper == NO_COLOR {
            return light;
        }
        let tint: [f32; 3] = std::array::from_fn(|channel| self.tint[channel]);
        let own = if self.animation_function > 1 {
            let period = if self.animation_period == 0.0 { 1.0 } else { self.animation_period };
            let phase = (self.animation_phase + time) / period;
            let u = periodic_evaluate(self.animation_function, phase);
            ce_color_interpolate(self.interpolation, rgb_of(self.lower), rgb_of(self.upper), u)
        } else {
            tint
        };
        std::array::from_fn(|channel| lerp(light[channel], own[channel], self.tint[3]))
    }
}

fn read_flare(source: &TagSource, path: &str) -> Option<CeFlare> {
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
        reflections.push(CeReflection {
            flags: reflection.read_int_any("flags").unwrap_or(0),
            image,
            position: reflection.read_real("position").unwrap_or(0.0),
            rotation: reflection.read_real("rotation offset").unwrap_or(0.0),
            radius: [radius.lower, radius.upper],
            brightness: [brightness.lower, brightness.upper],
            brightness_scaled_by: reflection.read_int_any("brightness scaled by").unwrap_or(0).max(0) as usize,
            tint: argb(reflection, "tint color"),
            lower: argb(reflection, "color lower bound"),
            upper: argb(reflection, "color upper bound"),
            interpolation: second_flags(reflection),
            animation_function: reflection.read_int_any("animation function").unwrap_or(0).max(0) as usize,
            animation_period: reflection.read_real("animation period").unwrap_or(0.0),
            animation_phase: reflection.read_real("animation phase").unwrap_or(0.0),
        });
    }
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0).cos();
    Some(CeFlare {
        cones: [angle("falloff angle"), angle("cutoff angle")],
        fade: [
            root.read_real("near fade distance").unwrap_or(0.0),
            root.read_real("far fade distance").unwrap_or(0.0),
        ],
        corona_scale: [
            root.read_real("horizontal scale").filter(|scale| *scale != 0.0).unwrap_or(1.0),
            root.read_real("vertical scale").filter(|scale| *scale != 0.0).unwrap_or(1.0),
        ],
        rotation_function: root.read_int_any("rotation function").unwrap_or(0),
        // Tool's postprocess takes nothing, or a whole turn in degrees, for one.
        rotation_scale: match root.read_real("rotation function scale").unwrap_or(0.0) {
            scale if scale == 0.0 || scale == 360.0 => std::f32::consts::TAU,
            scale => scale,
        },
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        occlusion_offset: root.read_int_any("occlusion offset direction").unwrap_or(0),
        reflections,
        images,
    })
}

/// Point lights a model takes: position and inverse radius squared, direction
/// and spot A, color and spot B, for each of two.
pub(crate) const CE_POINT_LIGHT_ROWS: usize = 6;

/// `lights_prepare_for_object_dynamic` for one object: the two lights strongest
/// at `center`, within `reach` of their radius.
pub(super) fn point_light_rows(
    lights: &[CeLight],
    object: u8,
    center: [f32; 3],
    reach: f32,
    state: impl Fn(u8) -> ([f32; 4], [[f32; 3]; 4]),
    bones: Option<&[[f32; 4]]>,
) -> [[f32; 4]; CE_POINT_LIGHT_ROWS] {
    let mut found: Vec<([[f32; 4]; 3], f32)> = Vec::new();
    for light in lights {
        if light.spares_owner && light.object == object {
            continue;
        }
        let (outputs, colors) = state(light.object);
        let Some((rows, radius)) = light.point_rows(&outputs, &colors, bones) else {
            continue;
        };
        let offset: [f32; 3] = std::array::from_fn(|axis| rows[0][axis] - center[axis]);
        let distance = dot(offset, offset).sqrt();
        if distance >= radius + reach {
            continue;
        }
        let brightness = (rows[2][0] + rows[2][1] + rows[2][2]) / 3.0;
        found.push((rows, brightness * (1.0 - distance * distance / (radius * radius))));
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out = [[0.0; 4]; CE_POINT_LIGHT_ROWS];
    for (index, (rows, _)) in found.iter().take(2).enumerate() {
        out[index * 3..index * 3 + 3].copy_from_slice(rows);
    }
    out
}

/// Mark the outputs an object's lights take their value from.
pub(super) fn mark_outputs_read(lights: &[CeLight], object: u8, read: &mut [bool; 4]) {
    let owned = lights.iter().filter(|light| light.object == object);
    for output in owned.filter_map(|light| light.primary) {
        read[output] = true;
    }
}

/// What tool's light postprocess makes of a pair of `falloff` and `cutoff`
/// angles: their cosines, or none for a light that shines all ways.
fn baked_cone(falloff: f32, cutoff: f32) -> Option<[f32; 2]> {
    use std::f32::consts::PI;
    let (falloff, cutoff) =
        if cutoff < UNSET { (PI, PI) } else { (falloff.min(PI), cutoff.min(PI)) };
    let falloff = if falloff > cutoff { 0.0 } else { falloff };
    (falloff < PI).then(|| [falloff.cos(), cutoff.cos()])
}

/// The same for color bounds with nothing in them, which turn white. Only the
/// color channels count; alpha stays.
fn baked_colors(mut lower: [f32; 4], mut upper: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    if lower[..3].iter().chain(&upper[..3]).all(|channel| *channel < UNSET) {
        lower[..3].fill(1.0);
        upper[..3].fill(1.0);
    }
    (lower, upper)
}

/// `A out`..`D out` of a `primary scale` or `change color` option, `none` first.
fn function_slot(value: Option<i128>) -> Option<usize> {
    usize::try_from(value? - 1).ok().filter(|slot| *slot < 4)
}

/// A `light` tag as tool's postprocess leaves it, placed nowhere yet.
fn read_light_tag(source: &TagSource, path: &str) -> Option<CeLight> {
    let light = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = light.root();
    let flare_path = root.read_tag_ref_path("lens flare").filter(|flare| !flare.is_empty());
    let flare = flare_path.and_then(|flare| read_flare(source, &flare)).map(Arc::new);
    let flags = root.read_int_any("flags").unwrap_or(0);
    // Tags on disk lack what tool's postprocess gives them for the game.
    let modifier = root.read_real_bounds("radius modifier");
    let radius_bounds = if modifier.lower < UNSET && modifier.upper < UNSET {
        [1.0, 1.0]
    } else {
        [modifier.lower, modifier.upper]
    };
    let radius = root.read_real("radius").filter(|radius| *radius != 0.0).unwrap_or(1.0);
    let angle = |name: &str| root.read_real(name).unwrap_or(0.0);
    let (lower, upper) = baked_colors(argb(root, "color lower bound"), argb(root, "color upper bound"));
    Some(CeLight {
        object: 0,
        node: 0,
        position: [0.0; 3],
        forward: [1.0, 0.0, 0.0],
        primary: None,
        change_color: None,
        lower,
        upper,
        interpolation: root.read_int_any("interpolation flags").unwrap_or(0),
        dynamic: flags & LIGHT_DYNAMIC != 0,
        spares_owner: flags & LIGHT_SPARES_OWNER != 0,
        radius,
        radius_bounds,
        cone: baked_cone(angle("falloff angle"), angle("cutoff angle")),
        flare,
        fixed: None,
    })
}

/// A light an effect's part makes with `light_new_unattached`: it lives for
/// the tag's `duration`, fading by its `falloff function`.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectLightDef {
    pub(crate) light: CeLight,
    /// In ticks, as tool's postprocess (`sub_49CF90`) leaves it.
    duration: f32,
    falloff: i32,
}

pub(super) fn read_effect_light(source: &TagSource, path: &str) -> Option<CeEffectLightDef> {
    let mut light = read_light_tag(source, path)?;
    let tag = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = tag.root();
    // Its flags are made 3 whatever the tag's: it is always dynamic.
    light.dynamic = true;
    Some(CeEffectLightDef {
        light,
        duration: root.read_real("duration").unwrap_or(0.0) * 30.0,
        falloff: root.read_int_any("falloff function").unwrap_or(0) as i32,
    })
}

/// `transition_function_build_table`: a byte per step of 1023.
fn transition_table(function: i32) -> [u8; 1024] {
    std::array::from_fn(|at| {
        let x = at as f32 * 0.000_977_517_1;
        let y = match function {
            1 => x.powf(0.5),
            2 => x.powf(0.25),
            3 => x.powi(2),
            4 => x.powi(4),
            _ => ((x * std::f32::consts::PI - std::f32::consts::FRAC_PI_2).sin() + 1.0) * 0.5,
        };
        ((y * 255.0) as i32).clamp(0, 255) as u8
    })
}

/// `transition_function_evaluate`: linear as it is, the rest read from their
/// tables between neighbours.
pub(super) fn transition_evaluate(function: i32, value: f32) -> f32 {
    // NaN goes to zero as the game's comparisons send it.
    let value = if value >= 0.0 { value.min(1.0) } else { 0.0 };
    if !(1..=5).contains(&function) {
        return value;
    }
    static TABLES: std::sync::OnceLock<[[u8; 1024]; 6]> = std::sync::OnceLock::new();
    let table = &TABLES.get_or_init(|| std::array::from_fn(|function| transition_table(function as i32)))[function as usize];
    let scaled = value * 1023.0;
    let index = (scaled - 0.5) as i32;
    let fraction = scaled % 1.0;
    let at = |index: i32| f32::from(table[index.clamp(0, 1023) as usize]) * 0.003_921_569;
    if index == 1023 { at(1023) } else { at(index) * (1.0 - fraction) + at(index + 1) * fraction }
}

impl CeEffectLightDef {
    /// `lights_preprocess_scene` for an unattached light `age` ticks old:
    /// none once past its duration.
    pub(crate) fn at(&self, object: u8, node: usize, position: [f32; 3], forward: [f32; 3], age: u32, scale: f32) -> Option<CeLight> {
        let age = age as f32;
        if age > self.duration {
            return None;
        }
        let value = (1.0 - transition_evaluate(self.falloff, age / self.duration)) * scale;
        Some(CeLight { object, node, position, forward, fixed: Some(value), ..self.light.clone() })
    }

    pub(crate) fn alive(&self, age: u32) -> bool {
        age as f32 <= self.duration
    }
}

/// The lights among `object`'s attachments, one per marker of the name.
/// `frame` turns a marker to where the preview holds it, and its node.
pub(super) fn read_ce_lights(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<CeLight> {
    let mut lights = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return lights;
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != light_group || path.is_empty() {
            continue;
        }
        let Some(template) = read_light_tag(source, &path) else {
            continue;
        };
        // `light_new`: it needs one or the other to exist.
        if template.flare.is_none() && !template.dynamic {
            continue;
        }
        let name = attachment.read_string("marker").unwrap_or_default();
        for marker in markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name)) {
            let (place, node) = frame(marker);
            lights.push(CeLight {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                primary: function_slot(attachment.read_int_any("primary scale")),
                change_color: function_slot(attachment.read_int_any("change color")),
                ..template.clone()
            });
        }
    }
    lights
}

/// The same, for a weapon held on `parent`.
pub(super) fn read_attached_lights(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    weapon: &RenderModelPreview,
    parent: MarkerFrame,
    child: MarkerFrame,
) -> Vec<CeLight> {
    read_ce_lights(source, object, owner, &weapon.markers, |marker| {
        (attached_marker_frame(marker, &parent, &child), parent.node.max(0) as usize)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: CeFlareView = CeFlareView {
        eye: [0.0, -10.0, 0.0],
        forward: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
    };

    fn reflection(position: f32) -> CeReflection {
        CeReflection {
            flags: 0,
            image: 0,
            position,
            rotation: 0.0,
            radius: [0.1, 0.5],
            brightness: [1.0, 1.0],
            brightness_scaled_by: 0,
            tint: NO_COLOR,
            lower: NO_COLOR,
            upper: NO_COLOR,
            interpolation: 0,
            animation_function: 0,
            animation_period: 0.0,
            animation_phase: 0.0,
        }
    }

    fn light(reflections: Vec<CeReflection>) -> CeLight {
        CeLight {
            object: 0,
            node: 0,
            position: [1.0, 0.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            primary: Some(0),
            change_color: None,
            lower: [0.0, 0.0, 0.0, 1.0],
            upper: [1.0, 0.5, 0.0, 1.0],
            interpolation: 0,
            dynamic: true,
            spares_owner: false,
            radius: 2.0,
            radius_bounds: [1.0, 1.0],
            cone: None,
            fixed: None,
            flare: Some(Arc::new(CeFlare {
                cones: [1.0, 1.0],
                fade: [0.0, 0.0],
                corona_scale: [2.0, 1.0],
                rotation_function: 0,
                rotation_scale: std::f32::consts::TAU,
                occlusion_radius: 0.1,
                occlusion_offset: 0,
                reflections,
                images: Vec::new(),
            })),
        }
    }

    fn sprites(light: &CeLight, value: f32) -> Vec<CeSprite> {
        light.sprites(&[value, 0.0, 0.0, 0.0], &[[1.0; 3]; 4], None, &VIEW, 0.0, 1.0)
    }

    #[test]
    fn a_light_at_its_black_bound_shows_nothing() {
        let light = light(vec![reflection(0.0)]);
        assert!(sprites(&light, 0.0).is_empty());
        assert_eq!(sprites(&light, 1.0)[0].color, [1.0, 0.5, 0.0]);
    }

    #[test]
    fn a_light_counts_as_reading_its_owners_output() {
        let mut light = light(vec![reflection(0.0)]);
        light.primary = Some(1);
        let mut read = [false; 4];
        mark_outputs_read(std::slice::from_ref(&light), 1, &mut read);
        assert_eq!(read, [false; 4]);
        mark_outputs_read(&[light], 0, &mut read);
        assert_eq!(read, [false, true, false, false]);
    }

    fn rows(lights: &[CeLight], object: u8, value: f32) -> [[f32; 4]; CE_POINT_LIGHT_ROWS] {
        let state = |_| ([value, 0.0, 0.0, 0.0], [[1.0; 3]; 4]);
        point_light_rows(lights, object, [0.0; 3], 0.5, state, None)
    }

    #[test]
    fn a_point_light_has_the_games_constants() {
        let mut lit = light(Vec::new());
        lit.cone = Some([0.75, 0.5]);
        let rows = rows(&[lit], 0, 1.0);
        assert_eq!(rows[0], [1.0, 0.0, 0.0, 0.25]);
        assert_eq!(rows[1], [0.0, -1.0, 0.0, 4.0]);
        assert_eq!(rows[2], [1.0, 0.5, 0.0, -2.0]);
    }

    #[test]
    fn a_light_with_no_cone_shines_all_ways() {
        let rows = rows(&[light(Vec::new())], 0, 1.0);
        assert_eq!((rows[1][3], rows[2][3]), (0.0, 1.0));
    }

    #[test]
    fn dark_flare_only_and_out_of_reach_lights_add_nothing() {
        let lit = light(Vec::new());
        assert_eq!(rows(std::slice::from_ref(&lit), 0, 0.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
        let mut flare_only = lit.clone();
        flare_only.dynamic = false;
        assert_eq!(rows(&[flare_only], 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
        let mut far = lit;
        far.position = [9.0, 0.0, 0.0];
        assert_eq!(rows(&[far], 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
    }

    #[test]
    fn a_light_can_spare_only_its_own_object() {
        let mut lit = light(Vec::new());
        lit.spares_owner = true;
        assert_eq!(rows(std::slice::from_ref(&lit), 0, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
        assert_ne!(rows(&[lit], 1, 1.0), [[0.0; 4]; CE_POINT_LIGHT_ROWS]);
    }

    #[test]
    fn the_two_strongest_lights_win() {
        let dim = |x: f32| CeLight { position: [x, 0.0, 0.0], ..light(Vec::new()) };
        let rows = rows(&[dim(1.5), dim(0.0), dim(1.0)], 0, 1.0);
        assert_eq!((rows[0][0], rows[3][0]), (0.0, 1.0));
    }

    #[test]
    fn tool_bakes_a_lights_cone() {
        use std::f32::consts::{FRAC_PI_3, FRAC_PI_2, PI};
        assert_eq!(baked_cone(0.0, 0.0), None);
        assert_eq!(baked_cone(FRAC_PI_2, 0.0), None);
        assert_eq!(baked_cone(PI, PI), None);
        assert_eq!(baked_cone(FRAC_PI_3, FRAC_PI_2), Some([FRAC_PI_3.cos(), FRAC_PI_2.cos()]));
        // A falloff past the cutoff starts from straight ahead.
        assert_eq!(baked_cone(FRAC_PI_2, FRAC_PI_3), Some([1.0, FRAC_PI_3.cos()]));
    }

    #[test]
    fn tool_whitens_a_light_with_no_color() {
        let black = [0.0, 0.0, 0.0, 0.5];
        assert_eq!(baked_colors(black, black), ([1.0, 1.0, 1.0, 0.5], [1.0, 1.0, 1.0, 0.5]));
        let upper = [1.0, 0.5, 0.0, 1.0];
        assert_eq!(baked_colors(black, upper), (black, upper));
    }

    #[test]
    fn a_hidden_flare_draws_nothing_and_a_half_hidden_one_dims() {
        let light = light(vec![reflection(0.0), reflection(0.0)]);
        let at = |occlusion| light.sprites(&[1.0; 4], &[[1.0; 3]; 4], None, &VIEW, 0.0, occlusion);
        assert!(at(0.0).is_empty());
        let half = at(0.5);
        assert_eq!(half.len(), 2);
        assert_eq!(half[0].alpha, 0.5);
    }

    #[test]
    fn the_occlusion_patch_sits_where_the_flare_says() {
        let patch = |offset| {
            let mut light = light(Vec::new());
            let mut flare = (**light.flare.as_ref().unwrap()).clone();
            flare.occlusion_offset = offset;
            light.flare = Some(Arc::new(flare));
            light.occlusion_patch(&[1.0; 4], &[[1.0; 3]; 4], None, &VIEW).unwrap().center
        };
        assert_eq!(patch(0), [1.0, -0.1, 0.0]);
        assert!((patch(1)[1] + 0.141_421_4).abs() < 1e-6);
        assert_eq!(patch(2), [1.0, 0.0, 0.0]);
        // A dark light tests nothing.
        assert!(light(Vec::new()).occlusion_patch(&[0.0; 4], &[[1.0; 3]; 4], None, &VIEW).is_none());
    }

    #[test]
    fn an_antenna_runs_straight_out_of_its_marker_facing_the_viewer() {
        let segment = CeAntennaSegment {
            length: 0.05,
            color: [1.0; 4],
            rect: [0.0, 1.0, 0.25, 0.75],
            registration: [0.0, 0.25],
            scale: 0.05,
        };
        let antenna = CeAntenna {
            node: 0,
            position: [0.0, 0.0, 1.0],
            forward: [0.0, 0.0, 2.0],
            segments: vec![segment.clone(), segment],
            fade: 1.0,
            image: Arc::new(H2Image::Flat(crate::app::model_preview::materials::TextureImage {
                rgba: Vec::new(),
                width: 0,
                height: 0,
                repeat_x: true,
                repeat_y: true,
                scale: [1.0; 2],
            })),
        };
        let quads = antenna.quads(None, &VIEW);
        assert_eq!(quads.len(), 2);
        let [first, _, _, last] = quads[0].corners;
        assert!((first[2] - 1.0).abs() < 1e-6);
        assert!((last[2] - 1.05).abs() < 1e-6);
        // The second starts where the first ends.
        assert!((quads[1].corners[0][2] - 1.05).abs() < 1e-6);
        // Facing the viewer down y, its width lies along x.
        assert!(first[1].abs() < 1e-6);
        assert!((first[0] - last[0]).abs() > 0.02);
    }

    #[test]
    fn a_corona_turns_with_how_the_light_faces_the_view() {
        // Seen head on, the light points straight back at the viewer.
        let facing = [0.0, -1.0, 0.0];
        assert_eq!(corona_rotation(0, facing, [0.0, 1.0, 0.0], &VIEW), 0.0);
        assert_eq!(corona_rotation(1, facing, [0.0, 1.0, 0.0], &VIEW), 0.0);
        // Turned a quarter away to the right, the share is a quarter turn.
        let sideways = corona_rotation(2, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], &VIEW);
        assert!((sideways.abs() - 0.25).abs() < 1e-5, "{sideways}");
        // Only the first reflection, the corona, carries it.
        let mut turning = light(vec![reflection(0.0), reflection(0.0)]);
        let flare = Arc::make_mut(turning.flare.as_mut().unwrap());
        flare.rotation_function = 2;
        turning.forward = [1.0, 0.0, 0.0];
        let drawn = sprites(&turning, 1.0);
        assert!(drawn[0].rotation.abs() > 1.0);
        assert_eq!(drawn[1].rotation, 0.0);
    }

    #[test]
    fn no_source_reads_full() {
        let mut light = light(vec![reflection(0.0)]);
        light.primary = None;
        assert_eq!(sprites(&light, 0.0).len(), 1);
    }

    #[test]
    fn the_radius_follows_the_light_and_only_the_corona_stretches() {
        let light = light(vec![reflection(0.0), reflection(0.0)]);
        let drawn = sprites(&light, 1.0);
        assert_eq!(drawn[0].extent, [1.0, 0.5]);
        assert_eq!(drawn[1].extent, [0.5, 0.5]);
        assert_eq!(sprites(&light, 0.5)[1].extent, [0.3, 0.3]);
    }

    #[test]
    fn reflections_run_across_the_screen_center() {
        let light = light(vec![reflection(0.0), reflection(1.0)]);
        let drawn = sprites(&light, 1.0);
        assert_eq!(drawn[0].center, [1.0, 0.0, 0.0]);
        // Two lengths along the axis lands it mirrored about the center.
        assert_eq!(drawn[1].center, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn the_first_reflection_dims_the_rest() {
        let mut first = reflection(0.0);
        first.brightness = [0.5, 0.5];
        let light = light(vec![first, reflection(0.0)]);
        let drawn = sprites(&light, 1.0);
        assert_eq!((drawn[0].alpha, drawn[1].alpha), (0.5, 0.5));
    }

    #[test]
    fn a_change_color_tints_by_the_bounds_alpha() {
        assert_eq!(light_color(0, [1.0; 4], [1.0; 4], [0.5; 3], 1.0), [1.0; 3]);
        let unset = [1.0, 1.0, 1.0, 0.0];
        assert_eq!(light_color(0, unset, unset, [0.5; 3], 1.0), [0.5; 3]);
    }

    #[test]
    fn transitions_read_their_tables_and_linear_reads_none() {
        assert_eq!(transition_evaluate(0, 0.3), 0.3);
        assert_eq!(transition_evaluate(0, 1.7), 1.0);
        assert_eq!(transition_evaluate(0, f32::NAN), 0.0);
        // Squared, a byte to the step: near enough a quarter at a half.
        assert!((transition_evaluate(3, 0.5) - 0.25).abs() < 0.01);
        assert!((transition_evaluate(1, 0.25) - 0.5).abs() < 0.01);
        assert_eq!(transition_evaluate(4, 0.0), 0.0);
    }

    #[test]
    fn an_effects_light_fades_by_its_age_and_then_is_gone() {
        let def = CeEffectLightDef { light: light(Vec::new()), duration: 30.0, falloff: 0 };
        let made = |age| def.at(0, 0, [0.0; 3], [1.0, 0.0, 0.0], age, 0.5);
        assert_eq!(made(0).and_then(|light| light.fixed), Some(0.5));
        let half = made(15).and_then(|light| light.fixed).unwrap();
        assert!((half - 0.25).abs() < 1e-6);
        assert!(made(30).is_some());
        assert!(made(31).is_none());
        assert!(def.alive(30) && !def.alive(31));
        // Its color is the bounds' alone, whatever the change colors say.
        let light = made(0).unwrap();
        assert_eq!(light.color(0.5, &[[0.0; 3]; 4]), [0.5, 0.25, 0.0]);
    }
}
