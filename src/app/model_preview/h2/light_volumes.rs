//! Halo 2's `light_volume`: sprites strung along a marker's forward, each a
//! quad facing the screen, stretched along the axis as the view turns side on.

use std::sync::Arc;

use blam_tags::{TagFile, TagFunction, TagStruct};

use super::super::attachment::MarkerFrame;
use super::super::common::fields::{elements, function_slot_of, has_flag, object_block};
use super::super::ce::lights::CeAntennaQuad;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::functions::FunctionValues;
use super::lights::H2EffectPart;
use super::super::gen3::effects::{for_each_attached_part, part_markers};
use super::super::dx9::passes::H2Image;
use super::super::common::named_function::tag_function_at;
use super::super::gen3::effects::marker_name;
use super::super::materials::decode_h2_image;
use super::super::*;
use crate::core::h2_pixel_shader::SamplerKind;
use crate::core::source::TagSource;
use crate::app::model_preview::common::math::dot;
use crate::app::model_preview::common::attached::attached;

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

/// Where a volume's sprites lie along it, as tool's postprocess decides.
#[derive(Debug, Clone, PartialEq)]
enum Spacing {
    /// Runtime flag 2: `index / (count - 1)`.
    Linear,
    /// Runtime flag 1: sprites growing geometrically between the radius
    /// function's ends, at the offsets a linear radius would give them.
    Geometric,
    /// Each sprite's offset at range 0 and 1, from tool's solver.
    Table(Vec<[f32; 2]>),
    /// No flag and no table: every sprite at offset 0.
    Nowhere,
}

/// One of a `light_volume`'s `volumes`.
#[derive(Debug, Clone)]
pub(crate) struct H2Volume {
    count: usize,
    fuzzy: bool,
    /// `scaled by marker`, read off the first volume for them all.
    scaled_by_marker: bool,
    /// `not scaled by event duration`, likewise: an effect's volume keeps
    /// its color over its event (`sub_515280`).
    not_scaled_by_duration: bool,
    spacing: Spacing,
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
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    falloff: [f32; 2],
    scale_by: String,
    /// What `render_submit_object_attachments` gives it: the marker's scale,
    /// for offsets and radii, if its first volume asks; and which change
    /// color tints it, if any.
    marker_scale: f32,
    change_color: Option<usize>,
    volumes: Arc<Vec<H2Volume>>,
    /// The effect part that makes it, if it is no attachment of its own.
    effect: Option<H2EffectPart>,
}

fn function_of(holder: TagStruct<'_>, name: &str) -> Option<TagFunction> {
    tag_function_at(&holder.descend(name)?, "function")
}

fn read_volume(source: &TagSource, volume: TagStruct<'_>, glow: Option<&Arc<H2Image>>) -> Option<H2Volume> {
    let flagged = |wanted: &str| has_flag(volume, "flags", wanted);
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
    let count = volume.read_int_any("sprite count")?.clamp(4, MOST_SPRITES as i128) as usize;
    let radius = function_of(volume, "radius function")?;
    let setting = |name: &str| {
        volume.read_real(&format!("{name}*")).or_else(|| volume.read_real(name)).unwrap_or(0.0)
    };
    let integer = |name: &str| {
        volume.read_int_any(&format!("{name}*")).or_else(|| volume.read_int_any(name)).unwrap_or(0)
    };
    let solver = Solver {
        min_fraction: setting("radius frac min").clamp(0.00390625, 1.0),
        spacing: integer("x-buffer spacing").clamp(1, 256) as usize,
        min_iterations: integer("x-buffer min iterations").clamp(1, 256) as usize,
        max_iterations: 0,
        tolerance: setting("x-delta max error").clamp(0.001, 0.1),
    };
    let solver = Solver {
        max_iterations: (integer("x-buffer max iterations") as usize).max(solver.min_iterations).min(256),
        ..solver
    };
    let forced = |wanted: &str| flagged(wanted);
    Some(H2Volume {
        count,
        fuzzy: flagged("fuzzy"),
        scaled_by_marker: flagged("scaled by marker"),
        not_scaled_by_duration: flagged("not scaled by event duration"),
        spacing: postprocessed_spacing(
            &radius,
            count,
            [forced("force linear radius function"), forced("force linear offset"), forced("force differential evaluation")],
            &solver,
        ),
        offset: function_of(volume, "offset function")?,
        radius,
        brightness: function_of(volume, "brightness function")?,
        color: function_of(volume, "color function")?,
        facing: function_of(volume, "facing function")?,
        aspect,
        bitmap,
    })
}

/// `globals`' `rasterizer data`, whose `glow` stands in for a volume's bitmap.
pub(in crate::app::model_preview) fn read_glow(source: &TagSource) -> Option<Arc<H2Image>> {
    let globals = load_referenced_tag_from_source(source, "globals\\globals", "globals", b"matg").ok()?;
    let data = elements(globals.root(), "rasterizer data");
    let path = data.first()?.read_tag_ref_path("glow").filter(|path| !path.is_empty())?;
    decode_h2_image(source, &path, SamplerKind::Texture2D, 0).map(Arc::new)
}

pub(in crate::app::model_preview) fn read_h2_bitmap(source: &TagSource, path: &str) -> Option<Arc<H2Image>> {
    decode_h2_image(source, path, SamplerKind::Texture2D, 0).map(Arc::new)
}

/// `render_submit_object_attachments`: one at every marker of the name. Then
/// each attached light's own `light volume`, which `light_submit_lens_flares`
/// submits while the light is on, at the light: its first marker, scale 1,
/// white, as bright as the light's scale (`sub_6BEDF0`). Then its effects'
/// `light_volume` parts, which `sub_515280` submits at every marker of their
/// location while their event runs: scale 1, white faded over the event, as
/// bright as the part's A and B scales.
pub(in crate::app::model_preview) fn read_h2_light_volumes(
    source: &TagSource,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H2LightVolume> {
    let (volume_group, light_group) = (u32::from_be_bytes(*b"MGS2"), u32::from_be_bytes(*b"ligh"));
    let mut found = Vec::new();
    let mut glow = None;
    let attachments = object_block(object, "attachments").map(|block| elements(block, "attachments"));
    for attachment in attachments.unwrap_or_default() {
        let Some((kind, path)) = attachment.read_tag_ref_with_group("type").filter(|(_, path)| !path.is_empty()) else {
            continue;
        };
        let (path, attached) = if kind == volume_group {
            (path, true)
        } else if kind == light_group {
            let Ok(light) = load_referenced_tag_from_source(source, &path, "light", b"ligh") else {
                continue;
            };
            let named = light.root().read_tag_ref_path("light volume").filter(|path| !path.is_empty());
            match named.filter(|_| super::lights::light_can_shine(light.root())) {
                Some(named) => (named, false),
                None => continue,
            }
        } else {
            continue;
        };
        let Ok(tag) = load_referenced_tag_from_source(source, &path, "light_volume", b"MGS2") else {
            continue;
        };
        let glow = glow.get_or_insert_with(|| read_glow(source));
        let root = tag.root();
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let volumes = elements(root, "volumes");
        let volumes: Vec<H2Volume> =
            volumes.iter().filter_map(|volume| read_volume(source, *volume, glow.as_ref())).collect();
        let scaled_by_marker = volumes.first().is_some_and(|volume| volume.scaled_by_marker);
        let volumes = Arc::new(volumes);
        let name = marker_name(attachment, "marker");
        // `sub_86F0D0`: none, or one of the four. A light's own volume is white.
        let change_color = function_slot_of(attachment, "change color").filter(|_| attached);
        let named = markers.iter().filter(|marker| marker.name.eq_ignore_ascii_case(&name));
        for marker in named.take(if attached { usize::MAX } else { 1 }) {
            let (place, node) = frame(marker);
            found.push(H2LightVolume {
                object: owner,
                only_under: marker.only_under.clone(),
                node,
                position: place.position,
                forward: place.axes[0],
                falloff: [real("falloff distance from camera"), real("cutoff distance from camera")],
                scale_by: attachment.read_string_id("primary scale").unwrap_or_default(),
                marker_scale: if attached && scaled_by_marker { marker.scale } else { 1.0 },
                change_color,
                volumes: Arc::clone(&volumes),
                effect: None,
            });
        }
    }
    for_each_attached_part(source, object, |part| {
        if part.group != volume_group {
            return;
        }
        let Ok(tag) = load_referenced_tag_from_source(source, &part.path, "light_volume", b"MGS2") else {
            return;
        };
        let glow = glow.get_or_insert_with(|| read_glow(source));
        let root = tag.root();
        let real = |name: &str| root.read_real(name).unwrap_or(0.0);
        let volumes = elements(root, "volumes");
        let volumes: Vec<H2Volume> =
            volumes.iter().filter_map(|volume| read_volume(source, *volume, glow.as_ref())).collect();
        let volumes = Arc::new(volumes);
        for marker in part_markers(markers, &part.marker, part.effect.once) {
            let (place, node) = frame(&marker);
            let (place, node) = part.placed(place, node);
            found.push(H2LightVolume {
                object: owner,
                only_under: marker.only_under.clone(),
                node,
                position: place.position,
                forward: place.axes[0],
                falloff: [real("falloff distance from camera"), real("cutoff distance from camera")],
                scale_by: part.effect.scale_by.clone(),
                marker_scale: 1.0,
                change_color: None,
                volumes: Arc::clone(&volumes),
                effect: Some(H2EffectPart::new(&part)),
            });
        }
    });
    found
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

/// The `x-buffer` settings of tool's offset solver.
#[derive(Debug, Clone, Copy)]
struct Solver {
    min_fraction: f32,
    spacing: usize,
    min_iterations: usize,
    max_iterations: usize,
    tolerance: f32,
}

/// Tool's `light_volume_volume_block` postprocess (`sub_B16260`), on the
/// radius function: `force differential evaluation` drops both forcing flags
/// and sets no runtime flag; else linear when forced or the radius cannot
/// space anything, geometric when forced or the radius is identity or linear.
/// The table is solved only when neither forcing flag stands.
fn postprocessed_spacing(
    radius: &TagFunction,
    count: usize,
    [linear_radius, linear_offset, differential]: [bool; 3],
    solver: &Solver,
) -> Spacing {
    let (linear_radius, linear_offset) = if differential { (false, false) } else { (linear_radius, linear_offset) };
    let h2 = radius.as_h2();
    let kind = h2.map(|h2| h2.function_type());
    let (low, high) = h2.map_or((0.0, 1.0), |h2| (h2.clamp_range_min(), h2.clamp_range_max()));
    use blam_tags::tag_function::FunctionType;
    let flag = if differential {
        None
    } else if linear_offset
        || kind == Some(FunctionType::Constant)
        || low < 0.0
        || high < 0.0
        || low == high
        || solver.min_fraction == 1.0
        || count == 0
    {
        Some(Spacing::Linear)
    } else if linear_radius || matches!(kind, Some(FunctionType::Identity | FunctionType::Linear)) {
        Some(Spacing::Geometric)
    } else {
        None
    };
    // The renderer takes the flags first, and the table only without them.
    flag.unwrap_or_else(|| {
        if linear_radius || linear_offset {
            Spacing::Nowhere
        } else {
            Spacing::Table(solve_offsets(radius, count, solver, (low, high)))
        }
    })
}

/// The solver of `sub_B16260`: samples start even; each pass sizes them by the
/// radius function (normalized, at least `min_fraction`), then moves each to
/// its share of the running sum, until the largest move against its own
/// radius is under the tolerance past the minimum passes, or out of passes.
/// Every `spacing`th sample is a sprite's offset, at range 0 and range 1.
fn solve_offsets(radius: &TagFunction, count: usize, solver: &Solver, (low, high): (f32, f32)) -> Vec<[f32; 2]> {
    let last = solver.spacing * (count - 1);
    let normal = if high > 0.0 || low > 0.0 { 1.0 / high.max(low) } else { 1.0 };
    let mut table = vec![[0.0; 2]; count];
    for range in 0..2 {
        let (mut x, mut r, mut sum) = (vec![0.0f32; last + 1], vec![0.0f32; last + 1], vec![0.0f32; last + 1]);
        for pass in 0..solver.max_iterations {
            let mut worst = 0.0f32;
            let mut done = false;
            for at in 0..=last {
                if pass == 0 {
                    x[at] = at as f32 / last as f32;
                    continue;
                }
                let moved = sum[at] / sum[last];
                let change = (moved - x[at]).abs();
                x[at] = moved;
                worst = worst.max(change / r[at]);
                let finished = pass + 1 == solver.max_iterations
                    || (pass + 1 >= solver.min_iterations && solver.tolerance >= worst);
                if at == last && finished {
                    for (sprite, entry) in table.iter_mut().enumerate() {
                        entry[range] = x[sprite * solver.spacing];
                    }
                    done = true;
                }
            }
            if done {
                break;
            }
            for at in 0..=last {
                let size = radius.evaluate(x[at], range as f32) * normal;
                r[at] = if solver.min_fraction <= size { size.min(1.0) } else { solver.min_fraction };
                sum[at] = r[at] + if at > 0 { sum[at - 1] } else { 0.0 };
            }
        }
    }
    table
}

/// `sub_94C8C0`'s geometric spacing for one frame: the radius function's ends
/// at this range, the smaller held to a 256th of the larger, and each sprite's
/// radius a step of `(r1 / r0)^(1 / (count - 1))` on, read back as the offset
/// a straight line from r0 to r1 gives it.
#[derive(Debug, Clone, Copy)]
struct Geometric {
    first: f32,
    ratio: f32,
    scale: f32,
    shift: f32,
}

impl Geometric {
    fn of(radius: &TagFunction, count: usize, range: f32) -> Self {
        let (mut r0, mut r1) = (radius.evaluate(0.0, range), radius.evaluate(1.0, range));
        if r1 <= r0 {
            if r1 <= r0 * 0.00390625 {
                r1 = r0 * 0.00390625;
            }
        } else if r0 <= r1 * 0.00390625 {
            r0 = r1 * 0.00390625;
        }
        let ratio = if r0.abs() >= 0.0001 { (r1 / r0).powf(1.0 / (count - 1) as f32) } else { 1.0 };
        let scale = if (r0 - r1).abs() < 0.0001 { 1.0 } else { 1.0 / (r1 - r0) };
        Self { first: r0, ratio, scale, shift: -(r0 * scale) }
    }

    fn place(&self, sprite: usize) -> f32 {
        self.scale * self.first * self.ratio.powi(sprite as i32) + self.shift
    }
}

impl H2Volume {
    /// Where sprite `sprite` lies along the volume, at `range`.
    fn place_along(&self, sprite: usize, range: f32, geometric: Option<&Geometric>) -> f32 {
        match &self.spacing {
            Spacing::Linear => sprite as f32 / (self.count - 1) as f32,
            Spacing::Geometric => geometric.map_or(0.0, |geometric| geometric.place(sprite)),
            Spacing::Table(table) => table.get(sprite).map_or(0.0, |[low, high]| (1.0 - range) * low + high * range),
            Spacing::Nowhere => 0.0,
        }
    }

    pub(crate) fn fuzzy(&self) -> bool {
        self.fuzzy
    }

    pub(crate) fn bitmap(&self) -> Option<&Arc<H2Image>> {
        self.bitmap.as_ref()
    }
}

impl H2LightVolume {
    #[cfg(test)]
    pub(crate) fn effect_part(&self) -> Option<&H2EffectPart> {
        self.effect.as_ref()
    }



    pub(crate) fn volumes(&self) -> &[H2Volume] {
        &self.volumes
    }

    pub(crate) fn scale_name(&self) -> &str {
        &self.scale_by
    }

    /// The color and brightness it is submitted with: an attachment's change
    /// color and function, or an effect part's white, faded over its event
    /// unless its first volume asks otherwise, and A and B scales. None while
    /// an effect's part is not drawn.
    fn submitted(
        &self,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        time: f32,
        onset: f32,
    ) -> Option<([f32; 3], f32)> {
        let Some(part) = &self.effect else {
            let tint = self.change_color.and_then(|index| change_colors.get(index)).copied().unwrap_or([1.0; 3]);
            return Some((tint, super::lights::attachment_value(&self.scale_by, values)));
        };
        let lit = part.light(&self.scale_by, values, time, onset)?;
        let kept = self.volumes.first().is_some_and(|volume| volume.not_scaled_by_duration);
        let fade = if lit.lasts && !kept { lit.fade } else { 1.0 };
        Some(([fade; 3], lit.brightness))
    }

    /// `rasterizer_light_volume_render`, as a quad a sprite.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn quads(
        &self,
        volume: &H2Volume,
        values: Option<&FunctionValues>,
        change_colors: &[[f32; 3]; 4],
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        time: f32,
        onset: f32,
    ) -> Vec<CeAntennaQuad> {
        let Some((tint_by, submitted)) = self.submitted(values, change_colors, time, onset) else {
            return Vec::new();
        };
        let (origin, forward) = posed_on(self.node, self.position, self.forward, bones);
        let to_eye: [f32; 3] = std::array::from_fn(|axis| view.eye[axis] - origin[axis]);
        let distance = dot(to_eye, to_eye).sqrt();
        let unit_eye = to_eye.map(|axis| axis / distance.max(f32::EPSILON));
        let fade = camera_fade(self.falloff, distance);
        let brightness = fade * submitted;
        if brightness <= 0.0 {
            return Vec::new();
        }
        let facing_cosine = dot(unit_eye, forward).abs();
        let facing = 1.0 - volume.facing.evaluate((1.0 - facing_cosine).clamp(0.0, 1.0), 0.0);
        let amount = volume.aspect.as_ref().map_or(0.0, |aspect| {
            aspect_amount(aspect.threshold, aspect.exponent, facing_cosine)
        });
        // `light_volume_sprite_aspect`: only a volume stretching along its
        // axis turns its quads to lie along it, as the axis runs on the
        // screen (both ends projected; here the projection's derivative at
        // the origin, which points the same way).
        let (cos, sin) = if amount > 0.0 {
            let depth = dot(to_eye, view.forward).abs().max(f32::EPSILON);
            let toward = dot(forward, view.forward);
            let (x, y) = (-dot(to_eye, view.right), -dot(to_eye, view.up));
            let across = dot(forward, view.right) * depth - x * toward;
            let up = dot(forward, view.up) * depth - y * toward;
            let length = (across * across + up * up).sqrt();
            if length > 0.0 { (across / length, up / length) } else { (1.0, 0.0) }
        } else {
            (1.0, 0.0)
        };
        let side: [f32; 3] = std::array::from_fn(|axis| view.right[axis] * cos + view.up[axis] * sin);
        let lift: [f32; 3] = std::array::from_fn(|axis| view.up[axis] * cos - view.right[axis] * sin);
        let geometric = (volume.spacing == Spacing::Geometric).then(|| Geometric::of(&volume.radius, volume.count, facing));
        (0..volume.count)
            .filter_map(|sprite| {
                let along = volume.place_along(sprite, facing, geometric.as_ref());
                let offset = volume.offset.evaluate(along, facing) * self.marker_scale;
                let radius = volume.radius.evaluate(along, facing) * self.marker_scale;
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
                // `rasterizer_light_volume_draw_sprites` never draws a sprite
                // narrower along the axis than across it.
                let (half_wide, half_high) = ((radius * wide).max(radius * high), radius * high);
                if half_wide <= 0.0 || half_high <= 0.0 {
                    return None;
                }
                let alpha = (shine * brightness).clamp(0.0, 1.0);
                let color = volume.color.evaluate_color(along, facing);
                let tint = std::array::from_fn::<f32, 3, _>(|channel| {
                    [color.red, color.green, color.blue][channel].clamp(0.0, 1.0) * tint_by[channel] * alpha
                });
                let center: [f32; 3] = std::array::from_fn(|axis| origin[axis] + forward[axis] * offset);
                let corner = |x: f32, y: f32, u: f32, v: f32| -> [f32; 5] {
                    let [a, b, c] = std::array::from_fn(|axis| {
                        center[axis] + side[axis] * x * half_wide + lift[axis] * y * half_high
                    });
                    [a, b, c, u, v]
                };
                Some(CeAntennaQuad {
                    corners: [corner(-1.0, 1.0, 0.0, 0.0), corner(1.0, 1.0, 1.0, 0.0), corner(-1.0, -1.0, 0.0, 1.0), corner(1.0, -1.0, 1.0, 1.0)],
                    // `widget_light_volume`'s pixel shader: colour by its alpha by the
                    // map's colour, and an alpha of 0, so under `ONE, INV_SRC_ALPHA`
                    // it only adds light and never darkens what is behind.
                    color: [tint[0], tint[1], tint[2], 0.0],
                })
            })
            .collect()
    }
}

attached!(H2LightVolume);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::common::attached::Attached;
    use crate::core::game::GameId;

    /// A held weapon's firing-effect volume answers its owner's panel under
    /// the weapon's name, as its flares and lights do; a volume an ordinary
    /// function scales stays with the weapon.
    #[test]
    fn an_attached_event_volume_moves_to_its_owner() {
        let volume = |scale_by: &str| H2LightVolume {
            object: 1,
            only_under: None,
            node: 0,
            position: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            falloff: [0.0; 2],
            scale_by: scale_by.to_owned(),
            marker_scale: 1.0,
            change_color: None,
            volumes: Arc::new(Vec::new()),
            effect: None,
        };
        let fired = volume("firing effect: firing").on_attached("gun");
        assert_eq!((fired.scale_by.as_str(), fired.object), ("gun: firing effect: firing", 0));
        let steady = volume("heat").on_attached("gun");
        assert_eq!((steady.scale_by.as_str(), steady.object), ("heat", 1));
    }

    #[test]
    fn geometric_sprites_run_from_the_start_to_the_end() {
        let geometric = Geometric { first: 0.5, ratio: 2.0f32.powf(1.0 / 3.0), scale: 2.0, shift: -1.0 };
        assert!(geometric.place(0).abs() < 1e-6);
        assert!((geometric.place(3) - 1.0).abs() < 1e-5, "r0 0.5 to r1 1.0 over four sprites");
        assert!(geometric.place(1) < 1.0 / 3.0, "the small end packs closer");
    }

    /// `BLAM_TEST_H2EK`: every light volume's solved offsets climb from the
    /// start toward the end, as tool's solver leaves them.
    #[test]
    fn every_solved_table_climbs() {
        let tags = crate::core::test_kits::h2ek_tags();
        if !tags.join("globals/globals.globals").is_file() {
            eprintln!("skipping: set BLAM_TEST_H2EK to a Halo 2 kit's tags folder");
            return;
        }
        let source = TagSource::LooseFolder {
            root: tags.clone(),
            game: Some(GameId::Halo2),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        let mut paths = Vec::new();
        let mut stack = vec![tags.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "light_volume") {
                    paths.push(path);
                }
            }
        }
        let (mut kinds, mut tables) = ([0usize; 4], 0);
        for path in paths {
            let relative = path.strip_prefix(&tags).unwrap().with_extension("");
            let name = relative.to_string_lossy().replace('/', "\\");
            let tag = load_referenced_tag_from_source(&source, &name, "light_volume", b"MGS2").unwrap();
            for volume in elements(tag.root(), "volumes") {
                let Some(read) = read_volume(&source, volume, None) else {
                    continue;
                };
                match &read.spacing {
                    Spacing::Linear => kinds[0] += 1,
                    Spacing::Geometric => kinds[1] += 1,
                    Spacing::Nowhere => kinds[3] += 1,
                    Spacing::Table(table) => {
                        kinds[2] += 1;
                        tables += 1;
                        for range in 0..2 {
                            let column: Vec<f32> = table.iter().map(|entry| entry[range]).collect();
                            assert!(column.windows(2).all(|pair| pair[1] >= pair[0] - 1e-5), "{name}: {column:?}");
                            assert!(column.iter().all(|x| (0.0..=1.0 + 1e-5).contains(x)), "{name}: {column:?}");
                        }
                    }
                }
            }
        }
        eprintln!("linear {}, geometric {}, table {}, nowhere {}", kinds[0], kinds[1], kinds[2], kinds[3]);
        assert!(tables > 0, "no volume needed the solver");
    }

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
