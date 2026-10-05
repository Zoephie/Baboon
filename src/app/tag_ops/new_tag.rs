//! New Tag: choosing a group and path, and creating the tag as a loose file or,
//! in Campaign Evolved, as a new container tag from a shipped template.

use super::*;
use crate::app::tag_ops::group_report::{group_authorability, shipped_counts_by_group};
use crate::app::documents::saving::register_saved_copy_in_loaded_source;
use crate::app::mods::campaign_entry_project_parts;
use crate::app::documents::saving::new_tag_output_path_from_dialog;
use crate::app::documents::saving::load_new_tag_groups;

impl Baboon {
    pub(in crate::app) fn open_new_tag_dialog(&mut self) {
        self.dialogs.open(self.new_tag_dialog());
    }

    /// A fresh New Tag dialog for the active workspace.
    fn new_tag_dialog(&self) -> NewTagDialog {
        let default_game = self
            .model.source()
            .and_then(|source| source.game)
            .unwrap_or(GameId::Halo3)
            .as_str()
            .to_owned();
        let mut dialog = NewTagDialog {
            kit: self.model.active_kit_id(),
            game: default_game,
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
            authorability: None,
        };
        dialog.refresh_groups(&self.model);
        dialog
    }

    /// Open the New Tag dialog pre-filled with a container folder (from a
    /// right-clicked folder node), leaving the leaf name for the user to type.
    pub(in crate::app) fn open_new_tag_dialog_in_folder(&mut self, folder_rel: Option<String>) {
        let mut dialog = self.new_tag_dialog();
        if let Some(folder) = folder_rel.filter(|f| !f.is_empty()) {
            // Pre-fill the path field with the folder + a trailing slash.
            dialog.rel_path = format!("{}/", folder.trim_end_matches('/'));
        }
        self.dialogs.open(dialog);
    }

    /// Create the tag the New Tag dialog describes. The dialog is taken out of
    /// the host while it is, and put back — carrying the reason — when the
    /// tag cannot be made.
    pub(in crate::app) fn create_new_tag(&mut self) {
        let Some(mut dialog) = self.dialogs.close::<NewTagDialog>() else {
            return;
        };
        if !self.create_tag_from(&mut dialog) {
            self.dialogs.open(dialog);
        }
    }

    /// Whether the tag `dialog` describes was created.
    fn create_tag_from(&mut self, dialog: &mut NewTagDialog) -> bool {
        // The tag is written into the active kit's source, and nothing below
        // names a workspace, so without this the tag is created in whichever
        // game was focused when Create was pressed rather than the one the
        // dialog was opened for.
        if !self.focus_navigation_kit(dialog.kit) {
            dialog.error = Some("The workspace this tag was being created in is closed".to_owned());
            return false;
        }
        // Campaign Evolved containers have no loose tags folder to write into —
        if self.refuse_read_only_edit(self.model.active) {
            return false;
        }
        // create the tag purely in memory and let Save / Export Mod write it.
        if self.model.current_source_is_container() {
            return self.create_new_container_tag(dialog);
        }
        let Some(root) = self.model.loaded_tags_root() else {
            dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return false;
        };
        let Some(group) = dialog.groups.get(dialog.selected_group).cloned() else {
            dialog.error = Some("Choose a tag group".to_owned());
            return false;
        };
        let Some(output) = dialog.output_path.clone() else {
            dialog.error = Some("Choose a tag name and location".to_owned());
            return false;
        };
        let output = match new_tag_output_path_from_dialog(&root, &output, &group.extension) {
            Ok((output, rel_path)) => {
                dialog.rel_path = rel_path;
                output
            }
            Err(error) => {
                dialog.error = Some(error);
                return false;
            }
        };
        if output.exists() {
            dialog.error = Some(format!("{} already exists", output.display()));
            return false;
        }
        // A Halo CE or Halo 2 kit takes the classic file, with the 64-byte
        // header its tool writes; `TagFile::new` makes the MCC container, which
        // that kit's Guerilla cannot load.
        let classic = match dialog.game.as_str() {
            "haloce_mcc" => Some(blam_tags::classic::ClassicEngine::HaloCe),
            "halo2_mcc" => Some(blam_tags::classic::ClassicEngine::Halo2V4),
            _ => None,
        };
        let created = match classic {
            Some(engine) => {
                TagFile::new_classic(&group.schema_path, engine).map_err(|e| e.to_string())
            }
            None => TagFile::new(&group.schema_path).map_err(|e| e.to_string()),
        };
        let tag = match created {
            Ok(mut tag) => {
                if CONVERSION_PROFILES.contains(&dialog.game.as_str())
                    && let Err(error) = apply_editing_kit_mcc_header(&mut tag, &dialog.game)
                {
                    dialog.error = Some(error);
                    return false;
                }
                tag
            }
            Err(error) => {
                dialog.error = Some(format!("Could not create tag: {error}"));
                return false;
            }
        };
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            dialog.error = Some(format!("Could not create {}: {error}", parent.display()));
            return false;
        }
        if let Err(error) = tag.write_atomic(&output) {
            dialog.error = Some(format!("Could not write {}: {error}", output.display()));
            return false;
        }

        // Built the way the folder scan builds it, so the key is the scan's
        // `file:` key: a bare display-path key cannot be read back out of the
        // entry index, and a row carrying one made the whole index fail to load.
        let names = self
            .model.source()
            .map(|source| source.names.clone())
            .unwrap_or_default();
        let entry = match loose_file_entry(&root, &output, &names) {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                dialog.error = Some(format!(
                    "Wrote {}, but it does not read back as a tag",
                    output.display()
                ));
                return false;
            }
            Err(error) => {
                dialog.error = Some(format!(
                    "Wrote {}, but could not inspect it: {error:#}",
                    output.display()
                ));
                return false;
            }
        };
        self.register_created_tag(entry, tag);
        self.model.status = format!("Created {}", output.display());
        true
    }

    /// Create a brand-new Campaign Evolved tag in memory (no pak write). The tag
    /// is a defaults-initialized `TagFile::new` from the group schema, registered
    /// dirty at the dialog's container-relative path; Save / Export Mod then write
    /// it via `write_new_tag_container`.
    fn create_new_container_tag(&mut self, dialog: &mut NewTagDialog) -> bool {
        let Some(group) = dialog.groups.get(dialog.selected_group).cloned() else {
            dialog.error = Some("Choose a tag group".to_owned());
            return false;
        };
        let rel = normalize_container_tag_rel(&dialog.rel_path);
        if rel.is_empty() {
            dialog.error = Some("Enter a tag path (e.g. objects/foo/bar)".to_owned());
            return false;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(mut tag) => {
                // `TagFile::new` zeroes the whole file-header generation; the
                // simulation expects Campaign Evolved's.
                if let Err(error) = apply_editing_kit_mcc_header(&mut tag, GameId::CampaignEvolved.as_str()) {
                    dialog.error = Some(error);
                    return false;
                }
                tag
            }
            Err(error) => {
                dialog.error = Some(format!("Could not create tag: {error}"));
                return false;
            }
        };
        match self.add_new_container_tag(&rel, group.group_tag, &group.name, &group.extension, tag)
        {
            Ok(()) => {
                self.model.status = format!("Created {rel}.{} (unsaved)", group.extension);
                true
            }
            Err(error) => {
                dialog.error = Some(error);
                false
            }
        }
    }

    /// Register a brand-new in-memory container tag (shared by New Tag and
    /// Import-of-a-new-path). `logical` is the normalized container-relative path
    /// (no extension). Fails if the path is empty, the group's wrapper can be
    /// neither cloned nor derived, or a new tag already occupies that path.
    pub(in crate::app) fn add_new_container_tag(
        &mut self,
        logical: &str,
        group_tag: u32,
        group_name: &str,
        extension: &str,
        tag: TagFile,
    ) -> Result<(), String> {
        if logical.is_empty() {
            return Err("Enter a tag path (e.g. objects/foo/bar)".to_owned());
        }
        let template =
            new_container_template_for(self.model.find_container_template(group_tag), group_name)?;
        let package = new_container_package(logical, group_name);
        let key = new_tag_entry_key(&package);
        if self.model.kits[self.model.active].parsed_tags.contains_key(&key)
            || self
                .model.source()
                .is_some_and(|s| s.entry_for_key(&key).is_some())
        {
            return Err(format!("A new tag already exists at {logical}"));
        }
        let entry = TagEntry {
            key,
            display_path: format!("{logical}.{extension}"),
            group_tag,
            group_name: Some(group_name.to_owned()),
            location: TagEntryLocation::NewContainer {
                template,
                package,
                group_tag,
            },
        };
        self.register_in_memory_tag(entry, tag);
        Ok(())
    }

    /// Save As for a tag the game ships: an unsaved copy at `new_rel`, which the
    /// editor moves to. Nothing is written until Save or Export Mod, as with New
    /// Tag. The copy is wrapped in the original's own `.uasset`, so it keeps the
    /// Unreal bindings the original has. The original's tab gives way, unless
    /// it holds unsaved edits: those stay with it. Returns the status line.
    pub(in crate::app) fn save_container_tag_as_copy(
        &mut self,
        key: &str,
        new_rel: &str,
    ) -> Result<String, String> {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return Err("Tag is no longer in the source".to_owned());
        };
        let TagEntryLocation::Container {
            container,
            rel_path,
        } = &entry.location
        else {
            return Err("Not a Campaign Evolved container tag".to_owned());
        };
        if new_rel.is_empty() {
            return Err("Enter a tag path (e.g. objects/foo/bar)".to_owned());
        }
        let uasset = rel_path
            .strip_suffix(".ubulk")
            .map(|stem| format!("{stem}.uasset"))
            .ok_or("This tag is not stored as a .ubulk")?;
        let (source, ..) = campaign_entry_project_parts(&entry)
            .ok_or("This tag has no project identity to copy it from")?;
        let group_name = entry
            .group_name
            .clone()
            .unwrap_or_else(|| format_group_tag(entry.group_tag));
        let extension = entry
            .display_path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_owned())
            .unwrap_or_else(|| group_name.clone());
        let package = new_container_package(new_rel, &group_name);
        let copy_entry = TagEntry {
            key: new_tag_entry_key(&package),
            display_path: format!("{new_rel}.{extension}"),
            group_tag: entry.group_tag,
            group_name: Some(group_name),
            location: TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Copy {
                    container: *container,
                    rel_path: uasset,
                    source,
                },
                package,
                group_tag: entry.group_tag,
            },
        };
        // By identity rather than key: a shipped tag at that path is keyed
        // differently from a new one, and either would be shadowed by the copy.
        let kit = self.model.active;
        let taken = self.model.kits[kit].parsed_tags.contains_key(&copy_entry.key)
            || campaign_entry_project_parts(&copy_entry)
                .is_some_and(|(identity, ..)| self.model.campaign_entry_for_identity(kit, &identity).is_some());
        if taken {
            return Err(format!("A tag already exists at {}", copy_entry.display_path));
        }
        // `TagFile` is not `Clone`; its own bytes are how a document is copied,
        // and they are exactly what Save would write.
        let document = self.model.kits[kit]
            .parsed_tags
            .get(key)
            .ok_or("Load the tag before saving it as a copy")?;
        let original_clean = !document.dirty.is_set();
        let bytes = document
            .tag
            .write_to_bytes()
            .map_err(|error| format!("Could not serialize the tag: {error}"))?;
        let copy = TagFile::read_from_bytes(&bytes)
            .map_err(|error| format!("Could not re-read the copied tag: {error}"))?;
        let copy_key = copy_entry.key.clone();
        let display = copy_entry.display_path.clone();
        self.register_in_memory_tag(copy_entry, copy);
        if original_clean {
            self.close_tab(key);
            self.model.kits[kit].selected_key = Some(copy_key);
        }
        Ok(format!("Saved as {display} (unsaved until Save or Export Mod)"))
    }

    /// Rename/move (`duplicate == false`) or copy (`duplicate == true`) a
    /// brand-new container tag to `new_rel`. Nothing is written: a new tag lives
    /// only in its document until Save/Export Mod, so this rewrites the entry
    /// (and re-homes the document under the new key) in memory. Returns the
    /// status line to show.
    pub(in crate::app) fn apply_new_container_rename(
        &mut self,
        key: &str,
        new_rel: &str,
        duplicate: bool,
    ) -> Result<String, String> {
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return Err("Tag is no longer in the source".to_owned());
        };
        let TagEntryLocation::NewContainer {
            template,
            group_tag,
            ..
        } = &entry.location
        else {
            return Err("Not a new Campaign Evolved tag".to_owned());
        };
        let (template, group_tag) = (template.clone(), *group_tag);
        if new_rel.is_empty() {
            return Err("Enter a tag path (e.g. objects/foo/bar)".to_owned());
        }
        let group_name = entry
            .group_name
            .clone()
            .unwrap_or_else(|| format_group_tag(entry.group_tag));
        let extension = entry
            .display_path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_owned())
            .unwrap_or_else(|| group_name.clone());

        if duplicate {
            // `TagFile` is not `Clone`; a round-trip through its own bytes is
            // how a document is copied, and it is exactly what Save would have
            // written anyway.
            let bytes = self.model.kits[self.model.active]
                .parsed_tags
                .get(key)
                .ok_or("Load the tag before copying it")?
                .tag
                .write_to_bytes()
                .map_err(|error| format!("Could not serialize the tag: {error}"))?;
            let copy = TagFile::read_from_bytes(&bytes)
                .map_err(|error| format!("Could not re-read the copied tag: {error}"))?;
            // A copy of a Save As copy wears the same shipped tag's wrapper; a
            // fresh donor would drop the bindings it was made to keep.
            if let NewContainerTemplate::Copy { .. } = &template {
                let package = new_container_package(new_rel, &group_name);
                let copy_key = new_tag_entry_key(&package);
                if self.model.kits[self.model.active].parsed_tags.contains_key(&copy_key)
                    || self
                        .model
                        .source()
                        .is_some_and(|source| source.entry_for_key(&copy_key).is_some())
                {
                    return Err(format!("A new tag already exists at {new_rel}"));
                }
                self.register_in_memory_tag(
                    TagEntry {
                        key: copy_key,
                        display_path: format!("{new_rel}.{extension}"),
                        group_tag,
                        group_name: Some(group_name),
                        location: TagEntryLocation::NewContainer {
                            template,
                            package,
                            group_tag,
                        },
                    },
                    copy,
                );
            } else {
                self.add_new_container_tag(new_rel, group_tag, &group_name, &extension, copy)?;
            }
            return Ok(format!("Copied to {new_rel}.{extension} (unsaved)"));
        }

        let package = new_container_package(new_rel, &group_name);
        let new_key = new_tag_entry_key(&package);
        if new_key == key {
            return Ok(format!("{} is already at that path", entry.display_path));
        }
        if self.model.kits[self.model.active].parsed_tags.contains_key(&new_key)
            || self
                .model.source()
                .is_some_and(|source| source.entry_for_key(&new_key).is_some())
        {
            return Err(format!("A tag already exists at {new_rel}"));
        }
        let Some(document) = self.model.kits[self.model.active].parsed_tags.remove(key) else {
            return Err("Load the tag before renaming it".to_owned());
        };
        // The project stashes overlays under the package path, so the old
        // identity has to go — otherwise the checkpoint keeps a copy of the tag
        // at its previous path and restores it as a second tag next session.
        let kit = self.model.active;
        self.forget_campaign_overlay(kit, key);
        self.forget_new_container_entry(kit, key);
        let old_display = entry.display_path.clone();
        self.register_in_memory_tag(
            TagEntry {
                key: new_key,
                display_path: format!("{new_rel}.{extension}"),
                group_tag: entry.group_tag,
                group_name: Some(group_name),
                location: TagEntryLocation::NewContainer {
                    template,
                    package,
                    group_tag,
                },
            },
            document.tag,
        );
        Ok(format!(
            "Renamed {old_display} → {new_rel}.{extension} (unsaved)"
        ))
    }

    /// Register an in-memory (unsaved) container tag: insert it into the browser
    /// entries, rebuild the folder + group trees so it shows up, open it in a
    /// **dirty** tab, and select it. Used by New Tag and Import for CE.
    pub(in crate::app) fn register_in_memory_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        self.stash_in_memory_tag(entry, tag);
        self.kit_and_view(self.model.active).open_tag_pane(&key);
        self.model.kits[self.model.active].selected_key = Some(key);
    }

    /// The same registration without opening or selecting the tag.
    ///
    /// Recovering a stashed new tag at startup has to put it back in the browser
    /// without deciding what the user is looking at: adopting two of them would
    /// otherwise open two tabs and steal the selection on every launch.
    pub(in crate::app) fn stash_in_memory_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        let folder_seeds = self.model.kits[self.model.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            source.upsert_entry(entry.clone(), &folder_seeds);
        }
        self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        // Index what the new tag points at. Nothing else can: the reverse-
        // dependency builder reads tags from their source, and this one has no
        // source — so without this the tag is invisible to reference queries in
        // both directions, and to the tag tree's children view.
        let dependencies = {
            let mut refs = Vec::new();
            collect_tag_dependency_refs(tag.root(), &mut refs);
            refs
        };
        self.model.kits[self.model.active].set_tag_references(&key, Some(dependencies));
        self.model.kits[self.model.active]
            .parsed_tags
            .insert(key, TagDocument::modified(tag));
    }

    pub(in crate::app) fn register_created_tag(&mut self, entry: TagEntry, tag: TagFile) {
        let key = entry.key.clone();
        let folder_seeds = self.model.kits[self.model.active].folder_seeds();
        if let Some(source) = self.source_mut() {
            register_created_tag_in_source(source, entry.clone(), &folder_seeds);
        }
        self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        // Keyed by entry, so a stale index would answer searches without the
        // tag that was just created.
        self.model.kits[self.model.active].field_index.invalidate();
        self.model.kits[self.model.active]
            .parsed_tags
            .insert(key.clone(), TagDocument::clean(tag));
        self.kit_and_view(self.model.active).open_tag_pane(&key);
        self.model.kits[self.model.active].selected_key = Some(key.clone());
    }

    pub(in crate::app) fn register_saved_copy_if_in_loaded_folder(
        &mut self,
        path: &Path,
    ) -> Result<Option<TagEntry>, String> {
        let Some(source) = self.source_mut() else {
            return Ok(None);
        };
        let registered = register_saved_copy_in_loaded_source(source, path)?;
        if registered.is_some() {
            self.model.kits[self.model.active].generation = self.model.kits[self.model.active].generation.wrapping_add(1);
        }
        Ok(registered)
    }
}

/// Choose the container tag whose `.uasset` a new tag will donate its package
/// structure from, returning its container index and container path.
///
/// Same-group only. A donor of another group is not an option: its wrapper is
/// the wrong shape for the destination class, and the two ways that can go
/// wrong are both silent. A donor carrying properties names *different*
/// properties under the destination's schema, because they are positional; a
/// bare donor given to a class that has properties declares none of them.
///
/// It does not have to be an option, either. A group the game ships no tag of
/// is served by [`NewContainerTemplate::Derived`] instead, which builds the
/// wrapper from the group's own rules. Measured over the mounted paks by
/// `blam-tags`' `ce_group_census` example: of the 141 defined groups the game
/// ships 101, and 36 of the remaining 40 derive. The last four are
/// `object`/`unit`/`item`/`device` — Halo's abstract base groups, which have no
/// standalone instances by design and are refused rather than fabricated.
pub(in crate::app) fn pick_container_template<'a>(
    entries: impl Iterator<Item = &'a TagEntry>,
    group_tag: u32,
) -> Option<(usize, String)> {
    entries
        .filter(|entry| entry.group_tag == group_tag)
        .find_map(|entry| match &entry.location {
            TagEntryLocation::Container {
                container,
                rel_path,
            } => rel_path
                .strip_suffix(".ubulk")
                .map(|stem| (*container, format!("{stem}.uasset"))),
            _ => None,
        })
}

/// Map a container `.ubulk` path to the UE package path the runtime hashes.
/// `Meteorite/Content/Tags/objects/.../foo-biped.ubulk` → `/Game/Tags/objects/.../foo-biped`.
/// Normalize a user-entered container tag path: lowercase, `\`→`/`, collapse
/// repeated slashes, and trim leading/trailing slashes and any tag extension.
/// Yields the container-relative logical path (e.g. `objects/foo/bar`).
/// Walk up from a resolved `Paks` directory to the folder a user would have
/// picked to open it.
///
/// Sessions written before the chosen folder was recorded hold the inner path,
/// and restoring from it remembers *that* as a recent folder -- so "Paks"
/// reappeared after every restart however often it was removed. Walking up
/// while the parent still resolves to the same directory undoes that without
/// assuming a particular layout.
pub(in crate::app) fn install_root_for_paks(paks_dir: &Path) -> PathBuf {
    // The two layouts `find_paks_dir` looks for directly, in its own order.
    // Probing it instead would walk too far: it also *searches* four levels
    // down, so distant ancestors resolve to this same directory and the walk
    // would climb out of the install entirely.
    for suffix in [
        ["Meteorite", "Content", "Paks"].as_slice(),
        ["Content", "Paks"].as_slice(),
    ] {
        let mut candidate = paks_dir;
        let matched = suffix.iter().rev().all(|expected| {
            let hit = candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(expected));
            if hit && let Some(parent) = candidate.parent() {
                candidate = parent;
            }
            hit
        });
        if matched {
            return candidate.to_path_buf();
        }
    }
    paks_dir.to_path_buf()
}

pub(in crate::app) fn normalize_container_tag_rel(input: &str) -> String {
    let lowered = input.trim().replace('\\', "/").to_ascii_lowercase();
    let mut segments: Vec<&str> = lowered
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    // Drop a trailing tag extension on the leaf (`foo.biped` → `foo`).
    if let Some(last) = segments.last_mut()
        && let Some((stem, _ext)) = last.rsplit_once('.')
        && !stem.is_empty()
    {
        *last = stem;
    }
    segments.join("/")
}

/// Decide where a new tag's `.uasset` wrapper will come from.
///
/// A same-group tag in the mounted paks is the first choice: cloning one is the
/// path with the most mileage on it, and it is right for every group the game
/// actually ships. When there is none, the wrapper is derivable — but only for
/// a group whose class adds nothing over `BlamTagDataAssetBase`, since anything
/// more names other packages and needs an import map that cannot be derived
/// from the group alone. A group with neither is refused here rather than
/// producing a tag that cannot be saved later.
pub(in crate::app) fn new_container_template_for(
    donor: Option<(usize, String)>,
    group_name: &str,
) -> Result<NewContainerTemplate, String> {
    if let Some((container, rel_path)) = donor {
        return Ok(NewContainerTemplate::Donor {
            container,
            rel_path,
        });
    }
    let usmap = meteorite_usmap()?;
    if blam_tags::iostore::asset::tag_package::is_bare_group(group_name, &usmap) {
        return Ok(NewContainerTemplate::Derived {
            group: group_name.to_owned(),
        });
    }
    Err(format!(
        "No existing {group_name} tag in the mounted paks to use as a template, and a \
         {group_name} wrapper cannot be derived because the group carries Unreal properties \
         that name other packages"
    ))
}

/// The embedded Campaign Evolved Unreal mappings, parsed once per process.
///
/// It is 2.4 MB, and new_container_template_for parsed it on every call. That
/// includes the stashed-overlay adoption, which retried every frame for an
/// overlay it could not place, so one such overlay reparsed it every frame.
pub(in crate::app) fn meteorite_usmap()
-> Result<std::sync::Arc<blam_tags::iostore::object::usmap::Usmap>, String> {
    static USMAP: std::sync::OnceLock<
        Result<std::sync::Arc<blam_tags::iostore::object::usmap::Usmap>, String>,
    > = std::sync::OnceLock::new();
    USMAP
        .get_or_init(|| {
            blam_tags::iostore::object::usmap::Usmap::meteorite()
                .map(std::sync::Arc::new)
                .map_err(|error| format!("Could not load the Unreal mappings: {error}"))
        })
        .clone()
}

/// The `.uasset` bytes to seed a new tag's package with, cloned or derived.
///
/// `reresolve` finds a fresh donor when the recorded one no longer reads. It is
/// a closure rather than a container index because the two callers scope it to
/// different kits: Save uses the active one, Export Mod the one being exported.
pub(in crate::app) fn new_container_template_bytes(
    template: &NewContainerTemplate,
    containers: &[crate::core::source::MountedContainer],
    package: &str,
    tag_len: u64,
    reresolve: impl FnOnce() -> Option<(usize, String)>,
) -> Result<Vec<u8>, String> {
    match template {
        NewContainerTemplate::Donor {
            container,
            rel_path,
        } => {
            // The recorded donor is a hint, not a fact: container indices are
            // positional, so a remount reorders them and a tag stashed in a
            // project outlives the index it was created against. Re-resolving
            // on a miss is what keeps such a tag saveable instead of failing
            // with "template container is stale".
            if let Some(bytes) = containers
                .get(*container)
                .and_then(|mounted| mounted.archive.read(rel_path).ok())
            {
                return Ok(bytes);
            }
            let (container, rel_path) =
                reresolve().ok_or("No tag in the mounted paks can donate a package template")?;
            containers
                .get(container)
                .ok_or("Template container is stale")?
                .archive
                .read(&rel_path)
                .map_err(|error| format!("Failed to read template .uasset: {error}"))
        }
        NewContainerTemplate::Copy {
            container,
            rel_path,
            source,
        } => containers
            .get(*container)
            .and_then(|mounted| mounted.archive.read(rel_path).ok())
            .ok_or_else(|| {
                format!("{source}, the tag this was copied from, is no longer in the mounted paks")
            }),
        NewContainerTemplate::Derived { group } => {
            let usmap = blam_tags::iostore::object::usmap::Usmap::meteorite()
                .map_err(|error| format!("Could not load the Unreal mappings: {error}"))?;
            // A derived wrapper is a valid template of its own group, so it goes
            // through the same writer path a cloned one does: that path rewrites
            // an identity this already has, and finds nothing to strip.
            blam_tags::iostore::asset::tag_package::build_bare_tag_package(
                group, package, tag_len, &usmap,
            )
            .map(|(bytes, _store)| bytes)
            .map_err(|error| format!("Could not derive a {group} wrapper: {error}"))
        }
    }
}

/// The UE package path a brand-new tag will be written at, from its normalized
/// container-relative path and group name (`objects/foo/bar` + `camera_track`
/// → `/Game/Tags/objects/foo/bar-camera_track`).
///
/// Shared by creation and rename on purpose: the entry key derives from this,
/// and a rename that derived either differently would produce an entry the save
/// and project-overlay paths no longer recognize as the same tag.
pub(in crate::app) fn new_container_package(logical: &str, group_name: &str) -> String {
    format!("/Game/Tags/{logical}-{group_name}")
}

/// A container-relative payload path as a `/Game/…` package path.
///
/// The content root is stripped case-insensitively and the remainder is left
/// exactly as the container spells it. Matching `Meteorite/Content/` literally
/// meant a container that wrote `meteorite/content/` produced a package path
/// with the cook's directory layout still embedded in it, which resolves to
/// nothing — the same class of bug as reassembling a `.uasset` path from a
/// `.ubulk` one. Lowercasing the remainder would be the other failure: the
/// package path is what the destination's directory-index entry is built from,
/// and that index is case-sensitive.
pub(in crate::app) fn container_rel_to_package_path(rel: &str) -> Option<String> {
    let no_ext = rel
        .strip_suffix(".ubulk")
        .or_else(|| rel.strip_suffix(".uasset"))
        .unwrap_or(rel);
    let after = strip_content_root(no_ext);
    Some(format!("/Game/{after}"))
}

/// Strip a container's content root (`Meteorite/Content/`, or a bare
/// `Content/`) however it is capitalised, leaving the rest untouched.
pub(in crate::app) fn strip_content_root(rel: &str) -> &str {
    for prefix in ["Meteorite/Content/", "Content/"] {
        if let Some(candidate) = rel.get(..prefix.len())
            && candidate.eq_ignore_ascii_case(prefix)
        {
            return &rel[prefix.len()..];
        }
    }
    rel
}

pub(in crate::app) fn register_created_tag_in_source(
    source: &mut LoadedSourceData,
    entry: TagEntry,
    pending_folders: &[String],
) {
    source.upsert_entry(entry, pending_folders);
}

impl NewTagDialog {
    /// Reload the groups for the selected game, and what the selected one
    /// allows.
    pub(in crate::app) fn refresh_groups(&mut self, model: &Model) {
        self.load_groups();
        self.refresh_authorability(model);
    }

    /// Load the groups the selected game's schemas define, keeping the
    /// selection in range and clearing what depended on the old list.
    fn load_groups(&mut self) {
        match load_new_tag_groups(&self.game) {
            Ok(groups) if groups.is_empty() => {
                self.groups = groups;
                self.selected_group = 0;
                self.error = Some(format!(
                    "No tag schemas found for {}",
                    self.game
                ));
            }
            Ok(groups) => {
                self.groups = groups;
                self.selected_group = self
                    .selected_group
                    .min(self.groups.len() - 1);
                self.rel_path.clear();
                self.output_path = None;
                self.error = None;
            }
            Err(error) => {
                self.groups.clear();
                self.selected_group = 0;
                self.rel_path.clear();
                self.output_path = None;
                self.error = Some(error);
            }
        }
    }

    /// Answer "can I make one of these?" for the group the New Tag dialog has
    /// selected, and cache it.
    ///
    /// Called when the group or the game changes, which is the only time the
    /// answer can move — it parses the whole mapping table, so it must not run
    /// per frame.
    pub(in crate::app) fn refresh_authorability(&mut self, model: &Model) {
        self.authorability = None;
        // Only Campaign Evolved has native classes standing behind its groups.
        // Everywhere else a new tag is a file, and there is nothing to refuse.
        if self.game != GameId::CampaignEvolved.as_str() {
            return;
        }
        let Some(group) = self
            .groups
            .get(self.selected_group)
        else {
            return;
        };
        let Ok(usmap) = blam_tags::iostore::object::usmap::Usmap::meteorite() else {
            return;
        };
        let shipped = model
            .source()
            .map(shipped_counts_by_group)
            .and_then(|counts| counts.get(&group.group_tag).copied())
            .unwrap_or(0);
        let verdict = group_authorability(&group.name, shipped, &usmap);
        self.authorability = Some((verdict.authorable(), verdict.summary()));
    }

    /// Ask where to write the new tag, under the loaded tags folder.
    pub(in crate::app) fn choose_output_path(&mut self, model: &Model) {
        let Some(root) = model.loaded_tags_root() else {
            self.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .groups
            .get(self.selected_group)
            .cloned()
        else {
            self.error = Some("Choose a tag group".to_owned());
            return;
        };

        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Create New {}", group.name))
            .set_directory(&root)
            .set_file_name(format!("new_tag.{}", group.extension))
            .add_filter(
                format!("{} tag", group.extension),
                &[group.extension.as_str()],
            );
        if let Some(output) = self.output_path.as_ref()
            && let Some(parent) = output.parent()
        {
            dialog = dialog.set_directory(parent);
        }
        let Some(picked) = dialog.save_file() else {
            return;
        };
        match new_tag_output_path_from_dialog(&root, &picked, &group.extension) {
            Ok((output, rel_path)) => {
                self.output_path = Some(output);
                self.rel_path = rel_path;
                self.error = None;
            }
            Err(error) => {
                self.output_path = None;
                self.rel_path.clear();
                self.error = Some(error);
            }
        }
    }
}

impl Model {
    /// Find an existing container tag of `group_tag` and return its owning
    /// container index plus its `.uasset` container path — the package template
    /// for a new tag of the same group.
    pub(in crate::app) fn find_container_template(&self, group_tag: u32) -> Option<(usize, String)> {
        self.find_container_template_in(self.active, group_tag)
    }

    /// A specific kit's template. Project recovery names its kit: the container
    /// a stashed new tag is modelled on has to come from the source that tag
    /// belongs to, not from whichever kit happens to be focused.
    ///
    /// Returning `None` is an ordinary answer, not a failure: it means the game
    /// ships no tag of this group, and the caller derives the wrapper instead.
    /// See [`pick_container_template`] for why no other group can stand in.
    pub(in crate::app) fn find_container_template_in(
        &self,
        kit: usize,
        group_tag: u32,
    ) -> Option<(usize, String)> {
        let source = self.kits.get(kit)?.source.as_ref()?;
        pick_container_template(
            source.entries.iter().chain(source.all_entries.iter()),
            group_tag,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::convert::CAMPAIGN_EVOLVED_GENERATION;

    // Creating a Campaign Evolved tag in a group the game ships no instance of.
    //
    // The dialog offers all 141 groups the definitions define, but the game ships a
    // tag for only 101 of them. Creation used to require an existing same-group
    // `.uasset` to donate the UE5 package structure, so the other 40 -- among them
    // `cinematic_scene`, the reported case -- could not be created at all. The
    // wrapper is now *derived* from the group's own rules when no same-group tag
    // ships, which covers 36 of those 40. The last four are `object`, `unit`,
    // `item` and `device`: Halo's abstract base groups, which have no standalone
    // instances by design and stay refused.

    fn definitions() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions")
    }

    fn definition(group: &str) -> std::path::PathBuf {
        definitions()
            .join("haloce_evolved")
            .join(format!("{group}.json"))
    }

    /// A mounted container tag, as the browser would have indexed it.
    fn container_entry(container: usize, path: &str, group: &str) -> TagEntry {
        let group_tag = TagFile::new(definition(group))
            .unwrap_or_else(|e| panic!("{group} has no CE schema: {e}"))
            .header
            .group_tag;
        TagEntry {
            key: format!("ublock:pakchunk0:{path}"),
            display_path: format!("{path}.{group}"),
            group_tag,
            group_name: Some(group.to_owned()),
            location: TagEntryLocation::Container {
                container,
                rel_path: format!("Tags/{path}-{group}.ubulk"),
            },
        }
    }

    /// An app with `entries` mounted as a Campaign Evolved container set, and
    /// `open` open as a clean document.
    fn campaign_app(entries: Vec<TagEntry>, open: &TagEntry) -> Baboon {
        let mut app = Baboon::for_test();
        let tree = crate::core::source::build_tree(&entries);
        let group_tree = crate::core::source::build_group_tree(&entries);
        app.install_loaded_source(LoadedSourceData {
            label: "save as test containers".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: PathBuf::from("C:/save-as-test/Paks"),
                containers: Vec::new(),
                index: Arc::new(crate::core::source::ContainerTagIndex::default()),
                packages: Arc::new(crate::core::source::ContainerPackageIndex::default()),
                shipped: Arc::new(crate::core::source::ShippedTagIndex::default()),
            },
            names: TagNameIndex::default(),
            game: None,
            entries,
            tree,
            group_tree,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
            key_hints: Default::default(),
            complete_scan: false,
            chosen_kit_layout: None,
        });
        let group = open.group_name.clone().unwrap();
        app.model.kits[0]
            .parsed_tags
            .insert(open.key.clone(), TagDocument::clean(TagFile::new(definition(&group)).unwrap()));
        app.kit_and_view(0).open_tag_pane(&open.key);
        app.model.kits[0].selected_key = Some(open.key.clone());
        app
    }

    fn tab_open(app: &Baboon, key: &str) -> bool {
        app.model.kits[0].open_tabs.iter().any(|tab| tab == key)
    }

    fn group_tag_of(group: &str) -> u32 {
        TagFile::new(definition(group))
            .unwrap_or_else(|e| panic!("{group} has no CE schema: {e}"))
            .header
            .group_tag
    }

    /// A new Campaign Evolved tag carries the generation the game ships, both as
    /// `TagFile::new` builds it and after New Tag stamps it again.
    ///
    /// The engine stamps it itself now: it takes the game from the definitions
    /// folder, which `TagFile::new` used to read from `_meta.json` (where Campaign
    /// Evolved calls itself `halocampaignevolved`, a name no table knew, so the
    /// header stayed zeroed). Baboon's own stamp, from the profile's game, must
    /// agree with it rather than undo it.
    ///
    /// The values are measured, not chosen: all 12,289 shipped CE tag blobs read
    /// `1 / 2 / 0xffffffff`, across all 101 groups, with no variation. See
    /// `the_shipped_tag_header_generation` in blam-tags.
    #[test]
    fn a_new_campaign_evolved_tag_carries_the_shipped_generation() {
        let mut tag = TagFile::new(definition("cinematic_scene")).expect("build from the CE schema");
        let generation = |tag: &TagFile| {
            (
                tag.header.build_version,
                tag.header.build_number,
                tag.header.version,
            )
        };
        assert_eq!(
            generation(&tag),
            CAMPAIGN_EVOLVED_GENERATION,
            "TagFile::new stamps a Campaign Evolved tag from its definitions folder"
        );
        apply_editing_kit_mcc_header(&mut tag, GameId::CampaignEvolved.as_str()).expect("CE is a known game");
        assert_eq!(generation(&tag), CAMPAIGN_EVOLVED_GENERATION);
    }

    /// Campaign Evolved's generation is Halo Reach's, because a CE `.ubulk` *is* a
    /// Reach-format tag — the UE5 package around it is only a wrapper. Pinned as an
    /// equality rather than two copies of the same literals, so the two can never be
    /// changed apart by accident.
    #[test]
    fn campaign_evolved_carries_the_same_generation_as_reach() {
        let stamp = |game: &str| {
            let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
            apply_editing_kit_mcc_header(&mut tag, game).unwrap_or_else(|e| panic!("{game}: {e}"));
            (
                tag.header.build_version,
                tag.header.build_number,
                tag.header.version,
            )
        };
        assert_eq!(stamp(GameId::CampaignEvolved.as_str()), stamp("haloreach_mcc"));
        assert_eq!(stamp(GameId::CampaignEvolved.as_str()), CAMPAIGN_EVOLVED_GENERATION);
    }

    /// Adding Campaign Evolved must not have moved the editing-kit games.
    ///
    /// `apply_editing_kit_mcc_header` is the only function the CE creation path
    /// shares with them, so it is the only place a CE change could reach
    /// H2EK/H3EK/H3ODSTEK/H4EK/H2AMPEK tags. The generations are pinned here as well
    /// as in `editing_kit_header_defaults_match_profile_generations` because that
    /// test asserts on serialized bytes and this one asserts on the match arms —
    /// CE joined the `build_number = 2` group, and H3/ODST must stay at 1.
    #[test]
    fn adding_campaign_evolved_left_the_editing_kit_games_alone() {
        for (game, build_number) in [
            ("halo3_mcc", 1),
            ("halo3odst_mcc", 1),
            ("haloreach_mcc", 2),
            ("halo4_mcc", 2),
            ("halo2amp_mcc", 2),
        ] {
            let schema = definitions().join(game).join("globals.json");
            let mut tag = TagFile::new(&schema).unwrap_or_else(|e| panic!("{game} globals: {e}"));
            apply_editing_kit_mcc_header(&mut tag, game).unwrap_or_else(|e| panic!("{game}: {e}"));
            assert_eq!(tag.header.build_version, 1, "{game} build_version");
            assert_eq!(tag.header.build_number, build_number, "{game} build_number");
            assert_eq!(tag.header.version, u32::MAX, "{game} version");
        }
    }

    /// The CE arm is an early return on one exact string, so it must not have
    /// widened what the function accepts. A game with no known generation is still
    /// an error rather than a tag stamped with someone else's.
    #[test]
    fn a_game_with_no_known_generation_is_still_rejected() {
        for game in ["", "haloce_evolved_x", "halo5", "haloce"] {
            let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
            // Start from a blank generation so any stamp at all shows: the engine
            // already stamps a CE-schema tag with Reach's values, which a wrong
            // stamp could repeat unseen.
            tag.header.build_version = 0;
            tag.header.build_number = 0;
            tag.header.version = 0;
            let before = (tag.header.build_version, tag.header.build_number, tag.header.version);
            assert!(
                apply_editing_kit_mcc_header(&mut tag, game).is_err(),
                "{game:?} should have no known tag-header defaults"
            );
            let after = (tag.header.build_version, tag.header.build_number, tag.header.version);
            assert_eq!(after, before, "{game:?} changed the header anyway");
        }
    }

    /// The classic profiles are a deliberate no-op, not an error.
    ///
    /// Halo CE and Halo 2 tags have no MCC generation: `write_classic_tag` copies the
    /// original 64-byte header through and patches only the checksum, so those three
    /// fields are not part of the format. Returning an error here would make every
    /// classic conversion fail; stamping them would corrupt the header the kit reads.
    /// The contract is "leave it alone, and say that went fine".
    #[test]
    fn a_classic_profile_is_left_unstamped_without_failing() {
        for game in ["haloce_mcc", "halo2_mcc"] {
            let mut tag = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
            // Start from a blank generation so any stamp at all shows: the engine
            // already stamps a CE-schema tag with Reach's values, which a wrong
            // stamp could repeat unseen.
            tag.header.build_version = 0;
            tag.header.build_number = 0;
            tag.header.version = 0;
            let before = (tag.header.build_version, tag.header.build_number, tag.header.version);
            assert!(
                apply_editing_kit_mcc_header(&mut tag, game).is_ok(),
                "{game:?} should be a no-op, not an error"
            );
            let after = (tag.header.build_version, tag.header.build_number, tag.header.version);
            assert_eq!(after, before, "{game:?} changed the header anyway");
        }
    }

    /// Every way a Campaign Evolved tag comes into existence has to end up with the
    /// same generation, or "correct" depends on which menu item was used.
    ///
    /// Three paths reach `add_new_container_tag`: New Tag builds from the schema and
    /// stamps; Copy round-trips an existing CE tag through its own bytes and so
    /// inherits a header that is already right; Import takes a file from disk, whose
    /// header is the *source's* and has to be restamped. This pins the two that go
    /// through a `TagFile` the caller supplies.
    #[test]
    fn an_imported_tag_is_restamped_for_campaign_evolved() {
        // A tag carrying another kit's generation — Halo 3's, `build_number = 1`.
        let mut foreign = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        apply_editing_kit_mcc_header(&mut foreign, "halo3_mcc").expect("H3 is a known game");
        assert_ne!(
            (
                foreign.header.build_version,
                foreign.header.build_number,
                foreign.header.version
            ),
            CAMPAIGN_EVOLVED_GENERATION,
            "the fixture has to start wrong for this to prove anything"
        );

        apply_editing_kit_mcc_header(&mut foreign, GameId::CampaignEvolved.as_str()).expect("CE is a known game");
        assert_eq!(
            (
                foreign.header.build_version,
                foreign.header.build_number,
                foreign.header.version
            ),
            CAMPAIGN_EVOLVED_GENERATION
        );

        // And a zeroed header — what an older Baboon wrote — restamps too.
        let mut zeroed = TagFile::new(definition("cinematic_scene")).expect("any tag will do");
        apply_editing_kit_mcc_header(&mut zeroed, GameId::CampaignEvolved.as_str()).expect("CE is a known game");
        assert_eq!(
            (
                zeroed.header.build_version,
                zeroed.header.build_number,
                zeroed.header.version
            ),
            CAMPAIGN_EVOLVED_GENERATION
        );
    }

    /// Every group the dialog offers must have a schema on disk, or the group list
    /// and the creation path disagree about what is creatable.
    #[test]
    fn every_offered_campaign_evolved_group_has_a_schema() {
        let groups = load_new_tag_groups("haloce_evolved").expect("the CE group table loads");
        assert!(
            groups.len() > 100,
            "expected the full CE group table, got {}",
            groups.len()
        );
        for group in &groups {
            assert!(group.schema_path.is_file(), "{} has no schema", group.name);
        }
        assert!(
            groups.iter().any(|g| g.name == "cinematic_scene"),
            "cinematic_scene is offered by the dialog"
        );
    }

    /// A group the game *does* ship donates its own wrapper, which is both the
    /// closest match and the only donor a binding can be carried through.
    #[test]
    fn a_shipped_group_donates_its_own_wrapper() {
        let entries = vec![
            container_entry(0, "objects/vehicles/warthog/warthog", "collision_model"),
            container_entry(0, "objects/characters/elite/elite", "biped"),
        ];
        let (container, rel) = pick_container_template(entries.iter(), group_tag_of("biped"))
            .expect("biped is mounted, so it donates its own");
        assert_eq!(container, 0);
        assert!(rel.ends_with("-biped.uasset"), "got {rel}");
    }

    /// The reported bug. `cinematic_scene` ships no tag, so the same-group scan
    /// finds nothing — and creation used to stop there with "No existing
    /// cinematic_scene tag in the mounted paks to use as a template".
    ///
    /// It now derives instead, which is why the scan returning `None` is the
    /// *expected* answer here rather than the failure it used to be. Both halves are
    /// asserted: no donor is found, and the group is still creatable.
    #[test]
    fn a_group_the_game_ships_no_tag_of_is_derived_rather_than_donated() {
        let entries = vec![
            container_entry(0, "objects/characters/elite/elite", "biped"),
            container_entry(1, "objects/vehicles/warthog/warthog", "collision_model"),
        ];
        let donor = pick_container_template(entries.iter(), group_tag_of("cinematic_scene"));
        assert!(
            donor.is_none(),
            "no other group may stand in as a donor, got {donor:?}"
        );
        assert!(
            matches!(
                new_container_template_for(donor, "cinematic_scene"),
                Ok(NewContainerTemplate::Derived { .. })
            ),
            "cinematic_scene has to remain creatable without one"
        );
    }

    /// The four groups that are neither shipped nor derivable are Halo's abstract
    /// base groups. Refusing them is the point: they have no standalone instances,
    /// and a wrapper donated from some bare group would declare none of the
    /// properties their classes actually carry — the silent failure cloning had.
    ///
    /// This is the half that makes the test above mean something. Asserting only
    /// that `cinematic_scene` is allowed would pass just as well if the gate had
    /// been deleted outright.
    #[test]
    fn an_abstract_base_group_is_refused_by_name() {
        for group in ["object", "unit", "item", "device"] {
            let error = new_container_template_for(None, group)
                .expect_err("an abstract base group has no wrapper to derive");
            assert!(
                error.contains(group),
                "the refusal has to name the group, got {error:?}"
            );
        }
    }

    /// A `biped` wrapper holds an `AssetReference` indexed against
    /// `BlamBipedTagDataAsset`'s schema, so donating it to another group would name
    /// a different property under the destination's.
    #[test]
    fn a_donor_of_another_group_is_never_offered() {
        let entries = vec![container_entry(
            0,
            "objects/characters/elite/elite",
            "biped",
        )];
        assert!(
            pick_container_template(entries.iter(), group_tag_of("cinematic_scene")).is_none(),
            "biped is not a cross-group donor"
        );
        // Nor is a bare one. `collision_model` carries no properties of its own, so
        // `blam-tags` would accept it as a donor — the reason it is refused is the
        // destination, not the donor.
        let bare = vec![container_entry(
            0,
            "objects/vehicles/warthog/warthog",
            "collision_model",
        )];
        assert!(
            pick_container_template(bare.iter(), group_tag_of("cinematic_scene")).is_none(),
            "a bare donor is still the wrong group"
        );
    }

    /// Only mounted container tags can donate. An in-memory tag created earlier in
    /// the session has no `.uasset` in any pak to read.
    #[test]
    fn an_unsaved_tag_is_not_a_donor() {
        let entries = vec![TagEntry {
            key: "newtag:/Game/Tags/test/probe-collision_model".into(),
            display_path: "test/probe.collision_model".into(),
            group_tag: group_tag_of("collision_model"),
            group_name: Some("collision_model".into()),
            location: TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Donor {
                    container: 0,
                    rel_path: "Tags/other-collision_model.uasset".into(),
                },
                package: "/Game/Tags/test/probe-collision_model".into(),
                group_tag: group_tag_of("collision_model"),
            },
        }];
        assert!(
            pick_container_template(entries.iter(), group_tag_of("collision_model")).is_none(),
            "an unsaved tag has no .uasset to donate"
        );
    }

    /// A mod without `_P` mounts at the same priority as the game's own
    /// containers and loses, so it builds correctly and does nothing. Renaming
    /// the default to something meaningful is exactly how it gets dropped --
    /// which is how one was reported.
    /// A session written before the chosen folder was recorded holds the
    /// resolved `Paks` directory. Restoring from it put that directory back
    /// into the recents list on every launch, which is how "Paks" kept
    /// reappearing however often it was removed.
    #[test]
    fn a_paks_directory_walks_back_up_to_the_opened_folder() {
        let root = std::env::temp_dir().join(format!("baboon-paks-{}", std::process::id()));
        let paks = root.join("Meteorite").join("Content").join("Paks");
        std::fs::create_dir_all(&paks).unwrap();
        // `find_paks_dir` needs a container present to recognise the folder.
        std::fs::write(paks.join("pakchunk0-WinGDK.utoc"), []).unwrap();

        assert_eq!(super::install_root_for_paks(&paks), root);
        // Already the opened folder: nothing to strip.
        assert_eq!(super::install_root_for_paks(&root), root);
        // The shorter layout the resolver also accepts.
        assert_eq!(
            super::install_root_for_paks(&root.join("Content").join("Paks")),
            root
        );
        // An unfamiliar layout is left exactly as it is rather than guessed at.
        let odd = root.join("somewhere").join("Paks");
        assert_eq!(super::install_root_for_paks(&odd), odd);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_container_tag_rel_cleans_path() {
        // Lowercases, normalizes separators, trims slashes, drops a leaf extension.
        assert_eq!(
            normalize_container_tag_rel("Objects\\Characters/Foo/Bar"),
            "objects/characters/foo/bar"
        );
        assert_eq!(
            normalize_container_tag_rel("/objects//foo/bar.biped/"),
            "objects/foo/bar"
        );
        assert_eq!(normalize_container_tag_rel("  Foo.Weapon  "), "foo");
        assert_eq!(normalize_container_tag_rel(""), "");
        assert_eq!(normalize_container_tag_rel("///"), "");
    }

    // The New Tag dialog through its Create: the create takes the dialog back
    // from the host, and returns it — with the reason — only when no tag was
    // made.

    /// A loose Halo 3 editing kit in a fresh temporary folder, holding nothing.
    fn loose_app() -> (Baboon, PathBuf) {
        loose_kit(GameId::Halo3)
    }

    /// A loose `game` editing kit in a fresh temporary folder, holding nothing.
    fn loose_kit(game: GameId) -> (Baboon, PathBuf) {
        let root = std::env::temp_dir()
            .join(format!("baboon-new-tag-{}", uuid::Uuid::new_v4()))
            .join("tags");
        fs::create_dir_all(&root).unwrap();
        let mut app = Baboon::for_test();
        let source = crate::core::source::load_editing_kit_layout(
            root.clone(),
            "New Tag Kit".to_owned(),
            game,
            &app.model.default_names,
            &crate::core::bundled::locate_definitions_root(),
        )
        .expect("an empty loose kit loads");
        app.install_loaded_source(source);
        (app, root)
    }

    /// With nowhere to write, Create leaves the dialog open and says why.
    #[test]
    fn a_refused_create_keeps_the_dialog_with_its_reason() {
        let mut app = Baboon::for_test();
        app.open_new_tag_dialog();
        app.create_new_tag();
        let dialog = app.dialogs.get::<NewTagDialog>().expect("still open");
        assert_eq!(
            dialog.error.as_deref(),
            Some("Load a loose editing-kit tags folder before creating a tag")
        );
    }

    /// A tag that can be made is written, and the dialog closes.
    #[test]
    fn a_created_tag_closes_the_dialog() {
        let (mut app, root) = loose_app();
        app.open_new_tag_dialog();
        let output = root.join("objects").join("new").join("new.scenery");
        {
            let dialog = app.dialogs.get_mut::<NewTagDialog>().expect("open");
            dialog.selected_group = dialog
                .groups
                .iter()
                .position(|group| group.name == "scenery")
                .expect("Halo 3 defines scenery");
            dialog.output_path = Some(output.clone());
        }
        app.create_new_tag();
        let created = output.exists();
        let _ = fs::remove_dir_all(root.parent().unwrap());
        assert!(created, "{}", app.model.status);
        assert!(app.dialogs.get::<NewTagDialog>().is_none());
        assert_eq!(app.model.status, format!("Created {}", output.display()));
    }

    /// A Halo CE or Halo 2 kit gets a classic tag, with the header its tool
    /// writes, that reads back as that game's. New Tag refused both games,
    /// saying it had no writer for the classic header.
    #[test]
    fn a_classic_kit_gets_a_classic_tag() {
        use blam_tags::classic::ClassicEngine;
        for (game, group, engine) in [
            (GameId::HaloCe, "scenery", ClassicEngine::HaloCe),
            (GameId::Halo2, "weapon", ClassicEngine::Halo2V4),
        ] {
            let (mut app, root) = loose_kit(game);
            app.open_new_tag_dialog();
            let output = root.join(format!("objects/new/new.{group}"));
            {
                let dialog = app.dialogs.get_mut::<NewTagDialog>().expect("open");
                assert_eq!(dialog.game, game.as_str());
                dialog.selected_group = dialog
                    .groups
                    .iter()
                    .position(|candidate| candidate.name == group)
                    .unwrap_or_else(|| panic!("{game:?} defines {group}"));
                dialog.output_path = Some(output.clone());
            }
            app.create_new_tag();
            let still_open = app
                .dialogs
                .get::<NewTagDialog>()
                .map(|dialog| dialog.error.clone());
            let read = crate::core::source::read_tag_at_path(
                &output,
                Some(game),
                Some(&crate::core::bundled::locate_definitions_root()),
                load_new_tag_groups(game.as_str())
                    .unwrap()
                    .iter()
                    .find(|candidate| candidate.name == group)
                    .unwrap()
                    .group_tag,
            );
            let _ = fs::remove_dir_all(root.parent().unwrap());
            assert_eq!(still_open, None, "{game:?}: {}", app.model.status);
            let tag = read.unwrap_or_else(|error| panic!("{game:?}: {error:#}"));
            assert_eq!(tag.classic_engine(), Some(engine), "{game:?}");
        }
    }

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    #[test]
    fn new_tags_strip_doc_strings_and_explanations_on_write() {
        // The engine strips explanation fields + cleans field names when building
        // a layout from JSON, so a freshly-created tag's embedded blay matches
        // shipped tags — no `#help`/`:units` text, no explanation bodies.
        let tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(
            !contains(b"attenuating"),
            "must not embed explanation/help text"
        );
        assert!(
            !contains(b"world units"),
            "must not embed `:units` annotations"
        );
        // And it must still round-trip cleanly.
        TagFile::read_from_bytes(&bytes).expect("stripped tag must parse");
    }

    /// Save As of a shipped tag makes an unsaved copy and moves the editor to
    /// it. The copy is wrapped in the original's own `.uasset`, so it keeps the
    /// original's Unreal bindings when it is written.
    #[test]
    fn save_as_of_a_shipped_tag_switches_to_an_unsaved_copy() {
        let original = container_entry(0, "objects/props/crate", "camera_track");
        let mut app = campaign_app(vec![original.clone()], &original);

        let status = app
            .save_container_tag_as_copy(&original.key, "objects/props/crate_copy")
            .unwrap();

        assert!(status.starts_with("Saved as objects/props/crate_copy"), "{status}");
        let selected = app.model.kits[0].selected_key.clone().unwrap();
        let copy = app.model.entry_for_key(&selected).cloned().expect("the copy is in the browser");
        assert_eq!(copy.display_path, "objects/props/crate_copy.camera_track");
        match &copy.location {
            TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Copy { container, rel_path, source },
                ..
            } => {
                assert_eq!(*container, 0);
                assert_eq!(rel_path, "Tags/objects/props/crate-camera_track.uasset");
                assert_eq!(
                    Some(source.as_str()),
                    campaign_entry_project_parts(&original).map(|parts| parts.0).as_deref()
                );
            }
            _ => panic!("expected a copy of the original's wrapper"),
        }
        assert_eq!(
            crate::app::mods::review::wrapper_origin_for(&copy.location),
            Some(blam_tags::iostore::writer::WrapperOrigin::Copy),
            "the copy keeps its bindings"
        );
        assert!(app.model.kits[0].parsed_tags[&selected].dirty.is_set(), "nothing is written yet");
        assert!(tab_open(&app, &selected));
        assert!(!tab_open(&app, &original.key), "the clean original gives way");
    }

    /// An original with unsaved edits keeps its tab: the edits stay with it
    /// rather than closing with the tab.
    #[test]
    fn save_as_keeps_an_original_with_unsaved_edits_open() {
        let original = container_entry(0, "objects/props/crate", "camera_track");
        let mut app = campaign_app(vec![original.clone()], &original);
        app.model.kits[0].parsed_tags.get_mut(&original.key).unwrap().dirty.touch();

        app.save_container_tag_as_copy(&original.key, "objects/props/crate_copy")
            .unwrap();

        let selected = app.model.kits[0].selected_key.clone().unwrap();
        assert_ne!(selected, original.key, "the editor is on the copy");
        assert!(tab_open(&app, &original.key));
        assert!(app.model.kits[0].parsed_tags[&original.key].dirty.is_set());
    }

    /// A path a shipped tag already holds is refused: the copy would shadow it.
    #[test]
    fn save_as_onto_a_shipped_tag_is_refused() {
        let original = container_entry(0, "objects/props/crate", "camera_track");
        let other = container_entry(0, "objects/props/barrel", "camera_track");
        let mut app = campaign_app(vec![original.clone(), other], &original);

        let error = app
            .save_container_tag_as_copy(&original.key, "objects/props/barrel")
            .unwrap_err();

        assert!(error.contains("already exists"), "{error}");
        assert_eq!(app.model.kits[0].selected_key.as_deref(), Some(original.key.as_str()));
        assert!(tab_open(&app, &original.key));
    }

    /// Save As of an unsaved copy keeps wearing the shipped tag's wrapper.
    #[test]
    fn a_copy_of_a_copy_keeps_the_shipped_wrapper() {
        let original = container_entry(0, "objects/props/crate", "camera_track");
        let mut app = campaign_app(vec![original.clone()], &original);
        app.save_container_tag_as_copy(&original.key, "objects/props/crate_copy")
            .unwrap();
        let first = app.model.kits[0].selected_key.clone().unwrap();

        app.apply_new_container_rename(&first, "objects/props/crate_copy_2", true)
            .unwrap();

        let second = app.model.kits[0].selected_key.clone().unwrap();
        assert_ne!(second, first);
        let location = &app.model.entry_for_key(&second).unwrap().location;
        assert!(matches!(
            location,
            TagEntryLocation::NewContainer { template: NewContainerTemplate::Copy { .. }, .. }
        ));
    }
}
