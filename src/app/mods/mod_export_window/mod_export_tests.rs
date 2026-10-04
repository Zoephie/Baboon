use super::*;
use std::path::PathBuf;

/// The reported case, from `review-diagnostic.json`: two top-level fields
/// changed, `zone set pvs[3]` deleted, a `zone sets` element added. What the
/// reporter asked to see is exactly three things, in the containers that
/// hold them -- not 131 rows of shifted indices.
#[test]
fn tree_matches_the_reported_case() {
    fn row(path: &str, base: Option<&str>, before: &str, after: &str) -> TagFieldDiff {
        TagFieldDiff {
            path: path.to_owned(),
            base_path: base.map(str::to_owned),
            a: before.to_owned(),
            b: after.to_owned(),
        }
    }
    let mut rows = vec![
        row("flags", Some("flags"), "0x0000 (none set)", "0x000E [...]"),
        row(
            "sandbox origin point",
            Some("sandbox origin point"),
            "x=0, y=-0, z=0",
            "x=1, y=2, z=3",
        ),
        row(
            "zone set pvs[3]",
            Some("zone set pvs[3]"),
            "removed — element 3",
            "",
        ),
    ];
    // The removed element's own fields follow it, and must fold into it.
    for field in ["structure bsp mask", "version"] {
        let path = format!("zone set pvs[3]/{field}");
        rows.push(row(&path, Some(&path), "11", ""));
    }
    rows.push(row("zone sets[5]", None, "", "added — element 5"));
    for field in ["cinematic zones", "hint previous zone set"] {
        rows.push(row(&format!("zone sets[5]/{field}"), None, "", "0"));
    }

    let sections = Baboon::build_diff_sections(&rows);
    let kinds: Vec<_> = sections
        .iter()
        .map(|s| (s.element.as_str(), s.kind, s.rows.len()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("", ModExportChange::Modified, 2),
            ("zone set pvs[3]", ModExportChange::Unresolved, 3),
            ("zone sets[5]", ModExportChange::New, 3),
        ],
    );

    let tree = Baboon::build_diff_tree(sections);
    // The two top-level field changes stay at the root; each block holds
    // only its own changed element.
    assert_eq!(tree.sections.len(), 1);
    assert_eq!(tree.sections[0].element, "");
    let containers: Vec<_> = tree
        .children
        .iter()
        .map(|c| (c.title.as_str(), c.sections.len(), c.children.len()))
        .collect();
    assert_eq!(
        containers,
        vec![("zone set pvs", 1, 0), ("zone sets", 1, 0)],
    );
}

/// A change buried several blocks deep reads as one breadcrumb, not a stack
/// of boxes each containing only the next.
#[test]
fn single_child_container_chains_collapse() {
    let section = DiffSection {
        element: "structure bsp pvs[0]/cluster pvs[0]/cluster pvs bit vectors[0]".to_owned(),
        base_element: None,
        label: String::new(),
        kind: ModExportChange::Modified,
        rows: Vec::new(),
        filter: Default::default(),
    };
    let tree = Baboon::build_diff_tree(vec![section]);
    assert_eq!(tree.children.len(), 1);
    let chain = &tree.children[0];
    // Intermediate containers keep their element index -- it is the only
    // place that says *which* cluster the change is in. The innermost one
    // drops it because the section row states it.
    assert_eq!(
        chain.title,
        "structure bsp pvs[0] › cluster pvs[0] › cluster pvs bit vectors",
    );
    assert_eq!(chain.sections.len(), 1);
    assert!(chain.children.is_empty());
}

fn dialog(name: &str) -> ModExportDialog {
    ModExportDialog {
        kit: KitId(0),
        review_only: false,
        snapshot: CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: PathBuf::new(),
            selected_identity: None,
            tabs: Vec::new(),
            overlays: Default::default(),
            history: Default::default(),
            folders: Default::default(),
        },
        rows: Vec::new(),
        name: name.to_owned(),
        folder: PathBuf::from("/tmp"),
        overwrite_acknowledged: false,
        expanded: Default::default(),
        diffs: Default::default(),
        controls_height: 0.0,
    }
}

/// `_P` is what gives a mod priority over the game's own containers, so it
/// is part of the name rather than something a rename can drop -- which is
/// exactly how a reported mod came to build correctly and do nothing.
#[test]
fn the_stem_always_carries_the_priority_suffix() {
    assert_eq!(dialog("h2a_magnum").stem(), "h2a_magnum_P");
    assert_eq!(dialog("h2a_magnum_P").stem(), "h2a_magnum_P");
    assert_eq!(dialog("  spaced  ").stem(), "spaced_P");
}

/// A heading names the block and the element within it, rather than an
/// indexed path the reader has to parse.
#[test]
fn an_element_path_splits_into_its_block_and_index() {
    assert_eq!(
        Baboon::split_element_index("zone set pvs[3]"),
        ("zone set pvs", Some(3))
    );
    // Nested: the chain stays, so it is clear which block is meant.
    assert_eq!(
        Baboon::split_element_index("weapons[2]/triggers[0]"),
        ("weapons[2]/triggers", Some(0))
    );
    // Not an element at all.
    assert_eq!(Baboon::split_element_index("flags"), ("flags", None));
}

/// Changes nest, and the innermost element is the one worth heading. The
/// whole chain is kept so it is unambiguous which element that is.
#[test]
fn a_diff_path_splits_into_its_element_and_field() {
    assert_eq!(
        Baboon::split_element_path("weapons[2]/triggers[0]/barrels[1]/damage"),
        ("weapons[2]/triggers[0]/barrels[1]", "damage")
    );
    // A row about the element itself -- added, removed or moved -- has no
    // field part, which is how the renderer tells the two apart.
    assert_eq!(
        Baboon::split_element_path("vehicle palette[3]"),
        ("vehicle palette[3]", "")
    );
    // A field at the top level of the tag belongs to no element.
    assert_eq!(Baboon::split_element_path("flags"), ("", "flags"));
}

/// The name becomes three file names in a folder the user never types, so
/// spaces and punctuation are separators to normalise, not characters to
/// carry through -- while the user's own capitalisation is theirs to keep.
#[test]
fn a_mod_name_becomes_a_file_safe_stem() {
    assert_eq!(sanitize_mod_name("My Cool Mod"), "My-Cool-Mod");
    assert_eq!(sanitize_mod_name("h2a magnum!"), "h2a-magnum");
    assert_eq!(sanitize_mod_name("  trimmed  "), "trimmed");
    // Path syntax cannot survive: these become three files somewhere the
    // user did not choose.
    assert_eq!(sanitize_mod_name("../../etc/passwd"), "etc-passwd");
    assert_eq!(sanitize_mod_name("my:mod?"), "my-mod");
    // Underscores stay, so `_P` keeps meaning what it means.
    assert_eq!(sanitize_mod_name("my_mod_P"), "my_mod_P");
}

/// The buffer holds what the user typed; only the file name is folded.
/// Folding as they type ate the space in "My Mod" before the second word
/// could be reached.
#[test]
fn a_name_is_folded_only_when_it_becomes_a_file_name() {
    assert_eq!(dialog("My Mod").stem(), "My-Mod_P");
    assert_eq!(dialog("My ").stem(), "My_P");
    // Already suffixed, in either case the game accepts.
    assert_eq!(dialog("thing_P").stem(), "thing_P");
    assert_eq!(dialog("thing_p").stem(), "thing_p");
}
