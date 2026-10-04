//! Moving a tag's identity without losing anything filed under it.
//!
//! `TagEntry::key` is what documents, tabs, previews, undo history and the
//! keyword sidecar are all addressed by, so a rename has to carry every one of
//! them. A map that gets missed does not crash — it strands that state under a
//! key nothing resolves any more, and from the outside the rename looks as
//! though it worked. These tests are the cheapest place to catch that.

use super::*;
use crate::app::kits::KitView;
use crate::app::browser::KitBrowser;
use crate::app::editor::EditorCaches;
use crate::app::shell::session::RestorePlan;

const OLD: &str = "ublock:pakchunk0:objects/vehicles/warthog";
const NEW: &str = "ublock:pakchunk0:objects/vehicles/scorpion";
/// A second tag in the same kit, which must come through untouched.
const BYSTANDER: &str = "ublock:pakchunk0:objects/weapons/magnum";

fn definition(group: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("definitions")
        .join("haloce_evolved")
        .join(format!("{group}.json"))
}

fn document() -> TagDocument {
    let tag = TagFile::new(definition("cinematic_scene")).expect("build a tag from the CE schema");
    TagDocument::modified(tag)
}

/// A kit and its view holding something for `OLD` in every map a rename has
/// to carry, plus the same state for a bystander tag.
fn kit_with_state() -> (Kit, KitView) {
    let mut kit = Kit::empty(KitId(1), TagNameIndex::default());
    let mut view = KitView::for_test(&kit);
    for key in [OLD, BYSTANDER] {
        kit.parsed_tags.insert(key.to_owned(), document());
        kit.restore.pending_history
            .insert(key.to_owned(), TagHistory::default());
        view.caches.bitmap_previews
            .insert(key.to_owned(), BitmapPreviewState::default());
        view.caches.model_previews
            .insert(key.to_owned(), ModelPreviewState::default());
        view.caches.ce_sound_bindings.insert(
            key.to_owned(),
            std::sync::Arc::new(crate::core::source::ce_audio::CeSoundBinding::default()),
        );
        view.pending_expand.insert(key.to_owned(), true);
        view.find_filter_applied.insert(
            key.to_owned(),
            AppliedFindFilter {
                signature: "shield".to_owned(),
                filter: Default::default(),
            },
        );
        kit.loading_tags.insert(key.to_owned());
        kit.keywords.add(key, "vehicle");
        kit.open_tabs.push(key.to_owned());
        kit.restore.pending_restore_tags.push(LastSessionTag {
            key: key.to_owned(),
            label: key.to_owned(),
            group_tag: 0,
            path: None,
        });
        view.edit_buffers
            .insert_clean(format!("{key}|name"), "typed".to_owned());
    }
    kit.selected_key = Some(OLD.to_owned());
    view.tag_tree = egui_tiles::Tree::new_tabs(
        tag_tree_id(kit.id),
        vec![OLD.to_owned(), BYSTANDER.to_owned()],
    );
    view.caches.rmdf_cache.insert("shaders/foo".to_owned(), None);
    view.caches.rmop_cache.insert("shaders/bar".to_owned(), None);
    view.browser.modified_signature = vec![OLD.to_owned()];
    (kit, view)
}

/// Sorted, because `Tiles::iter` is not in tab order — what matters here is
/// which keys the panes carry, not where they sit.
fn panes(view: &KitView) -> Vec<String> {
    let mut keys: Vec<String> = view
        .tag_tree
        .tiles
        .iter()
        .filter_map(|(_, tile)| match tile {
            egui_tiles::Tile::Pane(key) => Some(key.clone()),
            _ => None,
        })
        .collect();
    keys.sort();
    keys
}

#[test]
fn a_rekey_carries_every_map_the_old_key_addressed() {
    let (mut kit, mut view) = kit_with_state();
    let before = kit.generation;
    rekey_tag_in_kit(&mut kit, &mut view, OLD, NEW);

    assert!(!kit.parsed_tags.contains_key(OLD));
    assert!(kit.parsed_tags.contains_key(NEW));
    assert!(kit.restore.pending_history.contains_key(NEW));
    assert!(view.caches.bitmap_previews.contains_key(NEW));
    assert!(view.caches.model_previews.contains_key(NEW));
    assert!(view.caches.ce_sound_bindings.contains_key(NEW));
    assert_eq!(view.pending_expand.get(NEW), Some(&true));
    assert_eq!(
        view.find_filter_applied
            .get(NEW)
            .map(|applied| applied.signature.as_str()),
        Some("shield")
    );
    assert!(!kit.loading_tags.contains(OLD) && kit.loading_tags.contains(NEW));
    assert_eq!(kit.keywords.keywords(NEW), ["vehicle".to_owned()]);
    assert!(kit.keywords.keywords(OLD).is_empty());
    assert_eq!(kit.selected_key.as_deref(), Some(NEW));
    assert_eq!(kit.open_tabs, vec![NEW.to_owned(), BYSTANDER.to_owned()]);
    assert_eq!(panes(&view), vec![NEW.to_owned(), BYSTANDER.to_owned()]);
    assert!(
        kit.restore.pending_restore_tags
            .iter()
            .any(|staged| staged.key == NEW)
    );

    // The generation is what makes any of it visible: the browser's filter
    // cache, the deletable-key set and the field index are all keyed on it.
    assert_ne!(kit.generation, before);
}

/// The document is carried, not rebuilt. A rename that re-registered the tag
/// would produce a document that is equally present and has quietly lost the
/// unsaved edits and the undo stack that made it worth keeping open.
#[test]
fn the_document_keeps_its_unsaved_state_across_the_rename() {
    let (mut kit, mut view) = kit_with_state();
    assert!(kit.parsed_tags[OLD].dirty.is_set());
    rekey_tag_in_kit(&mut kit, &mut view, OLD, NEW);
    assert!(
        kit.parsed_tags[NEW].dirty.is_set(),
        "the renamed tag is still unsaved"
    );
}

#[test]
fn nothing_belonging_to_another_tag_moves() {
    let (mut kit, mut view) = kit_with_state();
    rekey_tag_in_kit(&mut kit, &mut view, OLD, NEW);

    assert!(kit.parsed_tags.contains_key(BYSTANDER));
    assert!(view.caches.bitmap_previews.contains_key(BYSTANDER));
    assert!(kit.loading_tags.contains(BYSTANDER));
    assert_eq!(kit.keywords.keywords(BYSTANDER), ["vehicle".to_owned()]);
    assert!(kit.open_tabs.contains(&BYSTANDER.to_owned()));
}

/// Renaming a tag to the path it already has is a no-op, not a self-move that
/// removes the key and then puts it back.
#[test]
fn rekeying_a_tag_onto_itself_changes_nothing() {
    let (mut kit, mut view) = kit_with_state();
    let before = kit.generation;
    rekey_tag_in_kit(&mut kit, &mut view, OLD, OLD);
    assert!(kit.parsed_tags.contains_key(OLD));
    assert_eq!(kit.selected_key.as_deref(), Some(OLD));
    assert_eq!(kit.generation, before);
}

/// Half-typed field values are dropped rather than carried, because replaying
/// one over a document that has just changed identity is an edit nobody asked
/// for. Stated as a test so the choice is deliberate and not a missed map.
/// `EditDrafts` has no reader — `retain` visiting every entry is how a test
/// sees what is in it without growing the type an accessor only tests use.
fn draft_keys(view: &mut KitView) -> Vec<String> {
    let mut keys = Vec::new();
    view.edit_buffers.retain(|key, _| {
        keys.push(key.clone());
        true
    });
    keys
}

#[test]
fn in_progress_drafts_are_discarded_rather_than_followed() {
    let (mut kit, mut view) = kit_with_state();
    rekey_tag_in_kit(&mut kit, &mut view, OLD, NEW);
    assert_eq!(
        draft_keys(&mut view),
        vec![format!("{BYSTANDER}|name")],
        "the renamed tag's draft is gone and the bystander is still mid-edit"
    );
}

/// Every field of `Kit` and of its `KitView` is either state a rename carries
/// or state it does not reach. Destructured exhaustively, with no `..`, on
/// purpose: adding a field to either breaks this test's compile, which is the
/// only mechanism Rust offers to make that classification a decision rather
/// than an oversight.
#[test]
fn every_field_of_a_kit_is_accounted_for() {
    let kit = Kit::empty(KitId(9), TagNameIndex::default());
    let view = KitView::for_test(&kit);
    let Kit {
        // Carried by `rekey_tag_in_kit`.
        parsed_tags: _,
        loading_tags: _,
        selected_key: _,
        open_tabs: _,
        keywords: _,

        // Dropped or invalidated by it, deliberately.
        index_jobs: _,
        generation: _,
        field_index: _,

        // The source's own entries, tree and indices, which the rename moves
        // through `apply_container_rename_source_state` rather than here: it
        // runs against the mounted source and can fail on its own terms, while
        // this function is total.
        source: _,

        // Chimp is a separate surface over the same container, keyed by package
        // path rather than tag key. A rename has to move it too, but through
        // `rekey_chimp_package` — the two key spaces do not convert into one
        // another and merging them here would guess.
        chimp: _,

        // Re-derived by the caller once the source entries have moved, because
        // it needs the tag's new `display_path` and this function only has keys.
        active_favorite_entries: _,
        active_favorite_folders: _,

        // Not addressed by a tag key at all.
        id: _,
        names: _,
        scanning_entries: _,
        requested_path: _,
        profile: _,
        project: _,
        pending_container_folders: _,
        // Classified field by field below.
        restore: _,
    } = kit;
    let KitView {
        // Carried by `rekey_tag_in_kit`.
        tag_tree: _,
        pending_expand: _,
        find_filter_applied: _,

        // Dropped by it, deliberately.
        edit_buffers: _,

        // The Bitmap and Model Libraries' snapshots and thumbnail caches. Keyed
        // on the kit generation, which a rename bumps, so both are rebuilt
        // against the new key rather than carried across it.
        bitmap_browser: _,
        model_browser: _,
        git_review: _,

        // Chimp's side of the surface switch and how it is browsed; see
        // `chimp` above.
        surface: _,
        chimp: _,

        // Not addressed by a tag key at all.
        blam: _,
        terminal: _,
        // Classified field by field below.
        browser: _,
        caches: _,
    } = view;
    let RestorePlan {
        // Carried by `rekey_tag_in_kit`.
        pending_history: _,
        pending_restore_tags: _,
        // Keyed by package path; moved through `rekey_chimp_package`.
        pending_restore_chimp_packages: _,
        pending_restore_active_chimp_package: _,
        // Not addressed by a tag key at all; consumed once the source lands.
        pending_restore_folders: _,
        pending_restore_bitmap_library: _,
        pending_restore_model_library: _,
        pending_launch_tags: _,
    } = RestorePlan::default();
    let KitBrowser {
        // Dropped by `rekey_tag_in_kit`, deliberately.
        modified_signature: _,
        // Rebuilt from the generation the moment it moves.
        filter_cache: _,
        modified_tags: _,
        deletable_keys: _,
        deletable_keys_generation: _,
        folder_browsers: _,
        // Not addressed by a tag key at all.
        mode: _,
        sort: _,
        filter: _,
    } = KitBrowser::default();
    let EditorCaches {
        // Carried by `rekey_tag_in_kit`.
        bitmap_previews: _,
        model_previews: _,
        ce_sound_bindings: _,
        // Dropped or invalidated by it, deliberately.
        rmdf_cache: _,
        rmop_cache: _,
        render_method_epoch: _,
        h2_templates: _,
    } = EditorCaches::default();
}
