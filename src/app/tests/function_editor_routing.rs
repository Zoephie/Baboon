use super::*;

fn constant_view() -> FunctionView {
    let bytes = decode_hex(&constant_function_hex(0.0)).expect("constant function bytes");
    FunctionView::from_function(TagFunction::parse(&bytes).expect("constant function"))
}

#[test]
fn h3_wrapped_mapping_functions_use_foundation_popup() {
    assert!(uses_foundation_function_popup(&constant_view()));
}

#[test]
fn h2_wrapped_mapping_functions_keep_legacy_editor() {
    let mut raw = vec![0; 52];
    raw[0] = FunctionType::Constant as u8;
    raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
    let legacy = H2LegacyFunctionView::parse(raw).expect("legacy H2 function");
    let view = constant_view().with_h2_legacy(legacy);

    assert!(!uses_foundation_function_popup(&view));
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
    assert!(
        uses_foundation_function_popup(&view),
        "a Reach function belongs in the Foundation editor, not the H2 legacy one"
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
    assert!(function.flags().is_clamped(), "CLAMPED");
    assert!(function.flags().is_gpu(), "GPU");
    assert!(!function.flags().is_optimized(), "postprocess clears OPTIMIZED");
}

/// Halo 2 models a function as a typed `MAPP` struct rather than a `data` blob,
/// so nothing is seeded and the legacy editor keeps owning it. Pinned because
/// both halves of this fix key on a schema name, and a change that started
/// matching H2 would write Reach-shaped bytes into an H2 tag.
#[test]
fn halo2_functions_are_left_to_the_legacy_path() {
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
        let mut anim = field.as_block_mut().expect("animation properties is a block");
        anim.add_element();
    }

    // H2's `function` is a struct of typed fields, so there is no `data` field
    // for the seeding to have touched.
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
