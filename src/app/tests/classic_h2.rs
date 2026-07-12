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
        assert!(
            color.field("Mapping").and_then(|f| f.as_struct()).is_none(),
            "first 'Mapping' should be the non-struct custom placeholder"
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
