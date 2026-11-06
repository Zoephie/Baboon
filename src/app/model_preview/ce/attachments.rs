//! What a Halo CE object brings with it at spawn. A unit's weapons are objects
//! of their own: a tank's gun is not in its model. `unit_add_initial_weapons`
//! makes them, and `unit_ready_desired_weapon` hangs the first on a marker
//! the unit's animation graph names.

use blam_tags::math::Matrix4;
use blam_tags::{TagFile, TagStruct};

use super::functions::{CeObjectFunctions, read_ce_forced_permutation, read_ce_object_functions};
use super::super::dx9::passes::{H2ChangeColors, read_h2_change_colors};
use super::super::loading::build_render_preview;
use super::super::*;
use super::super::attachment::*;
use crate::core::source::TagSource;

/// The stance a unit nobody is driving stands in.
const RESTING_SEAT: &str = "stand";

/// An object hung off the one that was opened.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeAttachedObject {
    pub(crate) name: String,
    /// The weapon's tag, and its markers where the hand holds them, on the
    /// hand's node: what its attachments and widgets hang on.
    pub(crate) path: String,
    pub(crate) markers: Vec<RenderModelPreviewMarker>,
    pub(crate) functions: Option<CeObjectFunctions>,
    pub(crate) change_colors: H2ChangeColors,
}

/// Clips the game runs without weapon IK, by the last word of their names.
/// `unit_animation_weapon_ik` turns it off for an action, a soft ping, and
/// the states of `unit_animation_set_state` these play in.
const NO_WEAPON_IK: &[&str] = &[
    // Actions.
    "disarm",
    "drop",
    "ready",
    "put-away",
    "reload",
    "reload-1",
    "reload-2",
    "melee",
    "throw-grenade",
    "overheat",
    // States.
    "airborne-dead",
    "landing-dead",
    "enter",
    "exit",
    "melee-airborne",
    "melee-continuous",
    "resurrect-front",
    "resurrect-back",
    "leap-start",
    "leap-airborne",
    "leap-melee",
    // Impulses, which `unit_start_animation_impulse` plays in one state.
    "dive-front",
    "dive-back",
    "dive-left",
    "dive-right",
    "surprise-front",
    "surprise-back",
    "berserk",
    "evade-left",
    "evade-right",
    "signal-move",
    "signal-attack",
    "warn",
    "celebrate",
    "panic",
    // Pings, by the part of the body hit.
    "head",
    "chest",
    "gut",
    "l-arm",
    "r-arm",
    "l-hand",
    "r-hand",
    "l-leg",
    "r-leg",
    "l-thigh",
    "r-thigh",
    "l-foot",
    "r-foot",
];
/// Whether the game keeps a held weapon's IK points on through a clip.
pub(in crate::app::model_preview) fn clip_runs_weapon_ik(clip: &str) -> bool {
    let last = clip.rsplit(' ').next().unwrap_or(clip);
    // `%1` numbers a clip's permutations.
    !NO_WEAPON_IK.contains(&last.split('%').next().unwrap_or(last))
}

/// `weapon_class_animation_block` slots of the aiming screens.
const AIM_STILL: usize = 25;
const AIM_MOVE: usize = 26;
/// `unit_seat_animation_block` slot of the looking screen.
const LOOK: usize = 9;

/// Which aiming screen `unit_animation_state_get_aiming_screen_index` gives a
/// clip's state. The screens run on through an overlay, not an action.
fn aiming_screen_of(clip: &str) -> Option<usize> {
    let last = clip.rsplit(' ').next()?;
    match last.split('%').next()? {
        "idle" | "turn-left" | "turn-right" | "airborne" | "land-soft" | "land-hard" | "fire-1"
        | "fire-2" | "chamber-1" | "chamber-2" | "charged-1" | "charged-2" => Some(AIM_STILL),
        name if name.starts_with("move-")
            || name.starts_with("stunned-")
            || name.starts_with("slide-") =>
        {
            Some(AIM_MOVE)
        }
        _ => None,
    }
}

/// `aiming_screen_apply`'s frame for no yaw or pitch off the facing: screens
/// run right to left, then down to up.
fn resting_screen_frame(holder: TagStruct<'_>) -> usize {
    let count = |name: &str| holder.read_int_any(name).unwrap_or(0).max(0) as usize;
    let yaws = count("right frame count") + count("left frame count") + 1;
    count("right frame count") + count("down pitch frame count") * yaws
}

/// The screens over a clip for a unit aiming and looking straight ahead, in
/// the order `unit_preprocess_node_orientations` applies them, each as an
/// animation index and its frame.
pub(in crate::app::model_preview) fn resting_aim(graph: &TagFile, clip: &str) -> Vec<(usize, usize)> {
    let Some(screen) = aiming_screen_of(clip) else {
        return Vec::new();
    };
    let mut words = clip.split(' ');
    let (Some(seat_label), Some(class)) = (words.next(), words.next()) else {
        return Vec::new();
    };
    let seats = elements(graph.root(), "UNITS");
    let is_seat = |seat: &&TagStruct<'_>| {
        seat.read_string("label").is_some_and(|label| label.eq_ignore_ascii_case(seat_label))
    };
    let Some(seat) = seats.iter().find(is_seat) else {
        return Vec::new();
    };
    let clip_at = |holder: TagStruct<'_>, slot: usize| {
        let entry = *elements(holder, "animations").get(slot)?;
        usize::try_from(entry.read_block_index("animation")).ok()
    };
    let weapons = elements(*seat, "weapons");
    let weapon = weapons.iter().find(|weapon| {
        weapon.read_string("name").is_some_and(|name| name.eq_ignore_ascii_case(class))
    });
    let aim = weapon.and_then(|weapon| Some((clip_at(*weapon, screen)?, resting_screen_frame(*weapon))));
    let look = clip_at(*seat, LOOK).map(|look| (look, resting_screen_frame(*seat)));
    aim.into_iter().chain(look).collect()
}

/// A hand held to a marker of the weapon: an IK point of the animations.
#[derive(Debug, Clone)]
pub(crate) struct CeWeaponIk {
    /// Which clips hold it: a Halo CE weapon class, the second word of their
    /// names, or a Halo 2 mode, the first.
    pub(in crate::app::model_preview) scope: String,
    hand: usize,
    elbow: usize,
    shoulder: usize,
    /// From the marker on the hand to its node.
    hand_from_marker: Matrix4,
    /// The node the weapon rides on.
    weapon_node: usize,
    /// The marker on the weapon, in bind space.
    grip: Matrix4,
    /// The nodes below the hand, which move with it.
    pub(in crate::app::model_preview) carried: Vec<usize>,
}

impl CeWeaponIk {
    pub(in crate::app::model_preview) fn hand(&self) -> usize {
        self.hand
    }

    /// Whether a clip is one of the held weapon's, played with IK on.
    pub(in crate::app::model_preview) fn runs_through(&self, clip: &str) -> bool {
        clip.split(' ').any(|word| word.eq_ignore_ascii_case(&self.scope)) && clip_runs_weapon_ik(clip)
    }

    /// Pose the arm. `worlds` holds each node's posed transform, and
    /// `inverse_binds` what `skin` undoes of it.
    pub(in crate::app::model_preview) fn apply(&self, worlds: &mut [Matrix4], inverse_binds: &[Matrix4]) {
        let (Some(weapon), Some(inverse)) =
            (worlds.get(self.weapon_node).copied(), inverse_binds.get(self.weapon_node).copied())
        else {
            return;
        };
        let desired = weapon * inverse * self.grip * self.hand_from_marker;
        let mut arm = [self.shoulder, self.elbow, self.hand].map(|node| worlds.get(node).copied());
        let [Some(shoulder), Some(elbow), Some(hand)] = &mut arm else {
            return;
        };
        let was = *hand;
        reach_with_arm(desired, shoulder, elbow, hand);
        (worlds[self.shoulder], worlds[self.elbow], worlds[self.hand]) = (*shoulder, *elbow, *hand);
        // `render_model_adjust_hand`: what it did to the hand, from there down.
        let moved = *hand * rigid_inverse(&was);
        for node in &self.carried {
            worlds[*node] = moved * worlds[*node];
        }
    }
}

pub(in crate::app::model_preview) fn elements<'a>(holder: TagStruct<'a>, name: &str) -> Vec<TagStruct<'a>> {
    let block = holder.field(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(|index| block.as_ref()?.element(index))
        .collect()
}

/// Whether the flags field `name` has its option `option` set, the option
/// named as the editors show it (its `#help` left out).
pub(in crate::app::model_preview) fn has_flag(holder: TagStruct<'_>, name: &str, option: &str) -> bool {
    holder.field(name).and_then(|field| field.flag(option)).is_some_and(|flag| flag.is_set())
}

/// What the animations say of a held weapon.
#[derive(Debug, Clone, Default, PartialEq)]
struct WeaponHold {
    class: String,
    hand: String,
    grip: String,
    /// Marker on the unit, then the marker on the weapon it holds to.
    ik_points: Vec<(String, String)>,
}

/// A weapon a biped's animations can hold.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CeEquippable {
    pub(crate) path: String,
    pub(crate) name: String,
    /// The `weapons` entry of the animations that holds it.
    pub(crate) class: String,
    label: String,
}

/// What a biped can hold, and which clips the held weapon leaves out.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeBipedWeapons {
    pub(crate) equippable: Vec<CeEquippable>,
    pub(crate) hidden_clips: std::collections::BTreeSet<usize>,
}

/// The game's own name for holding nothing, an empty type label in the tags.
const UNARMED: &str = "unarmed";

fn type_matches(kind: TagStruct<'_>, weapon_label: &str) -> bool {
    kind.read_string("label").is_some_and(|label| {
        label.eq_ignore_ascii_case(weapon_label) || (label.is_empty() && weapon_label == UNARMED)
    })
}

/// `unit_set_or_test_seat_and_weapon_label`: every seat and weapon is tried
/// and the last that matches stands. An empty type label is for `unarmed`.
fn weapon_hold(graph: &TagFile, weapon_label: &str) -> Option<WeaponHold> {
    let mut found = None;
    for seat in elements(graph.root(), "UNITS") {
        if !seat.read_string("label").is_some_and(|label| label.eq_ignore_ascii_case(RESTING_SEAT)) {
            continue;
        }
        for weapon in elements(seat, "weapons") {
            if elements(weapon, "weapon types").iter().any(|kind| type_matches(*kind, weapon_label)) {
                found = Some(WeaponHold {
                    class: weapon.read_string("name").unwrap_or_default(),
                    hand: weapon.read_string("hand marker").unwrap_or_default(),
                    grip: weapon.read_string("grip marker").unwrap_or_default(),
                    ik_points: elements(weapon, "ik points")
                        .iter()
                        .map(|point| {
                            (
                                point.read_string("marker").unwrap_or_default(),
                                point.read_string("attach to marker").unwrap_or_default(),
                            )
                        })
                        .collect(),
                });
            }
        }
    }
    found
}

/// Every clip a block of `animation` indices starts, with the permutations
/// hanging off each by `next animation`.
fn add_clips(
    holder: TagStruct<'_>,
    next: &[i16],
    clips: &mut std::collections::BTreeSet<usize>,
) {
    for entry in elements(holder, "animations") {
        let mut clip = usize::try_from(entry.read_block_index("animation")).ok();
        while let Some(index) = clip.filter(|index| *index < next.len()) {
            if !clips.insert(index) {
                break;
            }
            clip = usize::try_from(next[index]).ok();
        }
    }
}

/// Clips of weapons not held. Whatever no weapon names stays. A seat with
/// nothing for this weapon takes what `unit_enter_seat` does with no label:
/// the first type of the last weapon that has one.
fn hidden_clips(graph: &TagFile, weapon_label: &str) -> std::collections::BTreeSet<usize> {
    let next: Vec<i16> = elements(graph.root(), "animations")
        .iter()
        .map(|animation| animation.read_block_index("next animation"))
        .collect();
    let (mut held, mut other) = Default::default();
    for seat in elements(graph.root(), "UNITS") {
        let weapons = elements(seat, "weapons");
        let found = weapons.iter().enumerate().rev().find_map(|(index, weapon)| {
            let kinds = elements(*weapon, "weapon types");
            let kind = kinds.iter().position(|kind| type_matches(*kind, weapon_label))?;
            Some((index, kind))
        });
        let any = || {
            let has_types = |weapon: &TagStruct<'_>| !elements(*weapon, "weapon types").is_empty();
            Some((weapons.iter().rposition(has_types)?, 0))
        };
        let Some((held_weapon, held_kind)) = found.or_else(any) else {
            continue;
        };
        for (index, weapon) in weapons.iter().enumerate() {
            let set = if index == held_weapon { &mut held } else { &mut other };
            add_clips(*weapon, &next, set);
            for (kind_index, kind) in elements(*weapon, "weapon types").iter().enumerate() {
                let is_held = index == held_weapon && kind_index == held_kind;
                add_clips(*kind, &next, if is_held { &mut held } else { &mut other });
            }
        }
    }
    other.difference(&held).copied().collect()
}

/// `.weapon` tags of a loose folder, by path without the extension.
pub(in crate::app::model_preview) fn loose_weapon_paths(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for path in std::fs::read_dir(folder).into_iter().flatten().flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "weapon") {
                let inner = path.strip_prefix(root).unwrap_or(&path).with_extension("");
                found.push(inner.to_string_lossy().replace('/', "\\"));
            }
        }
    }
    found
}

/// The weapons `unit_can_use_weapon` lets this unit hold in its resting seat:
/// the biped has no say, only its animations do.
fn equippable_of(source: &TagSource, graph: &TagFile) -> Vec<CeEquippable> {
    let TagSource::LooseFolder { root, .. } = source else {
        return Vec::new();
    };
    let mut equippable: Vec<_> = loose_weapon_paths(root)
        .into_iter()
        .filter_map(|path| {
            let weapon = load_referenced_tag_from_source(source, &path, "weapon", b"weap").ok()?;
            let label = weapon.root().read_string("label")?;
            let class = weapon_hold(graph, &label)?.class;
            Some(CeEquippable { name: tag_path::name(&path).to_owned(), path, class, label })
        })
        .collect();
    equippable.sort_by(|a, b| (&a.class, &a.name).cmp(&(&b.class, &b.name)));
    equippable
}

/// A biped's weapon choices, with `chosen` held. `known` spares the scan when
/// only the choice changed.
pub(in crate::app::model_preview) fn biped_weapons(
    source: &TagSource,
    object: &TagFile,
    chosen: Option<&str>,
    known: Option<&[CeEquippable]>,
) -> CeBipedWeapons {
    let Some(graph) = object_reference(object, "animation graph").and_then(|graph| {
        load_referenced_tag_from_source(source, &graph, "model_animations", b"antr").ok()
    }) else {
        return CeBipedWeapons::default();
    };
    let equippable = known.map_or_else(|| equippable_of(source, &graph), <[_]>::to_vec);
    let held = chosen.and_then(|path| equippable.iter().find(|weapon| weapon.path == path));
    // With nothing held every clip stays.
    let hidden_clips =
        held.map_or_else(Default::default, |weapon| hidden_clips(&graph, weapon.label.as_str()));
    CeBipedWeapons { equippable, hidden_clips }
}

/// The clip to play after a weapon change: the same one if it is still shown,
/// or this weapon's idle in the same seat.
pub(in crate::app::model_preview) fn clip_after_weapon_change(
    clips: &[PreviewAnimationEntry],
    hidden: &std::collections::BTreeSet<usize>,
    clip: &str,
    class: &str,
) -> Option<usize> {
    let shown = |name: &str| {
        // A clip with permutations is named for its first, `idle%0`.
        let matches = |entry: &PreviewAnimationEntry| entry.name.split('%').next() == Some(name);
        (0..clips.len()).find(|index| matches(&clips[*index]) && !hidden.contains(index))
    };
    let seat = clip.split(' ').next()?;
    shown(clip.split('%').next()?).or_else(|| shown(&format!("{seat} {class} idle")))
}

/// The struct of an object tag holding a block, down its inherited structs.
pub(in crate::app::model_preview) fn object_block<'a>(object: &'a TagFile, block: &str) -> Option<TagStruct<'a>> {
    let mut holder = object.root();
    loop {
        if holder.field(block).is_some() {
            return Some(holder);
        }
        holder = holder.fields_all().find_map(|field| field.as_struct())?;
    }
}

/// The unit's first weapon, which it has ready, by path.
pub(in crate::app::model_preview) fn first_weapon(object: &TagFile) -> Option<String> {
    let weapons = object_block(object, "weapons")?.field("weapons")?.as_block()?;
    (0..weapons.len())
        .filter_map(|index| weapons.element(index)?.read_tag_ref_path("weapon"))
        .find(|path| !path.is_empty())
}

/// A marker of a held object, where holding it puts it.
pub(in crate::app::model_preview) fn attached_marker_frame(
    marker: &RenderModelPreviewMarker,
    parent: &MarkerFrame,
    child: &MarkerFrame,
) -> MarkerFrame {
    MarkerFrame {
        axes: marker.axes.map(|axis| attach_vector(parent, child, axis)),
        position: attach_point(parent, child, marker.position),
        node: parent.node,
    }
}

/// An IK point of a weapon held on `parent`, if both markers and the arm exist.
pub(in crate::app::model_preview) fn weapon_ik(
    preview: &RenderModelPreview,
    weapon: &RenderModelPreview,
    scope: &str,
    parent: MarkerFrame,
    child: MarkerFrame,
    marker: &str,
    attach_to: &str,
) -> Option<CeWeaponIk> {
    let named = |model: &RenderModelPreview, name: &str| {
        let found = model.markers.iter().any(|marker| marker.name.eq_ignore_ascii_case(name));
        found.then(|| MarkerFrame::find(model, name))
    };
    let on_hand = named(preview, marker)?;
    let on_weapon = named(weapon, attach_to)?;
    let hand = on_hand.node.max(0) as usize;
    let elbow = usize::try_from(preview.nodes.get(hand)?.parent).ok()?;
    let shoulder = usize::try_from(preview.nodes.get(elbow)?.parent).ok()?;
    let rows = preview.nodes.get(hand)?.inverse_bind;
    let inverse_bind = Matrix4 { m: [rows[0], rows[1], rows[2], [0.0, 0.0, 0.0, 1.0]] };
    let moved = on_weapon.axes.map(|axis| attach_vector(&parent, &child, axis));
    let position = attach_point(&parent, &child, on_weapon.position);
    Some(CeWeaponIk {
        scope: scope.to_owned(),
        hand,
        elbow,
        shoulder,
        hand_from_marker: rigid_inverse(&(inverse_bind * matrix_of(on_hand.axes, on_hand.position))),
        weapon_node: parent.node.max(0) as usize,
        grip: matrix_of(moved, position),
        carried: Vec::new(),
    })
}

/// Hang the unit's ready weapon on it, returning the object that adds.
pub(in crate::app::model_preview) fn attach_ce_weapon(
    source: &TagSource,
    object: &TagFile,
    path: &str,
    preview: &mut RenderModelPreview,
) -> Option<CeAttachedObject> {
    let weapon = load_referenced_tag_from_source(source, path, "weapon", b"weap").ok()?;
    let model = object_reference(&weapon, "model")?;
    let render = load_referenced_tag_from_source(source, &model, "gbxmodel", b"mod2").ok()?;
    let mut attached = build_render_preview(&render).ok()?;
    let forced = read_ce_forced_permutation(&weapon);
    if forced != 0 {
        attached.materials.iter_mut().for_each(|material| material.permutation = forced);
    }
    let label = weapon.root().read_string("label").unwrap_or_default();
    let hold = object_reference(object, "animation graph")
        .and_then(|graph| {
            load_referenced_tag_from_source(source, &graph, "model_animations", b"antr").ok()
        })
        .and_then(|graph| weapon_hold(&graph, &label))
        .unwrap_or_default();
    let parent = MarkerFrame::find(preview, &hold.hand);
    let child = MarkerFrame::find(&attached, &hold.grip);
    let name = tag_path::name(path).to_owned();
    for (marker, attach_to) in &hold.ik_points {
        preview.weapon_ik.extend(weapon_ik(preview, &attached, &hold.class, parent, child, marker, attach_to));
    }
    let lights = super::lights::read_attached_lights(source, &weapon, 1, &attached, parent, child);
    preview.ce_lights.extend(lights);
    let markers = attached
        .markers
        .iter()
        .map(|marker| {
            let held = attached_marker_frame(marker, &parent, &child);
            RenderModelPreviewMarker {
                position: held.position,
                axes: held.axes,
                node_index: held.node as i16,
                ..marker.clone()
            }
        })
        .collect();
    merge_attached(preview, attached, parent, child, 1, &name);
    Some(CeAttachedObject {
        name,
        path: path.to_owned(),
        markers,
        functions: read_ce_object_functions(&weapon),
        change_colors: read_h2_change_colors(&weapon),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::model_preview::common::math::{dot, unit_by_reciprocal};

    struct CeAnimationsForTest(Vec<String>);

    impl CeAnimationsForTest {
        fn new(graph: &TagFile) -> Self {
            let clips = elements(graph.root(), "animations");
            Self(clips.iter().map(|clip| clip.read_string("name").unwrap_or_default()).collect())
        }

        fn name(&self, index: usize) -> &str {
            &self.0[index]
        }
    }

    fn turned() -> MarkerFrame {
        // A quarter turn about up, set off along x.
        MarkerFrame {
            axes: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            position: [2.0, 0.0, 0.0],
            node: 3,
        }
    }

    #[test]
    fn a_grip_lands_on_the_hand_with_its_axes() {
        let child = MarkerFrame { position: [0.0, 0.0, 1.0], ..MarkerFrame::ORIGIN };
        assert_eq!(attach_point(&turned(), &child, [0.0, 0.0, 1.0]), [2.0, 0.0, 0.0]);
        assert_eq!(attach_point(&turned(), &child, [1.0, 0.0, 1.0]), [2.0, 1.0, 0.0]);
        assert_eq!(attach_vector(&turned(), &child, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
    }

    fn at(x: f32, y: f32, z: f32) -> Matrix4 {
        matrix_of(MarkerFrame::ORIGIN.axes, [x, y, z])
    }

    fn distance(a: &Matrix4, b: &Matrix4) -> f32 {
        let gap = apart(place_of(a), place_of(b));
        dot(gap, gap).sqrt()
    }

    #[test]
    fn an_arm_reaches_keeping_its_length_and_its_side() {
        let (mut shoulder, mut elbow, mut hand) = (at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(1.0, 1.0, 0.0));
        reach_with_arm(at(1.2, 0.2, 0.0), &mut shoulder, &mut elbow, &mut hand);
        assert!(distance(&hand, &at(1.2, 0.2, 0.0)) < 1e-5);
        assert!((distance(&elbow, &shoulder) - 1.0).abs() < 1e-5);
        assert!((distance(&hand, &elbow) - 1.0).abs() < 1e-5);
        // The elbow started below the line to the goal, and stays there.
        assert!(place_of(&elbow)[1] < place_of(&elbow)[0]);
        // Each bone's first axis still runs down it.
        let down = unit_by_reciprocal(apart(place_of(&elbow), place_of(&shoulder)));
        assert!(dot(axis_of(&shoulder, 0), down) > 0.999);
    }

    #[test]
    fn a_goal_out_of_reach_leaves_the_arm_just_short_of_straight() {
        let (mut shoulder, mut elbow, mut hand) = (at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(1.0, 1.0, 0.0));
        reach_with_arm(at(5.0, 0.0, 0.0), &mut shoulder, &mut elbow, &mut hand);
        assert!((distance(&hand, &shoulder) - 2.0 * LONGEST_REACH).abs() < 1e-5);
    }

    /// Bad data gives an arm whose joints sit together: it stays as it was.
    #[test]
    fn an_arm_with_no_length_is_left_alone() {
        let (mut shoulder, mut elbow, mut hand) = (at(1.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(1.0, 0.0, 0.0));
        reach_with_arm(at(3.0, 0.0, 0.0), &mut shoulder, &mut elbow, &mut hand);
        let all = [shoulder, elbow, hand].map(|matrix| matrix.m.into_iter().flatten().all(f32::is_finite));
        assert_eq!(all, [true; 3]);
        assert_eq!(distance(&hand, &at(1.0, 0.0, 0.0)), 0.0);
    }

    #[test]
    fn the_inverse_of_a_rigid_transform_undoes_it() {
        let matrix = matrix_of(turned().axes, [1.0, 2.0, 3.0]);
        let round = matrix * rigid_inverse(&matrix);
        assert!(distance(&round, &IDENTITY) < 1e-6);
        assert!((axis_of(&round, 0)[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ik_runs_through_idle_and_stops_for_actions() {
        assert!(clip_runs_weapon_ik("stand rifle idle"));
        assert!(clip_runs_weapon_ik("stand rifle move-front"));
        assert!(!clip_runs_weapon_ik("stand rifle ar melee"));
        assert!(!clip_runs_weapon_ik("stand rifle ar reload-1"));
        assert!(!clip_runs_weapon_ik("stand rifle throw-grenade"));
        assert!(!clip_runs_weapon_ik("stand rifle s-ping gut"));
        assert!(!clip_runs_weapon_ik("stand rifle h-kill l-hand%1"));
        assert!(clip_runs_weapon_ik("stand rifle move-front%0"));
        assert!(!clip_runs_weapon_ik("stand rifle evade-left"));
        // An overlay leaves the state alone, so it keeps the hand on.
        assert!(clip_runs_weapon_ik("stand rifle ar fire-1"));
        assert!(clip_runs_weapon_ik("stand rifle ar chamber-1"));
    }

    #[test]
    fn ik_belongs_to_the_held_weapons_clips() {
        let ik = CeWeaponIk {
            scope: "rifle".to_owned(),
            hand: 2,
            elbow: 1,
            shoulder: 0,
            hand_from_marker: IDENTITY,
            weapon_node: 3,
            grip: IDENTITY,
            carried: Vec::new(),
        };
        assert!(ik.runs_through("stand rifle idle"));
        assert!(!ik.runs_through("stand unarmed idle"));
        assert!(!ik.runs_through("stand rifle ar melee"));
    }

    #[test]
    fn each_state_takes_its_aiming_screen() {
        assert_eq!(aiming_screen_of("stand rifle idle"), Some(AIM_STILL));
        assert_eq!(aiming_screen_of("stand pistol idle%0"), Some(AIM_STILL));
        assert_eq!(aiming_screen_of("stand rifle move-front"), Some(AIM_MOVE));
        assert_eq!(aiming_screen_of("stand rifle ar fire-1"), Some(AIM_STILL));
        // Actions, and states past the aiming ones, keep the screens off.
        assert_eq!(aiming_screen_of("stand rifle ar melee"), None);
        assert_eq!(aiming_screen_of("stand rifle ar reload-1"), None);
        assert_eq!(aiming_screen_of("stand rifle throw-grenade"), None);
        assert_eq!(aiming_screen_of("stand rifle gesture"), None);
    }

    #[test]
    fn a_hidden_clip_gives_way_to_the_weapons_idle() {
        let clip = |name: &str| PreviewAnimationEntry {
            name: name.to_owned(),
            frame_count: 2,
            kind: "jma",
            playable: true,
        };
        let clips = [clip("stand rifle idle"), clip("stand pistol idle"), clip("stand gut")];
        let hidden = std::collections::BTreeSet::from([0]);
        let after = |name, class| clip_after_weapon_change(&clips, &hidden, name, class);
        assert_eq!(after("stand gut", "pistol"), Some(2));
        assert_eq!(after("stand rifle idle", "pistol"), Some(1));
        assert_eq!(after("stand rifle idle", "sword"), None);
        let clips = [clip("stand pistol idle%0"), clip("stand pistol idle%1")];
        let none = Default::default();
        assert_eq!(clip_after_weapon_change(&clips, &none, "stand rifle idle", "pistol"), Some(0));
    }

    #[test]
    fn no_marker_name_is_the_origin() {
        let mut preview = RenderModelPreview::default();
        preview.markers.push(RenderModelPreviewMarker {
            name: "Gun".to_owned(),
            node_index: 2,
            position: [1.0, 2.0, 3.0],
            scale: 1.0,
            axes: MarkerFrame::ORIGIN.axes,
            only_under: None,
        });
        assert_eq!(MarkerFrame::find(&preview, ""), MarkerFrame::ORIGIN);
        assert_eq!(MarkerFrame::find(&preview, "missing"), MarkerFrame::ORIGIN);
        assert_eq!(MarkerFrame::find(&preview, "gun").node, 2);
    }

    #[test]
    fn a_merged_model_keeps_its_own_indices_materials_and_owner() {
        let mut preview = RenderModelPreview {
            vertices: vec![RenderModelPreviewVertex::default(); 3],
            indices: vec![0, 1, 2],
            materials: vec![RenderModelPreviewMaterial::default()],
            ..Default::default()
        };
        let other = RenderModelPreview {
            vertices: vec![RenderModelPreviewVertex::default(); 3],
            indices: vec![0, 2, 1],
            materials: vec![RenderModelPreviewMaterial::default()],
            regions: vec![RenderModelPreviewRegion { name: "body".to_owned(), permutations: Vec::new() }],
            batches: vec![RenderModelPreviewBatch {
                region_name: "body".to_owned(),
                index_count: 3,
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_attached(&mut preview, other, turned(), MarkerFrame::ORIGIN, 1, "gun");
        assert_eq!(preview.indices, [0, 1, 2, 3, 5, 4]);
        let batch = &preview.batches[0];
        assert_eq!((batch.index_start, batch.material_index), (3, 1));
        assert_eq!(batch.region_name, "gun: body");
        assert_eq!(preview.regions[0].name, "gun: body");
        assert_eq!(preview.materials[1].object, 1);
        assert_eq!(preview.vertices[3].node_indices[0], 3.0);
        assert_eq!(preview.vertices[3].node_weights, [1.0, 0.0, 0.0, 0.0]);
    }

    /// With `BLAM_TEST_HCEEK`: the kit's markers, by the Chief's rifle and the
    /// Warthog's gun, and the empty pair a tank's gun leaves.
    #[test]
    fn a_unit_finds_its_weapons_markers_in_its_animations() {
        let Ok(root) = std::env::var("BLAM_TEST_HCEEK") else {
            eprintln!("skipping: BLAM_TEST_HCEEK is not set");
            return;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(crate::core::game::GameId::HaloCe),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        let graph = |path: &str| {
            load_referenced_tag_from_source(&source, path, "model_animations", b"antr").unwrap()
        };
        let hold = |path: &str, label: &str| weapon_hold(&graph(path), label);
        let hand = |path: &str, label: &str| hold(path, label).map(|hold| hold.hand);
        assert_eq!(hand("vehicles\\warthog\\warthog", "cg").as_deref(), Some("gun"));
        assert_eq!(hand("vehicles\\scorpion\\scorpion", "cannon").as_deref(), Some(""));
        assert_eq!(hand("vehicles\\scorpion\\scorpion", "nothing"), None);
        let rifle = hold("characters\\cyborg\\cyborg", "ar").unwrap();
        assert_eq!(rifle.class, "rifle");
        assert_eq!(rifle.hand, "right hand");
        assert_eq!(rifle.ik_points, [("left hand".to_owned(), "left hand cyborg".to_owned())]);

        let cyborg_graph = graph("characters\\cyborg\\cyborg");
        let names = CeAnimationsForTest::new(&cyborg_graph);
        let name_of = |index: usize| names.name(index);
        let aim = resting_aim(&cyborg_graph, "stand rifle idle");
        assert_eq!(name_of(aim[0].0), "stand rifle aim-still");
        assert_eq!(aim[0].1, 4);
        assert_eq!(name_of(aim[1].0), "stand look");
        assert!(resting_aim(&cyborg_graph, "stand rifle ar melee").is_empty());

        let cyborg = load_referenced_tag_from_source(&source, "characters\\cyborg\\cyborg", "biped", b"bipd").unwrap();
        let marine = graph("characters\\marine\\marine");
        assert_eq!(weapon_hold(&marine, "ar").unwrap().class, "rifle");
        assert_eq!(weapon_hold(&marine, "fb"), None);
        let held = |label: &str, path: &str| {
            let equippable = [CeEquippable {
                path: path.to_owned(),
                name: String::new(),
                class: String::new(),
                label: label.to_owned(),
            }];
            let hidden = biped_weapons(&source, &cyborg, Some(path), Some(&equippable)).hidden_clips;
            let tag = graph("characters\\cyborg\\cyborg");
            let clips: Vec<_> = elements(tag.root(), "animations")
                .iter()
                .map(|clip| clip.read_string("name").unwrap_or_default())
                .collect();
            move |name: &str| {
                let index = clips.iter().position(|clip| clip == name).unwrap();
                hidden.contains(&index)
            }
        };
        let rifle = held("ar", "rifle");
        assert!(!rifle("stand rifle idle"));
        assert!(!rifle("stand rifle ar reload-1"));
        assert!(rifle("stand rifle sg reload-1"));
        assert!(rifle("stand pistol idle"));
        assert!(rifle("stand unarmed move-front"));
        let pistol = held("hp", "pistol");
        assert!(!pistol("stand pistol idle"));
        assert!(pistol("stand rifle idle"));
        let unarmed = biped_weapons(&source, &cyborg, None, Some(&[])).hidden_clips;
        assert!(unarmed.is_empty(), "nothing held, nothing hidden: {} clips", unarmed.len());
        // `alert` has no rocket launcher, so it falls back on its last weapon.
        let tag = graph("characters\\cyborg\\cyborg");
        let alert_rifle = elements(tag.root(), "animations")
            .iter()
            .filter_map(|clip| clip.read_string("name"))
            .find(|name| name.starts_with("alert rifle "))
            .unwrap();
        let launcher = held("rl", "launcher");
        assert!(launcher(&alert_rifle));
        assert!(!launcher("alert unarmed move-front"));
        let nothing = held("", "unused");
        assert!(!nothing("stand unarmed move-front"));
        assert!(nothing("stand rifle idle"));
    }
}
