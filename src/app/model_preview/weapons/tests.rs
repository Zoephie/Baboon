use super::super::{RenderModelPreview, RenderModelPreviewNode, load_referenced_tag_from_source};
use super::*;
use crate::app::model_preview::harness::kit::kit_source;
use crate::core::game::GameId;
use std::collections::BTreeSet;

/// Bad node data gives no descendants rather than a hang or a panic.
#[test]
fn descendants_survive_a_parent_cycle_and_a_parent_past_the_end() {
    let node = |parent: i16| RenderModelPreviewNode {
        parent,
        ..Default::default()
    };
    // 0 is the root; 1 and 2 are each other's parent; 3's is past the end.
    let preview = RenderModelPreview {
        nodes: vec![node(-1), node(2), node(1), node(9), node(0)],
        ..Default::default()
    };
    assert_eq!(descendants(&preview, 0), vec![4]);
}

fn entry(mode: &str, class: &str, kind: &str, has_idle: bool) -> GraphEntry {
    GraphEntry {
        mode: mode.to_owned(),
        class: class.to_owned(),
        kind: kind.to_owned(),
        has_idle,
        slots: Vec::new(),
    }
}

/// `entry`, with slots by label and the clip each names.
fn slotted(mode: &str, class: &str, kind: &str, slots: &[(&str, &str)]) -> GraphEntry {
    let slots = slots
        .iter()
        .map(|(label, clip)| ((*label).to_owned(), Some((*clip).to_owned())))
        .collect();
    GraphEntry {
        slots,
        ..entry(mode, class, kind, false)
    }
}

fn weapon(name: &str, class: &str, kind: &str, dual: bool, only_dual: bool) -> Equippable {
    Equippable {
        option: WeaponOption {
            path: name.to_owned(),
            name: name.to_owned(),
            class: class.to_owned(),
        },
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
            slotted(
                "combat",
                "pistol",
                "hp",
                &[("grip", "combat:pistol:hp:grip")],
            ),
            entry("combat", "pistol", "ne", false),
            entry("combat", "rifle", "any", true),
            GraphEntry {
                has_idle: true,
                ..slotted(
                    "combat",
                    "dual",
                    "any",
                    &[
                        ("arm_offsets", "combat:dual:arm_offsets"),
                        ("grip", "combat:dual:grip"),
                    ],
                )
            },
            slotted(
                "combat",
                "dual",
                "hp",
                &[("arm_offsets", "combat:dual:hp:arm_offsets")],
            ),
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
        overlays: ["reload_1", "fire_1", "aim_still_up"]
            .map(str::to_owned)
            .into(),
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
    let walked: Vec<_> = unit
        .ik_of("combat", "pistol")
        .iter()
        .map(|ik| ik.attach_to.as_str())
        .collect();
    assert_eq!(walked, ["left_hand_mc", "any", "other"]);
}

#[test]
fn weapon_ik_stays_on_for_what_the_game_leaves_it_on_for() {
    let unit = unit();
    for clip in [
        "combat:pistol:idle",
        "combat:pistol:hp:fire_1",
        "combat:pistol:aim_still_up",
    ] {
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
    assert!(
        !unit.ik_runs_through("crouch:pistol:idle", "combat"),
        "another mode's points"
    );
}

#[test]
fn one_weapon_grips_with_the_whole_body_only_while_ik_runs() {
    let unit = unit();
    let clips = ["combat:pistol:hp:grip", "combat:pistol:reload_1"];
    let layers = |clip| unit.pose_layers(&clips, clip, Some("magnum"), None);
    let grip = |arm| {
        vec![PoseLayer {
            clip: clips[0].to_owned(),
            arm,
        }]
    };
    assert_eq!(layers("combat:pistol:idle"), grip(None));
    assert_eq!(layers("combat:pistol:reload_1"), grip(Some(Arm::NotLeft)));
    assert!(
        unit.pose_layers(&clips, "combat:pistol:idle", None, None)
            .is_empty()
    );
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
    assert_eq!(
        find("flying").as_deref(),
        Some("any:pistol:hp:grip"),
        "the any mode last"
    );
    let mut elsewhere = unit.clone();
    elsewhere.graph[1].slots[0].1 = None;
    let find = |mode| elsewhere.find_clip(&clips, mode, "pistol", "hp", "grip");
    assert_eq!(
        find("combat"),
        None,
        "found in another graph, so not the any mode's"
    );
}

#[test]
fn two_weapons_each_lay_their_own_side_with_the_nearest_clip() {
    let unit = unit();
    let clips = [
        "combat:dual:arm_offsets",
        "combat:dual:hp:arm_offsets",
        "combat:dual:grip",
    ];
    let layers = unit.pose_layers(&clips, "combat:dual:idle", Some("magnum"), Some("shield"));
    let row = |clip: &str, arm| PoseLayer {
        clip: clip.to_owned(),
        arm: Some(arm),
    };
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
    assert!(!accepts(
        &[entry("combat", "pistol", "hp", false)],
        "pistol",
        "hp"
    ));
}

#[test]
fn the_left_hand_opens_only_for_a_primary_that_pairs() {
    let unit = unit();
    assert!(unit.secondaries(None).is_empty());
    assert!(unit.secondaries(Some("rocket")).is_empty());
    let names: Vec<_> = unit
        .secondaries(Some("magnum"))
        .into_iter()
        .map(|w| w.name)
        .collect();
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
    assert_eq!(
        unit.held(Some("magnum"), None),
        ("pistol".to_owned(), vec!["hp".to_owned()])
    );
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
    assert!(
        unit.hidden(&clips, "unarmed", &[]).is_empty(),
        "nothing held, nothing hidden"
    );
}

fn graph_of(entries: &[(&str, &str, &str, bool)]) -> UnitWeapons {
    let graph = entries
        .iter()
        .map(|(mode, class, kind, has_idle)| GraphEntry {
            mode: (*mode).to_owned(),
            class: (*class).to_owned(),
            kind: (*kind).to_owned(),
            has_idle: *has_idle,
            slots: Vec::new(),
        });
    UnitWeapons {
        graph: graph.collect(),
        ..Default::default()
    }
}

#[test]
fn a_clip_the_held_types_slot_names_stays_whatever_it_is_called() {
    let mut weapons = graph_of(&[
        ("combat", "rifle", "ar", true),
        ("combat", "rifle", "br", true),
        ("combat", "rifle", "sr", true),
    ]);
    weapons.graph[0].slots = vec![(
        "fire_1".to_owned(),
        Some("combat:rifle:br:fire_1".to_owned()),
    )];
    let clips = [
        "combat:rifle:ar:grip",
        "combat:rifle:br:fire_1",
        "combat:rifle:br:grip",
        "combat:rifle:sr:fire_1",
    ];
    let hidden = weapons.hidden(&clips, "rifle", &["ar".to_owned()]);
    assert_eq!(
        hidden.iter().copied().collect::<Vec<_>>(),
        [2, 3],
        "only the BR's own stay hidden"
    );
    let hidden = weapons.hidden(&clips, "rifle", &["sr".to_owned()]);
    assert!(
        hidden.contains(&1),
        "the AR's slot means nothing to a held sniper rifle"
    );
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
    let clips = [
        "combat:unarmed:airborne_arc",
        "combat:unarmed:idle",
        "combat:rifle:melee_airborne",
    ];
    assert!(unit.hidden(&clips, UNARMED, &[]).is_empty());
    assert_eq!(unit.hidden(&clips, "rifle", &[]), BTreeSet::from([0, 1]));
}

/// With `BLAM_TEST_HREK`: Reach keeps a type's actions and overlays in its
/// `any` set, which the Spartan's rifles and its grip are read from.
#[test]
fn a_reach_graph_reads_its_slots_from_the_any_set() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let spartan = "objects\\characters\\spartans\\spartans";
    let graph = load_referenced_tag_from_source(&source, spartan, "model_animation_graph", b"jmad")
        .unwrap();
    let entries = graph_entries(&graph);
    let ar = entries.iter().find(|entry| {
        (
            entry.mode.as_str(),
            entry.class.as_str(),
            entry.kind.as_str(),
        ) == ("combat", "rifle", "ar")
    });
    let ar = ar.expect("combat:rifle:ar");
    assert!(
        ar.slots
            .iter()
            .any(|(label, clip)| label == GRIP && clip.as_deref() == Some("combat:rifle:ar:grip")),
        "{:?}",
        ar.slots
    );
    let rifle = entries.iter().find(|entry| {
        (
            entry.mode.as_str(),
            entry.class.as_str(),
            entry.kind.as_str(),
        ) == ("combat", "rifle", ANY)
    });
    assert!(
        rifle.is_some_and(|entry| entry.has_idle),
        "the rifle class idles under any"
    );
    let unit = load_referenced_tag_from_source(&source, spartan, "biped", b"bipd").unwrap();
    let weapons = unit_weapons(
        &crate::app::model_preview::load::TagReader::new(&source),
        &unit,
        None,
    );
    let paths: Vec<_> = weapons
        .equippable
        .iter()
        .map(|weapon| weapon.option.name.as_str())
        .collect();
    assert!(
        paths.contains(&"assault_rifle") && paths.contains(&"sniper_rifle"),
        "{paths:?}"
    );
    assert_eq!(
        clips_without_ik(&graph).len(),
        2,
        "two clips turn weapon IK off by their user flags"
    );
}
