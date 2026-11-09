//! Attaching a held weapon to its marker.

use super::*;

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
        let holder = super::super::animation::ik::reach::ReachIkHolder::Weapon(owner);
        let node = Some(parent.node.max(0) as usize);
        preview.gen3.ik_markers.extend(super::super::animation::ik::reach::reach_ik_markers(&attached, holder, place, node));
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
