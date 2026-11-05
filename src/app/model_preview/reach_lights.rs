//! Reach's lights on objects: the `light`s and `cheap_light`s an object
//! attaches. A light's `light method` decides how it draws: screen space, a
//! pass of its own over the lit frame (`c_lights_view::render`), or inline, the
//! `simple_lights` the static pass reads. Cheap lights draw in screen space.
//! See `docs/reach-preview-research/lights.md`.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce::attachments::{elements, object_block};
use super::ce::lights::posed_on;
use super::h2::functions::H2FunctionValues;
use super::dx9::passes::tag_function_at;
use super::gen3::effects::{adopt_event_function, marker_name, part_markers};
use super::*;
use crate::core::source::TagSource;

/// `light_update_state`'s range when no function names one.
const DEFAULT_RANGE: f32 = 0.5;
const SMALLEST: f32 = 1e-4;

/// `light` `flags`.
const FIRST_PERSON_ONLY: i128 = 0x4;
const SCREENSPACE_SPECULAR: i128 = 0x400;

/// `default.performance_throttles`' `max gpu dynamic lights`, the inline
/// lights `build_simple_light_draw_list` takes; `simple_lights` holds 8.
const MOST_INLINE_LIGHTS: usize = 6;
pub(crate) const SIMPLE_LIGHT_ROWS: usize = 8;

/// `cheap_light_albedo_blend` in `rasterizer_globals`, `material_coeff.x`.
pub(crate) const CHEAP_ALBEDO_BLEND: f32 = 1.0;

/// How a `light` draws: its `light method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReachLightMethod {
    Screenspace,
    Inline,
    /// Shadow-mapped, re-rendering what it lights. No attached object light
    /// in the kit uses it, and it is not drawn.
    Rerender,
}

/// A function of a light: `light_color_function_struct` and
/// `light_scalar_function_struct`.
#[derive(Debug, Clone)]
struct ReachChannel {
    input: String,
    range: String,
    function: Arc<TagFunction>,
}

impl ReachChannel {
    fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        let holder = root.descend(name)?;
        Some(Self {
            input: holder.read_string_id("Input Variable").unwrap_or_default(),
            range: holder.read_string_id("Range Variable").unwrap_or_default(),
            function: Arc::new(tag_function_at(&holder, "Mapping")?),
        })
    }
}

/// `object_get_function_value_simple`: no name reads full, and a name the
/// object does not answer, nothing.
pub(super) fn object_value(values: Option<&H2FunctionValues>, name: &str) -> f32 {
    if name.is_empty() || name == "one" {
        return 1.0;
    }
    values.and_then(|values| values.get(name)).copied().unwrap_or(0.0)
}

/// A `light` as the game has it after `light_definition_version_update` and
/// `light_postprocess` (tool 0x141787D10): the kit's tags are not
/// postprocessed, and still hold degrees.
#[derive(Debug, Clone)]
struct ReachLightTag {
    method: ReachLightMethod,
    first_person_only: bool,
    specular: bool,
    frustum: bool,
    end: f32,
    start: f32,
    near_width: f32,
    height_scale: f32,
    /// Radians.
    field_of_view: f32,
    hotspot: f32,
    falloff_speed: f32,
    near_fading: f32,
    color: ReachChannel,
    intensity: ReachChannel,
    /// Version 2 and before: the intensity rescaled by `1 / distance diffusion`.
    intensity_scale: f32,
}

fn read_light_tag(source: &TagSource, path: &str) -> Option<ReachLightTag> {
    let tag = load_referenced_tag_from_source(source, path, "light", b"ligh").ok()?;
    let root = tag.root();
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let version = root.read_int_any("version").unwrap_or(4);
    let mut flags = root.read_int_any("flags").unwrap_or(0);
    let mut method = root.read_int_any("light method").unwrap_or(0);
    let end = real("attenuation end distance");
    let mut start = real("attenuation start distance");
    let mut field_of_view = real("frustum field of view");
    let mut hotspot = real("angular hotspot");
    let mut falloff_speed = real("angular falloff speed");
    // `light_definition_version_update`.
    if version == 0 {
        method = (flags & 1) + 1;
        flags &= !1;
    }
    if version <= 2 {
        start = end / 3.0;
        hotspot = field_of_view.min(30.0);
    }
    if version <= 3 && falloff_speed <= 0.0 {
        falloff_speed = 5.0;
    }
    // `light_postprocess`: degrees to radians, the cone clamped.
    field_of_view = field_of_view.clamp(0.0, 160.0).to_radians();
    hotspot = hotspot.to_radians();
    let height_scale = match real("frustum height scale") {
        0.0 => 1.0,
        scale => scale,
    };
    let color = ReachChannel::read(root, "color")?;
    let intensity = ReachChannel::read(root, "intensity")?;
    // The old intensity scaled by `distance diffusion`, 10 when unset.
    let diffusion = real("distance diffusion");
    let intensity_scale = match (version <= 2, diffusion <= SMALLEST) {
        (false, _) => 1.0,
        (true, true) => 0.1,
        (true, false) => 1.0 / diffusion,
    };
    Some(ReachLightTag {
        method: match method {
            1 => ReachLightMethod::Inline,
            2 => ReachLightMethod::Rerender,
            _ => ReachLightMethod::Screenspace,
        },
        first_person_only: flags & FIRST_PERSON_ONLY != 0,
        specular: flags & SCREENSPACE_SPECULAR != 0,
        frustum: root.read_int_any("type") == Some(1),
        end,
        start,
        near_width: real("frustum near width"),
        height_scale,
        field_of_view,
        hotspot,
        falloff_speed,
        near_fading: real("near fading distance"),
        color,
        intensity,
        intensity_scale,
    })
}

/// A `light` at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct ReachLight {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    only_under: Option<(String, String)>,
    /// The attachment's `primary scale`, which scales the intensity and,
    /// while off, keeps the light from existing.
    scale_by: String,
    force_on: bool,
    change_color: Option<usize>,
    tag: Arc<ReachLightTag>,
}

/// A `cheap_light` at a marker.
#[derive(Debug, Clone)]
pub(crate) struct ReachCheapLight {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    only_under: Option<(String, String)>,
    scale_by: String,
    secondary_by: String,
    change_color: Option<usize>,
    color: Arc<TagFunction>,
    intensity: Arc<TagFunction>,
    radius: Arc<TagFunction>,
    falloff: f32,
}

/// A light as one frame draws it, in screen space or inline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReachLitLight {
    pub(crate) method: ReachLightMethod,
    pub(crate) specular: bool,
    pub(crate) frustum: bool,
    /// The volume's origin: the light, or a frustum's apex behind it.
    pub(crate) origin: [f32; 3],
    /// The light itself, which inline lights take.
    pub(crate) position: [f32; 3],
    pub(crate) forward: [f32; 3],
    pub(crate) left: [f32; 3],
    pub(crate) up: [f32; 3],
    /// The datum's colour, intensity in.
    pub(crate) color: [f32; 3],
    /// `light_colour_falloff_power.w`: the cone's falloff power.
    pub(crate) falloff_power: f32,
    /// `light_attenuation`: where it ends, from its origin; `1 / (end -
    /// start)`; the cone's cosine cutoff; and the cone's falloff ratio.
    pub(crate) attenuation: [f32; 4],
    /// `light_rotation`'s three rows.
    pub(crate) rotation: [[f32; 3]; 3],
    /// For an inline light: `end`, `start`.
    pub(crate) range: [f32; 2],
    /// Exposure the screen-space pass folds into the colour: tag lights take
    /// `g_exposure.r`, cheap lights the exposure whole.
    pub(crate) cheap: bool,
    /// `near fading distance`.
    pub(crate) near_fading: f32,
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `light_attenuation`'s cone terms: the cosine cutoff, and the falloff ratio.
/// A sphere's cutoff of -2 leaves every direction full.
fn cone(frustum: bool, field_of_view: f32, hotspot: f32) -> [f32; 2] {
    if !frustum {
        return [-2.0, 1.0];
    }
    let cutoff = (field_of_view * 0.5).cos();
    let spread = (hotspot * 0.5).cos() - cutoff;
    [cutoff, if spread >= SMALLEST { 1.0 / spread } else { 1.0 }]
}

impl ReachLight {
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
        let tag = &self.tag;
        [&tag.color.input, &tag.color.range, &tag.intensity.input, &tag.intensity.range, &self.scale_by]
            .into_iter()
            .map(String::as_str)
    }

    /// What turns it on, whose onset its age counts from.
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// Whether it changes with no one touching the object: an intensity that
    /// runs on the light's age.
    pub(crate) fn animates(&self) -> bool {
        self.tag.intensity.input.is_empty() && self.tag.intensity.function.as_constant().is_none()
    }

    /// `light_update_state` for an attached light this frame, or none while
    /// it does not exist or gives no light. `age` is seconds since it came on.
    pub(crate) fn lit(
        &self,
        values: Option<&H2FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        age: f32,
    ) -> Option<ReachLitLight> {
        let tag = &*self.tag;
        if tag.method == ReachLightMethod::Rerender {
            return None;
        }
        // `attachments_update`: it exists only while its scale is on.
        let scale = object_value(values, &self.scale_by);
        if !self.force_on && !self.scale_by.is_empty() && scale <= 0.0 {
            return None;
        }
        let range = |name: &str| if name.is_empty() { DEFAULT_RANGE } else { object_value(values, name) };
        let color_input = object_value(values, &tag.color.input);
        let color = tag.color.function.evaluate_color(color_input, range(&tag.color.range));
        let intensity_input = if tag.intensity.input.is_empty() { age } else { object_value(values, &tag.intensity.input) };
        let mut intensity =
            tag.intensity.function.evaluate(intensity_input, range(&tag.intensity.range)) * tag.intensity_scale;
        let mut tint = [1.0; 3];
        if let Some(channel) = self.change_color {
            tint = change_colors.get(channel).copied().unwrap_or([1.0; 3]);
            if !self.scale_by.is_empty() {
                intensity *= scale;
            }
        }
        let color = [color.red * tint[0], color.green * tint[1], color.blue * tint[2]].map(|channel| channel * intensity);
        // `light_theoretically_produces_light`.
        if color.iter().all(|channel| *channel <= 0.01) || tag.end <= SMALLEST {
            return None;
        }
        let (position, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, up) = posed_on(self.node, self.position, self.up, bones);
        let left = cross(forward, up);
        let [cutoff, ratio] = cone(tag.frustum, tag.field_of_view, tag.hotspot);
        // `light_build_intermediate_geometry`: a frustum's apex sits behind it.
        let behind = if tag.frustum {
            let wide = tag.field_of_view.clamp(0.005_555_6, std::f32::consts::PI - 0.005_555_6);
            (tag.near_width.max(SMALLEST) / (wide * 0.5).tan()) * 0.5
        } else {
            0.0
        };
        let origin = std::array::from_fn(|axis| position[axis] - forward[axis] * behind);
        let rotation = if tag.frustum {
            let cot = 1.0 / (tag.field_of_view * 0.5).tan();
            let row = |side: [f32; 3], by: f32| std::array::from_fn(|axis| 0.5 * (forward[axis] - side[axis] * by));
            [row(up, cot / tag.height_scale), row(left, cot), forward]
        } else {
            [up, left, forward]
        };
        Some(ReachLitLight {
            method: tag.method,
            specular: tag.specular,
            frustum: tag.frustum,
            origin,
            position,
            forward,
            left,
            up,
            color,
            falloff_power: tag.falloff_speed,
            attenuation: [tag.end + behind, 1.0 / (tag.end - tag.start).max(SMALLEST), cutoff, ratio],
            rotation,
            range: [tag.end, tag.start],
            cheap: false,
            near_fading: tag.near_fading,
        })
    }
}

impl ReachCheapLight {
    pub(crate) fn only_under(&self) -> Option<&(String, String)> {
        self.only_under.as_ref()
    }

    /// As [`ReachLight::on_attached`].
    pub(crate) fn on_attached(mut self, name: &str) -> Self {
        adopt_event_function(name, &mut self.scale_by, &mut self.object);
        if let Some((region, _)) = &mut self.only_under {
            *region = format!("{name}: {region}");
        }
        self
    }

    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        [&self.scale_by, &self.secondary_by].into_iter().map(String::as_str)
    }

    pub(crate) fn animates(&self) -> bool {
        [&self.color, &self.intensity, &self.radius].iter().any(|function| function.as_constant().is_none())
    }

    /// `render_object_attachments`' cheap light this frame. Its functions run
    /// on render time, not the object's; the attachment's scale only decides
    /// whether it draws.
    pub(crate) fn lit(&self, values: Option<&H2FunctionValues>, change_colors: &[[f32; 3]; 4], bones: Option<&[[f32; 4]]>, time: f32) -> Option<ReachLitLight> {
        let scale = object_value(values, &self.scale_by) * object_value(values, &self.secondary_by);
        if scale <= 0.0 {
            return None;
        }
        let color = self.color.evaluate_color(time, 1.0);
        let intensity = self.intensity.evaluate(time, 1.0);
        let radius = self.radius.evaluate(time, 1.0);
        let tint = self.change_color.and_then(|channel| change_colors.get(channel)).copied().unwrap_or([1.0; 3]);
        let color = [color.red * tint[0], color.green * tint[1], color.blue * tint[2]].map(|channel| channel * intensity);
        // `add_cheap_screenspace_light_to_draw_list`.
        if color.iter().all(|channel| *channel <= 0.0) || radius <= SMALLEST {
            return None;
        }
        let (position, _) = posed_on(self.node, self.position, [1.0, 0.0, 0.0], bones);
        let wide = radius * self.falloff;
        let ratio = if wide >= SMALLEST { 1.0 / wide } else { 0.0 };
        Some(ReachLitLight {
            method: ReachLightMethod::Screenspace,
            specular: false,
            frustum: false,
            origin: position,
            position,
            forward: [1.0, 0.0, 0.0],
            left: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            color,
            falloff_power: 5.0,
            attenuation: [radius, ratio, -2.0, 1.0],
            rotation: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            range: [radius, 0.0],
            cheap: true,
            near_fading: 0.0,
        })
    }
}

impl ReachLitLight {
    /// `add_simple_light_callback`'s five rows of `simple_lights`: no apex
    /// offset, and the colour unexposed, as the static pass exposes itself.
    pub(crate) fn simple_light_rows(&self) -> [[f32; 4]; 5] {
        let [end, start] = self.range;
        let end = if end <= 0.0 { 5.0 } else { end };
        let [_, _, cutoff, ratio] = self.attenuation;
        let [x, y, z] = self.position;
        let [i, j, k] = self.forward;
        let [r, g, b] = self.color;
        [
            [x, y, z, 0.0],
            [-i, -j, -k, 0.0],
            [r, g, b, 0.0],
            [cutoff, ratio, self.falloff_power, 0.0],
            [end * end, end, 1.0 / (end - start).max(SMALLEST), 0.0],
        ]
    }
}

/// An object's attached `light`s and `cheap_light`s, at every marker of each
/// name (`light_new_attached`), first-person-only lights left out of a third
/// person view.
pub(crate) fn read_reach_lights(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> (Vec<ReachLight>, Vec<ReachCheapLight>) {
    let (mut lights, mut cheap) = (Vec::new(), Vec::new());
    let Some(block) = object_block(object, "attachments") else {
        return (lights, cheap);
    };
    let light_group = u32::from_be_bytes(*b"ligh");
    let cheap_group = u32::from_be_bytes(*b"gldf");
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if path.is_empty() || (group != light_group && group != cheap_group) {
            continue;
        }
        // No marker is the object's root.
        let name = marker_name(attachment, "marker");
        let name = if name.is_empty() { "root".to_owned() } else { name };
        let scale_by = attachment.read_string_id("primary scale").unwrap_or_default();
        let secondary_by = attachment.read_string_id("secondary scale").unwrap_or_default();
        // The tag's first option is none; the channels follow.
        let change_color = attachment.read_int_any("change color").and_then(|color| usize::try_from(color - 1).ok());
        let force_on = attachment.read_flag_names("flags").unwrap_or_default().iter().any(|(_, flag)| flag.starts_with("force always on"));
        if group == light_group {
            let Some(tag) = read_light_tag(source, &path).filter(|tag| !tag.first_person_only).map(Arc::new) else {
                continue;
            };
            for marker in part_markers(markers, &name, false) {
                let (place, node) = frame(&marker);
                lights.push(ReachLight {
                    object: owner,
                    node,
                    position: place.position,
                    forward: place.axes[0],
                    up: place.axes[2],
                    only_under: marker.only_under.clone(),
                    scale_by: scale_by.clone(),
                    force_on,
                    change_color,
                    tag: Arc::clone(&tag),
                });
            }
        } else if let Some(read) = read_cheap_light(source, &path) {
            for marker in part_markers(markers, &name, false) {
                let (place, node) = frame(&marker);
                cheap.push(ReachCheapLight {
                    object: owner,
                    node,
                    position: place.position,
                    only_under: marker.only_under.clone(),
                    scale_by: scale_by.clone(),
                    secondary_by: secondary_by.clone(),
                    change_color,
                    ..read.clone()
                });
            }
        }
    }
    (lights, cheap)
}

fn read_cheap_light(source: &TagSource, path: &str) -> Option<ReachCheapLight> {
    let tag = load_referenced_tag_from_source(source, path, "cheap_light", b"gldf").ok()?;
    let root = tag.root();
    let function = |name: &str| root.descend(name).and_then(|holder| tag_function_at(&holder, "Mapping")).map(Arc::new);
    Some(ReachCheapLight {
        object: 0,
        node: 0,
        position: [0.0; 3],
        only_under: None,
        scale_by: String::new(),
        secondary_by: String::new(),
        change_color: None,
        color: function("color")?,
        intensity: function("intensity")?,
        radius: function("radius")?,
        // `cheap_light_postprocess` clamps it.
        falloff: root.read_real("falloff").unwrap_or(0.0).clamp(0.0, 1.0),
    })
}

/// `build_simple_light_draw_list`: the inline lights the static pass reads,
/// as many as the throttles allow.
pub(crate) fn inline_rows(lit: &[ReachLitLight]) -> Vec<[[f32; 4]; 5]> {
    lit.iter()
        .filter(|light| light.method == ReachLightMethod::Inline)
        .take(MOST_INLINE_LIGHTS.min(SIMPLE_LIGHT_ROWS))
        .map(ReachLitLight::simple_light_rows)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sphere_has_no_cone_and_a_frustum_closes_at_half_its_angle() {
        assert_eq!(cone(false, 1.0, 0.5), [-2.0, 1.0]);
        let [cutoff, ratio] = cone(true, 1.0, 0.5);
        assert!((cutoff - 0.5f32.cos()).abs() < 1e-6);
        assert!((ratio - 1.0 / (0.25f32.cos() - 0.5f32.cos())).abs() < 1e-3);
        assert_eq!(cone(true, 1.0, 1.0)[1], 1.0, "no spread falls back to 1");
    }

    fn lit(method: ReachLightMethod) -> ReachLitLight {
        ReachLitLight {
            method,
            specular: false,
            frustum: true,
            origin: [0.0, 0.0, -1.0],
            position: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            left: [0.0, 1.0, 0.0],
            up: [1.0, 0.0, 0.0],
            color: [1.0, 0.5, 0.25],
            falloff_power: 5.0,
            attenuation: [5.0, 0.25, 0.9, 2.0],
            rotation: [[0.0; 3]; 3],
            range: [4.0, 0.0],
            cheap: false,
            near_fading: 0.0,
        }
    }

    #[test]
    fn an_inline_light_packs_its_rows_from_the_light_not_the_apex() {
        let rows = lit(ReachLightMethod::Inline).simple_light_rows();
        assert_eq!(rows[0], [0.0, 0.0, 0.0, 0.0], "the light's own position");
        assert_eq!(rows[1], [-0.0, -0.0, -1.0, 0.0]);
        assert_eq!(rows[3], [0.9, 2.0, 5.0, 0.0]);
        assert_eq!(rows[4], [16.0, 4.0, 0.25, 0.0]);
    }

    #[test]
    fn only_inline_lights_reach_the_static_pass_and_no_more_than_the_throttle() {
        let mut all = vec![lit(ReachLightMethod::Screenspace)];
        all.extend(std::iter::repeat_n(lit(ReachLightMethod::Inline), 9));
        assert_eq!(inline_rows(&all).len(), MOST_INLINE_LIGHTS);
        assert!(inline_rows(&[lit(ReachLightMethod::Screenspace)]).is_empty());
    }
}
