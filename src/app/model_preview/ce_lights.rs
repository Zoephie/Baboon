//! Halo CE's lens flares: what `attachments_new` hangs on an object as a
//! `light`, and the sprites `rasterizer_lens_flares_draw` makes of one.

use super::ce_attachments::{MarkerFrame, attached_marker_frame, elements, object_block};
use super::ce_passes::{argb, periodic_evaluate};
use super::h2_passes::H2Image;
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;

/// Colors of a reflection that all read zero take the light's instead.
const NO_COLOR: [f32; 4] = [0.0; 4];

/// `reflection.flags`, as `rasterizer_lens_flares_draw` reads them.
const ALIGN_WITH_SCREEN_CENTER: i128 = 1;
const RADIUS_NOT_SCALED_BY_DISTANCE: i128 = 2;

/// One sprite of a flare, set along the axis through the screen's center.
#[derive(Debug, Clone)]
pub(crate) struct CeReflection {
    flags: i128,
    /// Which of [`CeFlare::image`]s it draws.
    pub(crate) image: usize,
    position: f32,
    rotation: f32,
    radius: [f32; 2],
    brightness: [f32; 2],
    brightness_scaled_by: usize,
    tint: [f32; 4],
    lower: [f32; 4],
    upper: [f32; 4],
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
    /// How far ahead of its marker it is tested against the model.
    pub(crate) occlusion_radius: f32,
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
    pub(crate) flare: Arc<CeFlare>,
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

fn lerp3(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 3] {
    std::array::from_fn(|channel| lerp(a[channel], b[channel], t))
}

/// `rgb_colors_interpolate_and_scale`: the bounds' alphas say how much of the
/// color the change color tints.
fn light_color(lower: [f32; 4], upper: [f32; 4], change: [f32; 3], value: f32) -> [f32; 3] {
    let color = lerp3(lower, upper, value);
    if lower[3] <= 0.0001 && upper[3] <= 0.0001 {
        return std::array::from_fn(|channel| color[channel] * change[channel]);
    }
    let own = lerp(lower[3], upper[3], value);
    std::array::from_fn(|channel| color[channel] * own + change[channel] * (1.0 - own))
}

impl CeLight {
    /// What `object_get_function_value` gives it.
    fn value(&self, outputs: &[f32; 4]) -> f32 {
        self.primary.map_or(1.0, |index| outputs[index])
    }

    /// `rasterizer_lens_flares_draw`, for one light of `lights_preprocess_scene`.
    pub(crate) fn sprites(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
    ) -> Vec<CeSprite> {
        let value = self.value(outputs);
        let change = self.change_color.map_or([1.0; 3], |index| change_colors[index]);
        let color = light_color(self.lower, self.upper, change, value);
        if color == [0.0; 3] {
            return Vec::new();
        }
        let (position, forward) = self.posed(bones);
        let flare = &*self.flare;
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
        let mut brightness = fade;
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
            if reflection.flags & RADIUS_NOT_SCALED_BY_DISTANCE != 0 {
                radius *= distance;
            }
            let mut rotation = reflection.rotation.to_radians();
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
            });
        }
        sprites
    }

    fn posed(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        let Some(rows) = bones.and_then(|bones| bones.get(self.node * 3..self.node * 3 + 3)) else {
            return (self.position, self.forward);
        };
        let [x, y, z] = self.position;
        let [i, j, k] = self.forward;
        let place = |row: &[f32; 4]| row[0] * x + row[1] * y + row[2] * z + row[3];
        let turn = |row: &[f32; 4]| row[0] * i + row[1] * j + row[2] * k;
        ([place(&rows[0]), place(&rows[1]), place(&rows[2])], [turn(&rows[0]), turn(&rows[1]), turn(&rows[2])])
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
            lerp3(self.lower, self.upper, u)
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
        occlusion_radius: root.read_real("occlusion radius").unwrap_or(0.0),
        reflections,
        images,
    })
}

/// Mark the outputs an object's lights take their value from.
pub(super) fn mark_outputs_read(lights: &[CeLight], object: u8, read: &mut [bool; 4]) {
    let owned = lights.iter().filter(|light| light.object == object);
    for output in owned.filter_map(|light| light.primary) {
        read[output] = true;
    }
}

/// `A out`..`D out` of a `primary scale` or `change color` option, `none` first.
fn function_slot(value: Option<i128>) -> Option<usize> {
    usize::try_from(value? - 1).ok().filter(|slot| *slot < 4)
}

/// The lens flares among `object`'s attachments, one per marker of the name.
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
        let Ok(light) = load_referenced_tag_from_source(source, &path, "light", b"ligh") else {
            continue;
        };
        let root = light.root();
        let flare_path = root.read_tag_ref_path("lens flare").filter(|flare| !flare.is_empty());
        let Some(flare) = flare_path.and_then(|flare| read_flare(source, &flare)).map(Arc::new) else {
            continue;
        };
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
                lower: argb(root, "color lower bound"),
                upper: argb(root, "color upper bound"),
                flare: Arc::clone(&flare),
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
            flare: Arc::new(CeFlare {
                cones: [1.0, 1.0],
                fade: [0.0, 0.0],
                corona_scale: [2.0, 1.0],
                occlusion_radius: 0.1,
                reflections,
                images: Vec::new(),
            }),
        }
    }

    fn sprites(light: &CeLight, value: f32) -> Vec<CeSprite> {
        light.sprites(&[value, 0.0, 0.0, 0.0], &[[1.0; 3]; 4], None, &VIEW, 0.0)
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
        assert_eq!(light_color([1.0; 4], [1.0; 4], [0.5; 3], 1.0), [1.0; 3]);
        let unset = [1.0, 1.0, 1.0, 0.0];
        assert_eq!(light_color(unset, unset, [0.5; 3], 1.0), [0.5; 3]);
    }
}
