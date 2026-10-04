use super::*;

#[test]
fn a_pasted_windows_path_loses_its_quotes() {
    // Explorer's "Copy as path" is the most likely way a path reaches the
    // box, and it quotes unconditionally.
    assert_eq!(
        normalize_import_input("  \"D:\\SteamLibrary\\H3EK\\tags\"  "),
        "D:\\SteamLibrary\\H3EK\\tags"
    );
    assert_eq!(normalize_import_input("D:/H3EK/tags"), "D:/H3EK/tags");
}

#[test]
fn a_destination_is_tidied_into_a_relative_tag_path() {
    assert_eq!(
        normalize_import_rel("\\objects\\characters\\\\masterchief\\"),
        "objects/characters/masterchief"
    );
    assert_eq!(normalize_import_rel("   "), "");
    assert_eq!(normalize_import_rel("./objects/./foo"), "objects/foo");
}

/// Import offers the profiles that convert *into* the kit, which is not the
/// same list as the ones it converts *out to* — Campaign Evolved pairs only
/// with Reach, in both directions.
#[test]
fn import_sources_are_the_profiles_that_convert_into_this_kit() {
    let into_reach = import_sources_for("haloreach_mcc");
    assert!(into_reach.contains(&"halo3_mcc"));
    assert!(into_reach.contains(&CAMPAIGN_EVOLVED_GAME));
    assert!(!into_reach.contains(&"haloreach_mcc"));

    let into_evolved = import_sources_for(CAMPAIGN_EVOLVED_GAME);
    assert_eq!(into_evolved, vec![CAMPAIGN_EVOLVED_PARENT]);

    let into_halo3 = import_sources_for("halo3_mcc");
    assert!(into_halo3.contains(&"haloce_mcc"));
    assert!(into_halo3.contains(&"halo2_mcc"));
    assert!(
        !into_halo3.contains(&CAMPAIGN_EVOLVED_GAME),
        "Campaign Evolved converts only with Reach"
    );
}

/// The kit a file sits in settles which game it is, without opening it.
///
/// The deepest matching root wins, because a kit configured as its own
/// `tags` folder is a prefix of nothing else while a kit root is a prefix of
/// every kit nested under it.
#[test]
fn a_path_inside_a_configured_kit_names_its_game() {
    let roots = vec![
        ("halo3_mcc".to_owned(), PathBuf::from("D:/Steam/H3EK")),
        (
            "haloreach_mcc".to_owned(),
            PathBuf::from("D:/Steam/H3EK/nested/HREK"),
        ),
    ];
    let (game, why) = detect_import_game(
        Path::new("D:/Steam/H3EK/tags/objects/foo.weapon"),
        &roots,
        None,
    )
    .expect("a path inside a kit is identifiable");
    assert_eq!(game, "halo3_mcc");
    assert!(why.contains("halo3_mcc"), "{why}");

    let (nested, _) = detect_import_game(
        Path::new("D:/Steam/H3EK/nested/HREK/tags/objects/foo.weapon"),
        &roots,
        None,
    )
    .expect("the deeper kit wins");
    assert_eq!(nested, "haloreach_mcc");
}

/// A path outside every kit, with nothing to sample, is honestly unknown
/// rather than guessed at.
#[test]
fn a_path_outside_every_kit_is_not_guessed() {
    assert!(
        detect_import_game(
            Path::new("C:/somewhere/else/foo.weapon"),
            &[("halo3_mcc".to_owned(), PathBuf::from("D:/Steam/H3EK"))],
            None,
        )
        .is_none()
    );
}

/// A source lands under the folder it was dropped on, keeping its own name.
///
/// The file/folder asymmetry is the point: a folder's name is its whole
/// name, while a file's extension names the *source* group and would be
/// wrong on the imported tag.
#[test]
fn a_source_lands_under_the_folder_it_was_dropped_on() {
    assert_eq!(
        seeded_destination(
            "objects/characters",
            Path::new("D:/H3EK/tags/objects/characters/masterchief"),
            true
        ),
        "objects/characters/masterchief"
    );
    assert_eq!(
        seeded_destination(
            "objects/weapons",
            Path::new("D:/H3EK/tags/objects/weapons/rifle.weapon"),
            false
        ),
        "objects/weapons/rifle"
    );
    // Opened from the File menu, with no folder to land under.
    assert_eq!(
        seeded_destination("", Path::new("D:/H3EK/tags/foo.weapon"), false),
        "foo"
    );
}

/// The written file takes the *target* group's extension.
#[test]
fn a_single_import_is_named_for_the_target_group() {
    let root = Path::new("D:/HREK/tags");
    assert_eq!(
        single_output_path(root, "objects/weapons/rifle", "weapon"),
        Some(PathBuf::from("D:/HREK/tags/objects/weapons/rifle.weapon"))
    );
    // A shader going into Halo 4 becomes a material; the destination the
    // user typed carries no say in that.
    assert_eq!(
        single_output_path(root, "shaders/wall", "material"),
        Some(PathBuf::from("D:/HREK/tags/shaders/wall.material"))
    );
    // A pasted filename keeps its stem rather than gaining a second suffix.
    assert_eq!(
        single_output_path(root, "shaders/wall.shader", "material"),
        Some(PathBuf::from("D:/HREK/tags/shaders/wall.material"))
    );
    assert_eq!(single_output_path(root, "  ", "weapon"), None);
}

/// Resolving a folder counts what is actually in it, and says so honestly
/// when it cannot tell which game the tags are from.
///
/// The count is what the Import button is enabled on, so a folder of
/// documentation must not read as importable. Uses `TagFile::new` against
/// the shipped definitions rather than a kit, so it runs everywhere.
#[test]
fn resolving_a_folder_counts_tags_and_skips_everything_else() {
    let definitions = locate_definitions_root();
    let root = std::env::temp_dir().join(format!(
        "baboon_import_resolve_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let nested = root.join("weapons/nested");
    fs::create_dir_all(&nested).unwrap();
    let mut tag = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    apply_editing_kit_mcc_header(&mut tag, "halo3_mcc").unwrap();
    tag.write_atomic(root.join("weapons/rifle.weapon")).unwrap();
    tag.write_atomic(nested.join("pistol.weapon")).unwrap();
    fs::write(root.join("readme.txt"), b"not a tag").unwrap();

    let names = TagNameIndex::load_from_definitions(&definitions);
    let facts = resolve_import_source_job(&root.display().to_string(), &[], &names).unwrap();
    assert!(facts.is_folder);
    assert_eq!(facts.tag_files, 2, "recurses into subfolders");
    assert_eq!(facts.skipped_files, 1, "the text file is not a tag");
    assert_eq!(facts.group_tag, None, "a folder has no single group");

    // A single file resolves to exactly one tag, and its group is named so
    // the analyze step knows how to read it.
    let one = resolve_import_source_job(
        &root.join("weapons/rifle.weapon").display().to_string(),
        &[],
        &names,
    )
    .unwrap();
    assert!(!one.is_folder);
    assert_eq!(one.tag_files, 1);
    assert_eq!(one.group_tag, Some(u32::from_be_bytes(*b"weap")));

    // Nothing points at a kit, so the profile is a question, not an answer.
    // The tag's layout is genuinely ambiguous across MCC profiles, which is
    // exactly why the dialog keeps the combo box.
    if let Some((_, why)) = one.detected_game {
        assert!(why.contains("layout"), "{why}");
    }

    // A path that names nothing is an error rather than an empty folder.
    assert!(
        resolve_import_source_job(
            &root.join("no/such/place").display().to_string(),
            &[],
            &names
        )
        .is_err()
    );
    // A file that is not a tag is refused outright, rather than counted as
    // a zero-tag import that would silently write nothing.
    assert!(
        resolve_import_source_job(&root.join("readme.txt").display().to_string(), &[], &names)
            .is_err()
    );

    fs::remove_dir_all(root).unwrap();
}

/// A folder import lands under the destination, keeping the source's shape.
///
/// End to end through the plan the Import button builds and the same worker
/// it runs, because the split between "root" and "relative path" is exactly
/// the kind of wiring that reports success from the wrong directory. The
/// assertion is on files existing at named paths, not on counts.
#[test]
fn a_folder_import_recreates_the_source_shape_under_the_destination() {
    let definitions = locate_definitions_root();
    let root = std::env::temp_dir().join(format!(
        "baboon_folder_import_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // A Halo 3 kit laid out the way a real one is, and a Reach kit to import
    // into.
    let source_folder = root.join("H3EK/tags/objects/characters/jackal");
    let target_tags = root.join("HREK/tags");
    fs::create_dir_all(source_folder.join("weapons")).unwrap();
    fs::create_dir_all(&target_tags).unwrap();
    let mut tag = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    apply_editing_kit_mcc_header(&mut tag, "halo3_mcc").unwrap();
    tag.write_atomic(source_folder.join("weapons/plasma_pistol.weapon"))
        .unwrap();

    let plan = plan_folder_import(&source_folder, &target_tags, "objects/characters/jackal")
        .expect("a plain destination inside the kit");
    let (tx, _rx) = mpsc::channel();
    let report = run_folder_conversion_job(
        FolderConversionJob {
            source: TagSource::LooseFolder {
                root: plan.source_root,
                game: Some("halo3_mcc".to_owned()),
                definitions_root: definitions.clone(),
            },
            names: TagNameIndex::load_from_definitions(&definitions),
            scope: FolderConversionScope::LooseSubtree {
                source_rel_path: plan.source_rel_path,
                destination_label: plan.destination_label,
                destination_parent: plan.destination_parent,
            },
            source_game: "halo3_mcc".to_owned(),
            target_game: "haloreach_mcc".to_owned(),
            target_tags_root: target_tags.clone(),
            kit_roots: HashMap::new(),
            accept_loss: false,
            replace: ReplacePolicy::Always,
            only: None,
            cancel: Arc::new(AtomicBool::new(false)),
        },
        &tx,
    )
    .unwrap();

    assert_eq!(report.failed_count(), 0, "{:?}", report.files[0].detail);
    assert_eq!(report.converted_count(), 1);
    // The subfolder survives: the source's `weapons/` is still `weapons/`.
    assert!(
        target_tags
            .join("objects/characters/jackal/weapons/plasma_pistol.weapon")
            .is_file(),
        "wrote to {} instead",
        report.destination_root.display()
    );
    assert_eq!(
        report.destination_root,
        normalize_conversion_path(&target_tags.join("objects/characters/jackal"))
    );

    fs::remove_dir_all(root).unwrap();
}

/// A destination that swallows or sits inside the source is refused.
///
/// Both directions matter and neither is exotic: importing a kit's own
/// `tags` folder into that same kit is the obvious mistake, and it would
/// otherwise be a run that reads what it is writing.
#[test]
fn an_overlapping_import_is_refused_in_both_directions() {
    let source = Path::new("D:/H3EK/tags/objects");
    // Destination inside the source.
    assert!(plan_folder_import(source, Path::new("D:/H3EK/tags"), "objects/imported").is_err());
    // Source inside the destination.
    assert!(plan_folder_import(source, Path::new("D:/H3EK/tags"), "objects").is_err());
    // A different kit is fine.
    let plan = plan_folder_import(source, Path::new("D:/HREK/tags"), "objects/from_h3")
        .expect("another kit does not overlap");
    assert_eq!(plan.source_rel_path, PathBuf::from("objects"));
    assert_eq!(plan.destination_label, "from_h3");
    assert_eq!(
        plan.source_root,
        normalize_conversion_path(Path::new("D:/H3EK/tags"))
    );
    assert_eq!(
        plan.destination_parent,
        normalize_conversion_path(Path::new("D:/HREK/tags/objects"))
    );
    // An empty destination is refused rather than meaning "the tags root",
    // which would scatter a kit's whole tree over the destination's root.
    assert!(plan_folder_import(source, Path::new("D:/HREK/tags"), "  /  ").is_err());
}

/// A kit root is used as given, and a template in it is actually found.
///
/// The regression this pins failed *silently*: `import_tags_root` appends
/// `tags` to anything not already called that, so resolving a root twice
/// produced a path that does not exist, the template search found nothing,
/// and the conversion carried on with a generated layout. Nothing errored —
/// the output was just quietly less native than it should have been. The
/// assertion is therefore that a template was *found*, not that the call
/// returned `Ok`.
#[test]
fn a_kit_root_is_taken_as_given_and_its_template_is_found() {
    let definitions = locate_definitions_root();
    let root = std::env::temp_dir().join(format!(
        "baboon_routed_roots_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Deliberately *not* named `tags`, which is what made the double resolve
    // visible; a folder called `tags` would have hidden it.
    let target_root = root.join("reach_content");
    fs::create_dir_all(&target_root).unwrap();
    let mut native = TagFile::new(definitions.join("haloreach_mcc/particle.json")).unwrap();
    apply_editing_kit_mcc_header(&mut native, "haloreach_mcc").unwrap();
    // A recorded source revision is what marks a tag as kit-authored and so
    // eligible to be a template.
    native.header.version = 8;
    native
        .write_atomic(target_root.join("stock.particle"))
        .unwrap();

    let source = TagFile::new(definitions.join("halo3_mcc/particle.json")).unwrap();
    let kit_roots = HashMap::from([("haloreach_mcc".to_owned(), target_root.clone())]);
    let mut cache = NativeTemplateCache::default();
    let draft = convert_tag_routed(
        &source,
        "halo3_mcc",
        "haloreach_mcc",
        &definitions,
        &kit_roots,
        &mut cache,
        LossPolicy::FailClosed,
    )
    .expect("Halo 3 to Reach converts directly");
    assert_eq!(
        draft.native_layout_template.as_deref(),
        Some(target_root.join("stock.particle").as_path()),
        "the template in the configured root should have been used"
    );
    assert!(
        draft.route.is_empty(),
        "a pair that converts directly must not be routed"
    );
    // Built once and kept, so the next tag in a folder run pays nothing.
    assert!(cache.templates_for("haloreach_mcc").is_some());

    fs::remove_dir_all(root).unwrap();
}

/// A pair the converter refuses is routed through the engines between them.
///
/// Baboon's own half of the fallback, not the engine's: the early return for
/// pairs with nothing in between is what would silently turn routing back
/// off, and it returns the direct refusal, which looks exactly like routing
/// having been tried and failed.
#[test]
fn a_refused_pair_falls_back_to_a_route() {
    let definitions = locate_definitions_root();
    // Reach has no `pixels offset` field, so a Halo 2 bitmap's pixels have
    // nothing indexing them there and the catalog refuses the pair by name.
    let bitmap = TagFile::new(definitions.join("halo2_mcc/bitmap.json")).unwrap();
    let mut cache = NativeTemplateCache::default();
    let draft = convert_tag_routed(
        &bitmap,
        "halo2_mcc",
        "haloreach_mcc",
        &definitions,
        &HashMap::new(),
        &mut cache,
        LossPolicy::FailClosed,
    )
    .expect("the refusal should be routed around, not surfaced");
    assert_eq!(draft.route, vec!["halo2_mcc", "halo3_mcc", "haloreach_mcc"]);

    // Adjacent profiles have nothing between them, so a refusal there is
    // final and must come back as itself rather than as a routing report.
    let weapon = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    let direct = convert_tag_routed(
        &weapon,
        "halo3_mcc",
        "halo3odst_mcc",
        &definitions,
        &HashMap::new(),
        &mut cache,
        LossPolicy::FailClosed,
    )
    .expect("adjacent MCC profiles convert");
    assert!(direct.route.is_empty());
}

/// A tag that loses audited data is held back, not failed, and comes back
/// carrying what it would give up.
///
/// The three-way split is the whole point of this change. Before it, a Halo 3
/// light that loses `percent spherical` going to Reach was indistinguishable
/// from a tag that could not be converted at all — both were an error string
/// — so there was nothing to offer the user a choice about.
///
/// Self-skips without the kits: a schema-built light has no authored values
/// to lose, so only a real one reaches this path.
#[test]
fn a_lossy_tag_is_held_back_with_its_losses_rather_than_failed() {
    let h3 = PathBuf::from("D:/SteamLibrary/steamapps/common/H3EK/tags");
    let reach = PathBuf::from("D:/SteamLibrary/steamapps/common/HREK/tags");
    if !h3.is_dir() || !reach.is_dir() {
        eprintln!("skipping: needs H3EK and HREK");
        return;
    }
    let definitions = locate_definitions_root();
    let kit_roots = HashMap::from([("haloreach_mcc".to_owned(), reach)]);
    let mut cache = NativeTemplateCache::default();
    let group_tag = u32::from_be_bytes(*b"ligh");

    // Measured: 69 of H3EK's first 400 lights are refused, the first at
    // index 95. Scanning fewer finds none and skips, proving nothing.
    let mut lights: Vec<PathBuf> = blam_tags::convert::walk_files(&h3)
        .into_iter()
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("light"))
        .filter(|path| {
            !path
                .components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("baboon_converted"))
        })
        .collect();
    lights.sort();

    let mut held = 0usize;
    let mut clean = 0usize;
    let mut example: Option<Vec<String>> = None;
    for path in lights.into_iter().take(200) {
        let Ok(tag) = crate::core::source::read_tag_at_path(
            &path,
            Some("halo3_mcc"),
            Some(&definitions),
            group_tag,
        ) else {
            continue;
        };
        match convert_tag_outcome(
            &tag,
            "halo3_mcc",
            "haloreach_mcc",
            &definitions,
            &kit_roots,
            &mut cache,
        ) {
            ConversionOutcome::Clean(_) => clean += 1,
            ConversionOutcome::Lossy { draft, refusal } => {
                held += 1;
                // Held back means the tag is *there* — converted, writable,
                // waiting on an answer — and that the answer can be an
                // informed one.
                assert!(
                    !draft.report.fail_closed_losses.is_empty(),
                    "{}: held back without saying what it loses",
                    path.display()
                );
                assert!(
                    refusal.contains("was not written"),
                    "{}: kept the wrong refusal: {refusal}",
                    path.display()
                );
                assert!(draft.tag.write_to_bytes().is_ok(), "{}", path.display());
                if example.is_none() {
                    example = Some(draft.report.fail_closed_losses.clone());
                }
            }
            ConversionOutcome::Failed(error) => {
                panic!(
                    "{}: lights should not fail outright: {error}",
                    path.display()
                )
            }
        }
    }
    assert!(clean > 0, "no H3EK light converted cleanly");
    assert!(
        held > 0,
        "no H3EK light was held back; this test proves nothing on this kit"
    );
    eprintln!("{clean} clean, {held} held back; first gives up {example:?}");
}

/// A clean conversion is never routed through the held-back path.
#[test]
fn a_clean_conversion_is_not_held_back() {
    let definitions = locate_definitions_root();
    let source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    let mut cache = NativeTemplateCache::default();
    let outcome = convert_tag_outcome(
        &source,
        "halo3_mcc",
        "haloreach_mcc",
        &definitions,
        &HashMap::new(),
        &mut cache,
    );
    assert!(matches!(outcome, ConversionOutcome::Clean(_)));
    assert!(outcome.losses().is_empty());
}

/// A configured kit whose game is not a conversion profile (Campaign Evolved
/// is configured as a containers folder, not a loose kit) must not be
/// reported as the source of a loose tag that merely lives beneath it.
#[test]
fn a_kit_root_that_is_not_a_conversion_profile_is_ignored() {
    let roots = vec![("some_unshipped_game".to_owned(), PathBuf::from("D:/Kits/X"))];
    assert!(
        detect_import_game(Path::new("D:/Kits/X/tags/foo.weapon"), &roots, None,).is_none()
    );
}
