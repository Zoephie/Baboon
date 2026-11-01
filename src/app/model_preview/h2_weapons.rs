//! What a Halo 2 unit can hold, and how it holds it. A weapon names its own
//! animation class and type, and `unit_can_use_weapon` lets it in when the
//! unit's graph has an idle for them. Dual wielding is a class of its own,
//! `dual`, once a second weapon is in the left hand.

use std::collections::BTreeSet;

use blam_tags::{TagFile, TagStruct};

use super::attachment::{MarkerFrame, WeaponOption, merge_attached};
use super::ce_attachments::{attached_marker_frame, elements, loose_weapon_paths, weapon_ik};
use super::loading::build_render_preview;
use super::ce_attachments::CeWeaponIk;
use super::*;
use crate::core::source::TagSource;

/// The mode a unit nobody controls is in.
const RESTING_MODE: &str = "combat";
/// Every level of the graph falls back on this.
const ANY: &str = "any";
const UNARMED: &str = "unarmed";
/// `unit_animation_set_weapon`'s class with something in the left hand.
const DUAL: &str = "dual";
const IDLE: &str = "idle";
/// `playback flags` bit a clip turns weapon IK off with.
const DISABLE_WEAPON_IK: i128 = 1 << 3;
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
/// `model flags` bit of a skeleton node on the left arm.
const LEFT_ARM_MEMBER: i128 = 1 << 5;
const CAN_BE_DUAL: &str = "can be dual wielded";
const ONLY_DUAL: &str = "can only be dual wielded";

/// A weapon a unit's animations can hold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H2Equippable {
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
}

/// Which nodes an overlay channel reaches: the graph's `left arm member`
/// nodes, or all the others.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum H2Arm {
    Left,
    NotLeft,
}

/// What `unit_preprocess_node_orientations` lays over a clip for what is held.
/// Aiming and looking are not among it: dead ahead at full weight it skips them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H2PoseLayer {
    pub(crate) clip: String,
    pub(crate) arm: Option<H2Arm>,
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
pub(crate) struct H2UnitWeapons {
    pub(crate) equippable: Vec<H2Equippable>,
    graph: Vec<GraphEntry>,
    ik: Vec<GraphIk>,
    /// Clips that set `disable weapon ik`.
    ik_off: BTreeSet<String>,
    /// Every overlay label in the graph.
    overlays: BTreeSet<String>,
    /// Filled once the clips are listed.
    pub(crate) hidden_clips: BTreeSet<usize>,
}

impl H2UnitWeapons {
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

    fn find(&self, path: Option<&str>) -> Option<&H2Equippable> {
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

    /// `sub_616470`: a clip for this class and type, or the nearest `any`. A
    /// name leaves `any` out.
    fn find_clip(clips: &[&str], mode: &str, class: &str, kind: &str, last: &str) -> Option<String> {
        let tried = [
            format!("{mode}:{class}:{kind}:{last}"),
            format!("{mode}:{class}:{last}"),
            format!("{mode}:{kind}:{last}"),
            format!("{mode}:{last}"),
        ];
        tried.into_iter().find(|name| clips.contains(&name.as_str()))
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
    ) -> Vec<H2PoseLayer> {
        let name = clip.split('%').next().unwrap_or(clip);
        let mode = name.split(':').next().unwrap_or(name);
        let layer = |class: &str, kind: &str, last: &str, arm| {
            let clip = Self::find_clip(clips, mode, class, kind, last)?;
            Some(H2PoseLayer { clip, arm })
        };
        match (self.find(primary), self.find(secondary)) {
            (Some(first), Some(second)) => {
                let right = (first.kind.as_str(), H2Arm::NotLeft);
                let left = (second.kind.as_str(), H2Arm::Left);
                [(ARM_OFFSETS, right), (ARM_OFFSETS, left), (GRIP, right), (GRIP, left)]
                    .into_iter()
                    .filter_map(|(last, (kind, arm))| layer(DUAL, kind, last, Some(arm)))
                    .collect()
            }
            (Some(held), None) => {
                let arm = (!self.ik_runs_through(clip, mode)).then_some(H2Arm::NotLeft);
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
    /// A mode with no clips for this class keeps all of its own: the Chief's
    /// `combat:unarmed` has none.
    pub(crate) fn hidden(&self, clips: &[&str], class: &str, kinds: &[String]) -> BTreeSet<usize> {
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
        let mut hidden = BTreeSet::new();
        for (index, clip) in clips.iter().enumerate() {
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
            let held = of_kind.is_none_or(|kind| kinds.iter().any(|held| held == kind));
            if *clip_class != class || !held {
                hidden.insert(index);
            }
        }
        hidden
    }
}

/// The struct holding a field, wherever in the tag it sits.
fn holder_of<'a>(from: TagStruct<'a>, field: &str) -> Option<TagStruct<'a>> {
    if from.field(field).is_some() {
        return Some(from);
    }
    from.fields_all().filter_map(|field| field.as_struct()).find_map(|inner| holder_of(inner, field))
}

fn graph_entries(graph: &TagFile) -> Vec<GraphEntry> {
    let mut entries = Vec::new();
    let Some(holder) = holder_of(graph.root(), "modes") else {
        return entries;
    };
    let label = |entry: TagStruct<'_>| entry.read_string_id("label").unwrap_or_default();
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                let actions = elements(kind, "actions");
                entries.push(GraphEntry {
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
                let overlays = elements(kind, "overlays");
                labels.extend(overlays.iter().filter_map(|overlay| overlay.read_string_id("label")));
            }
        }
    }
    labels
}

/// Which of a graph's nodes the `left arm member` channel reaches.
pub(super) fn left_arm_nodes(graph: &TagFile) -> Vec<bool> {
    let Some(holder) = holder_of(graph.root(), "skeleton nodes") else {
        return Vec::new();
    };
    let member = |node: &TagStruct<'_>| {
        node.read_int_any("model flags").is_some_and(|flags| flags & LEFT_ARM_MEMBER != 0)
    };
    elements(holder, "skeleton nodes").iter().map(member).collect()
}

fn clips_without_ik(graph: &TagFile) -> BTreeSet<String> {
    let Some(holder) = holder_of(graph.root(), "animations") else {
        return BTreeSet::new();
    };
    let flagged = elements(holder, "animations").into_iter().filter(|clip| {
        clip.read_int_any("playback flags").is_some_and(|flags| flags & DISABLE_WEAPON_IK != 0)
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

fn weapon_flags(weapon: &TagFile) -> Vec<String> {
    let names = holder_of(weapon.root(), "magazines").and_then(|holder| holder.read_flag_names("flags"));
    names.unwrap_or_default().into_iter().map(|(_, name)| name).collect()
}

fn string_id_in(tag: &TagFile, name: &str) -> String {
    holder_of(tag.root(), name).and_then(|holder| holder.read_string_id(name)).unwrap_or_default()
}

/// Every node below `node`.
fn descendants(preview: &RenderModelPreview, node: usize) -> Vec<usize> {
    let below = |mut index: usize| {
        while let Some(parent) = usize::try_from(preview.nodes[index].parent).ok() {
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
fn unit_graph(source: &TagSource, unit: &TagFile) -> Option<TagFile> {
    let model = holder_of(unit.root(), "model")?.read_tag_ref_path("model")?;
    let model = load_referenced_tag_from_source(source, &model, "model", b"hlmt").ok()?;
    let graph = model.root().read_tag_ref_path("animation").filter(|path| !path.is_empty())?;
    load_referenced_tag_from_source(source, &graph, "model_animation_graph", b"jmad").ok()
}

/// What a unit can hold, from a loose folder's weapons.
pub(super) fn unit_weapons(
    source: &TagSource,
    unit: &TagFile,
    known: Option<&[H2Equippable]>,
) -> H2UnitWeapons {
    let Some(tag) = unit_graph(source, unit) else {
        return H2UnitWeapons::default();
    };
    let graph = graph_entries(&tag);
    let equippable = known.map_or_else(
        || {
            let TagSource::LooseFolder { root, .. } = source else {
                return Vec::new();
            };
            let mut found: Vec<_> = loose_weapon_paths(root)
                .into_iter()
                .filter_map(|path| {
                    let weapon =
                        load_referenced_tag_from_source(source, &path, "weapon", b"weap").ok()?;
                    let class = string_id_in(&weapon, "weapon class");
                    let kind = string_id_in(&weapon, "weapon name");
                    accepts(&graph, &class, &kind).then(|| {
                        let flags = weapon_flags(&weapon);
                        let has = |flag: &str| flags.iter().any(|name| name.starts_with(flag));
                        H2Equippable {
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
    H2UnitWeapons {
        equippable,
        graph,
        ik: graph_ik(&tag),
        ik_off: clips_without_ik(&tag),
        overlays: graph_overlays(&tag),
        hidden_clips: BTreeSet::new(),
    }
}

/// `sub_88D990`, attaching a readied weapon: the unit's hand, to the unit's
/// preferred gun marker if the weapon has one, or else its handle node.
pub(super) fn attach_h2_weapon(
    source: &TagSource,
    unit: &TagFile,
    path: &str,
    left_hand: bool,
    owner: u8,
    ik: Option<&H2UnitWeapons>,
    preview: &mut RenderModelPreview,
) -> Option<H2AttachedObject> {
    let weapon = load_referenced_tag_from_source(source, path, "weapon", b"weap").ok()?;
    let model = holder_of(weapon.root(), "model")?.read_tag_ref_path("model")?;
    let model = load_referenced_tag_from_source(source, &model, "model", b"hlmt").ok()?;
    let render = model.root().read_tag_ref_path("render model")?;
    let render = load_referenced_tag_from_source(source, &render, "render_model", b"mode").ok()?;
    let attached = build_render_preview(&render).ok()?;
    let hand = string_id_in(unit, if left_hand { "left_hand_node" } else { "right_hand_node" });
    let preferred = string_id_in(unit, "preferred_gun_node");
    let has_preferred = attached.markers.iter().any(|marker| marker.name == preferred);
    let grip = if has_preferred && !preferred.is_empty() {
        preferred
    } else {
        string_id_in(&weapon, "handle node")
    };
    let parent = MarkerFrame::find(preview, &hand);
    let child = MarkerFrame::find(&attached, &grip);
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
    let (flares, lights, gels) =
        super::h2_lights::read_h2_flares(source, &weapon, owner, &attached.markers, |marker| {
            (attached_marker_frame(marker, &parent, &child), parent.node.max(0) as usize)
        });
    preview.h2_flares.extend(flares);
    preview.h2_point_lights.extend(lights);
    preview.h2_gel_lights.extend(gels);
    // No weapon in the kit has PRT of its own.
    if let Some(prt) = preview.h2_prt.as_mut() {
        prt.pad(attached.vertices.len());
    }
    merge_attached(preview, attached, parent, child, owner, tag_path::name(path));
    Some(H2AttachedObject { owner, functions: super::h2_functions::read_h2_object_functions(&weapon)? })
}

/// A weapon in a Halo 2 unit's hand.
#[derive(Debug, Clone)]
pub(crate) struct H2AttachedObject {
    /// What its materials carry as `object`.
    pub(crate) owner: u8,
    pub(crate) functions: super::h2_functions::H2ObjectFunctions,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mode: &str, class: &str, kind: &str, has_idle: bool) -> GraphEntry {
        GraphEntry { mode: mode.to_owned(), class: class.to_owned(), kind: kind.to_owned(), has_idle }
    }

    fn weapon(name: &str, class: &str, kind: &str, dual: bool, only_dual: bool) -> H2Equippable {
        H2Equippable {
            option: WeaponOption { path: name.to_owned(), name: name.to_owned(), class: class.to_owned() },
            kind: kind.to_owned(),
            dual,
            only_dual,
        }
    }

    fn unit() -> H2UnitWeapons {
        H2UnitWeapons {
            equippable: vec![
                weapon("magnum", "pistol", "hp", true, false),
                weapon("rocket", "missile", "rl", false, false),
                weapon("shield", "pistol", "js", true, true),
            ],
            graph: vec![
                entry("combat", "pistol", "any", true),
                entry("combat", "pistol", "hp", false),
                entry("combat", "pistol", "ne", false),
                entry("combat", "rifle", "any", true),
                entry("combat", "dual", "any", true),
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
        let grip = |arm| vec![H2PoseLayer { clip: clips[0].to_owned(), arm }];
        assert_eq!(layers("combat:pistol:idle"), grip(None));
        assert_eq!(layers("combat:pistol:reload_1"), grip(Some(H2Arm::NotLeft)));
        assert!(unit.pose_layers(&clips, "combat:pistol:idle", None, None).is_empty());
    }

    #[test]
    fn two_weapons_each_lay_their_own_side_with_the_nearest_clip() {
        let unit = unit();
        let clips = ["combat:dual:arm_offsets", "combat:dual:hp:arm_offsets", "combat:dual:grip"];
        let layers = unit.pose_layers(&clips, "combat:dual:idle", Some("magnum"), Some("shield"));
        let row = |clip: &str, arm| H2PoseLayer { clip: clip.to_owned(), arm: Some(arm) };
        assert_eq!(
            layers,
            [
                row("combat:dual:hp:arm_offsets", H2Arm::NotLeft),
                row("combat:dual:arm_offsets", H2Arm::Left),
                row("combat:dual:grip", H2Arm::NotLeft),
                row("combat:dual:grip", H2Arm::Left),
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
        assert!(unit.hidden(&clips, "unarmed", &[]).is_empty(), "no unarmed clips, nothing to swap");
    }
}
