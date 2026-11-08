//! Halo CE's widgets beyond antennas: `light_volume` (`mgs2`), drawn by
//! `light_volume_render` as a row of screen-facing sprites along a marker.
//! Read from the Xbox 360 build's symbols; what tool's postprocess
//! (`sub_5251C0`) fills in for the game is filled in here.

use std::sync::Arc;

use blam_tags::{TagFile, TagStruct};

use super::super::common::fields::{elements, function_slot, function_slot_of, has_flag, object_block};
use super::super::antennas::AntennaQuad;
use super::super::common::view::{ViewFrame, posed_on};
use super::passes::{FramebufferBlend, argb};
use super::super::dx9::passes::{ce_color_interpolate, ce_color_interpolation};
use super::super::common::image::H2Image;
use super::super::materials::decode_h2_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::app::model_preview::common::math::{cross, dot, nonzero_unit};
use crate::app::model_preview::common::random::{high24_over_2_24, lcg_step};

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
    /// Which object's functions drive it; 0 is the opened.
    pub(crate) object: u8,
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

fn read_light_volume(reader: &TagReader, path: &str, owner: u8, markers: &[RenderModelPreviewMarker]) -> Option<CeLightVolume> {
    let tag = reader.load(path, "light_volume", b"mgs2").ok()?;
    let root = tag.root();
    let frame = read_frame(*elements(root, "frames").first()?);
    let count = root.read_int_any("count").unwrap_or(0).max(0) as usize;
    if count == 0 {
        return None;
    }
    let name = root.read_string("attachment marker").unwrap_or_default();
    let (node, position, forward) = super::lights::marker_or_origin(markers, &name, owner)?;
    // `rasterizer_widget_set_texture`: no map is the globals' `glow`.
    let image = match root.read_tag_ref_path("map").filter(|path| !path.is_empty()) {
        Some(map) => {
            let bitmap = reader.load(&map, "bitmap", b"bitm").ok()?;
            let sequence = root.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
            let index = elements(bitmap.root(), "sequences")
                .get(sequence)
                .map_or(0, |sequence| sequence.read_int_any("first bitmap index").unwrap_or(0).max(0) as usize);
            Arc::new(decode_h2_image(reader, &map, SamplerKind::Texture2D, index)?)
        }
        None => super::super::h2::light_volumes::read_glow(reader)?,
    };
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let mut brightness = [real("perpendicular brightness scale"), real("parallel brightness scale")];
    if brightness == [0.0; 2] {
        brightness = [1.0; 2];
    }
    let source_slot = function_slot_of(root, "brightness scale source");
    Some(CeLightVolume {
        object: owner,
        node,
        position,
        forward,
        fade: [real("near fade distance"), real("far fade distance")],
        brightness,
        source: source_slot,
        count,
        interpolation: ce_color_interpolation(
            has_flag(root, "flags", "interpolate color in hsv"),
            has_flag(root, "flags", "...more colors"),
        ),
        frame,
        image,
    })
}

/// The light volumes among `object`'s widgets.
pub(in crate::app::model_preview) fn read_ce_light_volumes(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
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
        .filter_map(|(_, path)| read_light_volume(reader, &path, owner, markers))
        .collect()
}


impl super::functions::ReadsOutputs for CeLightVolume {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        self.source.into_iter().collect()
    }
}

impl super::functions::ReadsOutputs for CeLightning {
    fn object(&self) -> u8 {
        self.object
    }
    /// Its tint source names a change color, not an output.
    fn outputs_read(&self) -> Vec<usize> {
        [self.jitter_source, self.thickness_source, self.brightness_source].into_iter().flatten().collect()
    }
}

impl super::functions::ReadsOutputs for CeGlow {
    fn object(&self) -> u8 {
        self.object
    }
    fn outputs_read(&self) -> Vec<usize> {
        [self.rotation.source, self.translation.source, self.distance_source, self.size_source, self.color_source]
            .into_iter()
            .flatten()
            .collect()
    }
}

impl CeLightVolume {
    /// Where its marker is, which `light_volume_submit` sorts it by.
    pub(crate) fn marker(&self, bones: Option<&[[f32; 4]]>) -> [f32; 3] {
        posed_on(self.node, self.position, self.forward, bones).0
    }

    /// `light_volume_submit` and `light_volume_render`: sprites along the
    /// marker, brighter as it faces the camera the way the tag says.
    pub(crate) fn quads(&self, outputs: &[f32; 4], active: &[bool; 4], bones: Option<&[[f32; 4]]>, view: &ViewFrame) -> Vec<AntennaQuad> {
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let length = dot(forward, forward).sqrt().max(0.0001);
        let forward = forward.map(|axis| axis / length);
        let value = self.source.map(|slot| outputs[slot]);
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
        // `light_volume_render` scales by the output only while it is on.
        if let (Some(value), Some(slot)) = (value, self.source)
            && active[slot]
        {
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
                AntennaQuad {
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
    pub(crate) object: u8,
    markers: Vec<LightningMarker>,
    count: usize,
    /// `A out`.. of the jitter, thickness and brightness scales, and the
    /// change color that tints it.
    jitter_source: Option<usize>,
    thickness_source: Option<usize>,
    tint_source: Option<usize>,
    brightness_source: Option<usize>,
    /// Its `shader`'s blend, or `global_shader_effect_additive`'s.
    pub(crate) blend: FramebufferBlend,
    pub(crate) nonlinear_tint: bool,
    /// `shader flags`' `sort bias`: sorted a quarter unit nearer.
    pub(crate) sort_bias: bool,
    pub(crate) image: Arc<H2Image>,
}

/// A segment of a bolt: corners (place then u, v) and color.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CeLightningQuad {
    pub(crate) corners: [[f32; 5]; 4],
    /// Each corner's color: `lightning_submit` writes one per vertex.
    pub(crate) colors: [[f32; 4]; 4],
}

fn read_lightning(reader: &TagReader, path: &str, owner: u8, markers: &[RenderModelPreviewMarker]) -> Option<CeLightning> {
    let tag = reader.load(path, "lightning", b"elec").ok()?;
    let root = tag.root();
    // `lightning_submit` draws nothing unless the first marker is found; an
    // empty name counts, as the object's origin.
    let first = elements(root, "markers").first()?.read_string("attachment marker").unwrap_or_default();
    if !first.is_empty() && !markers.iter().any(|marker| marker.name.eq_ignore_ascii_case(&first)) {
        return None;
    }
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
                ends: has_flag(*entry, "flags", "not connected to next marker"),
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
    let image = Arc::new(decode_h2_image(reader, &bitmap, SamplerKind::Texture2D, 0)?);
    let shader = elements(root, "shader").first().copied();
    let (blend, nonlinear_tint, sort_bias) = match shader {
        Some(shader) => (
            FramebufferBlend::read(shader),
            has_flag(shader, "shader flags", "nonlinear tint"),
            has_flag(shader, "shader flags", "sort bias"),
        ),
        // `global_shader_effect_additive`.
        None => (FramebufferBlend::Add, false, false),
    };
    let source_of = |name: &str| function_slot_of(root, name);
    Some(CeLightning {
        object: owner,
        markers: found,
        count: root.read_int_any("count").unwrap_or(0).max(0) as usize,
        jitter_source: source_of("jitter scale source"),
        thickness_source: source_of("thickness scale source"),
        tint_source: source_of("tint modulation source"),
        brightness_source: source_of("brightness scale source"),
        blend,
        nonlinear_tint,
        sort_bias,
        image,
    })
}

/// The lightning among `object`'s widgets.
pub(in crate::app::model_preview) fn read_ce_lightning(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
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
        .filter_map(|(_, path)| read_lightning(reader, &path, owner, markers))
        .collect()
}



struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = lcg_step(self.0);
        high24_over_2_24(self.0)
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
        view: &ViewFrame,
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
                let across = nonzero_unit(cross(along, view.forward)).unwrap_or([0.0, 0.0, 1.0]);
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
        view: &ViewFrame,
        start_u: f32,
        quads: &mut Vec<CeLightningQuad>,
    ) {
        let last = points.len() - 1;
        let step = 1.0 / points.len() as f32;
        let vertex = |at: usize| {
            let previous = points[at.saturating_sub(1)].place;
            let next = points[(at + 1).min(last)].place;
            let along: [f32; 3] = std::array::from_fn(|axis| next[axis] - previous[axis]);
            let across = nonzero_unit(cross(along, view.forward)).unwrap_or([0.0; 3]);
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
            let [near, far] = [near, far].map(|color| color.map(|channel| channel.clamp(0.0, 1.0)));
            quads.push(CeLightningQuad { corners: [top, next_top, bottom, next_bottom], colors: [near, far, near, far] });
        }
    }
}

/// A `glow` widget's marker, where the model has it.
#[derive(Debug, Clone, Copy)]
struct GlowMarker {
    node: usize,
    position: [f32; 3],
    /// Forward, left, up.
    axes: [[f32; 3]; 3],
}

/// A marker posed for this frame: place, forward and up.
#[derive(Debug, Clone, Copy)]
struct PosedMarker {
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
}

/// A value an `attachment` may drive: the tag's value, then the multipliers
/// the function lerps between.
#[derive(Debug, Clone, Copy)]
struct Driven {
    source: Option<usize>,
    value: f32,
    low: f32,
    high: f32,
}

impl Driven {
    /// `object_get_function_value`, lerped as `glow_update` does.
    fn value(&self, outputs: &[f32; 4]) -> f32 {
        match self.source {
            Some(slot) => ((self.high - self.low) * outputs[slot] + self.low) * self.value,
            None => self.value,
        }
    }
}

/// A `glow` (`glw!`): particles spiralling along a chain of up to five
/// markers of one name, and particles shed from them. Read from the Xbox 360
/// build's `glow_new`, `glow_update` and `glow_render`; tool's postprocess
/// (`sub_525190`) only moves the `attachment`s down by one.
#[derive(Debug, Clone)]
pub(crate) struct CeGlow {
    pub(crate) object: u8,
    /// `object_get_marker_by_name` with room for five.
    markers: Vec<GlowMarker>,
    particle_count: usize,
    boundary: Option<Boundary>,
    normal_distribution: Option<NormalDistribution>,
    trailing_distribution: Option<TrailingDistribution>,
    flags: GlowFlags,
    rotation: Driven,
    translation: Driven,
    /// Distance bounds, lerped between by the function's multipliers.
    distance_source: Option<usize>,
    distance: [f32; 2],
    distance_multiplier: [f32; 2],
    size_source: Option<usize>,
    size: [f32; 2],
    color_source: Option<usize>,
    /// Red, green and blue of the two bounds.
    colors: [[f32; 3]; 2],
    color_rate: f32,
    fading: f32,
    generation: f32,
    lifetime: f32,
    trailing_velocity: f32,
    trailing_t: [f32; 2],
    /// `bitmap_dimension`: the first sprite's width in pixels.
    dimension: f32,
    /// The first sprite: left, right, top, bottom, its registration point,
    /// and the width of the image it is cut from.
    rect: [f32; 4],
    registration: [f32; 2],
    image_width: f32,
    pub(crate) image: Arc<H2Image>,
}

/// The `n`th field named `name`, as an integer.
fn read_bounds(root: TagStruct<'_>, name: &str) -> [f32; 2] {
    match root.field(name).and_then(|field| field.value()) {
        Some(blam_tags::TagFieldData::RealBounds(bounds)) => [bounds.lower, bounds.upper],
        _ => [0.0; 2],
    }
}

fn read_glow(reader: &TagReader, path: &str, owner: u8, markers: &[RenderModelPreviewMarker]) -> Option<CeGlow> {
    let tag = reader.load(path, "glow", b"glw!").ok()?;
    let root = tag.root();
    let name = root.read_string("attachment marker").unwrap_or_default();
    // What is not found is the origin.
    let mut found: Vec<GlowMarker> = markers
        .iter()
        .filter(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(&name))
        .take(5)
        .map(|marker| GlowMarker { node: marker.node_index.max(0) as usize, position: marker.position, axes: marker.axes })
        .collect();
    if found.is_empty() {
        found.push(GlowMarker { node: 0, position: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] });
    }
    // `glow_new`: only a bitmap of sprites makes one.
    let bitmap_path = root.read_tag_ref_path("texture").filter(|path| !path.is_empty())?;
    let bitmap = reader.load(&bitmap_path, "bitmap", b"bitm").ok()?;
    if !bitmap.root().field("type").is_some_and(|field| field.enum_is("sprites")) {
        return None;
    }
    let sequence = *elements(bitmap.root(), "sequences").first()?;
    let sprite = *elements(sequence, "sprites").first()?;
    let real_of = |holder: TagStruct<'_>, name: &str| holder.read_real(name).unwrap_or(0.0);
    let rect = [real_of(sprite, "left"), real_of(sprite, "right"), real_of(sprite, "top"), real_of(sprite, "bottom")];
    let registration = sprite.read_point2d("registration point");
    let index = sprite.read_int_any("bitmap index").unwrap_or(0).max(0) as usize;
    let image_width = elements(bitmap.root(), "bitmaps")
        .get(index)
        .map_or(1.0, |image| image.read_int_any("width").unwrap_or(1).max(1) as f32);
    let dimension = ((rect[1] - rect[0]) * image_width).trunc();
    let image = Arc::new(decode_h2_image(reader, &bitmap_path, SamplerKind::Texture2D, index)?);
    let real = |name: &str| root.read_real(name).unwrap_or(0.0);
    let attachment = |n: usize| function_slot(root.fields().filter(|field| field.name() == "attachment").nth(n));
    let [color_low, color_high] = [argb(root, "color_bound_0"), argb(root, "color_bound_1")];
    Some(CeGlow {
        object: owner,
        markers: found,
        particle_count: root.read_int_any("number of particles").unwrap_or(0).max(0) as usize,
        boundary: glow_option(root, "boundary effect", &[("bounce", Boundary::Bounce), ("wrap", Boundary::Wrap)]),
        normal_distribution: glow_option(
            root,
            "normal particle distribution",
            &[("distributed randomly", NormalDistribution::Randomly), ("distributed uniformly", NormalDistribution::Uniformly)],
        ),
        trailing_distribution: glow_option(
            root,
            "trailing particle distribution",
            &[
                ("emit vertically", TrailingDistribution::Vertically),
                ("emit normal (up)", TrailingDistribution::Normal),
                ("emit randomly", TrailingDistribution::Randomly),
            ],
        ),
        flags: GlowFlags::read(root),
        // The first `attachment`, of the particles' own rotation, is read by nothing.
        rotation: Driven {
            source: attachment(1),
            value: real("effect rotational velocity"),
            low: real("effect rot vel mul low"),
            high: real("effect rot vel mul high"),
        },
        translation: Driven {
            source: attachment(2),
            value: real("effect translational velocity"),
            low: real("effect trans vel mul low"),
            high: real("effect trans vel mul high"),
        },
        distance_source: attachment(3),
        distance: [real("min distance particle to object"), real("max distance particle to object")],
        distance_multiplier: [real("distance to object mul low"), real("distance to object mul high")],
        size_source: attachment(4),
        size: read_bounds(root, "particle size bounds"),
        color_source: attachment(5),
        colors: [[color_low[0], color_low[1], color_low[2]], [color_high[0], color_high[1], color_high[2]]],
        color_rate: real("color rate of change"),
        fading: real("fading percentage of glow"),
        generation: real("particle generation freq"),
        lifetime: real("lifetime of trailing particles"),
        trailing_velocity: real("velocity of trailing particles"),
        trailing_t: [real("trailing particle minimum t"), real("trailing particle maximum t")],
        dimension,
        rect,
        registration: [registration.x, registration.y],
        image_width,
        image,
    })
}

/// The glows among `object`'s widgets.
pub(in crate::app::model_preview) fn read_ce_glows(reader: &TagReader, object: &TagFile, owner: u8, markers: &[RenderModelPreviewMarker]) -> Vec<CeGlow> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let group = u32::from_be_bytes(*b"glw!");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("reference"))
        .filter(|(kind, path)| *kind == group && !path.is_empty())
        .filter_map(|(_, path)| read_glow(reader, &path, owner, markers))
        .collect()
}

/// A glow's `glow flags`, by name.
#[derive(Debug, Clone, Copy)]
struct GlowFlags {
    modify_color_by_t: bool,
    reverse: bool,
    alternate: bool,
    trailing_fade: bool,
    trailing_shrink: bool,
    trailing_slow: bool,
}

impl GlowFlags {
    fn read(root: TagStruct<'_>) -> Self {
        let set = |option: &str| has_flag(root, "glow flags", option);
        Self {
            modify_color_by_t: set("modify particle color in range"),
            reverse: set("particles move backwards"),
            alternate: set("particles move in both directions"),
            trailing_fade: set("trailing particles fade over time"),
            trailing_shrink: set("trailing particles shrink over time"),
            trailing_slow: set("trailing particles slow over time"),
        }
    }
}

/// `boundary effect`: what a particle does at the end of the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Bounce,
    Wrap,
}

/// `normal particle distribution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NormalDistribution {
    Randomly,
    Uniformly,
}

/// `trailing particle distribution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrailingDistribution {
    Vertically,
    Normal,
    Randomly,
}

/// The option `field` holds among `names`; none for a number the list does
/// not name, which the glow's switches pass over.
fn glow_option<T: Copy>(root: TagStruct<'_>, field: &str, names: &[(&str, T)]) -> Option<T> {
    let held = root.read_enum_option(field)?;
    names.iter().find(|(name, _)| *name == held).map(|(_, value)| *value)
}

/// A `glow_particle`.
#[derive(Debug, Clone, Copy, Default)]
struct GlowParticle {
    trailing: bool,
    /// Flag 1: travelling toward the start.
    reverse: bool,
    parent: usize,
    initial_angle: f32,
    color: [f32; 3],
    distance: f32,
    initial_size: f32,
    present_size: f32,
    t: f32,
    position: [f32; 3],
    initial_velocity: [f32; 3],
    ticks: i32,
    lifetime: i32,
    fade: f32,
}

/// A glow as `glow_datum` keeps it.
#[derive(Debug, Clone, Default)]
struct RunningGlow {
    initialized: bool,
    order: [usize; 5],
    times: [f32; 5],
    total: f32,
    particles: Vec<GlowParticle>,
    accumulated_ticks: i32,
    random: Lcg,
}

/// The glows of the opened object, as they move.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeGlowField {
    time: Option<f32>,
    running: Vec<RunningGlow>,
}

impl Default for Lcg {
    fn default() -> Self {
        Lcg(1)
    }
}

impl std::fmt::Debug for Lcg {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Lcg").field(&self.0).finish()
    }
}

impl Clone for Lcg {
    fn clone(&self) -> Self {
        Lcg(self.0)
    }
}

impl Lcg {
    /// `real_seed_random_range`.
    fn range(&mut self, lower: f32, upper: f32) -> f32 {
        lower + (upper - lower) * self.next()
    }
}

/// `nonuniform_cubic_spline_vector3d`: Newton's divided differences through
/// four points at four times.
fn nonuniform_cubic(f: [[f32; 3]; 4], times: [f32; 4], t: f32) -> [f32; 3] {
    let [t0, t1, t2, t3] = times;
    std::array::from_fn(|axis| {
        let d01 = (f[1][axis] - f[0][axis]) / (t1 - t0);
        let d12 = (f[2][axis] - f[1][axis]) / (t2 - t1);
        let d23 = (f[3][axis] - f[2][axis]) / (t3 - t2);
        let d012 = (d12 - d01) / (t2 - t0);
        let d123 = (d23 - d12) / (t3 - t1);
        let d0123 = (d123 - d012) / (t3 - t0);
        ((d0123 * (t - t2) + d012) * (t - t1) + d01) * (t - t0) + f[0][axis]
    })
}

impl RunningGlow {
    /// `get_particle_world_position`, for four or more markers: the spline
    /// through the four markers nearest `t`, swung about it by the angle.
    /// Two or three markers take a path in the game that reads curves it
    /// never fills in, so those glows are left out.
    fn world_position(&self, markers: &[PosedMarker], particle: &mut GlowParticle, rotation_per_unit_t: f32) -> bool {
        let count = markers.len();
        let last = count - 1;
        let t = particle.t;
        let parent = (0..last).find(|&at| self.times[at] <= t && self.times[at + 1] > t).unwrap_or(last);
        particle.parent = parent;
        if count < 4 {
            return false;
        }
        // Within range of the four the window takes; `t` past the ends would
        // read a fifth marker that is not there.
        let segment = (0..last).find(|&at| self.times[at] <= t && t <= self.times[at + 1]).unwrap_or(last).min(last - 1);
        let (mut low, mut high) = (segment, segment + 1);
        while high - low + 1 < 4 {
            low = low.saturating_sub(1);
            if high < last {
                high += 1;
            }
        }
        let picked: [PosedMarker; 4] = std::array::from_fn(|at| markers[self.order[low + at]]);
        let times: [f32; 4] = std::array::from_fn(|at| self.times[low + at]);
        let place = nonuniform_cubic(picked.map(|marker| marker.position), times, t);
        let up = nonuniform_cubic(picked.map(|marker| marker.up), times, t);
        let left = nonuniform_cubic(picked.map(|marker| cross(marker.up, marker.forward)), times, t);
        let angle = t * rotation_per_unit_t + particle.initial_angle;
        let (sine, cosine) = angle.sin_cos();
        particle.position = std::array::from_fn(|axis| place[axis] + (left[axis] * cosine + up[axis] * sine) * particle.distance);
        true
    }

    /// `glow_update`'s first pass: each marker's next is the one its forward
    /// points at most, and the chain is read back from the one with none.
    /// No more than `room` particles are made, as the shared array allows.
    fn initialize(&mut self, glow: &CeGlow, markers: &[PosedMarker], room: usize) {
        let count = markers.len();
        let mut next = [None; 5];
        for (at, marker) in markers.iter().enumerate() {
            let mut best = 0.0;
            for (other, candidate) in markers.iter().enumerate() {
                if other == at {
                    continue;
                }
                let mut toward: [f32; 3] = std::array::from_fn(|axis| candidate.position[axis] - marker.position[axis]);
                let length = dot(toward, toward).sqrt();
                if length.abs() >= 0.0001 {
                    toward = toward.map(|axis| axis / length);
                }
                let facing = dot(marker.forward, toward);
                if facing > best {
                    next[at] = Some(other);
                    best = facing;
                }
            }
        }
        let mut wanted = None;
        for slot in (0..count).rev() {
            let found = (0..count).rev().find(|&at| next[at] == wanted);
            if let Some(found) = found {
                self.order[slot] = found;
            }
            wanted = found;
        }
        self.times[0] = 0.0;
        self.total = 0.0;
        for at in 0..count - 1 {
            let [from, to] = [markers[self.order[at]].position, markers[self.order[at + 1]].position];
            let step: [f32; 3] = std::array::from_fn(|axis| to[axis] - from[axis]);
            self.total += dot(step, step).sqrt();
            self.times[at + 1] = self.total;
        }
        // `glow_particles_initialize`.
        let mut forward = true;
        for index in 0..glow.particle_count.min(room) {
            let mut particle = self.normal_particle(glow, index);
            if glow.flags.reverse {
                particle.reverse = true;
            }
            if glow.flags.alternate {
                particle.reverse = !forward;
                forward = !forward;
            }
            self.particles.push(particle);
        }
        self.initialized = true;
        self.accumulated_ticks = 0;
    }

    /// `glow_normal_particle_new`.
    fn normal_particle(&mut self, glow: &CeGlow, index: usize) -> GlowParticle {
        let random = &mut self.random;
        let mut particle = GlowParticle::default();
        if glow.distance_source.is_none() {
            particle.distance = random.range(glow.distance[0], glow.distance[1]);
        }
        if glow.size_source.is_none() {
            particle.initial_size = random.range(glow.size[0], glow.size[1]) / glow.dimension;
        }
        if glow.color_source.is_none() && !glow.flags.modify_color_by_t {
            let at = random.range(0.0, 1.0);
            particle.color = std::array::from_fn(|channel| (glow.colors[1][channel] - glow.colors[0][channel]) * at + glow.colors[0][channel]);
        }
        match glow.normal_distribution {
            Some(NormalDistribution::Randomly) => {
                particle.t = random.range(0.0, self.total);
                particle.initial_angle = random.range(0.0, std::f32::consts::TAU);
            }
            Some(NormalDistribution::Uniformly) => {
                particle.t = index as f32 / glow.particle_count as f32 * self.total;
                particle.initial_angle = random.range(0.0, std::f32::consts::TAU);
            }
            None => {}
        }
        particle
    }

    /// `glow_normal_particle_update_position`.
    fn move_normal(&self, glow: &CeGlow, outputs: &[f32; 4], markers: &[PosedMarker], particle: &mut GlowParticle, step: f32, rotation: f32) {
        if let Some(slot) = glow.distance_source {
            let [low, high] = glow.distance_multiplier;
            particle.distance = ((high - low) * outputs[slot] + low) * (glow.distance[1] - glow.distance[0]) + glow.distance[0];
        }
        let total = self.total;
        if particle.reverse {
            particle.t -= step;
            if particle.t < 0.0 && total > 0.0 {
                match glow.boundary {
                    Some(Boundary::Bounce) => {
                        while particle.t < 0.0 {
                            particle.t += total;
                        }
                        particle.reverse = false;
                        particle.t = total - particle.t;
                    }
                    Some(Boundary::Wrap) => {
                        while particle.t < 0.0 {
                            particle.t += total;
                        }
                    }
                    None => {}
                }
            }
        } else {
            particle.t += step;
            if particle.t > total && total > 0.0 {
                match glow.boundary {
                    Some(Boundary::Bounce) => {
                        while particle.t > total {
                            particle.t -= total;
                        }
                        particle.reverse = true;
                        particle.t = total - particle.t;
                    }
                    Some(Boundary::Wrap) => {
                        while particle.t > total {
                            particle.t -= total;
                        }
                    }
                    None => {}
                }
            }
        }
        self.world_position(markers, particle, rotation);
    }

    /// `glow_normal_particle_update_color`: its color, then the fade at
    /// either end of the chain.
    fn color_normal(&self, glow: &CeGlow, outputs: &[f32; 4], particle: &mut GlowParticle) {
        let [low, high] = glow.colors;
        if let Some(slot) = glow.color_source {
            particle.color = std::array::from_fn(|channel| (high[channel] - low[channel]) * outputs[slot] + low[channel]);
        }
        if glow.flags.modify_color_by_t {
            particle.color = std::array::from_fn(|channel| (high[channel] - low[channel]) * glow.color_rate * particle.t + low[channel]);
        }
        let along = particle.t / self.total;
        let edge = glow.fading * 0.5;
        particle.fade = if along < edge {
            along / edge
        } else if along <= 1.0 - edge {
            1.0
        } else {
            (1.0 - along) / edge
        };
        particle.fade = if particle.fade >= 0.0 { particle.fade.min(1.0) } else { 0.0 };
    }

    /// `glow_trailing_particle_new`.
    fn trailing_particle(&mut self, glow: &CeGlow, markers: &[PosedMarker]) -> GlowParticle {
        let mut particle = GlowParticle { trailing: true, ..GlowParticle::default() };
        if markers.len() <= 1 {
            particle.position = markers[0].position;
        } else {
            particle.t = self.random.range(glow.trailing_t[0] * self.total, glow.trailing_t[1] * self.total);
            self.world_position(markers, &mut particle, 0.0);
        }
        particle.initial_velocity = match glow.trailing_distribution {
            Some(TrailingDistribution::Vertically) => [0.0, 0.0, 1.0],
            Some(TrailingDistribution::Normal) => markers[particle.parent].up,
            Some(TrailingDistribution::Randomly) => {
                let random = &mut self.random;
                let direction = [random.range(-1.0, 1.0), random.range(-1.0, 1.0), random.range(-1.0, 1.0)];
                nonzero_unit(direction).unwrap_or(direction)
            }
            None => [0.0; 3],
        };
        let speed = glow.trailing_velocity * (1.0 / 30.0);
        particle.initial_velocity = particle.initial_velocity.map(|axis| axis * speed);
        particle.initial_size = self.random.range(glow.size[0], glow.size[1]) / glow.dimension;
        particle.lifetime = (glow.lifetime * 30.0) as i32;
        let at = self.random.range(0.0, 1.0);
        particle.color = std::array::from_fn(|channel| (glow.colors[1][channel] - glow.colors[0][channel]) * at + glow.colors[0][channel]);
        particle
    }
}

impl CeGlow {
    fn posed(&self, bones: Option<&[[f32; 4]]>) -> Vec<PosedMarker> {
        self.markers
            .iter()
            .map(|marker| {
                let (position, forward) = posed_on(marker.node, marker.position, marker.axes[0], bones);
                let (_, up) = posed_on(marker.node, marker.position, marker.axes[2], bones);
                PosedMarker { position, forward, up }
            })
            .collect()
    }
}

/// `glow_initialize`'s `glow particles` array, which every glow shares.
const MOST_GLOW_PARTICLES: usize = 512;

impl CeGlowField {
    /// `glow_update` for each glow, up to `time`.
    pub(crate) fn advance(&mut self, glows: &[CeGlow], outputs_of: impl Fn(u8) -> [f32; 4], bones: Option<&[[f32; 4]]>, time: f32) {
        if self.running.len() != glows.len() || self.time.is_none_or(|last| time < last) {
            self.running = (0..glows.len())
                .map(|at| RunningGlow { random: Lcg(at as u32 * 977 + 1), ..RunningGlow::default() })
                .collect();
            self.time = Some(time);
        }
        let last = resume_point(self.time.unwrap_or(time), time, 0.0);
        let seconds = (time - last).max(0.0);
        // `game_time_get_elapsed`, in ticks.
        let ticks = ((time * 30.0).floor() - (last * 30.0).floor()).max(0.0) as i32;
        self.time = Some(time);
        // `glow_initialize`'s array, shared by every glow's particles.
        let mut used: usize = self.running.iter().map(|running| running.particles.len()).sum();
        for (glow, running) in glows.iter().zip(&mut self.running) {
            let outputs = &outputs_of(glow.object);
            let markers = glow.posed(bones);
            let count = markers.len();
            if (2..4).contains(&count) {
                continue;
            }
            used -= running.particles.len();
            // The game orders the markers and makes the particles on one
            // update, draws them unplaced, and places them on the next; here
            // they are placed at once.
            if !running.initialized && count > 1 {
                running.initialize(glow, &markers, MOST_GLOW_PARTICLES.saturating_sub(used));
            }
            let mut velocity = 0.0;
            let mut rotation = 0.0;
            if count > 1 {
                velocity = glow.translation.value(outputs);
                rotation = glow.rotation.value(outputs) / velocity;
            }
            running.accumulated_ticks += ticks;
            let mut particles = std::mem::take(&mut running.particles);
            if count > 1 {
                for particle in particles.iter_mut().filter(|particle| !particle.trailing) {
                    running.move_normal(glow, outputs, &markers, particle, seconds * velocity, rotation);
                    running.color_normal(glow, outputs, particle);
                    particle.present_size = particle.initial_size;
                }
            }
            particles.retain_mut(|particle| {
                if !particle.trailing {
                    return true;
                }
                particle.ticks += ticks;
                let left = (1.0 - particle.ticks as f32 / particle.lifetime as f32).max(0.0);
                particle.fade = if glow.flags.trailing_fade {
                    (1.0 - particle.ticks as f32 / particle.lifetime as f32).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if glow.flags.trailing_shrink {
                    particle.present_size = left * particle.initial_size;
                }
                let velocity = if glow.flags.trailing_slow {
                    particle.initial_velocity.map(|axis| axis * left)
                } else {
                    particle.initial_velocity
                };
                particle.position = std::array::from_fn(|axis| particle.position[axis] + velocity[axis] * seconds);
                particle.ticks <= particle.lifetime
            });
            running.particles = particles;
            if glow.generation > 0.01 && ticks > 0 {
                let period = (30.0 / glow.generation).max(1.0);
                while running.accumulated_ticks as f32 > period {
                    if used + running.particles.len() < MOST_GLOW_PARTICLES {
                        let particle = running.trailing_particle(glow, &markers);
                        running.particles.push(particle);
                    }
                    running.accumulated_ticks -= period as i32;
                }
            }
            used += running.particles.len();
        }
    }

    /// `glow_render`: each particle a screen-facing sprite of the first
    /// sprite of the first sequence, through `global_shader_effect_additive`.
    /// A size of zero is drawn a texel to a pixel, as `build_sprite` does,
    /// which `pixel` gives the world size of at a place.
    pub(crate) fn quads(&self, glows: &[CeGlow], view: &ViewFrame, pixel: impl Fn([f32; 3]) -> f32) -> Vec<(usize, CeLightningQuad)> {
        let mut quads = Vec::new();
        for (index, (glow, running)) in glows.iter().zip(&self.running).enumerate() {
            let [left, right, top, bottom] = glow.rect;
            let center_uv = [left + glow.registration[0], top + glow.registration[1]];
            for particle in &running.particles {
                let scale = if particle.present_size == 0.0 { pixel(particle.position) } else { particle.present_size };
                let size = glow.image_width * scale;
                // `glow_update` divides by the tag's values unguarded; one with
                // a zero there goes NaN in the game too. Nothing to draw.
                if !(size.is_finite() && particle.position.iter().all(|axis| axis.is_finite())) {
                    continue;
                }
                let corner = |u: f32, v: f32| {
                    let x = u - center_uv[0];
                    let y = center_uv[1] - v;
                    let place: [f32; 3] =
                        std::array::from_fn(|axis| particle.position[axis] + (view.right[axis] * x + view.up[axis] * y) * size);
                    [place[0], place[1], place[2], u, v]
                };
                let [red, green, blue] = particle.color;
                quads.push((
                    index,
                    CeLightningQuad {
                        corners: [corner(left, bottom), corner(right, bottom), corner(left, top), corner(right, top)],
                        colors: [[red, green, blue, particle.fade].map(|channel| channel.clamp(0.0, 1.0)); 4],
                    },
                ));
            }
        }
        quads
    }
}
