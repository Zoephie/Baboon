use super::*;
use blam_tags::iostore::object::value::FName;
use blam_tags::iostore::package::name_map::{EMappedNameType, FNameMap};

/// The property editor reads each row's type out of one flattened schema
/// instead of asking the engine per row. Same answer, for every slot of
/// every class in the bundled schema, including a slot whose static-array
/// index does not match.
#[test]
fn declared_slot_types_match_the_engine_for_every_slot() {
    use blam_tags::iostore::object::value::SchemaSlot;
    let usmap = Usmap::meteorite().unwrap();
    let mut compared = 0usize;
    for class in &usmap.structs {
        let Ok(schema) =
            blam_tags::iostore::object::block::flattened_schema(&class.name, &usmap)
        else {
            continue;
        };
        for (index, (_, array_index, _)) in schema.iter().enumerate() {
            for array_index in [*array_index, array_index.wrapping_add(1)] {
                let slot = SchemaSlot {
                    index: index as u32,
                    array_index,
                    zero_masked: false,
                };
                assert_eq!(
                    declared_slot_type(&schema, slot),
                    property_type_for_slot(&class.name, slot, &usmap)
                        .ok()
                        .as_ref(),
                    "{}[{index}]",
                    class.name
                );
                compared += 1;
            }
        }
        // The omitted-property menu lists the same slots the engine's
        // catalog does.
        let catalog =
            blam_tags::iostore::object::edit::editable_schema_slots(&class.name, &usmap)
                .unwrap();
        assert_eq!(catalog.len(), schema.len());
        for ((name, slot, ty), (property, array_index, _)) in catalog.iter().zip(&schema) {
            assert_eq!(
                (name.as_str(), slot.array_index, ty),
                (property.name.as_str(), *array_index, &property.ty)
            );
        }
    }
    assert!(compared > 10_000, "compared only {compared} slots");
}

/// A position remembered for one enum list is checked before it is used.
#[test]
fn a_stale_enum_position_still_finds_the_enum() {
    let mut usmap = Usmap::meteorite().unwrap();
    let name = usmap.enums[0].name.clone();
    assert_eq!(usmap_enum(&usmap, &name).unwrap().name, name);
    let last = usmap.enums.len() - 1;
    usmap.enums.swap(0, last);
    assert_eq!(usmap_enum(&usmap, &name).unwrap().name, name);
    assert!(usmap_enum(&usmap, "NoSuchEnumAnywhere").is_none());
}

#[test]
fn chimp_scalar_property_row_does_not_consume_the_scroll_viewport() {
    let context = egui::Context::default();
    let mut value = 0_i64;
    let mut row_height = None;
    let _ = crate::app::run_ui_test(
        &context,
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_200.0, 800.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let row = ui.horizontal_top(|ui| {
                        ui.set_min_height(24.0);
                        ui.label("NumReplicatedProperties");
                        chimp_property_value_cell(ui, |ui| {
                            ui.add(egui::DragValue::new(&mut value));
                        });
                    });
                    row_height = Some(row.response.rect.height());
                });
            });
        },
    );

    let row_height = row_height.expect("the property row was rendered");
    assert!(
        row_height <= 40.0,
        "a scalar property row expanded to {row_height}px"
    );
}

/// Draw one FName box for a frame of `events`.
fn frame(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    value: &mut FName,
    names: &mut FNameMap,
) -> bool {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::new(400.0, 100.0),
        )),
        events,
        ..Default::default()
    };
    let mut changed = false;
    let _ = crate::app::run_ui_test(&ctx, input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            changed |= draw_chimp_fname(ui, "Name", value, names);
        });
    });
    changed
}

/// Typing a new name must add one name to the package, not one per
/// keystroke: every name interned is written into the saved package.
#[test]
fn typing_an_fname_interns_one_name_on_commit() {
    let ctx = egui::Context::default();
    let mut names =
        FNameMap::create_from_names(EMappedNameType::Package, vec!["None".to_owned()]);
    let mut value = FName::new(0, 0, "None");
    let before = names.len();

    // Click across the row until the text box has focus.
    for step in 0..40 {
        let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &ctx,
            vec![
                egui::Event::PointerMoved(pointer),
                click(true),
                click(false),
            ],
            &mut value,
            &mut names,
        );
        if ctx.memory(|memory| memory.focused()).is_some() {
            break;
        }
    }
    assert!(
        ctx.memory(|memory| memory.focused()).is_some(),
        "the box never took focus"
    );

    let key = |key, modifiers| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let select_all = egui::Modifiers {
        command: true,
        ..Default::default()
    };
    frame(
        &ctx,
        vec![key(egui::Key::A, select_all)],
        &mut value,
        &mut names,
    );
    let mut changes = 0;
    for letter in "Rocket".chars() {
        if frame(
            &ctx,
            vec![egui::Event::Text(letter.to_string())],
            &mut value,
            &mut names,
        ) {
            changes += 1;
        }
    }
    assert_eq!(names.len(), before, "nothing is interned while typing");
    if frame(
        &ctx,
        vec![key(egui::Key::Enter, Default::default())],
        &mut value,
        &mut names,
    ) {
        changes += 1;
    }

    assert_eq!(changes, 1, "one committed change");
    assert_eq!(value.to_string(), "Rocket");
    assert_eq!(
        names.len(),
        before + 1,
        "exactly one new name: {:?}",
        names.names()
    );
}

/// The synthetic `Thing`, its schema, and whether the editor has
/// reported a change on any frame since.
struct Editor {
    install: SyntheticInstall,
    document: ChimpDocument,
    usmap: Usmap,
    changed: bool,
    frames: Frames,
}

impl Editor {
    fn new() -> Self {
        let install = SyntheticInstall::new();
        let document = install.document(THING);
        Self {
            install,
            document,
            usmap: synthetic_usmap(),
            changed: false,
            frames: Frames::new(),
        }
    }

    /// Run `act` against the editor's frames, then report and clear
    /// whether any frame reported a change.
    fn act(&mut self, act: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut egui::Ui))) -> bool {
        let Self {
            document,
            usmap,
            changed,
            frames,
            ..
        } = self;
        let mut draw = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                *changed |= draw_chimp_export_editor(ui, document, usmap);
            });
        };
        act(frames, &mut draw);
        std::mem::take(changed)
    }

    fn value(&self, property: &str) -> PropValue {
        first_value(&self.document, property).clone()
    }

    fn shows(&self, text: &str) -> bool {
        self.frames.shows(text)
    }
}

fn ints(value: &PropValue) -> Vec<i64> {
    value
        .as_sequence()
        .unwrap_or_else(|| panic!("{value:?} is a sequence"))
        .iter()
        .map(|item| match item {
            PropValue::Int(value) => *value,
            other => panic!("{other:?}"),
        })
        .collect()
}

/// Every row of the synthetic class draws, labelled by name, with the
/// one omitted property offered and nothing reported changed.
#[test]
fn every_property_row_draws_with_its_value() {
    let mut editor = Editor::new();
    let changed = editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
        frames.frame(Vec::new(), draw);
    });
    assert!(!changed, "drawing is not editing");
    assert!(editor.shows("Thing"));
    assert!(editor.shows(&synthetic_class_key()));
    for name in SYNTHETIC_PROPERTIES {
        assert_eq!(
            editor.frames.labels.iter().any(|(label, _)| label == name),
            name != "Spare",
            "{name}"
        );
    }
    for text in [
        "Add omitted property… (1)",
        "Warthog",
        "Rocket",
        "Array (3)",
        "Map (1)",
        "Set (2)",
        "Struct (2 properties)",
        "Object -1",
        "Set value",
    ] {
        assert!(editor.shows(text), "{text}");
    }
    let (_, values) = synthetic_enum(&editor.usmap);
    assert!(editor.shows(&format!("{} ({})", values[1].1, values[1].0)));
}

/// Scalars edit in place: bool, int (clamped to its declared width),
/// float, string, object index.
#[test]
fn scalar_rows_edit_their_values() {
    let mut editor = Editor::new();
    assert!(editor.act(|frames, draw| frames.click_value_of("Enabled", draw)));
    assert!(matches!(editor.value("Enabled"), PropValue::Bool(false)));

    assert!(editor.act(|frames, draw| frames.enter_value_of("Count", "42", draw)));
    assert!(matches!(editor.value("Count"), PropValue::Int(42)));
    assert!(editor.act(|frames, draw| frames.enter_value_of("Count", "99999999999", draw)));
    assert!(
        matches!(editor.value("Count"), PropValue::Int(value) if value == i32::MAX as i64),
        "an Int is clamped to 32 bits: {:?}",
        editor.value("Count")
    );

    assert!(editor.act(|frames, draw| frames.enter_value_of("Scale", "2.25", draw)));
    assert!(matches!(editor.value("Scale"), PropValue::Float(value) if value == 2.25));

    assert!(editor.act(|frames, draw| frames.enter_value_of("Label", "Scorpion", draw)));
    assert!(
        matches!(editor.value("Label"), PropValue::Str(value) if value.as_str() == "Scorpion")
    );

    assert!(editor.act(|frames, draw| frames.enter_value_of("Target", "-2", draw)));
    assert!(matches!(editor.value("Target"), PropValue::Object(-2)));
}

/// A name commits once, on Enter, interning the new text into the
/// package's name map.
#[test]
fn a_name_row_interns_its_text_on_commit() {
    let mut editor = Editor::new();
    let before = editor.document.header.name_map.len();
    assert!(editor.act(|frames, draw| frames.enter_value_of("Tag", "Comet", draw)));
    let PropValue::Name(name) = editor.value("Tag") else {
        panic!("Tag is a name");
    };
    assert_eq!(name.as_str(), "Comet");
    assert_eq!(editor.document.header.name_map.len(), before + 1);
    assert_eq!(
        editor.document.header.name_map.names()[name.index as usize],
        "Comet"
    );
}

/// An enum row is a list of the enum's names, and choosing one stores
/// its value.
#[test]
fn an_enum_row_chooses_by_name() {
    let mut editor = Editor::new();
    let (_, values) = synthetic_enum(&editor.usmap);
    let selected = format!("{} ({})", values[1].1, values[1].0);
    let pick = values[2].1.clone();
    assert!(editor.act(|frames, draw| {
        frames.click(&selected, draw);
        frames.click_exact(&pick, 0, draw);
    }));
    assert!(
        matches!(editor.value("Mode"), PropValue::Int(value) if value == values[2].0 as i64)
    );
}

/// An array's rows move, duplicate, remove and append.
#[test]
fn an_array_row_reorders_duplicates_removes_and_appends() {
    let mut editor = Editor::new();
    assert!(!editor.act(|frames, draw| frames.click("Array (3)", draw)));
    assert!(editor.act(|frames, draw| frames.click_nth("Remove", 1, draw)));
    assert_eq!(ints(&editor.value("Values")), [10, 30]);
    assert!(editor.act(|frames, draw| frames.click_nth("↑", 1, draw)));
    assert_eq!(ints(&editor.value("Values")), [30, 10]);
    assert!(editor.act(|frames, draw| frames.click_nth("↓", 0, draw)));
    assert_eq!(ints(&editor.value("Values")), [10, 30]);
    assert!(editor.act(|frames, draw| frames.click_nth("Duplicate", 0, draw)));
    assert_eq!(ints(&editor.value("Values")), [10, 10, 30]);
    assert!(editor.act(|frames, draw| frames.click("+ Add", draw)));
    assert_eq!(ints(&editor.value("Values")), [10, 10, 30, 0]);
    // Moving the first up is not a change.
    assert!(!editor.act(|frames, draw| frames.click_nth("↑", 0, draw)));
    assert_eq!(ints(&editor.value("Values")), [10, 10, 30, 0]);
}

/// A set refuses an edit that would duplicate an element.
#[test]
fn a_set_row_refuses_duplicates() {
    let mut editor = Editor::new();
    editor.act(|frames, draw| frames.click("Set (2)", draw));
    assert!(!editor.act(|frames, draw| frames.click_nth("Duplicate", 0, draw)));
    assert_eq!(ints(&editor.value("Unique")), [4, 5]);
    assert!(editor.act(|frames, draw| frames.click("+ Add", draw)));
    assert_eq!(ints(&editor.value("Unique")), [4, 5, 0]);
    assert!(!editor.act(|frames, draw| frames.click("+ Add", draw)));
    assert_eq!(ints(&editor.value("Unique")), [4, 5, 0]);
}

/// A map appends a default pair, and refuses a second: the keys would
/// collide.
#[test]
fn a_map_row_appends_and_refuses_a_duplicate_key() {
    let mut editor = Editor::new();
    editor.act(|frames, draw| frames.click("Map (1)", draw));
    assert!(editor.shows("Key"));
    assert!(editor.act(|frames, draw| frames.click("+ Add", draw)));
    let PropValue::Map(pairs) = editor.value("Lookup") else {
        panic!("Lookup is a map");
    };
    assert!(matches!(
        pairs.as_slice(),
        [
            (PropValue::Int(1), PropValue::Int(100)),
            (PropValue::Int(0), PropValue::Int(0))
        ]
    ));
    assert!(!editor.act(|frames, draw| frames.click("+ Add", draw)));
    let PropValue::Map(pairs) = editor.value("Lookup") else {
        panic!("Lookup is a map");
    };
    assert_eq!(pairs.len(), 2, "a second zero key is refused");
}

/// An optional unsets to nothing, and an unset one takes its type's
/// default when set.
#[test]
fn an_optional_row_sets_and_unsets() {
    let mut editor = Editor::new();
    assert!(editor.act(|frames, draw| frames.click_exact("Unset", 0, draw)));
    assert!(matches!(editor.value("Maybe"), PropValue::Unset));
    // Both optionals are unset now; `Later` is the second.
    assert!(editor.act(|frames, draw| frames.click_nth("Set value", 1, draw)));
    assert!(matches!(editor.value("Later"), PropValue::Int(0)));
    assert!(matches!(editor.value("Maybe"), PropValue::Unset));
}

/// A property the package omits can be added from the schema, at its
/// slot, with its type's default.
#[test]
fn an_omitted_property_is_added_at_its_slot() {
    let mut editor = Editor::new();
    assert!(editor.act(|frames, draw| {
        frames.click("Add omitted property… (1)", draw);
        frames.click_exact("Spare", 0, draw);
    }));
    let block = first_block(&editor.document);
    let spare = block
        .entries
        .iter()
        .find(|entry| &*entry.name == "Spare")
        .unwrap();
    assert!(matches!(spare.value, PropValue::Int(0)));
    assert_eq!(spare.slot.map(|slot| slot.index), Some(12));
    editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(!editor.shows("Add omitted property"), "nothing left to add");
}

/// A nested struct draws its own rows under a header.
#[test]
fn a_struct_row_opens_onto_its_own_rows() {
    let mut editor = Editor::new();
    editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(!editor.shows("Depth"));
    editor.act(|frames, draw| frames.click("Struct (2 properties)", draw));
    assert!(editor.shows("Depth"));
    assert!(editor.shows("Weight"));
}

/// Edits made through the editor leave a package that rebuilds and
/// reads back as edited.
#[test]
fn an_edited_document_rebuilds_as_edited() {
    let mut editor = Editor::new();
    editor.act(|frames, draw| {
        frames.enter_value_of("Count", "42", draw);
        frames.click_exact("Unset", 0, draw);
        frames.click("Add omitted property… (1)", draw);
        frames.click_exact("Spare", 0, draw);
    });
    let world = &editor.install.world;
    let (bytes, _) = rebuild_chimp_document(world, &editor.document).unwrap();
    let reread =
        decode_chimp_document(world, editor.document.provider.clone(), bytes).unwrap();
    assert!(first_block(&reread).semantic_eq(first_block(&editor.document)));
    assert!(matches!(first_value(&reread, "Count"), PropValue::Int(42)));
    assert!(matches!(first_value(&reread, "Maybe"), PropValue::Unset));
    assert!(matches!(first_value(&reread, "Spare"), PropValue::Int(0)));
}

/// The editor's states for an export it cannot edit.
#[test]
fn unreadable_exports_say_why_and_edit_nothing() {
    let mut editor = Editor::new();
    editor.document.exports[0].decoded = Err("class could not be resolved".to_owned());
    assert!(!editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    }));
    assert!(editor.shows("class could not be resolved"));
    assert!(editor.shows("The raw export remains available"));

    editor.document.exports[0].decoded = Ok(Export::default());
    editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(editor.shows("This class has no reflected property block."));

    editor.document.exports.clear();
    editor.act(|frames, draw| {
        frames.frame(Vec::new(), draw);
    });
    assert!(editor.shows("This package has no exports."));
}

/// One value drawn alone, as its row would draw it with no declared type.
struct Lone {
    value: PropValue,
    names: FNameMap,
    usmap: Usmap,
    frames: Frames,
}

impl Lone {
    fn new(value: PropValue) -> Self {
        let mut lone = Self {
            value,
            names: FNameMap::create_from_names(
                EMappedNameType::Package,
                vec!["None".to_owned()],
            ),
            usmap: synthetic_usmap(),
            frames: Frames::new(),
        };
        assert!(!lone.act(|frames, draw| {
            frames.frame(Vec::new(), draw);
        }));
        lone
    }

    fn act(&mut self, act: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut egui::Ui))) -> bool {
        let Self {
            value,
            names,
            usmap,
            frames,
        } = self;
        let mut changed = false;
        let mut draw = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                changed |=
                    draw_chimp_value(ui, egui::Id::new("lone"), value, None, names, usmap, 0);
            });
        };
        act(frames, &mut draw);
        changed
    }

    fn click(&mut self, text: &str) -> bool {
        self.act(|frames, draw| frames.click(text, draw))
    }

    fn shows(&self, text: &str) -> bool {
        self.frames.shows(text)
    }
}

/// The hand-written shapes each open under one header and edit their
/// own fields.
#[test]
fn hand_written_values_edit_their_fields() {
    use chimp_hw::HandWritten as H;
    let open = |value: H| {
        let mut lone = Lone::new(PropValue::HandWritten(value));
        assert!(lone.shows("Typed Unreal structure"));
        assert!(!lone.click("Typed Unreal structure"));
        lone
    };

    let mut lone = open(H::PerQualityLevel(chimp_hw::PerQualityLevel {
        cooked: false,
        default_bits: 5,
        overrides: Vec::new(),
    }));
    assert!(lone.shows("Default bits: 5"));
    assert!(lone.click("Cooked"));
    lone.click("Overrides (0)");
    assert!(lone.click("+ Add"));
    let PropValue::HandWritten(H::PerQualityLevel(level)) = &lone.value else {
        unreachable!()
    };
    assert!(level.cooked);
    assert_eq!(level.overrides, [(0, 0)]);

    let mut lone = open(H::FontData(chimp_hw::FontData {
        font_face_asset: 0,
        inline_face: None,
        sub_face_index: 0,
    }));
    assert!(!lone.shows("Filename"));
    assert!(lone.click("Inline face"));
    assert!(lone.shows("Filename"));
    let PropValue::HandWritten(H::FontData(font)) = &lone.value else {
        unreachable!()
    };
    assert!(font.inline_face.is_some());

    let mut lone = open(H::MaterialLayersTree(chimp_hw::MaterialLayersTree {
        nodes: vec![[1, 2, 3, 4]],
        payloads: Vec::new(),
        root: 0,
    }));
    lone.click("Nodes (1)");
    assert!(lone.click("Remove"));
    let PropValue::HandWritten(H::MaterialLayersTree(tree)) = &lone.value else {
        unreachable!()
    };
    assert!(tree.nodes.is_empty());

    let mut lone = open(H::ShaderValueType(chimp_hw::ShaderValueType {
        kind: 1,
        is_dynamic_array: false,
        body: chimp_hw::ShaderValueTypeBody::Dimension {
            dimension: 2,
            counts: vec![3],
        },
    }));
    assert!(lone.click("Dynamic array"));
    lone.click("Counts (1)");
    assert!(lone.click("Duplicate"));
    let PropValue::HandWritten(H::ShaderValueType(shader)) = &lone.value else {
        unreachable!()
    };
    assert!(shader.is_dynamic_array);
    assert_eq!(
        shader.body,
        chimp_hw::ShaderValueTypeBody::Dimension {
            dimension: 2,
            counts: vec![3, 3]
        }
    );

    let mut lone = open(H::TimeWarpVariant(chimp_hw::TimeWarpVariant::Typed {
        kind: 1,
        object: None,
        payload: None,
    }));
    assert!(lone.click("Object"));
    let PropValue::HandWritten(H::TimeWarpVariant(chimp_hw::TimeWarpVariant::Typed {
        object,
        ..
    })) = &lone.value
    else {
        unreachable!()
    };
    assert_eq!(*object, Some(0));

    let node = chimp_hw::TreeNode {
        range_lower_kind: 0,
        range_lower: 0,
        range_upper_kind: 0,
        range_upper: 0,
        parent_children_handle: 0,
        parent_index: 0,
        children_id: 0,
        data_id: 0,
    };
    let mut lone = open(H::EvaluationTree(chimp_hw::EvaluationTree {
        root: node,
        child_entries: Vec::new(),
        child_nodes: Vec::new(),
        data_entries: Vec::new(),
        items: Vec::new(),
    }));
    lone.click("Child entries (0)");
    assert!(lone.click("+ Add"));
    let PropValue::HandWritten(H::EvaluationTree(tree)) = &lone.value else {
        unreachable!()
    };
    assert_eq!(
        tree.child_entries,
        [chimp_hw::TreeEntry {
            start: 0,
            size: 0,
            capacity: 0
        }]
    );
}

/// Native structs open under one header and edit their components.
#[test]
fn native_values_edit_their_components() {
    let mut lone = Lone::new(PropValue::Native(NativeStruct::PerPlatform {
        cooked: false,
        value: PerPlatformValue::Bool(false),
    }));
    assert!(!lone.click("Native struct"));
    assert!(lone.click("Cooked"));
    assert!(lone.click("Value"));
    assert!(matches!(
        lone.value,
        PropValue::Native(NativeStruct::PerPlatform {
            cooked: true,
            value: PerPlatformValue::Bool(true)
        })
    ));

    let mut lone = Lone::new(PropValue::Native(NativeStruct::Box3d {
        min: [0.0; 3],
        max: [1.0; 3],
        is_valid: 1,
    }));
    lone.click("Native struct");
    for label in ["Minimum", "Maximum", "Valid: 1", "0: 0", "2: 1"] {
        assert!(lone.shows(label), "{label}");
    }

    let mut lone = Lone::new(PropValue::Native(NativeStruct::Color([1, 2, 3, 4])));
    lone.click("Native struct");
    for label in ["B: 1", "G: 2", "R: 3", "A: 4"] {
        assert!(lone.shows(label), "{label}");
    }
}

/// Delegates and field paths edit as lists of their parts; raw bytes are
/// shown and left alone.
#[test]
fn delegate_field_path_and_raw_values() {
    let mut lone = Lone::new(PropValue::MulticastDelegate(Vec::new()));
    lone.click("Multicast delegate (0)");
    assert!(lone.click("+ Add"));
    assert!(matches!(&lone.value, PropValue::MulticastDelegate(list) if list.len() == 1));

    let mut lone = Lone::new(PropValue::FieldPath {
        path: Vec::new(),
        owner: 3,
    });
    assert!(lone.shows("Owner 3"));
    lone.click("Field path (0 segments)");
    assert!(lone.click("+ segment"));
    assert!(matches!(&lone.value, PropValue::FieldPath { path, .. } if path.len() == 1));
    assert!(lone.click("−"));
    assert!(matches!(&lone.value, PropValue::FieldPath { path, owner: 3 } if path.is_empty()));

    let lone = Lone::new(PropValue::Raw(vec![0; 7]));
    assert!(lone.shows("7 untyped bytes · preserved read-only"));
}
