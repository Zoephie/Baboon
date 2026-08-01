//! Foundation unit tests.
//! It owns test-only characterization and does not participate in runtime application behavior.

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_index_value_reads_all_variants() {
        use blam_tags::TagFieldData::*;
        assert_eq!(block_index_value(&CharBlockIndex(-1)), Some(-1));
        assert_eq!(block_index_value(&ShortBlockIndex(5)), Some(5));
        assert_eq!(block_index_value(&LongBlockIndex(42)), Some(42));
        assert_eq!(block_index_value(&CustomShortBlockIndex(3)), Some(3));
        // Non-block-index values don't read as a block index.
        assert_eq!(block_index_value(&LongInteger(7)), None);
    }

    /// Named components, and each one shown in degrees: euler angles are stored
    /// in radians, and the editor edits degrees like every other Halo tool.
    #[test]
    fn euler_angles_use_editable_named_components() {
        let parts = foundation_editable_component_parts(&TagFieldData::RealEulerAngles2d(
            blam_tags::math::RealEulerAngles2d {
                yaw: 45f32.to_radians(),
                pitch: (-90f32).to_radians(),
            },
        ))
        .unwrap();
        assert_eq!(
            parts,
            vec![
                ("yaw".to_owned(), "45".to_owned()),
                ("pitch".to_owned(), "-90".to_owned()),
            ]
        );

        let parts = foundation_editable_component_parts(&TagFieldData::RealEulerAngles3d(
            blam_tags::math::RealEulerAngles3d {
                yaw: (-0.65f32).to_radians(),
                pitch: 0.0,
                roll: 1.25f32.to_radians(),
            },
        ))
        .unwrap();
        assert_eq!(
            parts,
            vec![
                ("yaw".to_owned(), "-0.65".to_owned()),
                ("pitch".to_owned(), "0".to_owned()),
                ("roll".to_owned(), "1.25".to_owned()),
            ]
        );
    }

    #[test]
    fn parent_block_path_and_breadcrumb() {
        assert_eq!(
            parent_block_path("regions[0]/permutations").as_deref(),
            Some("regions")
        );
        assert_eq!(parent_block_path("a/b/c").as_deref(), Some("a/b"));
        assert_eq!(parent_block_path("a/b[3]").as_deref(), Some("a"));
        assert_eq!(parent_block_path("regions"), None);

        assert_eq!(
            breadcrumb_for_path("regions[0]/permutations"),
            "regions › permutations"
        );
        assert_eq!(breadcrumb_for_path("variants"), "variants");
    }

    #[test]
    fn ce_collision_geometry_reference_uses_loaded_game_extension() {
        let definitions_root = locate_definitions_root();
        let ce_names = TagNameIndex::load_game(&definitions_root, "haloce_mcc").unwrap();
        let h3_names = TagNameIndex::load_game(&definitions_root, "halo3_mcc").unwrap();
        let coll = parse_group_tag("coll").unwrap();
        let root = std::env::temp_dir().join(format!(
            "baboon_ce_collision_reference_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("weapons").join("assault rifle")).unwrap();
        let rel = "weapons\\assault rifle\\assault rifle";
        std::fs::write(
            root.join("weapons")
                .join("assault rifle")
                .join("assault rifle.model_collision_geometry"),
            [],
        )
        .unwrap();

        assert!(!reference_target_missing(
            Some(&ce_names),
            Some(&root),
            coll,
            rel
        ));
        assert!(reference_target_missing(
            Some(&h3_names),
            Some(&root),
            coll,
            rel
        ));
        assert!(reference_target_missing(None, Some(&root), coll, rel));
        std::fs::write(
            root.join("weapons")
                .join("assault rifle")
                .join("assault rifle.collision_model"),
            [],
        )
        .unwrap();
        assert!(!reference_target_missing(
            Some(&h3_names),
            Some(&root),
            coll,
            rel
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn with_test_edit_context(assertion: impl FnOnce(&mut FieldEditContext<'_>)) {
        let definitions_root = locate_definitions_root();
        let mut buffers = EditDrafts::default();
        let mut pending = Vec::new();
        let mut block_ops = Vec::new();
        let mut block_confirm = None;
        let mut open_request = None;
        let mut sound_play_request = None;
        let mut sound_extract_request = None;
        let mut tool_import = None;
        let mut bitmap_reimport = None;
        let mut shader_ops = Vec::new();
        let mut shader_param_ops = Vec::new();
        let mut h2_shader_param_ops = Vec::new();
        let mut function_data_ops = Vec::new();
        let mut model_variant_ops = Vec::new();
        let mut color_request = None;
        let mut function_request = None;
        let mut block_clip_request = None;
        let mut tsv_paste_request = None;
        let mut tag_reference_picker = None;
        let edit = FieldEditContext {
            expand_all: None,
            nested_default: NestedDefault::default(),
            view_scope: "test",
            tag_key: "test",
            group_tag: parse_group_tag("jpt!").unwrap(),
            root: None,
            game: Some("halo3_mcc"),
            definitions_root: Some(definitions_root.as_path()),
            names: None,
            tags_root: None,
            tag_reference_catalog: None,
            tag_reference_picker: &mut tag_reference_picker,
            status: None,
            editable: true,
            show_block_sizes: false,
            buffers: &mut buffers,
            pending: &mut pending,
            block_ops: &mut block_ops,
            block_confirm: &mut block_confirm,
            open_request: &mut open_request,
            sound_play_request: &mut sound_play_request,
            sound_status: None,
            sound_volume: 1.0,
            sound_extract_request: &mut sound_extract_request,
            sound_language: None,
            ce_sound: None,
            ce_sound_ref_request: &mut None,
            ce_paks_root: None,
            tool_import: &mut tool_import,
            bitmap_reimport: &mut bitmap_reimport,
            shader_ops: &mut shader_ops,
            shader_param_ops: &mut shader_param_ops,
            h2_shader_param_ops: &mut h2_shader_param_ops,
            function_data_ops: &mut function_data_ops,
            model_variant_ops: &mut model_variant_ops,
            color_request: &mut color_request,
            function_request: &mut function_request,
            block_clipboard: None,
            docs: None,
            tsv_paste_request: &mut tsv_paste_request,
            block_clip_request: &mut block_clip_request,
            field_filter: None,
            field_nav: None,
        };
        let mut edit = edit;
        assertion(&mut edit);
    }

    #[test]
    fn screen_flash_explanation_fallback_present() {
        let text = known_explanation_text("screen flash").unwrap();
        assert!(text.contains("There are seven screen flash types"));
        assert!(text.contains("LIGHTEN"));

        assert!(text.contains("DST'"));
    }

    #[test]
    fn internal_placeholder_titles_do_not_leak() {
        assert_eq!(
            inline_function_label("dirty whore", "rumble/low frequency rumble"),
            "function"
        );
        assert_eq!(
            visible_container_title("dirty whore", "rumble/low frequency rumble"),
            "low frequency rumble"
        );
        assert!(is_internal_schema_marker_name("HIDE_GROUP_ID"));
        assert!(is_internal_schema_marker_name("END_HIDE_GROUP_ID"));
        assert!(is_internal_schema_marker_name("whore function"));
    }

    #[test]
    fn legacy_mapping_function_bytes_build_inline_function_view() {
        let mut raw = vec![0; 20];
        raw[0] = 4;
        raw[1] = 1;
        raw[2] = 5;
        raw[4..8].copy_from_slice(&0.8f32.to_le_bytes());
        raw[8..12].copy_from_slice(&0.4f32.to_le_bytes());
        raw[12..16].copy_from_slice(&0.25f32.to_le_bytes());

        let view = legacy_mapping_function_view(&raw).expect("legacy data should parse");

        assert!(view.h2_legacy.is_some());
        assert_eq!(view.data_bytes(), raw);
    }

    #[test]
    fn tag_reference_picker_paths_must_be_under_tags_root() {
        let tags_root = PathBuf::from("tags");
        let picked = tags_root
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.bitmap");

        assert_eq!(
            tag_reference_relative_path_with_extension(&picked, &tags_root).unwrap(),
            r"objects\characters\brute\bitmaps\mask.bitmap"
        );

        let outside = PathBuf::from("data")
            .join("objects")
            .join("characters")
            .join("brute")
            .join("bitmaps")
            .join("mask.tif");
        assert_eq!(
            tag_reference_relative_path_with_extension(&outside, &tags_root).unwrap_err(),
            "Selected file must be inside the tags folder"
        );
    }

    #[test]
    fn tag_reference_group_validator_allows_none_and_matching_group() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let empty = TagReferenceData {
            group_tag_and_name: None,
        };
        let matching = TagReferenceData {
            group_tag_and_name: Some((render_model, r"objects\foo\foo".to_owned())),
        };
        let mismatched = TagReferenceData {
            group_tag_and_name: Some((collision_model, r"objects\foo\foo".to_owned())),
        };

        assert!(tag_reference_group_allowed(&empty, render_model));
        assert!(tag_reference_group_allowed(&matching, render_model));
        assert!(!tag_reference_group_allowed(&mismatched, render_model));
    }

    #[test]
    fn empty_schema_constrained_reference_keeps_its_required_group() {
        let structure_design = parse_group_tag("sddt").unwrap();
        let meta = FieldDisplayMeta {
            label: "structure design".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: vec![structure_design],
            read_only: false,
            advanced: false,
        };

        assert_eq!(
            tag_reference_required_group(&meta, None),
            Some(structure_design)
        );
    }

    #[test]
    fn catalog_picker_uses_schema_then_current_group_then_all_groups() {
        let animation = parse_group_tag("jmad").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();
        let weapon = parse_group_tag("weap").unwrap();
        let target = (weapon, r"objects\weapons\rifle\rifle".to_owned());
        let meta = |allowed| FieldDisplayMeta {
            label: "reference".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
        };

        let single = meta(vec![animation]);
        assert!(tag_reference_catalog_group_allowed(
            &single,
            Some(&target),
            animation,
            false,
        ));
        assert!(!tag_reference_catalog_group_allowed(
            &single,
            Some(&target),
            weapon,
            false,
        ));

        let multiple = meta(vec![biped, vehicle]);
        assert!(tag_reference_catalog_group_allowed(
            &multiple,
            Some(&target),
            biped,
            false,
        ));
        assert!(tag_reference_catalog_group_allowed(
            &multiple,
            Some(&target),
            vehicle,
            false,
        ));
        assert!(!tag_reference_catalog_group_allowed(
            &multiple,
            Some(&target),
            weapon,
            false,
        ));

        let unconstrained = meta(Vec::new());
        assert!(tag_reference_catalog_group_allowed(
            &unconstrained,
            Some(&target),
            weapon,
            false,
        ));
        assert!(!tag_reference_catalog_group_allowed(
            &unconstrained,
            Some(&target),
            animation,
            false,
        ));
        assert!(tag_reference_catalog_group_allowed(
            &unconstrained,
            None,
            animation,
            false,
        ));
        assert!(tag_reference_catalog_group_allowed(
            &unconstrained,
            None,
            weapon,
            false,
        ));
        assert!(tag_reference_catalog_group_allowed(
            &single,
            Some(&target),
            weapon,
            true,
        ));
    }

    #[test]
    fn catalog_picker_searches_names_and_groups_not_parent_folders() {
        let model = parse_group_tag("mode").unwrap();
        let weapon = parse_group_tag("weap").unwrap();
        let parent_only = TagEntry {
            key: "ublock:model".to_owned(),
            display_path: "objects/characters/elite/garbage/hg_arm.render_model".to_owned(),
            group_tag: model,
            group_name: Some("render_model".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/objects/characters/elite/garbage/hg_arm-render_model.ubulk"
                    .to_owned(),
            },
        };
        let rifle = TagEntry {
            key: "ublock:weapon".to_owned(),
            display_path: "objects/weapons/rifle/battle_rifle.weapon".to_owned(),
            group_tag: weapon,
            group_name: Some("weapon".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/objects/weapons/rifle/battle_rifle-weapon.ubulk".to_owned(),
            },
        };

        assert!(!tag_reference_catalog_entry_matches(&parent_only, "elite"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "rifle"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "weapon"));
        assert!(tag_reference_catalog_entry_matches(&rifle, "WEAP"));
    }

    #[test]
    fn catalog_picker_is_exposed_only_for_iostore_sources() {
        let container_source = LoadedSourceData {
            label: "Campaign Evolved".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: PathBuf::from("C:/CampaignEvolved/Content/Paks"),
                containers: Vec::new(),
                index: std::sync::Arc::new(crate::source::ContainerTagIndex::default()),
                packages: std::sync::Arc::new(crate::source::ContainerPackageIndex::default()),
                shipped: std::sync::Arc::new(crate::source::ShippedTagIndex::default()),
            },
            names: TagNameIndex::default(),
            game: Some("haloce_evolved".to_owned()),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        };
        let catalog = tag_reference_catalog_for_source(&container_source, true)
            .expect("container source should expose a catalog");
        assert!(catalog.expert_mode);

        let loose_source = LoadedSourceData {
            label: "H3EK".to_owned(),
            source: TagSource::LooseFolder {
                root: PathBuf::from("C:/H3EK/tags"),
                game: Some("halo3_mcc".to_owned()),
                definitions_root: PathBuf::from("C:/H3EK/definitions"),
            },
            names: TagNameIndex::default(),
            game: Some("halo3_mcc".to_owned()),
            entries: Vec::new(),
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        };
        assert!(tag_reference_catalog_for_source(&loose_source, true).is_none());
    }

    #[test]
    fn picker_resolves_structure_design_from_loaded_game_definitions() {
        let definitions_root = locate_definitions_root();
        for game in ["halo3_mcc", "halo3odst_mcc", "haloreach_mcc", "halo4_mcc"] {
            let names = TagNameIndex::load_game(&definitions_root, game).unwrap();
            let structure_design = parse_group_tag("sddt").unwrap();
            assert_eq!(
                tag_reference_group_for_extension(
                    "structure_design",
                    Some(structure_design),
                    Some(&names),
                )
                .unwrap(),
                structure_design,
                "{game}"
            );
        }
    }

    #[test]
    fn tag_reference_value_icon_prefers_typed_or_committed_group() {
        let render_model = parse_group_tag("mode").unwrap();
        let collision_model = parse_group_tag("coll").unwrap();
        let biped = parse_group_tag("bipd").unwrap();
        let vehicle = parse_group_tag("vehi").unwrap();
        let bitmap = parse_group_tag("bitm").unwrap();
        let target = (collision_model, r"objects\foo\foo".to_owned());
        let meta = |allowed| FieldDisplayMeta {
            label: "reference".to_owned(),
            unit: None,
            range: None,
            help: None,
            tag_reference_allowed: allowed,
            read_only: false,
            advanced: false,
        };

        assert_eq!(
            tag_reference_value_icon_group(
                &meta(vec![render_model]),
                Some(&target),
                r"objects\foo\foo.bitmap"
            ),
            Some(bitmap)
        );
        assert_eq!(
            tag_reference_value_icon_group(
                &meta(vec![render_model]),
                Some(&target),
                r"objects\foo\foo"
            ),
            Some(collision_model)
        );
        assert_eq!(
            tag_reference_value_icon_group(&meta(vec![render_model]), None, "NONE"),
            Some(render_model)
        );
        assert_eq!(
            tag_reference_value_icon_group(&meta(vec![biped, vehicle]), None, "NONE"),
            None
        );
    }

    #[test]
    fn format_block_size_label_is_stable_and_human_readable() {
        assert_eq!(format_block_size_label(2, 36), "2 x 36 B = 72 B");
        assert_eq!(format_block_size_label(64, 36), "64 x 36 B = 2.2 KiB");
    }

    #[test]
    fn combo_scroll_next_index_clamps_and_uses_delta_direction() {
        assert_eq!(combo_scroll_next_index(1, 3, 1), Some(2));
        assert_eq!(combo_scroll_next_index(1, 3, 120), Some(2));
        assert_eq!(combo_scroll_next_index(1, 3, -1), Some(0));
        assert_eq!(combo_scroll_next_index(1, 3, -120), Some(0));
        assert_eq!(combo_scroll_next_index(0, 3, -1), None);
        assert_eq!(combo_scroll_next_index(2, 3, 1), None);
        assert_eq!(combo_scroll_next_index(0, 0, 1), None);
    }

    #[test]
    fn foundation_selected_width_reserves_only_current_header_cells() {
        assert_eq!(foundation_selected_width(1_000.0), 376.0);
        assert_eq!(foundation_selected_width(500.0), 120.0);
        assert_eq!(foundation_selected_width(2_000.0), 420.0);
    }

    #[test]
    fn semantic_short_index_target_names_cover_damage_sections() {
        let cases = [
            ("parent variant", Some("variants")),
            ("variant", Some("variants")),
            ("parent node", Some("nodes")),
            ("damage section", Some("damage sections")),
            ("indirect damage section", Some("damage sections")),
            ("runtime region index", None),
        ];

        for (field_name, expected) in cases {
            assert_eq!(semantic_short_index_target_key(field_name), expected);
        }
    }

    /// Expand/collapse-all is a direct instruction about the whole tag, so it
    /// has to win over the rules that otherwise decide a container's open
    /// state — the search filter's, and a reference jump forcing its target's
    /// ancestors open. Every container type resolves through here, so this is
    /// the one place that ordering is decided.
    #[test]
    fn expand_all_overrides_the_other_open_rules() {
        with_test_edit_context(|edit| {
            // Nothing asked for: the caller's own default stands.
            assert_eq!(edit.resolve_open("some/block", true), None);

            edit.expand_all = Some(true);
            assert_eq!(edit.resolve_open("some/block", false), Some(true));

            edit.expand_all = Some(false);
            assert_eq!(edit.resolve_open("some/block", true), Some(false));
        });
    }

    /// The preference adjusts each container's *default* rather than forcing
    /// its state, so a group the user has since opened or closed keeps their
    /// choice — egui only consults a default when it has nothing stored.
    #[test]
    fn nested_default_overrides_only_the_schema_default() {
        with_test_edit_context(|edit| {
            edit.nested_default = NestedDefault::Schema;
            assert!(edit.default_open(true));
            assert!(!edit.default_open(false));

            edit.nested_default = NestedDefault::Collapsed;
            assert!(!edit.default_open(true), "collapsed must close a section the schema opens");

            edit.nested_default = NestedDefault::Expanded;
            assert!(edit.default_open(false), "expanded must open a section the schema closes");

            // And it stays a default: nothing here forces an open state.
            assert_eq!(edit.resolve_open("some/block", true), None);
        });
    }

    /// The reported complaint, at the layer that causes it: a value the editor
    /// considers uneditable is *painted text* — there is no cursor to place in
    /// it, nothing to drag a selection across, and nothing to copy. A value in
    /// an editable context is a text box, which is what makes retyping a tag
    /// build's numbers somewhere else possible at all.
    ///
    /// Driven through real pointer input rather than by asking which branch was
    /// taken: what matters is whether clicking the cell puts a caret in it.
    #[test]
    fn only_an_editable_value_row_can_be_clicked_into() {
        assert!(
            click_across_value_row(true),
            "an editable value row must take a caret, or its text cannot be selected or copied"
        );
        assert!(
            !click_across_value_row(false),
            "a read-only row is painted text — nothing there can take focus"
        );
    }

    /// Render one `real_vector_3d` row (the `Point 0 / x y z` shape from the
    /// report) and click along it until something takes keyboard focus.
    /// Returns whether anything ever did.
    fn click_across_value_row(editable: bool) -> bool {
        let ctx = egui::Context::default();
        let mut tag =
            TagFile::new(crate::app::test_definition_path("halo4_mcc/camera_track.json")).unwrap();
        crate::app::add_block_element(&mut tag, "control points").unwrap();
        let mut focused = false;

        with_test_edit_context(|edit| {
            edit.editable = editable;
            // Sweep the row rather than trusting a hand-computed cell position:
            // the widths are layout details, and a click that lands on the label
            // by accident would report "not editable" for the wrong reason.
            for step in 0..90 {
                let pointer = egui::Pos2::new(step as f32 * 10.0, 12.0);
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(900.0, 200.0),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(pointer),
                        click(true),
                        click(false),
                    ],
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let field = tag
                            .root()
                            .field_path("control points[0]/position")
                            .expect("the control point's position field");
                        let value = field.value().expect("position has a value");
                        let meta = field_display_meta(field.name());
                        draw_foundation_value_row(
                            ui,
                            field,
                            &meta,
                            field.type_name(),
                            &value,
                            &TagNameIndex::default(),
                            0,
                            "control points[0]/position",
                            edit,
                            None,
                            None,
                            300.0,
                        );
                    });
                });
                if ctx.memory(|memory| memory.focused()).is_some() {
                    focused = true;
                    break;
                }
            }
        });

        focused
    }
}
