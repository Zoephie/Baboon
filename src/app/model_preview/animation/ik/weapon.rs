//! A held weapon's IK points: the arm bent so the hand reaches a marker of the
//! weapon, as `inverse_kinematics_adjust_matrices` does, and the clips the
//! game keeps them on through.

use blam_tags::math::Matrix4;

use super::super::super::common::marker::{MarkerFrame, attach_point, attach_vector};
use super::super::super::common::math::{
    axis_of, cross, dot, matrix_of, minus, place_of, plus, rigid_inverse, scaled,
    unit_by_reciprocal,
};
use super::super::super::*;

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

/// A hand held to a marker of the weapon: an IK point of the animations.
#[derive(Debug, Clone)]
pub(crate) struct CeWeaponIk {
    /// Which clips hold it: a Halo CE weapon class, the second word of their
    /// names, or a Halo 2 mode, the first.
    pub(in crate::app::model_preview) scope: String,
    pub(in crate::app::model_preview) hand: usize,
    pub(in crate::app::model_preview) elbow: usize,
    pub(in crate::app::model_preview) shoulder: usize,
    /// From the marker on the hand to its node.
    pub(in crate::app::model_preview) hand_from_marker: Matrix4,
    /// The node the weapon rides on.
    pub(in crate::app::model_preview) weapon_node: usize,
    /// The marker on the weapon, in bind space.
    pub(in crate::app::model_preview) grip: Matrix4,
    /// The nodes below the hand, which move with it.
    pub(in crate::app::model_preview) carried: Vec<usize>,
}

impl CeWeaponIk {
    pub(in crate::app::model_preview) fn hand(&self) -> usize {
        self.hand
    }

    /// Whether a clip is one of the held weapon's, played with IK on.
    pub(in crate::app::model_preview) fn runs_through(&self, clip: &str) -> bool {
        clip.split(' ')
            .any(|word| word.eq_ignore_ascii_case(&self.scope))
            && clip_runs_weapon_ik(clip)
    }

    /// Pose the arm. `worlds` holds each node's posed transform, and
    /// `inverse_binds` what `skin` undoes of it.
    pub(in crate::app::model_preview) fn apply(
        &self,
        worlds: &mut [Matrix4],
        inverse_binds: &[Matrix4],
    ) {
        let (Some(weapon), Some(inverse)) = (
            worlds.get(self.weapon_node).copied(),
            inverse_binds.get(self.weapon_node).copied(),
        ) else {
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
        let found = model
            .markers
            .iter()
            .any(|marker| marker.name.eq_ignore_ascii_case(name));
        found.then(|| MarkerFrame::find(model, name))
    };
    let on_hand = named(preview, marker)?;
    let on_weapon = named(weapon, attach_to)?;
    let hand = on_hand.node.max(0) as usize;
    let elbow = usize::try_from(preview.nodes.get(hand)?.parent).ok()?;
    let shoulder = usize::try_from(preview.nodes.get(elbow)?.parent).ok()?;
    let rows = preview.nodes.get(hand)?.inverse_bind;
    let inverse_bind = Matrix4 {
        m: [rows[0], rows[1], rows[2], [0.0, 0.0, 0.0, 1.0]],
    };
    let moved = on_weapon
        .axes
        .map(|axis| attach_vector(&parent, &child, axis));
    let position = attach_point(&parent, &child, on_weapon.position);
    Some(CeWeaponIk {
        scope: scope.to_owned(),
        hand,
        elbow,
        shoulder,
        hand_from_marker: rigid_inverse(
            &(inverse_bind * matrix_of(on_hand.axes, on_hand.position)),
        ),
        weapon_node: parent.node.max(0) as usize,
        grip: matrix_of(moved, position),
        carried: Vec::new(),
    })
}

/// `inverse_kinematics_adjust_matrices` keeps the arm this much short of straight.
pub(in crate::app::model_preview) const LONGEST_REACH: f32 = 0.98;

/// `inverse_kinematics_adjust_matrices`: bend at the elbow, on the side it
/// already bends, so the hand reaches `desired`.
pub(in crate::app::model_preview) fn reach_with_arm(
    desired: Matrix4,
    shoulder: &mut Matrix4,
    elbow: &mut Matrix4,
    hand: &mut Matrix4,
) {
    let start = place_of(shoulder);
    let joint = place_of(elbow);
    let upper = dot(minus(joint, start), minus(joint, start)).sqrt();
    let fore = dot(minus(place_of(hand), joint), minus(place_of(hand), joint)).sqrt();
    let to_goal = minus(place_of(&desired), start);
    let mut reach = dot(to_goal, to_goal).sqrt();
    if reach < 0.0001 {
        return;
    }
    let toward = scaled(to_goal, 1.0 / reach);
    let side = unit_by_reciprocal(cross(toward, minus(joint, start)));
    let bend = cross(side, toward);
    let mut goal = desired;
    let longest = (upper + fore) * LONGEST_REACH;
    // An arm with no length has nothing to bend.
    if longest < 0.0001 {
        return;
    }
    if longest < reach {
        reach = longest;
        let place = plus(start, scaled(toward, reach));
        (0..3).for_each(|row| goal.m[row][3] = place[row]);
    }
    let along = (reach * reach + upper * upper - fore * fore) / (reach * 2.0);
    let out = (upper * upper - along * along).max(0.0).sqrt();
    // A bone's first axis runs down it, and its second stays as near as it can.
    let aimed = |x: [f32; 3], up: [f32; 3], place: [f32; 3]| {
        let z = unit_by_reciprocal(cross(x, up));
        matrix_of([x, cross(z, x), z], place)
    };
    let upper_axis = unit_by_reciprocal(plus(scaled(toward, along), scaled(bend, out)));
    let fore_axis = unit_by_reciprocal(plus(scaled(toward, reach - along), scaled(bend, -out)));
    let bent = plus(start, scaled(upper_axis, upper));
    *elbow = aimed(fore_axis, axis_of(elbow, 1), bent);
    *shoulder = aimed(upper_axis, axis_of(shoulder, 1), start);
    *hand = goal;
}
