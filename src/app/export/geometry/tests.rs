use super::*;

fn add(tag: &mut TagFile, path: &str) {
    add_block_element(tag, path).unwrap_or_else(|error| panic!("add {path}: {error}"));
}

fn set(tag: &mut TagFile, path: &str, value: &str) {
    apply_field_edit(tag, path, value)
        .unwrap_or_else(|error| panic!("set {path} to {value:?}: {error}"));
}

/// CE's collision hierarchy is node -> BSP index, with the node selecting
/// a region and the BSP index selecting one of that region's permutations.
/// The old generic export instead wrote the node name into the material
/// label, which made Model Setup show `pelvis` / `bip01` and left the
/// node-local hull at the origin. Exercise the complete synthetic tag path.
#[test]
fn halo1_collision_uses_region_permutation_names_and_gbxmodel_pose() {
    let mut tag = TagFile::new(test_definition_path(
        "haloce_mcc/model_collision_geometry.json",
    ))
    .expect("load CE collision definition");

    add(&mut tag, "materials");
    set(&mut tag, "materials[0]/name", "cyborg armor");
    add(&mut tag, "regions");
    set(&mut tag, "regions[0]/name", "body");
    add(&mut tag, "regions[0]/permutations");
    set(&mut tag, "regions[0]/permutations[0]/name", "base");
    add(&mut tag, "nodes");
    set(&mut tag, "nodes[0]/name", "bip01 pelvis");
    set(&mut tag, "nodes[0]/region", "0");
    add(&mut tag, "nodes[0]/bsps");
    add(&mut tag, "nodes[0]/bsps[0]/surfaces");
    set(&mut tag, "nodes[0]/bsps[0]/surfaces[0]/first edge", "0");
    set(&mut tag, "nodes[0]/bsps[0]/surfaces[0]/material", "0");
    for edge in 0..3 {
        add(&mut tag, "nodes[0]/bsps[0]/edges");
        set(
            &mut tag,
            &format!("nodes[0]/bsps[0]/edges[{edge}]/start vertex"),
            &edge.to_string(),
        );
        set(
            &mut tag,
            &format!("nodes[0]/bsps[0]/edges[{edge}]/end vertex"),
            &((edge + 1) % 3).to_string(),
        );
        set(
            &mut tag,
            &format!("nodes[0]/bsps[0]/edges[{edge}]/forward edge"),
            &((edge + 1) % 3).to_string(),
        );
        set(
            &mut tag,
            &format!("nodes[0]/bsps[0]/edges[{edge}]/left surface"),
            "0",
        );
    }
    for (index, point) in ["0, 0, 0", "1, 0, 0", "0, 1, 0"].into_iter().enumerate() {
        add(&mut tag, "nodes[0]/bsps[0]/vertices");
        set(
            &mut tag,
            &format!("nodes[0]/bsps[0]/vertices[{index}]/point"),
            point,
        );
    }

    let skeleton = vec![blam_tags::JmsNode {
        name: "bip01 pelvis".to_owned(),
        parent: -1,
        rotation: blam_tags::math::RealQuaternion::IDENTITY,
        translation: blam_tags::math::RealPoint3d {
            x: 250.0,
            y: 0.0,
            z: 0.0,
        },
    }];
    let jms = halo1_collision_jms(&tag, Some(&skeleton)).expect("build CE collision JMS");

    assert_eq!(jms.regions, ["body"]);
    assert_eq!(jms.materials.len(), 1);
    assert_eq!(jms.materials[0].material_name, "base body");
    assert_eq!(jms.triangles.len(), 1);
    assert_eq!(jms.triangles[0].region, 0);
    assert_eq!(jms.vertices[0].position.x, 250.0);
    assert_eq!(jms.vertices[0].node_sets, [(0, 1.0)]);
    assert_eq!(jms.nodes[0].translation.x, 250.0);
}

#[test]
fn preview_skeleton_uses_canonical_render_model_pose() {
    let mut model = RenderModel::default();
    // This is the corrected rotation produced by the CE gbxmodel reader;
    // the raw tag stores the conjugate (-i, -j, -k, w).
    model.nodes.push(blam_tags::render_model::Node {
        name: "root".to_owned(),
        parent_node: -1,
        first_child_node: -1,
        next_sibling_node: -1,
        default_translation: blam_tags::math::RealPoint3d {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        default_rotation: blam_tags::math::RealQuaternion {
            i: 0.70710677,
            j: 0.0,
            k: 0.0,
            w: 0.70710677,
        },
        inverse_forward: Default::default(),
        inverse_left: Default::default(),
        inverse_up: Default::default(),
        inverse_position: Default::default(),
        inverse_scale: 1.0,
        distance_from_parent: 0.0,
    });

    let nodes = render_model_skeleton_nodes(&model);
    assert_eq!(nodes.len(), 1);
    assert!((nodes[0].rotation.i - 0.70710677).abs() < 1.0e-5);
    assert!((nodes[0].rotation.w - 0.70710677).abs() < 1.0e-5);
    assert_eq!(nodes[0].translation.x, 100.0);
    assert_eq!(nodes[0].translation.y, 200.0);
    assert_eq!(nodes[0].translation.z, 300.0);
}

#[test]
fn a_tags_own_reference_path_drops_only_the_group_extension() {
    let root = PathBuf::from("/kits/halo3/tags");
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: PathBuf::from("/definitions"),
    };
    let loose = |relative: &str| TagEntry {
        key: relative.to_owned(),
        display_path: relative.to_owned(),
        group_tag: u32::from_be_bytes(*b"coll"),
        group_name: Some("collision_model".to_owned()),
        location: TagEntryLocation::LooseFile(root.join(relative)),
    };

    assert_eq!(
        entry_reference_path(
            &source,
            &loose("objects/characters/flood_tank/flood_tank.collision_model")
        )
        .as_deref(),
        Some("objects\\characters\\flood_tank\\flood_tank")
    );
    // Authoring names may contain a literal dot. The group extension is a
    // known suffix, so it comes off without taking the dotted name with it.
    assert_eq!(
        entry_reference_path(&source, &loose("objects/gear/crate_1.5.collision_model"))
            .as_deref(),
        Some("objects\\gear\\crate_1.5")
    );
    // A monolithic cache stores the reference name itself — no derivation.
    let cached = TagEntry {
        key: "cache:coll:x".to_owned(),
        display_path: "objects/gear/crate_1.collision_model".to_owned(),
        group_tag: u32::from_be_bytes(*b"coll"),
        group_name: Some("collision_model".to_owned()),
        location: TagEntryLocation::Monolithic {
            name: "objects\\gear\\crate_1.5".to_owned(),
            group_tag: u32::from_be_bytes(*b"coll"),
        },
    };
    assert_eq!(
        entry_reference_path(&source, &cached).as_deref(),
        Some("objects\\gear\\crate_1.5")
    );
}

/// A `collision_model` / `physics_model` extracted on its own used to come
/// out collapsed — every bone at the origin and every hull and shape stacked
/// there with it — because nothing supplied the rest pose those tags do not
/// store. Guards that the owning `.model` is now found from the tag's own
/// path and its skeleton composed in.
///
/// Ignored by default — it needs a loose Halo 3 tag tree.
///
/// Run with:
///   H3_TAGS=~/Halo/halo3_mcc/tags \
///     cargo test standalone_collision -- --ignored --nocapture
#[test]
#[ignore = "requires a loose Halo 3 tag tree; set H3_TAGS"]
fn standalone_collision_and_physics_export_is_posed_by_the_owning_model() {
    let Ok(root) = std::env::var("H3_TAGS") else {
        eprintln!("skipping: set H3_TAGS to a loose Halo 3 tags directory");
        return;
    };
    let root = PathBuf::from(root);
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::Halo3),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // A rigged character: its collision hulls and physics shapes are stored
    // per-bone, so an unposed export piles all of them on the origin.
    let stem = "objects/characters/flood_tank/flood_tank";
    let cases = [("collision_model", *b"coll"), ("physics_model", *b"phmo")];
    for (extension, group_tag) in cases {
        let path = root.join(format!("{stem}.{extension}"));
        assert!(path.is_file(), "{} is not in this tag tree", path.display());
        let entry = TagEntry {
            key: file_entry_key(&path),
            display_path: format!("{stem}.{extension}"),
            group_tag: u32::from_be_bytes(group_tag),
            group_name: Some(extension.to_owned()),
            location: TagEntryLocation::LooseFile(path),
        };

        let skeleton = owning_model_skeleton(&source, &entry)
            .unwrap_or_else(|| panic!("{extension}: no owning model resolved"));
        let posed = skeleton
            .nodes()
            .iter()
            .filter(|node| {
                node.translation.x != 0.0
                    || node.translation.y != 0.0
                    || node.translation.z != 0.0
            })
            .count();
        assert!(
            posed > 1,
            "{extension}: the owning model's skeleton is itself unposed \
                 ({posed}/{} bones off the origin)",
            skeleton.nodes().len()
        );

        let tag = read_entry(&source, &entry).expect("read tag");
        let unposed = match extension {
            "collision_model" => collision_jms_for_game(&tag, None),
            _ => physics_jms_for_game(&tag, None),
        }
        .expect("build unposed jms");
        let jms = match extension {
            "collision_model" => collision_jms_for_game(&tag, Some(skeleton.nodes())),
            _ => physics_jms_for_game(&tag, Some(skeleton.nodes())),
        }
        .expect("build posed jms");

        // The armature the file emits, not just the skeleton handed in.
        let off_origin = jms
            .nodes
            .iter()
            .filter(|node| {
                node.translation.x != 0.0
                    || node.translation.y != 0.0
                    || node.translation.z != 0.0
            })
            .count();
        assert!(
            off_origin > 1,
            "{extension}: {off_origin}/{} emitted bones are off the origin — \
                 the skeleton was not overlaid",
            jms.nodes.len()
        );
        assert_eq!(
            unposed
                .nodes
                .iter()
                .filter(|node| node.translation.x != 0.0)
                .count(),
            0,
            "{extension}: the no-skeleton control is supposed to be collapsed; \
                 if it is not, this test proves nothing"
        );
        // The armature also has to be a hierarchy, not a flat pile of roots
        // — a collision node spells its parent link `parent node`.
        assert!(
            jms.nodes.iter().filter(|node| node.parent >= 0).count() > 1,
            "{extension}: the emitted armature has no bone hierarchy"
        );

        // Collision vertices are absolute, so composing the skeleton in has
        // to move them; physics shapes stay node-local and are placed by the
        // bone at import time, so only the armature changes there.
        if extension == "collision_model" {
            assert_ne!(
                unposed.vertices.len(),
                0,
                "collision_model exported no geometry"
            );
            let moved = jms
                .vertices
                .iter()
                .zip(unposed.vertices.iter())
                .filter(|(a, b)| a.position.x != b.position.x || a.position.z != b.position.z)
                .count();
            assert!(
                moved > jms.vertices.len() / 2,
                "only {moved}/{} collision vertices moved when the skeleton was applied",
                jms.vertices.len()
            );
        }

        // And the same through the entry point the browser's Extract
        // Geometry menu actually calls.
        let out = std::env::temp_dir().join("baboon_standalone_geometry_test");
        let _ = std::fs::remove_dir_all(&out);
        let message = extract_geometry_for_entry(&source, &entry, &out, Game::Halo3)
            .expect("extract geometry");
        println!("{message}");
        assert!(
            !message.contains("no owning model"),
            "{extension}: the export path did not resolve a skeleton — {message}"
        );
    }
}

/// A `model_animation_graph` that stores no `additional node data` has no
/// rest pose reachable from the graph alone, so extracting it on its own
/// wrote every bone at identity. Guards that the owning `.model` is now
/// found from the graph's own path and its render_model used instead.
///
/// Ignored by default — it needs a loose Halo Reach tag tree.
///
/// Run with:
///   REACH_TAGS=~/Halo/haloreach_mcc/tags \
///     cargo test animation_graph_without -- --ignored --nocapture
#[test]
#[ignore = "requires a loose Halo Reach tag tree; set REACH_TAGS"]
fn animation_graph_without_its_own_rest_pose_borrows_the_owning_models() {
    let Ok(root) = std::env::var("REACH_TAGS") else {
        eprintln!("skipping: set REACH_TAGS to a loose Halo Reach tags directory");
        return;
    };
    let root = PathBuf::from(root);
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: Some(GameId::HaloReach),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    };
    // The magnum's own graph: five gun bones, and not one `additional node
    // data` entry to place them with.
    let stem = "objects/weapons/pistol/magnum/magnum";
    let path = root.join(format!("{stem}.model_animation_graph"));
    assert!(path.is_file(), "{} is not in this tag tree", path.display());
    let entry = TagEntry {
        key: file_entry_key(&path),
        display_path: format!("{stem}.model_animation_graph"),
        group_tag: u32::from_be_bytes(*b"jmad"),
        group_name: Some("model_animation_graph".to_owned()),
        location: TagEntryLocation::LooseFile(path),
    };

    let jmad = read_entry(&source, &entry).expect("read the graph");
    let skeleton = blam_tags::Skeleton::from_tag(&jmad);
    assert!(
        !animation_rest_pose_is_complete(&jmad, &skeleton),
        "this graph carries its own rest pose, so it proves nothing here"
    );

    let owner = animation_graph_owner(&source, &entry, &jmad)
        .expect("the magnum's .model should be found and should name this graph");
    let resolved = blam_tags::extract::animation::resolve_animation_inputs(
        &owner,
        &SourceResolver { source: &source },
    )
    .expect("resolve through the owning model");
    let render = resolved
        .render_model
        .as_ref()
        .expect("the owning model names a render_model");

    let object_space = blam_tags::extract::animation::additional_node_data_is_object_space(
        &blam_tags::Animation::new(&jmad).expect("read animations"),
    );
    let without =
        blam_tags::extract::animation::build_defaults(&skeleton, &jmad, None, object_space);
    let with = blam_tags::extract::animation::build_defaults(
        &skeleton,
        &jmad,
        Some(render),
        object_space,
    );

    let posed = |set: &[blam_tags::NodeTransform]| {
        set.iter()
            .filter(|t| {
                t.translation.x != 0.0 || t.translation.y != 0.0 || t.translation.z != 0.0
            })
            .count()
    };
    assert_eq!(
        posed(&without),
        0,
        "the graph-only control is supposed to be collapsed to identity; \
             if it is not, this test proves nothing"
    );
    assert!(
        posed(&with) > 1,
        "only {}/{} bones got a rest pose from the owning model",
        posed(&with),
        with.len()
    );
}

/// End-to-end against a real Campaign Evolved install: mount the pak set,
/// find a structure BSP and the scenario that references it, and run both
/// export paths the browser's Extract menu now offers.
///
/// CE is a Blam/Unreal hybrid — the Blam tag owns collision, Unreal owns
/// rendered geometry — so a CE BSP legitimately exports collision only.
/// What this guards is that it exports *something substantial*: before the
/// collision fallback in blam-tags, `c10/level_a` produced a single
/// 198-vertex object out of 775 instanced definitions.
///
/// Ignored by default — it needs the shipped game.
///
/// Run with:
///   CE_ROOT="D:/SteamLibrary/steamapps/common/Halo Campaign Evolved" \
///     cargo test ce_structure_bsp -- --ignored --nocapture
#[test]
#[ignore = "requires a Campaign Evolved install; set CE_ROOT"]
fn ce_structure_bsp_and_scenario_export_geometry() {
    let Ok(root) = std::env::var("CE_ROOT") else {
        eprintln!("skipping: set CE_ROOT to the Campaign Evolved install root");
        return;
    };
    let root = PathBuf::from(root);
    let paks = crate::core::source::find_paks_dir(&root)
        .unwrap_or_else(|| panic!("no Paks dir under {}", root.display()));

    let definitions = crate::core::bundled::locate_definitions_root();
    let names = crate::core::format::TagNameIndex::load_game(&definitions, GameId::CampaignEvolved)
        .expect("load haloce_evolved tag-name index");
    let loaded = crate::core::source::load_iostore_container_set(paks, &names, &definitions)
        .expect("mount CE container set");

    let find = |suffix: &str, group: &[u8; 4]| {
        let want = u32::from_be_bytes(*group);
        loaded
            .all_entries
            .iter()
            .chain(loaded.entries.iter())
            .find(|e| {
                e.group_tag == want
                    && e.display_path
                        .replace('\\', "/")
                        .to_ascii_lowercase()
                        .contains(suffix)
            })
            .cloned()
    };

    let out = std::env::temp_dir().join("baboon_ce_geometry_test");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("create out dir");

    // Single BSP → one ASS.
    let bsp = find("c10/_generated_/level_a", b"sbsp")
        .or_else(|| find("level_a", b"sbsp"))
        .expect("no c10 level_a scenario_structure_bsp mounted");
    let msg = extract_geometry_for_entry(&loaded.source, &bsp, &out, Game::Halo3)
        .expect("export BSP");
    println!("{msg}");
    let ass = out.join(format!("{}.ASS", tag_file_stem(&bsp)));
    let len = std::fs::metadata(&ass).expect("BSP ASS written").len();
    assert!(
        len > 1_000_000,
        "{} is only {len} bytes — the collision fallback did not run (needs a \
             blam-tags with the collision-only BSP support)",
        ass.display()
    );

    // Scenario → one file per referenced BSP.
    let scnr = find("c10", b"scnr").expect("no c10 scenario mounted");
    let msg = extract_scenario_geometry(&loaded.source, &scnr, &out, Game::Halo3)
        .expect("export scenario");
    println!("{msg}");
    let structure = out.join(tag_file_stem(&scnr)).join("structure");
    let emitted: Vec<_> = std::fs::read_dir(&structure)
        .unwrap_or_else(|e| panic!("read {}: {e}", structure.display()))
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("ass"))
        })
        .collect();
    assert!(
        emitted.len() > 1,
        "scenario emitted {} file(s) into {} — expected one per structure_bsp",
        emitted.len(),
        structure.display()
    );
    for e in &emitted {
        let len = e.metadata().map(|m| m.len()).unwrap_or(0);
        println!("  {} — {len} bytes", e.path().display());
        assert!(len > 0, "{} is empty", e.path().display());
    }
}
