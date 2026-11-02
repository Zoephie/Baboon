//! Halo CE's widgets beyond antennas: `light_volume` (`mgs2`), drawn by
//! `light_volume_render` as a row of screen-facing sprites along a marker.
//! Read from the Xbox 360 build's symbols; what tool's postprocess
//! (`sub_5251C0`) fills in for the game is filled in here.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::ce_attachments::{elements, object_block};
use super::ce_lights::{CeAntennaQuad, CeFlareView, posed_on};
use super::ce_passes::argb;
use super::h2_passes::{H2Image, ce_color_interpolate};
use super::materials::decode_h2_image;
use super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;

/// One of `frames`, as tool leaves it.
#[derive(Debug, Clone, Copy)]
struct LightVolumeFrame {
    offset: f32,
    offset_exponent: f32,
    length: f32,
    radius: [f32; 2],
    radius_exponent: f32,
    /// Red, green, blue and alpha, hither and yon.
    colors: [[f32; 4]; 2],
    color_exponent: f32,
    brightness_exponent: f32,
}

/// A `light_volume` widget at the first marker of its name.
#[derive(Debug, Clone)]
pub(crate) struct CeLightVolume {
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    fade: [f32; 2],
    /// Perpendicular and parallel.
    brightness: [f32; 2],
    source: Option<usize>,
    count: usize,
    interpolation: i128,
    /// `light_volume_interpolate_frames` lerps the first frame with itself,
    /// so the first is the one that draws.
    frame: LightVolumeFrame,
    pub(crate) image: Arc<H2Image>,
}

fn one_if_unset(value: f32) -> f32 {
    if value <= 0.0 { 1.0 } else { value }
}

fn read_frame(frame: TagStruct<'_>) -> LightVolumeFrame {
    let real = |name: &str| frame.read_real(name).unwrap_or(0.0);
    LightVolumeFrame {
        offset: real("offset from marker"),
        offset_exponent: one_if_unset(real("offset exponent")),
        length: real("length"),
        radius: [real("radius hither"), real("radius yon")],
        radius_exponent: one_if_unset(real("radius exponent")),
        colors: [argb(frame, "tint color hither"), argb(frame, "tint color yon")],
        color_exponent: one_if_unset(real("tint color exponent")),
        brightness_exponent: one_if_unset(real("brightness exponent")),
    }
}

fn read_light_volume(source: &TagSource, path: &str, markers: &[RenderModelPreviewMarker]) -> Option<CeLightVolume> {
    let tag = load_referenced_tag_from_source(source, path, "light_volume", b"mgs2").ok()?;
    let root = tag.root();
    let frame = read_frame(*elements(root, "frames").first()?);
    let count = root.read_int_any("count").unwrap_or(0).max(0) as usize;
    if count == 0 {
        return None;
    }
    let name = root.read_string("attachment marker").unwrap_or_default();
    let (node, position, forward) = if name.is_empty() {
        (0, [0.0; 3], [1.0, 0.0, 0.0])
    } else {
        let marker = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(&name))?;
        (marker.node_index.max(0) as usize, marker.position, marker.axes[0])
    };
    // `rasterizer_widget_set_texture`: no map is the globals' `glow`.
    let image = match root.read_tag_ref_path("map").filter(|path| !path.is_empty()) {
        Some(map) => {
            let bitmap = load_referenced_tag_from_source(source, &map, "bitmap", b"bitm").ok()?;
            let sequence = root.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
            let index = elements(bitmap.root(), "sequences")
                .get(sequence)
                .map_or(0, |sequence| sequence.read_int_any("first bitmap index").unwrap_or(0).max(0) as usize);
            Arc::new(decode_h2_image(source, &map, SamplerKind::Texture2D, index)?)
        }
        None => super::h2_light_volumes::read_glow(source)?,
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let mut brightness = [real("perpendicular brightness scale"), real("parallel brightness scale")];
    if brightness == [0.0; 2] {
        brightness = [1.0; 2];
    }
    let source_slot = usize::try_from(root.read_int_any("brightness scale source").unwrap_or(0) - 1).ok().filter(|slot| *slot < 4);
    Some(CeLightVolume {
        node,
        position,
        forward,
        fade: [real("near fade distance"), real("far fade distance")],
        brightness,
        source: source_slot,
        count,
        interpolation: root.read_int_any("flags").unwrap_or(0) & 3,
        frame,
        image,
    })
}

/// The light volumes among `object`'s widgets.
pub(super) fn read_ce_light_volumes(
    source: &TagSource,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeLightVolume> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"mgs2");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(kind, path)| *kind == group && !path.is_empty())
        .filter_map(|(_, path)| read_light_volume(source, &path, markers))
        .collect()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl CeLightVolume {
    /// `light_volume_submit` and `light_volume_render`: sprites along the
    /// marker, brighter as it faces the camera the way the tag says.
    pub(crate) fn quads(&self, outputs: &[f32; 4], bones: Option<&[[f32; 4]]>, view: &CeFlareView) -> Vec<CeAntennaQuad> {
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let length = dot(forward, forward).sqrt().max(0.0001);
        let forward = forward.map(|axis| axis / length);
        let value = self.source.map(|slot| outputs[slot]);
        if value.is_some_and(|value| value <= 0.0) {
            return Vec::new();
        }
        let depth = dot(view.forward, std::array::from_fn(|axis| origin[axis] - view.eye[axis]));
        let [near, far] = self.fade;
        let mut fade = 1.0;
        if far > 0.0 {
            if depth >= far {
                return Vec::new();
            }
            fade = ((depth - far) / (near - far)).clamp(0.0, 1.0);
        }
        let facing = dot(view.forward, forward).abs();
        let [perpendicular, parallel] = self.brightness;
        let mut brightness = (perpendicular * (1.0 - facing) + parallel * facing).clamp(0.0, 1.0) * fade;
        if let Some(value) = value {
            brightness *= value;
        }
        let frame = &self.frame;
        let [hither, yon] = frame.colors;
        if brightness <= 0.0 || (hither[3] <= 0.0 && yon[3] <= 0.0) || (frame.radius[0] <= 0.0 && frame.radius[1] <= 0.0) {
            return Vec::new();
        }
        let last = (self.count - 1).max(1) as f32;
        (0..self.count)
            .map(|at| {
                let mut x = at as f32 / last;
                if frame.offset_exponent != 1.0 {
                    x = x.powf(frame.offset_exponent);
                }
                let radius_at = if frame.radius_exponent != 1.0 { x.powf(frame.radius_exponent) } else { x };
                let radius = (1.0 - radius_at) * frame.radius[0] + frame.radius[1] * radius_at;
                let color_at = if frame.color_exponent != 1.0 { x.powf(frame.color_exponent) } else { x };
                let brightness_at = if frame.brightness_exponent != 1.0 { x.powf(frame.brightness_exponent) } else { x };
                let rgb = ce_color_interpolate(
                    self.interpolation,
                    [hither[0], hither[1], hither[2]],
                    [yon[0], yon[1], yon[2]],
                    color_at,
                );
                let alpha = ((1.0 - brightness_at) * hither[3] + yon[3] * brightness_at) * brightness;
                let along = frame.length * x + frame.offset;
                let center: [f32; 3] = std::array::from_fn(|axis| origin[axis] + forward[axis] * along);
                let corner = |u: f32, v: f32| {
                    let place: [f32; 3] = std::array::from_fn(|axis| {
                        center[axis] + view.right[axis] * radius * (u * 2.0 - 1.0) + view.up[axis] * radius * (1.0 - v * 2.0)
                    });
                    [place[0], place[1], place[2], u, v]
                };
                CeAntennaQuad {
                    corners: [corner(0.0, 1.0), corner(1.0, 1.0), corner(0.0, 0.0), corner(1.0, 0.0)],
                    color: [rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)],
                }
            })
            .collect()
    }
}

/// One of a `lightning`'s `markers`.
#[derive(Debug, Clone, Copy)]
struct LightningMarker {
    node: usize,
    position: [f32; 3],
    axes: [[f32; 3]; 3],
    /// `not connected to next marker`: a bolt ends here.
    ends: bool,
    octaves: u32,
    bounds: [f32; 3],
    jitter: f32,
    thickness: f32,
    /// Alpha, red, green, blue.
    tint: [f32; 4],
}

/// A `lightning` widget: bolts from marker to marker, made afresh each frame.
#[derive(Debug, Clone)]
pub(crate) struct CeLightning {
    markers: Vec<LightningMarker>,
    count: usize,
    /// `A out`.. of the jitter, thickness and brightness scales, and the
    /// change color that tints it.
    jitter_source: Option<usize>,
    thickness_source: Option<usize>,
    tint_source: Option<usize>,
    brightness_source: Option<usize>,
    /// Its `shader`'s blend, or `global_shader_effect_additive`'s.
    pub(crate) blend: i32,
    pub(crate) nonlinear_tint: bool,
    pub(crate) image: Arc<H2Image>,
}

/// A segment of a bolt: corners (place then u, v) and color.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CeLightningQuad {
    pub(crate) corners: [[f32; 5]; 4],
    pub(crate) color: [f32; 4],
}

fn slot(value: Option<i128>) -> Option<usize> {
    usize::try_from(value? - 1).ok().filter(|slot| *slot < 4)
}

fn read_lightning(source: &TagSource, path: &str, markers: &[RenderModelPreviewMarker]) -> Option<CeLightning> {
    let tag = load_referenced_tag_from_source(source, path, "lightning", b"elec").ok()?;
    let root = tag.root();
    let found = elements(root, "markers")
        .iter()
        .map(|entry| {
            let name = entry.read_string("attachment marker").unwrap_or_default();
            // `object_get_marker_by_name`: what is not found is the origin.
            let (node, position, axes) = markers
                .iter()
                .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(&name))
                .map_or((0, [0.0; 3], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]), |marker| {
                    (marker.node_index.max(0) as usize, marker.position, marker.axes)
                });
            let bounds = match entry.field("random position bounds").and_then(|field| field.value()) {
                Some(blam_tags::TagFieldData::RealVector3d(vector)) => [vector.i, vector.j, vector.k],
                _ => [0.0; 3],
            };
            let [red, green, blue, alpha] = argb(*entry, "tint");
            LightningMarker {
                node,
                position,
                axes,
                ends: entry.read_int_any("flags").unwrap_or(0) & 1 != 0,
                octaves: entry.read_int_any("octaves to next marker").unwrap_or(0).clamp(0, 8) as u32,
                bounds,
                jitter: entry.read_real("random jitter").unwrap_or(0.0),
                thickness: entry.read_real("thickness").unwrap_or(0.0),
                tint: [alpha, red, green, blue],
            }
        })
        .collect::<Vec<_>>();
    if found.is_empty() {
        return None;
    }
    let bitmap = root.read_tag_ref_path("bitmap").filter(|path| !path.is_empty())?;
    let image = Arc::new(decode_h2_image(source, &bitmap, SamplerKind::Texture2D, 0)?);
    let shader = elements(root, "shader").first().copied();
    let (blend, nonlinear_tint) = match shader {
        Some(shader) => (
            shader.read_int_any("framebuffer blend function").unwrap_or(0) as i32,
            shader.read_int_any("shader flags").unwrap_or(0) & 2 != 0,
        ),
        // `global_shader_effect_additive`.
        None => (3, false),
    };
    let source_of = |name: &str| slot(root.read_int_any(name));
    Some(CeLightning {
        markers: found,
        count: root.read_int_any("count").unwrap_or(0).max(0) as usize,
        jitter_source: source_of("jitter scale source"),
        thickness_source: source_of("thickness scale source"),
        tint_source: source_of("tint modulation source"),
        brightness_source: source_of("brightness scale source"),
        blend,
        nonlinear_tint,
        image,
    })
}

/// The lightning among `object`'s widgets.
pub(super) fn read_ce_lightning(
    source: &TagSource,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeLightning> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"elec");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(kind, path)| *kind == group && !path.is_empty())
        .filter_map(|(_, path)| read_lightning(source, &path, markers))
        .collect()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalized(a: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(a, a).sqrt();
    (length > 0.0).then(|| a.map(|axis| axis / length))
}

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }
}

/// A point of a bolt: place, thickness, and tint (alpha first).
#[derive(Debug, Clone, Copy)]
struct BoltPoint {
    place: [f32; 3],
    thickness: f32,
    tint: [f32; 4],
}

impl CeLightning {
    /// `lightning_submit`: `count` bolts through the markers, each displaced
    /// across the view at every octave, drawn wherever one ends.
    pub(crate) fn quads(
        &self,
        outputs: &[f32; 4],
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        seed: u32,
    ) -> Vec<CeLightningQuad> {
        let mut random = Lcg(seed | 1);
        let value = |source: Option<usize>| source.map_or(1.0, |slot| outputs[slot]);
        let jitter_scale = value(self.jitter_source);
        let thickness_scale = value(self.thickness_source);
        let brightness_scale = value(self.brightness_source);
        let tint = self.tint_source.map_or([1.0; 3], |slot| change_colors[slot]);
        // `lightning_offset_marker_position`, within the marker's bounds.
        let placed = |marker: &LightningMarker, random: &mut Lcg| -> [f32; 3] {
            let random: [f32; 3] = std::array::from_fn(|axis| (random.next() * 2.0 - 1.0) * marker.bounds[axis]);
            let local: [f32; 3] = std::array::from_fn(|axis| {
                marker.position[axis] + (0..3).map(|row| marker.axes[row][axis] * random[row]).sum::<f32>()
            });
            posed_on(marker.node, local, [0.0; 3], bones).0
        };
        let mut quads = Vec::new();
        for _ in 0..self.count {
            let mut points: Vec<BoltPoint> = Vec::new();
            let mut fresh = true;
            for (at, marker) in self.markers.iter().enumerate() {
                if fresh {
                    points.clear();
                    points.push(BoltPoint { place: placed(marker, &mut random), thickness: marker.thickness, tint: marker.tint });
                    fresh = false;
                }
                if marker.ends || at + 1 == self.markers.len() {
                    if points.len() > 2 {
                        self.strip(&points, thickness_scale, brightness_scale, tint, view, random.next(), &mut quads);
                    }
                    fresh = true;
                    continue;
                }
                // To the next marker, `1 << octaves` steps, by halves.
                let to = &self.markers[at + 1];
                let end = BoltPoint { place: placed(to, &mut random), thickness: to.thickness, tint: to.tint };
                let start = points.len() - 1;
                let steps = 1usize << marker.octaves;
                points.resize(start + steps + 1, end);
                let along: [f32; 3] = std::array::from_fn(|axis| end.place[axis] - points[start].place[axis]);
                let across = normalized(cross(along, view.forward)).unwrap_or([0.0, 0.0, 1.0]);
                let mut amplitude = 1.0;
                for level in 1..=marker.octaves {
                    let half = 1usize << (marker.octaves - level);
                    let mut middle = half;
                    while middle < steps {
                        let (low, high) = (points[start + middle - half], points[start + middle + half]);
                        let t = middle as f32 / steps as f32;
                        let reach = ((to.jitter - marker.jitter) * t + marker.jitter) * amplitude * jitter_scale;
                        let push = (random.next() * 2.0 - 1.0) * reach;
                        points[start + middle] = BoltPoint {
                            place: std::array::from_fn(|axis| (low.place[axis] + high.place[axis]) * 0.5 + across[axis] * push),
                            thickness: (low.thickness + high.thickness) * 0.5,
                            tint: std::array::from_fn(|channel| (low.tint[channel] + high.tint[channel]) * 0.5),
                        };
                        middle += half * 2;
                    }
                    amplitude *= 0.5;
                }
            }
        }
        quads
    }

    /// One strip of a bolt, facing the camera, `u` started at random.
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &self,
        points: &[BoltPoint],
        thickness_scale: f32,
        brightness_scale: f32,
        tint: [f32; 3],
        view: &CeFlareView,
        start_u: f32,
        quads: &mut Vec<CeLightningQuad>,
    ) {
        let last = points.len() - 1;
        let step = 1.0 / points.len() as f32;
        let vertex = |at: usize| {
            let previous = points[at.saturating_sub(1)].place;
            let next = points[(at + 1).min(last)].place;
            let along: [f32; 3] = std::array::from_fn(|axis| next[axis] - previous[axis]);
            let across = normalized(cross(along, view.forward)).unwrap_or([0.0; 3]);
            let point = points[at];
            let width = point.thickness * thickness_scale;
            let u = at as f32 * step + start_u;
            let side = |sign: f32, v: f32| -> [f32; 5] {
                let [x, y, z] = std::array::from_fn(|axis| point.place[axis] + across[axis] * width * sign);
                [x, y, z, u, v]
            };
            let [alpha, red, green, blue] = point.tint;
            (side(1.0, 0.0), side(-1.0, 1.0), [red * tint[0], green * tint[1], blue * tint[2], alpha * brightness_scale])
        };
        for at in 0..last {
            let (top, bottom, near) = vertex(at);
            let (next_top, next_bottom, far) = vertex(at + 1);
            let color = std::array::from_fn(|channel| ((near[channel] + far[channel]) * 0.5).clamp(0.0, 1.0));
            quads.push(CeLightningQuad { corners: [top, next_top, bottom, next_bottom], color });
        }
    }
}
