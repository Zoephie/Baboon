use super::*;
use crate::app::kits::loading::replace_loaded_tree_scope;

fn sound(path: &str) -> TagEntry {
    TagEntry {
        key: path.to_owned(),
        display_path: format!("{path}.sound"),
        group_tag: u32::from_be_bytes(*b"snd!"),
        group_name: Some("sound".to_owned()),
        location: TagEntryLocation::LooseFile(PathBuf::from(format!(
            "C:/kit/tags/{path}.sound"
        ))),
    }
}

#[test]
fn loading_an_extraction_scope_materializes_its_nested_tags() {
    let entries = vec![sound("sound/a"), sound("sound/sub/b")];
    let mut tree = TagTree {
        children: vec![TagTreeNode {
            label: "sound".to_owned(),
            rel_path: PathBuf::from("sound"),
            children_loaded: true,
            entries_loaded: true,
            ..Default::default()
        }],
        entries: Vec::new(),
    };

    replace_loaded_tree_scope(&mut tree, Path::new(""), Path::new("sound"), &entries);

    let sound_node = &tree.children[0];
    assert!(sound_node.children_loaded && sound_node.entries_loaded);
    assert_eq!(
        crate::app::browser::collect_sound_keys(sound_node, &entries),
        vec!["sound/a".to_owned(), "sound/sub/b".to_owned()]
    );
}
