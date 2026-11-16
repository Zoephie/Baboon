//! Event functions and the effects nested inside events.

use super::super::TagReader;
use super::elements;
use super::object::ObjectEffect;
use blam_tags::TagStruct;

pub(super) const WORLD_AXES: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Past any pose, so what holds it stays at rest.
pub(super) const NO_NODE: usize = usize::MAX / 8;

pub(super) const WORLD_ORIENTATION: &str = "use parent position but world orientation";

/// Whether a function is one the preview made to start an event's effect.
pub(in crate::app::model_preview) fn is_event_function(name: &str) -> bool {
    name.rsplit_once(": ")
        .is_some_and(|(field, _)| field.contains("effect") || field.contains("detonation"))
}

/// Hand an attached object's event effect to the object it is on, under its
/// name, so that one panel plays them all.
pub(in crate::app::model_preview) fn adopt_event_function(
    name: &str,
    function: &mut String,
    object: &mut u8,
) {
    if is_event_function(function) {
        *function = format!("{name}: {function}");
        *object = 0;
    }
}

/// How deep an effect's parts may name further effects: the tags go one down.
const MOST_NESTING: usize = 3;

/// Each effect, then those its parts name, at those parts' locations. A part's
/// effect is played by its event, once, under the same function.
pub(super) fn with_nested_effects(
    reader: &TagReader,
    mut effects: Vec<ObjectEffect>,
) -> Vec<ObjectEffect> {
    let effect_group = u32::from_be_bytes(*b"effe");
    let mut level = 0..effects.len();
    for _ in 0..MOST_NESTING {
        let mut nested = Vec::new();
        for outer in &effects[level.clone()] {
            let Ok(tag) = reader.load(&outer.path, "effect", b"effe") else {
                continue;
            };
            let locations = elements(tag.root(), "locations");
            let parts = elements(tag.root(), "events")
                .into_iter()
                .flat_map(|event| elements(event, "parts"));
            for part in parts {
                let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                    continue;
                };
                let location = usize::try_from(part.read_block_index("location")).ok();
                let location = location.and_then(|location| locations.get(location));
                if let (true, false, Some(location)) =
                    (group == effect_group, path.is_empty(), location)
                {
                    let marker = outer.marker_of(*location);
                    nested.push(ObjectEffect {
                        path,
                        marker,
                        once: true,
                        vectors: false,
                        ..outer.clone()
                    });
                }
            }
        }
        let start = effects.len();
        effects.extend(nested);
        level = start..effects.len();
        if level.is_empty() {
            break;
        }
    }
    effects
}

/// The name an event's effect goes by among the object's functions.
pub(in crate::app::model_preview) fn event_function_name(field: &str, path: &str) -> String {
    let stem = path.rsplit('\\').next().unwrap_or(path);
    format!("{field}: {stem}")
}

/// An effect an event plays: the field that names it, its path, and the
/// marker the field beside it names, which its `replace` locations take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app::model_preview) struct EventEffectRef {
    pub(in crate::app::model_preview) field: String,
    pub(in crate::app::model_preview) path: String,
    pub(in crate::app::model_preview) marker: String,
}

/// The marker field that goes with an effect field in the same element: an
/// instant response's `transition effect` takes `effect marker name` (Reach
/// splits it into `generic effect marker` and `specific effect marker`); a
/// `delay effect`, `looping effect` or `pre recharge effect` takes the field
/// of its own name plus `marker name` or `marker`. Empty when there is none,
/// and the effect plays at the object's origin.
pub(in crate::app::model_preview) fn effect_marker_beside(
    holder: TagStruct<'_>,
    effect_field: &str,
) -> String {
    let mut candidates = vec![
        format!("{effect_field} marker name"),
        format!("{effect_field} marker"),
    ];
    if effect_field.starts_with("transition effect") {
        if effect_field.contains("(specific)") {
            candidates.push("specific effect marker".to_owned());
        } else {
            candidates.push("generic effect marker".to_owned());
        }
        candidates.push("effect marker name".to_owned());
    }
    candidates
        .iter()
        .map(|field| super::placement::marker_name(holder, field))
        .find(|marker| !marker.is_empty())
        .unwrap_or_default()
}

/// Every effect reference under `holder`, by the field that names it, leaving
/// out `attachments`, which run on their own.
pub(in crate::app::model_preview) fn collect_event_effects(
    holder: TagStruct<'_>,
    found: &mut Vec<EventEffectRef>,
) {
    let effect_group = u32::from_be_bytes(*b"effe");
    for field in holder.fields() {
        let name = field.name();
        if name == "attachments" {
            continue;
        }
        if field.field_type() == blam_tags::TagFieldType::TagReference {
            if let Some((group, path)) = holder.read_tag_ref_with_group(name)
                && group == effect_group
                && !path.is_empty()
            {
                found.push(EventEffectRef {
                    field: name.to_owned(),
                    path,
                    marker: effect_marker_beside(holder, name),
                });
            }
        } else if let Some(child) = field.as_struct() {
            collect_event_effects(child, found);
        } else if let Some(block) = field.as_block() {
            block
                .iter()
                .for_each(|element| collect_event_effects(element, found));
        } else if let Some(array) = field.as_array() {
            array
                .iter()
                .for_each(|element| collect_event_effects(element, found));
        }
    }
}
