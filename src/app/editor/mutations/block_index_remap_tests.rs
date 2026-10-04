use super::*;

fn test_tag() -> TagFile {
    TagFile::new(crate::app::test_definition_path(
        "haloreach_mcc/test_tag.json",
    ))
    .expect("load test-tag definition")
}

fn add_basic_elements(tag: &mut TagFile, count: usize) {
    for _ in 0..count {
        apply_one_block_op(
            tag,
            &BlockOp {
                path: "basic block".to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .expect("add basic block element");
    }
}

fn set_test_indices(tag: &mut TagFile, char_index: i64, short_index: i64, long_index: i64) {
    apply_field_edit(tag, "char block index", &char_index.to_string()).unwrap();
    apply_field_edit(tag, "short block index", &short_index.to_string()).unwrap();
    apply_field_edit(tag, "long block index", &long_index.to_string()).unwrap();
}

fn test_indices(tag: &TagFile) -> [i128; 3] {
    let root = tag.root();
    [
        root.read_int_any("char block index").unwrap(),
        root.read_int_any("short block index").unwrap(),
        root.read_int_any("long block index").unwrap(),
    ]
}

#[test]
fn insert_and_delete_preserve_declared_block_index_targets() {
    let mut tag = test_tag();
    add_basic_elements(&mut tag, 3);
    set_test_indices(&mut tag, 0, 1, 2);

    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "basic block".to_owned(),
            kind: BlockOpKind::Insert(1),
        },
    )
    .unwrap();
    assert_eq!(test_indices(&tag), [0, 2, 3]);

    // Removing the newly inserted element restores every old position.
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "basic block".to_owned(),
            kind: BlockOpKind::Delete(1),
        },
    )
    .unwrap();
    assert_eq!(test_indices(&tag), [0, 1, 2]);

    // A reference to the removed entry becomes <none>; later references
    // move down while earlier references remain unchanged.
    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "basic block".to_owned(),
            kind: BlockOpKind::Delete(1),
        },
    )
    .unwrap();
    assert_eq!(test_indices(&tag), [0, -1, 1]);
}

#[test]
fn duplicate_shifts_only_entries_after_the_copy_source() {
    let mut tag = test_tag();
    add_basic_elements(&mut tag, 3);
    set_test_indices(&mut tag, 0, 1, 2);

    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "basic block".to_owned(),
            kind: BlockOpKind::Duplicate(1),
        },
    )
    .unwrap();
    assert_eq!(test_indices(&tag), [0, 1, 3]);
}

#[test]
fn general_mapping_is_ready_for_future_reordering() {
    let mut tag = test_tag();
    add_basic_elements(&mut tag, 3);
    set_test_indices(&mut tag, 0, 1, 2);

    // Future table reorder: old [0, 1, 2] becomes new [2, 0, 1].
    let remap = BlockElementRemap {
        old_to_new: vec![Some(2), Some(0), Some(1)],
        excluded_new_elements: None,
    };
    assert_eq!(
        remap_block_index_references(&mut tag, "basic block", &remap).unwrap(),
        3
    );
    assert_eq!(test_indices(&tag), [2, 0, 1]);
}

#[test]
fn nested_declared_reference_resolves_its_ancestor_target() {
    let mut tag =
        TagFile::new(crate::app::test_definition_path("halo2_mcc/model.json")).unwrap();
    add_elements_at(&mut tag, "variants", 3);
    add_elements_at(&mut tag, "variants[0]/regions", 1);
    apply_field_edit(&mut tag, "variants[0]/regions[0]/parent variant", "2").unwrap();

    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "variants".to_owned(),
            kind: BlockOpKind::Insert(1),
        },
    )
    .unwrap();
    assert_eq!(
        tag.root()
            .descend("variants[0]/regions[0]")
            .and_then(|region| region.read_int_any("parent variant")),
        Some(3)
    );
}

#[test]
fn classic_parent_node_reference_is_remapped() {
    let mut tag =
        TagFile::new(crate::app::test_definition_path("haloce_mcc/model.json")).unwrap();
    add_elements_at(&mut tag, "nodes", 3);
    apply_field_edit(&mut tag, "nodes[0]/parent node index", "2").unwrap();

    apply_one_block_op(
        &mut tag,
        &BlockOp {
            path: "nodes".to_owned(),
            kind: BlockOpKind::Insert(1),
        },
    )
    .unwrap();
    assert_eq!(
        tag.root()
            .descend("nodes[0]")
            .and_then(|node| node.read_int_any("parent node index")),
        Some(3)
    );
}

fn add_elements_at(tag: &mut TagFile, path: &str, count: usize) {
    for _ in 0..count {
        apply_one_block_op(
            tag,
            &BlockOp {
                path: path.to_owned(),
                kind: BlockOpKind::Add,
            },
        )
        .unwrap();
    }
}
