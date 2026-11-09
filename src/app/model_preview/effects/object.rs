//! The effects an object carries, with their markers and vectors.

use super::super::{RenderModelPreviewMarker, TagReader, object_reference};
use super::events::{
    WORLD_AXES, WORLD_ORIENTATION, collect_event_effects, event_function_name, with_nested_effects,
};
use super::placement::marker_name;
use super::times::{EffectTimes, MOST_MARKERS, REPLACE};
use super::{
    MarkerFrame, cross, dot, elements, function_slot_of, has_flag, object_block, perpendicular,
    unit_by_reciprocal,
};
use blam_tags::{TagFile, TagStruct};

/// An effect an object runs: an attachment, on while its function is, or one
/// an event plays, which the preview gives a function of its own to start it.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct ObjectEffect {
    pub(in crate::app::model_preview) path: String,
    pub(super) marker: String,
    pub(in crate::app::model_preview) scale_by: String,
    pub(in crate::app::model_preview) secondary_by: String,
    pub(in crate::app::model_preview) change_color: Option<usize>,
    /// Plays through once, as an event's does; attachments and `looping effect`s loop.
    pub(in crate::app::model_preview) once: bool,
    /// Made with `gravity` and `up` vectors at its marker, as Reach's
    /// `effect_new_attached_from_marker_name` (the 360's 0x82EA7CF8) makes an
    /// attachment's.
    pub(super) vectors: bool,
}

impl ObjectEffect {
    /// `effect_build_locations`' marker for a location.
    pub(in crate::app::model_preview) fn marker_of(&self, location: TagStruct<'_>) -> String {
        let marker = marker_name(location, "marker name");
        if marker == REPLACE {
            self.marker.clone()
        } else {
            marker
        }
    }

    /// `use parent position but world orientation`.
    pub(in crate::app::model_preview) fn world_oriented(effect: TagStruct<'_>) -> bool {
        has_flag(effect, "flags", WORLD_ORIENTATION)
    }

    pub(in crate::app::model_preview) fn times_of(&self, effect: TagStruct<'_>) -> EffectTimes {
        EffectTimes {
            once: self.once,
            ..EffectTimes::read(effect)
        }
    }

    /// What a location named `name` (after `marker_of`) lands on. Reach's
    /// `effect_build_locations` (the 360's 0x82EADD50) takes a vector of that
    /// name before any marker; this effect's are `build_gravity_effect_vectors`'
    /// (0x82EA6DB0): `gravity` straight down and `up` straight up, at its
    /// marker. Anything else is `part_markers`'. A `replace` location is the
    /// marker itself and asks for no vector; after `marker_of` it reads as the
    /// marker's name, which no Reach attachment marker shares with a vector.
    pub(in crate::app::model_preview) fn locations<
        F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize),
    >(
        &self,
        markers: &[RenderModelPreviewMarker],
        name: &str,
        frame: &F,
    ) -> Vec<RenderModelPreviewMarker> {
        let down = if name.eq_ignore_ascii_case(GRAVITY) {
            Some(-1.0)
        } else {
            name.eq_ignore_ascii_case(UP).then_some(1.0)
        };
        match down {
            Some(z) if self.vectors && !name.eq_ignore_ascii_case(&self.marker) => {
                vec![vector_marker(
                    markers,
                    &self.marker,
                    name,
                    [0.0, 0.0, z],
                    frame,
                )]
            }
            _ => part_markers(markers, name, self.once),
        }
    }
}

const GRAVITY: &str = "gravity";

const UP: &str = "up";

/// `effect_marker_list_get_marker` for a world `direction` at the first marker
/// named `at` (`object_get_markers_by_string_id`, one wanted; none, or no name,
/// is the object's origin, as `MarkerFrame::find` has it): forward along it and
/// up `perpendicular3d` of that, held on the marker's node. The game turns the
/// vector into that node's space first; the preview has only the object's rest
/// space here, so it squares the vector there, which is the node's own on an
/// object whose node sits unturned at rest, as an effect anchor's does.
/// `frame` takes the object's space to the world's, so the vector is turned
/// back through it.
fn vector_marker<F: Fn(&RenderModelPreviewMarker) -> (MarkerFrame, usize)>(
    markers: &[RenderModelPreviewMarker],
    at: &str,
    name: &str,
    direction: [f32; 3],
    frame: &F,
) -> RenderModelPreviewMarker {
    let found = markers
        .iter()
        .find(|marker| !at.is_empty() && marker.name.eq_ignore_ascii_case(at));
    let (position, node_index, only_under) = found.map_or(([0.0; 3], 0, None), |marker| {
        (
            marker.position,
            marker.node_index,
            marker.only_under.clone(),
        )
    });
    let probe = RenderModelPreviewMarker {
        node_index,
        axes: WORLD_AXES,
        scale: 1.0,
        ..Default::default()
    };
    let (world, _) = frame(&probe);
    let forward = unit_by_reciprocal(std::array::from_fn(|index| {
        dot(world.axes[index], direction)
    }));
    let up = unit_by_reciprocal(perpendicular(forward));
    RenderModelPreviewMarker {
        name: name.to_owned(),
        node_index,
        position,
        axes: [forward, cross(up, forward), up],
        scale: 1.0,
        only_under,
    }
}

/// What a location of this name lands on: its markers, or, for what no marker
/// answers, the effect's own point. `build_common_effect_vectors` names those
/// directions at the point an event happened, and `root` is the object's.
pub(in crate::app::model_preview) fn part_markers(
    markers: &[RenderModelPreviewMarker],
    name: &str,
    once: bool,
) -> Vec<RenderModelPreviewMarker> {
    let named = markers
        .iter()
        .filter(|marker| marker.name.eq_ignore_ascii_case(name));
    let named: Vec<_> = named.take(MOST_MARKERS).cloned().collect();
    if !named.is_empty() {
        return named;
    }
    if !once && !name.eq_ignore_ascii_case("root") {
        return Vec::new();
    }
    vec![RenderModelPreviewMarker {
        name: name.to_owned(),
        node_index: 0,
        position: [0.0; 3],
        axes: axes_of_vector(effect_vector(name)),
        scale: 1.0,
        only_under: None,
    }]
}

/// Where a named vector points, on an object facing along x with nothing under
/// it but the ground. Anything else, such as `root`, is the object's own.
fn effect_vector(name: &str) -> [f32; 3] {
    match name {
        "gravity" => [0.0, 0.0, -1.0],
        "up" | "normal" => [0.0, 0.0, 1.0],
        "backward" => [-1.0, 0.0, 0.0],
        _ => [1.0, 0.0, 0.0],
    }
}

pub(super) fn axes_of_vector(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let [x, y, z] = forward;
    if z.abs() > 0.5 {
        [forward, [0.0, 1.0, 0.0], [-z, 0.0, x]]
    } else {
        [forward, [-y, x, 0.0], [0.0, 0.0, 1.0]]
    }
}

/// Every effect `object` and its `.model` name, attachments first.
pub(in crate::app::model_preview) fn object_effects(
    reader: &TagReader,
    object: &TagFile,
) -> Vec<ObjectEffect> {
    let mut effects = Vec::new();
    let effect_group = u32::from_be_bytes(*b"effe");
    let vectors = reader.rules().effect_vectors;
    if let Some(block) = object_block(object, "attachments") {
        for attachment in elements(block, "attachments") {
            let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
                continue;
            };
            if group == effect_group && !path.is_empty() {
                effects.push(ObjectEffect {
                    path,
                    marker: marker_name(attachment, "marker"),
                    scale_by: attachment
                        .read_string_id("primary scale")
                        .unwrap_or_default(),
                    secondary_by: attachment
                        .read_string_id("secondary scale")
                        .unwrap_or_default(),
                    change_color: function_slot_of(attachment, "change color"),
                    once: false,
                    vectors,
                });
            }
        }
    }
    let mut named = Vec::new();
    collect_event_effects(object.root(), &mut named);
    let model = object_reference(object, "model")
        .and_then(|path| reader.load(&path, "model", b"hlmt").ok());
    if let Some(model) = &model {
        collect_event_effects(model.root(), &mut named);
    }
    let mut seen = std::collections::HashSet::new();
    for (field, path) in named {
        if seen.insert(path.clone()) {
            effects.push(ObjectEffect {
                scale_by: event_function_name(&field, &path),
                once: !field.contains("looping"),
                path,
                marker: String::new(),
                secondary_by: String::new(),
                change_color: None,
                vectors: false,
            });
        }
    }
    with_nested_effects(reader, effects)
}
