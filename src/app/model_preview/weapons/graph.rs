//! What a unit's animation graph says about the weapons it holds: modes, overlays, IK and arms.

use super::super::{RenderModelPreview, TagReader};
use super::unit::WeaponOption;
use super::{elements, has_flag, holder_of};
use blam_tags::{TagFile, TagStruct};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The mode a unit nobody controls is in.
const RESTING_MODE: &str = "combat";

/// Every level of the graph falls back on this.
pub(super) const ANY: &str = "any";

pub(super) const UNARMED: &str = "unarmed";

/// `unit_animation_set_weapon`'s class with something in the left hand.
pub(super) const DUAL: &str = "dual";

const IDLE: &str = "idle";

/// Base actions `unit_weapon_ik_allowed` turns weapon IK off for.
pub(super) const NO_IK_ACTIONS: &[&str] = &[
    "flying_front",
    "flying_back",
    "flying_left",
    "flying_right",
    "user_animation",
    "resurrect_front",
    "resurrect_back",
    "melee_continuous",
    "leap_start",
    "leap_airborne",
    "airborne_dead",
    "landing_dead",
    "exit",
    "h_ping",
    // `dword_D4B190`, the entries of a seat.
    "enter",
    "enter_a",
    "enter_b",
    "enter_c",
    "enter_d",
];

/// The only overlays `unit_weapon_ik_allowed` leaves weapon IK on through.
pub(super) const IK_OVERLAYS: &[&str] = &[
    "fire_1",
    "fire_2",
    "chamber_1",
    "chamber_2",
    "charged_1",
    "charged_2",
    "melee",
    "melee_1sthit",
    "melee_1sthit2idle",
    "melee_2ndhit",
    "melee_2ndhit2idle",
    "melee_3rdhit",
    "melee_airborne",
    "melee_back",
    "melee_dash",
    "melee_dash_airborne",
    "melee_flurry",
    "melee_lunge",
    "melee_lunge_airborne",
    "melee_off_hand",
    "melee_strike_1",
    "melee_strike_2",
    "melee_strike_3",
    "melee_strike_4",
    "melee_tackle",
    "leap_melee",
];

/// Overlays of the aiming screens run on a channel of their own.
pub(super) const AIM_OVERLAY: &str = "aim_";

pub(super) const GRIP: &str = "grip";

pub(super) const ARM_OFFSETS: &str = "arm_offsets";

pub(super) const CAN_BE_DUAL: &str = "can be dual wielded";

pub(super) const ONLY_DUAL: &str = "can only be dual wielded";

/// A weapon a unit's animations can hold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Equippable {
    pub(crate) option: WeaponOption,
    pub(super) kind: String,
    pub(super) dual: bool,
    pub(super) only_dual: bool,
}

/// One `mode:class:type` of a graph.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphEntry {
    pub(super) mode: String,
    pub(super) class: String,
    pub(super) kind: String,
    pub(super) has_idle: bool,
    /// Its actions and overlays by label, with the clip each names, whatever
    /// that is called (the assault rifle's `fire_1` is `combat:rifle:br:fire_1`),
    /// or none for one in another graph.
    pub(super) slots: Vec<(String, Option<String>)>,
}

/// Which nodes an overlay channel reaches: the graph's `left arm member`
/// nodes, or all the others.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Arm {
    Left,
    NotLeft,
}

/// What `unit_preprocess_node_orientations` lays over a clip for what is held.
/// Aiming and looking are not among it: dead ahead at full weight it skips them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PoseLayer {
    pub(crate) clip: String,
    pub(crate) arm: Option<Arm>,
}

/// One `weapon ik` entry of a graph.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphIk {
    pub(super) mode: String,
    pub(super) class: String,
    pub(super) marker: String,
    pub(super) attach_to: String,
}

/// A weapon type's actions and overlays. Reach puts them a level down, in
/// the type's `sets`, where `any` is the set a unit is in until something
/// names another; earlier graphs keep them on the type.
fn type_slots<'a>(kind: TagStruct<'a>) -> (Vec<TagStruct<'a>>, Vec<TagStruct<'a>>) {
    let Some(sets) = kind.field("sets").and_then(|field| field.as_block()) else {
        return (elements(kind, "actions"), elements(kind, "overlays"));
    };
    let set = (0..sets.len())
        .filter_map(|index| sets.element(index))
        .find(|set| set.read_string_id("label").as_deref() == Some(ANY));
    set.map_or_else(Default::default, |set| {
        (
            elements(set, "actions"),
            elements(set, "overlay animations"),
        )
    })
}

pub(super) fn graph_entries(graph: &TagFile) -> Vec<GraphEntry> {
    let mut entries = Vec::new();
    let Some(holder) = holder_of(graph.root(), "modes") else {
        return entries;
    };
    let label = |entry: TagStruct<'_>| entry.read_string_id("label").unwrap_or_default();
    let names: Vec<String> = holder_of(graph.root(), "animations")
        .map(|holder| elements(holder, "animations"))
        .unwrap_or_default()
        .iter()
        .map(|animation| animation.read_string_id("name").unwrap_or_default())
        .collect();
    let clip_of = |slot: &TagStruct<'_>| {
        let reference = slot.descend("animation")?;
        // Only a graph's own: what `graph index` names lies in another.
        if reference
            .read_int_any("graph index")
            .is_some_and(|graph| graph >= 0)
        {
            return None;
        }
        names
            .get(usize::try_from(reference.read_block_index("animation")).ok()?)
            .cloned()
    };
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                let (actions, overlays) = type_slots(kind);
                entries.push(GraphEntry {
                    slots: actions
                        .iter()
                        .chain(&overlays)
                        .map(|slot| (label(*slot), clip_of(slot)))
                        .collect(),
                    mode: label(mode),
                    class: label(class),
                    kind: label(kind),
                    has_idle: actions.iter().any(|action| label(*action) == IDLE),
                });
            }
        }
    }
    entries
}

pub(super) fn graph_ik(graph: &TagFile) -> Vec<GraphIk> {
    let mut entries = Vec::new();
    let Some(holder) = holder_of(graph.root(), "modes") else {
        return entries;
    };
    let name = |entry: TagStruct<'_>, field: &str| entry.read_string_id(field).unwrap_or_default();
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for ik in elements(class, "weapon ik") {
                entries.push(GraphIk {
                    mode: name(mode, "label"),
                    class: name(class, "label"),
                    marker: name(ik, "marker"),
                    attach_to: name(ik, "attach to marker"),
                });
            }
        }
    }
    entries
}

pub(super) fn graph_overlays(graph: &TagFile) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    let Some(holder) = holder_of(graph.root(), "modes") else {
        return labels;
    };
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                let (_, overlays) = type_slots(kind);
                labels.extend(
                    overlays
                        .iter()
                        .filter_map(|overlay| overlay.read_string_id("label")),
                );
            }
        }
    }
    labels
}

/// Which of a graph's nodes the `left arm member` channel reaches.
pub(in crate::app::model_preview) fn left_arm_nodes(graph: &TagFile) -> Vec<bool> {
    let Some(holder) = holder_of(graph.root(), "skeleton nodes") else {
        return Vec::new();
    };
    let member = |node: &TagStruct<'_>| has_flag(*node, "model flags", "left arm member");
    elements(holder, "skeleton nodes")
        .iter()
        .map(member)
        .collect()
}

pub(super) fn clips_without_ik(graph: &TagFile) -> BTreeSet<String> {
    let Some(holder) = holder_of(graph.root(), "animations") else {
        return BTreeSet::new();
    };
    let flagged = elements(holder, "animations").into_iter().filter(|clip| {
        // Reach and Halo 4 call them `user flags`.
        has_flag(*clip, "playback flags", "disable weapon ik")
            || has_flag(*clip, "user flags", "disable weapon ik")
    });
    flagged
        .filter_map(|clip| clip.read_string_id("name"))
        .collect()
}

/// `unit_can_use_weapon`, in the mode a unit rests in: an idle for the class,
/// under the weapon's own type or `any`.
pub(super) fn accepts(graph: &[GraphEntry], class: &str, kind: &str) -> bool {
    graph.iter().any(|entry| {
        entry.has_idle
            && entry.mode == RESTING_MODE
            && entry.class == class
            && (entry.kind == kind || entry.kind == ANY)
    })
}

pub(super) fn string_id_in(tag: &TagFile, name: &str) -> String {
    holder_of(tag.root(), name)
        .and_then(|holder| holder.read_string_id(name))
        .unwrap_or_default()
}

/// Every node below `node`.
pub(super) fn descendants(preview: &RenderModelPreview, node: usize) -> Vec<usize> {
    // No chain is longer than the node count; one that is loops back on itself.
    let below = |mut index: usize| {
        for _ in 0..preview.nodes.len() {
            let Some(parent) = preview
                .nodes
                .get(index)
                .and_then(|at| usize::try_from(at.parent).ok())
            else {
                return false;
            };
            if parent == node {
                return true;
            }
            index = parent;
        }
        false
    };
    (0..preview.nodes.len())
        .filter(|index| below(*index))
        .collect()
}

/// A unit's graph, by way of its `.model`.
pub(super) fn unit_graph(reader: &TagReader, unit: &TagFile) -> Option<Arc<TagFile>> {
    let model = holder_of(unit.root(), "model")?.read_tag_ref_path("model")?;
    let model = reader.load(&model, "model", b"hlmt").ok()?;
    let graph = model
        .root()
        .read_tag_ref_path("animation")
        .filter(|path| !path.is_empty())?;
    reader.load(&graph, "model_animation_graph", b"jmad").ok()
}
