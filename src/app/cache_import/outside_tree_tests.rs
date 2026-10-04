use super::*;

fn reference(path: &str) -> OutsideReference {
    OutsideReference {
        key: format!("cache:bitm:{}", path.replace('/', "\\")),
        display_path: path.to_owned(),
    }
}

/// A folder holds what is under it, however deep that is.
///
/// The point of the tree over the flat list it replaced: `objects` counting
/// only the tags directly inside it would report two thousand tags as none,
/// and ticking it would bring nothing.
#[test]
fn a_folder_counts_and_carries_its_whole_subtree() {
    let tree = OutsideTree::build(&[
        reference("objects/characters/elite/elite.biped"),
        reference("objects/characters/elite/bitmaps/elite_diffuse.bitmap"),
        reference("objects/weapons/rifle/assault_rifle.weapon"),
        reference("fx/decals/scorch.bitmap"),
    ]);

    assert_eq!(
        tree.roots.iter().cloned().collect::<Vec<_>>(),
        ["fx", "objects"]
    );
    assert_eq!(tree.totals.get("objects").copied(), Some(3));
    assert_eq!(
        tree.totals.get("objects/characters/elite").copied(),
        Some(2)
    );
    assert_eq!(tree.keys_under("objects").len(), 3);
    assert_eq!(tree.keys_under("objects/weapons").len(), 1);
    // The leaves hang off the folder that actually holds them, not off the
    // first two segments the old grouping used.
    assert_eq!(
        tree.tags
            .get("objects/characters/elite")
            .map(|tags| tags.len()),
        Some(1),
    );
}

/// A folder's tick reports how much of it is chosen, not just whether any is.
#[test]
fn a_partly_chosen_folder_says_so() {
    let references = [
        reference("objects/characters/elite/elite.biped"),
        reference("objects/characters/elite/bitmaps/elite_diffuse.bitmap"),
        reference("objects/weapons/rifle/assault_rifle.weapon"),
    ];
    let tree = OutsideTree::build(&references);
    let mut picked: BTreeMap<String, bool> =
        references.iter().map(|r| (r.key.clone(), false)).collect();

    assert_eq!(tree.tally("objects", &picked), (0, 3));
    picked.insert(references[0].key.clone(), true);
    assert_eq!(tree.tally("objects", &picked), (1, 3));
    assert_eq!(tree.tally("objects/characters", &picked), (1, 2));
    assert_eq!(tree.tally("objects/weapons", &picked), (0, 1));
    for wanted in picked.values_mut() {
        *wanted = true;
    }
    assert_eq!(tree.tally("objects", &picked), (3, 3));
}

/// A tag with no folder still lands somewhere the window can draw it.
#[test]
fn a_tag_at_the_top_is_not_lost() {
    let tree = OutsideTree::build(&[reference("globals.globals")]);
    assert!(tree.roots.is_empty());
    assert_eq!(tree.tags.get("").map(|tags| tags.len()), Some(1));
    assert_eq!(tree.keys_under("").len(), 1);
}
