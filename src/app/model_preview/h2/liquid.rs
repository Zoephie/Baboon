//! Halo 2's `liquid`: a beam of sprites along a marker's forward, thrown about
//! by octaves of noise that only time moves. A preview has no projectile to
//! reach, so of the tag's types only `standard` shows, and only when an effect
//! attachment of the object is running it.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::ce::attachments::{elements, object_block};
use super::super::ce::lights::{CeAntennaQuad, CeFlareView, posed_on};
use super::functions::H2FunctionValues;
use super::light_volumes::{read_glow, read_h2_bitmap};
use super::lights::attachment_value;
use super::super::dx9::passes::{H2Image, tag_function_at};
use super::super::gen3::effects::marker_name;
use super::super::*;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::{cross, dot};
use crate::app::model_preview::common::random::{high16_times_inverse_65535, lcg_step};
use crate::app::model_preview::common::attached::attached;
use crate::app::model_preview::ce::attachments::has_flag;

const OCTAVES: usize = 9;
/// `liquid_arc_state_new`'s random starts, which never need to be the game's.
const STARTS: [u32; OCTAVES] = [40_503, 11_213, 52_711, 7_919, 31_337, 60_493, 2_741, 18_947, 44_497];

/// One of an arc's `cores`.
#[derive(Debug, Clone)]
struct LiquidCore {
    thickness: TagFunction,
    color: TagFunction,
    time: TagFunction,
    facing: TagFunction,
    along: TagFunction,
}

/// One of a `liquid`'s `arcs`.
#[derive(Debug, Clone)]
pub(crate) struct LiquidArc {
    /// `4 << sprite count`.
    sprites: usize,
    relative_up: bool,
    length: f32,
    instances: usize,
    spread: f32,
    rotation_period: f32,
    horizontal: TagFunction,
    vertical: TagFunction,
    negative: f32,
    roughness: TagFunction,
    frequencies: [f32; OCTAVES],
    octaves: [bool; OCTAVES],
    range_scale: TagFunction,
    brightness_scale: TagFunction,
    cores: Vec<LiquidCore>,
    bitmap: Option<Arc<H2Image>>,
}

/// A `liquid` at a marker of the object.
#[derive(Debug, Clone)]
pub(crate) struct H2LiquidAt {
    pub(crate) object: u8,
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    scale_by: String,
    falloff: [f32; 2],
    arcs: Arc<Vec<LiquidArc>>,
}

fn function_of(holder: TagStruct<'_>, name: &str) -> Option<TagFunction> {
    tag_function_at(&holder.descend(name)?, "function")
}

fn read_arc(source: &TagSource, arc: TagStruct<'_>, glow: Option<&Arc<H2Image>>) -> Option<LiquidArc> {
    let sprites = arc.read_enum_option("sprite count")?.split(' ').next()?.parse().ok()?;
    let cores = elements(arc, "cores").into_iter().filter_map(|core| {
        Some(LiquidCore {
            thickness: function_of(core, "thickness")?,
            color: function_of(core, "color")?,
            time: function_of(core, "brightness-time")?,
            facing: function_of(core, "brightness-facing")?,
            along: function_of(core, "along-axis scale")?,
        })
    });
    let own = arc.read_tag_ref_path("bitmap").filter(|path| !path.is_empty());
    let bitmap = own.and_then(|path| read_h2_bitmap(source, &path)).or_else(|| glow.cloned());
    Some(LiquidArc {
        sprites,
        relative_up: has_flag(arc, "flags", "basis marker-relative"),
        length: arc.read_real("natural length")?,
        instances: arc.read_int_any("instances")?.clamp(0, 8) as usize,
        spread: arc.read_real("instance spread angle")?,
        rotation_period: arc.read_real("instance rotation period")?,
        horizontal: function_of(arc, "horizontal range")?,
        vertical: function_of(arc, "vertical range")?,
        negative: arc.read_real("vertical negative scale")?,
        roughness: function_of(arc, "roughness")?,
        frequencies: std::array::from_fn(|octave| {
            arc.read_real(&format!("octave {} frequency", octave + 1)).unwrap_or(0.0)
        }),
        octaves: std::array::from_fn(|octave| has_flag(arc, "octave flags", &format!("octave {}", octave + 1))),
        range_scale: function_of(arc, "range-scale")?,
        brightness_scale: function_of(arc, "brightness-scale")?,
        cores: cores.collect(),
        bitmap,
    })
}

/// The attachment whose effect has a part of this `liquid`, and so runs it:
/// `standard` ones start at nothing until `liquid_set_scale`.
fn driver_of(source: &TagSource, object: &TagFile, liquid: &str) -> Option<String> {
    let block = object_block(object, "attachments")?;
    let group = u32::from_be_bytes(*b"effe");
    let wanted = u32::from_be_bytes(*b"tdtl");
    elements(block, "attachments").into_iter().find_map(|attachment| {
        let (kind, path) = attachment.read_tag_ref_with_group("type")?;
        if kind != group || path.is_empty() {
            return None;
        }
        let effect = load_referenced_tag_from_source(source, &path, "effect", b"effe").ok()?;
        let events = elements(effect.root(), "events");
        let found = events.iter().flat_map(|event| elements(*event, "parts")).any(|part| {
            part.read_tag_ref_with_group("type")
                .is_some_and(|(kind, path)| kind == wanted && path.eq_ignore_ascii_case(liquid))
        });
        found.then(|| attachment.read_string_id("primary scale").unwrap_or_default())
    })
}

/// `liquid_new` for an object's `widgets`.
pub(in crate::app::model_preview) fn read_h2_liquids(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H2LiquidAt> {
    let group = u32::from_be_bytes(*b"tdtl");
    let mut found = Vec::new();
    let Some(block) = object_block(object, "widgets") else {
        return found;
    };
    let widgets = elements(block, "widgets");
    let paths: Vec<String> = widgets
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("type"))
        .filter_map(|(kind, path)| (kind == group && !path.is_empty()).then_some(path))
        .collect();
    if paths.is_empty() {
        return found;
    }
    let glow = read_glow(source);
    for path in paths {
        let Ok(tag) = load_referenced_tag_from_source(source, &path, "liquid", b"tdtl") else {
            continue;
        };
        let root = tag.root();
        if root.read_enum_option("type").as_deref() != Some("standard") {
            continue;
        }
        let Some(scale_by) = driver_of(source, object, &path) else {
            continue;
        };
        let name = marker_name(root, "attachment marker name");
        // `object_get_markers_by_string_id` for one.
        let Some(marker) = markers.iter().find(|marker| marker.name.eq_ignore_ascii_case(&name)) else {
            continue;
        };
        let arcs = elements(root, "arcs").into_iter().filter_map(|arc| read_arc(source, arc, glow.as_ref())).collect();
        let (place, node) = frame(marker);
        found.push(H2LiquidAt {
            object: owner,
            only_under: marker.only_under.clone(),
            node,
            position: place.position,
            forward: place.axes[0],
            up: place.axes[2],
            scale_by,
            falloff: [
                root.read_real("falloff distance from camera").unwrap_or(0.0),
                root.read_real("cutoff distance from camera").unwrap_or(0.0),
            ],
            arcs: Arc::new(arcs),
        });
    }
    found
}

fn scrambled(seed: u32) -> impl FnMut() -> f32 {
    let mut seed = seed;
    move || {
        seed = lcg_step(seed);
        high16_times_inverse_65535(seed) * 2.0 - 1.0
    }
}

impl LiquidArc {
    /// `liquid_build_arc`'s noise: where each of `sprites + 1` points is thrown,
    /// sideways and up, as `-1..1`. The nine octaves' seeds step on with
    /// `frequency * time` and each blends across three of them.
    fn noise(&self, instance: usize, time: f32) -> Vec<[f32; 2]> {
        let count = self.sprites;
        let octaves = (count.trailing_zeros() as usize).min(OCTAVES);
        let mut sums = vec![[0.0_f32; 2]; count + 1];
        let mut weights = vec![0.0_f32; count + 1];
        for octave in (0..octaves).filter(|octave| self.octaves[*octave]) {
            let step = count >> octave;
            let moved = self.frequencies[octave].max(0.0) * time;
            let t = moved.fract();
            let seed = (instance as u32) << 8;
            let seed = seed.wrapping_add(STARTS[octave]).wrapping_add(moved as u32).wrapping_add(octave as u32 * 3);
            let shares = [0.5 - t + t * t * 0.5, t + 0.5 - t * t, t * t * 0.5];
            let mut layer = vec![[0.0_f32; 2]; count + 1];
            for (at, share) in shares.into_iter().enumerate() {
                let mut random = scrambled(seed.wrapping_add(at as u32));
                for knot in (0..=count).step_by(step) {
                    layer[knot][0] += random() * share;
                    layer[knot][1] += random() * share;
                }
            }
            if step > 1 {
                let half = step / 2;
                for middle in (half..=count).step_by(step) {
                    layer[middle] = std::array::from_fn(|axis| (layer[middle - half][axis] + layer[middle + half][axis]) * 0.5);
                }
            }
            // A curve from each middle to the next, with the knot as its handle.
            if step > 2 {
                let half = step / 2;
                for knot in (0..=count).step_by(step) {
                    let before = if knot >= half { knot - half } else { knot };
                    let after = (knot + half).min(count);
                    let (start, handle, end) = (layer[before], layer[knot], layer[after]);
                    for at in (before + 1)..after {
                        let s = (at + half - knot) as f32 / step as f32;
                        let r = 1.0 - s;
                        layer[at] = std::array::from_fn(|axis| {
                            r * r * start[axis] + 2.0 * s * r * handle[axis] + s * s * end[axis]
                        });
                    }
                }
            }
            for at in 0..=count {
                let along = at as f32 / count as f32;
                let weight = 2.0_f32.powf(-(1.0 - self.roughness.evaluate(along, 0.0)) * octave as f32);
                sums[at] = std::array::from_fn(|axis| sums[at][axis] + layer[at][axis] * weight);
                weights[at] += weight;
            }
        }
        sums.iter()
            .zip(&weights)
            .map(|(sum, weight)| if *weight > 0.0 { sum.map(|axis| axis / weight) } else { [0.0; 2] })
            .collect()
    }

    pub(crate) fn bitmap(&self) -> Option<&Arc<H2Image>> {
        self.bitmap.as_ref()
    }
}



impl H2LiquidAt {
    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }



    pub(crate) fn arcs(&self) -> &[LiquidArc] {
        &self.arcs
    }

    /// `rasterizer_liquid_draw`, a sprite at each point but the last, turned
    /// along the line to the next.
    pub(crate) fn quads(
        &self,
        arc: &LiquidArc,
        values: Option<&H2FunctionValues>,
        bones: Option<&[[f32; 4]]>,
        view: &CeFlareView,
        time: f32,
    ) -> Vec<CeAntennaQuad> {
        // `sub_515280`: what an effect's part of no duration asks for is all
        // of it. `rasterizer_liquid_draw` keeps the scale as a byte and dims
        // the whole arc by it.
        let scale = attachment_value(&self.scale_by, values).clamp(0.0, 1.0);
        if scale <= 0.0 {
            return Vec::new();
        }
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let (_, marker_up) = posed_on(self.node, self.position, self.up, bones);
        let up = if arc.relative_up { marker_up } else { [0.0, 0.0, 1.0] };
        let right = cross(forward, up);
        let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - origin[axis]);
        let distance = dot(to_eye, to_eye).sqrt();
        let [falloff, cutoff] = self.falloff;
        let fade = if cutoff > falloff { ((cutoff - distance) / (cutoff - falloff)).clamp(0.0, 1.0) } else { 1.0 };
        let reach = arc.range_scale.evaluate(1.0, 0.0);
        let shine = arc.brightness_scale.evaluate(1.0, 0.0) * fade;
        let camera = cross(view.right, view.up);
        let facing = dot(forward, camera).abs();
        let last = arc.sprites;
        let mut quads = Vec::new();
        for instance in 0..arc.instances {
            let mut end: [f32; 3] = std::array::from_fn(|axis| origin[axis] + forward[axis] * arc.length);
            if arc.spread != 0.0 {
                let turn = if arc.rotation_period != 0.0 { time / arc.rotation_period } else { 0.0 };
                let angle = (turn + instance as f32 / arc.instances as f32) * std::f32::consts::TAU;
                let width = arc.spread.tan() * arc.length;
                end = std::array::from_fn(|axis| end[axis] + right[axis] * angle.cos() * width - up[axis] * angle.sin() * width);
            }
            let noise = arc.noise(instance, time);
            let points: Vec<[f32; 3]> = (0..=last)
                .map(|at| {
                    let along = at as f32 / last as f32;
                    let [side, lift] = noise[at];
                    let lift = if arc.negative == 1.0 {
                        lift
                    } else {
                        ((arc.negative + 1.0) * 0.5 + (1.0 - arc.negative) * 0.25 * lift) * lift + (1.0 - arc.negative) * 0.25
                    };
                    let side = arc.horizontal.evaluate(along, 0.0) * side * reach;
                    let lift = arc.vertical.evaluate(along, 0.0) * lift * reach;
                    std::array::from_fn(|axis| {
                        origin[axis] + (end[axis] - origin[axis]) * along + right[axis] * side + up[axis] * lift
                    })
                })
                .collect();
            for (index, core) in arc.cores.iter().enumerate() {
                let bright = shine * scale * core.time.evaluate(index as f32 + time, 0.0) * core.facing.evaluate(facing, 0.0);
                let alpha = bright.clamp(0.0, 1.0);
                if alpha <= 0.0 {
                    continue;
                }
                for at in 0..last {
                    let along = at as f32 / last as f32;
                    let thick = core.thickness.evaluate(along, 0.0);
                    let long = core.along.evaluate(along, 0.0).max(thick);
                    if thick <= 0.0 {
                        continue;
                    }
                    // `sub_94C450`: a segment with either end behind the eye
                    // draws nothing; the sprite sits halfway along it.
                    let behind = |point: [f32; 3]| {
                        dot(std::array::from_fn(|axis| point[axis] - view.eye[axis]), view.forward) <= 0.0
                    };
                    if behind(points[at]) || behind(points[at + 1]) {
                        continue;
                    }
                    let segment: [f32; 3] = std::array::from_fn(|axis| points[at + 1][axis] - points[at][axis]);
                    let (across, rise) = (dot(segment, view.right), dot(segment, view.up));
                    let length = (across * across + rise * rise).sqrt();
                    let (cos, sin) = if length > 0.0 { (across / length, rise / length) } else { (1.0, 0.0) };
                    let side: [f32; 3] = std::array::from_fn(|axis| view.right[axis] * cos + view.up[axis] * sin);
                    let lift: [f32; 3] = std::array::from_fn(|axis| view.up[axis] * cos - view.right[axis] * sin);
                    let center: [f32; 3] = std::array::from_fn(|axis| (points[at][axis] + points[at + 1][axis]) * 0.5);
                    let corner = |x: f32, y: f32, u: f32, v: f32| -> [f32; 5] {
                        let [a, b, c] = std::array::from_fn(|axis| center[axis] + side[axis] * x * long + lift[axis] * y * thick);
                        [a, b, c, u, v]
                    };
                    let color = core.color.evaluate_color(along, 0.0);
                    let tint = [color.red, color.green, color.blue].map(|channel| channel.clamp(0.0, 1.0) * alpha);
                    quads.push(CeAntennaQuad {
                        corners: [corner(-1.0, 1.0, 0.0, 0.0), corner(1.0, 1.0, 1.0, 0.0), corner(-1.0, -1.0, 0.0, 1.0), corner(1.0, -1.0, 1.0, 1.0)],
                        // `widget_light_volume`'s pixel shader: colour by its alpha by the
                    // map's colour, and an alpha of 0, so under `ONE, INV_SRC_ALPHA`
                    // it only adds light and never darkens what is behind.
                    color: [tint[0], tint[1], tint[2], 0.0],
                    });
                }
            }
        }
        quads
    }
}

attached!(keeps function: H2LiquidAt);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_moment_throws_the_same_way_and_a_later_one_does_not() {
        let mut random = scrambled(7);
        let first = random();
        assert_eq!(first, scrambled(7)());
        assert_ne!(first, random());
        assert!((-1.0..=1.0).contains(&first));
    }
}
