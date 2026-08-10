//! Renaming a tag inside the container that already holds it: moving every
//! piece of per-key state that follows the tag, and the browser state that
//! describes where it now lives.
//! Container surgery belongs to `blam-tags`, and the dialogs belong to the UI
//! modules; what lives here is the bookkeeping in between.

use super::*;

use super::container_folders::normalize_folder_rel;
use super::duplicate::{
    backup_paths_text, container_duplicate_index_key, container_index_for_utoc,
    create_duplicate_backup, validate_leaf_characters,
};
use std::thread;

/// Where a rename is going, in every form the write and the browser need.
struct RenameDestination {
    /// `/Game/Tags/<rel>-<group>` — the only form `blam-tags` is given. The
    /// container paths are derived there, from the directory index, so the
    /// pak's own spelling of its mount prefix is never guessed at here.
    package: String,
    /// `<rel>.<group>` as the browser shows it.
    display: String,
}

/// Work out where a rename lands, and refuse anything malformed before a
/// container is opened.
fn container_rename_destination(
    entry: &TagEntry,
    old_rel_path: &str,
    new_rel: &str,
) -> Result<RenameDestination, String> {
    let normalized = normalize_folder_rel(new_rel);
    if normalized.is_empty() {
        return Err("Enter a tag path (e.g. objects/vehicles/warthog)".to_owned());
    }
    // Every component, not just the leaf: a rename is also how a tag moves into
    // a folder, so the folder names have to survive the same rules the leaf does.
    for component in normalized.split('/') {
        validate_leaf_characters(component, "A folder or tag name", "Enter a tag path")?;
    }
    let group_name = entry
        .group_name
        .clone()
        .unwrap_or_else(|| format_group_tag(entry.group_tag));
    // Taken from the path the container actually holds, not from the group name:
    // the two agree for everything Baboon writes, and the container is the one
    // that has to be able to find the tag afterwards.
    let stem = old_rel_path
        .rsplit('/')
        .next()
        .and_then(|file| file.strip_suffix(".ubulk"))
        .ok_or("This tag's container path is not a .ubulk")?;
    let group_suffix = stem
        .rsplit_once('-')
        .map(|(_, group)| group.to_owned())
        .ok_or("This tag's container path has no group suffix")?;
    Ok(RenameDestination {
        package: format!("/Game/Tags/{normalized}-{group_suffix}"),
        display: format!("{normalized}.{group_name}"),
    })
}

#[derive(Clone)]
struct ContainerRenameWorkerInput {
    root: PathBuf,
    containers: Vec<crate::source::MountedContainer>,
    target_container: usize,
    key: String,
    group_tag: u32,
    group_name: String,
    old_package: String,
    new_package: String,
    old_ubulk: String,
    old_display: String,
    new_display: String,
    /// The provenance line from the ledger. The writer proves it again before
    /// moving anything, which is what keeps this off the game's own tags even
    /// if the eligibility check above were somehow wrong.
    minimum_appended_index: u32,
    target_label: String,
    is_mod: bool,
    /// Rebuilt bytes when the document has unsaved edits, so the rename carries
    /// them rather than forcing a save first. One transaction, one backup.
    tag_bytes: Option<Vec<u8>>,
}

fn run_container_rename(
    input: ContainerRenameWorkerInput,
) -> Result<ContainerRenameResult, String> {
    let target = input
        .containers
        .get(input.target_container)
        .ok_or("Container provenance is stale")?;
    let backup = create_duplicate_backup(&target.utoc_path)?;
    let archive = target.archive.clone();

    let request = blam_tags::iostore::writer::InPlaceTagRename {
        old_package_path: &input.old_package,
        new_package_path: &input.new_package,
        tag_bytes: input.tag_bytes.as_deref(),
        minimum_appended_index: Some(input.minimum_appended_index),
        // Deliberately none. A container redirect does not forward references —
        // measured in the game, with a tag every level scenario imports, moved
        // once with a redirect verified present in the rewritten container and
        // once without, to the same result. Writing one would add a header entry
        // that does nothing and then make the tag harder to move or retire
        // again, since both refuse a package a redirect points at.
        redirect: false,
    };
    if let Err(error) =
        blam_tags::iostore::writer::rename_tag_in_place_with(&archive, &target.utoc_path, &request)
    {
        return Err(format!(
            "Renaming {} in {} failed: {error}. Backup kept at {}",
            input.old_display,
            input.target_label,
            backup_paths_text(&backup)
        ));
    }

    let reopened = crate::source::reopen_container_archive(
        &input.root,
        &input.containers,
        input.target_container,
    )
    .map_err(|error| {
        format!(
            "Renamed {} in {}, but reopening failed: {error}. Backup kept at {}",
            input.old_display,
            input.target_label,
            backup_paths_text(&backup)
        )
    })?;

    // Read back rather than predicted. `blam-tags` rewrites only the folder tail
    // and the leaf of the path the directory index already held, preserving the
    // container's own casing of everything else — so the container is the only
    // thing that can say where the tag now is.
    let wanted = input.new_package.to_ascii_lowercase();
    let new_uasset = reopened
        .entries()
        .iter()
        .find(|entry| {
            crate::source::container_package_name(&entry.path).as_deref() == Some(wanted.as_str())
        })
        .map(|entry| entry.path.clone())
        .ok_or_else(|| {
            format!(
                "Renamed {} in {}, but the moved tag is not at {} in the reopened container. \
                 Backup kept at {}",
                input.old_display,
                input.target_label,
                input.new_package,
                backup_paths_text(&backup)
            )
        })?;
    let new_ubulk = new_uasset
        .strip_suffix(".uasset")
        .map(|stem| format!("{stem}.ubulk"))
        .ok_or("The renamed tag's container path is not a .uasset")?;

    let chunk_label = target.chunk_label.clone();
    let entry = TagEntry {
        key: format!("ublock:{chunk_label}:{new_ubulk}"),
        display_path: input.new_display.clone(),
        group_tag: input.group_tag,
        group_name: Some(input.group_name.clone()),
        location: TagEntryLocation::Container {
            container: input.target_container,
            rel_path: new_ubulk.clone(),
        },
    };
    let record = CreatedTagRecord {
        utoc_path: target.utoc_path.display().to_string(),
        chunk_label,
        package_id: super::package_id_for(&input.new_package),
        package_path: input.new_package.clone(),
        uasset_path: new_uasset.clone(),
        ubulk_path: new_ubulk.clone(),
        display_path: input.new_display.clone(),
        group_tag: input.group_tag,
        source_display: input.old_display.clone(),
        container_entry_count_before: input.minimum_appended_index,
        // Overwritten by `record_rename` from whatever the old row said. Set
        // here only because the struct needs a value.
        origin: CreatedTagOrigin::Authored,
        created_unix_secs: 0,
    };

    Ok(ContainerRenameResult {
        old_key: input.key,
        target_container: input.target_container,
        target_utoc: target.utoc_path.clone(),
        archive: Arc::new(reopened),
        entry,
        group_tag: input.group_tag,
        old_package: input.old_package,
        new_package: input.new_package,
        new_uasset_path: new_uasset,
        old_ubulk_path: input.old_ubulk,
        new_ubulk_path: new_ubulk,
        old_display: input.old_display,
        target_label: input.target_label,
        is_mod: input.is_mod,
        backup,
        record,
    })
}

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

/// What a rename is allowed to proceed on: the provenance line the writer
/// re-checks before it moves anything.
///
/// Only ever produced for a tag Baboon itself put in the container, and that is
/// the whole safety argument — see [`container_rename_eligibility`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct AuthoredRename {
    /// How many chunks the container held before Baboon first wrote to it.
    /// Every chunk of the tag being moved sits at or past this, which the
    /// writer proves again for itself.
    pub(in crate::app) minimum_appended_index: u32,
}

/// Whether a tag may be renamed inside the pak that holds it — or why it may
/// not.
///
/// Only a tag **Baboon authored** may be. This was going to be a two-tier
/// policy, with the game's own tags behind Expert mode and a confirmation, on
/// the reasoning that a rename is closer to duplication than to deletion: it
/// retires two chunks, writes equivalents, and leaves a forwarding redirect.
///
/// The redirect does not forward. That was measured in the game rather than
/// argued about: renaming `assault_rifle-weapon` removes the assault rifle,
/// and renaming it with a redirect verified present in the rewritten container
/// removes it just the same. So a rename does not relocate a tag that anything
/// points at — it deletes it and leaves a copy under a name nothing asks for.
///
/// A tag Baboon authored is safe for exactly one reason: nothing the game
/// shipped can reference it, because it did not exist when the game was built.
/// Whatever references it, the user made, and can move with it.
///
/// The shipped case is refused rather than gated, because a warning cannot make
/// a broken reference work. Renaming those needs every referrer's import table
/// rewritten in the same transaction, which is a separate piece of work.
///
/// What this cannot see, and does not try to: whether the container is encrypted
/// or signed, whether it carries ordinal-keyed blocks, whether the package owns
/// an unexpected chunk. Those are properties of the container rather than of
/// Baboon's records, and `blam-tags` refuses them where it can prove them.
pub(in crate::app) fn container_rename_eligibility(
    entry: &TagEntry,
    containers: &[crate::source::MountedContainer],
    ledger: &CreatedTagLedger,
) -> Result<AuthoredRename, String> {
    let (container, rel_path) = match &entry.location {
        TagEntryLocation::Container {
            container,
            rel_path,
        } => (*container, rel_path.as_str()),
        TagEntryLocation::NewContainer { .. } => {
            return Err(
                "This tag is not in a pak yet — rename it from its tab, which costs nothing"
                    .to_owned(),
            );
        }
        TagEntryLocation::LooseFile(_) => {
            return Err("Loose tags are renamed on disk, not inside a pak".to_owned());
        }
        TagEntryLocation::Monolithic { .. } => {
            return Err("Monolithic cache tags are read-only".to_owned());
        }
    };
    let target = containers
        .get(container)
        .ok_or("This tag's container is no longer mounted")?;

    // A record that says `RenamedFromShipped` still names a tag the game
    // shipped, so it does not qualify — otherwise renaming twice would launder
    // one into a tag that renames freely.
    if let Some(record) = ledger.find(&target.utoc_path, rel_path)
        && record.origin == CreatedTagOrigin::Authored
    {
        return Ok(AuthoredRename {
            minimum_appended_index: record.container_entry_count_before,
        });
    }
    Err(format!(
        "{} is one of the game's own tags. Renaming it inside {} would move it \
         out from under everything that references it, and the pak format has no \
         way to forward those references. Duplicate it instead, and rename the copy.",
        entry.display_path, target.chunk_label
    ))
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
        let old_index_key =
            container_duplicate_index_key(request.group_tag, request.old_ubulk_path)
                .ok_or("Rename completed from an invalid container path")?;
        let new_index_key =
            container_duplicate_index_key(request.group_tag, request.new_ubulk_path)
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
    source
        .all_entries
        .retain(|existing| existing.key != old_key);
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

impl Baboon {
    /// Move a Baboon-authored tag to a new path inside the pak that holds it.
    ///
    /// `new_rel` is the whole path, not just a leaf, so this is also how a tag
    /// moves into a folder — which is what makes a pending folder become a real
    /// one, since a pak cannot encode a directory that has no file under it.
    pub(in crate::app) fn begin_container_rename_in_place(
        &mut self,
        key: &str,
        new_rel: &str,
        ctx: egui::Context,
    ) {
        let kit = self.active_kit_id();
        // All three in-place writers are mutually exclusive per workspace: two
        // of them on one `.utoc` would race, and each validates against a handle
        // the other is invalidating.
        if self.container_duplicate_running.contains(&kit)
            || self.container_delete_running.contains(&kit)
            || self.container_rename_running.contains(&kit)
        {
            self.status = "Another container write is already running in this workspace".to_owned();
            return;
        }
        let Some(entry) = self.entry_for_key(key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = entry.location.clone()
        else {
            self.status = "Only Campaign Evolved container tags are renamed in place".to_owned();
            return;
        };

        let containers = self.mounted_containers().unwrap_or_default();
        let grounds = match container_rename_eligibility(&entry, &containers, &self.created_tags) {
            Ok(grounds) => grounds,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let destination = match container_rename_destination(&entry, &rel_path, new_rel) {
            Ok(destination) => destination,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let Some(old_package) = container_rel_to_package_path(&rel_path) else {
            self.status = "This tag's container path has no package name".to_owned();
            return;
        };
        if destination.package.eq_ignore_ascii_case(&old_package) {
            // `FPackageId` lowercases, so a case-only change hashes to the id it
            // already has and the writer would refuse it anyway. Saying so here
            // costs nothing and does not open a container to find out.
            self.status = format!("{} is already at that path", entry.display_path);
            return;
        }
        if self.source().is_some_and(|source| {
            source
                .entries
                .iter()
                .chain(source.all_entries.iter())
                .any(|existing| existing.display_path == destination.display)
        }) {
            self.status = format!("A tag already exists at {}", destination.display);
            return;
        }

        let Some(target) = containers.get(container) else {
            self.status = "This tag's container is no longer mounted".to_owned();
            return;
        };
        let (target_label, is_mod, target_utoc) = (
            target.chunk_label.clone(),
            target.is_mod,
            target.utoc_path.clone(),
        );
        let root = match self.source().map(|source| &source.source) {
            Some(TagSource::IoStoreContainerSet { root, .. }) => root.clone(),
            _ => {
                self.status = "Source is not a Campaign Evolved container source".to_owned();
                return;
            }
        };
        // Serialized here rather than refused: Chimp aside, the only other way
        // to keep an edit is to save first, which for a container tag is a
        // second in-place write with its own backup. One transaction is both
        // simpler and safer.
        let tag_bytes = match self.kits[self.active]
            .parsed_tags
            .get(key)
            .filter(|document| document.dirty.is_set())
        {
            Some(document) => match document.tag.write_to_bytes() {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    self.status = format!("Could not serialize unsaved edits: {error}");
                    return;
                }
            },
            None => None,
        };

        let lease = match self
            .acquire_container_write_lease(&target_utoc, ContainerWriteMode::AppendInPlace)
        {
            Ok(lease) => lease,
            Err(failure) => {
                self.status = failure.to_string();
                return;
            }
        };
        let lease_id = self.park_container_write_lease(lease);
        let stamp = KitStamp {
            kit,
            generation: self.kits[self.active].generation,
        };
        self.container_rename_running.insert(kit);
        self.status = format!("Renaming {} → {}…", entry.display_path, destination.display);
        let input = ContainerRenameWorkerInput {
            root,
            containers,
            target_container: container,
            key: key.to_owned(),
            group_tag: entry.group_tag,
            group_name: entry
                .group_name
                .clone()
                .unwrap_or_else(|| format_group_tag(entry.group_tag)),
            old_package,
            new_package: destination.package,
            old_ubulk: rel_path,
            old_display: entry.display_path.clone(),
            new_display: destination.display,
            minimum_appended_index: grounds.minimum_appended_index,
            target_label,
            is_mod,
            tag_bytes,
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = run_container_rename(input);
            let _ = tx.send(WorkerMessage::ContainerRenameFinished {
                stamp,
                lease: lease_id,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub(in crate::app) fn handle_container_rename_finished(
        &mut self,
        stamp: KitStamp,
        lease: ContainerLeaseId,
        result: Result<ContainerRenameResult, String>,
        ctx: &egui::Context,
    ) -> bool {
        // Settled first, and on both paths: a write that landed changed the
        // `.utoc`, so the Unreal workspace's parsed copy of it is stale either
        // way, and a failed write still released nothing until this runs.
        if let Some(lease) = self.take_container_write_lease(lease) {
            let outcome = if result.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            };
            self.release_container_write_lease(lease, outcome, ctx);
        }
        self.container_rename_running.remove(&stamp.kit);
        let kit_index = self.kit_index(stamp.kit);

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.status = error.clone();
                self.operation_notice = Some(OperationNotice {
                    title: "Rename failed".to_owned(),
                    message: error,
                    failed: true,
                });
                return false;
            }
        };
        let Some(kit_index) = kit_index else {
            // The workspace closed mid-write. The tag is at its new path in the
            // pak and will be there on the next load; there is no browser left
            // to move it in.
            return true;
        };
        let Some(target_container) = container_index_for_utoc(
            self.kits[kit_index].source.as_ref(),
            result.target_container,
            &result.target_utoc,
        ) else {
            self.status = format!(
                "Renamed in {}, but this workspace no longer has that container mounted — \
                 reload the source to see it. Backup: {}",
                result.target_label,
                backup_paths_text(&result.backup)
            );
            return false;
        };

        let mut result = result;
        result.target_container = target_container;
        if let TagEntryLocation::Container { container, .. } = &mut result.entry.location {
            *container = target_container;
        }
        let new_key = result.entry.key.clone();
        let folder_seeds = self.kits[kit_index].folder_seeds();
        {
            let Some(source) = self.kits[kit_index].source.as_mut() else {
                self.status = "Rename completed after its source was unloaded".to_owned();
                return false;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
                self.status = "Rename completed against a non-container source".to_owned();
                return false;
            };
            let Some(target) = containers.get_mut(target_container) else {
                self.status = "Rename completed with stale container provenance".to_owned();
                return false;
            };
            target.archive = result.archive.clone();
            let request = ContainerRenameMove {
                container: target_container,
                group_tag: result.group_tag,
                old_package: &result.old_package,
                new_package: &result.new_package,
                new_uasset_path: &result.new_uasset_path,
                old_ubulk_path: &result.old_ubulk_path,
                new_ubulk_path: &result.new_ubulk_path,
                is_mod: result.is_mod,
                // No redirect was written, so nothing should claim the old
                // path still resolves. See `run_container_rename`.
                redirect: false,
            };
            if let Err(error) = apply_container_rename_source_state(
                source,
                &result.old_key,
                &result.entry,
                &request,
                &folder_seeds,
            ) {
                self.status = error;
                return false;
            }
        }

        // The ledger decides the origin itself from the row being replaced, so
        // a tag that was Baboon's stays Baboon's across any number of moves.
        self.created_tags
            .record_rename(&result.old_ubulk_path, result.record);
        let ledger_error = self.created_tags.save().err();

        // The project stashes overlays under the tag's logical path, so the old
        // identity has to go or a checkpoint restores the tag at both paths.
        self.forget_campaign_overlay(kit_index, &result.old_key);
        rekey_tag_in_kit(&mut self.kits[kit_index], &result.old_key, &new_key);
        self.refresh_favorite_entries_for(kit_index);
        if self
            .reveal_target
            .as_ref()
            .is_some_and(|target| target.key == result.old_key)
        {
            self.reveal_target = None;
        }

        self.status = match ledger_error {
            Some(error) => format!(
                "Renamed {} → {} in {}, but the record could not be saved: {error}",
                result.old_display, result.entry.display_path, result.target_label
            ),
            None => format!(
                "Renamed {} → {} in {}",
                result.old_display, result.entry.display_path, result.target_label
            ),
        };
        true
    }
}

#[cfg(test)]
#[path = "../tests/rekey_tag.rs"]
mod rekey_tag_tests;

#[cfg(test)]
#[path = "../tests/rename_eligibility.rs"]
mod rename_eligibility_tests;

#[cfg(test)]
#[path = "../tests/container_rename_state.rs"]
mod container_rename_state_tests;
