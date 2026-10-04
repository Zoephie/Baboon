use super::*;

static PAKS: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));

fn read_a15() -> Option<TagFile> {
    if !std::path::Path::new(*PAKS).exists() {
        return None;
    }
    let defs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let names = crate::core::format::TagNameIndex::load_from_definitions(&defs);
    let loaded = crate::core::source::load_iostore_container_set(
        std::path::PathBuf::from(*PAKS),
        &names,
        &defs,
    )
    .ok()?;
    let entry = loaded
        .entries
        .iter()
        .find(|entry| entry.display_path.ends_with("a15.scenario"))?
        .clone();
    crate::core::source::read_entry(&loaded.source, &entry).ok()
}

/// Deleting one `zone set pvs` element reported a screen of changes through
/// everything below it, each value shifted by exactly one position -- the
/// signature of comparing element n against element n+1.
///
/// These elements are a version, a mask and some flags, with everything
/// that tells them apart in nested blocks, so a fingerprint of the fixed
/// data alone could not distinguish them and the aligner paired the wrong
/// ones.
#[test]
fn deleting_a_zone_set_reports_only_that_deletion() {
    let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
        eprintln!("skipping: Campaign Evolved not present");
        return;
    };
    let names = crate::core::format::TagNameIndex::default();
    let before = base
        .root()
        .fields_all()
        .find_map(|field| {
            (field.name() == "zone set pvs")
                .then(|| field.as_block())
                .flatten()
                .map(|block| block.len())
        })
        .expect("a zone set pvs block");
    assert!(before > 4, "need several elements to shift, got {before}");

    let mut dirty = Dirty::default();
    crate::app::apply_block_ops(
        &mut edited,
        vec![BlockOp {
            path: "zone set pvs".to_owned(),
            kind: BlockOpKind::Delete(3),
        }],
        &mut dirty,
    );

    let (rows, _) = diff_tags(&base, &edited, &names, 5000);
    let removals: Vec<&TagFieldDiff> = rows
        .iter()
        .filter(|row| row.b.is_empty() && row.a.starts_with("removed"))
        .collect();
    assert_eq!(
        removals.len(),
        1,
        "one element was deleted, so one removal: {:?}",
        removals.iter().map(|row| &row.path).collect::<Vec<_>>()
    );

    // Everything else in the tag is untouched by a deletion further up, so
    // nothing outside the removed element may be reported as changed.
    let removed = &removals[0].path;
    let strays: Vec<&String> = rows
        .iter()
        .filter(|row| !row.path.starts_with(removed.as_str()))
        .map(|row| &row.path)
        .collect();
    assert!(
        strays.is_empty(),
        "{} field(s) outside the deleted element reported as changed, e.g. {:?}",
        strays.len(),
        strays.iter().take(5).collect::<Vec<_>>()
    );
}

/// The reported case, in full: an element deleted from `zone set pvs` and
/// a value edited inside one that shifts up to take its place.
///
/// The edit changes that element's contents, so it drops out of the common
/// subsequence and used to be paired positionally against its neighbour --
/// reporting every field below as changed, each shifted by one position.
#[test]
fn editing_an_element_that_also_shifts_is_still_the_same_element() {
    let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
        eprintln!("skipping: Campaign Evolved not present");
        return;
    };
    let names = crate::core::format::TagNameIndex::default();
    let mut dirty = Dirty::default();
    crate::app::apply_block_ops(
        &mut edited,
        vec![BlockOp {
            path: "zone set pvs".to_owned(),
            kind: BlockOpKind::Delete(3),
        }],
        &mut dirty,
    );
    // Element 4 is now element 3. Editing inside it is what defeated the
    // alignment.
    let applied = crate::app::apply_pending_edits(
        &mut edited,
        vec![PendingFieldEdit {
            path: "zone set pvs[3]/bsp checksums[0]/bsp checksum".to_owned(),
            input: "12345".to_owned(),
        }],
        &mut dirty,
    );
    assert!(
        applied.status.is_some(),
        "the edit has to land for this to test anything"
    );

    let (rows, _) = diff_tags(&base, &edited, &names, 5000);
    let modifications: Vec<&TagFieldDiff> = rows
        .iter()
        .filter(|row| !row.a.is_empty() && !row.b.is_empty())
        .collect();
    assert_eq!(
        modifications.len(),
        1,
        "one value was edited, so one modification: {:?}",
        modifications
            .iter()
            .map(|row| (&row.path, &row.a, &row.b))
            .collect::<Vec<_>>()
    );
    assert_eq!(modifications[0].b, "12345");
}

/// Deleting one element and adding another, as reported.
#[test]
fn a_deletion_and_an_addition_change_no_values() {
    let (Some(base), Some(mut edited)) = (read_a15(), read_a15()) else {
        eprintln!("skipping: Campaign Evolved not present");
        return;
    };
    let names = crate::core::format::TagNameIndex::default();
    let mut dirty = Dirty::default();
    crate::app::apply_block_ops(
        &mut edited,
        vec![
            BlockOp {
                path: "zone set pvs".to_owned(),
                kind: BlockOpKind::Delete(3),
            },
            BlockOp {
                path: "zone sets".to_owned(),
                kind: BlockOpKind::Add,
            },
        ],
        &mut dirty,
    );
    // The dialog compares the shipped tag against the project overlay,
    // which is the edited tag serialized and read back -- not the
    // in-memory one it was edited in.
    let bytes = edited.write_to_bytes().expect("serialize the edited tag");
    let edited = TagFile::read_from_bytes(&bytes).expect("read the overlay back");
    let (rows, _) = diff_tags(&base, &edited, &names, 5000);
    // Removing one element and adding another changes no value anywhere,
    // so nothing may be reported as modified: renumbering is not an edit.
    let modifications: Vec<&TagFieldDiff> = rows
        .iter()
        .filter(|row| !row.a.is_empty() && !row.b.is_empty())
        .collect();
    assert!(
        modifications.is_empty(),
        "{} field(s) reported as changed by a deletion and an addition: {:?}",
        modifications.len(),
        modifications
            .iter()
            .take(5)
            .map(|row| (&row.path, &row.a, &row.b))
            .collect::<Vec<_>>()
    );
    assert!(
        rows.iter().any(|row| row.b.starts_with("added")),
        "the new element should be reported"
    );
    assert!(
        rows.iter().any(|row| row.a.starts_with("removed")),
        "the deleted element should be reported"
    );
}
