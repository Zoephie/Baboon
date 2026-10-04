use super::*;

/// The name-map filter matches exactly what lowercasing each name did,
/// without the per-row, per-frame copy.
#[test]
fn the_name_filter_matches_as_lowercasing_did() {
    let names = [
        "SM_Warthog_Chassis",
        "/Game/Vehicles/Warthog",
        "ÉCLAIR_Mesh",
        "éclair_mesh",
        "",
        "hog",
    ];
    for filter in ["", "hog", "warthog_c", "/game/", "éclair", "clair_m", "zzz"] {
        for name in names {
            assert_eq!(
                contains_ignore_ascii_case(name, filter),
                name.to_ascii_lowercase().contains(filter),
                "{name:?} / {filter:?}"
            );
        }
    }
}

/// Object flags decide how a payload is *read*, so a wrong one is not a
/// mislabel — it is a different interpretation of the same bytes. The named
/// bits have to be the ones the engine uses.
#[test]
fn the_named_object_flag_bits_match_the_engine_values() {
    let named: Vec<(u32, &str)> = CHIMP_OBJECT_FLAG_BITS.to_vec();
    assert_eq!(named[0], (0x0000_0001, "Public"));
    assert_eq!(named[1], (0x0000_0002, "Standalone"));
    assert_eq!(named[2], (0x0000_0008, "Transactional"));
    // The one that actually changes decoding, via the native tail branch.
    assert_eq!(named[3], (0x0000_0010, "ClassDefaultObject"));
    assert_eq!(named[4], (0x0000_0020, "ArchetypeObject"));

    // Campaign Evolved's measured tag flags decompose into these, with
    // nothing unnamed left over: 0xb is Public + Standalone + Transactional.
    let all: u32 = CHIMP_OBJECT_FLAG_BITS.iter().map(|(bit, _)| bit).sum();
    assert_eq!(0xb_u32 & !all, 0, "0xb is fully named");
    assert_eq!(
        0x1_u32 & !all,
        0,
        "the generated-group value is fully named"
    );
}

/// The synthetic `Thing` in the Header view, collecting what each frame
/// asked for.
struct Header {
    install: SyntheticInstall,
    document: ChimpDocument,
    pane: ChimpDocumentUi,
    expert: bool,
    changed: bool,
    scan: bool,
    frames: Frames,
}

impl Header {
    fn new() -> Self {
        let install = SyntheticInstall::new();
        let document = install.document(THING);
        let pane = install.pane(&document);
        let mut header = Self {
            install,
            document,
            pane,
            expert: false,
            changed: false,
            scan: false,
            frames: Frames::new(),
        };
        header.act(|frames, draw| {
            frames.frame(Vec::new(), draw);
        });
        header
    }

    /// Run `act`, then report and clear whether any frame applied an edit.
    fn act(&mut self, act: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut egui::Ui))) -> bool {
        let Self {
            install,
            document,
            pane,
            expert,
            changed,
            scan,
            frames,
        } = self;
        let world = install.world.clone();
        let mut draw = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let mut asked = false;
                *changed |= draw_chimp_header_view(ui, document, pane, &world, *expert, &mut asked);
                *scan |= asked;
            });
        };
        act(frames, &mut draw);
        std::mem::take(changed)
    }

    fn click(&mut self, text: &str) -> bool {
        self.act(|frames, draw| frames.click(text, draw))
    }

    fn shows(&self, text: &str) -> bool {
        self.frames.shows(text)
    }
}

/// The view lists the identity, every name with how often it is used,
/// every import slot with the properties naming it, and the export map
/// behind a closed header.
#[test]
fn the_header_view_lists_names_imports_and_usage() {
    let header = Header::new();
    let usage = header.pane.header_usage.as_ref().expect("refreshed");
    assert_eq!(
        usage
            .names
            .iter()
            .map(|usage| (usage.count, usage.is_package_identity))
            .collect::<Vec<_>>(),
        [(1, true), (1, false), (1, false)]
    );
    assert_eq!(usage.import_references, [1, 0]);
    for text in [
        "Identity",
        "Edit flags and versioning...",
        "0x00000000",
        "Name map (3)",
        "Rocket",
        "Import map (2)",
        "/Game/Test/Other#",
        "/Script/ChimpTest#",
        "1 property",
        "unreferenced",
        "Export map (1)",
        "Referenced by",
    ] {
        assert!(header.shows(text), "{text}");
    }
    assert!(!header.shows("flags 0x0000000B"), "the export map starts closed");
}

/// Clicking a name opens its editor with the blast radius, and Enter
/// renames the entry and every value showing it.
#[test]
fn renaming_a_name_entry_retargets_every_reference() {
    let mut header = Header::new();
    assert!(!header.act(|frames, draw| frames.click_exact("Rocket", 0, draw)));
    let edit = header.pane.header_name_edit.as_ref().expect("editing");
    assert_eq!((edit.index, edit.text.as_str()), (2, "Rocket"));
    assert!(header.shows("Editing entry 2 · Rocket"));
    assert!(header.shows("1 reference will follow this rename:"));
    assert!(header.shows("    • export 0 properties"));

    assert!(header.act(|frames, draw| {
        frames.replace_text("Comet", draw);
        frames.key(egui::Key::Enter, egui::Modifiers::NONE, draw);
    }));
    assert!(header.pane.header_name_edit.is_none());
    assert_eq!(header.document.header.name_map.names()[2], "Comet");
    assert!(matches!(
        first_value(&header.document, "Tag"),
        PropValue::Name(name) if name.as_str() == "Comet"
    ));
}

/// The package's own name has no editor: it is the package's identity.
#[test]
fn the_package_name_entry_is_not_editable() {
    let mut header = Header::new();
    // The identity grid shows it first; the name-map row second.
    assert!(!header.act(|frames, draw| frames.click_exact(THING, 1, draw)));
    assert!(header.pane.header_name_edit.is_none());
}

/// Flags and versioning are drafted, applied only if the package reads
/// back as written, and a bad value is refused with the draft kept.
#[test]
fn identity_edits_apply_and_refuse_bad_flags() {
    let mut header = Header::new();
    assert!(!header.click("Edit flags and versioning..."));
    let edit = header.pane.header_identity_edit.as_ref().expect("drafting");
    assert_eq!(edit.package_flags, "00000000");
    assert!(header.shows("Versioning fields are Expert mode only"));
    assert!(!header.click("0x80002200"));
    assert_eq!(
        header.pane.header_identity_edit.as_ref().unwrap().package_flags,
        "80002200"
    );
    assert!(header.act(|frames, draw| frames.click_exact("Apply", 0, draw)));
    assert_eq!(header.document.header.summary.package_flags, 0x8000_2200);
    assert!(header.pane.header_identity_edit.is_none());

    header.click("Edit flags and versioning...");
    header.pane.header_identity_edit.as_mut().unwrap().package_flags = "zz".to_owned();
    assert!(!header.act(|frames, draw| frames.click_exact("Apply", 0, draw)));
    assert_eq!(
        header.pane.header_error.as_deref(),
        Some("\"zz\" is not a 32-bit hex value")
    );
    assert!(header.pane.header_identity_edit.is_some(), "the draft is kept");
    assert!(header.shows("\"zz\" is not a 32-bit hex value"));
    assert_eq!(header.document.header.summary.package_flags, 0x8000_2200);

    header.act(|frames, draw| frames.click_exact("Cancel", 0, draw));
    assert!(header.pane.header_identity_edit.is_none());
    assert!(header.pane.header_error.is_none());
}

/// Expert mode exposes the versioning fields themselves.
#[test]
fn expert_identity_edits_show_the_versioning_fields() {
    let mut header = Header::new();
    header.expert = true;
    header.click("Edit flags and versioning...");
    for text in ["Unversioned", "Zen version", "File version UE4", "These decide the header's shape"] {
        assert!(header.shows(text), "{text}");
    }
    assert!(!header.shows("Versioning fields are Expert mode only"));
}

/// A new import slot is drafted, its target's exports listed from the
/// mount to pick a name from, and applied at the end of the map.
#[test]
fn an_import_slot_is_added_from_a_listed_export() {
    let mut header = Header::new();
    assert!(!header.click("+ Add import slot"));
    {
        let edit = header.pane.header_import_edit.as_mut().expect("drafting");
        assert_eq!(edit.slot, 2);
        assert!(edit.kind == ChimpImportKind::Null);
        edit.kind = ChimpImportKind::Package;
        edit.package_path = OTHER.to_owned();
    }
    header.click("List exports");
    let edit = header.pane.header_import_edit.as_ref().unwrap();
    assert_eq!(
        edit.resolved.as_ref().unwrap().as_ref().unwrap(),
        &[("OtherThing".to_owned(), public_export_hash("OtherThing"))]
    );
    header.act(|frames, draw| frames.click_exact("OtherThing", 0, draw));
    assert_eq!(
        header.pane.header_import_edit.as_ref().unwrap().object_name,
        "OtherThing"
    );
    assert!(header.act(|frames, draw| frames.click_exact("Apply", 0, draw)));
    assert!(header.pane.header_import_edit.is_none());
    let slots = read_import_slots(&header.document.header).unwrap();
    assert_eq!(slots.len(), 3);
    assert_eq!(
        slots[2],
        ImportSlot::Package(ImportTarget {
            package: OTHER.to_owned(),
            object_hash: public_export_hash("OtherThing"),
        })
    );
    assert!(header.shows("Import map (3)"));
}

/// Clicking an existing slot drafts it as it stands, with the object
/// name recovered from the target package.
#[test]
fn retargeting_an_import_slot_starts_from_what_is_there() {
    let mut header = Header::new();
    header.click("/Game/Test/Other#");
    let edit = header.pane.header_import_edit.as_ref().expect("drafting");
    assert_eq!(edit.slot, 0);
    assert!(edit.kind == ChimpImportKind::Package);
    assert_eq!(edit.package_path, OTHER);
    assert_eq!(edit.object_name, "OtherThing");
    assert!(header.shows("Import slot 0"));
}

/// An export's name and flags are drafted and applied; a flag change
/// re-reads the export through the new flags without losing an unsaved
/// property edit, and the stored hash stays put unless asked.
#[test]
fn an_export_edit_renames_and_reflags_keeping_unsaved_values() {
    let mut header = Header::new();
    set_first_value(&mut header.document, "Count", PropValue::Int(42));
    header.click("Export map (1)");
    assert!(header.shows("flags 0x0000000B"));
    // The name map lists "Thing" first; the export row second.
    header.act(|frames, draw| frames.click_exact("Thing", 1, draw));
    let edit = header.pane.header_export_edit.as_ref().expect("drafting");
    assert_eq!((edit.index, edit.object_name.as_str(), edit.object_flags), (0, "Thing", 0xb));
    assert!(!header.click("ClassDefaultObject"));
    assert_eq!(header.pane.header_export_edit.as_ref().unwrap().object_flags, 0x1b);
    header.pane.header_export_edit.as_mut().unwrap().object_name = "Thing2".to_owned();
    header.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(header.shows("This export is named after the package (Thing)."));
    assert!(header.shows("Public export hash 0x"));
    assert!(!header.shows("Recompute it as"), "an expert-mode choice");

    assert!(header.act(|frames, draw| frames.click_exact("Apply", 0, draw)));
    let entry = &header.document.header.export_map[0];
    assert_eq!(entry.object_flags, 0x1b);
    assert_eq!(entry.public_export_hash, public_export_hash("Thing"));
    assert_eq!(
        header.document.header.name_map.try_get(entry.object_name).as_deref(),
        Some("Thing2")
    );
    assert_eq!(header.document.header.name_map.len(), 4, "interned, not renamed");
    assert_eq!(header.document.exports[0].object, "Thing2");
    assert!(matches!(first_value(&header.document, "Count"), PropValue::Int(42)));
}

/// The referrer section asks for a scan, and reports one honestly.
#[test]
fn the_referrer_section_asks_for_a_scan_and_reports_it() {
    let mut header = Header::new();
    header.click("Referenced by");
    assert!(!header.scan);
    header.click("Find packages that import this");
    assert!(header.scan, "the scan is asked of the caller");

    header.pane.referrers = ChimpReferrerState::Done(ChimpReferrerScan {
        referrers: vec![OTHER.to_owned()],
        scanned: 1,
        unreadable: 0,
    });
    header.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(header.shows("1 package of 1 imports this"));
    assert!(header.shows(OTHER));

    header.pane.referrers = ChimpReferrerState::Done(ChimpReferrerScan {
        referrers: Vec::new(),
        scanned: 3,
        unreadable: 2,
    });
    header.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(header.shows("No hard import, of 3 packages read"));
    assert!(header.shows("Soft references live in export data and are not counted."));
    assert!(header.shows("2 package(s) could not be read and are not ruled out."));
}
