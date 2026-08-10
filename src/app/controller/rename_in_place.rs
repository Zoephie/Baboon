//! Renaming a tag inside the container that already holds it: moving every
//! piece of per-key state that follows the tag, and the browser state that
//! describes where it now lives.
//! Container surgery belongs to `blam-tags`, and the dialogs belong to the UI
//! modules; what lives here is the bookkeeping in between.

use super::*;

use super::duplicate::container_duplicate_index_key;

/// Move one entry of a key-addressed map from `old` to `new`.
///
/// A miss is not an error — most of these maps hold state only for tags the
/// user has actually touched, so an absent key means "nothing to carry".
fn move_key<V>(map: &mut HashMap<String, V>, old: &str, new: &str) {
    if let Some(value) = map.remove(old) {
        map.insert(new.to_owned(), value);
    }
}

/// Carry every per-key trace of a tag from `old` to `new`.
///
/// The counterpart of `forget_tag_in_kit`, and the harder half of the pair. A
/// delete only has to *drop* state, and a map it misses merely leaks. A rename
/// has to *carry* it, and a map this misses strands the tag's document, its
/// undo history, or its keywords under a key nothing will ever ask for again —
/// which is worse than a crash, because from the outside it looks like the
/// rename worked.
///
/// Deliberately does **not** touch the source's entries, tree or indices; that
/// is `apply_container_rename_source_state`, which runs against the mounted
/// source and can fail on its own terms. Splitting them keeps this function
/// total: given any kit and any two keys, it always leaves a consistent kit.
pub(in crate::app) fn rekey_tag_in_kit(kit: &mut Kit, old: &str, new: &str) {
    if old == new {
        return;
    }

    // The document moves whole — `TagDocument` carries the parsed tag, its
    // dirty flag and its undo journal together, and a rename is not a reason
    // to lose any of the three.
    move_key(&mut kit.parsed_tags, old, new);
    move_key(&mut kit.pending_history, old, new);
    move_key(&mut kit.bitmap_previews, old, new);
    move_key(&mut kit.model_previews, old, new);
    move_key(&mut kit.ce_sound_bindings, old, new);
    move_key(&mut kit.pending_expand, old, new);
    move_key(&mut kit.field_search, old, new);
    move_key(&mut kit.field_search_applied, old, new);

    if kit.loading_tags.remove(old) {
        kit.loading_tags.insert(new.to_owned());
    }

    // Half-typed field values are dropped rather than carried. A draft is a
    // value the user is mid-way through typing into a specific document, and
    // re-applying one over a document that has just changed identity is a
    // silent edit nobody asked for.
    kit.edit_buffers.forget_tag(old);

    kit.keywords.rekey_tag(old, new);
    kit.keywords.save_if_dirty();

    if kit.selected_key.as_deref() == Some(old) {
        kit.selected_key = Some(new.to_owned());
    }
    for tab in &mut kit.open_tabs {
        if tab == old {
            *tab = new.to_owned();
        }
    }
    // The pane payload *is* the key, so the tiles are edited in place. Closing
    // and reopening the tab would work and would also throw away wherever the
    // user had split or dragged it to.
    let panes: Vec<egui_tiles::TileId> = kit
        .tag_tree
        .tiles
        .iter()
        .filter_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(key) if key == old => Some(*id),
            _ => None,
        })
        .collect();
    for id in panes {
        if let Some(egui_tiles::Tile::Pane(key)) = kit.tag_tree.tiles.get_mut(id) {
            *key = new.to_owned();
        }
    }
    for staged in &mut kit.pending_restore_tags {
        if staged.key == old {
            staged.key = new.to_owned();
        }
    }

    // Both are keyed by the path of the tag being *referred to*, not by the tag
    // holding the reference — so a renamed render-method definition leaves a
    // cached hit under a path that no longer resolves. They are pure caches, so
    // dropping them costs one re-resolve and cannot be wrong.
    kit.rmdf_cache.clear();
    kit.rmop_cache.clear();

    // Forces `modified_tags` to be rebuilt: it maps keys to entries, and the
    // signature is what decides whether that is worth doing again.
    kit.modified_signature.clear();

    // Last, and what makes the rest visible: the browser's memoised filter, the
    // deletable-key set and the field-value index are all keyed on the
    // generation, so without this the browser keeps answering with the old key.
    kit.generation = kit.generation.wrapping_add(1);
    kit.field_index.invalidate();
}

/// Where a renamed tag was, and where it now is, in the terms the mounted
/// source is indexed by.
///
/// Both halves are carried explicitly rather than derived from the entry pair:
/// the container spells a path in its own casing, which is routinely not the
/// casing of the package name, and re-deriving one from the other is exactly
/// the mistake that files a tag under a second, sibling folder node.
pub(in crate::app) struct ContainerRenameMove<'a> {
    pub(in crate::app) container: usize,
    pub(in crate::app) group_tag: u32,
    pub(in crate::app) old_package: &'a str,
    pub(in crate::app) new_package: &'a str,
    pub(in crate::app) old_uasset_path: &'a str,
    pub(in crate::app) new_uasset_path: &'a str,
    pub(in crate::app) old_ubulk_path: &'a str,
    pub(in crate::app) new_ubulk_path: &'a str,
    /// Whether the container this landed in is a mod rather than one of the
    /// game's own packs, which is what the shipped index records.
    pub(in crate::app) is_mod: bool,
    /// Whether the write left an old→new redirect in the container header. When
    /// it did, the old logical path stays *resolvable* even though it stops
    /// being *browsable*, and Baboon's own reference index has to say the same
    /// thing or in-app navigation breaks where the game still works.
    pub(in crate::app) redirect: bool,
}

/// Move a renamed tag through the mounted source: its indices, its entry, the
/// browser tree, and the reference graph.
///
/// The counterpart of `apply_container_duplicate_source_state`, and not a
/// variation on it — a duplicate only inserts, while this has to remove first.
/// That ordering is load-bearing in one place and stated at it.
pub(in crate::app) fn apply_container_rename_source_state(
    source: &mut LoadedSourceData,
    old_key: &str,
    entry: &TagEntry,
    request: &ContainerRenameMove<'_>,
    pending_folders: &[String],
) -> Result<(), String> {
    {
        let TagSource::IoStoreContainerSet {
            index,
            packages,
            shipped,
            ..
        } = &mut source.source
        else {
            return Err("Rename completed against a non-container source".to_owned());
        };
        let old_index_key = container_duplicate_index_key(request.group_tag, request.old_ubulk_path)
            .ok_or("Rename completed from an invalid container path")?;
        let new_index_key = container_duplicate_index_key(request.group_tag, request.new_ubulk_path)
            .ok_or("Rename completed with an invalid container destination path")?;
        let index = Arc::make_mut(index);
        if request.redirect {
            // Mirrors what the container header now says: a reference to the
            // old path still resolves, and it resolves to the tag's new home.
            // The browser draws from `entries`, not from this, so the old path
            // stays resolvable without becoming visible again.
            index.insert(
                old_index_key,
                request.container,
                request.new_ubulk_path.to_owned(),
            );
        } else {
            index.remove(&old_index_key);
        }
        index.insert(
            new_index_key,
            request.container,
            request.new_ubulk_path.to_owned(),
        );

        // Both removes have to come first, because `ContainerPackageIndex`
        // is first-insert-wins: an insert over a row that is already there does
        // nothing at all. The old package's row is now a tombstone, and a row
        // already sitting at the destination is stale by construction — the
        // chunks the write just placed there are the newest thing that has ever
        // been at that package path in this container. Neither may survive an
        // insert that silently declines to happen.
        let packages = Arc::make_mut(packages);
        packages.remove(request.old_package);
        packages.remove(request.new_package);
        packages.insert(
            request.new_package.to_ascii_lowercase(),
            request.container,
            request.new_uasset_path.to_owned(),
        );

        if !request.is_mod {
            let shipped = Arc::make_mut(shipped);
            shipped.remove(request.old_ubulk_path);
            shipped.insert(request.new_ubulk_path, request.container);
        }
    }

    source.entries.retain(|existing| existing.key != old_key);
    source.all_entries.retain(|existing| existing.key != old_key);
    // Sorted rather than pushed, for the same reason a duplicate is: the mount
    // orders by `natural_key` and the browser draws a folder in entry-vector
    // order, so a pushed entry lands at the bottom of its new folder instead of
    // in the place the user will look for it.
    crate::source::insert_entry_sorted(&mut source.entries, entry.clone());
    if !source.all_entries.is_empty() {
        crate::source::insert_entry_sorted(&mut source.all_entries, entry.clone());
    }
    crate::source::rebuild_folder_tree(source, pending_folders);
    source.group_tree = crate::source::build_group_tree(if source.all_entries.is_empty() {
        &source.entries
    } else {
        &source.all_entries
    });
    // Dropped whole rather than patched. The index is keyed by tag key on the
    // referring side *and* on the referred-to side, so a rename moves rows this
    // has no way to enumerate — and a half-patched reference graph gives wrong
    // answers silently, where an absent one is simply rebuilt on the next query.
    source.reverse_dependencies = None;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/rekey_tag.rs"]
mod rekey_tag_tests;

#[cfg(test)]
#[path = "../tests/container_rename_state.rs"]
mod container_rename_state_tests;
