//! What a Halo CE object brings with it at spawn. A unit's weapons are objects
//! of their own: a tank's gun is not in its model. `unit_add_initial_weapons`
//! makes them, and `unit_ready_desired_weapon` hangs the first on a marker
//! the unit's animation graph names.

use blam_tags::math::Matrix4;
use blam_tags::{TagFile, TagStruct};

use super::ce_functions::{CeObjectFunctions, read_ce_forced_permutation, read_ce_object_functions};
use super::h2_passes::{H2ChangeColors, read_h2_change_colors};
use super::loading::build_render_preview;
use super::*;
use crate::core::source::TagSource;

/// The stance a unit nobody is driving stands in.
const RESTING_SEAT: &str = "stand";

/// An object hung off the one that was opened.
#[derive(Debug, Clone, Default)]
pub(crate) struct CeAttachedObject {
    pub(crate) name: String,
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
/// `inverse_kinematics_adjust_matrices` keeps the arm this much short of straight.
const LONGEST_REACH: f32 = 0.98;

/// Whether the game keeps a held weapon's IK points on through a clip.
pub(super) fn clip_runs_weapon_ik(clip: &str) -> bool {
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
pub(super) fn resting_aim(graph: &TagFile, clip: &str) -> Vec<(usize, usize)> {
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
    /// The weapon class whose clips hold it, the second word of their names.
    class: String,
    hand: usize,
    elbow: usize,
    shoulder: usize,
    /// From the marker on the hand to its node.
    hand_from_marker: Matrix4,
    /// The node the weapon rides on.
    weapon_node: usize,
    /// The marker on the weapon, in bind space.
    grip: Matrix4,
}

const IDENTITY: Matrix4 = Matrix4 {
    m: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scaled(a: [f32; 3], by: f32) -> [f32; 3] {
    a.map(|value| value * by)
}

fn added(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] + b[axis])
}

fn apart(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

/// Unit length, or as it is when it has none to speak of.
fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    if length >= 0.0001 { scaled(a, 1.0 / length) } else { a }
}

fn axis_of(matrix: &Matrix4, axis: usize) -> [f32; 3] {
    std::array::from_fn(|row| matrix.m[row][axis])
}

fn place_of(matrix: &Matrix4) -> [f32; 3] {
    axis_of(matrix, 3)
}

fn matrix_of(axes: [[f32; 3]; 3], position: [f32; 3]) -> Matrix4 {
    let mut matrix = IDENTITY;
    for row in 0..3 {
        for axis in 0..3 {
            matrix.m[row][axis] = axes[axis][row];
        }
        matrix.m[row][3] = position[row];
    }
    matrix
}

fn rigid_inverse(matrix: &Matrix4) -> Matrix4 {
    let axes = [axis_of(matrix, 0), axis_of(matrix, 1), axis_of(matrix, 2)];
    let place = place_of(matrix);
    let mut inverse = IDENTITY;
    for row in 0..3 {
        inverse.m[row][0] = axes[row][0];
        inverse.m[row][1] = axes[row][1];
        inverse.m[row][2] = axes[row][2];
        inverse.m[row][3] = -dot(axes[row], place);
    }
    inverse
}

/// `inverse_kinematics_adjust_matrices`: bend at the elbow, on the side it
/// already bends, so the hand reaches `desired`.
fn reach_with_arm(desired: Matrix4, shoulder: &mut Matrix4, elbow: &mut Matrix4, hand: &mut Matrix4) {
    let start = place_of(shoulder);
    let joint = place_of(elbow);
    let upper = dot(apart(joint, start), apart(joint, start)).sqrt();
    let fore = dot(apart(place_of(hand), joint), apart(place_of(hand), joint)).sqrt();
    let to_goal = apart(place_of(&desired), start);
    let mut reach = dot(to_goal, to_goal).sqrt();
    if reach < 0.0001 {
        return;
    }
    let toward = scaled(to_goal, 1.0 / reach);
    let side = unit(cross(toward, apart(joint, start)));
    let bend = cross(side, toward);
    let mut goal = desired;
    let longest = (upper + fore) * LONGEST_REACH;
    if longest < reach {
        reach = longest;
        let place = added(start, scaled(toward, reach));
        (0..3).for_each(|row| goal.m[row][3] = place[row]);
    }
    let along = (reach * reach + upper * upper - fore * fore) / (reach * 2.0);
    let out = (upper * upper - along * along).max(0.0).sqrt();
    // A bone's first axis runs down it, and its second stays as near as it can.
    let aimed = |x: [f32; 3], up: [f32; 3], place: [f32; 3]| {
        let z = unit(cross(x, up));
        matrix_of([x, cross(z, x), z], place)
    };
    let upper_axis = unit(added(scaled(toward, along), scaled(bend, out)));
    let fore_axis = unit(added(scaled(toward, reach - along), scaled(bend, -out)));
    let bent = added(start, scaled(upper_axis, upper));
    *elbow = aimed(fore_axis, axis_of(elbow, 1), bent);
    *shoulder = aimed(upper_axis, axis_of(shoulder, 1), start);
    *hand = goal;
}

impl CeWeaponIk {
    /// Whether a clip is one of the held weapon's, played with IK on.
    pub(super) fn runs_through(&self, clip: &str) -> bool {
        clip.split(' ').any(|word| word.eq_ignore_ascii_case(&self.class)) && clip_runs_weapon_ik(clip)
    }

    /// Pose the arm. `worlds` holds each node's posed transform, and
    /// `inverse_binds` what `skin` undoes of it.
    pub(super) fn apply(&self, worlds: &mut [Matrix4], inverse_binds: &[Matrix4]) {
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
        reach_with_arm(desired, shoulder, elbow, hand);
        (worlds[self.shoulder], worlds[self.elbow], worlds[self.hand]) = (*shoulder, *elbow, *hand);
    }
}

/// A marker as a rigid transform: its three axes, then its position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MarkerFrame {
    pub(super) axes: [[f32; 3]; 3],
    pub(super) position: [f32; 3],
    pub(super) node: i16,
}

impl MarkerFrame {
    const ORIGIN: Self = Self {
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        position: [0.0; 3],
        node: 0,
    };

    /// `object_get_marker_by_name`: no name, or none found, is the origin.
    fn find(preview: &RenderModelPreview, name: &str) -> Self {
        preview
            .markers
            .iter()
            .find(|marker| !name.is_empty() && marker.name.eq_ignore_ascii_case(name))
            .map_or(Self::ORIGIN, |marker| Self {
                axes: marker.axes,
                position: marker.position,
                node: marker.node_index.max(0),
            })
    }

    /// A direction in this frame's axes, in the space the frame sits in.
    fn rotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            (0..3).map(|index| self.axes[index][axis] * value[index]).sum()
        })
    }

    /// `rotate`'s inverse: axes are unit and square to each other.
    fn unrotate(&self, value: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|index| (0..3).map(|axis| self.axes[index][axis] * value[axis]).sum())
    }
}

/// Where a point of the child goes when its `child` marker meets `parent`.
fn attach_point(parent: &MarkerFrame, child: &MarkerFrame, point: [f32; 3]) -> [f32; 3] {
    let local = child.unrotate(std::array::from_fn(|axis| point[axis] - child.position[axis]));
    let turned = parent.rotate(local);
    std::array::from_fn(|axis| turned[axis] + parent.position[axis])
}

fn attach_vector(parent: &MarkerFrame, child: &MarkerFrame, vector: [f32; 3]) -> [f32; 3] {
    parent.rotate(child.unrotate(vector))
}

pub(super) fn elements<'a>(holder: TagStruct<'a>, name: &str) -> Vec<TagStruct<'a>> {
    let block = holder.field(name).and_then(|field| field.as_block());
    (0..block.as_ref().map_or(0, |block| block.len()))
        .filter_map(|index| block.as_ref()?.element(index))
        .collect()
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
    pub(super) label: String,
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
fn loose_weapon_paths(root: &std::path::Path) -> Vec<String> {
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
pub(super) fn biped_weapons(
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
    let hidden_clips = hidden_clips(&graph, held.map_or(UNARMED, |weapon| weapon.label.as_str()));
    CeBipedWeapons { equippable, hidden_clips }
}

/// The clip to play after a weapon change: the same one if it is still shown,
/// or this weapon's idle in the same seat.
pub(super) fn clip_after_weapon_change(
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
pub(super) fn object_block<'a>(object: &'a TagFile, block: &str) -> Option<TagStruct<'a>> {
    let mut holder = object.root();
    loop {
        if holder.field(block).is_some() {
            return Some(holder);
        }
        holder = holder.fields_all().find_map(|field| field.as_struct())?;
    }
}

/// The unit's first weapon, which it has ready, by path.
pub(super) fn first_weapon(object: &TagFile) -> Option<String> {
    let weapons = object_block(object, "weapons")?.field("weapons")?.as_block()?;
    (0..weapons.len())
        .filter_map(|index| weapons.element(index)?.read_tag_ref_path("weapon"))
        .find(|path| !path.is_empty())
}

/// A marker of a held object, where holding it puts it.
pub(super) fn attached_marker_frame(
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

/// Add `other` to `preview`, moved onto `parent` and owned by object `owner`.
fn merge_attached(
    preview: &mut RenderModelPreview,
    mut other: RenderModelPreview,
    parent: MarkerFrame,
    child: MarkerFrame,
    owner: u8,
    name: &str,
) {
    let vertex_base = preview.vertices.len() as u32;
    let index_base = preview.indices.len() as u32;
    let material_base = preview.materials.len() as u16;
    let bound = [f32::from(parent.node), 0.0, 0.0, 0.0];
    for vertex in &mut other.vertices {
        vertex.position = attach_point(&parent, &child, vertex.position);
        vertex.normal = attach_vector(&parent, &child, vertex.normal);
        vertex.tangent = attach_vector(&parent, &child, vertex.tangent);
        vertex.binormal = attach_vector(&parent, &child, vertex.binormal);
        vertex.node_indices = bound;
        vertex.node_weights = [1.0, 0.0, 0.0, 0.0];
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(vertex.position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(vertex.position[axis]);
        }
    }
    preview.vertices.append(&mut other.vertices);
    preview.indices.extend(other.indices.iter().map(|index| index + vertex_base));
    for material in &mut other.materials {
        material.object = owner;
    }
    preview.materials.append(&mut other.materials);
    let label = |region: &str| format!("{name}: {region}");
    for mut batch in other.batches {
        batch.index_start += index_base;
        batch.material_index += material_base;
        batch.region_name = label(&batch.region_name);
        preview.batches.push(batch);
    }
    for mut region in other.regions {
        region.name = label(&region.name);
        preview.regions.push(region);
    }
}

/// An IK point of a weapon held on `parent`, if both markers and the arm exist.
fn weapon_ik(
    preview: &RenderModelPreview,
    weapon: &RenderModelPreview,
    class: &str,
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
        class: class.to_owned(),
        hand,
        elbow,
        shoulder,
        hand_from_marker: rigid_inverse(&(inverse_bind * matrix_of(on_hand.axes, on_hand.position))),
        weapon_node: parent.node.max(0) as usize,
        grip: matrix_of(moved, position),
    })
}

/// Hang the unit's ready weapon on it, returning the object that adds.
pub(super) fn attach_ce_weapon(
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
    let lights = super::ce_lights::read_attached_lights(source, &weapon, 1, &attached, parent, child);
    preview.ce_lights.extend(lights);
    merge_attached(preview, attached, parent, child, 1, &name);
    Some(CeAttachedObject {
        name,
        functions: read_ce_object_functions(&weapon),
        change_colors: read_h2_change_colors(&weapon),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let down = unit(apart(place_of(&elbow), place_of(&shoulder)));
        assert!(dot(axis_of(&shoulder, 0), down) > 0.999);
    }

    #[test]
    fn a_goal_out_of_reach_leaves_the_arm_just_short_of_straight() {
        let (mut shoulder, mut elbow, mut hand) = (at(0.0, 0.0, 0.0), at(1.0, 0.0, 0.0), at(1.0, 1.0, 0.0));
        reach_with_arm(at(5.0, 0.0, 0.0), &mut shoulder, &mut elbow, &mut hand);
        assert!((distance(&hand, &shoulder) - 2.0 * LONGEST_REACH).abs() < 1e-5);
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
            class: "rifle".to_owned(),
            hand: 2,
            elbow: 1,
            shoulder: 0,
            hand_from_marker: IDENTITY,
            weapon_node: 3,
            grip: IDENTITY,
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
            axes: MarkerFrame::ORIGIN.axes,
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
