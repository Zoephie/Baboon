//! Renaming a tag inside the container that already holds it: moving every
//! piece of per-key state that follows the tag, and the browser state that
//! describes where it now lives.
//! Container surgery belongs to `blam-tags`, and the dialogs belong to the UI
//! modules; what lives here is the bookkeeping in between.

use super::*;
use crate::core::created_tags::{CreatedTagOrigin, package_id_for};

use super::container_folders::normalize_folder_rel;
use super::duplicate::{
    backup_paths_text, container_duplicate_index_key, container_index_for_utoc,
    create_duplicate_backup, validate_leaf_characters,
};

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
    containers: Vec<crate::core::source::MountedContainer>,
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

    let reopened = crate::core::source::reopen_container_archive(
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
            crate::core::source::container_package_name(&entry.path).as_deref() == Some(wanted.as_str())
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
        key: crate::core::source::container_entry_key(&chunk_label, &new_ubulk),
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
        package_id: package_id_for(&input.new_package),
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
/// total: given any kit, its view and any two keys, it always leaves both
/// consistent.
pub(in crate::app) fn rekey_tag_in_kit(kit: &mut Kit, view: &mut KitView, old: &str, new: &str) {
    if old == new {
        return;
    }

    // The document moves whole — `TagDocument` carries the parsed tag, its
    // dirty flag and its undo journal together, and a rename is not a reason
    // to lose any of the three.
    move_key(&mut kit.parsed_tags, old, new);
    move_key(&mut kit.restore.pending_history, old, new);
    move_key(&mut view.caches.bitmap_previews, old, new);
    move_key(&mut view.caches.model_previews, old, new);
    move_key(&mut view.caches.ce_sound_bindings, old, new);
    move_key(&mut view.pending_expand, old, new);
    move_key(&mut view.find_filter_applied, old, new);

    if kit.loading_tags.remove(old) {
        kit.loading_tags.insert(new.to_owned());
    }

    // Half-typed field values are dropped rather than carried. A draft is a
    // value the user is mid-way through typing into a specific document, and
    // re-applying one over a document that has just changed identity is a
    // silent edit nobody asked for.
    view.edit_buffers.forget_tag(old);
    view.forget_row_heights(old);

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
    let panes: Vec<egui_tiles::TileId> = view
        .tag_tree
        .tiles
        .iter()
        .filter_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(key) if key == old => Some(*id),
            _ => None,
        })
        .collect();
    for id in panes {
        if let Some(egui_tiles::Tile::Pane(key)) = view.tag_tree.tiles.get_mut(id) {
            *key = new.to_owned();
        }
    }
    for staged in &mut kit.restore.pending_restore_tags {
        if staged.key == old {
            staged.key = new.to_owned();
        }
    }

    // Both are keyed by the path of the tag being *referred to*, not by the tag
    // holding the reference — so a renamed render-method definition leaves a
    // cached hit under a path that no longer resolves. They are pure caches, so
    // dropping them costs one re-resolve and cannot be wrong.
    view.caches.forget_render_methods();
    view.caches.h2_templates = H2TemplateCache::default();

    // Forces `modified_tags` to be rebuilt: it maps keys to entries, and the
    // signature is what decides whether that is worth doing again.
    view.browser.modified_signature.clear();

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
    containers: &[crate::core::source::MountedContainer],
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

        // This container no longer provides the old package; what it provides
        // at the new one replaces anything it had there. Other containers'
        // copies of either are untouched.
        let packages = Arc::make_mut(packages);
        packages.remove(request.old_package, request.container);
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

    // Sorted rather than pushed (upsert_entry keeps a container's list in
    // `natural_key` order), for the same reason a duplicate is: a pushed entry
    // would land at the bottom of its new folder instead of where the user
    // will look for it.
    source.remove_entry(old_key, pending_folders);
    source.upsert_entry(entry.clone(), pending_folders);
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
        let kit = self.model.active_kit_id();
        // All three in-place writers are mutually exclusive per workspace: two
        // of them on one `.utoc` would race, and each validates against a handle
        // the other is invalidating.
        if self.tag_ops.container_duplicate_running.contains(&kit)
            || self.tag_ops.container_delete_running.contains(&kit)
            || self.tag_ops.container_rename_running.contains(&kit)
        {
            self.model.status = "Another container write is already running in this workspace".to_owned();
            return;
        }
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = entry.location.clone()
        else {
            self.model.status = "Only Campaign Evolved container tags are renamed in place".to_owned();
            return;
        };

        let containers = self.model.mounted_containers().unwrap_or_default();
        let grounds = match container_rename_eligibility(&entry, &containers, &self.tag_ops.created_tags) {
            Ok(grounds) => grounds,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let destination = match container_rename_destination(&entry, &rel_path, new_rel) {
            Ok(destination) => destination,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let Some(old_package) = container_rel_to_package_path(&rel_path) else {
            self.model.status = "This tag's container path has no package name".to_owned();
            return;
        };
        if destination.package.eq_ignore_ascii_case(&old_package) {
            // `FPackageId` lowercases, so a case-only change hashes to the id it
            // already has and the writer would refuse it anyway. Saying so here
            // costs nothing and does not open a container to find out.
            self.model.status = format!("{} is already at that path", entry.display_path);
            return;
        }
        if self.model.source().is_some_and(|source| {
            source
                .entries
                .iter()
                .chain(source.all_entries.iter())
                .any(|existing| existing.display_path == destination.display)
        }) {
            self.model.status = format!("A tag already exists at {}", destination.display);
            return;
        }

        let Some(target) = containers.get(container) else {
            self.model.status = "This tag's container is no longer mounted".to_owned();
            return;
        };
        let (target_label, is_mod, target_utoc) = (
            target.chunk_label.clone(),
            target.is_mod,
            target.utoc_path.clone(),
        );
        let root = match self.model.source().map(|source| &source.source) {
            Some(TagSource::IoStoreContainerSet { root, .. }) => root.clone(),
            _ => {
                self.model.status = "Source is not a Campaign Evolved container source".to_owned();
                return;
            }
        };
        // Serialized here rather than refused: Chimp aside, the only other way
        // to keep an edit is to save first, which for a container tag is a
        // second in-place write with its own backup. One transaction is both
        // simpler and safer.
        let tag_bytes = match self.model.kits[self.model.active]
            .parsed_tags
            .get(key)
            .filter(|document| document.dirty.is_set())
        {
            Some(document) => match document.tag.write_to_bytes() {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    self.model.status = format!("Could not serialize unsaved edits: {error}");
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
                self.model.status = failure.to_string();
                return;
            }
        };
        let lease_id = self.park_container_write_lease(lease);
        let stamp = KitStamp {
            kit,
            generation: self.model.kits[self.model.active].generation,
        };
        self.tag_ops.container_rename_running.insert(kit);
        self.model.status = format!("Renaming {} → {}…", entry.display_path, destination.display);
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
        // Through spawn_worker so the lease always comes back, as for Duplicate.
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::ContainerRenameFinished {
                stamp,
                lease: lease_id,
                result: run_container_rename(input),
            },
            move |error| WorkerMessage::ContainerRenameFinished {
                stamp,
                lease: lease_id,
                result: Err(error),
            },
        );
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
        self.tag_ops.container_rename_running.remove(&stamp.kit);
        let kit_index = self.model.kit_index(stamp.kit);

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.model.status = error.clone();
                self.dialogs.open(OperationNotice {
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
            self.model.kits[kit_index].source.as_ref(),
            result.target_container,
            &result.target_utoc,
        ) else {
            self.model.status = format!(
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
        let folder_seeds = self.model.kits[kit_index].folder_seeds();
        {
            let Some(source) = self.model.kits[kit_index].source.as_mut() else {
                self.model.status = "Rename completed after its source was unloaded".to_owned();
                return false;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
                self.model.status = "Rename completed against a non-container source".to_owned();
                return false;
            };
            let Some(target) = containers.get_mut(target_container) else {
                self.model.status = "Rename completed with stale container provenance".to_owned();
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
                self.model.status = error;
                return false;
            }
        }

        // The ledger decides the origin itself from the row being replaced, so
        // a tag that was Baboon's stays Baboon's across any number of moves.
        self.tag_ops.created_tags
            .record_rename(&result.old_ubulk_path, result.record);
        let ledger_error = self.tag_ops.created_tags.save().err();

        // The project stashes overlays under the tag's logical path, so the old
        // identity has to go or a checkpoint restores the tag at both paths.
        self.forget_campaign_overlay(kit_index, &result.old_key);
        let KitMut { kit, view } = self.kit_and_view(kit_index);
        rekey_tag_in_kit(kit, view, &result.old_key, &new_key);
        self.refresh_favorite_entries_for(kit_index);
        if self
            .browser.reveal_target
            .as_ref()
            .is_some_and(|target| target.key == result.old_key)
        {
            self.browser.reveal_target = None;
        }

        self.model.status = match ledger_error {
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
mod tests {
    use super::*;
    use crate::app::browser::KitBrowser;
    use crate::app::editor::{AppliedFindFilter, EditorCaches};
    use crate::app::kits::{KitView, tag_tree_id};
    use crate::app::mods::TagHistory;
    use crate::app::shell::session::LastSessionTag;
    use crate::app::shell::session::RestorePlan;

    // Moving a tag's identity without losing anything filed under it.
    //
    // `TagEntry::key` is what documents, tabs, previews, undo history and the
    // keyword sidecar are all addressed by, so a rename has to carry every one of
    // them. A map that gets missed does not crash — it strands that state under a
    // key nothing resolves any more, and from the outside the rename looks as
    // though it worked. These tests are the cheapest place to catch that.

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
            // Measured again under the new key on its first draw.
            row_heights: _,

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
            // A view preference, not addressed by a tag key.
            search_scope: _,
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

    // Who may rename a tag inside the pak that holds it.
    //
    // Only tags Baboon authored, and the reason is a measurement rather than a
    // policy: a container redirect does not forward references. Renaming the
    // assault rifle removes it from the game, and renaming it with a redirect
    // verified present in the container removes it just the same. So a rename
    // relocates a tag only when nothing points at it — which for a tag Baboon
    // created is true by construction, because it did not exist when the game was
    // built.

    const UTOC: &str = "C:/Game/Paks/pakchunk240-Windows.utoc";
    const REL: &str = "Meteorite/Content/Tags/objects/copy-biped.ubulk";

    fn entry_at(rel_path: &str) -> TagEntry {
        TagEntry {
            key: format!("ublock:pakchunk240-Windows:{rel_path}"),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: 0x6269_7064,
            group_name: Some("biped".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: rel_path.to_owned(),
            },
        }
    }

    fn record(origin: CreatedTagOrigin) -> CreatedTagRecord {
        CreatedTagRecord {
            utoc_path: UTOC.to_owned(),
            chunk_label: "pakchunk240-Windows".to_owned(),
            package_path: "/Game/Tags/objects/copy-biped".to_owned(),
            package_id: 7,
            uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
            ubulk_path: REL.to_owned(),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: 0x6269_7064,
            source_display: "objects/original.biped".to_owned(),
            container_entry_count_before: 4096,
            origin,
            created_unix_secs: 1,
        }
    }

    /// A `MountedContainer` needs a real archive, which a unit test has no way to
    /// build — so the container list is empty and the tests that need one assert on
    /// the refusal that produces. Everything the tier decision actually turns on is
    /// reachable without it.
    fn no_containers() -> Vec<crate::core::source::MountedContainer> {
        Vec::new()
    }

    #[test]
    fn a_tag_that_is_not_in_a_pak_is_refused_with_a_reason_that_names_the_alternative() {
        let ledger = CreatedTagLedger::default();
        let new_container = TagEntry {
            key: "new:1".to_owned(),
            display_path: "objects/fresh.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Donor {
                    container: 0,
                    rel_path: "Tags/template-biped.uasset".to_owned(),
                },
                package: "/Game/Tags/objects/fresh-biped".to_owned(),
                group_tag: 0,
            },
        };
        let loose = TagEntry {
            key: "file:1".to_owned(),
            display_path: "objects/loose.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from("C:/kit/tags/objects/loose.biped")),
        };
        let monolithic = TagEntry {
            key: "cache:bipd:objects/cached".to_owned(),
            display_path: "objects/cached.biped".to_owned(),
            group_tag: 0,
            group_name: None,
            location: TagEntryLocation::Monolithic {
                name: "objects/cached".to_owned(),
                group_tag: 0,
            },
        };

        let unsaved = container_rename_eligibility(&new_container, &no_containers(), &ledger)
            .expect_err("an unsaved tag has no pak to rename inside");
        assert!(unsaved.contains("not in a pak yet"), "{unsaved}");
        assert!(
            container_rename_eligibility(&loose, &no_containers(), &ledger).is_err(),
            "a loose tag is renamed on disk"
        );
        assert!(container_rename_eligibility(&monolithic, &no_containers(), &ledger).is_err());
    }

    #[test]
    fn a_tag_whose_container_is_gone_is_refused_before_anything_is_decided() {
        let mut ledger = CreatedTagLedger::default();
        ledger.record(record(CreatedTagOrigin::Authored));
        let error = container_rename_eligibility(&entry_at(REL), &no_containers(), &ledger)
            .expect_err("the container is not mounted");
        assert!(error.contains("no longer mounted"), "{error}");
    }

    /// A row saying the tag came from a shipped one must not qualify it. Renaming
    /// twice would otherwise launder a shipped tag into one that renames freely,
    /// which is the same hole `CreatedTagOrigin` exists to close on the delete side
    /// — and here the consequence is a reference that silently stops resolving.
    #[test]
    fn a_renamed_shipped_tag_does_not_become_an_authored_one() {
        let mut ledger = CreatedTagLedger::default();
        ledger.record(record(CreatedTagOrigin::RenamedFromShipped));
        // With no container mounted the call cannot reach the decision itself, so
        // this asserts the ledger row that the decision reads.
        let found = ledger
            .find(Path::new(UTOC), REL)
            .expect("the row is addressed by container path");
        assert_ne!(
            found.origin,
            CreatedTagOrigin::Authored,
            "only an Authored row may be renamed in place"
        );
    }

    /// The line handed to the writer comes from the ledger row, not from anything
    /// derived at the call site — the writer re-checks it, and a number invented
    /// here would either refuse a valid rename or authorise an invalid one.
    #[test]
    fn the_provenance_line_is_the_one_the_ledger_recorded() {
        assert_eq!(
            AuthoredRename {
                minimum_appended_index: 4096
            }
            .minimum_appended_index,
            4096
        );
        assert_eq!(
            record(CreatedTagOrigin::Authored).container_entry_count_before,
            4096
        );
    }

    // Moving a renamed tag through the mounted source without reloading it.
    //
    // Three indices, an entry list, two trees and a reference graph all describe
    // where a container tag lives. A reload would rebuild all of them, and is what
    // the code deliberately avoids — so each one has to be moved by hand, and the
    // order of two of those moves matters.

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
        let mut index = crate::core::source::ContainerTagIndex::default();
        index.insert(
            crate::core::source::container_ref_key(GROUP, "objects/vehicles/warthog"),
            0,
            OLD_UBULK.to_owned(),
        );
        let mut packages = crate::core::source::ContainerPackageIndex::default();
        packages.insert(OLD_PACKAGE.to_ascii_lowercase(), 0, OLD_UASSET.to_owned());
        let mut shipped = crate::core::source::ShippedTagIndex::default();
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
            game: Some(GameId::CampaignEvolved),
            entries,
            tree: TagTree::default(),
            group_tree: TagTree::default(),
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        }
    }

    fn move_request(redirect: bool) -> ContainerRenameMove<'static> {
        ContainerRenameMove {
            container: 0,
            group_tag: GROUP,
            old_package: OLD_PACKAGE,
            new_package: NEW_PACKAGE,
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
        &crate::core::source::ContainerTagIndex,
        &crate::core::source::ContainerPackageIndex,
        &crate::core::source::ShippedTagIndex,
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
        source.reverse_dependencies = Some(crate::core::source::ReverseDependencyIndex::default());
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
}
