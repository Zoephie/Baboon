//! Moving a renamed tag through the mounted source without reloading it.
//!
//! Three indices, an entry list, two trees and a reference graph all describe
//! where a container tag lives. A reload would rebuild all of them, and is what
//! the code deliberately avoids — so each one has to be moved by hand, and the
//! order of two of those moves matters.

use super::*;

const GROUP: u32 = 0x6269_7064; // 'bipd'
const OLD_UBULK: &str = "Meteorite/Content/Tags/objects/vehicles/warthog-vehicle.ubulk";
const NEW_UBULK: &str = "Meteorite/Content/Tags/objects/vehicles/scorpion-vehicle.ubulk";
const OLD_UASSET: &str = "Meteorite/Content/Tags/objects/vehicles/warthog-vehicle.uasset";
const NEW_UASSET: &str = "Meteorite/Content/Tags/objects/vehicles/scorpion-vehicle.uasset";
const OLD_PACKAGE: &str = "/Game/Tags/objects/vehicles/warthog-vehicle";
const NEW_PACKAGE: &str = "/Game/Tags/objects/vehicles/scorpion-vehicle";
const OLD_KEY: &str = "ublock:pakchunk0:objects/vehicles/warthog";
const NEW_KEY: &str = "ublock:pakchunk0:objects/vehicles/scorpion";

fn entry(key: &str, logical: &str, rel_path: &str) -> TagEntry {
    TagEntry {
        key: key.to_owned(),
        display_path: format!("{logical}.vehicle"),
        group_tag: GROUP,
        group_name: Some("vehicle".to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: rel_path.to_owned(),
        },
    }
}

/// A mounted container source holding one tag, indexed exactly as the mount
/// would have indexed it.
fn source_with_one_tag() -> LoadedSourceData {
    let mut index = crate::source::ContainerTagIndex::default();
    index.insert(
        crate::source::container_ref_key(GROUP, "objects/vehicles/warthog"),
        0,
        OLD_UBULK.to_owned(),
    );
    let mut packages = crate::source::ContainerPackageIndex::default();
    packages.insert(OLD_PACKAGE.to_ascii_lowercase(), 0, OLD_UASSET.to_owned());
    let mut shipped = crate::source::ShippedTagIndex::default();
    shipped.insert(OLD_UBULK, 0);

    let entries = vec![
        entry(OLD_KEY, "objects/vehicles/warthog", OLD_UBULK),
        // A neighbour, so the sorted re-insert has something to sort against.
        entry(
            "ublock:pakchunk0:objects/vehicles/mongoose",
            "objects/vehicles/mongoose",
            "Meteorite/Content/Tags/objects/vehicles/mongoose-vehicle.ubulk",
        ),
    ];
    LoadedSourceData {
        label: "Campaign Evolved".to_owned(),
        source: TagSource::IoStoreContainerSet {
            root: PathBuf::from("D:/Paks"),
            containers: Vec::new(),
            index: Arc::new(index),
            packages: Arc::new(packages),
            shipped: Arc::new(shipped),
        },
        names: TagNameIndex::default(),
        game: Some("haloce_evolved".to_owned()),
        entries,
        tree: TagTree::default(),
        group_tree: TagTree::default(),
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
    }
}

fn move_request(redirect: bool) -> ContainerRenameMove<'static> {
    ContainerRenameMove {
        container: 0,
        group_tag: GROUP,
        old_package: OLD_PACKAGE,
        new_package: NEW_PACKAGE,
        old_uasset_path: OLD_UASSET,
        new_uasset_path: NEW_UASSET,
        old_ubulk_path: OLD_UBULK,
        new_ubulk_path: NEW_UBULK,
        is_mod: false,
        redirect,
    }
}

fn indices(
    source: &LoadedSourceData,
) -> (
    &crate::source::ContainerTagIndex,
    &crate::source::ContainerPackageIndex,
    &crate::source::ShippedTagIndex,
) {
    let TagSource::IoStoreContainerSet {
        index,
        packages,
        shipped,
        ..
    } = &source.source
    else {
        panic!("the fixture is a container source");
    };
    (index, packages, shipped)
}

fn apply(source: &mut LoadedSourceData, redirect: bool) {
    apply_container_rename_source_state(
        source,
        OLD_KEY,
        &entry(NEW_KEY, "objects/vehicles/scorpion", NEW_UBULK),
        &move_request(redirect),
        &[],
    )
    .expect("the fixture is a container source");
}

#[test]
fn the_tag_leaves_its_old_path_and_arrives_at_the_new_one() {
    let mut source = source_with_one_tag();
    apply(&mut source, false);

    let (index, packages, shipped) = indices(&source);
    assert_eq!(
        index.lookup(GROUP, "objects/vehicles/scorpion"),
        Some((0, NEW_UBULK))
    );
    assert_eq!(index.lookup(GROUP, "objects/vehicles/warthog"), None);
    assert_eq!(packages.lookup(NEW_PACKAGE), Some((0, NEW_UASSET)));
    assert_eq!(packages.lookup(OLD_PACKAGE), None);
    assert_eq!(shipped.container_for(NEW_UBULK), Some(0));
    assert_eq!(shipped.container_for(OLD_UBULK), None);

    assert!(source.entries.iter().all(|entry| entry.key != OLD_KEY));
    assert!(source.entries.iter().any(|entry| entry.key == NEW_KEY));
}

/// The package index is first-insert-wins, so a rename that inserted before it
/// removed would leave the new package resolving to the old, now-retired path
/// — and nothing would report an error.
#[test]
fn the_package_index_is_not_left_pointing_at_the_retired_path() {
    let mut source = source_with_one_tag();
    // Seed the destination as though it had been in use before, which is the
    // case a first-insert-wins map gets wrong.
    {
        let TagSource::IoStoreContainerSet { packages, .. } = &mut source.source else {
            unreachable!()
        };
        Arc::make_mut(packages).insert(NEW_PACKAGE.to_ascii_lowercase(), 0, OLD_UASSET.to_owned());
    }
    apply(&mut source, false);
    let (_, packages, _) = indices(&source);
    assert_eq!(
        packages.lookup(NEW_PACKAGE),
        Some((0, NEW_UASSET)),
        "the stale row was replaced, not kept"
    );
}

/// A redirect makes the old path still *resolve*; it does not make it
/// *browsable*. Mirroring that in Baboon's own index is what keeps in-app
/// reference navigation agreeing with what the game will do.
#[test]
fn a_redirect_leaves_the_old_reference_resolving_to_the_new_home() {
    let mut source = source_with_one_tag();
    apply(&mut source, true);

    let (index, _, _) = indices(&source);
    assert_eq!(
        index.lookup(GROUP, "objects/vehicles/warthog"),
        Some((0, NEW_UBULK)),
        "a reference to the old path follows the tag"
    );
    assert_eq!(
        index.lookup(GROUP, "objects/vehicles/scorpion"),
        Some((0, NEW_UBULK))
    );
    // Resolvable, but gone from the browser, which draws from `entries`.
    assert!(source.entries.iter().all(|entry| entry.key != OLD_KEY));
}

/// The browser draws a folder in entry-vector order, so an entry pushed onto
/// the end lands at the bottom of its folder rather than where the user will
/// look for it.
#[test]
fn the_renamed_entry_is_filed_in_order_rather_than_appended() {
    let mut source = source_with_one_tag();
    apply(&mut source, false);
    let paths: Vec<&str> = source
        .entries
        .iter()
        .map(|entry| entry.display_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "objects/vehicles/mongoose.vehicle",
            "objects/vehicles/scorpion.vehicle"
        ]
    );
}

/// Dropped rather than patched: the index is keyed by tag key on both sides, so
/// a rename moves rows this cannot enumerate. An absent index is rebuilt on the
/// next query; a half-patched one answers wrongly and says nothing.
#[test]
fn the_reference_graph_is_dropped_rather_than_half_moved() {
    let mut source = source_with_one_tag();
    source.reverse_dependencies = Some(crate::source::ReverseDependencyIndex::default());
    apply(&mut source, false);
    assert!(source.reverse_dependencies.is_none());
}

/// A mod's paths are not the game's, so the shipped index — which answers "what
/// does the game itself carry here?" — must not learn a mod's rename as though
/// it were shipped content.
#[test]
fn renaming_inside_a_mod_leaves_the_shipped_index_alone() {
    let mut source = source_with_one_tag();
    let mut request = move_request(false);
    request.is_mod = true;
    apply_container_rename_source_state(
        &mut source,
        OLD_KEY,
        &entry(NEW_KEY, "objects/vehicles/scorpion", NEW_UBULK),
        &request,
        &[],
    )
    .expect("a container source");

    let (_, _, shipped) = indices(&source);
    assert_eq!(
        shipped.container_for(OLD_UBULK),
        Some(0),
        "the game still ships the tag at its own path"
    );
    assert_eq!(shipped.container_for(NEW_UBULK), None);
}
