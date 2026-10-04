use super::*;
use crate::core::source::TagSource;
use blam_tags::TagOptions;
// Both the editor and the engine define this, identically, and two globs make
// the bare name ambiguous. Name the engine's explicitly: this scaffolding is a
// copy of the engine's own, so it should key fields the way the engine does.
use blam_tags::convert::clean_field_key;

/// A cache tag lands at its own path, unless the user picked one.
///
/// Its own path is what makes a folder import work at all: a reference
/// carries the path string the build gave it and nothing rewrites those, so
/// a tag written where the cache said it lived is a tag every other tag
/// already points at. Somewhere else is a real answer to a real question --
/// trying a build's version of a level beside the kit's own -- and the
/// window says what it costs rather than refusing.
///
/// The two relocations differ, and the difference is the point: a whole
/// folder keeps its shape under the folder that was picked, while a single
/// tag keeps only its name. Both fall out of the same rule, which is that
/// what gets stripped is the part the user already named.
#[test]
fn a_relocated_cache_tag_keeps_its_name_and_nothing_else_of_its_path() {
    let entry = TagEntry {
        key: r"cache:bitm:objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
        display_path: r"objects\weapons\rifle\bitmaps\ar_diffuse.bitmap".to_owned(),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::Monolithic {
            name: r"objects\weapons\rifle\bitmaps\ar_diffuse".to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
        },
    };
    let landed = Ok((u32::from_be_bytes(*b"bitm"), "bitmap".to_owned()));
    let destination_root = PathBuf::from("D:/HREK/tags");
    let scope = |destination: CacheDestination| FolderConversionScope::CacheSubtree {
        prefix: String::new(),
        entries: Vec::new(),
        seed: CacheSeed::Folder,
        destination,
    };

    let place = |destination: CacheDestination| {
        target_destination_for_entry(
            &entry,
            &scope(destination),
            Path::new(""),
            &destination_root,
            &landed,
        )
        .map(|path| normalize_conversion_path(&path))
    };
    let expect = |relative: &str| normalize_conversion_path(&destination_root.join(relative));

    assert_eq!(
        place(CacheDestination::OwnPath).expect("its own path"),
        expect("objects/weapons/rifle/bitmaps/ar_diffuse.bitmap"),
    );

    // One tag, so what is stripped is the folder it sits in and what is
    // left is the name.
    assert_eq!(
        place(CacheDestination::Folder {
            root: PathBuf::from("scratch/imported"),
            strip: r"objects\weapons\rifle\bitmaps".to_owned(),
        })
        .expect("the folder that was picked"),
        expect("scratch/imported/ar_diffuse.bitmap"),
    );

    // A folder, so what is stripped is the folder that was asked for and
    // the shape below it survives. Importing `objects\weapons` into
    // `scratch` has to keep the rifle and its bitmaps apart from every
    // other weapon's, or a folder of two thousand tags lands as two
    // thousand files in one directory.
    assert_eq!(
        place(CacheDestination::Folder {
            root: PathBuf::from("scratch"),
            strip: r"objects\weapons".to_owned(),
        })
        .expect("the folder that was picked"),
        expect("scratch/rifle/bitmaps/ar_diffuse.bitmap"),
    );

    // A tag that does not sit under the folder that was named still has to
    // land inside it: a second pass brings references from anywhere in the
    // build, and a path that escaped the destination would be refused
    // outright rather than written somewhere surprising.
    assert_eq!(
        place(CacheDestination::Folder {
            root: PathBuf::from("scratch"),
            strip: r"levels\multi".to_owned(),
        })
        .expect("inside the folder that was picked"),
        expect("scratch/ar_diffuse.bitmap"),
    );
}

/// A run told not to replace a tag leaves the one the kit has alone.
///
/// The importer only ever overwrote, which is right for a kit being filled
/// from a build and wrong for one that has been worked in: a kit's own
/// level is somebody's edits, and an import that eats it cannot be undone
/// from inside Baboon. Checked by the version stamp rather than by the
/// file's date, because a keep and a replace-with-identical-bytes look the
/// same to a timestamp on a fast disk.
#[test]
fn a_run_told_to_keep_a_tag_does_not_write_over_it() {
    let definitions = locate_definitions_root();
    let unique = format!(
        "baboon_replace_policy_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let source_root = root.join("source_tags");
    let source_folder = source_root.join("characters/jackal");
    let target_tags = root.join("target_tags");
    let destination_parent = target_tags.join("objects/characters");
    fs::create_dir_all(&source_folder).unwrap();
    fs::create_dir_all(&target_tags).unwrap();

    let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    seed_weapon_fields(&mut source);
    apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
    source
        .write_atomic(source_folder.join("jackal.weapon"))
        .unwrap();

    // The kit's own copy, marked so a replacement is recognisable.
    let output = destination_parent.join("jackal/jackal.weapon");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    let mut existing = TagFile::new(definitions.join("haloreach_mcc/weapon.json")).unwrap();
    apply_editing_kit_mcc_header(&mut existing, "haloreach_mcc").unwrap();
    existing.header.version = 4321;
    existing.write_atomic(&output).unwrap();

    let run = |replace: ReplacePolicy| {
        let (tx, _rx) = mpsc::channel();
        run_folder_conversion_job(
            FolderConversionJob {
                source: TagSource::LooseFolder {
                    root: source_root.clone(),
                    game: Some(GameId::Halo3),
                    definitions_root: definitions.clone(),
                },
                names: TagNameIndex::load_from_definitions(&definitions),
                scope: FolderConversionScope::LooseSubtree {
                    source_rel_path: PathBuf::from("characters/jackal"),
                    destination_label: "jackal".to_owned(),
                    destination_parent: destination_parent.clone(),
                },
                source_game: "halo3_mcc".to_owned(),
                target_game: "haloreach_mcc".to_owned(),
                target_tags_root: target_tags.clone(),
                kit_roots: HashMap::new(),
                accept_loss: false,
                replace,
                only: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &tx,
        )
        .unwrap()
    };

    let kept = run(ReplacePolicy::Never);
    assert_eq!(
        TagFile::read(&output).unwrap().header.version,
        4321,
        "the kit's own tag was written over"
    );
    assert_eq!(kept.converted_count(), 0);
    assert!(
        kept.files
            .iter()
            .any(|file| file.status == FolderConversionFileStatus::Kept),
        "a kept tag has to be reported, or the answer to \"why is this one still the \
             old one?\" is nowhere",
    );

    // And the other way, so a passing test is not one where the run simply
    // did nothing at all.
    let replaced = run(ReplacePolicy::Always);
    assert_ne!(
        TagFile::read(&output).unwrap().header.version,
        4321,
        "the import did not write"
    );
    assert_eq!(replaced.converted_count(), 1);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_conversion_recurses_overwrites_and_continues_after_failure() {
    let definitions = locate_definitions_root();
    let unique = format!(
        "baboon_folder_conversion_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let source_root = root.join("source_tags");
    let source_folder = source_root.join("characters/jackal");
    let target_tags = root.join("target_tags");
    let destination_parent = target_tags.join("objects/characters");
    fs::create_dir_all(source_folder.join("nested")).unwrap();
    fs::create_dir_all(&target_tags).unwrap();

    let mut source = TagFile::new(definitions.join("halo3_mcc/weapon.json")).unwrap();
    seed_weapon_fields(&mut source);
    apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
    source
        .write_atomic(source_folder.join("nested/jackal.weapon"))
        .unwrap();
    source
        .write_atomic(source_folder.join("nested/jackal_alt.weapon"))
        .unwrap();
    fs::write(source_folder.join("notes.txt"), b"not a tag").unwrap();

    let mut bad = TagFile::new(definitions.join("halo3_mcc/light.json")).unwrap();
    apply_editing_kit_mcc_header(&mut bad, "halo3_mcc").unwrap();
    let bad_bytes = bad.write_to_bytes().unwrap();
    fs::write(source_folder.join("broken.light"), &bad_bytes[..64]).unwrap();

    let output = destination_parent.join("jackal/nested/jackal.weapon");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    let mut existing = TagFile::new(definitions.join("haloreach_mcc/weapon.json")).unwrap();
    apply_editing_kit_mcc_header(&mut existing, "haloreach_mcc").unwrap();
    existing.header.version = 8;
    existing.write_atomic(&output).unwrap();

    let source = TagSource::LooseFolder {
        root: source_root,
        game: Some(GameId::Halo3),
        definitions_root: definitions.clone(),
    };
    let names = TagNameIndex::load_from_definitions(&definitions);
    let (tx, _rx) = mpsc::channel();
    let report = run_folder_conversion_job(
        FolderConversionJob {
            source,
            names,
            scope: FolderConversionScope::LooseSubtree {
                source_rel_path: PathBuf::from("characters/jackal"),
                destination_label: "jackal".to_owned(),
                destination_parent: destination_parent.clone(),
            },
            source_game: "halo3_mcc".to_owned(),
            target_game: "haloreach_mcc".to_owned(),
            target_tags_root: target_tags,
            kit_roots: HashMap::new(),
            accept_loss: false,
            replace: ReplacePolicy::Always,
            only: None,
            cancel: Arc::new(AtomicBool::new(false)),
        },
        &tx,
    )
    .unwrap();

    // The pre-existing Reach weapon written to the output path below is a
    // usable template, and a kit tag still wins over the definitions while
    // the from-definitions path is unproven against the native tools. Set
    // `BLAM_BUILD_FROM_DEFINITIONS` and these two swap.
    assert_eq!(report.native_count(), 2);
    assert_eq!(report.generated_count(), 0);
    assert_eq!(report.failed_count(), 1);
    assert_eq!(report.ignored_files, vec!["characters/jackal/notes.txt"]);
    assert!(report.files.iter().any(|file| {
        file.source == "characters/jackal/nested/jackal.weapon" && file.overwritten
    }));
    let reopened = TagFile::read(&output).unwrap();
    let mut references = Vec::new();
    collect_reference_values(reopened.root(), "", &mut references);
    assert!(references.iter().any(|reference| {
        reference.group_tag == u32::from_be_bytes(*b"bitm")
            && reference.tag_path == "objects\\test\\icon"
    }));
    assert!(
        destination_parent
            .join("jackal/nested/jackal_alt.weapon")
            .is_file()
    );

    fs::remove_dir_all(root).unwrap();
}

/// A folder run names a renamed class the way the converter does.
///
/// Halo 4 calls Halo 3's `contrail_system` a `tracer_system`. Importing one
/// tag worked and importing the folder it sits in failed on the same tag,
/// because the folder run planned its output by canonical name: Halo 4 has no
/// `contrail_system` group, so the file could not be named and the tag was
/// reported as unconvertible before anything tried to convert it.
///
/// The quieter half of the same bug is worse and is why the extension comes
/// from the draft as well: Halo 4 *does* still declare `shader`, so a Reach
/// shader was named `.shader` while the converter built a `.material`.
#[test]
fn a_folder_run_writes_a_renamed_class_under_its_new_name() {
    let definitions = locate_definitions_root();
    let unique = format!(
        "baboon_renamed_class_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let source_root = root.join("source_tags");
    let source_folder = source_root.join("fx");
    let target_tags = root.join("target_tags");
    let destination_parent = target_tags.join("fx");
    fs::create_dir_all(&source_folder).unwrap();
    fs::create_dir_all(&target_tags).unwrap();

    let mut source = TagFile::new(definitions.join("halo3_mcc/contrail_system.json")).unwrap();
    apply_editing_kit_mcc_header(&mut source, "halo3_mcc").unwrap();
    source
        .write_atomic(source_folder.join("smoke.contrail_system"))
        .unwrap();

    let source = TagSource::LooseFolder {
        root: source_root,
        game: Some(GameId::Halo3),
        definitions_root: definitions.clone(),
    };
    let names = TagNameIndex::load_from_definitions(&definitions);
    let (tx, _rx) = mpsc::channel();
    let report = run_folder_conversion_job(
        FolderConversionJob {
            source,
            names,
            scope: FolderConversionScope::LooseSubtree {
                source_rel_path: PathBuf::from("fx"),
                destination_label: "fx".to_owned(),
                destination_parent: destination_parent.clone(),
            },
            source_game: "halo3_mcc".to_owned(),
            target_game: "halo4_mcc".to_owned(),
            target_tags_root: target_tags,
            kit_roots: HashMap::new(),
            accept_loss: true,
            replace: ReplacePolicy::Always,
            only: None,
            cancel: Arc::new(AtomicBool::new(false)),
        },
        &tx,
    )
    .unwrap();

    let written = destination_parent.join("fx/smoke.tracer_system");
    assert_eq!(
        report.failed_count(),
        0,
        "{:?}",
        report
            .files
            .iter()
            .map(|file| file.detail.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(report.converted_count(), 1);
    assert!(written.is_file(), "expected {}", written.display());
    assert!(!destination_parent.join("fx/smoke.contrail_system").exists());

    fs::remove_dir_all(root).unwrap();
}

// Duplicated from the engine's own conversion tests rather than exported:
// scaffolding that seeds a tag with one of each field kind is test code, and
// making the engine ship it just so this one worker test can call it would put
// test-only surface in the published library.
#[derive(Clone)]
struct LeafSeed {
    ordinal: usize,
    field_type: TagFieldType,
    option: Option<String>,
}

fn first_direct_leaf(tag: &TagFile, wanted: impl Fn(TagFieldType) -> bool) -> LeafSeed {
    tag.root()
        .fields()
        .enumerate()
        .find_map(|(ordinal, field)| {
            wanted(field.field_type()).then(|| {
                let option = match field.options() {
                    Some(TagOptions::Enum { names, .. }) => {
                        names.get(1).or(names.first()).map(|s| (*s).to_owned())
                    }
                    Some(TagOptions::Flags(options)) => {
                        options.first().map(|option| option.name.to_owned())
                    }
                    None => None,
                };
                LeafSeed {
                    ordinal,
                    field_type: field.field_type(),
                    option,
                }
            })
        })
        .expect("expected direct field type")
}

fn seed_weapon_fields(tag: &mut TagFile) {
    let reference =
        first_direct_leaf(tag, |field_type| field_type == TagFieldType::TagReference);
    tag.root_mut()
        .field_at_mut(reference.ordinal)
        .unwrap()
        .set(TagFieldData::TagReference(TagReferenceData {
            group_tag_and_name: Some((
                u32::from_be_bytes(*b"bitm"),
                "objects\\test\\icon".to_owned(),
            )),
        }))
        .unwrap();

    let real = first_direct_leaf(tag, is_real_scalar);
    tag.root_mut()
        .field_at_mut(real.ordinal)
        .unwrap()
        .set(real_field_value(real.field_type, 0.625))
        .unwrap();

    let enumeration = first_direct_leaf(tag, is_enum_type);
    let enum_name = enumeration.option.unwrap();
    let enum_value = match enumeration.field_type {
        TagFieldType::CharEnum => TagFieldData::CharEnum {
            value: 1,
            name: Some(enum_name),
        },
        TagFieldType::ShortEnum => TagFieldData::ShortEnum {
            value: 1,
            name: Some(enum_name),
        },
        TagFieldType::LongEnum => TagFieldData::LongEnum {
            value: 1,
            name: Some(enum_name),
        },
        _ => unreachable!(),
    };
    tag.root_mut()
        .field_at_mut(enumeration.ordinal)
        .unwrap()
        .set(enum_value)
        .unwrap();

    let flags = first_direct_leaf(tag, is_flags_type);
    let flag_name = flags.option.unwrap();
    let flag_value = match flags.field_type {
        TagFieldType::ByteFlags => TagFieldData::ByteFlags {
            value: 1,
            names: vec![(0, flag_name)],
        },
        TagFieldType::WordFlags => TagFieldData::WordFlags {
            value: 1,
            names: vec![(0, flag_name)],
        },
        TagFieldType::LongFlags => TagFieldData::LongFlags {
            value: 1,
            names: vec![(0, flag_name)],
        },
        _ => unreachable!(),
    };
    tag.root_mut()
        .field_at_mut(flags.ordinal)
        .unwrap()
        .set(flag_value)
        .unwrap();

    let string_id = first_direct_leaf(tag, is_string_id_type);
    let string_value = if string_id.field_type == TagFieldType::StringId {
        TagFieldData::StringId(StringIdData {
            string: "converted-label".to_owned(),
        })
    } else {
        TagFieldData::OldStringId(StringIdData {
            string: "converted-label".to_owned(),
        })
    };
    tag.root_mut()
        .field_at_mut(string_id.ordinal)
        .unwrap()
        .set(string_value)
        .unwrap();

    let magazines = tag
        .root()
        .fields()
        .enumerate()
        .find(|(_, field)| {
            field.field_type() == TagFieldType::Block
                && clean_field_key(field.name()) == "magazines"
        })
        .map(|(ordinal, _)| ordinal)
        .expect("weapon has magazines block");
    let mut root = tag.root_mut();
    let mut field = root.field_at_mut(magazines).unwrap();
    let mut block = field.as_block_mut().unwrap();
    block.add_element();
}

/// A folder import brings the folder, and asks about the rest.
///
/// Two things are checked together because either alone would pass while the
/// feature was broken. The first run attempts what is under the folder and
/// nothing else — a folder import that quietly pulled in two thousand tags
/// is the behaviour this replaced. And what it *reports* has to be complete:
/// every tag it needed is either written, named in the report, or was
/// already missing from the build, because that list is the question the
/// user answers and a gap in it is a gap they never see.
///
/// The destination here is an empty directory, so every tag refuses: a
/// byte-order upgrade will not build a tag from the schema, and an empty
/// kit ships no example of anything (see
/// `blam_tags::convert::analyze_conversion_inner`). That is deliberate. It
/// keeps the test off the user's real kit, and the scope and reporting this
/// is about happen either way — a tag names what it needs whether or not
/// it converts.
#[test]
fn a_cache_folder_import_converts_the_folder_and_reports_what_it_reaches() {
    let (Some(cache_root), definitions) = (reach_x360_cache_root(), locate_definitions_root())
    else {
        eprintln!("skipping: needs BABOON_REACH_X360_CACHE");
        return;
    };
    let names = TagNameIndex::load_from_definitions(&definitions);
    let loaded = match crate::core::source::load_monolithic_blob_index(
        cache_root.join("blob_index.dat"),
        &names,
    ) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("skipping: could not open the cache: {error}");
            return;
        }
    };
    // A weapon is the useful seed: small, and it reaches a model, a
    // collision model and a set of shaders without dragging in a scenario.
    let Some(seed) = loaded
        .entries
        .iter()
        .find(|entry| entry.group_tag == u32::from_be_bytes(*b"weap"))
    else {
        eprintln!("skipping: no weapon in the cache");
        return;
    };
    let TagEntryLocation::Monolithic { name, .. } = &seed.location else {
        unreachable!("a cache entry is monolithic");
    };
    let prefix = name
        .rsplit_once('\\')
        .map(|(folder, _)| folder.to_owned())
        .unwrap_or_default();
    let folder_keys: HashSet<String> = loaded
        .entries
        .iter()
        .filter(|entry| cache_entry_is_under(entry, &prefix))
        .map(|entry| entry.key.clone())
        .collect();

    let target_tags = std::env::temp_dir().join(format!(
        "baboon_cache_import_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&target_tags).unwrap();

    let run = |seed: CacheSeed| {
        let (tx, _rx) = mpsc::channel();
        run_folder_conversion_job(
            FolderConversionJob {
                source: loaded.source.clone(),
                names: names.clone(),
                scope: FolderConversionScope::CacheSubtree {
                    destination: CacheDestination::OwnPath,
                    prefix: prefix.clone(),
                    entries: loaded.entries.clone(),
                    seed,
                },
                // The pair the byte order makes real. See
                // `blam_tags::convert::analyze_conversion_inner`.
                source_game: "haloreach_mcc".to_owned(),
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
        .unwrap()
    };

    let report = run(CacheSeed::Folder);
    assert_eq!(
        report.files.len() + report.held_back.len(),
        folder_keys.len(),
        "the run did not account for every tag in {prefix}"
    );
    // Only the folder. Anything else it needed is a question, not a fait
    // accompli.
    for file in &report.files {
        let source = file.source.replace('/', "\\").to_ascii_lowercase();
        assert!(
            source.starts_with(&prefix.to_ascii_lowercase()),
            "{} is outside {prefix} and was converted anyway",
            file.source
        );
    }
    assert!(
        !report.outside_references.is_empty(),
        "a weapon folder reached nothing outside itself, which cannot be right"
    );

    // Accepting the answer brings them, and only them.
    let accepted: HashSet<String> = report
        .outside_references
        .iter()
        .map(|reference| reference.key.clone())
        .collect();
    let second = run(CacheSeed::Keys(accepted.clone()));
    assert_eq!(
        second.files.len() + second.held_back.len(),
        accepted.len(),
        "the second run did not account for every tag it was given"
    );

    // Nothing points at a tag nobody accounted for. Written is the good
    // case; failed, held back, still-outside and already-broken-in-the-build
    // are the honest ones, because each is named in a report the user reads.
    // Silence is the bug this catches.
    let mut written = HashSet::new();
    let mut accounted: HashSet<String> = HashSet::new();
    for report in [&report, &second] {
        for file in &report.files {
            let stem = file
                .source
                .rsplit_once('.')
                .map(|(stem, _)| stem)
                .unwrap_or(&file.source)
                .replace('\\', "/")
                .to_ascii_lowercase();
            accounted.insert(stem.clone());
            if let Some(output) = file.output.as_ref() {
                assert!(
                    output.starts_with(&target_tags),
                    "{} escaped the kit",
                    output.display()
                );
                let tag = TagFile::read(output).unwrap_or_else(|error| {
                    panic!("{} did not reparse: {error}", output.display())
                });
                assert_eq!(
                    tag.endian,
                    Endian::Le,
                    "{} was written big-endian",
                    output.display()
                );
                written.insert(stem);
            }
        }
        for entry in &report.held_back {
            accounted.insert(
                entry
                    .source
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(&entry.source)
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            );
        }
        for reference in &report.outside_references {
            accounted.insert(
                reference
                    .display_path
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(&reference.display_path)
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            );
        }
        for missing in &report.unresolved_references {
            accounted.insert(
                missing
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(missing)
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            );
        }
    }
    let mut dangling = Vec::new();
    for output in report
        .files
        .iter()
        .chain(&second.files)
        .filter_map(|file| file.output.as_ref())
    {
        let tag = TagFile::read(output).unwrap();
        let mut refs = Vec::new();
        collect_tag_dependency_refs(tag.root(), &mut refs);
        for reference in refs {
            let wanted = reference.rel_path.replace('\\', "/").to_ascii_lowercase();
            if !accounted.contains(&wanted) {
                dangling.push(format!("{} -> {wanted}", output.display()));
            }
        }
    }
    let _ = fs::remove_dir_all(&target_tags);
    let _ = folder_keys;
    assert!(
        dangling.is_empty(),
        "{} reference(s) nobody accounted for: {:?}",
        dangling.len(),
        dangling.iter().take(8).collect::<Vec<_>>()
    );
}

/// The cache root this was developed against, from the environment.
///
/// Not a fixture that can be committed: it is a 27 GB game build. Absent, the
/// tests above say so and return, the same bargain the kit-backed tests make.
fn reach_x360_cache_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("BABOON_REACH_X360_CACHE").ok()?);
    root.join("blob_index.dat").is_file().then_some(root)
}

fn cache_entry(name: &str, group: &[u8; 4]) -> TagEntry {
    let group_tag = u32::from_be_bytes(*group);
    TagEntry {
        key: format!("cache:{}:{name}", format_group_tag(group_tag)),
        display_path: name.replace('\\', "/"),
        group_tag,
        group_name: None,
        location: TagEntryLocation::Monolithic {
            name: name.to_owned(),
            group_tag,
        },
    }
}

/// A folder is a run of path segments, not a run of characters.
///
/// The distinction is the whole of it: `objects\weapons\rifle` and
/// `objects\weapons\rifleman` share a string prefix and share no folder, and
/// a plain `starts_with` would drag the second into a run asked for the
/// first.
#[test]
fn a_cache_folder_takes_its_own_tags_and_not_its_neighbours() {
    let rifle = cache_entry(r"objects\weapons\rifle\assault_rifle", b"weap");
    let deeper = cache_entry(r"objects\weapons\rifle\scope\scope", b"weap");
    let neighbour = cache_entry(r"objects\weapons\rifleman\rifleman", b"weap");
    let elsewhere = cache_entry(r"objects\vehicles\warthog\warthog", b"vehi");

    for entry in [&rifle, &deeper] {
        assert!(
            cache_entry_is_under(entry, r"objects\weapons\rifle"),
            "{} should be under the folder",
            entry.display_path
        );
    }
    for entry in [&neighbour, &elsewhere] {
        assert!(
            !cache_entry_is_under(entry, r"objects\weapons\rifle"),
            "{} should not be",
            entry.display_path
        );
    }
    // Forward slashes and case are what the browser hands back, not what the
    // cache stores.
    assert!(cache_entry_is_under(&rifle, "Objects/Weapons/Rifle"));
    // Trailing separators come from the same place.
    assert!(cache_entry_is_under(&rifle, r"objects\weapons\rifle\"));
    // The whole cache.
    assert!(cache_entry_is_under(&elsewhere, ""));
}

/// A reference and the entry it names have to fold to the same key.
///
/// Tag paths inside a tag are written by whoever authored it, so the same
/// tag turns up spelled several ways across a build. A lookup that respected
/// those differences would follow some references and quietly drop others.
#[test]
fn a_reference_folds_to_the_key_of_the_tag_it_names() {
    let entry = cache_entry(r"objects\weapons\rifle\assault_rifle", b"weap");
    let TagEntryLocation::Monolithic { name, group_tag } = &entry.location else {
        unreachable!()
    };
    let canonical = folded_cache_key(*group_tag, name);
    for spelling in [
        r"objects\weapons\rifle\assault_rifle",
        "objects/weapons/rifle/assault_rifle",
        r"Objects\Weapons\Rifle\Assault_Rifle",
    ] {
        assert_eq!(
            folded_cache_key(u32::from_be_bytes(*b"weap"), spelling),
            canonical,
            "{spelling} folded to a different key"
        );
    }
    // The group is part of the identity: two classes can share a path.
    assert_ne!(
        folded_cache_key(u32::from_be_bytes(*b"hlmt"), name),
        canonical
    );
}
