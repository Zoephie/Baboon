use super::*;

static CE_PAKS: std::sync::LazyLock<&'static str> =
    std::sync::LazyLock::new(|| crate::test_kits::leak(crate::test_kits::ce_paks()));

fn find_entry<'a>(
    loaded: &'a crate::core::source::LoadedSourceData,
    group: &[u8; 4],
    path: &str,
) -> &'a TagEntry {
    let group_tag = u32::from_be_bytes(*group);
    loaded
        .entries
        .iter()
        .chain(loaded.all_entries.iter())
        .find(|entry| {
            entry.group_tag == group_tag
                && entry.display_path.to_ascii_lowercase().replace('\\', "/") == path
        })
        .unwrap_or_else(|| panic!("no {path} entry in the mounted containers"))
}

/// Container tags carry no `want` stream, so their dependencies have to come
/// out of the parsed tag. This walks the real path end to end: a Campaign
/// Evolved biped must report outbound references, and the reverse index they
/// feed must resolve back to the referenced tag's own entry — i.e. the
/// reference strings inside a container tag normalize to the same key as the
/// entry display paths built from the pak directory. Skips without the paks.
#[test]
fn campaign_evolved_container_tags_report_their_dependencies() {
    let paks = PathBuf::from(*CE_PAKS);
    if !paks.exists() {
        eprintln!("skip: CE paks not found");
        return;
    }
    let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let loaded =
        crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
            .expect("mount CE container set");
    let names = TagNameIndex::load_game(&definitions, "haloce_evolved")
        .expect("load Campaign Evolved tag names");

    // The index build and every lookup read the complete set through
    // `full_entry_set`; a container mount keeps it in `entries`.
    assert!(loaded.all_entries.is_empty());
    assert_eq!(loaded.full_entry_set().len(), loaded.entries.len());
    assert!(
        loaded.full_entry_set().len() > 10_000,
        "expected the full CE tag set, got {}",
        loaded.full_entry_set().len()
    );

    let biped = find_entry(&loaded, b"bipd", "objects/characters/elite/elite.biped").clone();
    let deps = read_entry_dependencies(&loaded.source, &biped).expect("read biped deps");
    assert!(
        !deps.is_empty(),
        "elite.biped reported no dependencies; the container parse path is not running"
    );

    // The model reference must land on the entry the browser shows.
    let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
    let model_ref =
        dependency_entry_reference_path(&model, &names).expect("model reference path");
    let mut index = ReverseDependencyIndex::default();
    index.set_tag_dependencies(biped.key.clone(), deps);
    assert!(
        index
            .dependents_for(model.group_tag, &model_ref)
            .contains(&biped.key),
        "elite.model has no recorded referrer; container reference paths do not \
             normalize to entry display paths"
    );
}

/// The whole-corpus build, for when the cost of indexing containers is in
/// question — it parses every tag. Ignored by default (minutes in a debug
/// build); run with:
///   cargo test --release -- --ignored campaign_evolved_full_reference_index
#[test]
#[ignore = "parses all ~12k Campaign Evolved tags"]
fn campaign_evolved_full_reference_index_resolves_referrers() {
    let paks = PathBuf::from(*CE_PAKS);
    if !paks.exists() {
        eprintln!("skip: CE paks not found");
        return;
    }
    let definitions = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    let loaded =
        crate::core::source::load_iostore_container_set(paks, &TagNameIndex::default(), &definitions)
            .expect("mount CE container set");
    let names = TagNameIndex::load_game(&definitions, "haloce_evolved")
        .expect("load Campaign Evolved tag names");

    let started = std::time::Instant::now();
    let mut index = ReverseDependencyIndex::default();
    let mut failed = 0usize;
    for entry in loaded.full_entry_set() {
        match read_entry_dependencies(&loaded.source, entry) {
            Ok(deps) => index.set_tag_dependencies(entry.key.clone(), deps),
            Err(_) => failed += 1,
        }
    }
    eprintln!(
        "[perf] indexed {} tags in {:.1?} ({failed} unreadable)",
        loaded.full_entry_set().len(),
        started.elapsed()
    );
    assert_eq!(failed, 0, "some container tags could not be read");

    // A shared tag must come back with many referrers, and the elite biped
    // must be among the referrers of its own model.
    let model = find_entry(&loaded, b"hlmt", "objects/characters/elite/elite.model").clone();
    let model_ref =
        dependency_entry_reference_path(&model, &names).expect("model reference path");
    let referrers = index.dependents_for(model.group_tag, &model_ref);
    assert!(
        !referrers.is_empty(),
        "elite.model has no referrers in the full index"
    );
    let unreferenced = loaded
        .full_entry_set()
        .iter()
        .filter(|entry| {
            dependency_entry_reference_path(entry, &names)
                .map(|rel| index.dependents_for(entry.group_tag, &rel).is_empty())
                .unwrap_or(false)
        })
        .count();
    eprintln!(
        "[perf] {unreferenced} of {} tags are unreferenced",
        loaded.full_entry_set().len()
    );
    assert!(
        unreferenced < loaded.full_entry_set().len() / 2,
        "most tags came back unreferenced ({unreferenced}); reference paths are \
             probably not matching entry paths"
    );
}
