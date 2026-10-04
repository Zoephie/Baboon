//! New Tag: choosing a group and path, and creating the tag as a loose file or,
//! in Campaign Evolved, as a new container tag from a shipped template.

use super::*;
use crate::app::documents::saving::register_saved_copy_in_loaded_source;
use crate::app::documents::saving::new_tag_output_path_from_dialog;
use crate::app::documents::saving::load_new_tag_groups;

impl Baboon {
    pub(in crate::app) fn open_new_tag_dialog(&mut self) {
        let default_game = self
            .model.source()
            .and_then(|source| source.game)
            .unwrap_or(GameId::Halo3)
            .as_str()
            .to_owned();
        self.tag_ops.new_tag_dialog = NewTagDialog {
            kit: Some(self.model.active_kit_id()),
            game: default_game,
            rel_path: String::new(),
            output_path: None,
            groups: Vec::new(),
            selected_group: 0,
            error: None,
            authorability: None,
        };
        self.refresh_new_tag_groups();
        self.tag_ops.new_tag_open = true;
    }

    /// Open the New Tag dialog pre-filled with a container folder (from a
    /// right-clicked folder node), leaving the leaf name for the user to type.
    pub(in crate::app) fn open_new_tag_dialog_in_folder(&mut self, folder_rel: Option<String>) {
        self.open_new_tag_dialog();
        if let Some(folder) = folder_rel.filter(|f| !f.is_empty()) {
            // Pre-fill the path field with the folder + a trailing slash.
            self.tag_ops.new_tag_dialog.rel_path = format!("{}/", folder.trim_end_matches('/'));
        }
    }

    pub(in crate::app) fn refresh_new_tag_groups(&mut self) {
        self.refresh_new_tag_groups_inner();
        self.refresh_group_authorability();
    }

    pub(in crate::app) fn refresh_new_tag_groups_inner(&mut self) {
        match load_new_tag_groups(&self.tag_ops.new_tag_dialog.game) {
            Ok(groups) if groups.is_empty() => {
                self.tag_ops.new_tag_dialog.groups = groups;
                self.tag_ops.new_tag_dialog.selected_group = 0;
                self.tag_ops.new_tag_dialog.error = Some(format!(
                    "No tag schemas found for {}",
                    self.tag_ops.new_tag_dialog.game
                ));
            }
            Ok(groups) => {
                self.tag_ops.new_tag_dialog.groups = groups;
                self.tag_ops.new_tag_dialog.selected_group = self
                    .tag_ops.new_tag_dialog
                    .selected_group
                    .min(self.tag_ops.new_tag_dialog.groups.len() - 1);
                self.tag_ops.new_tag_dialog.rel_path.clear();
                self.tag_ops.new_tag_dialog.output_path = None;
                self.tag_ops.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.groups.clear();
                self.tag_ops.new_tag_dialog.selected_group = 0;
                self.tag_ops.new_tag_dialog.rel_path.clear();
                self.tag_ops.new_tag_dialog.output_path = None;
                self.tag_ops.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(in crate::app) fn choose_new_tag_output_path(&mut self) {
        let Some(root) = self.model.loaded_tags_root() else {
            self.tag_ops.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .tag_ops.new_tag_dialog
            .groups
            .get(self.tag_ops.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.tag_ops.new_tag_dialog.error = Some("Choose a tag group".to_owned());
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
        if let Some(output) = self.tag_ops.new_tag_dialog.output_path.as_ref()
            && let Some(parent) = output.parent()
        {
            dialog = dialog.set_directory(parent);
        }
        let Some(picked) = dialog.save_file() else {
            return;
        };
        match new_tag_output_path_from_dialog(&root, &picked, &group.extension) {
            Ok((output, rel_path)) => {
                self.tag_ops.new_tag_dialog.output_path = Some(output);
                self.tag_ops.new_tag_dialog.rel_path = rel_path;
                self.tag_ops.new_tag_dialog.error = None;
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.output_path = None;
                self.tag_ops.new_tag_dialog.rel_path.clear();
                self.tag_ops.new_tag_dialog.error = Some(error);
            }
        }
    }

    pub(in crate::app) fn create_new_tag(&mut self) {
        // The tag is written into the active kit's source, and nothing below
        // names a workspace, so without this the tag is created in whichever
        // game was focused when Create was pressed rather than the one the
        // dialog was opened for.
        let dialog_kit = self.tag_ops.new_tag_dialog.kit;
        if !dialog_kit.is_some_and(|kit| self.focus_navigation_kit(kit)) {
            self.tag_ops.new_tag_dialog.error =
                Some("The workspace this tag was being created in is closed".to_owned());
            return;
        }
        // Campaign Evolved containers have no loose tags folder to write into —
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        // create the tag purely in memory and let Save / Export Mod write it.
        if self.model.current_source_is_container() {
            self.create_new_container_tag();
            return;
        }
        let Some(root) = self.model.loaded_tags_root() else {
            self.tag_ops.new_tag_dialog.error =
                Some("Load a loose editing-kit tags folder before creating a tag".to_owned());
            return;
        };
        let Some(group) = self
            .tag_ops.new_tag_dialog
            .groups
            .get(self.tag_ops.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.tag_ops.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };
        let Some(output) = self.tag_ops.new_tag_dialog.output_path.clone() else {
            self.tag_ops.new_tag_dialog.error = Some("Choose a tag name and location".to_owned());
            return;
        };
        let output = match new_tag_output_path_from_dialog(&root, &output, &group.extension) {
            Ok((output, rel_path)) => {
                self.tag_ops.new_tag_dialog.rel_path = rel_path;
                output
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.error = Some(error);
                return;
            }
        };
        if output.exists() {
            self.tag_ops.new_tag_dialog.error = Some(format!("{} already exists", output.display()));
            return;
        }
        // `TagFile::new` can only build an MCC container — it hard-codes
        // `TagContainer::Mcc` and `Endian::Le`, and nothing synthesizes a classic
        // 64-byte header. Writing one into an H1EK/H2EK tags tree produces a file
        // Guerilla cannot load, and one Baboon itself re-reads as MCC, so nothing
        // surfaces the mistake. Refuse until there is a classic constructor.
        if CLASSIC_CONVERSION_GAMES.contains(&self.tag_ops.new_tag_dialog.game.as_str()) {
            self.tag_ops.new_tag_dialog.error = Some(format!(
                "Baboon cannot create a new {} tag: classic Halo CE and Halo 2 \
                 tags carry a 64-byte header it has no writer for, so the file \
                 would not load in the editing kit. Duplicate an existing tag \
                 instead.",
                self.tag_ops.new_tag_dialog.game
            ));
            return;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(mut tag) => {
                if CONVERSION_PROFILES.contains(&self.tag_ops.new_tag_dialog.game.as_str())
                    && let Err(error) =
                        apply_editing_kit_mcc_header(&mut tag, &self.tag_ops.new_tag_dialog.game)
                {
                    self.tag_ops.new_tag_dialog.error = Some(error);
                    return;
                }
                tag
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.error = Some(format!("Could not create tag: {error}"));
                return;
            }
        };
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.tag_ops.new_tag_dialog.error =
                Some(format!("Could not create {}: {error}", parent.display()));
            return;
        }
        if let Err(error) = tag.write_atomic(&output) {
            self.tag_ops.new_tag_dialog.error =
                Some(format!("Could not write {}: {error}", output.display()));
            return;
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
                self.tag_ops.new_tag_dialog.error = Some(format!(
                    "Wrote {}, but it does not read back as a tag",
                    output.display()
                ));
                return;
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.error = Some(format!(
                    "Wrote {}, but could not inspect it: {error:#}",
                    output.display()
                ));
                return;
            }
        };
        self.register_created_tag(entry, tag);
        self.tag_ops.new_tag_open = false;
        self.model.status = format!("Created {}", output.display());
    }

    /// Create a brand-new Campaign Evolved tag in memory (no pak write). The tag
    /// is a defaults-initialized `TagFile::new` from the group schema, registered
    /// dirty at the dialog's container-relative path; Save / Export Mod then write
    /// it via `write_new_tag_container`.
    pub(in crate::app) fn create_new_container_tag(&mut self) {
        let Some(group) = self
            .tag_ops.new_tag_dialog
            .groups
            .get(self.tag_ops.new_tag_dialog.selected_group)
            .cloned()
        else {
            self.tag_ops.new_tag_dialog.error = Some("Choose a tag group".to_owned());
            return;
        };
        let rel = normalize_container_tag_rel(&self.tag_ops.new_tag_dialog.rel_path);
        if rel.is_empty() {
            self.tag_ops.new_tag_dialog.error = Some("Enter a tag path (e.g. objects/foo/bar)".to_owned());
            return;
        }
        let tag = match TagFile::new(&group.schema_path) {
            Ok(mut tag) => {
                // `TagFile::new` zeroes the whole file-header generation; the
                // simulation expects Campaign Evolved's.
                if let Err(error) = apply_editing_kit_mcc_header(&mut tag, GameId::CampaignEvolved.as_str()) {
                    self.tag_ops.new_tag_dialog.error = Some(error);
                    return;
                }
                tag
            }
            Err(error) => {
                self.tag_ops.new_tag_dialog.error = Some(format!("Could not create tag: {error}"));
                return;
            }
        };
        match self.add_new_container_tag(&rel, group.group_tag, &group.name, &group.extension, tag)
        {
            Ok(()) => {
                self.tag_ops.new_tag_open = false;
                self.model.status = format!("Created {rel}.{} (unsaved)", group.extension);
            }
            Err(error) => self.tag_ops.new_tag_dialog.error = Some(error),
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
            new_container_template_for(self.find_container_template(group_tag), group_name)?;
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
            self.add_new_container_tag(new_rel, group_tag, &group_name, &extension, copy)?;
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

    /// Find an existing container tag of `group_tag` and return its owning
    /// container index plus its `.uasset` container path — the package template
    /// for a new tag of the same group.
    pub(in crate::app) fn find_container_template(&self, group_tag: u32) -> Option<(usize, String)> {
        self.find_container_template_in(self.model.active, group_tag)
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
        let source = self.model.kits.get(kit)?.source.as_ref()?;
        pick_container_template(
            source.entries.iter().chain(source.all_entries.iter()),
            group_tag,
        )
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

    pub(in crate::app) fn register_saved_copy_if_in_loaded_folder(&mut self, path: &Path) -> Result<bool, String> {
        let Some(source) = self.source_mut() else {
            return Ok(false);
        };
        let registered = register_saved_copy_in_loaded_source(source, path)?;
        if registered {
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

#[cfg(test)]
mod campaign_new_tag_tests;

#[cfg(test)]
mod container_path_tests;
