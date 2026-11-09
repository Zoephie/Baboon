//! Reach's antennas: a widget's line of sprites whose far end its spring
//! moves, read as `antenna_new` and `widgets_new` read them.

use std::sync::Arc;

use blam_tags::TagFile;

use super::super::antennas::{
    Antenna, AntennaCurve, AntennaSpring, AntennaSprite, AntennaVertex, antenna_sprite,
};
use super::super::common::attached::Attached as _;
use super::super::common::fields::{argb, elements, object_block};
use super::super::common::marker::MarkerFrame;
use super::super::gen3::materials::read_gen3_bitmap;
use super::super::materials::decode_image;
use super::super::{RenderModelPreviewMarker, TagReader};
use crate::core::h2_pixel_shader::SamplerKind;

/// What Reach takes off a sprite's pixels besides its spacing:
/// `get_vertex_scale_by_distance` (the 360's 0x82C93A88, its
/// constant at 0x82C93BFC) takes half a pixel.
pub(in crate::app::model_preview) const REACH_ANTENNA_TRIM: f32 = 0.5;

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
        .map(|tag| crate::app::model_preview::common::point_physics::PointPhysics::read(&tag));
    let reach = AntennaSpring {
        strength: root.read_real("spring strength coefficient").unwrap_or(0.0),
        length: total_length,
        physics,
    };
    let image_index = image_index.unwrap_or(0);
    let image = decode_image(reader, &bitmap_path, SamplerKind::Texture2D, image_index)?;
    let gen3_bitmap = read_gen3_bitmap(
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
        spring: Some(reach),
        only_under: marker.only_under.clone(),
        gen3_bitmap,
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
