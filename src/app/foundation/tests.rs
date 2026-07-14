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

    fn with_test_edit_context(assertion: impl FnOnce(&FieldEditContext<'_>)) {
        let definitions_root = locate_definitions_root();
        let mut buffers = HashMap::new();
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
        let edit = FieldEditContext {
            view_scope: "test",
            tag_key: "test",
            group_tag: parse_group_tag("jpt!").unwrap(),
            root: None,
            game: Some("halo3_mcc"),
            definitions_root: Some(definitions_root.as_path()),
            names: None,
            tags_root: None,
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
        assertion(&edit);
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
}
