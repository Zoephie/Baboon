//! Shared fixtures for Chimp's unit tests.
//! It owns the synthetic headers and documents several test modules build on; the tests themselves sit beside their code.

use super::*;
use blam_tags::iostore::package::name_map::{EMappedNameType, FNameMap};
use blam_tags::iostore::package::zen::{
    EExportCommandType, FDependencyBundleHeader, FExportBundleEntry, FExportMapEntry,
};

/// A header with `names` interned and one export named after the first.
pub(super) fn header_with_names(names: &[&str]) -> FZenPackageHeader {
    let mut header = FZenPackageHeader {
        container_header_version: CE_HEADER_VERSION,
        is_unversioned: true,
        ..Default::default()
    };
    header.name_map = FNameMap::create(EMappedNameType::Package);
    for name in names {
        header.name_map.store(name);
    }
    header.summary.name = FMappedName::create(0, EMappedNameType::Package, 0);
    header.export_map = vec![FExportMapEntry {
        cooked_serial_offset: 0,
        cooked_serial_size: 0,
        object_name: FMappedName::create(0, EMappedNameType::Package, 0),
        outer_index: FPackageObjectIndex::default(),
        class_index: FPackageObjectIndex::default(),
        super_index: FPackageObjectIndex::default(),
        template_index: FPackageObjectIndex::default(),
        public_export_hash: 0,
        object_flags: 0,
        filter_flags: EExportFilterFlags::None,
        padding: [0; 3],
    }];
    // The reader enforces a Create and a Serialize command per export, plus
    // a dependency bundle header. A header without them is not a package —
    // which the reopen gate rightly refuses, so the fixture has to be one.
    for command_type in [EExportCommandType::Create, EExportCommandType::Serialize] {
        header.export_bundle_entries.push(FExportBundleEntry {
            local_export_index: 0,
            command_type,
        });
    }
    header
        .dependency_bundle_headers
        .push(FDependencyBundleHeader::default());
    header
}

/// A document with two names, an export named after the second, and one
/// reflected property holding `FName`s that point at it.
pub(super) fn rename_fixture() -> ChimpDocument {
    use blam_tags::iostore::object::value::{BlockLayout, PropertyEntry};

    let mut header = header_with_names(&["Warthog", "Material"]);
    // The export is named after entry 1, leaving entry 0 as the package's
    // own identity — the one the guard refuses.
    header.export_map[0].object_name = FMappedName::create(1, EMappedNameType::Package, 0);
    header.export_map[0].public_export_hash = public_export_hash("Material");

    let block = PropertyBlock {
        entries: vec![
            PropertyEntry {
                name: "Plain".into(),
                value: PropValue::Name(FName::new(1, 0, "Material")),
                slot: None,
            },
            PropertyEntry {
                name: "Numbered".into(),
                // Number 3 renders as `_2`, which is how the reader composes
                // it — the refresh has to reproduce that, not just the base.
                value: PropValue::Array(vec![PropValue::Name(FName::new(1, 3, "Material_2"))]),
                slot: None,
            },
            PropertyEntry {
                name: "Other".into(),
                value: PropValue::Name(FName::new(0, 0, "Warthog")),
                slot: None,
            },
        ],
        layout: BlockLayout::Unversioned {
            schema_len: 3,
            leading_empty: 0,
        },
    };

    ChimpDocument {
        package: "/Game/Test/Thing".to_owned(),
        provider: PackageProvider {
            container: 0,
            entry_path: "Content/Test/Thing.uasset".to_owned(),
            read_order: 0,
        },
        original: Vec::new(),
        header,
        payloads: vec![Vec::new()],
        exports: vec![ChimpExport {
            object: "Material".to_owned(),
            class: None,
            decoded: Ok(Export {
                block: ExportBlock::Reflected(block),
                trailer: blam_tags::iostore::object::export::Trailer::NoGuid,
                tail: Vec::new(),
            }),
        }],
        texture_previews: Vec::new(),
        mesh_kind: None,
        mesh_preview: None,
        mesh_preview_state: Default::default(),
        selected_export: 0,
        dirty: false,
        view: ChimpDocumentView::Header,
        document_text: String::new(),
        document_lines: ChimpJsonLines::default(),
        document_text_dirty: false,
        metadata_text: String::new(),
        metadata_lines: ChimpJsonLines::default(),
        metadata_text_dirty: false,
        header_usage: None,
        header_name_filter: String::new(),
        header_name_edit: None,
        header_import_edit: None,
        header_export_edit: None,
        header_identity_edit: None,
        header_error: None,
        referrers: ChimpReferrerState::Idle,
        orphaned: false,
        checkpoint_due: None,
        edits: 0,
    }
}

/// The synthetic class every [`SyntheticInstall`] package is an instance of,
/// imported from a script package the bundled USMAP has never heard of.
pub(super) const SYNTHETIC_CLASS_PACKAGE: &str = "/Script/ChimpTest";
pub(super) const SYNTHETIC_CLASS: &str = "ChimpThing";
/// The reflected struct the class's `Inner` property holds.
pub(super) const SYNTHETIC_INNER: &str = "ChimpInner";
pub(super) const THING: &str = "/Game/Test/Thing";
pub(super) const OTHER: &str = "/Game/Test/Other";

/// The synthetic class's properties, in schema order.
pub(super) const SYNTHETIC_PROPERTIES: [&str; 14] = [
    "Count", "Scale", "Enabled", "Label", "Tag", "Values", "Lookup", "Unique", "Maybe", "Mode",
    "Target", "Inner", "Spare", "Later",
];

/// The USMAP key a package import of the synthetic class resolves to: what
/// `World::class_key` produces for a `PackageImport` class index.
pub(super) fn synthetic_class_key() -> String {
    format!(
        "{SYNTHETIC_CLASS_PACKAGE}#{:016x}",
        public_export_hash(SYNTHETIC_CLASS)
    )
}

/// The first enum in the bundled USMAP with at least three values, so the
/// enum row has a real list to choose from.
pub(super) fn synthetic_enum(usmap: &Usmap) -> (String, Vec<(u64, String)>) {
    usmap
        .enums
        .iter()
        .find(|definition| definition.values.len() >= 3)
        .map(|definition| (definition.name.clone(), definition.values.clone()))
        .expect("the bundled USMAP has enums")
}

/// The bundled USMAP with the synthetic class and its inner struct
/// registered: one property of each shape the property editor draws.
pub(super) fn synthetic_usmap() -> Usmap {
    use blam_tags::iostore::object::usmap::UsmapProperty;
    let mut usmap = Usmap::meteorite().expect("the bundled USMAP parses");
    let (enum_name, _) = synthetic_enum(&usmap);
    let property = |index: u16, name: &str, ty: PropertyType| UsmapProperty {
        schema_index: index,
        array_dim: 1,
        name: name.to_owned(),
        ty,
    };
    usmap.register_struct(
        SYNTHETIC_INNER,
        None,
        vec![
            property(0, "Depth", PropertyType::Int),
            property(1, "Weight", PropertyType::Float),
        ],
    );
    let types = [
        PropertyType::Int,
        PropertyType::Float,
        PropertyType::Bool,
        PropertyType::Str,
        PropertyType::Name,
        PropertyType::Array(Box::new(PropertyType::Int)),
        PropertyType::Map(Box::new(PropertyType::Int), Box::new(PropertyType::Int)),
        PropertyType::Set(Box::new(PropertyType::Int)),
        PropertyType::Optional(Box::new(PropertyType::Int)),
        PropertyType::Enum {
            inner: Box::new(PropertyType::Byte { enum_name: None }),
            enum_name,
        },
        PropertyType::Object,
        PropertyType::Struct(SYNTHETIC_INNER.to_owned()),
        PropertyType::Int,
        PropertyType::Optional(Box::new(PropertyType::Int)),
    ];
    usmap.register_struct(
        &synthetic_class_key(),
        None,
        SYNTHETIC_PROPERTIES
            .iter()
            .zip(types)
            .enumerate()
            .map(|(index, (name, ty))| property(index as u16, name, ty))
            .collect(),
    );
    usmap
}

/// An entry at schema `index` of a block named by `names`.
fn synthetic_entry(
    names: &[&str],
    index: usize,
    value: PropValue,
) -> blam_tags::iostore::object::value::PropertyEntry {
    blam_tags::iostore::object::value::PropertyEntry {
        name: names[index].into(),
        value,
        slot: Some(blam_tags::iostore::object::value::SchemaSlot {
            index: index as u32,
            array_index: 0,
            zero_masked: false,
        }),
    }
}

/// `Thing`'s property block: every property but `Spare` present with a
/// distinct non-zero value, and `Later` an unset optional.
fn synthetic_thing_block(usmap: &Usmap, tag: FName) -> PropertyBlock {
    use blam_tags::iostore::object::value::{BlockLayout, FStr};
    let (_, enum_values) = synthetic_enum(usmap);
    let inner = PropertyBlock {
        entries: vec![
            synthetic_entry(&["Depth", "Weight"], 0, PropValue::Int(3)),
            synthetic_entry(&["Depth", "Weight"], 1, PropValue::Float(0.5)),
        ],
        layout: BlockLayout::Unversioned {
            schema_len: 2,
            leading_empty: 0,
        },
    };
    let values = [
        PropValue::Int(7),
        PropValue::Float(1.5),
        PropValue::Bool(true),
        PropValue::Str(FStr::new("Warthog", false)),
        PropValue::Name(tag),
        PropValue::Array(vec![
            PropValue::Int(10),
            PropValue::Int(20),
            PropValue::Int(30),
        ]),
        PropValue::Map(vec![(PropValue::Int(1), PropValue::Int(100))]),
        PropValue::Set(vec![PropValue::Int(4), PropValue::Int(5)]),
        PropValue::Int(9),
        PropValue::Int(enum_values[1].0 as i64),
        PropValue::Object(import_package_index(0)),
        PropValue::Struct(inner),
    ];
    let mut entries: Vec<_> = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| synthetic_entry(&SYNTHETIC_PROPERTIES, index, value))
        .collect();
    // `Spare` (12) is left out for "Add omitted property"; `Later` is present
    // but unset.
    entries.push(synthetic_entry(&SYNTHETIC_PROPERTIES, 13, PropValue::Unset));
    PropertyBlock {
        entries,
        layout: BlockLayout::Unversioned {
            schema_len: SYNTHETIC_PROPERTIES.len() as u32,
            leading_empty: 0,
        },
    }
}

/// A package `name` with one export `object` of the synthetic class, whose
/// import map is `imports` followed by the class.
fn synthetic_package(
    usmap: &Usmap,
    name: &str,
    object: &str,
    imports: &[ImportSlot],
    block: impl FnOnce(&mut FZenPackageHeader) -> PropertyBlock,
) -> (Vec<u8>, blam_tags::iostore::container::header::StoreEntry) {
    use blam_tags::iostore::object::export::Trailer;
    // Written and read back once before anything is imported. Measured: an
    // import map added to a header that has never been through the writer
    // does not reopen ("failed to fill whole buffer"), while the same map on
    // a reopened header does.
    let (bytes, _) = write_package(
        &header_with_names(&[name, object]),
        &[Vec::new()],
        CE_HEADER_VERSION,
    )
    .expect("the bare synthetic header writes");
    let mut header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .expect("the bare synthetic header reopens");
    let mut slots = imports.to_vec();
    slots.push(ImportSlot::Package(ImportTarget {
        package: SYNTHETIC_CLASS_PACKAGE.to_owned(),
        object_hash: public_export_hash(SYNTHETIC_CLASS),
    }));
    write_import_slots(&mut header, &slots).expect("the synthetic import map writes");
    let class = *header.import_map.last().expect("the class slot");
    let entry = &mut header.export_map[0];
    entry.object_name = FMappedName::create(1, EMappedNameType::Package, 0);
    entry.public_export_hash = public_export_hash(object);
    // Public | Standalone | Transactional, as Campaign Evolved's assets are.
    entry.object_flags = 0xb;
    entry.class_index = class;
    let export = Export {
        block: ExportBlock::Reflected(block(&mut header)),
        trailer: Trailer::NoGuid,
        tail: Vec::new(),
    };
    let payload = write_export_in(&synthetic_class_key(), &export, usmap, None)
        .expect("the synthetic export serializes");
    write_package(&header, &[payload], CE_HEADER_VERSION).expect("the synthetic package writes")
}

/// Add a directory index naming `files` (file `i` is chunk `i`) under
/// `mount` to a `.utoc` the override writer produced, which has none.
fn add_synthetic_directory_index(utoc: &Path, mount: &str, files: &[&str]) {
    fn fstring(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&(text.len() as i32 + 1).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }
    const NONE: u32 = u32::MAX;
    let mut index = Vec::new();
    fstring(&mut index, mount);
    // One root directory holding every file.
    index.extend_from_slice(&1u32.to_le_bytes());
    for field in [NONE, NONE, NONE, 0] {
        index.extend_from_slice(&field.to_le_bytes());
    }
    let count = files.len() as u32;
    index.extend_from_slice(&count.to_le_bytes());
    for file in 0..count {
        let next = if file + 1 < count { file + 1 } else { NONE };
        for field in [file, next, file] {
            index.extend_from_slice(&field.to_le_bytes());
        }
    }
    index.extend_from_slice(&count.to_le_bytes());
    for file in files {
        fstring(&mut index, file);
    }

    let mut toc = fs::read(utoc).expect("read the synthetic TOC");
    let entries = u32::from_le_bytes(toc[24..28].try_into().unwrap()) as usize;
    // The writer closes the TOC with one 24-byte meta per chunk; the
    // directory index sits immediately before them.
    let at = toc.len() - entries * 24;
    toc[48..52].copy_from_slice(&(index.len() as u32).to_le_bytes());
    toc.splice(at..at, index);
    fs::write(utoc, toc).expect("write the synthetic TOC");
}

/// A Paks folder holding one container with two synthetic packages, mounted
/// against [`synthetic_usmap`].
///
/// `Thing` carries one property of every shape the editor draws and imports
/// `Other`, so `Other` has a referrer. Nothing comes from a real install: the
/// engine's own package writer writes the packages and its override writer
/// the container, with a directory index added so a mount can name them.
pub(super) struct SyntheticInstall {
    pub(super) root: PathBuf,
    pub(super) world: Arc<World>,
}

impl SyntheticInstall {
    pub(super) fn new() -> Self {
        use blam_tags::iostore::container::writer::{
            CHUNK_TYPE_EXPORT_BUNDLE_DATA, OverrideContainerWriter, make_chunk_id,
        };
        use blam_tags::iostore::package::ue_types::FPackageId;

        let usmap = synthetic_usmap();
        let thing = synthetic_package(
            &usmap,
            THING,
            "Thing",
            &[ImportSlot::Package(ImportTarget {
                package: OTHER.to_owned(),
                object_hash: public_export_hash("OtherThing"),
            })],
            |header| {
                let tag = header.name_map.store("Rocket");
                synthetic_thing_block(&usmap, FName::new(tag.index(), 0, "Rocket"))
            },
        );
        let other = synthetic_package(&usmap, OTHER, "OtherThing", &[], |_| PropertyBlock {
            entries: Vec::new(),
            layout: blam_tags::iostore::object::value::BlockLayout::Unversioned {
                schema_len: SYNTHETIC_PROPERTIES.len() as u32,
                leading_empty: 0,
            },
        });

        let root =
            std::env::temp_dir().join(format!("baboon-chimp-synthetic-{}", uuid::Uuid::new_v4()));
        let paks = root.join("Paks");
        fs::create_dir_all(&paks).expect("create the synthetic Paks folder");
        let utoc = paks.join("pakchunk0-Windows.utoc");
        let mut writer = OverrideContainerWriter::new("../../../");
        for (name, (bytes, store)) in [(THING, thing), (OTHER, other)] {
            let id = FPackageId::from_name(name);
            writer.add_package(
                make_chunk_id(id.0, 0, CHUNK_TYPE_EXPORT_BUNDLE_DATA),
                bytes,
                id,
                store,
            );
        }
        writer.write(&utoc).expect("write the synthetic container");
        add_synthetic_directory_index(
            &utoc,
            "../../../Meteorite/Content/Test/",
            &["Thing.uasset", "Other.uasset"],
        );
        let world = Arc::new(World::open(&root, usmap).expect("mount the synthetic install"));
        Self { root, world }
    }

    /// The kit source a Campaign Evolved workspace over this install has.
    pub(super) fn source(&self) -> LoadedSourceData {
        LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: self.root.clone(),
                containers: Vec::new(),
                index: Default::default(),
                packages: Default::default(),
                shipped: Default::default(),
            },
            names: TagNameIndex::default(),
            game: Some(GameId::CampaignEvolved),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    /// Where a workspace over this install keeps its recovery checkpoints:
    /// the app's data folder, keyed by a hash of the Paks root.
    pub(super) fn recovery_dir(&self) -> PathBuf {
        let digest = Sha256::digest(self.root.to_string_lossy().as_bytes());
        let key: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        crate::core::storage::data_path(&format!("chimp-recovery-{key}"))
    }

    /// `package`, decoded as opening it does.
    pub(super) fn document(&self, package: &str) -> ChimpDocument {
        load_chimp_document(&self.world, package).expect("the synthetic package decodes")
    }

    /// A test app whose kit 0 is this install, mounted, with `packages` open.
    pub(super) fn app_with_open(&self, packages: &[&str]) -> Baboon {
        let mut app = Baboon::for_test();
        app.model.kits[0].source = Some(self.source());
        app.model.kits[0].chimp.mount = ChimpMount::Ready(self.world.clone());
        let kit = app.model.kits[0].id;
        for package in packages {
            app.model.kits[0]
                .chimp
                .documents
                .insert((*package).to_owned(), self.document(package));
            app.model.kits[0].chimp.open_document_pane(kit, package);
        }
        app
    }
}

impl Drop for SyntheticInstall {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The value of `property` in `document`'s first export.
pub(super) fn first_value<'a>(document: &'a ChimpDocument, property: &str) -> &'a PropValue {
    first_block(document)
        .get(property)
        .unwrap_or_else(|| panic!("{property} is present"))
}

/// Set `property` of `document`'s first export in place, the way the
/// property editor mutates it.
pub(super) fn set_first_value(document: &mut ChimpDocument, property: &str, value: PropValue) {
    let block = document.exports[0]
        .decoded
        .as_mut()
        .expect("decoded")
        .properties_mut()
        .expect("reflected");
    let entry = block
        .entries
        .iter_mut()
        .find(|entry| &*entry.name == property)
        .unwrap_or_else(|| panic!("{property} is present"));
    entry.value = value;
}

/// Apply worker messages as frames would, until `done` holds. One frame
/// applies everything that has arrived, so two jobs that finish together
/// settle in one go.
///
/// Polls rather than taking a message off the channel to wait on it: putting
/// it back queues it behind any that arrived meanwhile, and a mount's result
/// applied after its own type index leaves the workspace "indexing" for good.
pub(super) fn apply_until(app: &mut Baboon, mut done: impl FnMut(&Baboon) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "the workers never settled: {} / mount {} / indexing {}",
            app.model.status,
            match &app.model.kits[0].chimp.mount {
                ChimpMount::Idle => "idle".to_owned(),
                ChimpMount::Loading => "loading".to_owned(),
                ChimpMount::Ready(_) => "ready".to_owned(),
                ChimpMount::Failed(error) => error.clone(),
            },
            app.model.kits[0].chimp.type_indexing
        );
        std::thread::sleep(Duration::from_millis(10));
        app.process_worker_messages(&egui::Context::default());
    }
}

/// The decoded property block of `document`'s first export.
pub(super) fn first_block(document: &ChimpDocument) -> &PropertyBlock {
    document.exports[0]
        .decoded
        .as_ref()
        .expect("decoded")
        .properties()
        .expect("reflected")
}

/// Just inside the right edge of a default central panel on a [`Frames`]
/// screen: where a right-aligned value widget sits.
pub(super) const VALUE_X: f32 = 1600.0 - 8.0 - 6.0;

/// Headless frames with a clock that advances and a pointer that moves the
/// way a hand does: onto a target over several frames, so hover and hit
/// tests (which read the previous frame) have settled before it presses.
pub(super) struct Frames {
    pub(super) ctx: egui::Context,
    time: f64,
    pointer: egui::Pos2,
    /// Where each painted text landed on the last frame.
    pub(super) labels: Vec<(String, egui::Rect)>,
    /// Where [`Frames::click_value_of`] clicks across: [`VALUE_X`] unless
    /// the panel is nested deeper.
    pub(super) value_x: f32,
}

impl Frames {
    pub(super) fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            pointer: egui::Pos2::ZERO,
            labels: Vec::new(),
            value_x: VALUE_X,
        }
    }

    /// The clock the next frame will run at.
    pub(super) fn time(&self) -> f64 {
        self.time
    }

    /// One frame of `events`, drawn by `draw`.
    pub(super) fn frame(
        &mut self,
        events: Vec<egui::Event>,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) -> &[(String, egui::Rect)] {
        // A tenth of a second, so two clicks a press apart are not a
        // double click.
        self.time += 0.1;
        let output = crate::app::run_ui_test(
            &self.ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 2400.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| draw(ui),
        );
        self.labels = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect();
        &self.labels
    }

    /// Whether any painted text on the last frame starts with `text`. A
    /// `DragValue` paints its prefix and its number as two texts, one after
    /// the other on the same line; such a pair counts as one text.
    pub(super) fn shows(&self, text: &str) -> bool {
        self.labels.iter().any(|(label, _)| label.starts_with(text))
            || self.labels.windows(2).any(|pair| {
                let ((first, a), (second, b)) = (&pair[0], &pair[1]);
                let gap = b.left() - a.right();
                (a.center().y - b.center().y).abs() < 1.0
                    && (-0.5..20.0).contains(&gap)
                    && format!("{first}{second}").starts_with(text)
            })
    }

    /// The `nth` painted text starting with `text`, once it has stopped
    /// moving. A window lays itself out unseen on its first frame, and an
    /// anchored one settles its position on the next.
    pub(super) fn find(
        &mut self,
        text: &str,
        nth: usize,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) -> egui::Rect {
        self.find_by(&|label| label.starts_with(text), text, nth, draw)
    }

    fn find_by(
        &mut self,
        matches: &dyn Fn(&str) -> bool,
        text: &str,
        nth: usize,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) -> egui::Rect {
        let located = |labels: &[(String, egui::Rect)]| {
            labels
                .iter()
                .filter(|(label, _)| matches(label))
                .nth(nth)
                .map(|(_, rect)| *rect)
        };
        let mut previous = located(&self.labels);
        for _ in 0..5 {
            let current = located(self.frame(Vec::new(), draw));
            if current.is_some() && current == previous {
                return current.unwrap();
            }
            previous = current;
        }
        panic!("no settled `{text}` #{nth} drawn; drew {:?}", self.labels);
    }

    /// Click the `nth` painted text that is exactly `text`.
    pub(super) fn click_exact(
        &mut self,
        text: &str,
        nth: usize,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) {
        let rect = self.find_by(&|label| label == text, text, nth, draw);
        self.slide_to(rect.center(), draw);
        self.press(egui::PointerButton::Primary, draw);
    }

    /// Right-click the first painted text that is exactly `text`.
    pub(super) fn right_click_exact(&mut self, text: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        let rect = self.find_by(&|label| label == text, text, 0, draw);
        self.slide_to(rect.center(), draw);
        self.press(egui::PointerButton::Secondary, draw);
    }

    /// Click the value widget on the row labelled exactly `label`, which a
    /// property row right-aligns against the panel's edge.
    pub(super) fn click_value_of(&mut self, label: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        let rect = self.find_by(&|text| text == label, label, 0, draw);
        self.slide_to(egui::pos2(self.value_x, rect.center().y), draw);
        self.press(egui::PointerButton::Primary, draw);
    }

    /// Type over the value on the row labelled `label` and press Enter.
    pub(super) fn enter_value_of(
        &mut self,
        label: &str,
        text: &str,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) {
        self.click_value_of(label, draw);
        self.replace_text(text, draw);
        self.key(egui::Key::Enter, egui::Modifiers::NONE, draw);
        self.frame(Vec::new(), draw);
    }

    /// Move onto `target` over three frames.
    pub(super) fn slide_to(&mut self, target: egui::Pos2, draw: &mut dyn FnMut(&mut egui::Ui)) {
        let start = self.pointer;
        for step in 1..=3 {
            let at = start + (target - start) * (step as f32 / 3.0);
            self.pointer = at;
            self.frame(vec![egui::Event::PointerMoved(at)], draw);
        }
    }

    /// Press and release at the pointer, then draw a frame for the result.
    pub(super) fn press(
        &mut self,
        button: egui::PointerButton,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) {
        let pos = self.pointer;
        let event = |pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![event(true)], draw);
        self.frame(vec![event(false)], draw);
        self.frame(Vec::new(), draw);
    }

    /// Click the `nth` painted text starting with `text`.
    pub(super) fn click_nth(
        &mut self,
        text: &str,
        nth: usize,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) {
        let rect = self.find(text, nth, draw);
        self.slide_to(rect.center(), draw);
        self.press(egui::PointerButton::Primary, draw);
    }

    pub(super) fn click(&mut self, text: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        self.click_nth(text, 0, draw);
    }

    /// Click at `pos`, which need not be on any text.
    pub(super) fn click_at(&mut self, pos: egui::Pos2, draw: &mut dyn FnMut(&mut egui::Ui)) {
        self.slide_to(pos, draw);
        self.press(egui::PointerButton::Primary, draw);
    }

    /// Type `text` into whatever has focus.
    pub(super) fn type_text(&mut self, text: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        self.frame(vec![egui::Event::Text(text.to_owned())], draw);
    }

    pub(super) fn key(
        &mut self,
        key: egui::Key,
        modifiers: egui::Modifiers,
        draw: &mut dyn FnMut(&mut egui::Ui),
    ) {
        self.frame(
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            draw,
        );
    }

    /// Select everything in the focused text box and replace it with `text`.
    pub(super) fn replace_text(&mut self, text: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        self.key(egui::Key::A, egui::Modifiers::COMMAND, draw);
        self.type_text(text, draw);
    }
}

#[test]
fn the_synthetic_install_mounts_and_decodes_every_property() {
    let install = SyntheticInstall::new();
    assert_eq!(install.world.packages().len(), 2);
    let document = install.document(THING);
    assert_eq!(document.package, THING);
    assert_eq!(document.exports[0].object, "Thing");
    assert_eq!(
        document.exports[0].class.as_deref(),
        Some(synthetic_class_key().as_str())
    );
    let mut header = document.header.clone();
    let expected = synthetic_thing_block(
        install.world.usmap(),
        FName::new(header.name_map.store("Rocket").index(), 0, "Rocket"),
    );
    assert_eq!(header.name_map.len(), document.header.name_map.len());
    assert!(
        first_block(&document).semantic_eq(&expected),
        "the package reads back as it was written: {:#?}",
        first_block(&document)
    );
    assert!(install.document(OTHER).exports[0].decoded.is_ok());
}
