//! Halo 4's IK sets (`process_ik_set`, X360 0x84B19FD0, MCC 0x140F16D10): on a
//! data-driven graph, before foot locking and the IK chains, each point of the
//! clip's IK set moves the node a unit's source marker rides so that marker
//! lands on a destination marker, usually the held weapon's, at full weight.
//! This is how a Spartan's left hand holds a rifle. See
//! `docs/h4-preview-research/objects.md` §3.6.

use blam_tags::math::Matrix4;
use blam_tags::{TagFile, TagStruct};

use crate::app::model_preview::common::math::rigid_inverse;
use super::super::common::fields::elements;
use crate::app::model_preview::common::fields::holder_of;
use super::reach_ik::{ReachIkHolder, ReachIkMarker, two_bone_ik};
use super::super::RenderModelPreviewNode;

/// `disable ik sets`, of an animation's `user flags`
/// (`unit_animation_find_user_flag(unit, 10)`).
const DISABLE_IK_SETS: i128 = 1 << 10;

/// `animation_ik_target_enum_definition`.
const ATTACH_TO_PARENT: i128 = 0;
const ATTACH_TO_PRIMARY_WEAPON: i128 = 2;
const ATTACH_TO_SECONDARY_WEAPON: i128 = 3;

/// `animation_ik_set_item_flags` bit 0: the point is off.
const POINT_DISABLED: i128 = 1;

/// One point of an IK set: the `ik data` entry it names.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct H4IkPoint {
    /// On the unit; the node it rides is what moves.
    pub(crate) source: String,
    /// Whose `destination` marker it reaches for.
    pub(crate) holder: ReachIkHolder,
    pub(crate) destination: String,
    /// A point the elbow or knee bends toward, in the unit's space.
    pub(crate) pole: Option<[f32; 3]>,
}

/// The `ik set` index an entry of the graph's mode tree carries, if any.
fn ik_set(entry: TagStruct<'_>) -> Option<usize> {
    usize::try_from(entry.read_int_any("ik set")?).ok()
}

/// The IK set of animation `index` of `graph`
/// (`find_hierarchical_property_index_and_graph(property 1)`, X360
/// 0x82E9EBE0): the most specific entry of the mode tree holding the clip
/// that names one, of its overlay or action, set, type, class and mode.
/// Clips other graphs' trees name are not followed [inferred: no kit unit
/// draws an IK set from a parent graph].
fn clip_ik_set(graph: &TagFile, index: usize) -> Option<usize> {
    let holder = holder_of(graph.root(), "modes")?;
    let refers = |entry: TagStruct<'_>| {
        let Some(animation) = entry.descend("animation") else { return false };
        let graph = animation.read_int_any("graph index").unwrap_or(-1);
        (graph == 0 || graph == -1) && animation.read_int_any("animation").and_then(|at| usize::try_from(at).ok()) == Some(index)
    };
    for mode in elements(holder, "modes") {
        for class in elements(mode, "weapon class") {
            for kind in elements(class, "weapon type") {
                for set in elements(kind, "sets") {
                    let entries = elements(set, "actions").into_iter().chain(elements(set, "overlay animations"));
                    for entry in entries.filter(|entry| refers(*entry)) {
                        let found = [entry, set, kind, class, mode].into_iter().find_map(ik_set);
                        if found.is_some() {
                            return found;
                        }
                    }
                }
            }
        }
    }
    None
}

/// The points `process_ik_set` solves for animation `index` of `graph`: none
/// when the clip turns IK sets off, and none for a disabled point or one that
/// attaches to "any child", which the game logs as not yet implemented.
pub(crate) fn read_h4_ik_set(graph: &TagFile, index: usize) -> Vec<H4IkPoint> {
    let Some(definitions) = graph.root().descend("definitions") else {
        return Vec::new();
    };
    let clip = elements(definitions, "animations").get(index).copied();
    if clip.and_then(|clip| clip.read_int_any("user flags")).is_some_and(|flags| flags & DISABLE_IK_SETS != 0) {
        return Vec::new();
    }
    let Some(set) = clip_ik_set(graph, index).and_then(|set| elements(definitions, "ik sets").get(set).copied()) else {
        return Vec::new();
    };
    let data = elements(definitions, "ik data");
    let name = |entry: TagStruct<'_>, field: &str| entry.read_string_id(field).unwrap_or_default();
    elements(set, "ik points")
        .into_iter()
        .filter(|point| point.read_int_any("flags").unwrap_or(0) & POINT_DISABLED == 0)
        .filter_map(|point| {
            let entry = *data.get(usize::try_from(point.read_int_any("ik point")?).ok()?)?;
            let holder = match entry.read_int_any("attach to")? {
                ATTACH_TO_PARENT => ReachIkHolder::Parent,
                ATTACH_TO_PRIMARY_WEAPON => ReachIkHolder::Weapon(1),
                ATTACH_TO_SECONDARY_WEAPON => ReachIkHolder::Weapon(2),
                _ => return None,
            };
            let pole = match entry.field("pole marker").and_then(|field| field.value()) {
                Some(blam_tags::TagFieldData::RealPoint3d(point)) => Some([point.x, point.y, point.z]),
                _ => None,
            };
            Some(H4IkPoint {
                source: name(entry, "source marker"),
                holder,
                destination: name(entry, "destination marker"),
                pole: pole.filter(|pole| pole.iter().any(|lane| *lane != 0.0)),
            })
        })
        .collect()
}

/// Solve `points` on the posed `worlds` (`render_model_apply_two_bone_ik`,
/// X360 0x82AE5B88, at weight 1): the source marker's node is placed at the
/// destination marker's matrix times the inverse of where the source marker
/// sits on that node. A point whose markers are missing is skipped, as the
/// game skips one whose object or marker it cannot find.
pub(crate) fn apply_h4_ik_set(
    points: &[H4IkPoint],
    markers: &[ReachIkMarker],
    nodes: &[RenderModelPreviewNode],
    inverse_binds: &[Matrix4],
    worlds: &mut [Matrix4],
) {
    let find = |holder: ReachIkHolder, name: &str| {
        markers.iter().find(|marker| marker.holder == holder && marker.name.eq_ignore_ascii_case(name))
    };
    for point in points {
        let (Some(source), Some(destination)) = (find(ReachIkHolder::Unit, &point.source), find(point.holder, &point.destination))
        else {
            continue;
        };
        let (Some(holder), Some(inverse)) = (worlds.get(destination.node), inverse_binds.get(destination.node)) else {
            continue;
        };
        let target = *holder * *inverse * destination.bind;
        let Some(source_inverse) = inverse_binds.get(source.node) else {
            continue;
        };
        // Where the source marker sits on its node.
        let on_node = *source_inverse * source.bind;
        let desired = target * rigid_inverse(&on_node);
        two_bone_ik(nodes, worlds, source.node, desired, 1.0, point.pole);
    }
}

#[cfg(test)]
mod tests {
    use crate::app::model_preview::harness::kit::kit_root;
    use crate::app::model_preview::*;
    use crate::core::format::TagNameIndex;
    use crate::core::game::GameId;
    use crate::core::source::{TagEntry, TagEntryLocation, TagSource};
    use std::sync::Arc;

    /// The Chief biped, with the AR, posed at `clip`, from the kit at `root`.
    fn h4_chief_with_rifle(root: &std::path::Path, clip: &str) -> (ModelPreviewData, ModelPreviewState) {
        let source = TagSource::LooseFolder {
            root: root.to_path_buf(),
            game: Some(GameId::Halo4),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let TagSource::LooseFolder { definitions_root, .. } = &source else { unreachable!() };
        let names = TagNameIndex::load_game(definitions_root, GameId::Halo4).unwrap();
        let path = "objects\\characters\\storm_masterchief\\storm_masterchief";
        let tag = load_referenced_tag_from_source(&source, path, "biped", b"bipd").unwrap();
        let file = root.join("objects/characters/storm_masterchief/storm_masterchief.biped");
        let entry = TagEntry {
            key: format!("{path}.biped"),
            display_path: format!("{path}.biped"),
            group_tag: u32::from_be_bytes(*b"bipd"),
            group_name: Some("biped".to_owned()),
            location: TagEntryLocation::LooseFile(file),
        };
        let weapon = "objects\\weapons\\rifle\\storm_assault_rifle\\storm_assault_rifle".to_owned();
        let settings = PreviewLoadSettings { weapon: Some(weapon.clone()), ..Default::default() };
        let mut data = load_model_preview(&tag, &entry, &names, Some(&source), &settings).unwrap();
        let mut state = ModelPreviewState::default();
        reset_model_preview_selection(&mut state, &data, default_variant_index(&data));
        state.weapon = Some(weapon);
        animation::pose_for_shot(&source, &entry, &mut data, &mut state, clip);
        (data, state)
    }

    /// With `BLAM_TEST_H4EK` (the kit's tags): the Chief holding the AR in
    /// `combat:rifle:idle` puts each IK set point's marker on the weapon's, the
    /// left hand on `left_hand_spartan`, within a millimetre.
    #[test]
    fn h4_chief_left_hand_holds_the_assault_rifle() {
        let Some(root) = kit_root(GameId::Halo4) else {
            return;
        };
        let (data, mut state) = h4_chief_with_rifle(&root, "combat:rifle:idle");
        let points = state.animation.pose.as_ref().unwrap().h4_ik_set.clone();
        assert!(!points.is_empty(), "the clip's IK set");
        let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
        let posed = |marker: &crate::app::model_preview::animation::reach_ik::ReachIkMarker| {
            let skin = &rows[marker.node * 3..marker.node * 3 + 3];
            let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
            std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
        };
        let markers = &data.preview.gen3.ik_markers;
        let find = |holder, name: &str| markers.iter().find(|marker| marker.holder == holder && marker.name.eq_ignore_ascii_case(name));
        let mut solved = 0;
        for point in points.iter() {
            let (Some(source), Some(destination)) =
                (find(crate::app::model_preview::animation::reach_ik::ReachIkHolder::Unit, &point.source), find(point.holder, &point.destination))
            else {
                continue;
            };
            let (from, to) = (posed(source), posed(destination));
            let apart = (0..3).map(|axis| (from[axis] - to[axis]).powi(2)).sum::<f32>().sqrt();
            eprintln!("NOTE IK {} -> {} apart {apart}", point.source, point.destination);
            // A millimetre is 1/3048 of a world unit.
            assert!(apart < 1.0 / 3048.0, "{} misses {} by {apart}", point.source, point.destination);
            solved += 1;
        }
        assert!(solved > 0, "no point found both markers: {points:?}");
        // Without the set the animation alone leaves the hand off the grip.
        let mut pose = (**state.animation.pose.as_ref().unwrap()).clone();
        pose.h4_ik_set = Arc::default();
        state.animation.pose = Some(Arc::new(pose));
        let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
        let posed = |marker: &crate::app::model_preview::animation::reach_ik::ReachIkMarker| {
            let skin = &rows[marker.node * 3..marker.node * 3 + 3];
            let at = [marker.bind.m[0][3], marker.bind.m[1][3], marker.bind.m[2][3]];
            std::array::from_fn::<f32, 3, _>(|row| skin[row][0] * at[0] + skin[row][1] * at[1] + skin[row][2] * at[2] + skin[row][3])
        };
        let point = &points[0];
        let (from, to) = (posed(find(crate::app::model_preview::animation::reach_ik::ReachIkHolder::Unit, &point.source).unwrap()), posed(find(point.holder, &point.destination).unwrap()));
        let apart = (0..3).map(|axis| (from[axis] - to[axis]).powi(2)).sum::<f32>().sqrt();
        eprintln!("NOTE IK off: apart {apart}");
        assert!(apart > 1.0 / 3048.0, "the clip alone already reaches the grip ({apart}), so this proves nothing");
    }

    /// With `BLAM_TEST_H4EK`: the AR reload's chain events, whose unused data
    /// indices are 0xFF, become chain tracks on the held weapon and solve.
    #[test]
    fn h4_chief_reload_reads_its_ik_chains() {
        let Some(root) = kit_root(GameId::Halo4) else {
            return;
        };
        let (data, state) = h4_chief_with_rifle(&root, "combat:rifle:ar:reload_1");
        let pose = state.animation.pose.as_ref().unwrap();
        assert!(!pose.reach_ik.is_empty(), "the reload's chains");
        assert!(pose.reach_ik.iter().any(|track| track.holder == crate::app::model_preview::animation::reach_ik::ReachIkHolder::Weapon(1)));
        let rows = animation::animation_skinning_rows(&data, &state).expect("a posed frame");
        assert!(rows.iter().flatten().all(|lane| lane.is_finite()));
    }
}
