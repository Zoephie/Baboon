//! Attaching a held weapon to its marker.

use super::super::super::tag_path;
use super::super::{RenderModelPreview, TagReader};
use super::graph::{descendants, string_id_in};
use super::unit::UnitWeapons;
use super::{
    AttachedObject, MarkerFrame, WeaponIk, attach_point, attach_vector, build_render_preview,
    fold_attached, holder_of, object_variant, weapon_ik,
};
use blam_tags::TagFile;

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
    let rules = reader.rules();
    let own_hand = rules
        .weapon_hand_marker
        .and_then(|marker| marker(unit, &weapon, left_hand));
    let hand = own_hand.unwrap_or_else(|| {
        string_id_in(
            unit,
            if left_hand {
                "left_hand_node"
            } else {
                "right_hand_node"
            },
        )
    });
    let preferred = string_id_in(unit, "preferred_gun_node");
    let has_preferred = attached
        .markers
        .iter()
        .any(|marker| marker.name == preferred);
    let grip = if has_preferred && !preferred.is_empty() {
        preferred
    } else {
        string_id_in(&weapon, "handle node")
    };
    let parent = MarkerFrame::find(preview, &hand);
    let child = MarkerFrame::find(&attached, &grip);
    if rules.ik_markers_on_weapons {
        let place = |frame: &MarkerFrame| {
            (
                frame.axes.map(|axis| attach_vector(&parent, &child, axis)),
                attach_point(&parent, &child, frame.position),
            )
        };
        let holder = super::super::animation::ik::reach::IkHolder::Weapon(owner);
        let node = Some(parent.node.max(0) as usize);
        preview
            .gen3
            .ik_markers
            .extend(super::super::animation::ik::reach::reach_ik_markers(
                &attached, holder, place, node,
            ));
    }
    if let Some(weapons) = ik {
        let class = string_id_in(&weapon, "weapon class");
        for mode in weapons.modes() {
            let mut points: Vec<WeaponIk> = Vec::new();
            for entry in weapons.ik_of(mode, &class) {
                let Some(mut point) = weapon_ik(
                    preview,
                    &attached,
                    mode,
                    parent,
                    child,
                    &entry.marker,
                    &entry.attach_to,
                ) else {
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
    let variant = object_variant(&weapon, &model, "")
        .map(|variant| variant.name)
        .unwrap_or_default();
    fold_attached(
        reader,
        &weapon,
        &model,
        &variant,
        attached,
        parent,
        child,
        owner,
        tag_path::name(path),
        preview,
    )
}
