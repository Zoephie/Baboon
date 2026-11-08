//! What a unit of Halo 2 or a later game can hold, and how it holds it. A
//! weapon names its own animation class and type, and `unit_can_use_weapon`
//! lets it in when the unit's graph has an idle for them. Dual wielding is a
//! class of its own, `dual`, once a second weapon is in the left hand. What
//! an attached object brings with it is in [`attached`], and a variant's
//! child objects in [`variants`].

use std::collections::BTreeSet;

use blam_tags::{TagFile, TagStruct};

use super::attachment::{MarkerFrame, WeaponOption};
use super::ce::attachments::{loose_weapon_paths, weapon_ik};
use super::common::fields::{elements, has_flag};
use super::attachment::{attach_point, attach_vector};
use super::load::build_render_preview;
use super::ce::attachments::CeWeaponIk;
use super::*;
use crate::core::source::TagSource;

pub(in crate::app::model_preview) mod attached;
pub(in crate::app::model_preview) mod variants;

use attached::{AttachedObject, fold_attached};
use variants::object_variant;

/// The mode a unit nobody controls is in.
const RESTING_MODE: &str = "combat";
/// Every level of the graph falls back on this.
const ANY: &str = "any";
const UNARMED: &str = "unarmed";
/// `unit_animation_set_weapon`'s class with something in the left hand.
const DUAL: &str = "dual";
const IDLE: &str = "idle";
/// Base actions `unit_weapon_ik_allowed` turns weapon IK off for.
const NO_IK_ACTIONS: &[&str] = &[
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
const IK_OVERLAYS: &[&str] = &[
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
const AIM_OVERLAY: &str = "aim_";
const GRIP: &str = "grip";
const ARM_OFFSETS: &str = "arm_offsets";
const CAN_BE_DUAL: &str = "can be dual wielded";
const ONLY_DUAL: &str = "can only be dual wielded";

/// A weapon a unit's animations can hold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Equippable {
    pub(crate) option: WeaponOption,
    kind: String,
    dual: bool,
    only_dual: bool,
}

/// One `mode:class:type` of a graph.
#[derive(Debug, Clone, PartialEq)]
struct GraphEntry {
    mode: String,
    class: String,
    kind: String,
    has_idle: bool,
    /// Its actions and overlays by label, with the clip each names, whatever
    /// that is called (the assault rifle's `fire_1` is `combat:rifle:br:fire_1`),
    /// or none for one in another graph.
    slots: Vec<(String, Option<String>)>,
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
struct GraphIk {
    mode: String,
    class: String,
    marker: String,
    attach_to: String,
}

/// What a Halo 2 unit can hold, and which clips what it holds leaves out.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitWeapons {
    pub(crate) equippable: Vec<Equippable>,
    graph: Vec<GraphEntry>,
    ik: Vec<GraphIk>,
    /// Clips that set `disable weapon ik`.
    ik_off: BTreeSet<String>,
    /// Every overlay label in the graph.
    overlays: BTreeSet<String>,
    /// Filled once the clips are listed.
    pub(crate) hidden_clips: BTreeSet<usize>,
}

impl UnitWeapons {
    pub(crate) fn primaries(&self) -> Vec<WeaponOption> {
        let usable = self.equippable.iter().filter(|weapon| !weapon.only_dual);
        usable.map(|weapon| weapon.option.clone()).collect()
    }

    /// What can go in the left hand, as `unit_get_weapon_pickup_options` has
    /// it: only with a primary that can pair, and only what can pair with it.
    pub(crate) fn secondaries(&self, primary: Option<&str>) -> Vec<WeaponOption> {
        let held = self.find(primary);
        if !held.is_some_and(|held| held.dual) || !self.has_dual() {
            return Vec::new();
        }
        let usable = self.equippable.iter().filter(|weapon| weapon.dual);
        usable.map(|weapon| weapon.option.clone()).collect()
    }

    /// Whether the graph has anything for two weapons.
    fn has_dual(&self) -> bool {
        self.graph.iter().any(|entry| entry.class == DUAL && entry.has_idle)
    }

    fn find(&self, path: Option<&str>) -> Option<&Equippable> {
        self.equippable.iter().find(|weapon| Some(weapon.option.path.as_str()) == path)
    }

    /// `animation_weapon_ik_iterator_next`: each mode's entries for `class`,
    /// then for `any`, then the same under the `any` mode.
    fn ik_of(&self, mode: &str, class: &str) -> Vec<&GraphIk> {
        let mut walked = Vec::new();
        for (mode, class) in [(mode, class), (mode, ANY), (ANY, class), (ANY, ANY)] {
            walked.extend(self.ik.iter().filter(|ik| ik.mode == mode && ik.class == class));
        }
        walked
    }

    fn modes(&self) -> BTreeSet<&str> {
        self.graph.iter().map(|entry| entry.mode.as_str()).collect()
    }

    /// `unit_weapon_ik_allowed`, for a clip of `mode` played on its own.
    pub(crate) fn ik_runs_through(&self, clip: &str, mode: &str) -> bool {
        let name = clip.split('%').next().unwrap_or(clip);
        let last = name.rsplit(':').next().unwrap_or(name);
        let overlay = self.overlays.contains(last)
            && !last.starts_with(AIM_OVERLAY)
            && !IK_OVERLAYS.contains(&last);
        name.split(':').next() == Some(mode)
            && !self.ik_off.contains(name)
            && !NO_IK_ACTIONS.contains(&last)
            && !overlay
    }

    /// `sub_616470`'s search, `sub_54FDE0` over `animation_search_path_iterator`
    /// (`sub_675570`): the first of the graph's `mode:class:type` entries with
    /// a slot labelled `last`, trying `any` for the type, then the class, then
    /// both, then the same under the `any` mode, and skipping a step that asks
    /// for `any` again. Its clip, if the preview has it: one in another graph
    /// still ends the search.
    fn find_clip(&self, clips: &[&str], mode: &str, class: &str, kind: &str, last: &str) -> Option<String> {
        for step in 0..8u8 {
            let anything = |bit: u8, value: &str| step & bit != 0 && value == ANY;
            if anything(4, mode) || anything(2, class) || anything(1, kind) {
                continue;
            }
            let mode = if step & 4 != 0 { ANY } else { mode };
            let class = if step & 2 != 0 { ANY } else { class };
            let kind = if step & 1 != 0 { ANY } else { kind };
            let entry = self.graph.iter().find(|entry| entry.mode == mode && entry.class == class && entry.kind == kind);
            if let Some((_, clip)) = entry.and_then(|entry| entry.slots.iter().find(|(label, _)| label == last)) {
                return clip.as_deref().filter(|clip| clips.contains(clip)).map(str::to_owned);
            }
        }
        None
    }

    /// The overlays `unit_animation_cache_overlays` leaves running over `clip`.
    /// One weapon's `grip` reaches the whole body while weapon IK blends in, and
    /// stops short of the left arm when it does not; two weapons each lay
    /// `arm_offsets`, then `grip`, on their own side.
    pub(crate) fn pose_layers(
        &self,
        clips: &[&str],
        clip: &str,
        primary: Option<&str>,
        secondary: Option<&str>,
    ) -> Vec<PoseLayer> {
        let name = clip.split('%').next().unwrap_or(clip);
        let mode = name.split(':').next().unwrap_or(name);
        let layer = |class: &str, kind: &str, last: &str, arm| {
            let clip = self.find_clip(clips, mode, class, kind, last)?;
            Some(PoseLayer { clip, arm })
        };
        match (self.find(primary), self.find(secondary)) {
            (Some(first), Some(second)) => {
                let right = (first.kind.as_str(), Arm::NotLeft);
                let left = (second.kind.as_str(), Arm::Left);
                [(ARM_OFFSETS, right), (ARM_OFFSETS, left), (GRIP, right), (GRIP, left)]
                    .into_iter()
                    .filter_map(|(last, (kind, arm))| layer(DUAL, kind, last, Some(arm)))
                    .collect()
            }
            (Some(held), None) => {
                let arm = (!self.ik_runs_through(clip, mode)).then_some(Arm::NotLeft);
                layer(&held.option.class, &held.kind, GRIP, arm).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    /// The class and types `unit_animation_set_weapon` puts the unit in.
    pub(crate) fn held(&self, primary: Option<&str>, secondary: Option<&str>) -> (String, Vec<String>) {
        let (first, second) = (self.find(primary), self.find(secondary));
        match (first, second) {
            (Some(first), Some(second)) => {
                (DUAL.to_owned(), vec![first.kind.clone(), second.kind.clone()])
            }
            (Some(first), None) => (first.option.class.clone(), vec![first.kind.clone()]),
            _ => (UNARMED.to_owned(), Vec::new()),
        }
    }

    /// Clips of what is not held. A name leaves `any` out, as `combat:rifle:idle`
    /// beside `combat:rifle:pr:idle`, so its words are read against the graph.
    /// A mode with no clips for this class keeps all of its own, and what is
    /// under `any` is for anything, as the game's search falls through to it.
    /// With nothing held every clip stays.
    pub(crate) fn hidden(&self, clips: &[&str], class: &str, kinds: &[String]) -> BTreeSet<usize> {
        if class == UNARMED {
            return BTreeSet::new();
        }
        let in_graph = |mode: &str, class: &str| {
            self.graph.iter().any(|entry| entry.mode == mode && entry.class == class)
        };
        let is_kind = |mode: &str, class: &str, kind: &str| {
            self.graph.iter().any(|entry| {
                entry.mode == mode && entry.class == class && entry.kind == kind
            })
        };
        let has_clips = |mode: &str| {
            let prefix = format!("{mode}:{class}:");
            clips.iter().any(|clip| clip.starts_with(&prefix))
        };
        // What the held weapon's own slots point at is its, by any name.
        let slotted: BTreeSet<&str> = self
            .graph
            .iter()
            .filter(|entry| entry.class == class && kinds.contains(&entry.kind))
            .flat_map(|entry| entry.slots.iter().filter_map(|(_, clip)| clip.as_deref()))
            .collect();
        let mut hidden = BTreeSet::new();
        for (index, clip) in clips.iter().enumerate() {
            if slotted.contains(clip) {
                continue;
            }
            let words: Vec<_> = clip.split(':').collect();
            let [mode, clip_class, rest @ ..] = words.as_slice() else {
                continue;
            };
            if !in_graph(mode, clip_class) || !has_clips(mode) {
                continue;
            }
            let of_kind = match rest {
                [kind, _, ..] if is_kind(mode, clip_class, kind) => Some(*kind),
                _ => None,
            };
            let held = of_kind.is_none_or(|kind| kind == ANY || kinds.iter().any(|held| held == kind));
            if (*clip_class != class && *clip_class != ANY) || !held {
                hidden.insert(index);
            }
        }
        hidden
    }
}

/// The struct holding a field, wherever in the tag it sits.
pub(in crate::app::model_preview) fn holder_of<'a>(from: TagStruct<'a>, field: &str) -> Option<TagStruct<'a>> {
    if from.field(field).is_some() {
        return Some(from);
    }
    from.fields_all().filter_map(|field| field.as_struct()).find_map(|inner| holder_of(inner, field))
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
    set.map_or_else(Default::default, |set| (elements(set, "actions"), elements(set, "overlay animations")))
}

fn graph_entries(graph: &TagFile) -> Vec<GraphEntry> {
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
        if reference.read_int_any("graph index").is_some_and(|graph| graph >= 0) {
            return None;
        }
        names.get(usize::try_from(reference.read_block_index("animation")).ok()?).cloned()
    };
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                let (actions, overlays) = type_slots(kind);
                entries.push(GraphEntry {
                    slots: actions.iter().chain(&overlays).map(|slot| (label(*slot), clip_of(slot))).collect(),
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

fn graph_ik(graph: &TagFile) -> Vec<GraphIk> {
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

fn graph_overlays(graph: &TagFile) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    let Some(holder) = holder_of(graph.root(), "modes") else {
        return labels;
    };
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                let (_, overlays) = type_slots(kind);
                labels.extend(overlays.iter().filter_map(|overlay| overlay.read_string_id("label")));
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
    let member = |node: &TagStruct<'_>| {
        has_flag(*node, "model flags", "left arm member")
    };
    elements(holder, "skeleton nodes").iter().map(member).collect()
}

fn clips_without_ik(graph: &TagFile) -> BTreeSet<String> {
    let Some(holder) = holder_of(graph.root(), "animations") else {
        return BTreeSet::new();
    };
    let flagged = elements(holder, "animations").into_iter().filter(|clip| {
        // Reach and Halo 4 call them `user flags`.
        has_flag(*clip, "playback flags", "disable weapon ik") || has_flag(*clip, "user flags", "disable weapon ik")
    });
    flagged.filter_map(|clip| clip.read_string_id("name")).collect()
}

/// `unit_can_use_weapon`, in the mode a unit rests in: an idle for the class,
/// under the weapon's own type or `any`.
fn accepts(graph: &[GraphEntry], class: &str, kind: &str) -> bool {
    graph.iter().any(|entry| {
        entry.has_idle
            && entry.mode == RESTING_MODE
            && entry.class == class
            && (entry.kind == kind || entry.kind == ANY)
    })
}

fn string_id_in(tag: &TagFile, name: &str) -> String {
    holder_of(tag.root(), name).and_then(|holder| holder.read_string_id(name)).unwrap_or_default()
}

/// Every node below `node`.
fn descendants(preview: &RenderModelPreview, node: usize) -> Vec<usize> {
    // No chain is longer than the node count; one that is loops back on itself.
    let below = |mut index: usize| {
        for _ in 0..preview.nodes.len() {
            let Some(parent) = preview.nodes.get(index).and_then(|at| usize::try_from(at.parent).ok()) else {
                return false;
            };
            if parent == node {
                return true;
            }
            index = parent;
        }
        false
    };
    (0..preview.nodes.len()).filter(|index| below(*index)).collect()
}

/// A unit's graph, by way of its `.model`.
fn unit_graph(reader: &TagReader, unit: &TagFile) -> Option<Arc<TagFile>> {
    let model = holder_of(unit.root(), "model")?.read_tag_ref_path("model")?;
    let model = reader.load(&model, "model", b"hlmt").ok()?;
    let graph = model.root().read_tag_ref_path("animation").filter(|path| !path.is_empty())?;
    reader.load(&graph, "model_animation_graph", b"jmad").ok()
}

/// What a unit can hold, from a loose folder's weapons.
pub(in crate::app::model_preview) fn unit_weapons(
    reader: &TagReader,
    unit: &TagFile,
    known: Option<&[Equippable]>,
) -> UnitWeapons {
    let Some(tag) = unit_graph(reader, unit) else {
        return UnitWeapons::default();
    };
    let graph = graph_entries(&tag);
    let equippable = known.map_or_else(
        || {
            let TagSource::LooseFolder { root, .. } = &reader.source else {
                return Vec::new();
            };
            let mut found: Vec<_> = loose_weapon_paths(root)
                .into_iter()
                .filter_map(|path| {
                    let weapon =
                        reader.load(&path, "weapon", b"weap").ok()?;
                    let class = string_id_in(&weapon, "weapon class");
                    let kind = string_id_in(&weapon, "weapon name");
                    accepts(&graph, &class, &kind).then(|| {
                        // The weapon's own `flags` sit beside its `magazines`.
                        let holder = holder_of(weapon.root(), "magazines");
                        let has = |flag: &str| holder.is_some_and(|holder| has_flag(holder, "flags", flag));
                        Equippable {
                            option: WeaponOption {
                                name: tag_path::name(&path).to_owned(),
                                path,
                                class,
                            },
                            kind,
                            dual: has(CAN_BE_DUAL) || has(ONLY_DUAL),
                            only_dual: has(ONLY_DUAL),
                        }
                    })
                })
                .collect();
            found.sort_by(|a, b| {
                (&a.option.class, &a.option.name).cmp(&(&b.option.class, &b.option.name))
            });
            found
        },
        <[_]>::to_vec,
    );
    UnitWeapons {
        equippable,
        graph,
        ik: graph_ik(&tag),
        ik_off: clips_without_ik(&tag),
        overlays: graph_overlays(&tag),
        hidden_clips: BTreeSet::new(),
    }
}

/// Halo 4's `unit_show_weapon_from_inventory` (X360 0x84AB9B40): the unit's
/// `weapon specific markers` entry for the weapon's class and name gives the
/// hand's marker before `right_hand_node` / `left_hand_node` do. No kit unit
/// fills the block.
fn h4_weapon_specific_marker(reader: &TagReader, unit: &TagFile, weapon: &TagFile, left_hand: bool) -> Option<String> {
    if reader.source.game() != Some(GameId::Halo4) {
        return None;
    }
    let holder = holder_of(unit.root(), "weapon specific markers")?;
    let (class, name) = (string_id_in(weapon, "weapon class"), string_id_in(weapon, "weapon name"));
    let entry = elements(holder, "weapon specific markers").into_iter().find(|entry| {
        entry.read_string_id("weapon class").unwrap_or_default() == class
            && entry.read_string_id("weapon name").unwrap_or_default() == name
    })?;
    entry.read_string_id(if left_hand { "left hand marker" } else { "right hand marker" }).filter(|marker| !marker.is_empty())
}

/// `sub_88D990`, attaching a readied weapon: the unit's hand, to the unit's
/// preferred gun marker if the weapon has one, or else its handle node.
pub(in crate::app::model_preview) fn attach_weapon(
    reader: &TagReader,
    unit: &TagFile,
    path: &str,
    left_hand: bool,
    owner: u8,
    ik: Option<&UnitWeapons>,
    preview: &mut RenderModelPreview,
) -> Option<AttachedObject> {
    let weapon = reader.load(path, "weapon", b"weap").ok()?;
    let model = holder_of(weapon.root(), "model")?.read_tag_ref_path("model")?;
    let model = reader.load(&model, "model", b"hlmt").ok()?;
    let render = model.root().read_tag_ref_path("render model")?;
    let render = reader.load(&render, "render_model", b"mode").ok()?;
    let attached = build_render_preview(&render).ok()?;
    let hand = h4_weapon_specific_marker(reader, unit, &weapon, left_hand)
        .unwrap_or_else(|| string_id_in(unit, if left_hand { "left_hand_node" } else { "right_hand_node" }));
    let preferred = string_id_in(unit, "preferred_gun_node");
    let has_preferred = attached.markers.iter().any(|marker| marker.name == preferred);
    let grip = if has_preferred && !preferred.is_empty() {
        preferred
    } else {
        string_id_in(&weapon, "handle node")
    };
    let parent = MarkerFrame::find(preview, &hand);
    let child = MarkerFrame::find(&attached, &grip);
    // Reach's IK chains, and Halo 4's IK sets, reach for the held weapon's
    // markers where they ride.
    if matches!(reader.source.game(), Some(GameId::HaloReach | GameId::Halo4)) {
        let place = |frame: &MarkerFrame| {
            (frame.axes.map(|axis| attach_vector(&parent, &child, axis)), attach_point(&parent, &child, frame.position))
        };
        let holder = super::animation::reach_ik::ReachIkHolder::Weapon(owner);
        let node = Some(parent.node.max(0) as usize);
        preview.gen3.ik_markers.extend(super::animation::reach_ik::reach_ik_markers(&attached, holder, place, node));
    }
    if let Some(weapons) = ik {
        let class = string_id_in(&weapon, "weapon class");
        for mode in weapons.modes() {
            let mut points: Vec<CeWeaponIk> = Vec::new();
            for entry in weapons.ik_of(mode, &class) {
                let Some(mut point) =
                    weapon_ik(preview, &attached, mode, parent, child, &entry.marker, &entry.attach_to)
                else {
                    continue;
                };
                // `unit_apply_ik_points` solves a node once.
                if points.iter().all(|other| other.hand() != point.hand()) {
                    point.carried = descendants(preview, point.hand());
                    points.push(point);
                }
            }
            preview.weapon_ik.extend(points);
        }
    }
    let variant = object_variant(&weapon, &model, "").map(|variant| variant.name).unwrap_or_default();
    fold_attached(reader, &weapon, &model, &variant, attached, parent, child, owner, tag_path::name(path), preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bad node data gives no descendants rather than a hang or a panic.
    #[test]
    fn descendants_survive_a_parent_cycle_and_a_parent_past_the_end() {
        let node = |parent: i16| RenderModelPreviewNode { parent, ..Default::default() };
        // 0 is the root; 1 and 2 are each other's parent; 3's is past the end.
        let preview =
            RenderModelPreview { nodes: vec![node(-1), node(2), node(1), node(9), node(0)], ..Default::default() };
        assert_eq!(descendants(&preview, 0), vec![4]);
    }

    fn entry(mode: &str, class: &str, kind: &str, has_idle: bool) -> GraphEntry {
        GraphEntry { mode: mode.to_owned(), class: class.to_owned(), kind: kind.to_owned(), has_idle, slots: Vec::new() }
    }

    /// `entry`, with slots by label and the clip each names.
    fn slotted(mode: &str, class: &str, kind: &str, slots: &[(&str, &str)]) -> GraphEntry {
        let slots = slots.iter().map(|(label, clip)| ((*label).to_owned(), Some((*clip).to_owned()))).collect();
        GraphEntry { slots, ..entry(mode, class, kind, false) }
    }

    fn weapon(name: &str, class: &str, kind: &str, dual: bool, only_dual: bool) -> Equippable {
        Equippable {
            option: WeaponOption { path: name.to_owned(), name: name.to_owned(), class: class.to_owned() },
            kind: kind.to_owned(),
            dual,
            only_dual,
        }
    }

    fn unit() -> UnitWeapons {
        UnitWeapons {
            equippable: vec![
                weapon("magnum", "pistol", "hp", true, false),
                weapon("rocket", "missile", "rl", false, false),
                weapon("shield", "pistol", "js", true, true),
            ],
            graph: vec![
                entry("combat", "pistol", "any", true),
                slotted("combat", "pistol", "hp", &[("grip", "combat:pistol:hp:grip")]),
                entry("combat", "pistol", "ne", false),
                entry("combat", "rifle", "any", true),
                GraphEntry {
                    has_idle: true,
                    ..slotted("combat", "dual", "any", &[
                        ("arm_offsets", "combat:dual:arm_offsets"),
                        ("grip", "combat:dual:grip"),
                    ])
                },
                slotted("combat", "dual", "hp", &[("arm_offsets", "combat:dual:hp:arm_offsets")]),
                entry("combat", "unarmed", "any", true),
                entry("ghost_d", "any", "any", true),
            ],
            ik: vec![
                ik("combat", "any", "left_hand", "any"),
                ik("any", "pistol", "left_hand", "other"),
                ik("combat", "pistol", "left_hand", "left_hand_mc"),
                ik("combat", "rifle", "right_hand", "right_hand"),
            ],
            ik_off: BTreeSet::from(["combat:pistol:gut".to_owned()]),
            overlays: ["reload_1", "fire_1", "aim_still_up"].map(str::to_owned).into(),
            hidden_clips: BTreeSet::new(),
        }
    }

    fn ik(mode: &str, class: &str, marker: &str, attach_to: &str) -> GraphIk {
        GraphIk {
            mode: mode.to_owned(),
            class: class.to_owned(),
            marker: marker.to_owned(),
            attach_to: attach_to.to_owned(),
        }
    }

    #[test]
    fn weapon_ik_walks_the_exact_entries_before_any() {
        let unit = unit();
        let walked: Vec<_> =
            unit.ik_of("combat", "pistol").iter().map(|ik| ik.attach_to.as_str()).collect();
        assert_eq!(walked, ["left_hand_mc", "any", "other"]);
    }

    #[test]
    fn weapon_ik_stays_on_for_what_the_game_leaves_it_on_for() {
        let unit = unit();
        for clip in ["combat:pistol:idle", "combat:pistol:hp:fire_1", "combat:pistol:aim_still_up"] {
            assert!(unit.ik_runs_through(clip, "combat"), "{clip}");
        }
        for clip in [
            "combat:pistol:reload_1",
            "combat:pistol:gut",
            "combat:exit",
            "combat:enter_a",
            "combat:flying_front",
        ] {
            assert!(!unit.ik_runs_through(clip, "combat"), "{clip}");
        }
        assert!(!unit.ik_runs_through("crouch:pistol:idle", "combat"), "another mode's points");
    }

    #[test]
    fn one_weapon_grips_with_the_whole_body_only_while_ik_runs() {
        let unit = unit();
        let clips = ["combat:pistol:hp:grip", "combat:pistol:reload_1"];
        let layers = |clip| unit.pose_layers(&clips, clip, Some("magnum"), None);
        let grip = |arm| vec![PoseLayer { clip: clips[0].to_owned(), arm }];
        assert_eq!(layers("combat:pistol:idle"), grip(None));
        assert_eq!(layers("combat:pistol:reload_1"), grip(Some(Arm::NotLeft)));
        assert!(unit.pose_layers(&clips, "combat:pistol:idle", None, None).is_empty());
    }

    /// The search tries `any` for the type, then the class, before the mode:
    /// the `combat` mode's `any` class wins over the `any` mode's own pistol.
    /// A slot whose clip lies in another graph still ends it.
    #[test]
    fn a_clip_is_found_along_the_games_search_path() {
        let unit = UnitWeapons {
            graph: vec![
                slotted("any", "pistol", "hp", &[("grip", "any:pistol:hp:grip")]),
                slotted("combat", "any", "any", &[("grip", "combat:grip")]),
                slotted("crouch", "pistol", "any", &[("grip", "crouch:pistol:grip")]),
            ],
            ..Default::default()
        };
        let clips = ["any:pistol:hp:grip", "combat:grip", "crouch:pistol:grip"];
        let find = |mode| unit.find_clip(&clips, mode, "pistol", "hp", "grip");
        assert_eq!(find("combat").as_deref(), Some("combat:grip"));
        assert_eq!(find("crouch").as_deref(), Some("crouch:pistol:grip"));
        assert_eq!(find("flying").as_deref(), Some("any:pistol:hp:grip"), "the any mode last");
        let mut elsewhere = unit.clone();
        elsewhere.graph[1].slots[0].1 = None;
        let find = |mode| elsewhere.find_clip(&clips, mode, "pistol", "hp", "grip");
        assert_eq!(find("combat"), None, "found in another graph, so not the any mode's");
    }

    #[test]
    fn two_weapons_each_lay_their_own_side_with_the_nearest_clip() {
        let unit = unit();
        let clips = ["combat:dual:arm_offsets", "combat:dual:hp:arm_offsets", "combat:dual:grip"];
        let layers = unit.pose_layers(&clips, "combat:dual:idle", Some("magnum"), Some("shield"));
        let row = |clip: &str, arm| PoseLayer { clip: clip.to_owned(), arm: Some(arm) };
        assert_eq!(
            layers,
            [
                row("combat:dual:hp:arm_offsets", Arm::NotLeft),
                row("combat:dual:arm_offsets", Arm::Left),
                row("combat:dual:grip", Arm::NotLeft),
                row("combat:dual:grip", Arm::Left),
            ]
        );
    }

    #[test]
    fn a_weapon_needs_an_idle_for_its_class() {
        let graph = unit().graph;
        assert!(accepts(&graph, "pistol", "hp"));
        assert!(accepts(&graph, "rifle", "smg"), "type falls back on any");
        assert!(!accepts(&graph, "sword", "eb"));
        assert!(!accepts(&[entry("combat", "pistol", "hp", false)], "pistol", "hp"));
    }

    #[test]
    fn the_left_hand_opens_only_for_a_primary_that_pairs() {
        let unit = unit();
        assert!(unit.secondaries(None).is_empty());
        assert!(unit.secondaries(Some("rocket")).is_empty());
        let names: Vec<_> = unit.secondaries(Some("magnum")).into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["magnum", "shield"]);
    }

    #[test]
    fn what_only_pairs_is_no_primary() {
        let names: Vec<_> = unit().primaries().into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["magnum", "rocket"]);
    }

    #[test]
    fn a_unit_with_no_dual_clips_has_no_left_hand() {
        let mut unit = unit();
        unit.graph.retain(|entry| entry.class != DUAL);
        assert!(unit.secondaries(Some("magnum")).is_empty());
    }

    #[test]
    fn a_second_weapon_makes_the_class_dual() {
        let unit = unit();
        assert_eq!(unit.held(None, None), ("unarmed".to_owned(), Vec::new()));
        assert_eq!(unit.held(Some("magnum"), None), ("pistol".to_owned(), vec!["hp".to_owned()]));
        let (class, kinds) = unit.held(Some("magnum"), Some("shield"));
        assert_eq!((class.as_str(), kinds.len()), ("dual", 2));
    }

    #[test]
    fn clips_of_what_is_not_held_go() {
        let unit = unit();
        let clips = [
            "combat:pistol:idle",
            "combat:pistol:hp:fire_1",
            "combat:pistol:ne:fire_1",
            "combat:rifle:idle",
            "combat:look",
            "ghost_d:any:idle",
            "stand",
        ];
        let hidden = unit.hidden(&clips, "pistol", &["hp".to_owned()]);
        assert_eq!(hidden, BTreeSet::from([2, 3]));
        assert!(unit.hidden(&clips, "unarmed", &[]).is_empty(), "nothing held, nothing hidden");
    }

    fn graph_of(entries: &[(&str, &str, &str, bool)]) -> UnitWeapons {
        let graph = entries.iter().map(|(mode, class, kind, has_idle)| GraphEntry {
            mode: (*mode).to_owned(),
            class: (*class).to_owned(),
            kind: (*kind).to_owned(),
            has_idle: *has_idle,
            slots: Vec::new(),
        });
        UnitWeapons { graph: graph.collect(), ..Default::default() }
    }

    #[test]
    fn a_clip_the_held_types_slot_names_stays_whatever_it_is_called() {
        let mut weapons = graph_of(&[
            ("combat", "rifle", "ar", true),
            ("combat", "rifle", "br", true),
            ("combat", "rifle", "sr", true),
        ]);
        weapons.graph[0].slots = vec![("fire_1".to_owned(), Some("combat:rifle:br:fire_1".to_owned()))];
        let clips = ["combat:rifle:ar:grip", "combat:rifle:br:fire_1", "combat:rifle:br:grip", "combat:rifle:sr:fire_1"];
        let hidden = weapons.hidden(&clips, "rifle", &["ar".to_owned()]);
        assert_eq!(hidden.iter().copied().collect::<Vec<_>>(), [2, 3], "only the BR's own stay hidden");
        let hidden = weapons.hidden(&clips, "rifle", &["sr".to_owned()]);
        assert!(hidden.contains(&1), "the AR's slot means nothing to a held sniper rifle");
    }

    #[test]
    fn what_is_under_any_stays_for_every_weapon() {
        let unit = graph_of(&[
            ("combat", "rifle", "any", true),
            ("combat", "rifle", "br", false),
            ("combat", "rifle", "ar", false),
            ("mongoose_p", "any", "any", true),
            ("mongoose_p", "rifle", "any", true),
            ("mongoose_p", "flag", "any", true),
        ]);
        let clips = [
            "combat:rifle:any:melee",
            "combat:rifle:br:melee_strike_1",
            "combat:rifle:ar:grip",
            "mongoose_p:any:idle",
            "mongoose_p:flag:melee",
            "mongoose_p:rifle:idle",
        ];
        let hidden = unit.hidden(&clips, "rifle", &["ar".to_owned()]);
        assert_eq!(hidden, BTreeSet::from([1, 4]));
    }

    #[test]
    fn with_nothing_held_every_clip_stays() {
        let unit = graph_of(&[
            ("combat", "unarmed", "any", true),
            ("combat", "rifle", "any", true),
        ]);
        let clips = ["combat:unarmed:airborne_arc", "combat:unarmed:idle", "combat:rifle:melee_airborne"];
        assert!(unit.hidden(&clips, UNARMED, &[]).is_empty());
        assert_eq!(unit.hidden(&clips, "rifle", &[]), BTreeSet::from([0, 1]));
    }

    /// With `BLAM_TEST_HREK`: Reach keeps a type's actions and overlays in its
    /// `any` set, which the Spartan's rifles and its grip are read from.
    #[test]
    fn a_reach_graph_reads_its_slots_from_the_any_set() {
        let Ok(root) = std::env::var("BLAM_TEST_HREK") else {
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloReach),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let spartan = "objects\\characters\\spartans\\spartans";
        let graph = load_referenced_tag_from_source(&source, spartan, "model_animation_graph", b"jmad").unwrap();
        let entries = graph_entries(&graph);
        let ar = entries.iter().find(|entry| (entry.mode.as_str(), entry.class.as_str(), entry.kind.as_str()) == ("combat", "rifle", "ar"));
        let ar = ar.expect("combat:rifle:ar");
        assert!(ar.slots.iter().any(|(label, clip)| label == GRIP && clip.as_deref() == Some("combat:rifle:ar:grip")), "{:?}", ar.slots);
        let rifle = entries.iter().find(|entry| (entry.mode.as_str(), entry.class.as_str(), entry.kind.as_str()) == ("combat", "rifle", ANY));
        assert!(rifle.is_some_and(|entry| entry.has_idle), "the rifle class idles under any");
        let unit = load_referenced_tag_from_source(&source, spartan, "biped", b"bipd").unwrap();
        let weapons = unit_weapons(&crate::app::model_preview::load::TagReader::new(&source), &unit, None);
        let paths: Vec<_> = weapons.equippable.iter().map(|weapon| weapon.option.name.as_str()).collect();
        assert!(paths.contains(&"assault_rifle") && paths.contains(&"sniper_rifle"), "{paths:?}");
        assert_eq!(clips_without_ik(&graph).len(), 2, "two clips turn weapon IK off by their user flags");
    }
}
