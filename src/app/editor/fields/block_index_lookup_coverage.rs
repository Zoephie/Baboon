use super::*;
use crate::core::document::apply::{apply_field_edit, apply_one_block_op_with_targets};
use crate::core::document::ops::{BlockOp, BlockOpKind};
use blam_tags::TagFile;

const GAMES: [&str; 7] = [
    "halo2_mcc",
    "halo3_mcc",
    "halo3odst_mcc",
    "haloreach_mcc",
    "halo4_mcc",
    "halo2amp_mcc",
    "haloce_evolved",
];

fn op(
    tag: &mut TagFile,
    path: &str,
    kind: BlockOpKind,
    targets: &blam_tags::index_targets::IndexTargets,
) {
    apply_one_block_op_with_targets(
        tag,
        &BlockOp {
            path: path.into(),
            kind,
        },
        targets,
    )
    .unwrap();
}

fn ensure_holder(tag: &mut TagFile, path: &str, targets: &blam_tags::index_targets::IndexTargets) {
    let mut prefix = String::new();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if let Some((block, index)) = segment.split_once('[') {
            let index: usize = index.trim_end_matches(']').parse().unwrap();
            let block_path = if prefix.is_empty() {
                block.to_owned()
            } else {
                format!("{prefix}/{block}")
            };
            while tag
                .root()
                .field_path(&block_path)
                .unwrap_or_else(|| panic!("missing {block_path}"))
                .as_block()
                .unwrap()
                .len()
                <= index
            {
                op(tag, &block_path, BlockOpKind::Add, &targets);
            }
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
    }
}

fn seed(
    tag: &mut TagFile,
    path: &str,
    names: [&str; 2],
    targets: &blam_tags::index_targets::IndexTargets,
) {
    if let Some((parent, _)) = path.rsplit_once('/') {
        ensure_holder(tag, parent, &targets);
    }
    for name in names {
        let index = tag
            .root()
            .field_path(path)
            .unwrap()
            .as_block()
            .unwrap()
            .len();
        op(tag, path, BlockOpKind::Add, &targets);
        let label = if path.ends_with("Actors") { "Actor name" } else { "name" };
        apply_field_edit(tag, &format!("{path}[{index}]/{label}"), name).unwrap();
    }
}

fn value(tag: &TagFile, path: &str) -> i16 {
    match tag.root().field_path(path).unwrap().value().unwrap() {
        TagFieldData::CustomShortBlockIndex(index) => index,
        other => panic!("unexpected {other:?}"),
    }
}

fn verify(
    game: &str,
    group: &str,
    holder_path: &str,
    field_name: &str,
    selector: Option<&str>,
    target_path: &str,
    alternate: Option<&str>,
) {
    let targets = blam_tags::index_targets::IndexTargets::load(
        crate::core::bundled::locate_definitions_root().join(game),
    )
    .unwrap();
    let mut tag = TagFile::new(crate::app::test_definition_path(&format!(
        "{game}/{group}.json"
    )))
    .unwrap();
    ensure_holder(&mut tag, holder_path, &targets);
    if let Some(alternate) = alternate {
        seed(
            &mut tag,
            alternate,
            ["other_alpha", "other_bravo"],
            &targets,
        );
    }
    seed(&mut tag, target_path, ["alpha", "bravo"], &targets);
    if let Some(selector) = selector {
        apply_field_edit(&mut tag, selector, "1").unwrap();
    }
    let path = format!("{holder_path}/{field_name}");
    apply_field_edit(&mut tag, &path, "1").unwrap();
    let bytes = tag.write_to_bytes().unwrap();
    for mut tag in [tag, TagFile::read_from_bytes(&bytes).unwrap()] {
        let resolve = |tag: &TagFile| {
            let root = tag.root();
            let holder = root.descend(holder_path).unwrap();
            let field = holder.field_path(field_name).unwrap();
            block_index_target_options(&holder, &field, Some(root), holder_path, &targets)
        };
        let target = resolve(&tag).unwrap_or_else(|| panic!("{game} {path}: missing named picker"));
        assert_eq!(target.len, 2);
        let labels = block_index_target_labels(Some(tag.root()), &target, &TagNameIndex::default());
        assert!(
            labels[0].contains("alpha")
                && labels[1].contains("bravo")
                && !labels[0].contains("other"),
            "{game} {path}: {labels:?}"
        );
        if let Some(selector) = selector {
            for invalid in ["-1", "999"] {
                apply_field_edit(&mut tag, selector, invalid).unwrap();
                assert!(
                    resolve(&tag).is_none(),
                    "{game} {path}: invalid parent must not guess a target"
                );
            }
            apply_field_edit(&mut tag, selector, "1").unwrap();
        }
        if let Some(alternate) = alternate {
            op(&mut tag, alternate, BlockOpKind::Insert(0), &targets);
            assert_eq!(
                value(&tag, &path),
                1,
                "{game} {path}: unrelated parent must not remap this index"
            );
        }
        op(&mut tag, target_path, BlockOpKind::Insert(0), &targets);
        assert_eq!(value(&tag, &path), 2, "{game} {path}");
        op(
            &mut tag,
            target_path,
            BlockOpKind::Reorder {
                order: vec![2, 0, 1],
            },
            &targets,
        );
        assert_eq!(value(&tag, &path), 0, "{game} {path}");
        op(&mut tag, target_path, BlockOpKind::Delete(0), &targets);
        assert_eq!(value(&tag, &path), -1, "{game} {path}");
    }
}

#[test]
fn custom_indices_link_squad_tasks_to_the_selected_objective() {
    for game in GAMES.into_iter().filter(|g| *g != "halo2_mcc") {
        verify(
            game,
            "scenario",
            "squads[0]",
            "initial task",
            Some("squads[0]/initial objective"),
            "ai objectives[1]/tasks",
            Some("ai objectives[0]/tasks"),
        );
    }
}

#[test]
fn custom_indices_link_order_areas_to_the_selected_zone() {
    for game in GAMES {
        let holders = if game == "haloce_evolved" {
            vec!["ai objectives[0]/tasks[0]/areas[0]".to_owned()]
        } else {
            vec![
                "Orders[0]/Primary area set[0]".to_owned(),
                "Orders[0]/Secondary area set[0]".to_owned(),
            ]
        };
        for holder in holders {
            verify(
                game,
                "scenario",
                &holder,
                "area",
                Some(&format!("{holder}/zone")),
                "zones[1]/areas",
                Some("zones[0]/areas"),
            );
        }
    }
}

#[test]
fn custom_indices_link_patrol_point_sets_and_selected_points() {
    for game in GAMES.into_iter().filter(|g| *g != "halo2_mcc") {
        let parent = if game == "halo3_mcc" {
            "squads[0]/fire-teams[0]"
        } else {
            "squads[0]/spawn points[0]"
        };
        verify(
            game,
            "scenario",
            parent,
            "point set",
            None,
            "scripting data[0]/point sets",
            None,
        );
        verify(
            game,
            "scenario",
            &format!("{parent}/points[0]"),
            "point",
            Some(&format!("{parent}/point set")),
            "scripting data[0]/point sets[1]/points",
            Some("scripting data[0]/point sets[0]/points"),
        );
    }
}

#[test]
fn custom_indices_link_physics_permutations_to_the_selected_region() {
    for game in GAMES {
        verify(
            game,
            "physics_model",
            "rigid bodies[0]",
            "permutattion",
            Some("rigid bodies[0]/region"),
            "regions[1]/permutations",
            Some("regions[0]/permutations"),
        );
    }
}

#[test]
fn installed_h3_custom_indices_resolve_named_entries() {
    fn visit(
        holder: TagStruct<'_>,
        root: TagStruct<'_>,
        prefix: &str,
        counts: &mut std::collections::BTreeMap<String, usize>,
        targets: &blam_tags::index_targets::IndexTargets,
    ) {
        for field in holder.fields_all() {
            let path = format!("{prefix}/{}#{}", field.clean_name(), field.ordinal());
            if let Some(TagFieldData::CustomShortBlockIndex(index)) = field.value()
                && index >= 0
                && let Some(target) =
                    block_index_target_options(&holder, &field, Some(root), prefix, &targets)
                && (index as usize) < target.len
            {
                let block = root.field_path(&target.path).unwrap().as_block().unwrap();
                let entry = block.element(index as usize).unwrap();
                let name = entry
                    .field_path("name")
                    .and_then(|f| f.value())
                    .and_then(|value| match value {
                        TagFieldData::String(s) | TagFieldData::LongString(s) => Some(s),
                        TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => {
                            Some(id.string)
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                if !name.is_empty() {
                    let labels =
                        block_index_target_labels(Some(root), &target, &TagNameIndex::default());
                    assert!(labels[index as usize].contains(&name), "{path}: {name}");
                    *counts.entry(field.clean_name().into_owned()).or_default() += 1;
                }
            }
            if let Some(child) = field.as_struct() {
                visit(child, root, &path, counts, &targets);
            }
            if let Some(block) = field.as_block() {
                for (index, child) in block.iter().enumerate() {
                    visit(child, root, &format!("{path}[{index}]"), counts, &targets);
                }
            }
        }
    }
    let definitions = crate::core::bundled::locate_definitions_root();
    let targets =
        blam_tags::index_targets::IndexTargets::load(definitions.join("halo3_mcc")).unwrap();
    for (relative, group, blocks, required) in [
        (
            "levels/solo/030_outskirts/030_outskirts.scenario",
            "scnr",
            vec!["squads", "ai objectives", "Orders"],
            vec!["initial task", "area"],
        ),
        (
            "objects/characters/brute/brute.physics_model",
            "phmo",
            vec!["rigid bodies"],
            vec!["permutattion"],
        ),
    ] {
        let path =
            std::path::PathBuf::from(crate::core::test_kits::tag_path("halo3_mcc", relative));
        if !path.exists() {
            eprintln!("skip: {} unavailable", path.display());
            continue;
        }
        let tag = crate::core::source::read_tag_at_path(
            &path,
            Some(GameId::Halo3),
            Some(&definitions),
            u32::from_be_bytes(group.as_bytes().try_into().unwrap()),
        )
        .unwrap();
        let root = tag.root();
        let mut counts = std::collections::BTreeMap::new();
        for name in blocks {
            let field = root.field_path(name).unwrap();
            let path = format!("{}#{}", field.clean_name(), field.ordinal());
            for (index, holder) in field.as_block().unwrap().iter().enumerate() {
                visit(
                    holder,
                    root,
                    &format!("{path}[{index}]"),
                    &mut counts,
                    &targets,
                );
            }
        }
        for required in required {
            assert!(
                counts.get(required).copied().unwrap_or(0) > 0,
                "{relative}: expected populated {required} references; {counts:?}"
            );
        }
        eprintln!("{relative}: named custom index references {counts:?}");
    }
}

const THESPIAN_GAMES: [&str; 4] = ["haloreach_mcc", "halo4_mcc", "halo2amp_mcc", "haloce_evolved"];

#[test]
fn thespian_points_follow_the_performance_point_set() {
    for game in THESPIAN_GAMES {
        for (holder, field) in [
            ("thespian[0]/Actors[0]", "debug spawn point"),
            ("thespian[0]/Lines[0]/Point interaction[0]", "point"),
            ("thespian[0]/Lines[0]/Sync Actions[0]", "attach to point"),
            ("thespian[0]/Lines[0]/Sounds[0]", "attach to point"),
        ] {
            verify(
                game,
                "scenario",
                holder,
                field,
                Some("thespian[0]/Point set"),
                "scripting data[0]/point sets[1]/points",
                Some("scripting data[0]/point sets[0]/points"),
            );
        }
    }
}

#[test]
fn thespian_actors_stay_within_their_performance() {
    for game in THESPIAN_GAMES {
        for (holder, field) in [
            ("thespian[0]/Lines[0]", "Actor"),
            ("thespian[0]/Lines[0]/Point interaction[0]", "actor"),
        ] {
            verify(
                game,
                "scenario",
                holder,
                field,
                None,
                "thespian[0]/Actors",
                Some("thespian[1]/Actors"),
            );
        }
    }
}

#[test]
fn thespian_point_sets_use_the_script_data() {
    for game in THESPIAN_GAMES {
        verify(
            game,
            "scenario",
            "thespian[0]",
            "Point set",
            None,
            "scripting data[0]/point sets",
            None,
        );
    }
}

#[test]
fn thespian_and_cue_tasks_follow_their_objective() {
    for game in THESPIAN_GAMES {
        for (holder, selector) in [
            ("thespian[0]/tasks[0]", "thespian[0]/tasks[0]/objective"),
            (
                "actual cues[0]/distribution/tasks[0]",
                "actual cues[0]/distribution/tasks[0]/objective",
            ),
        ] {
            verify(
                game,
                "scenario",
                holder,
                "task",
                Some(selector),
                "ai objectives[1]/tasks",
                Some("ai objectives[0]/tasks"),
            );
        }
    }
}

#[test]
fn sync_action_actor_types_use_the_character_palette() {
    let path = "thespian[0]/Lines[0]/Sync Actions[0]/Actors[0]/Actor type";
    for game in THESPIAN_GAMES {
        let targets = blam_tags::index_targets::IndexTargets::load(
            crate::core::bundled::locate_definitions_root().join(game),
        )
        .unwrap();
        let mut tag = TagFile::new(crate::app::test_definition_path(&format!(
            "{game}/scenario.json"
        )))
        .unwrap();
        ensure_holder(&mut tag, path.rsplit_once('/').unwrap().0, &targets);
        for _ in 0..2 {
            op(&mut tag, "character palette", BlockOpKind::Add, &targets);
        }
        apply_field_edit(&mut tag, path, "1").unwrap();
        op(&mut tag, "character palette", BlockOpKind::Insert(0), &targets);
        assert_eq!(value(&tag, path), 2, "{game}");
    }
}
