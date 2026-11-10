//! A unit's weapons, as the panel offers them.

use super::super::super::tag_path;
use super::super::TagReader;
use super::graph::{
    AIM_OVERLAY, ANY, ARM_OFFSETS, Arm, CAN_BE_DUAL, DUAL, Equippable, GRIP, GraphEntry, GraphIk,
    IK_OVERLAYS, NO_IK_ACTIONS, ONLY_DUAL, PoseLayer, UNARMED, accepts, clips_without_ik,
    graph_entries, graph_ik, graph_overlays, string_id_in, unit_graph,
};
use super::{TagSource, has_flag, holder_of, loose_weapon_paths};
use blam_tags::TagFile;
use std::collections::BTreeSet;

/// What a Halo 2 unit can hold, and which clips what it holds leaves out.
#[derive(Debug, Clone, Default)]
pub(crate) struct UnitWeapons {
    /// The kit's weapons the unit can hold: none until listed, which the
    /// weapon picker asks for when first opened ([`list_unit_weapons`]), as
    /// listing reads every weapon in the kit.
    pub(crate) equippable: Option<Vec<Equippable>>,
    pub(super) graph: Vec<GraphEntry>,
    pub(super) ik: Vec<GraphIk>,
    /// Clips that set `disable weapon ik`.
    pub(super) ik_off: BTreeSet<String>,
    /// Every overlay label in the graph.
    pub(super) overlays: BTreeSet<String>,
    /// Filled once the clips are listed.
    pub(crate) hidden_clips: BTreeSet<usize>,
}

impl UnitWeapons {
    /// Whether the unit's animations hold weapons at all, so it has a picker.
    pub(crate) fn can_hold(&self) -> bool {
        !self.graph.is_empty()
    }

    /// The listed weapons, none before they are.
    fn listed(&self) -> &[Equippable] {
        self.equippable.as_deref().unwrap_or_default()
    }

    /// What the right hand can hold, or `None` until the weapons are listed.
    pub(crate) fn primaries(&self) -> Option<Vec<WeaponOption>> {
        let listed = self.equippable.as_deref()?;
        let usable = listed.iter().filter(|weapon| !weapon.only_dual);
        Some(usable.map(|weapon| weapon.option.clone()).collect())
    }

    /// What can go in the left hand, as `unit_get_weapon_pickup_options` has
    /// it: only with a primary that can pair, and only what can pair with it.
    pub(crate) fn secondaries(&self, primary: Option<&str>) -> Vec<WeaponOption> {
        let held = self.find(primary);
        if !held.is_some_and(|held| held.dual) || !self.has_dual() {
            return Vec::new();
        }
        let usable = self.listed().iter().filter(|weapon| weapon.dual);
        usable.map(|weapon| weapon.option.clone()).collect()
    }

    /// Whether the graph has anything for two weapons.
    fn has_dual(&self) -> bool {
        self.graph
            .iter()
            .any(|entry| entry.class == DUAL && entry.has_idle)
    }

    fn find(&self, path: Option<&str>) -> Option<&Equippable> {
        self.listed()
            .iter()
            .find(|weapon| Some(weapon.option.path.as_str()) == path)
    }

    /// `animation_weapon_ik_iterator_next`: each mode's entries for `class`,
    /// then for `any`, then the same under the `any` mode.
    pub(super) fn ik_of(&self, mode: &str, class: &str) -> Vec<&GraphIk> {
        let mut walked = Vec::new();
        for (mode, class) in [(mode, class), (mode, ANY), (ANY, class), (ANY, ANY)] {
            walked.extend(
                self.ik
                    .iter()
                    .filter(|ik| ik.mode == mode && ik.class == class),
            );
        }
        walked
    }

    pub(super) fn modes(&self) -> BTreeSet<&str> {
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
    pub(super) fn find_clip(
        &self,
        clips: &[&str],
        mode: &str,
        class: &str,
        kind: &str,
        last: &str,
    ) -> Option<String> {
        for step in 0..8u8 {
            let anything = |bit: u8, value: &str| step & bit != 0 && value == ANY;
            if anything(4, mode) || anything(2, class) || anything(1, kind) {
                continue;
            }
            let mode = if step & 4 != 0 { ANY } else { mode };
            let class = if step & 2 != 0 { ANY } else { class };
            let kind = if step & 1 != 0 { ANY } else { kind };
            let entry = self
                .graph
                .iter()
                .find(|entry| entry.mode == mode && entry.class == class && entry.kind == kind);
            if let Some((_, clip)) =
                entry.and_then(|entry| entry.slots.iter().find(|(label, _)| label == last))
            {
                return clip
                    .as_deref()
                    .filter(|clip| clips.contains(clip))
                    .map(str::to_owned);
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
                [
                    (ARM_OFFSETS, right),
                    (ARM_OFFSETS, left),
                    (GRIP, right),
                    (GRIP, left),
                ]
                .into_iter()
                .filter_map(|(last, (kind, arm))| layer(DUAL, kind, last, Some(arm)))
                .collect()
            }
            (Some(held), None) => {
                let arm = (!self.ik_runs_through(clip, mode)).then_some(Arm::NotLeft);
                layer(&held.option.class, &held.kind, GRIP, arm)
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The class and types `unit_animation_set_weapon` puts the unit in.
    pub(crate) fn held(
        &self,
        primary: Option<&str>,
        secondary: Option<&str>,
    ) -> (String, Vec<String>) {
        let (first, second) = (self.find(primary), self.find(secondary));
        match (first, second) {
            (Some(first), Some(second)) => (
                DUAL.to_owned(),
                vec![first.kind.clone(), second.kind.clone()],
            ),
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
            self.graph
                .iter()
                .any(|entry| entry.mode == mode && entry.class == class)
        };
        let is_kind = |mode: &str, class: &str, kind: &str| {
            self.graph
                .iter()
                .any(|entry| entry.mode == mode && entry.class == class && entry.kind == kind)
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
            let held =
                of_kind.is_none_or(|kind| kind == ANY || kinds.iter().any(|held| held == kind));
            if (*clip_class != class && *clip_class != ANY) || !held {
                hidden.insert(index);
            }
        }
        hidden
    }
}

/// What a unit's animations let it hold, with the weapons `known` lists:
/// `known` is an earlier load's list, or none, which leaves the weapons to
/// [`list_unit_weapons`].
pub(in crate::app::model_preview) fn unit_weapons(
    reader: &TagReader,
    unit: &TagFile,
    known: Option<&[Equippable]>,
) -> UnitWeapons {
    let Some(tag) = unit_graph(reader, unit) else {
        return UnitWeapons::default();
    };
    let graph = graph_entries(&tag);
    UnitWeapons {
        equippable: known.map(<[_]>::to_vec),
        graph,
        ik: graph_ik(&tag),
        ik_off: clips_without_ik(&tag),
        overlays: graph_overlays(&tag),
        hidden_clips: BTreeSet::new(),
    }
}

/// The loose folder's weapons `unit`'s animations let it hold, by class then
/// name: every `.weapon` in the kit is read, so this runs on a worker, when
/// the picker first opens.
pub(in crate::app::model_preview) fn list_unit_weapons(
    reader: &TagReader,
    unit: &TagFile,
) -> Vec<Equippable> {
    let TagSource::LooseFolder { root, .. } = &reader.source else {
        return Vec::new();
    };
    let Some(tag) = unit_graph(reader, unit) else {
        return Vec::new();
    };
    let graph = graph_entries(&tag);
    let mut found: Vec<_> = loose_weapon_paths(root)
        .into_iter()
        .filter_map(|path| {
            let weapon = reader.load(&path, "weapon", b"weap").ok()?;
            let class = string_id_in(&weapon, "weapon class");
            let kind = string_id_in(&weapon, "weapon name");
            accepts(&graph, &class, &kind).then(|| {
                // The weapon's own `flags` sit beside its `magazines`.
                let holder = holder_of(weapon.root(), "magazines");
                let has =
                    |flag: &str| holder.is_some_and(|holder| has_flag(holder, "flags", flag));
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
    found.sort_by(|a, b| (&a.option.class, &a.option.name).cmp(&(&b.option.class, &b.option.name)));
    found
}

/// One row of a weapon dropdown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WeaponOption {
    pub(crate) path: String,
    pub(crate) name: String,
    /// The class its row sits under.
    pub(crate) class: String,
}
