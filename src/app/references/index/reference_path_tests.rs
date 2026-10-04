
use super::*;

#[test]
fn ancestor_block_indices_splits_indexed_path() {
    // Nested blocks: each pair's path is the drawn `path_prefix` (parent
    // indices kept, own index dropped).
    assert_eq!(
        ancestor_block_indices("custom references[3]/sounds[1]/melee sound"),
        vec![
            ("custom references".to_owned(), 3),
            ("custom references[3]/sounds".to_owned(), 1),
        ],
    );
    // A plain struct segment between blocks carries no selection.
    assert_eq!(
        ancestor_block_indices("weapon[2]/melee/damage sound"),
        vec![("weapon".to_owned(), 2)],
    );
    // A top-level (unindexed) reference field has no ancestor blocks.
    assert_eq!(
        ancestor_block_indices("havok cleanup resources"),
        Vec::<(String, usize)>::new(),
    );
    assert_eq!(
        ancestor_block_indices("custom references#5[3]/sounds#2[1]/melee sound#4"),
        vec![
            ("custom references#5".to_owned(), 3),
            ("custom references#5[3]/sounds#2".to_owned(), 1),
        ],
    );
    // Foundation renders inherited wrappers without ordinals, so selector
    // IDs beneath Unit/Object must preserve those plain wrapper segments.
    assert_eq!(
        ancestor_block_indices("unit/object/functions#25[2]/import name#3"),
        vec![("unit/object/functions#25".to_owned(), 2)],
    );
    // Reference-jump paths may retain schema ordinals on inherited wrappers;
    // normalize those to the same selector ID as canonical Find paths.
    assert_eq!(
        ancestor_block_indices("unit#0/object#0/functions#25[2]/import name#3"),
        vec![("unit/object/functions#25".to_owned(), 2)],
    );
}

#[test]
fn occurrence_label_keeps_indices_and_cleans_names() {
    assert_eq!(
        occurrence_label("custom references[3]/melee sound"),
        "custom references[3] › melee sound",
    );
    assert_eq!(
        occurrence_label("havok cleanup resources"),
        "havok cleanup resources"
    );
    assert_eq!(
        occurrence_label("custom references#5[3]/melee sound#4"),
        "custom references[3] › melee sound",
    );
}

#[test]
fn normalize_ref_matches_dependency_key_form() {
    assert_eq!(
        normalize_ref("Sound/Materials/Hard/Human_Weap_Melee"),
        normalize_ref("sound\\materials\\hard\\human_weap_melee"),
    );
}
