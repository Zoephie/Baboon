//! Where Halo CE effects hang on an object: its attachments' effects and those its events name.

use super::*;

/// Where a location sits on the model: one per marker of its name.
#[derive(Debug, Clone, Copy)]
pub(super) struct CeLocation {
    pub(super) node: usize,
    pub(super) position: [f32; 3],
    /// Forward, left and up.
    pub(super) axes: [[f32; 3]; 3],
}

/// What `effect_new_unattached_from_markers` gives an effect a particle
/// makes: its scales, its color, and its velocity per tick.
#[derive(Debug, Clone, Copy)]
pub(super) struct CeSpawn {
    pub(super) scales: [f32; 2],
    pub(super) color: [f32; 3],
    pub(super) velocity: [f32; 3],
}

/// An effect hung on the object by one of its `attachments`, one the object
/// names for an event, or made by a particle on its own.
#[derive(Debug, Clone)]
pub(crate) struct CeEffectAt {
    pub(crate) object: u8,
    pub(crate) def: Arc<CeEffectDef>,
    /// What made it, if not an attachment: it then runs its events once.
    pub(super) spawn: Option<CeSpawn>,
    /// When it is one the object names for an event (`firing effect`,
    /// `shield depleted effect`, ...): it never runs itself, and each play
    /// runs a copy, as each event makes a new one.
    pub(crate) event: Option<CeEventEffect>,
    /// `A out`.. of `primary scale`, `secondary scale` and `change color`.
    pub(super) primary: Option<usize>,
    pub(super) secondary: Option<usize>,
    pub(super) change_color: Option<usize>,
    pub(super) locations: Vec<Vec<CeLocation>>,
}

/// An effect an object names for an event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CeEventEffect {
    /// What the panel plays it by: the field that names it, and its name.
    pub(crate) name: String,
    pub(crate) path: String,
}

impl super::super::functions::ReadsOutputs for CeEffectAt {
    fn object(&self) -> u8 {
        self.object
    }
    /// One a particle or an event made reads none.
    fn outputs_read(&self) -> Vec<usize> {
        if self.spawn.is_some() || self.event.is_some() {
            return Vec::new();
        }
        [self.primary, self.secondary].into_iter().flatten().collect()
    }
}

/// The effects among `object`'s attachments.
pub(in crate::app::model_preview) fn read_ce_effects(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeEffectAt> {
    let Some(block) = object_block(object, "attachments") else {
        return Vec::new();
    };
    let effect_group = u32::from_be_bytes(*b"effe");
    let mut effects = Vec::new();
    let mut cache = CeEffectCache::default();
    let cache = &mut cache;
    for attachment in elements(block, "attachments") {
        let Some((group, path)) = attachment.read_tag_ref_with_group("type") else {
            continue;
        };
        if group != effect_group || path.is_empty() {
            continue;
        }
        let Some(def) = cache.effect(reader, &path) else {
            continue;
        };
        let locations = locate(&def, markers);
        effects.push(CeEffectAt {
            object: owner,
            def,
            spawn: None,
            event: None,
            primary: function_slot_of(attachment, "primary scale"),
            secondary: function_slot_of(attachment, "secondary scale"),
            change_color: function_slot_of(attachment, "change color"),
            locations,
        });
    }
    effects
}

/// `effect_build_locations` with `object_get_marker_by_name`: each of an
/// effect's locations at every marker of its name; no name is the origin, a
/// missing one nothing at all.
fn locate(def: &CeEffectDef, markers: &[RenderModelPreviewMarker]) -> Vec<Vec<CeLocation>> {
    def.locations
        .iter()
        .map(|name| {
            if name.is_empty() {
                return vec![CeLocation { node: 0, position: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] }];
            }
            markers
                .iter()
                .filter(|marker| marker.name.eq_ignore_ascii_case(name))
                .map(|marker| CeLocation {
                    node: marker.node_index.max(0) as usize,
                    position: marker.position,
                    axes: marker.axes,
                })
                .collect()
        })
        .collect()
}

/// The effects `object` names for its events (every effect reference but
/// its `attachments`), each by the name the panel plays it by. The game
/// makes them with `effect_new_from_object` on the object's markers: not
/// looping, white, and deleted once their events have run.
pub(in crate::app::model_preview) fn read_ce_event_effects(
    reader: &TagReader,
    object: &TagFile,
    owner: u8,
    markers: &[RenderModelPreviewMarker],
) -> Vec<CeEffectAt> {
    let mut named = Vec::new();
    super::super::super::effects::collect_event_effects(object.root(), &mut named);
    let mut cache = CeEffectCache::default();
    let mut seen = std::collections::HashSet::new();
    let mut effects = Vec::new();
    for (field, path) in named {
        if !seen.insert(path.clone()) {
            continue;
        }
        // One with nothing the preview draws, such as a sound alone, has
        // nothing to play.
        let Some(def) = cache.effect(reader, &path) else {
            continue;
        };
        let locations = locate(&def, markers);
        effects.push(CeEffectAt {
            object: owner,
            def,
            spawn: None,
            event: Some(CeEventEffect { name: super::super::super::effects::event_function_name(&field, &path), path: path.clone() }),
            primary: None,
            secondary: None,
            change_color: None,
            locations,
        });
    }
    effects
}
