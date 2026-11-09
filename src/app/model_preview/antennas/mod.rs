//! Antennas: the lines of sprites a `widget` hangs from a marker. Halo 2's,
//! Halo 3's, Reach's and Halo 4's are [`Antenna`]s, Reach's far end moved by
//! its [`spring`]; Halo CE's own are in [`ce`]. The sprite sizing and the
//! quads they draw are shared.

use std::sync::Arc;

use blam_tags::TagFile;

use super::common::fields::{elements, object_block};
use super::common::image::Image;
use super::common::math::{cross, dot};
use super::common::view::{ViewFrame, posed_on};
use super::gen3::materials::{Gen3Bitmap, read_h3_bitmap};
use super::materials::decode_image;
use super::*;
use crate::app::model_preview::common::attached::{Attached, attached};
use crate::app::model_preview::common::fields::argb;
use crate::app::model_preview::common::marker::MarkerFrame;
use crate::app::model_preview::common::math::UNSET;
use crate::app::model_preview::common::sprites::AntennaQuad;
use crate::core::h2_pixel_shader::SamplerKind;
use spring::{ReachAntennaSpring, posed_marker};

pub(in crate::app::model_preview) mod ce;
pub(super) mod spring;

/// The first sprite of an antenna vertex's sequence, as `antenna_new` and
/// `get_vertex_scale_by_distance` size it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app::model_preview) struct AntennaSprite {
    /// Left, right, top and bottom, across the bitmap.
    pub(in crate::app::model_preview) rect: [f32; 4],
    pub(in crate::app::model_preview) registration: [f32; 2],
    /// The image of the bitmap it is cut from.
    pub(in crate::app::model_preview) index: usize,
    /// What of the bitmap's width a sprite runs along.
    pub(in crate::app::model_preview) covered: f32,
    /// That image's width, in texels.
    pub(in crate::app::model_preview) width: f32,
}

/// What Halo CE, 2 and 3 take off a sprite's pixels besides its spacing.
pub(in crate::app::model_preview) const ANTENNA_TRIM: f32 = 1.0;
/// Reach's: `get_vertex_scale_by_distance` (the 360's 0x82C93A88, its
/// constant at 0x82C93BFC) takes half a pixel.
pub(in crate::app::model_preview) const REACH_ANTENNA_TRIM: f32 = 0.5;

/// `antenna_new`'s scale for a sprite, or none where a bitmap has no such
/// sprite to draw: its `pixels`, `width · (right - left) - 2 · spacing -
/// trim`, must be more than none.
pub(in crate::app::model_preview) fn antenna_sprite(
    bitmap: &TagFile,
    sequence: usize,
    trim: f32,
) -> Option<AntennaSprite> {
    let root = bitmap.root();
    let sequences = elements(root, "sequences");
    let sprite = *elements(*sequences.get(sequence)?, "sprites").first()?;
    let read = |name: &str| sprite.read_real(name).unwrap_or(0.0);
    let index = sprite.read_int_any("bitmap index")?.max(0) as usize;
    let width = elements(root, "bitmaps")
        .get(index)?
        .read_int_any("width")? as f32;
    let spacing = root.read_int_any("sprite spacing").unwrap_or(0) as f32;
    let rect = [read("left"), read("right"), read("top"), read("bottom")];
    let registration = sprite.read_point2d("registration point");
    let pixels = (rect[1] - rect[0]) * width - spacing * 2.0 - trim;
    let registration = [registration.x, registration.y];
    (pixels > 0.0).then_some(AntennaSprite {
        rect,
        registration,
        index,
        covered: pixels / width,
        width,
    })
}

#[derive(Debug, Clone)]
struct AntennaVertex {
    /// `sub_A63CD0`: where the next vertex lies, with +Z the way this one points.
    offset: [f32; 3],
    color: [f32; 4],
    /// Left, right, top and bottom of the sprite, across the bitmap.
    rect: [f32; 4],
    registration: [f32; 2],
    /// World units per width of the bitmap: `antenna_new` runs the sprite the
    /// length of the link.
    scale: f32,
    /// What of the bitmap's width a sprite runs along.
    covered: f32,
    /// Where along the curve this link ends, for an antenna that has one.
    hermite_t: f32,
    /// Reach's: how wide the sprite is, in world units, as `build_sprite`
    /// scales its `texel to world width scale` by the bitmap's width; none
    /// where the sprite has no pixels to draw.
    width: Option<f32>,
}

/// Halo 3's antenna: one curve through two points, with each link a stretch of
/// it, as `antenna_get_hermite_vectors` sets it up.
#[derive(Debug, Clone, Copy)]
struct AntennaCurve {
    total_length: f32,
    point_of_bend: f32,
    starting_bend: f32,
    ending_bend: f32,
}

impl AntennaCurve {
    /// Where `antenna_update`'s spring rests the far end: straight out of the
    /// marker, with no gravity or motion on it.
    fn rest(&self, origin: [f32; 3], direction: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| origin[axis] + direction[axis] * self.total_length)
    }

    /// `c_hermite::evaluate` from `origin` to the far end at `end`, as
    /// `antenna_get_hermite_vectors` sets it up.
    fn points(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        end: [f32; 3],
        at: impl Iterator<Item = f32>,
    ) -> Vec<[f32; 3]> {
        let along = |distance: f32| -> [f32; 3] {
            std::array::from_fn(|axis| origin[axis] + direction[axis] * distance)
        };
        let bend = along(self.total_length * self.point_of_bend);
        let start_tangent = direction.map(|axis| axis * self.starting_bend);
        let end_tangent: [f32; 3] =
            std::array::from_fn(|axis| (end[axis] - bend[axis]) * self.ending_bend);
        let mut points = vec![origin];
        points.extend(at.map(|t| -> [f32; 3] {
            let (square, cube) = (t * t, t * t * t);
            let h1 = 2.0 * cube - 3.0 * square + 1.0;
            let h2 = -2.0 * cube + 3.0 * square;
            let h3 = cube - 2.0 * square + t;
            let h4 = cube - square;
            std::array::from_fn(|axis| {
                h1 * origin[axis]
                    + h2 * end[axis]
                    + h3 * start_tangent[axis]
                    + h4 * end_tangent[axis]
            })
        }));
        points
    }
}

/// A `widget` that hangs a line of sprites from a marker.
#[derive(Debug, Clone)]
pub(crate) struct Antenna {
    node: usize,
    position: [f32; 3],
    forward: [f32; 3],
    vertices: Vec<AntennaVertex>,
    curve: Option<AntennaCurve>,
    /// Reach's spring, which moves the far end; the others rest it.
    reach: Option<ReachAntennaSpring>,
    /// The region and permutation its marker belongs to, if only one.
    only_under: Option<(String, String)>,
    /// Halo 3 and Reach read their bitmap as the card would.
    pub(crate) h3_bitmap: Option<Gen3Bitmap>,
    pub(crate) image: Arc<Image>,
}

/// `sub_421A90`: `vector` turned about the unit `axis`.
fn rotated(vector: [f32; 3], axis: [f32; 3], sine: f32, cosine: f32) -> [f32; 3] {
    let along = dot(vector, axis) * (1.0 - cosine);
    let across = cross(axis, vector);
    std::array::from_fn(|i| axis[i] * along + vector[i] * cosine + across[i] * sine)
}

/// `offset`, with its +Z swung onto `direction`, as `antenna_update_points`
/// lays out a link.
fn aimed(offset: [f32; 3], direction: [f32; 3]) -> [f32; 3] {
    let length = dot(direction, direction).sqrt();
    if length < UNSET {
        return offset;
    }
    let [x, y, z] = direction.map(|axis| axis / length);
    let axis = [-y, x, 0.0];
    let width = dot(axis, axis).sqrt();
    let axis = if width < UNSET {
        [0.0, 1.0, 0.0]
    } else {
        axis.map(|a| a / width)
    };
    rotated(offset, axis, width, z)
}

impl Antenna {
    /// Whether this is Reach's, with a spring of its own.
    pub(crate) fn is_reach(&self) -> bool {
        self.reach.is_some()
    }

    pub(crate) fn reach_spring(&self) -> Option<&ReachAntennaSpring> {
        self.reach.as_ref()
    }

    /// Where its marker is posed, and the unit way it faces.
    pub(crate) fn marker(&self, bones: Option<&[[f32; 4]]>) -> ([f32; 3], [f32; 3]) {
        posed_marker(self.node, self.position, self.forward, bones)
    }

    /// `antenna_render`. Halo 2's and 3's rest: the spring has every link at
    /// the place it pulls toward, with no wind or motion to sway it. Reach's
    /// far end is at `tip`, where its spring has it, or at rest before it has
    /// run.
    pub(crate) fn quads(
        &self,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        tip: Option<[f32; 3]>,
    ) -> Vec<AntennaQuad> {
        if let (Some(_), Some(curve)) = (&self.reach, &self.curve) {
            return self.reach_quads(curve, bones, view, tip);
        }
        let (mut origin, mut direction) = posed_on(self.node, self.position, self.forward, bones);
        let mut points = vec![origin];
        if let Some(curve) = &self.curve {
            let end = curve.rest(origin, direction);
            points = curve.points(
                origin,
                direction,
                end,
                self.vertices.iter().map(|vertex| vertex.hermite_t),
            );
        } else {
            for vertex in &self.vertices {
                let link = aimed(vertex.offset, direction);
                origin = std::array::from_fn(|axis| origin[axis] + link[axis]);
                direction = link;
                points.push(origin);
            }
        }
        let quad = |(index, vertex): (usize, &AntennaVertex)| {
            let (start, end) = (points[index], points[index + 1]);
            let link: [f32; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
            let length = dot(link, link).sqrt().max(UNSET);
            let link = link.map(|axis| axis / length);
            // `get_vertex_scale_by_distance`: a short link is a thin one too.
            let scale = if self.curve.is_some() {
                length / vertex.covered
            } else {
                vertex.scale
            };
            let across = cross(view.forward, link);
            let width = dot(across, across).sqrt().max(UNSET);
            let across = across.map(|axis| axis / width);
            let [left, right, top, bottom] = vertex.rect;
            let [x, y] = vertex.registration;
            let corner = |u: f32, v: f32| {
                let along = (u - (x + left)) * scale;
                let side = (top + y - v) * scale;
                let place: [f32; 3] = std::array::from_fn(|axis| {
                    start[axis] + link[axis] * along - across[axis] * side
                });
                [place[0], place[1], place[2], u, v]
            };
            AntennaQuad {
                corners: [
                    corner(left, bottom),
                    corner(right, bottom),
                    corner(left, top),
                    corner(right, top),
                ],
                color: vertex.color,
            }
        };
        self.vertices.iter().enumerate().map(quad).collect()
    }

    /// Reach's `antenna_render_proper` (the 360's 0x82C93C68): each vertex's
    /// sprite along the link to its point on the curve, from the last.
    /// `get_vertex_scale_by_distance` stretches it the link's length over
    /// its pixels; `build_sprite` (0x84BFF6B8) scales both ways by the
    /// bitmap's width, and turns its width across the ray from the eye.
    fn reach_quads(
        &self,
        curve: &AntennaCurve,
        bones: Option<&[[f32; 4]]>,
        view: &ViewFrame,
        tip: Option<[f32; 3]>,
    ) -> Vec<AntennaQuad> {
        let (origin, forward) = self.marker(bones);
        let end = tip.unwrap_or_else(|| curve.rest(origin, forward));
        let points = curve.points(
            origin,
            forward,
            end,
            self.vertices.iter().map(|vertex| vertex.hermite_t),
        );
        let mut quads = Vec::with_capacity(self.vertices.len());
        for (index, vertex) in self.vertices.iter().enumerate() {
            let (start, next) = (points[index], points[index + 1]);
            let link: [f32; 3] = std::array::from_fn(|axis| next[axis] - start[axis]);
            let length = dot(link, link).sqrt();
            let (along_scale, width) = (length / vertex.covered, vertex.width.unwrap_or(0.0));
            if !(along_scale > 0.0 && width > 0.0) {
                continue;
            }
            let link = link.map(|axis| axis / length);
            // `build_sprite_compute_basis`, case 1, in view space: across the
            // ray from the eye, which is the world's turned. Which side it
            // lies on rests on the view matrix keeping its handedness.
            // [inferred]
            let from_eye: [f32; 3] = std::array::from_fn(|axis| start[axis] - view.eye[axis]);
            let across = cross(from_eye, link);
            let breadth = dot(across, across).sqrt().max(UNSET);
            let across = across.map(|axis| axis / breadth);
            let [left, right, top, bottom] = vertex.rect;
            let [x, y] = vertex.registration;
            let corner = |u: f32, v: f32| {
                let along = (u - (left + x)) * along_scale;
                let side = (top + y - v) * width;
                let place: [f32; 3] = std::array::from_fn(|axis| {
                    start[axis] + link[axis] * along + across[axis] * side
                });
                [place[0], place[1], place[2], u, v]
            };
            // `real_argb_color_to_pixel32`: the tag's color, held to what a
            // byte can carry.
            let color = vertex.color.map(|channel| channel.clamp(0.0, 1.0));
            quads.push(AntennaQuad {
                // `build_sprite`'s strip: right top, right bottom, left top, left bottom.
                corners: [
                    corner(right, top),
                    corner(right, bottom),
                    corner(left, top),
                    corner(left, bottom),
                ],
                color,
            });
        }
        quads
    }
}

fn read_antenna(
    reader: &TagReader,
    path: &str,
    markers: &[RenderModelPreviewMarker],
) -> Option<Antenna> {
    /// The pixels `antenna_render` takes an antenna to cover, whatever it does.
    const PIXELS: f32 = 100.0;
    let tag = reader.load(path, "antenna", b"ant!").ok()?;
    let root = tag.root();
    let name = root
        .read_string_id("attachment marker name")
        .or_else(|| root.read_string("attachment marker name"))?;
    let marker = markers
        .iter()
        .find(|marker| marker.name.eq_ignore_ascii_case(&name))?;
    let bitmap_path = root
        .read_tag_ref_path("bitmaps")
        .filter(|path| !path.is_empty())?;
    let bitmap = reader.load(&bitmap_path, "bitmap", b"bitm").ok()?;
    let falloff = root.read_real("falloff pixels").unwrap_or(0.0);
    let cutoff = root.read_real("cutoff pixels").unwrap_or(0.0);
    // Halo 3 bakes these out of the tags it ships.
    let alpha = if falloff == cutoff {
        1.0
    } else {
        ((PIXELS - cutoff) / (falloff - cutoff)).clamp(0.0, 1.0)
    };
    let curve = root.field("point of bend").is_some().then(|| AntennaCurve {
        total_length: root.read_real("runtime total length").unwrap_or(0.0),
        point_of_bend: root.read_real("point of bend").unwrap_or(0.0),
        starting_bend: root.read_real("starting bend").unwrap_or(0.0),
        ending_bend: root.read_real("ending bend").unwrap_or(0.0),
    });
    let mut vertices = Vec::new();
    for vertex in elements(root, "vertices") {
        let sequence = vertex.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
        let AntennaSprite {
            rect,
            registration,
            covered,
            ..
        } = antenna_sprite(&bitmap, sequence, ANTENNA_TRIM)?;
        let length = vertex.read_real("length").unwrap_or(0.0);
        let angles = vertex.read_euler2d("angles");
        let [red, green, blue, own_alpha] = argb(vertex, "color");
        // `antenna_render_proper` hands `build_sprite` the color as it stands.
        let alpha = if curve.is_some() { own_alpha } else { alpha };
        vertices.push(AntennaVertex {
            offset: [
                angles.yaw.cos() * length * angles.pitch.sin(),
                angles.yaw.sin() * length * angles.pitch.sin(),
                angles.pitch.cos() * length,
            ],
            color: [red, green, blue, alpha],
            rect,
            registration,
            scale: length / covered,
            covered,
            hermite_t: vertex.read_real("hermite t").unwrap_or(0.0),
            width: None,
        });
    }
    let image = decode_image(reader, &bitmap_path, SamplerKind::Texture2D, 0)?;
    let h3_bitmap =
        curve.and_then(|_| read_h3_bitmap(reader, &bitmap_path, 0, SamplerKind::Texture2D));
    Some(Antenna {
        node: marker.node_index.max(0) as usize,
        position: marker.position,
        forward: marker.axes[0],
        vertices,
        curve,
        reach: None,
        only_under: None,
        h3_bitmap,
        image: Arc::new(image),
    })
}

/// The antennas among `object`'s widgets.
pub(in crate::app::model_preview) fn read_antennas(
    reader: &TagReader,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
) -> Vec<Antenna> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let antenna_group = u32::from_be_bytes(*b"ant!");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("type"))
        .filter(|(group, _)| *group == antenna_group)
        .filter_map(|(_, path)| read_antenna(reader, &path, markers))
        .collect()
}

/// Reach's antenna at `path`, by `antenna_new` (the 360's 0x82C92F10, MCC's
/// 0x140E51B70): none where the object has no marker of its name, or it has
/// fewer than two vertices. Its marker is placed by `frame`, so an attached
/// object's lands on its parent.
fn read_reach_antenna<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    reader: &TagReader,
    path: &str,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
) -> Option<Antenna> {
    /// What `antenna_postprocess` (0x84C26A00) floors a total length at.
    const SHORTEST: f32 = 1e-4;
    let tag = reader.load(path, "antenna", b"ant!").ok()?;
    let root = tag.root();
    let name = root
        .read_string_id("attachment marker name")
        .or_else(|| root.read_string("attachment marker name"))?;
    let marker = markers
        .iter()
        .find(|marker| marker.name.eq_ignore_ascii_case(&name))?;
    let listed = elements(root, "vertices");
    if listed.len() < 2 {
        return None;
    }
    let bitmap_path = root
        .read_tag_ref_path("bitmaps")
        .filter(|path| !path.is_empty())?;
    let bitmap = reader.load(&bitmap_path, "bitmap", b"bitm").ok()?;
    let texel_scale = root.read_real("texel to world width scale").unwrap_or(0.0);
    // `falloff pixels`, `cutoff pixels`, `LOD color`, `angles` and `vector to
    // next` are read by nothing that draws or moves it.
    let mut image_index = None;
    let mut vertices = Vec::new();
    let mut summed = 0.0;
    for vertex in listed {
        let sequence = vertex.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
        // A sprite with no pixels is left out, and its link still runs on.
        let sprite = antenna_sprite(&bitmap, sequence, REACH_ANTENNA_TRIM);
        let AntennaSprite {
            rect,
            registration,
            index,
            covered,
            width,
        } = sprite.unwrap_or(AntennaSprite {
            rect: [0.0; 4],
            registration: [0.0; 2],
            index: 0,
            covered: 0.0,
            width: 0.0,
        });
        if sprite.is_some() {
            image_index.get_or_insert(index);
        }
        summed += vertex.read_real("length").unwrap_or(0.0);
        vertices.push(AntennaVertex {
            offset: [0.0; 3],
            color: argb(vertex, "color"),
            rect,
            registration,
            scale: 0.0,
            covered,
            hermite_t: vertex.read_real("hermite t").unwrap_or(0.0),
            width: sprite.map(|_| texel_scale * width),
        });
    }
    let stored = root.read_real("runtime total length").unwrap_or(0.0);
    // The kit's tags hold what the postprocess makes; this is only a net.
    // [inferred]
    let total_length = if stored > 0.0 {
        stored
    } else {
        summed.max(SHORTEST)
    };
    let curve = AntennaCurve {
        total_length,
        point_of_bend: root.read_real("point of bend").unwrap_or(0.0),
        starting_bend: root.read_real("starting bend").unwrap_or(0.0),
        ending_bend: root.read_real("ending bend").unwrap_or(0.0),
    };
    let physics = root
        .read_tag_ref_path("physics")
        .filter(|path| !path.is_empty())
        .and_then(|path| reader.load(&path, "point_physics", b"pphy").ok())
        .map(|tag| crate::app::model_preview::common::point_physics::CePointPhysics::read(&tag));
    let reach = ReachAntennaSpring {
        strength: root.read_real("spring strength coefficient").unwrap_or(0.0),
        length: total_length,
        physics,
    };
    let image_index = image_index.unwrap_or(0);
    let image = decode_image(reader, &bitmap_path, SamplerKind::Texture2D, image_index)?;
    let h3_bitmap = read_h3_bitmap(
        reader,
        &bitmap_path,
        image_index as i16,
        SamplerKind::Texture2D,
    );
    let (place, node) = frame(marker);
    Some(Antenna {
        node,
        position: place.position,
        forward: place.axes[0],
        vertices,
        curve: Some(curve),
        reach: Some(reach),
        only_under: marker.only_under.clone(),
        h3_bitmap,
        image: Arc::new(image),
    })
}

/// Reach's antennas among `object`'s widgets (`widgets_new`, the 360's
/// 0x82BCBFC0), named after the attached object they hang on, if any.
pub(in crate::app::model_preview) fn read_reach_antennas<
    F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
>(
    reader: &TagReader,
    object: &TagFile,
    markers: &[RenderModelPreviewMarker],
    frame: &F,
    name: Option<&str>,
) -> Vec<Antenna> {
    let Some(block) = object_block(object, "widgets") else {
        return Vec::new();
    };
    let antenna_group = u32::from_be_bytes(*b"ant!");
    elements(block, "widgets")
        .iter()
        .filter_map(|widget| widget.read_tag_ref_with_group("type"))
        .filter(|(group, path)| *group == antenna_group && !path.is_empty())
        .filter_map(|(_, path)| read_reach_antenna(reader, &path, markers, frame))
        .map(|antenna| match name {
            Some(name) => antenna.on_attached(name),
            None => antenna,
        })
        .collect()
}

attached!(keeps function: Antenna);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::common::math::length;
    use crate::app::model_preview::harness::kit::kit_source;
    use crate::core::game::GameId;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn a_link_keeps_its_offset_along_plus_z() {
        assert!(close(
            aimed([0.1, 0.2, 0.3], [0.0, 0.0, 2.0]),
            [0.1, 0.2, 0.3]
        ));
    }

    #[test]
    fn a_link_swings_plus_z_onto_the_way_the_last_points() {
        assert!(close(
            aimed([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            [1.0, 0.0, 0.0]
        ));
        assert!(close(
            aimed([0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
            [0.0, -1.0, 0.0]
        ));
        assert!(close(
            aimed([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            [0.0, 0.0, -1.0]
        ));
    }

    #[test]
    fn a_halo_3_antenna_rests_along_its_marker_with_links_as_the_curve_spaces_them() {
        let curve = AntennaCurve {
            total_length: 0.25,
            point_of_bend: 0.1,
            starting_bend: 0.3,
            ending_bend: 0.2,
        };
        let end = curve.rest([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let points = curve.points(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            end,
            [0.05, 0.11, 1.0].into_iter(),
        );
        assert_eq!(points[0], [1.0, 0.0, 0.0]);
        assert!(
            points
                .iter()
                .all(|point| point[0] == 1.0 && point[1] == 0.0)
        );
        assert!((points[3][2] - 0.25).abs() < 1e-6);
        // Faster off the marker than a straight line through the tag's `t`.
        assert!((points[1][2] - 0.05 * 0.25).abs() > 0.001, "{points:?}");
        assert!((points[1][2] - 0.015243).abs() < 0.00001, "{points:?}");
    }

    /// The warthog's antenna, sequence 0's sprite on each of two vertices.
    fn reach_antenna(widths: [Option<f32>; 2]) -> Antenna {
        let vertex = |hermite_t: f32, width: Option<f32>| AntennaVertex {
            offset: [0.0; 3],
            color: [0.102, 0.102, 0.102, 1.0],
            rect: [0.46875, 0.921875, 0.4375, 0.625],
            registration: [0.03125, 0.09375],
            scale: 0.0,
            covered: 49.5 / 128.0,
            hermite_t,
            width,
        };
        Antenna {
            node: 0,
            position: [0.0; 3],
            forward: [0.0, 0.0, 1.0],
            vertices: vec![vertex(0.5, widths[0]), vertex(1.0, widths[1])],
            curve: Some(AntennaCurve {
                total_length: 0.4,
                point_of_bend: 0.1,
                starting_bend: 0.3,
                ending_bend: 0.2,
            }),
            reach: Some(ReachAntennaSpring {
                strength: 0.1,
                length: 0.4,
                physics: None,
            }),
            only_under: None,
            h3_bitmap: None,
            image: Arc::new(Image::Flat(super::super::super::materials::TextureImage {
                rgba: Vec::new(),
                width: 0,
                height: 0,
                repeat_x: true,
                repeat_y: true,
                scale: [1.0; 2],
            })),
        }
    }

    /// Looking at the antenna from +X, a little above.
    fn side_view() -> ViewFrame {
        ViewFrame {
            eye: [2.0, 0.0, 0.2],
            forward: [-1.0, 0.0, 0.0],
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
        }
    }

    fn between(a: [f32; 5], b: [f32; 5]) -> [f32; 3] {
        std::array::from_fn(|axis| a[axis] - b[axis])
    }

    /// Reach's sprite is as wide as its `texel to world width scale` times the
    /// bitmap's width, across the ray from the eye, added; Halo 3's is as wide
    /// as the link's length scales it, across the view's forward, taken away.
    #[test]
    fn a_reach_antenna_is_as_wide_as_its_texel_scale_across_the_eyes_ray() {
        let width = 0.0006 * 128.0;
        let reach = reach_antenna([Some(width); 2]);
        let quads = reach.quads(None, &side_view(), None);
        assert_eq!(quads.len(), 2);
        let [right_top, right_bottom, left_top, left_bottom] = quads[1].corners;
        let across = between(right_top, right_bottom);
        assert!(
            (length(across) - (0.625 - 0.4375) * width).abs() < 1e-6,
            "{across:?}"
        );
        assert!(
            across[1] > 0.0 && across[0].abs() < 1e-6,
            "top toward +Y, cross(start - eye, link): {across:?}"
        );
        assert_eq!(between(left_top, left_bottom), across);
        assert_eq!(
            quads[1].color,
            [0.102, 0.102, 0.102, 1.0],
            "the tag's color as it is"
        );
        // Along the link from p(0.5) to the tip, its length over the pixels.
        let [start, end] = [0.5_f32, 1.0].map(|t| curve_point(&reach, t));
        let link = length(std::array::from_fn(|axis| end[axis] - start[axis]));
        let along = between(right_top, left_top);
        assert!(
            (length(along) - (0.921875 - 0.46875) * link * 128.0 / 49.5).abs() < 1e-5,
            "{along:?}"
        );
        let halo3 = Antenna {
            reach: None,
            ..reach_antenna([None; 2])
        };
        let [_, right_bottom, _, right_top] = halo3.quads(None, &side_view(), None)[1].corners;
        let h3_across = between(right_top, right_bottom);
        assert!(
            (length(h3_across) - (0.625 - 0.4375) * link * 128.0 / 49.5).abs() < 1e-5,
            "{h3_across:?}"
        );
        assert!(
            h3_across[1] < 0.0,
            "Halo 3 takes its side away: {h3_across:?}"
        );
    }

    fn curve_point(antenna: &Antenna, t: f32) -> [f32; 3] {
        let curve = antenna.curve.unwrap();
        let end = curve.rest([0.0; 3], [0.0, 0.0, 1.0]);
        *curve
            .points([0.0; 3], [0.0, 0.0, 1.0], end, [t].into_iter())
            .last()
            .unwrap()
    }

    /// The curve runs to where the spring has the far end, and a sprite with
    /// no pixels is left out while the next still starts where it ended.
    #[test]
    fn a_reach_antenna_bends_to_its_tip_and_skips_a_sprite_without_pixels() {
        let curve = AntennaCurve {
            total_length: 0.4,
            point_of_bend: 0.1,
            starting_bend: 0.3,
            ending_bend: 0.2,
        };
        let tip = [0.1, 0.0, 0.387];
        let points = curve.points([0.0; 3], [0.0, 0.0, 1.0], tip, [0.5, 1.0].into_iter());
        assert_eq!(points[2], tip);
        let width = 0.0006 * 128.0;
        let resting = reach_antenna([Some(width); 2]).quads(None, &side_view(), None);
        let sprung = reach_antenna([Some(width); 2]).quads(None, &side_view(), Some(tip));
        assert_ne!(resting[1].corners, sprung[1].corners);
        let skipped = reach_antenna([None, Some(width)]).quads(None, &side_view(), None);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].corners, resting[1].corners);
    }

    /// With `BLAM_TEST_HREK`: the warthog antenna's sprite is 128 texels
    /// wide, and Reach takes half a pixel off its 58 where the others take one.
    #[test]
    fn kit_reach_takes_half_a_pixel_off_an_antenna_sprite() {
        let Some(source) = kit_source(GameId::HaloReach) else {
            return;
        };
        let path = "objects\\vehicles\\human\\warthog\\bitmaps\\antenna";
        let bitmap = load_referenced_tag_from_source(&source, path, "bitmap", b"bitm").unwrap();
        let reach = antenna_sprite(&bitmap, 0, REACH_ANTENNA_TRIM).unwrap();
        assert_eq!(reach.width, 128.0);
        assert!(
            (reach.covered * reach.width - 49.5).abs() < 1e-4,
            "{reach:?}"
        );
        let others = antenna_sprite(&bitmap, 0, ANTENNA_TRIM).unwrap();
        assert!(
            (others.covered * others.width - 49.0).abs() < 1e-4,
            "{others:?}"
        );
    }

    #[test]
    fn a_link_pointed_straight_down_turns_about_the_games_fallback_axis() {
        assert!(close(
            aimed([0.0, 0.0, 1.0], [0.0, 0.0, -1.0]),
            [0.0, 0.0, -1.0]
        ));
        assert!(close(
            aimed([0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
            [0.0, 1.0, 0.0]
        ));
    }
}
