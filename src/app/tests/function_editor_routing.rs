use super::*;

fn constant_view() -> FunctionView {
    let bytes = decode_hex(&constant_function_hex(0.0)).expect("constant function bytes");
    FunctionView::from_function(TagFunction::parse(&bytes).expect("constant function"))
}

#[test]
fn every_function_reads_in_its_games_encoding() {
    assert_eq!(constant_view().function.encoding(), FunctionEncoding::Blob);
    let mut raw = vec![0; 28];
    raw[0] = FunctionType::Constant as u8;
    raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    let view = FunctionView::from_function(h2_tag_function(&raw).expect("an H2 block"));
    assert_eq!(view.function.encoding(), FunctionEncoding::H2);
}

/// Adding a block element that contains a `mapping_function` used to leave the
/// function at `data [0 bytes]`, which fell past
/// `inline_mapping_function_from_struct` and drew a raw byte row with no
/// function editor. The reported case: `equipment`'s hologram block and its
/// `shimmer to camo function`.
#[test]
fn a_new_block_elements_function_is_recognized_by_the_editor() {
    let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
        .expect("the Reach equipment schema loads");
    {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut("hologram")
            .expect("hologram field resolves");
        let mut hologram = field.as_block_mut().expect("hologram is a block");
        hologram.add_element();
    }

    // The row the UI actually draws is the inner `mapping_function`, not the
    // named wrapper around it — the wrapper has no `data` field of its own, so
    // the field tree descends into it first. That is the nesting in the report:
    // "shimmer to camo function" > "function" > "data".
    // `scalar_function_named_struct` has two fields called "function" — the
    // `fned` editor marker and the `mapping_function` struct — so pick it the
    // way the field tree does, by walking fields, rather than by path ordinal.
    let wrapper = tag
        .root()
        .field_path("hologram[0]/shimmer to camo function")
        .and_then(|field| field.as_struct())
        .expect("the new element exposes the shimmer wrapper struct");
    let shimmer = wrapper
        .fields_all()
        .find_map(|field| field.as_struct())
        .expect("the wrapper holds the mapping_function struct");
    let (view, data_path) = inline_mapping_function_from_struct(
        shimmer,
        "hologram[0]/shimmer to camo function/function",
    )
    .expect("a fresh function must reach the function editor, not a raw data row");

    assert_eq!(
        data_path,
        "hologram[0]/shimmer to camo function/function/data"
    );
    assert_eq!(
        view.function.encoding(),
        FunctionEncoding::Blob,
        "a Reach function is an H3+ blob, not a Halo 2 byte-block"
    );
}

/// The seeded function has to be the engine's, not merely non-empty: Identity,
/// CLAMPED | GPU, clamping to 0..1. Asserting only that the editor opens would
/// pass on any 32 bytes that happen to parse.
#[test]
fn a_new_functions_bytes_match_what_the_engine_writes() {
    let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
        .expect("the Reach equipment schema loads");
    {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut("hologram")
            .expect("hologram field resolves");
        let mut hologram = field.as_block_mut().expect("hologram is a block");
        hologram.add_element();
    }

    let bytes = tag
        .root()
        .field_path("hologram[0]/shimmer to camo function/function/data")
        .and_then(|field| field.as_data().map(|d| d.to_vec()))
        .expect("the function's data field");

    assert_eq!(
        bytes,
        blam_tags::default_function_definition_bytes(blam_tags::io::Endian::Le),
        "a fresh function must be the blob c_function_definition::tag_placement_new writes"
    );
    let function = TagFunction::parse(&bytes).expect("it parses");
    assert_eq!(function.function_type(), FunctionType::Identity);
    let function = function.as_blob().expect("a blob");
    assert!(function.flags().is_clamped(), "CLAMPED");
    assert!(function.flags().is_gpu(), "GPU");
    assert!(
        !function.flags().is_optimized(),
        "postprocess clears OPTIMIZED"
    );
}

/// A new Halo 2 element's function is an empty `data` byte-block. It must get
/// the function editor (opened as the identity the engine grows an empty block
/// into) and must never be seeded with an H3+ blob, which the H2 engine would
/// misread.
#[test]
fn a_new_halo2_function_opens_as_h2_and_gets_no_h3_blob() {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/shader.json"))
        .expect("the Halo 2 shader schema loads");
    {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut("parameters")
            .expect("parameters field resolves");
        let mut params = field.as_block_mut().expect("parameters is a block");
        params.add_element();
    }
    {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut("parameters[0]/animation properties")
            .expect("animation properties field resolves");
        let mut anim = field
            .as_block_mut()
            .expect("animation properties is a block");
        anim.add_element();
    }

    // Nothing seeded: the byte-block is still empty and there is no data field.
    let function_struct = tag
        .root()
        .field_path("parameters[0]/animation properties[0]/function")
        .and_then(|field| field.as_struct())
        .expect("the H2 function struct");
    let seeded: Vec<_> = function_struct
        .fields_all()
        .filter(|field| field.field_type() == TagFieldType::Data)
        .filter_map(|field| field.as_data().map(|d| d.len()))
        .filter(|len| *len > 0)
        .collect();
    assert!(
        seeded.is_empty(),
        "Halo 2 should carry no seeded function blob, found {seeded:?}"
    );
    assert_eq!(
        halo2_function_bytes_from_struct(function_struct),
        Some(Vec::new())
    );

    let (view, data_path) = inline_mapping_function_from_struct(
        function_struct,
        "parameters[0]/animation properties[0]/function",
    )
    .expect("an empty H2 function still reaches the function editor");
    assert_eq!(view.function.encoding(), FunctionEncoding::H2);
    assert_eq!(view.function.function_type(), FunctionType::Identity);
    assert_eq!(
        data_path,
        "parameters[0]/animation properties[0]/function/data"
    );
}

/// Seeding is worthless if the bytes do not persist. A fresh element's function
/// has to come back byte-identical after a write/read cycle — a `data` sub-chunk
/// that the writer drops would look correct in memory and be empty again on
/// reopen, which is the same symptom the fix was for.
#[test]
fn a_seeded_function_survives_a_save_and_reload() {
    let mut tag = TagFile::new(test_definition_path("haloreach_mcc/equipment.json"))
        .expect("the Reach equipment schema loads");
    {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut("hologram")
            .expect("hologram field resolves");
        let mut hologram = field.as_block_mut().expect("hologram is a block");
        hologram.add_element();
    }
    let before = tag
        .root()
        .field_path("hologram[0]/shimmer to camo function/function/data")
        .and_then(|field| field.as_data().map(|d| d.to_vec()))
        .expect("the seeded function");

    let bytes = tag.write_to_bytes().expect("the tag serializes");
    let reloaded = TagFile::read_from_bytes(&bytes).expect("and reads back");
    let after = reloaded
        .root()
        .field_path("hologram[0]/shimmer to camo function/function/data")
        .and_then(|field| field.as_data().map(|d| d.to_vec()))
        .expect("the function survives the round trip");

    assert_eq!(after, before, "the seeded function changed across a save");
    assert_eq!(after.len(), 32, "and it is still a whole function");
}

/// Every `mapping_function` in a struct tree, with its resolvable path.
fn collect_h2_mapping_functions(st: TagStruct<'_>, path: &str, out: &mut Vec<String>) {
    if halo2_function_bytes_from_struct(st).is_some_and(|bytes| !bytes.is_empty()) {
        out.push(path.to_owned());
    }
    for field in st.fields_all() {
        let field_path = append_field_path_for(path, &field);
        if let Some(nested) = field.as_struct() {
            collect_h2_mapping_functions(nested, &field_path, out);
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().enumerate() {
                collect_h2_mapping_functions(element, &format!("{field_path}[{index}]"), out);
            }
        }
    }
}

/// Luna's report went through the field tree: a shipped Halo 2 effect's
/// functions drew with the wrong editor and their edits never landed. Every
/// function in the tag now derives the H2 encoding and byte-block storage from
/// the real path, and an edit written through the byte-block writer reads back
/// as exactly that edit.
#[test]
fn shipped_h2_effect_functions_derive_h2_and_write_back() {
    let tag_path = crate::test_kits::tag_path("halo2_mcc", "effects/cinematics/03/iac_engine_fire.effect");
    let def = test_definition_path("halo2_mcc/effect.json");
    if !std::path::Path::new(tag_path).exists() || !def.exists() {
        eprintln!("skipping: H2 effect/definition not present");
        return;
    }
    let bytes = std::fs::read(tag_path).unwrap();
    let layout = blam_tags::layout::TagLayout::from_json(&def).unwrap();
    let mut tag = blam_tags::classic::read_classic_tag_file(&bytes, layout).unwrap();

    let mut paths = Vec::new();
    collect_h2_mapping_functions(tag.root(), "", &mut paths);
    assert!(
        paths.len() > 5,
        "the effect has particle functions ({} found)",
        paths.len()
    );
    eprintln!("checked {} H2 functions", paths.len());

    let mut target = None;
    for path in &paths {
        let root = tag.root();
        let st = root.descend(path).expect("the collected path resolves");
        let original = halo2_function_bytes_from_struct(st).unwrap();
        let (view, data_path) =
            inline_mapping_function_from_struct(st, path).expect("the editor finds the function");
        assert_eq!(view.function.encoding(), FunctionEncoding::H2, "{path}");
        assert_eq!(
            view.data_bytes(),
            original,
            "{path}: reading never rewrites"
        );
        let edit_paths = foundation_function_edit_paths(&data_path, view.function.encoding());
        assert!(
            matches!(edit_paths.data, FunctionDataStorage::Halo2ByteBlock(_)),
            "{path}"
        );
        if target.is_none() && view.function.color_count() == 0 {
            target = Some((view, edit_paths));
        }
    }

    // Edit one scalar function's output range and write it back.
    let (mut view, edit_paths) = target.expect("a scalar function to edit");
    let before = view.data_bytes();
    let previous = FunctionSnapshot::from_view(&view);
    view.function
        .as_h2_mut()
        .unwrap()
        .set_clamp_range(0.25, 4.0)
        .unwrap();
    let batch = push_function_edit(&edit_paths, &previous, &view);
    assert!(
        batch.edits.is_empty(),
        "no hex string edit for a byte-block"
    );
    assert_eq!(batch.data_ops.len(), 1);
    let op = &batch.data_ops[0];
    replace_halo2_function_byte_block(&mut tag, &op.block_path, &op.data)
        .expect("the writer accepts it");

    let struct_path = op
        .block_path
        .strip_suffix("/data")
        .unwrap_or(&op.block_path);
    let written =
        halo2_function_bytes_from_struct(tag.root().descend(struct_path).unwrap()).unwrap();
    assert_eq!(written, op.data);
    let reread = h2_tag_function(&written).unwrap();
    let f = reread.as_h2().unwrap();
    assert_eq!((f.clamp_range_min(), f.clamp_range_max()), (0.25, 4.0));
    assert_eq!(&written[..4], &before[..4], "header untouched");
    assert_eq!(&written[12..], &before[12..], "graph data untouched");
}
