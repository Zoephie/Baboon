//! Classic Halo 2 regression tests that exercise real tag fixtures when available.

use super::*;

/// Editing a field inside an H2 particle's `Mapping` struct must target
/// the struct field rather than an earlier custom placeholder with the
/// same name.
#[test]
fn h2_particle_mapping_field_edit_targets_exact_field() {
    let tag_path = "/Users/camden/Halo/halo2_mcc/tags/effects/generic/smoke/steam.particle";
    let def = test_definition_path("halo2_mcc/particle.json");
    if !std::path::Path::new(tag_path).exists() || !std::path::Path::new(&def).exists() {
        eprintln!("skipping: H2 particle/definition not present");
        return;
    }
    let bytes = std::fs::read(tag_path).unwrap();
    let layout = blam_tags::layout::TagLayout::from_json(&def).unwrap();
    let mut tag = blam_tags::classic::read_classic_tag_file(&bytes, layout).unwrap();

    let path = {
        let root = tag.root();
        let color_field = root
            .fields_all()
            .find(|f| f.name() == "color" && f.as_struct().is_some())
            .expect("color struct");
        let color = color_field.as_struct().unwrap();
        // `color` used to hold two fields named `Mapping` -- the struct, and a
        // `custom` placeholder ahead of it -- and this reached the struct
        // through that ambiguity. The ambiguity was Baboon's own: a
        // kit-authored layout names no `custom`, and once the builder stopped
        // naming them the duplicate went away. The placeholder is still there,
        // unnamed. Disambiguation against a duplicate a tag really has is
        // gated by `h2_contrail_derived_path_targets_the_second_duplicate`.
        assert!(
            color.fields_all().any(|f| f.name().is_empty()),
            "the custom placeholder should still be there, just unnamed"
        );
        let mapping_field = color
            .fields_all()
            .find(|f| f.name() == "Mapping" && f.as_struct().is_some())
            .expect("Mapping struct");
        let mapping = mapping_field.as_struct().unwrap();
        let function_type = mapping
            .fields_all()
            .find(|f| f.name() == "Function Type")
            .expect("Function Type");
        let path = append_field_path_for("", &color_field);
        let path = append_field_path_for(&path, &mapping_field);
        append_field_path_for(&path, &function_type)
    };
    assert!(
        path.contains("Mapping#"),
        "path should carry ordinals: {path}"
    );

    apply_field_edit(&mut tag, &path, "4").unwrap();
    let value = tag.root().field_path(&path).and_then(|field| field.value());
    assert!(
        matches!(value, Some(TagFieldData::CharInteger(4))),
        "Function Type not set via positional path: {value:?}"
    );
}

/// The path the editor derives for a field must reach *that* field, when the
/// tag really does name two the same.
///
/// This is the case positional paths exist for. It runs against a duplicate an
/// H2 contrail actually ships -- `bitmap` twice at the root -- rather than one
/// Baboon created for itself by naming a `custom`, which is what the particle
/// fixture above used to rely on. 709 of the kit's 28,195 tags carry a real
/// duplicate.
#[test]
fn h2_contrail_derived_path_targets_the_second_duplicate() {
    let tag_path = "/Users/camden/Halo/halo2_mcc/tags/effects/objects/weapons/rifle/sniper_rifle/sniper.contrail";
    let def = test_definition_path("halo2_mcc/contrail.json");
    if !std::path::Path::new(tag_path).exists() || !std::path::Path::new(&def).exists() {
        eprintln!("skipping: H2 contrail/definition not present");
        return;
    }
    let bytes = std::fs::read(tag_path).unwrap();
    let layout = blam_tags::layout::TagLayout::from_json(&def).unwrap();
    let tag = blam_tags::classic::read_classic_tag_file(&bytes, layout).unwrap();
    let root = tag.root();

    let duplicates: Vec<_> = root.fields_all().filter(|f| f.name() == "bitmap").collect();
    assert_eq!(
        duplicates.len(),
        2,
        "this contrail should declare `bitmap` twice"
    );

    // Derive each path the way the editor does, then resolve it back. The
    // second is the one a bare name can never reach.
    for field in &duplicates {
        let path = append_field_path_for("", field);
        let resolved = root
            .field_path(&path)
            .unwrap_or_else(|| panic!("derived path {path} no longer resolves"));
        assert_eq!(
            resolved.ordinal(),
            field.ordinal(),
            "derived path {path} resolved to a different field"
        );
    }

    // And the two really are distinct, so the loop above is not passing by
    // resolving the same field twice.
    let derived: Vec<String> = duplicates
        .iter()
        .map(|f| append_field_path_for("", f))
        .collect();
    assert_ne!(derived[0], derived[1], "both duplicates derived one path");
}

fn load_h2_tag(tag_path: &str, def_rel: &str) -> Option<blam_tags::TagFile> {
    let def = test_definition_path(def_rel);
    if !std::path::Path::new(tag_path).exists() || !std::path::Path::new(&def).exists() {
        eprintln!("skipping: {tag_path} / {def_rel} not present");
        return None;
    }
    let bytes = std::fs::read(tag_path).ok()?;
    let layout = blam_tags::layout::TagLayout::from_json(&def).ok()?;
    blam_tags::classic::read_classic_tag_file(&bytes, layout).ok()
}

/// A field inside a **versioned** classic block (H2 `bitmap_data`, on-disk 116
/// bytes vs its base struct 140) must resolve and edit. The path descent used to
/// bounds-check against the base size and reject the descent entirely.
#[test]
fn h2_bitmap_versioned_block_field_resolves_and_edits() {
    let Some(mut tag) = load_h2_tag(
        "/Users/camden/Halo/halo2_mcc/tags/globals/loading_screen.bitmap",
        "halo2_mcc/bitmap.json",
    ) else {
        return;
    };

    let path = {
        let root = tag.root();
        let bitmaps = root
            .fields_all()
            .find(|f| f.clean_name() == "bitmaps" && f.as_block().is_some())
            .expect("bitmaps block");
        let block_path = format!("{}[0]", append_field_path_for("", &bitmaps));
        let element = bitmaps.as_block().unwrap().element(0).expect("bitmaps[0]");
        let depth = element
            .fields_all()
            .find(|f| f.clean_name() == "depth")
            .expect("depth field inside bitmap_data");
        append_field_path_for(&block_path, &depth)
    };

    // The fix: descent into the versioned block resolves.
    assert!(
        tag.root().field_path(&path).is_some(),
        "versioned-block descent failed for {path}"
    );
    apply_field_edit(&mut tag, &path, "2").unwrap();
    let value = tag.root().field_path(&path).and_then(|f| f.value());
    assert!(
        matches!(
            value,
            Some(TagFieldData::ShortInteger(2)) | Some(TagFieldData::CharInteger(2))
        ),
        "depth not written through versioned-block path: {value:?}"
    );
}

/// Recursively find the first field whose RAW name carries the `|CODE` dumper
/// artifact, returning (clean resolvable path, raw name). Descends struct/block/
/// array via element 0.
fn find_coded_field(st: &blam_tags::TagStruct<'_>, prefix: &str) -> Option<(String, String)> {
    for field in st.fields_all() {
        let path = append_field_path_for(prefix, &field);
        if field.name().contains('|') {
            return Some((path, field.name().to_owned()));
        }
        let nested = if let Some(inner) = field.as_struct() {
            Some((inner, path.clone()))
        } else if let Some(el) = field.as_block().and_then(|b| b.element(0)) {
            Some((el, format!("{path}[0]")))
        } else if let Some(el) = field.as_array().and_then(|a| a.element(0)) {
            Some((el, format!("{path}[0]")))
        } else {
            None
        };
        if let Some((child, child_prefix)) = nested {
            if let Some(found) = find_coded_field(&child, &child_prefix) {
                return Some(found);
            }
        }
    }
    None
}

/// An H2 `model_animation_graph` block whose stored name carries the `|CODE`
/// dumper artifact (e.g. `animations|ABCDCC`) must resolve. `clean_field_name`
/// now strips the code, so the built path (`resources[0]/animations#N`) matches.
#[test]
fn h2_jmad_name_code_block_resolves() {
    let Some(tag) = load_h2_tag(
        "/Users/camden/Halo/halo2_mcc/tags/objects/cinematics/cigar/cigar.model_animation_graph",
        "halo2_mcc/model_animation_graph.json",
    ) else {
        return;
    };

    let found = find_coded_field(&tag.root(), "");
    let Some((path, raw_name)) = found else {
        eprintln!("skipping: no populated |CODE block in this jmad fixture");
        return;
    };

    assert!(
        raw_name.contains('|'),
        "expected a |CODE raw name, got {raw_name:?}"
    );
    assert!(
        !path.contains('|'),
        "built path must carry the CLEAN name: {path}"
    );
    assert!(
        tag.root().field_path(&path).is_some(),
        "jmad |CODE field {raw_name:?} did not resolve via clean path {path}"
    );
}
