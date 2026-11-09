//! A Halo 4 tracer on an object, and reading the tracers an object carries.

use super::super::super::{RenderModelPreviewMarker, TagReader};
use super::definition::{H4TracerDefinition, TracerLod, TracerPath, read_tracer};
use super::{
    EffectTimes, MarkerFrame, elements, for_each_attached_part, has_flag, high24_over_2_24,
    lcg_step, part_frame, read_part_placement,
};
use std::sync::Arc;

/// A tracer at a marker of the object it is attached to.
#[derive(Debug, Clone)]
pub(crate) struct H4TracerAt {
    pub(crate) object: u8,
    pub(super) start: (usize, [f32; 3]),
    pub(super) path: TracerPath,
    pub(super) only_under: Option<(String, String)>,
    pub(super) scale_by: String,
    pub(super) secondary_by: String,
    /// `real_random_range(0, 1)` twice, the tracer's correlations.
    pub(super) seeds: [f32; 2],
    pub(super) times: Arc<EffectTimes>,
    pub(super) event: usize,
    pub(super) lod: TracerLod,
    pub(crate) definition: Arc<H4TracerDefinition>,
}

/// The tracer's two correlations, a generator's for each tracer in turn.
pub(super) fn seeds_of(index: usize) -> [f32; 2] {
    let first = lcg_step((index as u32).wrapping_mul(2_654_435_761) | 1);
    [first, lcg_step(first)].map(high24_over_2_24)
}

/// The tracers of an object's attached effects, one for each location (and,
/// point to point, each location pair). A part runs only when the system's
/// `point-to-point` agrees with its having a secondary location
/// (`UpdateLocations` X 0x8296EB50).
pub(in crate::app::model_preview) fn read_h4_tracers(
    reader: &TagReader,
    object: &blam_tags::TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
    frame: impl Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
) -> Vec<H4TracerAt> {
    let mut tracers = Vec::new();
    let tracer_group = u32::from_be_bytes(*b"trac");
    for_each_attached_part(reader, object, |part| {
        if part.group != tracer_group {
            return;
        }
        let Ok(tag) = reader.load(&part.path, "tracer_system", b"trac") else {
            return;
        };
        let root = tag.root();
        let point_to_point = has_flag(root, "tracer system flags", "point-to-point");
        if point_to_point != part.secondary_marker.is_some() {
            // "… It won't render."
            return;
        }
        let lod = TracerLod::read(root);
        let definitions: Vec<Arc<H4TracerDefinition>> = elements(root, "tracers")
            .into_iter()
            .filter_map(|element| read_tracer(reader, element))
            .map(Arc::new)
            .collect();
        let (offset, yaw, pitch) = read_part_placement(part.part);
        let starts = part.effect.locations(markers, &part.marker, &frame);
        let ends = match &part.secondary_marker {
            Some(secondary) => part
                .effect
                .locations(markers, secondary, &frame)
                .into_iter()
                .map(Some)
                .collect(),
            None => vec![None; starts.len()],
        };
        for (start, end) in starts.iter().zip(ends) {
            let (place, node) = frame(start);
            let (place, node) = part.placed(place, node);
            let place = part_frame(&place, offset, yaw, pitch);
            let path = match end {
                Some(end) => {
                    let (far, far_node) = frame(&end);
                    TracerPath::PointToPoint {
                        end: (far_node, far.position),
                    }
                }
                None => TracerPath::Trailing {
                    forward: place.axes[0],
                },
            };
            for definition in &definitions {
                tracers.push(H4TracerAt {
                    object: owner,
                    start: (node, place.position),
                    path,
                    only_under: start.only_under.clone(),
                    scale_by: part.effect.scale_by.clone(),
                    secondary_by: part.effect.secondary_by.clone(),
                    seeds: seeds_of(tracers.len()),
                    times: Arc::clone(&part.times),
                    event: part.event,
                    lod,
                    definition: Arc::clone(definition),
                });
            }
        }
    });
    tracers
}
