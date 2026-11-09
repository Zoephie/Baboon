//! Lightning widgets: their markers, bolts and quads.

use super::*;

impl super::super::functions::ReadsOutputs for CeLightning {
    fn object(&self) -> u8 {
        self.object
    }
    /// Its tint source names a change color, not an output.
    fn outputs_read(&self) -> Vec<usize> {
        [self.jitter_source, self.thickness_source, self.brightness_source].into_iter().flatten().collect()
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
    pub(crate) image: Arc<Image>,
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
    let image = Arc::new(decode_image(reader, &bitmap, SamplerKind::Texture2D, 0)?);
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

pub(super) struct Lcg(pub(super) CeRandom);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0.next()
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
        let mut random = Lcg(CeRandom(seed | 1));
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

impl Default for Lcg {
    fn default() -> Self {
        Lcg(CeRandom(1))
    }
}

impl std::fmt::Debug for Lcg {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Lcg").field(&self.0.0).finish()
    }
}

impl Clone for Lcg {
    fn clone(&self) -> Self {
        Lcg(self.0)
    }
}

impl Lcg {
    /// `real_seed_random_range`.
    pub(super) fn range(&mut self, lower: f32, upper: f32) -> f32 {
        lower + (upper - lower) * self.next()
    }
}
