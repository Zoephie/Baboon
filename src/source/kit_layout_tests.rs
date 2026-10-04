use super::*;

#[test]
fn a_kits_root_and_data_sit_beside_its_tags_folder() {
    let layout = KitLayout::from_tags_folder(Path::new("/ek/tags")).unwrap();
    assert_eq!(layout.root, PathBuf::from("/ek"));
    assert_eq!(layout.tags, PathBuf::from("/ek/tags"));
    assert_eq!(layout.data, PathBuf::from("/ek/data"));
    assert!(layout.tags_is_root_tags_folder());
}

/// A folder of loose tags with another name still has its parent for a
/// root, the same as the terminal and sound extraction always used; the
/// tools can't see it, so scenario launching refuses it.
#[test]
fn a_tags_folder_with_another_name_is_not_the_tools_folder() {
    let layout = KitLayout::from_tags_folder(Path::new("/ek/tags_moda")).unwrap();
    assert_eq!(layout.root, PathBuf::from("/ek"));
    assert_eq!(layout.data, PathBuf::from("/ek/data"));
    assert!(!layout.tags_is_root_tags_folder());
    assert!(KitLayout::from_tags_folder(Path::new("/")).is_none());
}

#[test]
fn non_default_languages_use_the_data_folders_language_sibling() {
    let layout = KitLayout::from_tags_folder(Path::new("/ek/tags")).unwrap();
    assert_eq!(layout.data_for_language(None), PathBuf::from("/ek/data"));
    assert_eq!(
        layout.data_for_language(Some("french")),
        PathBuf::from("/ek/data_french")
    );
}

#[test]
fn only_a_loose_folder_has_a_kit_layout() {
    let loose = TagSource::LooseFolder {
        root: PathBuf::from("/ek/tags"),
        game: None,
        definitions_root: PathBuf::new(),
    };
    let single = TagSource::SingleFile {
        path: PathBuf::from("/ek/tags/a.weapon"),
    };
    let with = |source| LoadedSourceData {
        label: String::new(),
        source,
        names: TagNameIndex::default(),
        game: None,
        entries: Vec::new(),
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: false,
        chosen_kit_layout: None,
    };
    assert_eq!(
        with(loose).kit_layout().map(|layout| layout.root),
        Some(PathBuf::from("/ek"))
    );
    assert_eq!(with(single).kit_layout(), None);
}
