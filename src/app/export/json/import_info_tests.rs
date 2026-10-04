use super::*;

#[test]
fn import_info_paths_are_sanitized_to_relative_paths() {
    assert_eq!(
        sanitize_import_info_path(r"c:\mcc\source\objects\brute\brute.jms"),
        PathBuf::from("mcc")
            .join("source")
            .join("objects")
            .join("brute")
            .join("brute.jms")
    );
    assert_eq!(
        sanitize_import_info_path(r"..\..\escape.jms"),
        PathBuf::from("escape.jms")
    );
    assert_eq!(sanitize_import_info_path(""), PathBuf::from("file"));
}

#[test]
fn h2_render_model_import_info_resolves_from_root_block() {
    let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
    {
        let mut root = tag.root_mut();
        let mut import_info_field = root.field_path_mut("import info").unwrap();
        let mut import_info = import_info_field.as_block_mut().unwrap();
        import_info.add_element();
    }

    let root = tag.root();
    let import_info = resolve_import_info_struct(&tag, root).expect("root import info block");

    assert!(import_info.field("files").is_some());
}
