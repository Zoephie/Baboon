//! Halo 2's `light_volume`: sprites strung along a marker's forward, each a
//! quad facing the screen, stretched along the axis as the view turns side on.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::attachment::MarkerFrame;
use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeAntennaQuad, CeFlareView, posed_on};
use super::h2_functions::H2FunctionValues;
use super::h2_passes::{H2Image, tag_function_at};
use super::h3_effects::marker_name;
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// `sprites[256]`.
const MOST_SPRITES: usize = 256;

/// `s_light_volume_aspect`: how far a sprite stretches as the axis turns away.
#[derive(Debug, Clone)]
struct VolumeAspect {
    along: TagFunction,
    across: TagFunction,
    parallel_scale: f32,
    threshold: f32,
    exponent: f32,
}

/// One of a `light_volume`'s `volumes`.
#[derive(Debug, Clone)]
pub(crate) struct H2Volume {
    count: usize,
    fuzzy: bool,
    geometric: bool,
    offset: TagFunction,
    radius: TagFunction,
    brightness: TagFunction,
    color: TagFunction,
    facing: TagFunction,
    aspect: Option<VolumeAspect>,
    bitmap: Option<Arc<H2Image>>,
}

/// A `light_volume` at a marker of the object.
#[derive(Debug, Clone)]
pub(crate) struct H2LightVolume {
    pub(crate) object: u8,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    falloff: [f32; 2],
    scale_by: String,
    volumes: Arc<Vec<H2Volume>>,
}

fn function_of(holder: TagStruct<'_>, name: &str) -> Option<TagFunction> {
    tag_function_at(&holder.descend(name)?, "function")
}

fn read_volume(source: &TagSource, volume: TagStruct<'_>, glow: Option<&Arc<H2Image>>) -> Option<H2Volume> {
    let flags = volume.read_flag_names("flags").unwrap_or_default();
    let flagged = |wanted: &str| flags.iter().any(|(_, name)| name.starts_with(wanted));
    let aspect = elements(volume, "aspect").first().and_then(|aspect| {
        Some(VolumeAspect {
            along: function_of(*aspect, "along axis")?,
            across: function_of(*aspect, "away from axis")?,
            parallel_scale: aspect.read_real("parallel scale")?,
            threshold: aspect.read_real("parallel threshold angle")?,
            exponent: aspect.read_real("parallel exponent")?,
        })
    });
    let own = volume.read_tag_ref_path("bitmap").filter(|path| !path.is_empty());
    let bitmap = match own {
        Some(path) => decode_h2_image(source, &path, SamplerKind::Texture2D, 0).map(Arc::new),
        None => glow.cloned(),
    };
    Some(H2Volume {
        count: volume.read_int_any("sprite count")?.clamp(2, MOST_SPRITES as i128) as usize,
        fuzzy: flagged("fuzzy"),
        geometric: flagged("force linear radius function"),
        offset: function_of(volume, "offset function")?,
        radius: function_of(volume, "radius function")?,
        brightness: function_of(volume, "brightness function")?,
        color: function_of(volume, "color function")?,
        facing: function_of(volume, "facing function")?,
        aspect,
        bitmap,
    })
}

/// `globals`' `rasterizer data`, whose `glow` stands in for a volume's bitmap.
pub(super) fn read_glow(source: &TagSource) -> Option<Arc<H2Image>> {
    let globals = load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
    let data = elements(globals.root(), "rasterizer data");
    let path = data.first()?.read_tag_ref_path("glow").filter(|path| !path.is_empty())?;
    decode_h2_image(source, &path, SamplerKind::Texture2D, 0).map(Arc::new)
}

pub(super) fn read_h2_bitmap(source: &TagSource, path: &str) -> Option<Arc<H2Image>> {
    decode_h2_image(source, path, SamplerKind::Texture2D, 0).map(Arc::new)
}

/// `render_submit_object_attachments`: one at every marker of the name.
pub(super) fn read_h2_light_volumes(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H2LightVolume> {
    let group = u32::from_be_bytes(*b"MGS2");
    let mut found = Vec::new();
    let Some(block) = object_block(object, "attachments") else {
        return found;
    };
    let attachments = elements(block, "attachments");
    if !attachments.iter().any(|at| at.read_tag_ref_with_group("type").is_some_and(|(kind, _)| kind == group)) {
        return found;
    }
    let glow = read_glow(source);
    for attachment in attachments {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if kind != group || path.is_empty() {
            continue;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &path, "light_volume", b"MGS2") else {
            continue;
        };
        let root = tag.root();
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let volumes = elements(root, "volumes");
        let volumes: Vec<H2Volume> =
            volumes.iter().filter_map(|volume| read_volume(source, *volume, glow.as_ref())).collect();
        let volumes = Arc::new(volumes);
        let name = marker_name(attachment, "marker");
        for marker in markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name)) {
            let (place, node) = frame(marker);
            found.push(H2LightVolume {
                object: owner,
                node,
                position: place.position,
                forward: place.axes[0],
                falloff: [real("falloff distance from camera"), real("cutoff distance from camera")],
                scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                volumes: Arc::clone(&volumes),
            });
        }
    }
    found
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `(cutoff - distance) / (cutoff - falloff)`, or all of it with no range.
fn camera_fade([falloff, cutoff]: [f32; 2], distance: f32) -> f32 {
    if cutoff <= falloff { 1.0 } else { ((cutoff - distance) / (cutoff - falloff)).clamp(0.0, 1.0) }
}

/// `sub_94BE80`: how much of the aspect applies, by how side on the axis is.
fn aspect_amount(threshold_radians: f32, exponent: f32, facing_cosine: f32) -> f32 {
    let side_on = (facing_cosine.clamp(0.0, 1.0).acos() * 2.0 / std::f32::consts::PI).clamp(0.0, 1.0);
    let threshold = (threshold_radians * 2.0 / std::f32::consts::PI).clamp(0.0, 0.9999);
    let amount = ((side_on - threshold) / (1.0 - threshold)).clamp(0.0, 1.0);
    if amount > 0.0 && exponent > 0.0 { amount.powf(exponent) } else if amount > 0.0 { 1.0 } else { 0.0 }
}

impl H2Volume {
    /// `sprite_index / (count - 1)`, the `force linear offset` spacing. The
    /// others use a table `tool.exe` builds, which tags on disk do not have.
    fn place_along(&self, sprite: usize) -> f32 {
        sprite as f32 / (self.count - 1) as f32
    }

    pub(crate) fn fuzzy(&self) -> bool {
        self.fuzzy
    }

    pub(crate) fn bitmap(&self) -> Option<&Arc<H2Image>> {
        self.bitmap.as_ref()
    }
}

impl H2LightVolume {
    pub(crate) fn volumes(&self) -> &[H2Volume] {
        &self.volumes
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// `rasterizer_light_volume_render`, as a quad a sprite.
    pub(crate) fn quads(
        &self,
        volume: &H2Volume,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
    ) -> Vec<CeAntennaQuad> {
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - origin[axis]);
        let distance = dot(to_eye, to_eye).sqrt();
        let unit_eye = to_eye.map(|axis| axis / distance.max(f32::EPSILON));
        let fade = camera_fade(self.falloff, distance);
        let scale_by = |name: &str| values.and_then(|values| values.get(name)).copied().unwrap_or(0.0);
        let brightness = fade * if self.scale_by.is_empty() { 1.0 } else { scale_by(&self.scale_by) };
        if brightness <= 0.0 {
            return Vec::new();
        }
        let facing_cosine = dot(unit_eye, forward).abs();
        let facing = 1.0 - volume.facing.evaluate((1.0 - facing_cosine).clamp(0.0, 1.0), 0.0);
        let amount = volume.aspect.as_ref().map_or(0.0, |aspect| {
            aspect_amount(aspect.threshold.to_radians(), aspect.exponent, facing_cosine)
        });
        // The axis on the screen, which the quads turn to lie along.
        let (across, up) = (dot(forward, view.right), dot(forward, view.up));
        let length = (across * across + up * up).sqrt();
        let (cos, sin) = if length > 0.0 { (across / length, up / length) } else { (1.0, 0.0) };
        let side: [f32; 3] = std::array::from_fn(|axis| view.right[axis] * cos + view.up[axis] * sin);
        let lift: [f32; 3] = std::array::from_fn(|axis| view.up[axis] * cos - view.right[axis] * sin);
        (0..volume.count)
            .filter_map(|sprite| {
                let along = volume.place_along(sprite);
                let offset = volume.offset.evaluate(along, facing);
                let radius = volume.radius.evaluate(along, facing);
                let shine = volume.brightness.evaluate(along, facing);
                let (wide, high) = volume.aspect.as_ref().map_or((1.0, 1.0), |aspect| {
                    let base = aspect.parallel_scale;
                    if amount <= 0.0 {
                        (base, base)
                    } else {
                        (
                            base - (base - aspect.along.evaluate(along, facing)) * amount,
                            base - (base - aspect.across.evaluate(along, facing)) * amount,
                        )
                    }
                });
                let (half_wide, half_high) = (radius * wide, radius * high);
                if half_wide <= 0.0 || half_high <= 0.0 {
                    return None;
                }
                let alpha = (shine * brightness).clamp(0.0, 1.0);
                let color = volume.color.evaluate_color(along, facing);
                let tint = [color.red, color.green, color.blue].map(|channel| channel.clamp(0.0, 1.0) * alpha);
                let center: [f32; 3] = std::array::from_fn(|axis| origin[axis] + forward[axis] * offset);
                let corner = |x: f32, y: f32, u: f32, v: f32| -> [f32; 5] {
                    let [a, b, c] = std::array::from_fn(|axis| {
                        center[axis] + side[axis] * x * half_wide + lift[axis] * y * half_high
                    });
                    [a, b, c, u, v]
                };
                Some(CeAntennaQuad {
                    corners: [corner(-1.0, 1.0, 0.0, 0.0), corner(1.0, 1.0, 1.0, 0.0), corner(-1.0, -1.0, 0.0, 1.0), corner(1.0, -1.0, 1.0, 1.0)],
                    color: [tint[0], tint[1], tint[2], alpha],
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_goes_out_between_its_camera_distances() {
        assert_eq!(camera_fade([90.0, 100.0], 80.0), 1.0);
        assert_eq!(camera_fade([90.0, 100.0], 95.0), 0.5);
        assert_eq!(camera_fade([90.0, 100.0], 120.0), 0.0);
        assert_eq!(camera_fade([0.0, 0.0], 500.0), 1.0, "no range is no fade");
    }

    #[test]
    fn the_aspect_comes_in_as_the_axis_turns_side_on() {
        assert_eq!(aspect_amount(0.0, 1.0, 1.0), 0.0, "end on, none");
        assert!((aspect_amount(0.0, 1.0, 0.0) - 1.0).abs() < 1e-5, "side on, all");
        assert_eq!(aspect_amount(std::f32::consts::FRAC_PI_4, 1.0, 0.9), 0.0, "short of the threshold");
        let half = aspect_amount(0.0, 2.0, std::f32::consts::FRAC_PI_4.cos());
        assert!((half - 0.25).abs() < 1e-5, "{half}");
    }
}
