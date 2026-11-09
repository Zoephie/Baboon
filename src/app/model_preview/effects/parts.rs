//! An effect's parts: which run, for whom, and what each makes.

use super::*;

/// An event's part on an attachment's effect.
pub(in crate::app::model_preview) struct EffectPart<'a> {
    pub(in crate::app::model_preview) effect: &'a ObjectEffect,
    pub(in crate::app::model_preview) part: TagStruct<'a>,
    pub(in crate::app::model_preview) group: u32,
    pub(in crate::app::model_preview) path: String,
    /// The marker its location resolves to, by name.
    pub(in crate::app::model_preview) marker: String,
    /// `secondary location (beams)`, by name.
    pub(in crate::app::model_preview) secondary_marker: Option<String>,
    /// `use parent position but world orientation`, of its effect.
    pub(in crate::app::model_preview) world_oriented: bool,
    /// `never attached to object`: it stays where it began.
    pub(in crate::app::model_preview) unattached: bool,
    /// `A scales values` takes in the attachment's scale.
    pub(in crate::app::model_preview) scaled: bool,
    /// `B scales values` does, for the secondary scale.
    pub(in crate::app::model_preview) b_scaled: bool,
    pub(in crate::app::model_preview) times: Arc<EffectTimes>,
    /// Which of the effect's events it belongs to.
    pub(in crate::app::model_preview) event: usize,
}

/// Whether `event_generate_parts` makes a part on an object in third person:
/// out of the water, and not asked to be nonviolent.
pub(super) fn part_runs(part: TagStruct<'_>) -> bool {
    part_runs_by(part, "create in", part_violence(part))
}

/// Where a part keeps its violence mode. Halo 2 names both of its part's
/// settings `create in`, and the second is the violence (`halo2_tag_test`
/// 0x515FC0 reads the part's second short). Only where there is no `violence
/// mode`: an ordinal that misses falls back to the first `create in`.
pub(super) fn part_violence(part: TagStruct<'_>) -> &'static str {
    if part.field("violence mode").is_some() { "violence mode" } else { "create in#1" }
}

/// `part_runs`, for parts and particle systems, which name these two apart.
/// Each is a field path, so a `#ordinal` picks one of two of a name.
pub(in crate::app::model_preview) fn part_runs_by(part: TagStruct<'_>, environment: &str, violence: &str) -> bool {
    let named = |path: &str| match part.field_path(path).and_then(|field| field.value()) {
        Some(
            TagFieldData::CharEnum { name, .. }
            | TagFieldData::ShortEnum { name, .. }
            | TagFieldData::LongEnum { name, .. },
        ) => name.unwrap_or_default(),
        _ => String::new(),
    };
    let environment = named(environment);
    matches!(environment.as_str(), "" | ANYWHERE | AIR)
        && named(violence) != ONLY_NONVIOLENT
        && named("camera mode") != ONLY_FIRST_PERSON
        && !part_disabled(part)
}

/// `disabled for debugging`, which the cache build's `s_effect_scrubber::scrub`
/// drops.
fn part_disabled(part: TagStruct<'_>) -> bool {
    has_flag(part, "flags", DISABLED)
}

/// A `cheap_particle_emitter` part's group.
pub(in crate::app::model_preview) const CHEAP_PARTICLE_EMITTER: u32 = u32::from_be_bytes(*b"cpem");

/// Whether a part of `group` is made, given whether it passes `part_runs`
/// (`generated`: environment, violence, camera and the scrub) and whether it
/// is `disabled`. Reach's `effects_submit_cheap_attachments` (the 360's
/// 0x82EA9570) submits a cheap particle part on its location and tag alone,
/// so only the scrubber's drop keeps one out.
pub(super) fn part_made(group: u32, generated: bool, disabled: bool) -> bool {
    if group == CHEAP_PARTICLE_EMITTER { !disabled } else { generated }
}

/// Every part of every effect `object` attaches, with its effect held open.
pub(in crate::app::model_preview) fn for_each_attached_part(
    reader: &TagReader,
    object: &TagFile,
    mut visit: impl for<'a> FnMut(EffectPart<'a>),
) {
    for object_effect in object_effects(reader, object) {
        let Ok(effect) = reader.load(&object_effect.path, "effect", b"effe") else {
            continue;
        };
        let locations = elements(effect.root(), "locations");
        let times = Arc::new(object_effect.times_of(effect.root()));
        let world_oriented = ObjectEffect::world_oriented(effect.root());
        for (at, event) in elements(effect.root(), "events").into_iter().enumerate() {
            for part in elements(event, "parts") {
                let Some((group, path)) = part.read_tag_ref_with_group("type") else {
                    continue;
                };
                let location = usize::try_from(part.read_block_index("location")).ok();
                let location = location.and_then(|location| locations.get(location));
                let made = part_made(group, part_runs(part), part_disabled(part));
                let (false, Some(location), true) = (path.is_empty(), location, made) else {
                    continue;
                };
                let marker = object_effect.marker_of(*location);
                let secondary = usize::try_from(part.read_block_index("secondary location (beams)")).ok();
                let secondary_marker = secondary.and_then(|at| locations.get(at)).map(|at| marker_name(*at, "marker name"));
                let unattached = has_flag(part, "flags", "never attached to object");
                let scaled = has_flag(part, "A scales values", PART_SCALE);
                let b_scaled = has_flag(part, "B scales values", PART_SCALE);
                let times = Arc::clone(&times);
                visit(EffectPart {
                    effect: &object_effect,
                    part,
                    group,
                    path,
                    marker,
                    secondary_marker,
                    world_oriented,
                    unattached,
                    scaled,
                    b_scaled,
                    times,
                    event: at,
                });
            }
        }
    }
}

impl EffectPart<'_> {
    /// Where a marker puts this part: its frame and the node it follows.
    pub(in crate::app::model_preview) fn placed(&self, place: MarkerFrame, node: usize) -> (MarkerFrame, usize) {
        let place = if self.world_oriented { MarkerFrame { axes: WORLD_AXES, ..place } } else { place };
        (place, if self.unattached { NO_NODE } else { node })
    }
}
