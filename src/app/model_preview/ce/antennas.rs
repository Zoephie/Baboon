//! Halo CE's antennas: what `antenna_new` makes of a `widget`, and the
//! sprites `antenna_render_proper` draws along it.

use std::sync::Arc;

use blam_tags::TagFile;

use super::super::antennas::{ANTENNA_TRIM, AntennaSprite, antenna_sprite};
use super::super::common::fields::{elements, object_block};
use super::super::common::image::Image;
use super::super::common::math::dot;
use super::super::common::sprites::AntennaQuad;
use super::super::common::view::{ViewFrame, posed_on};
use super::super::materials::decode_image;
use super::super::{RenderModelPreviewMarker, TagReader};
use super::lights::marker_or_origin;
use crate::app::model_preview::common::fields::argb;
use crate::core::h2_pixel_shader::SamplerKind;

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
    pub(crate) image: Arc<Image>,
}

/// What `antenna_render_proper` measures an antenna's `falloff pixels` and
/// `cutoff pixels` against: a constant (`flt_A2AFDC` in MCC tag_test), not
/// the antenna's size on screen, so its fade,
/// `(100 - cutoff) / (falloff - cutoff)`, is the same at any distance.
const ANTENNA_FADE_PIXELS: f32 = 100.0;

impl CeAntenna {
    /// `antenna_render_proper`, for an antenna at rest: its vertices lie
    /// straight out of the marker, which is where `antenna_update`'s spring
    /// pulls them, without the point physics that sway them.
    pub(crate) fn quads(&self, bones: Option<&[[f32; 4]]>, view: &ViewFrame) -> Vec<AntennaQuad> {
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
                let place: [f32; 3] = std::array::from_fn(|axis| {
                    origin[axis] + forward[axis] * along + across[axis] * side
                });
                [place[0], place[1], place[2], u, v]
            };
            let [red, green, blue, alpha] = segment.color;
            quads.push(AntennaQuad {
                corners: [
                    corner(left, bottom),
                    corner(right, bottom),
                    corner(left, top),
                    corner(right, top),
                ],
                color: [red, green, blue, alpha * self.fade],
            });
            origin = std::array::from_fn(|axis| origin[axis] + forward[axis] * segment.length);
        }
        quads
    }
}

fn read_antenna(
    reader: &TagReader,
    path: &str,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Option<CeAntenna> {
    let tag = reader.load(path, "antenna", b"ant!").ok()?;
    let root = tag.root();
    let name = root
        .read_string("attachment marker name")
        .unwrap_or_default();
    let (node, position, forward) = marker_or_origin(markers, &name, owner)?;
    let bitmap_path = root
        .read_tag_ref_path("bitmaps")
        .filter(|path| !path.is_empty())?;
    let bitmap = reader.load(&bitmap_path, "bitmap", b"bitm").ok()?;
    let mut image_index = None;
    let mut segments = Vec::new();
    for vertex in elements(root, "vertices") {
        let sequence = vertex.read_int_any("sequence index").unwrap_or(0).max(0) as usize;
        let AntennaSprite {
            rect,
            registration,
            index,
            covered,
            ..
        } = antenna_sprite(&bitmap, sequence, ANTENNA_TRIM)?;
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
    let image = decode_image(reader, &bitmap_path, SamplerKind::Texture2D, image_index?)?;
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
pub(in crate::app::model_preview) fn read_ce_antennas(
    reader: &TagReader,
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
        .filter_map(|(_, path)| read_antenna(reader, &path, owner, markers))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::super::ce::lights::tests::VIEW;

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
            image: Arc::new(Image::Flat(
                crate::app::model_preview::materials::TextureImage {
                    rgba: Vec::new().into(),
                    width: 0,
                    height: 0,
                    repeat_x: true,
                    repeat_y: true,
                    scale: [1.0; 2],
                },
            )),
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
}
