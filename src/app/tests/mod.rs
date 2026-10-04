use crate::app::controller::new_tag_output_path_from_dialog;

use super::*;

mod classic_h2;

/// Helper programs Baboon runs out of sight must go through
/// `background_command`, or a release build (a GUI-subsystem program)
/// flashes a console window for each one on Windows. `git` and `taskkill`
/// were launched with a bare `Command::new`. The flag cannot be observed
/// off Windows, so this checks the launches themselves.
#[test]
fn helper_programs_launch_through_background_command() {
    let files = [
        ("git_review/mod.rs", include_root_str!("src/app/git_review/mod.rs")),
        ("ui/tag_compare/mod.rs", include_root_str!("src/app/ui/tag_compare/mod.rs")),
        ("controller/terminal/mod.rs", include_root_str!("src/app/controller/terminal/mod.rs")),
        ("controller/updates/mod.rs", include_root_str!("src/app/controller/updates/mod.rs")),
    ];
    let mut bare = Vec::new();
    let mut routed = 0;
    for (file, text) in files {
        // Test code launches git to build fixtures; only what ships counts.
        let shipped = text.split("#[cfg(test)]").next().unwrap_or(text);
        for program in ["git", "taskkill", "powershell.exe", "cmd"] {
            for call in ["Command::new", "process::Command::new"] {
                if shipped.contains(&format!("{call}(\"{program}\")")) {
                    bare.push(format!("{file}: {program}"));
                }
            }
            routed += shipped.matches(&format!("background_command(\"{program}\")")).count();
        }
    }
    assert!(bare.is_empty(), "launched without CREATE_NO_WINDOW: {bare:?}");
    assert!(routed >= 10, "only {routed} launches found; the scan is not looking");
}

#[test]
fn strip_node_indices_drops_element_subscripts() {
    // Search-fields paths must be element-independent so a match resolves
    // regardless of which block element happens to be selected.
    assert_eq!(strip_node_indices("contact points"), "contact points");
    assert_eq!(
        strip_node_indices("contact points[0]/markers[12]"),
        "contact points/markers"
    );
    assert_eq!(strip_node_indices("unit/object"), "unit/object");
    assert_eq!(
        strip_node_indices("color#10/Mapping#5/Function Type#1"),
        "color/Mapping/Function Type"
    );
    assert_eq!(
        strip_node_indices("regions#3[0]/permutations#7[2]"),
        "regions/permutations"
    );
}

#[test]
fn strip_element_indices_keeps_ordinals_drops_subscripts() {
    // Block-clipboard compatibility must ignore which parent element is
    // selected (the `[N]` subscript) but keep the field ordinal (`#N`) so a
    // copied element pastes into the same block under a *different* parent
    // index, while distinct same-named sibling blocks stay separate.
    assert_eq!(
        strip_element_indices("damage sections#3[0]/instant responses#5"),
        "damage sections#3/instant responses#5"
    );
    assert_eq!(
        strip_element_indices("damage sections#3[1]/instant responses#5"),
        "damage sections#3/instant responses#5"
    );
    // The two paths above — same block, different parent index — match.
    assert_eq!(
        strip_element_indices("damage sections#3[0]/instant responses#5"),
        strip_element_indices("damage sections#3[7]/instant responses#5"),
    );
    assert_eq!(
        strip_element_indices("regions#3[0]/permutations#7[2]"),
        "regions#3/permutations#7"
    );
}

#[test]
fn clean_field_name_strips_every_markup_form() {
    // Display-label cleaning must strip all Guerilla markup so a segment
    // built from a field name never carries a path-grammar character. These
    // are the forms the compatibility sweep found across all 5 MCC kits.
    assert_eq!(clean_field_name("jump velocity"), "jump velocity");
    assert_eq!(clean_field_name("ambient color:[0,255]"), "ambient color");
    assert_eq!(
        clean_field_name("max sounds per tag [1,16]"),
        "max sounds per tag"
    );
    assert_eq!(clean_field_name("shader flags*"), "shader flags");
    assert_eq!(clean_field_name("activity!"), "activity");
    assert_eq!(clean_field_name("activity name^"), "activity name");
    assert_eq!(
        clean_field_name("acoustics{background sounds}*"),
        "acoustics"
    );
    assert_eq!(
        clean_field_name("bounds 480i&bounds 4x3 (640x480)"),
        "bounds 480i"
    );
    // H2 model_animation_graph dumper artifact `name|CODE`.
    assert_eq!(clean_field_name("animations|ABCDCC"), "animations");
}

#[test]
fn canonical_field_path_normalizes_names_paths_and_codes() {
    // Single names normalize to their clean form.
    assert_eq!(
        canonical_field_path("ambient color:[0,255]"),
        "ambient color"
    );
    // Multi-segment paths drop element indices AND field ordinals per segment,
    // cleaning each name — the form the conversion loss-detector relies on.
    assert_eq!(
        canonical_field_path("bitmaps#21[0]/signature#0"),
        "bitmaps/signature"
    );
    assert_eq!(
        canonical_field_path("resources#0/animations|ABCDCC#8"),
        "resources/animations"
    );
    assert_eq!(
        canonical_field_path(
            "new damage info#19[0]/damage sections#26[0]/instant responses#3[0]/response type#0"
        ),
        "new damage info/damage sections/instant responses/response type"
    );
    // clean_field_key is the lowercased canonical form.
    assert_eq!(clean_field_key("Shader Flags*"), "shader flags");
    assert_eq!(
        clean_field_key("Bitmaps#21[0]/Signature#0"),
        "bitmaps/signature"
    );
}

#[test]
fn resolvable_and_canonical_path_flavors_stay_consistent() {
    // The search-filter invariant (see `collect_visible_paths` /
    // `field_visible`): the canonical key a container is stored under equals
    // `strip_node_indices` of the resolvable render path. This only holds
    // because resolvable paths carry CLEAN names (a range hint left in the
    // name would strip-to a different key and break the filter).
    //
    // Simulate the two flavors for a markup-named field. `resolvable` is what
    // `append_field_path_for` emits (clean name + ordinal, plus a parent
    // element index); `canon` is what `collect_visible_paths` records.
    let seg = clean_field_name("max sounds per tag [1,16]"); // build-time clean
    let resolvable = format!("ambient color#5[0]/{seg}#9");
    let canon = format!("ambient color/{seg}");
    assert_eq!(strip_node_indices(&resolvable), canon);
    assert_eq!(canon, "ambient color/max sounds per tag");
}

#[test]
fn tag_ref_path_helpers() {
    // Null terminator stripped so the ref resolves on disk.
    assert_eq!(
        sanitize_ref_path("objects\\characters\\masterchief\\masterchief\u{0}"),
        "objects\\characters\\masterchief\\masterchief"
    );
    // tool source dir is the parent of the tag path.
    assert_eq!(
        model_source_dir("objects\\characters\\masterchief\\masterchief"),
        "objects\\characters\\masterchief"
    );
    assert_eq!(model_source_dir("solo"), "solo");
}

#[test]
fn engine_emblems_are_separate_from_game_banners() {
    assert!(get_game_emblem_bytes("haloce_mcc").is_some());
    assert!(get_game_emblem_bytes("halo2_mcc").is_some());
    assert!(get_game_emblem_bytes("haloce_evolved").is_some());
    assert_ne!(
        get_game_emblem_bytes("halo2_mcc"),
        get_game_emblem_bytes("halo2amp_mcc")
    );
    assert!(get_game_emblem_bytes("unknown").is_none());
    assert_ne!(
        get_game_emblem_bytes("haloce_mcc"),
        Some(get_game_banner_bytes("haloce_mcc"))
    );
    assert_ne!(
        get_game_banner_bytes("haloce_evolved"),
        get_game_banner_bytes("haloce_mcc")
    );
    assert_ne!(
        get_game_emblem_bytes("haloce_evolved"),
        get_game_emblem_bytes("haloce_mcc")
    );
}

#[test]
fn color_channel_parsing_matches_swatch_output() {
    // The color picker writes "r, g, b" / "a, r, g, b"; the field parser
    // must accept exactly that and reject the wrong channel count.
    let [r, g, b] = parse_color_channels::<3>("0.14902, 0.662745, 0.145098").unwrap();
    assert!((r - 0.14902).abs() < 1e-6);
    assert!((g - 0.662745).abs() < 1e-6);
    assert!((b - 0.145098).abs() < 1e-6);

    let [a, r, _, _] = parse_color_channels::<4>("0.5, 1.0, 0.0, 0.25").unwrap();
    assert_eq!(a, 0.5);
    assert_eq!(r, 1.0);
    assert_eq!(
        parse_rgb_or_argb_color_channels("1.0, 0.0, 0.25").unwrap(),
        (1.0, 1.0, 0.0, 0.25)
    );
    assert_eq!(
        parse_rgb_or_argb_color_channels("0.5, 1.0, 0.0, 0.25").unwrap(),
        (0.5, 1.0, 0.0, 0.25)
    );
    assert_eq!(color_float_to_u8(1.0), 255);
    assert_eq!(color_float_to_u8(0.0), 0);
    assert_eq!(color_float_to_u8(0.5), 128);

    assert!(parse_color_channels::<3>("0.1, 0.2").is_err());
    assert!(parse_color_channels::<3>("0.1, 0.2, x").is_err());
    assert!(parse_rgb_or_argb_color_channels("0.1, 0.2").is_err());
}

#[test]
fn editable_color_hex_accepts_standard_rgb_codes() {
    assert_eq!(parse_rgb_hex("#FF8040").unwrap(), [255, 128, 64]);
    assert_eq!(parse_rgb_hex("00aaff").unwrap(), [0, 170, 255]);
    assert_eq!(format_rgb_hex(1.0, 0.5, 0.0), "#FF8000");
    assert!(parse_rgb_hex("#12345").is_err());
    assert!(parse_rgb_hex("#12XX56").is_err());
}

#[test]
fn baboon_palette_format_round_trips_custom_swatches() {
    let mut swatches = vec![None; CUSTOM_COLOR_SWATCH_COUNT];
    swatches[0] = Some(ColorPaletteSwatch::named([0, 0, 0, 255], "Black"));
    swatches[1] = Some(ColorPaletteSwatch::unnamed([255, 87, 51, 255]));
    swatches[3] = Some(ColorPaletteSwatch::unnamed([51, 255, 87, 128]));

    let encoded = encode_baboon_palette("My Custom Palette", &swatches);
    assert!(encoded.contains("# Baboon Colour Palette"));
    assert!(encoded.contains("# Name: My Custom Palette"));
    assert!(encoded.contains("#FF5733FF"));
    assert!(encoded.contains("#000000FF\tBlack"));
    assert!(encoded.contains("#empty"));

    let decoded = decode_baboon_palette(&encoded).unwrap();
    assert_eq!(decoded.len(), CUSTOM_COLOR_SWATCH_COUNT);
    assert_eq!(
        decoded[0],
        Some(ColorPaletteSwatch::named([0, 0, 0, 255], "Black"))
    );
    assert_eq!(
        decoded[1],
        Some(ColorPaletteSwatch::unnamed([255, 87, 51, 255]))
    );
    assert_eq!(decoded[2], None);
    assert_eq!(
        decoded[3],
        Some(ColorPaletteSwatch::unnamed([51, 255, 87, 128]))
    );
}

#[test]
fn bundled_default_palette_has_48_colors_and_16_empty_slots() {
    let swatches = default_color_swatches();
    assert_eq!(swatches.len(), CUSTOM_COLOR_SWATCH_COUNT);
    assert!(swatches[..48].iter().all(Option::is_some));
    assert!(swatches[48..].iter().all(Option::is_none));
}

#[test]
fn baboon_palette_load_pads_and_ignores_comments() {
    let decoded = decode_baboon_palette(
        "# Baboon Colour Palette\n# Name: Small\n# Comment\n#11223344\n#empty\n",
    )
    .unwrap();

    assert_eq!(decoded.len(), CUSTOM_COLOR_SWATCH_COUNT);
    assert_eq!(
        decoded[0],
        Some(ColorPaletteSwatch::unnamed([17, 34, 51, 68]))
    );
    assert_eq!(decoded[1], None);
    assert!(decoded[2..].iter().all(Option::is_none));
}

#[test]
fn halo3_color_preferences_load_by_slot_with_names_and_opaque_alpha() {
    let decoded = decode_halo3_color_preferences(
        "\u{feff}1,0,255,0,Green\n0,255,0,0,Red\n2,255,255,255,\n",
    )
    .unwrap();

    assert_eq!(decoded.len(), CUSTOM_COLOR_SWATCH_COUNT);
    assert_eq!(
        decoded[0],
        Some(ColorPaletteSwatch::named([255, 0, 0, 255], "Red"))
    );
    assert_eq!(
        decoded[1],
        Some(ColorPaletteSwatch::named([0, 255, 0, 255], "Green"))
    );
    assert_eq!(
        decoded[2],
        Some(ColorPaletteSwatch::unnamed([255, 255, 255, 255]))
    );
    assert_eq!(decoded[3], None);
}

#[test]
fn halo3_color_preferences_save_rgb_slots_and_names() {
    let mut swatches = vec![None; CUSTOM_COLOR_SWATCH_COUNT];
    swatches[0] = Some(ColorPaletteSwatch::named([255, 16, 0, 127], "Warm red"));
    swatches[2] = Some(ColorPaletteSwatch::unnamed([1, 2, 3, 255]));

    let encoded = encode_halo3_color_preferences(&swatches);
    assert_eq!(encoded, "0,255,16,0,Warm red\r\n2,1,2,3,\r\n");

    let decoded = decode_halo3_color_preferences(&encoded).unwrap();
    assert_eq!(
        decoded[0],
        Some(ColorPaletteSwatch::named([255, 16, 0, 255], "Warm red"))
    );
    assert_eq!(
        decoded[2],
        Some(ColorPaletteSwatch::unnamed([1, 2, 3, 255]))
    );
    assert_eq!(decoded[1], None);
}

#[test]
fn hex_blob_ferry_round_trips() {
    // The shader function editor ships edited blobs through the string
    // edit channel as hex. encode_hex -> decode_hex must be lossless for
    // arbitrary bytes, or saved shader functions would corrupt.
    let blob: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
    let encoded = encode_hex(&blob);
    assert_eq!(encoded.len(), blob.len() * 2);
    let decoded = decode_hex(&encoded).expect("decode hex");
    assert_eq!(decoded, blob);
    // Odd-length / invalid input is rejected, not silently truncated.
    assert!(decode_hex("abc").is_err());
    assert!(decode_hex("zz").is_err());
}

#[test]
fn copied_classic_definitions_load_halo2_shader_layout() {
    let schema_path = locate_definitions_root()
        .join("halo2_mcc")
        .join("shader.json");
    assert!(schema_path.is_file());
    TagFile::new(&schema_path).expect("copied halo2 shader schema loads");
    let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::Halo2)
        .expect("copied halo2 meta loads");
    assert_eq!(names.name_for(u32::from_be_bytes(*b"shad")), Some("shader"));
}

#[test]
fn copied_definitions_include_all_known_games() {
    let root = locate_definitions_root();
    for game in [
        "haloce_mcc",
        "halo2_mcc",
        "halo2amp_mcc",
        "halo3_mcc",
        "halo3odst_mcc",
        "haloreach_mcc",
        "halo4_mcc",
    ] {
        assert!(
            root.join(game).join("_meta.json").is_file(),
            "missing copied definitions for {game}"
        );
    }
}

#[test]
fn bitmap_channel_filter_supports_alpha_only_view() {
    let data = BitmapPreviewData {
        width: 1,
        height: 1,
        image_count: 1,
        mip_count: 1,
        format_name: "a8r8g8b8".to_owned(),
        type_name: "2D texture".to_owned(),
        rgba: vec![10, 20, 30, 128],
    };
    let mut preview = BitmapPreviewState::default();
    preview.show_red = false;
    preview.show_green = false;
    preview.show_blue = false;
    preview.show_alpha = true;

    assert_eq!(
        filtered_bitmap_rgba(&data, &preview),
        vec![128, 128, 128, 255]
    );
}

#[test]
fn folder_bitmap_collector_finds_nested_bitmap_entries() {
    let entries = vec![
        TagEntry {
            key: "bitmap".into(),
            display_path: "objects/test/diffuse.bitmap".into(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("diffuse.bitmap")),
        },
        TagEntry {
            key: "model".into(),
            display_path: "objects/test/object.model".into(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: Some("model".into()),
            location: TagEntryLocation::LooseFile(PathBuf::from("object.model")),
        },
    ];
    let node = TagTreeNode {
        label: "objects".into(),
        rel_path: PathBuf::from("objects"),
        entries: vec![],
        children: vec![TagTreeNode {
            label: "test".into(),
            rel_path: PathBuf::from("objects/test"),
            entries: vec![0, 1],
            children: vec![],
            children_loaded: true,
            entries_loaded: true,
            ..Default::default()
        }],
        children_loaded: true,
        entries_loaded: true,
        ..Default::default()
    };

    assert_eq!(collect_bitmap_keys(&node, &entries), vec!["bitmap"]);
}

#[test]
fn folder_json_path_preserves_tag_extension_under_source_tree() {
    let entry = TagEntry {
        key: "model".into(),
        display_path: "objects/test/spartans.model".into(),
        group_tag: u32::from_be_bytes(*b"hlmt"),
        group_name: Some("model".into()),
        location: TagEntryLocation::LooseFile(PathBuf::from("spartans.model")),
    };

    assert_eq!(
        tag_json_relative_path(&entry),
        PathBuf::from("objects/test/spartans.model.json")
    );
}

#[test]
fn shader_function_summary_uses_curve_points_instead_of_placeholder() {
    let mut blob = vec![0u8; 32];
    blob[0] = 5; // LinearKey
    blob[4..8].copy_from_slice(&0.0f32.to_le_bytes());
    blob[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    for &(x, y) in &[(0.0_f32, 0.0_f32), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)] {
        blob.extend_from_slice(&x.to_le_bytes());
        blob.extend_from_slice(&y.to_le_bytes());
    }
    for _ in 0..4 {
        blob.extend_from_slice(&0.0_f32.to_le_bytes());
    }
    for _ in 0..4 {
        blob.extend_from_slice(&0.0_f32.to_le_bytes());
    }
    for &v in &[1.0_f32, 0.0, -1.0, 0.0] {
        blob.extend_from_slice(&v.to_le_bytes());
    }

    let function = TagFunction::parse(&blob).unwrap();
    let summary = shader_function_grid_text(&function);

    assert!(summary.contains("curve:"));
    assert!(summary.contains("(0.25, 1.0)"));
    assert!(!summary.contains("function data goes here"));
}

#[test]
fn shader_function_summary_reads_static_color_data() {
    let mut blob = [0u8; 32];
    blob[0] = 1; // Constant
    blob[2] = ColorGraphType::OneColor as u8;
    blob[4..8].copy_from_slice(&0xFF_33_66_99u32.to_le_bytes());

    let function = TagFunction::parse(&blob).unwrap();
    let summary = shader_function_grid_text(&function);

    assert!(summary.contains("RGB"));
    assert!(summary.contains("0.2, 0.4, 0.6"));
}

#[test]
fn shader_constant_color_with_unset_alpha_displays_opaque() {
    let mut blob = [0u8; 32];
    blob[0] = 1; // Constant
    blob[2] = ColorGraphType::TwoColor as u8;
    blob[4..8].copy_from_slice(&0x00_C0_C0_C0u32.to_le_bytes());
    blob[16..20].copy_from_slice(&0x00_00_00_00u32.to_le_bytes());

    let function = TagFunction::parse(&blob).unwrap();
    let [r, g, b, a] = extract_constant_color(&function).unwrap();

    assert_eq!(
        (r, g, b, a),
        (192.0 / 255.0, 192.0 / 255.0, 192.0 / 255.0, 1.0)
    );
}

#[test]
fn generated_constant_color_functions_use_render_method_gpu_flag() {
    let bytes = decode_hex(&constant_color_function_hex(1.0, 0.0, 0.0, 1.0)).unwrap();
    let function = TagFunction::parse(&bytes).unwrap();

    assert_eq!(function.color_graph_type(), ColorGraphType::OneColor);
    let function = function.as_blob().unwrap();
    assert!(function.flags().is_gpu());
    assert_eq!(function.header().colors[0], 0xFFFF0000);
}

#[test]
fn function_edit_data_field_still_emits_hex_field_edit() {
    let bytes = decode_hex(&constant_function_hex(0.25)).unwrap();
    let function = TagFunction::parse(&bytes).unwrap();
    let view = FunctionView::from_function(function).with_edit(FunctionEditPaths {
        data: FunctionDataStorage::DataField("function/data".to_owned()),
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    });
    let previous_function =
        TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap()).unwrap();
    let previous = FunctionSnapshot::from_view(&FunctionView::from_function(previous_function));

    let batch = push_function_edit(view.edit.as_ref().unwrap(), &previous, &view);

    assert_eq!(batch.data_ops.len(), 0);
    assert_eq!(batch.edits.len(), 1);
    assert_eq!(batch.edits[0].path, "function/data");
    assert_eq!(batch.edits[0].input, encode_hex(&bytes));
}

#[test]
fn function_edit_halo2_byte_block_emits_data_op() {
    let bytes = h2_constant_scalar_function_data(0.5, None);
    let function = h2_tag_function(&bytes).unwrap();
    let view = FunctionView::from_function(function).with_edit(FunctionEditPaths {
        data: FunctionDataStorage::Halo2ByteBlock("parameters[0]/function/data".to_owned()),
        parameter_type: String::new(),
        input_name: String::new(),
        range_name: String::new(),
        time_period: String::new(),
        block_path: String::new(),
        block_index: 0,
    });
    let previous_function =
        h2_tag_function(&h2_constant_scalar_function_data(0.0, None)).unwrap();
    let previous = FunctionSnapshot::from_view(&FunctionView::from_function(previous_function));

    let batch = push_function_edit(view.edit.as_ref().unwrap(), &previous, &view);

    assert!(batch.edits.is_empty());
    assert_eq!(batch.data_ops.len(), 1);
    assert_eq!(batch.data_ops[0].block_path, "parameters[0]/function/data");
    assert_eq!(batch.data_ops[0].data, bytes);
}

#[test]
fn halo2_function_byte_block_replacement_roundtrips_bytes() {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let bytes = h2_constant_scalar_function_data(-0.25, None);

    seed_halo2_function_byte_block_for_test(
        &mut tag,
        "parameters[0]/animation properties[0]/function/data",
        &bytes,
    );

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
    let function = h2_tag_function(&bytes).unwrap();
    let reparsed =
        h2_tag_function(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
    assert_eq!(reparsed.to_bytes(), function.to_bytes());
}

#[test]
fn classic_halo2_shader_model_exposes_byte_block_function_row() {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
    tag.container = blam_tags::file::TagContainer::Classic {
        engine: blam_tags::classic::ClassicEngine::Halo2V4,
        header: vec![0; 64],
    };
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let bytes = h2_constant_scalar_function_data(0.75, None);
    seed_halo2_function_byte_block_for_test(
        &mut tag,
        "parameters[0]/animation properties[0]/function/data",
        &bytes,
    );

    let entry = h2_shader_entry(u32::from_be_bytes(*b"shad"));
    let model = build_h2ek_shader_editor_model(
        &tag,
        &entry,
        &TagNameIndex::default(),
        None,
        &mut H2TemplateCache::default(),
    )
    .unwrap();
    let (function_bytes, path) = first_halo2_byte_block_function_row(&model).unwrap();

    assert_eq!(function_bytes, bytes);
    assert_eq!(path, "parameters[0]/animation properties[0]/function/data");
}

#[test]
fn h2ek_shader_model_routes_only_classic_halo2_shader_family() {
    let entry = h2_shader_entry(u32::from_be_bytes(*b"rmsh"));
    let mut classic = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
    classic.container = blam_tags::file::TagContainer::Classic {
        engine: blam_tags::classic::ClassicEngine::Halo2V4,
        header: vec![0; 64],
    };
    assert!(
        build_h2ek_shader_editor_model(
            &classic,
            &entry,
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default()
        )
        .is_some()
    );

    let mcc = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
    assert!(
        build_h2ek_shader_editor_model(
            &mcc,
            &entry,
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default()
        )
        .is_none()
    );

    let non_shader = classic;
    let non_shader_entry = h2_shader_entry(u32::from_be_bytes(*b"bitm"));
    assert!(
        build_h2ek_shader_editor_model(
            &non_shader,
            &non_shader_entry,
            &TagNameIndex::default(),
            None,
            &mut H2TemplateCache::default(),
        )
        .is_none()
    );
}

#[test]
fn h2ek_shader_model_exposes_schema_backed_value_rows() {
    let mut tag = h2_classic_shader_tag();
    apply_field_edit(
        &mut tag,
        "template",
        "stem:shaders/shader_templates/transparent/plasma_mask_offset",
    )
    .unwrap();
    apply_field_edit(&mut tag, "material name", "test_material").unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut tag, "parameters[0]/name", "diffuse_map").unwrap();
    apply_field_edit(&mut tag, "parameters[0]/type", "1").unwrap();
    apply_field_edit(&mut tag, "parameters[0]/const value", "0.5").unwrap();

    let model = build_h2ek_shader_editor_model(
        &tag,
        &h2_shader_entry(u32::from_be_bytes(*b"rmsh")),
        &TagNameIndex::default(),
        None,
        &mut H2TemplateCache::default(),
    )
    .unwrap();

    let material_name = shader_row_edit_path_and_kind(&model, "material_name").unwrap();
    assert_eq!(material_name, ("material name".to_owned(), "string_id"));

    let template = shader_row_edit_path_and_kind(&model, "template").unwrap();
    assert_eq!(template, ("template".to_owned(), "shader_template_ref"));

    let const_value = shader_row_edit_path_and_kind(&model, "diffuse_map").unwrap();
    assert_eq!(
        const_value,
        ("parameters[0]/const value".to_owned(), "scalar")
    );
}

#[test]
fn h2ek_shader_standard_rows_use_guerilla_widgets() {
    let mut tag = h2_classic_shader_tag();
    apply_field_edit(&mut tag, "flags", "5").unwrap();
    apply_field_edit(&mut tag, "specular type", "1").unwrap();
    apply_field_edit(&mut tag, "lightmap type", "2").unwrap();
    apply_field_edit(&mut tag, "shader LOD bias", "1").unwrap();

    let model = build_h2ek_shader_editor_model(
        &tag,
        &h2_shader_entry(u32::from_be_bytes(*b"rmsh")),
        &TagNameIndex::default(),
        None,
        &mut H2TemplateCache::default(),
    )
    .unwrap();

    assert_eq!(
        shader_row_edit_path_and_kind(&model, "flags"),
        Some(("flags".to_owned(), "flags"))
    );
    assert_eq!(
        shader_row_edit_path_and_kind(&model, "dynamic_light_specular_type"),
        Some(("specular type".to_owned(), "enum"))
    );
    assert_eq!(
        shader_row_value_text_for_test(&model, "dynamic_light_specular_type").as_deref(),
        Some("default shiny")
    );
    assert_eq!(
        shader_row_value_text_for_test(&model, "lightmap_type").as_deref(),
        Some("dull specular")
    );
    assert_eq!(
        shader_row_value_text_for_test(&model, "shader_lod_bias").as_deref(),
        Some("4x size")
    );
}

#[test]
fn h2ek_shader_range_flag_updates_same_length_function_data() {
    let mut data = vec![0; 28];
    data[0] = 1;
    data[1] = FunctionFlags::GPU;
    data[4..8].copy_from_slice(&1.0f32.to_le_bytes());
    data[8..12].copy_from_slice(&1.0f32.to_le_bytes());

    let ranged = h2_function_data_with_range_for_test(&data, true, Some(2.5));
    assert_eq!(ranged.len(), data.len());
    assert_eq!(h2_function_data_range_for_test(&ranged), (true, Some(2.5)));
    assert_eq!(ranged[1] & FunctionFlags::GPU, FunctionFlags::GPU);

    let unranged = h2_function_data_with_range_for_test(&ranged, false, None);
    assert_eq!(unranged.len(), data.len());
    assert_eq!(h2_function_data_range_for_test(&unranged).0, false);
    assert_eq!(unranged[1] & FunctionFlags::GPU, FunctionFlags::GPU);
}

#[test]
fn h2ek_shader_template_reference_accepts_h2ek_extension_path() {
    let mut tag = h2_classic_shader_tag();
    apply_field_edit(
        &mut tag,
        "template",
        "stem:shaders\\shader_templates\\transparent\\plasma_mask_offset.shader_template",
    )
    .unwrap();

    assert_eq!(
        h2_shader_template_reference_for_test(&tag).as_deref(),
        Some("shaders\\shader_templates\\transparent\\plasma_mask_offset")
    );
}

#[test]
fn h2ek_shader_template_rows_drive_visible_parameters() {
    let mut shader = h2_classic_shader_tag();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut shader, "parameters[0]/name", "self_illum_color").unwrap();
    apply_field_edit(&mut shader, "parameters[0]/type", "2").unwrap();

    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    for index in 0..2 {
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let parameter_path = format!("categories[0]/parameters[{index}]");
        let name = if index == 0 {
            "noise_map1"
        } else {
            "plasma_mask"
        };
        apply_field_edit(&mut template, &format!("{parameter_path}/name"), name).unwrap();
        apply_field_edit(&mut template, &format!("{parameter_path}/type"), "0").unwrap();
    }
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap animation flags",
        "6",
    )
    .unwrap();

    let labels = h2_template_row_labels_for_test(&shader, &template);

    for expected in [
        "noise_map1",
        "noise_map1_scale_x",
        "noise_map1_scale_y",
        "noise_map1_translation_x",
        "noise_map1_translation_y",
        "plasma_mask",
    ] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected} in {labels:?}"
        );
    }
    assert!(!labels.iter().any(|label| label == "self_illum_color"));
}

#[test]
fn h2ek_shader_3d_bitmap_template_rows_include_z_transform() {
    let shader = h2_classic_shader_tag();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap type",
        "3D",
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap animation flags",
        "5",
    )
    .unwrap();

    let labels = h2_template_row_labels_for_test(&shader, &template);

    for expected in [
        "noyze0",
        "noyze0_scale",
        "noyze0_translation_x",
        "noyze0_translation_y",
        "noyze0_translation_z",
    ] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected} in {labels:?}"
        );
    }
}

#[test]
fn h2ek_shader_missing_function_rows_are_numeric_create_fields() {
    let shader = h2_classic_shader_tag();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap animation flags",
        "5",
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap scale",
        "7.5",
    )
    .unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
        Some("h2_create_function_scalar")
    );
    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
        Some("value: 7.5")
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_translation_x"),
        Some("h2_create_function_scalar")
    );
    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze0_translation_x")
            .as_deref(),
        Some("value: 0.0")
    );
}

#[test]
fn h2ek_shader_existing_constant_function_rows_stay_numeric() {
    let mut shader = h2_classic_shader_tag();
    apply_one_h2_shader_param_op(
        &mut shader,
        &H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path: "parameters".to_owned(),
            parameter_name: "noyze0".to_owned(),
            parameter_type_index: 0,
            animation_type_index: 0,
            initial_function_data: h2_constant_scalar_function_data(7.5, None),
        },
    )
    .unwrap();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/name", "noyze0").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "0").unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/bitmap animation flags",
        "1",
    )
    .unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
        Some("h2_function_scalar")
    );
    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
        Some("value: 7.5")
    );
}

#[test]
fn h2ek_shader_color_tint_rows_use_color_animation_type() {
    let shader = h2_classic_shader_tag();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/name",
        "color_wide",
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/default const color",
        "1, 1, 1",
    )
    .unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "color_wide_tint"),
        Some("h2_create_function_color")
    );
    assert_eq!(
        h2_template_row_value_color_for_test(&shader, &template, "color_wide_tint"),
        Some((255, 255, 255, 255))
    );
}

#[test]
fn h2ek_shader_existing_constant_color_functions_render_swatch() {
    let mut shader = h2_classic_shader_tag();
    apply_one_h2_shader_param_op(
        &mut shader,
        &H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path: "parameters".to_owned(),
            parameter_name: "color_sharp".to_owned(),
            parameter_type_index: 2,
            animation_type_index: 12,
            initial_function_data: h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None),
        },
    )
    .unwrap();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/name",
        "color_sharp",
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp_tint"),
        Some("h2_function_color")
    );
    assert_eq!(
        h2_template_row_value_color_for_test(&shader, &template, "color_sharp_tint"),
        Some((255, 0, 0, 255))
    );
    assert_h2_write_atomic_verifies(&shader, "h2_color_function_existing");
}

#[test]
fn h2ek_shader_postprocess_constants_initialize_template_rows() {
    let mut shader = h2_classic_shader_tag();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/value properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/value properties[0]/value",
        "7.5",
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/color properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/color properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/color properties[1]/color",
        "1, 0, 0",
    )
    .unwrap();

    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    for (index, (name, ty, flags)) in [("noyze0", "0", "1"), ("color_sharp", "2", "0")]
        .into_iter()
        .enumerate()
    {
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let path = format!("categories[0]/parameters[{index}]");
        apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
        apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
        apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
        if name == "noyze0" {
            apply_field_edit(
                &mut template,
                &format!("{path}/bitmap animation flags"),
                "1",
            )
            .unwrap();
        }
    }

    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
        Some("value: 7.5")
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
        Some("scalar")
    );
    assert_eq!(
        h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
        Some((255, 0, 0, 255))
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
        Some("color")
    );
}

#[test]
fn h2ek_shader_legacy_animation_bytes_initialize_template_rows() {
    let mut shader = h2_classic_shader_tag();
    for (index, (name, ty, anim_ty, data)) in [
        ("noyze0", "0", "0", {
            let mut data = vec![0; 28];
            data[0] = 1;
            data[4..8].copy_from_slice(&7.5f32.to_le_bytes());
            data[8..12].copy_from_slice(&1.0f32.to_le_bytes());
            data
        }),
        ("color_sharp", "2", "12", {
            let mut data = vec![0; 28];
            data[0] = 1;
            data[1] = 0x20;
            data[4] = 0;
            data[5] = 0;
            data[6] = 255;
            data[7] = 255;
            data
        }),
        ("noyze1", "0", "5", {
            let mut data = vec![0; 52];
            data[0] = 3;
            data[2] = 0x0a;
            data[10] = 0x80;
            data[11] = 0x3f;
            data[22] = 0x80;
            data[23] = 0x3f;
            data
        }),
    ]
    .into_iter()
    .enumerate()
    {
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let path = format!("parameters[{index}]");
        apply_field_edit(&mut shader, &format!("{path}/name"), name).unwrap();
        apply_field_edit(&mut shader, &format!("{path}/type"), ty).unwrap();
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: format!("{path}/animation properties"),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            &mut shader,
            &format!("{path}/animation properties[0]/type"),
            anim_ty,
        )
        .unwrap();
        seed_halo2_raw_function_byte_block_for_test(
            &mut shader,
            &format!("{path}/animation properties[0]/function/data"),
            &data,
        );
    }

    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    for (index, (name, ty, flags, bitmap_flags)) in [
        ("noyze0", "0", "1", "1"),
        ("color_sharp", "2", "1", "0"),
        ("noyze1", "0", "1", "4"),
    ]
    .into_iter()
    .enumerate()
    {
        apply_one_block_op(
            &mut template,
            &BlockOp {
                path: "categories[0]/parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        let path = format!("categories[0]/parameters[{index}]");
        apply_field_edit(&mut template, &format!("{path}/name"), name).unwrap();
        apply_field_edit(&mut template, &format!("{path}/type"), ty).unwrap();
        apply_field_edit(&mut template, &format!("{path}/flags"), flags).unwrap();
        apply_field_edit(
            &mut template,
            &format!("{path}/bitmap animation flags"),
            bitmap_flags,
        )
        .unwrap();
        if name == "noyze1" {
            apply_field_edit(&mut template, &format!("{path}/bitmap type"), "3D").unwrap();
        }
    }

    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze0_scale").as_deref(),
        Some("value: 7.5")
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze0_scale"),
        Some("h2_function_scalar")
    );
    assert_eq!(
        h2_template_row_value_color_for_test(&shader, &template, "color_sharp"),
        Some((255, 0, 0, 255))
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "color_sharp"),
        Some("h2_function_color")
    );
    assert_eq!(
        h2_template_row_value_text_for_test(&shader, &template, "noyze1_translation_y")
            .as_deref(),
        Some("<function data goes here>")
    );
    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "noyze1_translation_y"),
        None
    );
    assert_eq!(
        h2_template_row_function_data_path_for_test(&shader, &template, "noyze1_translation_y")
            .as_deref(),
        Some("parameters[2]/animation properties[0]/function/data")
    );

    let mut legacy_scale = vec![0; 28];
    legacy_scale[0] = 1;
    legacy_scale[4..8].copy_from_slice(&7.5f32.to_le_bytes());
    let scale_edit = h2_constant_scalar_function_data(5.0, Some(&legacy_scale));
    assert_eq!(scale_edit.len(), 28);
    assert_eq!(
        f32::from_le_bytes(scale_edit[4..8].try_into().unwrap()),
        5.0
    );
    let color_edit = h2_constant_color_function_data(
        0.0,
        1.0,
        0.0,
        1.0,
        Some(&[
            1, 0x20, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]),
    );
    assert_eq!(&color_edit[..8], &[1, 0x20, 0, 0, 0, 255, 0, 255]);
}

#[test]
fn h2_shader_template_switch_prunes_unmatched_parameters() {
    let mut shader = h2_classic_shader_tag();
    for (index, name) in ["keep_me", "drop_me"].into_iter().enumerate() {
        apply_one_block_op(
            &mut shader,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(&mut shader, &format!("parameters[{index}]/name"), name).unwrap();
        apply_field_edit(&mut shader, &format!("parameters[{index}]/type"), "0").unwrap();
    }

    apply_one_h2_shader_param_op(
        &mut shader,
        &H2ShaderParamOp::SwitchTemplate {
            parameters_block_path: "parameters".to_owned(),
            allowed_parameter_names: vec!["keep_me".to_owned()],
        },
    )
    .unwrap();

    let parameters = shader
        .root()
        .field("parameters")
        .and_then(|field| field.as_block())
        .unwrap();
    assert_eq!(parameters.len(), 1);
    assert_eq!(
        parameters
            .element(0)
            .and_then(|parameter| parameter.read_string_id("name")),
        Some("keep_me".to_owned())
    );
}

#[test]
fn h2ek_shader_postprocess_color_overlay_initializes_tint_swatch() {
    let mut shader = h2_classic_shader_tag();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/overlays".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/overlay references".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/overlay references[0]/overlay index",
        "0",
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/overlay references[0]/transform index",
        "0",
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/animated parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/animated parameters[0]/overlay references/block index data",
        "0",
    )
    .unwrap();
    apply_one_block_op(
        &mut shader,
        &BlockOp {
            path: "postprocess definition[0]/animated parameter references".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut shader,
        "postprocess definition[0]/animated parameter references[0]/parameter index",
        "0",
    )
    .unwrap();
    seed_halo2_wrapped_function_byte_block_for_test(
        &mut shader,
        "postprocess definition[0]/overlays[0]/function",
        &h2_constant_color_function_data(1.0, 1.0, 0.0, 1.0, None),
    );

    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/name", "transparent").unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/name",
        "center_line",
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "2").unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/flags", "1").unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "center_line_tint"),
        Some("h2_function_color")
    );
    assert_eq!(
        h2_template_row_value_color_for_test(&shader, &template, "center_line_tint"),
        Some((255, 255, 0, 255))
    );

    apply_one_h2_shader_param_op(
        &mut shader,
        &H2ShaderParamOp::EditFunctionData {
            block_path: "postprocess definition[0]/overlays[0]/function/function/data"
                .to_owned(),
            data: h2_constant_color_function_data(0.0, 0.0, 0.0, 1.0, None),
        },
    )
    .unwrap();
    let overlay = shader
        .root()
        .descend("postprocess definition[0]/overlays[0]/function")
        .unwrap();
    let function_struct = overlay
        .fields()
        .find(|field| field.name() == "function" && field.field_type() == TagFieldType::Struct)
        .and_then(|field| field.as_struct())
        .unwrap();
    let bytes = halo2_function_bytes_from_struct(function_struct).unwrap();
    let function = h2_tag_function(&bytes).unwrap();
    assert_eq!(
        extract_constant_color(&function),
        Some([0.0, 0.0, 0.0, 1.0])
    );
}

#[test]
fn h2_shader_color_function_create_and_edit_reparse() {
    let mut tag = h2_classic_shader_tag();
    let red = h2_constant_color_function_data(1.0, 0.0, 0.0, 1.0, None);
    apply_one_h2_shader_param_op(
        &mut tag,
        &H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path: "parameters".to_owned(),
            parameter_name: "center_line".to_owned(),
            parameter_type_index: 2,
            animation_type_index: 12,
            initial_function_data: red,
        },
    )
    .unwrap();

    let parameters = tag
        .root()
        .field("parameters")
        .and_then(|field| field.as_block())
        .unwrap();
    let animation = parameters
        .element(0)
        .unwrap()
        .field("animation properties")
        .and_then(|field| field.as_block())
        .and_then(|block| block.element(0))
        .unwrap();
    assert_eq!(animation.read_int_any("type"), Some(12));
    let data_path = "parameters[0]/animation properties[0]/function/data";
    let grey = h2_constant_color_function_data(0.5, 0.5, 0.5, 1.0, None);
    apply_one_h2_shader_param_op(
        &mut tag,
        &H2ShaderParamOp::EditFunctionData {
            block_path: data_path.to_owned(),
            data: grey.clone(),
        },
    )
    .unwrap();

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), grey);
    let function =
        h2_tag_function(&halo2_function_bytes_from_struct(mapping).unwrap()).unwrap();
    let color = extract_constant_color(&function).unwrap();
    for (actual, expected) in
        color
            .iter()
            .zip([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0])
    {
        assert!((actual - expected).abs() < 0.0001);
    }
    assert_h2_write_atomic_verifies(&tag, "h2_color_function_edit");
}

#[test]
fn h2ek_shader_missing_template_value_row_is_create_editable() {
    let shader = h2_classic_shader_tag();
    let mut template =
        TagFile::new(test_definition_path("halo2_mcc/shader_template.json")).unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut template,
        &BlockOp {
            path: "categories[0]/parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/name",
        "plasma_factor",
    )
    .unwrap();
    apply_field_edit(&mut template, "categories[0]/parameters[0]/type", "1").unwrap();
    apply_field_edit(
        &mut template,
        "categories[0]/parameters[0]/default const value",
        "0.35",
    )
    .unwrap();

    assert_eq!(
        h2_template_row_edit_kind_for_test(&shader, &template, "plasma_factor"),
        Some("h2_create_template_value")
    );
}

#[test]
fn h2_shader_template_value_edit_creates_single_parameter() {
    let mut tag = h2_classic_shader_tag();
    apply_one_h2_shader_param_op(
        &mut tag,
        &H2ShaderParamOp::EditTemplateBackedValue {
            parameters_block_path: "parameters".to_owned(),
            parameter_name: "plasma_brightness".to_owned(),
            parameter_type_index: 1,
            field: "const value".to_owned(),
            input: "1.25".to_owned(),
        },
    )
    .unwrap();

    let parameters = tag
        .root()
        .field("parameters")
        .and_then(|field| field.as_block())
        .unwrap();
    assert_eq!(parameters.len(), 1);
    let parameter = parameters.element(0).unwrap();
    assert_eq!(
        parameter.read_string_id("name").as_deref(),
        Some("plasma_brightness")
    );
    assert_eq!(parameter.read_int_any("type"), Some(1));
    assert_eq!(parameter.read_real("const value"), Some(1.25));
}

#[test]
fn h2_shader_template_function_create_materializes_backing_data() {
    let mut tag = h2_classic_shader_tag();
    let bytes = h2_constant_scalar_function_data(0.5, None);
    apply_one_h2_shader_param_op(
        &mut tag,
        &H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path: "parameters".to_owned(),
            parameter_name: "noise_map1".to_owned(),
            parameter_type_index: 0,
            animation_type_index: 5,
            initial_function_data: bytes.clone(),
        },
    )
    .unwrap();

    let parameters = tag
        .root()
        .field("parameters")
        .and_then(|field| field.as_block())
        .unwrap();
    assert_eq!(parameters.len(), 1);
    let parameter = parameters.element(0).unwrap();
    assert_eq!(
        parameter.read_string_id("name").as_deref(),
        Some("noise_map1")
    );
    assert_eq!(parameter.read_int_any("type"), Some(0));
    let animation = parameter
        .field("animation properties")
        .and_then(|field| field.as_block())
        .and_then(|block| block.element(0))
        .unwrap();
    assert_eq!(animation.read_int_any("type"), Some(5));
    let mapping = animation
        .field("function")
        .and_then(|field| field.as_struct())
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
    assert_h2_write_atomic_verifies(&tag, "h2_function_create");
}

#[test]
fn h2ek_shader_function_row_exposes_byte_block_and_wrapper_paths() {
    let mut tag = h2_classic_shader_tag();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut tag, "parameters[0]/name", "animated_scalar").unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_field_edit(&mut tag, "parameters[0]/animation properties[0]/type", "8").unwrap();
    apply_field_edit(
        &mut tag,
        "parameters[0]/animation properties[0]/input name",
        "time",
    )
    .unwrap();
    apply_field_edit(
        &mut tag,
        "parameters[0]/animation properties[0]/range name",
        "random",
    )
    .unwrap();
    apply_field_edit(
        &mut tag,
        "parameters[0]/animation properties[0]/time period",
        "2.5",
    )
    .unwrap();
    let bytes = h2_constant_scalar_function_data(0.75, None);
    seed_halo2_function_byte_block_for_test(
        &mut tag,
        "parameters[0]/animation properties[0]/function/data",
        &bytes,
    );

    let model = build_h2ek_shader_editor_model(
        &tag,
        &h2_shader_entry(u32::from_be_bytes(*b"rmsh")),
        &TagNameIndex::default(),
        None,
        &mut H2TemplateCache::default(),
    )
    .unwrap();
    let summary = first_h2_function_edit_summary(&model).expect("function row");

    assert_eq!(summary.bytes, bytes);
    assert_eq!(summary.output_index, Some(8));
    assert_eq!(summary.input_name, "time");
    assert_eq!(summary.range_name, "random");
    assert_eq!(summary.time_period, 2.5);
    assert_eq!(
        summary.data_path,
        "parameters[0]/animation properties[0]/function/data"
    );
    assert_eq!(
        summary.parameter_type_path,
        "parameters[0]/animation properties[0]/type"
    );
    assert_eq!(
        summary.input_name_path,
        "parameters[0]/animation properties[0]/input name"
    );
    assert_eq!(
        summary.range_name_path,
        "parameters[0]/animation properties[0]/range name"
    );
    assert_eq!(
        summary.time_period_path,
        "parameters[0]/animation properties[0]/time period"
    );
}

#[test]
fn h2ek_shader_input_name_edit_writes_without_truncated_struct_panic() {
    let mut tag = h2_classic_shader_tag();
    for _ in 0..2 {
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
    }
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[1]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let bytes = h2_constant_scalar_function_data(0.75, None);
    seed_halo2_function_byte_block_for_test(
        &mut tag,
        "parameters[1]/animation properties[0]/function/data",
        &bytes,
    );

    apply_field_edit(
        &mut tag,
        "parameters[1]/animation properties[0]/input name",
        "shield_strength",
    )
    .unwrap();

    assert_h2_write_atomic_verifies(&tag, "h2_input_name_edit");
    let written = tag.write_to_bytes().expect("write edited h2 shader");
    assert!(
        written
            .windows("shield_strength".len())
            .any(|window| { window == "shield_strength".as_bytes() })
    );
}

#[test]
fn halo2_function_byte_block_rejects_invalid_mapping_function_before_clear() {
    let mut tag = h2_classic_shader_tag();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let original = h2_constant_scalar_function_data(0.25, None);
    let block_path = "parameters[0]/animation properties[0]/function/data";
    seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);

    assert!(replace_halo2_function_byte_block(&mut tag, block_path, &[1, 2, 3]).is_err());

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), original);
}

#[test]
fn halo2_function_byte_block_same_length_edit_writes_in_place() {
    let mut tag = h2_classic_shader_tag();
    for _ in 0..7 {
        apply_one_block_op(
            &mut tag,
            &BlockOp {
                path: "parameters".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
    }
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[6]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let block_path = "parameters[6]/animation properties[0]/function/data";
    let original = h2_constant_scalar_function_data(0.25, None);
    seed_halo2_function_byte_block_for_test(&mut tag, block_path, &original);
    let edited = h2_constant_scalar_function_data(0.75, None);

    replace_halo2_function_byte_block(&mut tag, block_path, &edited).unwrap();

    let mapping = tag
        .root()
        .descend("parameters[6]/animation properties[0]/function")
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), edited);
    assert_h2_write_atomic_verifies(&tag, "h2_function_same_len");
}

#[test]
fn damage_effect_vibration_byte_block_same_length_edit_preserves_36_bytes() {
    let mut tag = h2_classic_shader_tag();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let block_path = "parameters[0]/animation properties[0]/function/data";
    let mut original = vec![0; 36];
    original[0] = 2;
    original[1] = 0;
    original[2] = 1;
    original[20..24].copy_from_slice(&0.8f32.to_le_bytes());
    original[24..28].copy_from_slice(&0.4f32.to_le_bytes());
    original[32..36].copy_from_slice(&1.0f32.to_le_bytes());
    seed_halo2_raw_function_byte_block_for_test(&mut tag, block_path, &original);
    let mut edited = original.clone();
    edited[2] = 2;
    edited[20..24].copy_from_slice(&1.0f32.to_le_bytes());
    edited[24..28].copy_from_slice(&0.7f32.to_le_bytes());

    replace_halo2_function_byte_block(&mut tag, block_path, &edited).unwrap();

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    let written = halo2_function_bytes_from_struct(mapping).unwrap();
    assert_eq!(written.len(), 36);
    assert_eq!(written, edited);
    assert_eq!(&written[32..36], &original[32..36]);
}

#[test]
fn halo2_function_byte_block_existing_length_change_rebuilds_block() {
    let mut tag = h2_classic_shader_tag();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let block_path = "parameters[0]/animation properties[0]/function/data";
    seed_halo2_function_byte_block_for_test(
        &mut tag,
        block_path,
        &h2_constant_scalar_function_data(0.25, None),
    );
    let mut linear_key = vec![0u8; 32];
    linear_key[0] = 5;
    linear_key[4..8].copy_from_slice(&0.0f32.to_le_bytes());
    linear_key[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    for &(x, y) in &[(0.0_f32, 0.0_f32), (0.25, 1.0), (0.75, 1.0), (1.0, 0.0)] {
        linear_key.extend_from_slice(&x.to_le_bytes());
        linear_key.extend_from_slice(&y.to_le_bytes());
    }
    for _ in 0..12 {
        linear_key.extend_from_slice(&0.0_f32.to_le_bytes());
    }

    replace_halo2_function_byte_block(&mut tag, block_path, &linear_key).unwrap();

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    assert_eq!(
        halo2_function_bytes_from_struct(mapping).unwrap(),
        linear_key
    );
    assert_h2_write_atomic_verifies(&tag, "h2_function_resize");
}

#[test]
fn halo2_function_byte_block_empty_creation_rebuilds_block() {
    let mut tag = h2_classic_shader_tag();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "parameters[0]/animation properties".to_owned(),
            kind: BlockOpKind::Add,
        },
    )
    .unwrap();
    let bytes = h2_constant_scalar_function_data(0.25, None);
    replace_halo2_function_byte_block(
        &mut tag,
        "parameters[0]/animation properties[0]/function/data",
        &bytes,
    )
    .unwrap();

    let mapping = tag
        .root()
        .descend("parameters[0]/animation properties[0]/function")
        .unwrap();
    assert_eq!(halo2_function_bytes_from_struct(mapping).unwrap(), bytes);
    assert_h2_write_atomic_verifies(&tag, "h2_function_empty_create");
}

/// Undo and redo snapshot a document as `write_to_bytes` and restore it
/// through `read_tag_from_bytes`. A classic (Halo CE / Halo 2) document
/// snapshots in classic format, which only the classic reader with the
/// game's layout can parse; the MCC reader refuses it.
#[test]
fn a_classic_snapshot_restores_through_the_classic_reader() {
    // A Halo 2 `sound_mix` as a classic file: the 64-byte header (group
    // and `BLM!` stored reversed), the root block header, one zeroed
    // element. Read through the same reader a loose classic tag uses.
    let group = u32::from_be_bytes(*b"snmx");
    let definitions = locate_definitions_root();
    let layout =
        blam_tags::TagLayout::from_json(definitions.join("halo2_mcc/sound_mix.json")).unwrap();
    let root = layout.block_layouts[layout.header.tag_group_block_index as usize].struct_index;
    let size = layout.struct_layouts[root as usize].size as usize;
    let mut bytes = vec![0u8; 64];
    bytes[36..40].copy_from_slice(b"xmns");
    bytes[60..64].copy_from_slice(b"!MLB");
    bytes.extend_from_slice(b"dfbt");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(size as u32).to_le_bytes());
    bytes.resize(bytes.len() + size, 0);
    let mut tag = crate::core::source::read_tag_from_bytes(
        &bytes,
        Some(GameId::Halo2),
        Some(&definitions),
        group,
    )
    .expect("a classic tag");
    apply_field_edit(&mut tag, "left stereo gain", "-3").unwrap();
    let snapshot = tag.write_to_bytes().expect("snapshot");

    assert!(
        TagFile::read_from_bytes(&snapshot).is_err(),
        "the MCC reader cannot parse a classic snapshot"
    );
    assert!(
        crate::core::source::read_tag_from_bytes(&snapshot, None, None, group).is_err(),
        "a classic snapshot needs the game to find its layout"
    );
    let restored = crate::core::source::read_tag_from_bytes(
        &snapshot,
        Some(GameId::Halo2),
        Some(&locate_definitions_root()),
        group,
    )
    .expect("restore");
    assert!(matches!(
        restored.container,
        blam_tags::file::TagContainer::Classic { .. }
    ));
    let gain = |tag: &TagFile| match tag.root().field_path("left stereo gain")?.value()? {
        TagFieldData::Real(value) => Some(value),
        _ => None,
    };
    assert_eq!(gain(&restored), Some(-3.0), "the edit survives the round trip");
    assert_eq!(restored.write_to_bytes().unwrap(), snapshot);
}

/// The root element of a freshly created tag gets every block index at 0,
/// while an element added to a block gets NONE (-1). The engine's
/// invariant is NONE everywhere; the root is a known gap in
/// `TagBlockData::new_root_default`. This pins today's behaviour so that
/// closing the gap is a deliberate change, and shows the contrast.
#[test]
fn a_fresh_root_has_block_indices_at_zero_unlike_a_new_element() {
    let block_index = |tag: &TagFile, path: &str| match tag.root().field_path(path)?.value()? {
        TagFieldData::CharBlockIndex(v) | TagFieldData::CustomCharBlockIndex(v) => {
            Some(v as i64)
        }
        TagFieldData::ShortBlockIndex(v) | TagFieldData::CustomShortBlockIndex(v) => {
            Some(v as i64)
        }
        TagFieldData::LongBlockIndex(v) | TagFieldData::CustomLongBlockIndex(v) => {
            Some(v as i64)
        }
        _ => None,
    };
    let effect = TagFile::new(test_definition_path("haloreach_mcc/effect.json")).unwrap();
    // Known gap: the engine invariant says this should be NONE (-1).
    assert_eq!(block_index(&effect, "loop start event"), Some(0));

    let mut physics =
        TagFile::new(test_definition_path("haloreach_mcc/physics_model.json")).unwrap();
    physics
        .root_mut()
        .field_path_mut("materials")
        .unwrap()
        .as_block_mut()
        .unwrap()
        .add_element();
    assert_eq!(block_index(&physics, "materials[0]/phantom type"), Some(-1));
}

fn h2_classic_shader_tag() -> TagFile {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json")).unwrap();
    let mut header = vec![0; 64];
    header[36..40].copy_from_slice(b"hsmr");
    header[56..58].copy_from_slice(&0u16.to_le_bytes());
    header[60..64].copy_from_slice(b"!MLB");
    tag.container = blam_tags::file::TagContainer::Classic {
        engine: blam_tags::classic::ClassicEngine::Halo2V4,
        header,
    };
    tag
}

fn assert_h2_write_atomic_verifies(tag: &TagFile, name: &str) {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!(
        "baboon_{name}_{}_{}.shader",
        std::process::id(),
        stamp
    ));
    let _ = fs::remove_file(&path);
    tag.write_atomic(&path).unwrap_or_else(|error| {
        panic!(
            "write_atomic verification failed for {}: {error}",
            path.display()
        )
    });
    let _ = fs::remove_file(&path);
}

fn seed_halo2_function_byte_block_for_test(tag: &mut TagFile, block_path: &str, data: &[u8]) {
    h2_tag_function(data).expect("seed data is an H2 block");
    seed_halo2_raw_function_byte_block_for_test(tag, block_path, data);
}

fn seed_halo2_raw_function_byte_block_for_test(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) {
    apply_one_block_op(
        tag,
        &BlockOp {
            path: block_path.to_owned(),
            kind: BlockOpKind::DeleteAll,
        },
    )
    .unwrap();
    for (index, byte) in data.iter().copied().enumerate() {
        apply_one_block_op(
            tag,
            &BlockOp {
                path: block_path.to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
        apply_field_edit(
            tag,
            &format!("{block_path}[{index}]/Value"),
            &(byte as i8).to_string(),
        )
        .unwrap();
    }
}

fn seed_halo2_wrapped_function_byte_block_for_test(
    tag: &mut TagFile,
    wrapper_path: &str,
    data: &[u8],
) {
    h2_tag_function(data).expect("seed data is an H2 block");
    let mut root = tag.root_mut();
    let mut wrapper_field = root.field_path_mut(wrapper_path).unwrap();
    let mut wrapper = wrapper_field.as_struct_mut().unwrap();
    let mut wrote = false;
    wrapper.for_each_field_mut(|mut field| {
        if wrote
            || field.as_ref().name() != "function"
            || field.as_ref().field_type() != TagFieldType::Struct
        {
            return;
        }
        let Some(mut mapping) = field.as_struct_mut() else {
            return;
        };
        let Some(mut data_field) = mapping.field_mut("data") else {
            return;
        };
        let Some(mut block) = data_field.as_block_mut() else {
            return;
        };
        block.clear();
        for byte in data.iter().copied() {
            let index = block.add_element();
            let mut element = block.element_mut(index).unwrap();
            element
                .field_mut("Value")
                .unwrap()
                .set(TagFieldData::CharInteger(byte as i8))
                .unwrap();
        }
        wrote = true;
    });
    assert!(wrote, "failed to seed wrapped H2 function bytes");
}

/// The H2 shader grid is rebuilt every frame, and used to read and parse
/// its `.shader_template` off disk each time. Now the template is read
/// once, and again only when the file changes.
#[test]
fn the_h2_shader_grid_reads_its_template_once_per_change() {
    let root = std::env::temp_dir().join(format!(
        "baboon-h2-template-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("shaders")).unwrap();
    let template_path = root.join("shaders/test.shader_template");
    std::fs::write(&template_path, b"not a template").unwrap();

    let mut tag = h2_classic_shader_tag();
    crate::app::apply_field_edit(&mut tag, "template", "stem:shaders/test").unwrap();
    let entry = h2_shader_entry(u32::from_be_bytes(*b"shad"));
    let source = TagSource::LooseFolder {
        root: root.clone(),
        game: None,
        definitions_root: PathBuf::new(),
    };
    let mut templates = H2TemplateCache::default();
    let frames = |templates: &mut H2TemplateCache| {
        for _ in 0..3 {
            build_h2ek_shader_editor_model(
                &tag,
                &entry,
                &TagNameIndex::default(),
                Some(&source),
                templates,
            );
        }
    };

    frames(&mut templates);
    assert_eq!(templates.loads, 1, "three frames, one read");
    // A different size is a different file, whatever the clock says.
    std::fs::write(&template_path, b"still not a template, but longer").unwrap();
    frames(&mut templates);

    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(templates.loads, 2, "a changed file is read again, once");
}

fn h2_shader_entry(group_tag: u32) -> TagEntry {
    TagEntry {
        key: "objects/test/example.shader".into(),
        display_path: "objects/test/example.shader".into(),
        group_tag,
        group_name: Some("shader".into()),
        location: TagEntryLocation::LooseFile(PathBuf::from("example.shader")),
    }
}

#[test]
fn new_tag_dialog_path_uses_selected_file_name_inside_tags_root() {
    let root = Path::new("C:/kit/tags");

    let (output, display) =
        new_tag_output_path_from_dialog(root, Path::new("C:/kit/tags/objects/foo"), "shader")
            .unwrap();
    assert_eq!(output, PathBuf::from("C:/kit/tags/objects/foo.shader"));
    assert_eq!(display, "objects/foo.shader");

    let (output, display) = new_tag_output_path_from_dialog(
        root,
        Path::new("C:/kit/tags/objects/foo.model"),
        "shader",
    )
    .unwrap();
    assert_eq!(output, PathBuf::from("C:/kit/tags/objects/foo.shader"));
    assert_eq!(display, "objects/foo.shader");

    assert!(
        new_tag_output_path_from_dialog(root, Path::new("C:/other/foo.shader"), "shader")
            .is_err()
    );
    assert!(
        new_tag_output_path_from_dialog(
            root,
            Path::new("C:/kit/tags/../other/foo.shader"),
            "shader",
        )
        .is_err()
    );
}
