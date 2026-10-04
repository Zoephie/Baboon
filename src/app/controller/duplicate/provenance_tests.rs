use super::*;
use crate::source::ContainerPackageIndex;

/// A stand-in for the mount's directory indexes: one `Vec` of paths per
/// container, matched exactly the way `IoStoreArchive` matches them.
struct FakePaths(Vec<Vec<&'static str>>);

impl ContainerPaths for FakePaths {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn contains(&self, container: usize, path: &str) -> bool {
        self.0
            .get(container)
            .is_some_and(|paths| paths.iter().any(|entry| *entry == path))
    }

    fn find_ignoring_case(&self, container: usize, path: &str) -> Option<String> {
        self.0
            .get(container)?
            .iter()
            .find(|entry| entry.eq_ignore_ascii_case(path))
            .map(|entry| (*entry).to_owned())
    }
}

const MARINE_UBULK: &str =
    "Meteorite/Content/Tags/objects/characters/Marine/marine-biped.ubulk";
const MARINE_UASSET: &str =
    "Meteorite/Content/Tags/objects/characters/Marine/marine-biped.uasset";

#[test]
fn the_indexed_wrapper_wins_over_the_one_that_would_be_assembled() {
    // The package index records what mounting saw. Even when swapping the
    // extension happens to produce a path that also exists, the recorded
    // one is the answer, because it is the one provenance can vouch for.
    let mut packages = ContainerPackageIndex::default();
    packages.insert(
        "/game/tags/objects/characters/marine/marine-biped".to_owned(),
        1,
        MARINE_UASSET.to_owned(),
    );
    let containers = FakePaths(vec![vec![], vec![MARINE_UBULK, MARINE_UASSET]]);
    let resolved =
        resolve_source_uasset_in(&containers, &packages, 1, MARINE_UBULK).expect("resolved");
    assert_eq!(resolved.rel_path, MARINE_UASSET);
    assert_eq!(resolved.container, 1);
    assert_eq!(resolved.how, "package index");
}

#[test]
fn a_wrapper_spelt_with_different_case_is_still_found() {
    // The Marine case. The payload's folder is `Marine`, the wrapper's is
    // `marine` — the same package to Unreal, whose chunk ids hash the
    // lowercased name, and two different keys to a container directory
    // index, which is matched byte for byte. Swapping the extension
    // produces a path no container holds.
    const WRAPPER: &str =
        "Meteorite/Content/Tags/objects/characters/marine/marine-biped.uasset";
    let containers = FakePaths(vec![vec![MARINE_UBULK, WRAPPER]]);
    let resolved = resolve_source_uasset_in(
        &containers,
        &ContainerPackageIndex::default(),
        0,
        MARINE_UBULK,
    )
    .expect("resolved despite the case difference");
    assert_eq!(resolved.rel_path, WRAPPER);
    assert_eq!(resolved.how, "same container, different case");
}

#[test]
fn a_mod_carrying_only_the_payload_falls_back_to_the_layer_beneath_it() {
    // Mounted last-wins, so the mod at index 1 provides the tag; the only
    // wrapper there is is the game's own, one layer down.
    let containers = FakePaths(vec![vec![MARINE_UBULK, MARINE_UASSET], vec![MARINE_UBULK]]);
    let resolved = resolve_source_uasset_in(
        &containers,
        &ContainerPackageIndex::default(),
        1,
        MARINE_UBULK,
    )
    .expect("resolved through the lower layer");
    assert_eq!(resolved.container, 0);
    assert_eq!(resolved.how, "lower-priority container");
}

#[test]
fn a_stale_package_index_entry_does_not_win() {
    // The index says container 2 has it; container 2 does not. A recorded
    // path that no longer resolves is provenance that has gone stale, and
    // trusting it would fail the read with the index's answer rather than
    // finding the copy that is actually there.
    let mut packages = ContainerPackageIndex::default();
    packages.insert(
        "/game/tags/objects/characters/marine/marine-biped".to_owned(),
        2,
        MARINE_UASSET.to_owned(),
    );
    let containers = FakePaths(vec![vec![MARINE_UBULK, MARINE_UASSET], vec![], vec![]]);
    let resolved =
        resolve_source_uasset_in(&containers, &packages, 0, MARINE_UBULK).expect("resolved");
    assert_eq!(resolved.container, 0);
    assert_eq!(resolved.how, "same container");
}

#[test]
fn a_missing_wrapper_says_what_it_looked_for() {
    let containers = FakePaths(vec![vec![MARINE_UBULK]]);
    let error = resolve_source_uasset_in(
        &containers,
        &ContainerPackageIndex::default(),
        0,
        MARINE_UBULK,
    )
    .expect_err("nothing carries the wrapper");
    assert!(error.contains(MARINE_UASSET), "{error}");
    assert!(error.contains("in any case"), "{error}");
}

#[test]
fn the_destination_is_built_from_the_source_path_not_the_display_path() {
    // The display path is lowercased and dotted (`objects/characters/marine
    // /marine.biped`); the container path is neither. The destination has
    // to inherit the container's folder, in the container's case, or the
    // copy lands in a folder the container does not have.
    let paths = container_duplicate_paths(
        MARINE_UBULK,
        "objects/characters/marine/marine.biped",
        "marine_copy",
    )
    .expect("built");
    assert_eq!(
        paths.ubulk,
        "Meteorite/Content/Tags/objects/characters/Marine/marine_copy-biped.ubulk"
    );
    assert_eq!(
        paths.uasset,
        "Meteorite/Content/Tags/objects/characters/Marine/marine_copy-biped.uasset"
    );
    assert_eq!(
        paths.package,
        "/Game/Tags/objects/characters/Marine/marine_copy-biped"
    );
    assert_eq!(paths.display, "objects/characters/marine/marine_copy.biped");
}

#[test]
fn the_content_root_is_stripped_however_it_is_capitalised() {
    for rel in [
        "Meteorite/Content/Tags/objects/x-biped.ubulk",
        "meteorite/content/Tags/objects/x-biped.ubulk",
        "METEORITE/CONTENT/Tags/objects/x-biped.ubulk",
    ] {
        assert_eq!(
            super::super::container_rel_to_package_path(rel).as_deref(),
            Some("/Game/Tags/objects/x-biped"),
            "{rel}"
        );
    }
}

#[test]
fn the_diagnostic_names_every_path_a_report_would_need() {
    let diagnostics = DuplicateDiagnostics {
        display_path: "objects/characters/marine/marine.biped".to_owned(),
        source_container: 3,
        source_container_label: "pakchunk0-WinGDK".to_owned(),
        source_utoc: PathBuf::from("D:/Game/Paks/pakchunk0-WinGDK.utoc"),
        source_ubulk: MARINE_UBULK.to_owned(),
        source_uasset: MARINE_UASSET.to_owned(),
        source_uasset_container: "pakchunk0-WinGDK".to_owned(),
        source_uasset_how: "package index",
        source_package: "/game/tags/objects/characters/marine/marine-biped".to_owned(),
        package_basename: "marine-biped.uasset".to_owned(),
        destination_package: "/Game/Tags/objects/characters/Marine/marine_copy-biped"
            .to_owned(),
        destination_uasset: "…/marine_copy-biped.uasset".to_owned(),
        destination_ubulk: "…/marine_copy-biped.ubulk".to_owned(),
    };
    let text = diagnostics.to_string();
    for expected in [
        "objects/characters/marine/marine.biped",
        "pakchunk0-WinGDK",
        "pakchunk0-WinGDK.utoc",
        MARINE_UBULK,
        MARINE_UASSET,
        "package index",
        "/game/tags/objects/characters/marine/marine-biped",
        "marine-biped.uasset",
        "/Game/Tags/objects/characters/Marine/marine_copy-biped",
    ] {
        assert!(text.contains(expected), "missing {expected} in:\n{text}");
    }
}
