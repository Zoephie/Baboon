//! Tag duplication workflows.
//! Loose files are copied synchronously on the UI thread; Campaign Evolved
//! package duplication is snapshot/worker/UI-result work because it mutates a
//! mounted IoStore container in place.

use super::*;
use crate::core::created_tags::{CreatedTagOrigin, package_id_for};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blam_tags::iostore::parse_ublock_stem;
use serde::Serialize;

const DUPLICATE_BACKUP_SUFFIX: &str = ".baboon-duplicate-backup";
const DUPLICATE_BACKUP_MANIFEST_TAIL: &str = ".manifest.json";
#[cfg(test)]
const DUPLICATE_BACKUP_MANIFEST_SUFFIX: &str = ".baboon-duplicate-backup.manifest.json";
const DUPLICATE_BACKUP_VERSION: u32 = 1;
/// How many immutable backups may pile up beside one container before Baboon
/// stops writing to it. Enough for ordinary editing; low enough that a runaway
/// loop cannot quietly fill a drive with copies of a shipped TOC.
const MAX_BACKUP_SLOTS: u32 = 32;

#[derive(Clone, Debug)]
struct ContainerDuplicatePaths {
    package: String,
    uasset: String,
    ubulk: String,
    display: String,
}

#[derive(Clone)]
struct ContainerDuplicateWorkerInput {
    root: PathBuf,
    containers: Vec<crate::core::source::MountedContainer>,
    target_container: usize,
    /// The paired `.uasset`, read on the UI thread through the path the mount
    /// recorded rather than one reassembled from the payload's name.
    wrapper_bytes: Vec<u8>,
    /// What the resolution actually settled on, carried so a failure names it.
    diagnostics: DuplicateDiagnostics,
    source_key: String,
    group_tag: u32,
    group_name: String,
    body_bytes: Vec<u8>,
    paths: ContainerDuplicatePaths,
    target_label: String,
    is_mod: bool,
    /// Chunks the target held before this write, captured on the UI thread
    /// against the same archive handle the worker validates. Recorded as the
    /// copy's provenance, and later the proof that lets it be deleted.
    entry_count_before: u32,
    /// The tag being copied, for the ledger's own record.
    source_display: String,
}

#[derive(Serialize)]
struct DuplicateBackupManifest {
    version: u32,
    original_utoc_filename: String,
    original_ucas_length: u64,
}

/// Validate one duplicate leaf and its browser-visible destination.
///
/// The same helper is used by loose and Campaign Evolved duplicate dialogs so
/// all writes share the same Windows-safe, case-insensitive naming contract.
pub(in crate::app) fn validate_duplicate_leaf_name(
    raw: &str,
    destination_display: &str,
    existing_display_paths: &[String],
) -> Result<String, String> {
    let name = validate_leaf_characters(raw, "Tag names", "Enter a new tag name")?;
    let name = name.as_str();
    let destination_key = normalized_display_path(destination_display);
    if existing_display_paths
        .iter()
        .map(|path| normalized_display_path(path))
        .any(|path| path == destination_key)
    {
        return Err("A tag with that name already exists in this source".to_owned());
    }
    Ok(name.to_owned())
}

/// The character rules every user-named leaf shares — tags and container
/// folders alike.
///
/// Extracted so the two cannot drift: a folder rejected as a tag name is a
/// folder no tag could ever be created inside, and both end up as a directory
/// node in a pak's index and as a path component on disk during extraction.
/// `noun` heads the messages (`"Tag names"`); `empty_message` is used verbatim,
/// because "enter a new tag name" and "enter a folder name" are the one place
/// the two callers genuinely differ.
pub(in crate::app) fn validate_leaf_characters(
    raw: &str,
    noun: &str,
    empty_message: &str,
) -> Result<String, String> {
    if raw.chars().any(|character| character.is_ascii_control()) {
        return Err(format!("{noun} cannot contain control characters"));
    }
    if raw.ends_with([' ', '.']) {
        return Err(format!("{noun} cannot end with a space or dot"));
    }
    let name = raw.trim();
    if name.is_empty() {
        return Err(empty_message.to_owned());
    }
    if name == "." || name == ".." {
        return Err(format!("{noun} cannot be . or .."));
    }
    if name.contains(['/', '\\']) {
        return Err("Enter a leaf name only; the parent folder is fixed".to_owned());
    }
    if name.contains('.') {
        return Err(format!("{noun} cannot contain a dot or extension"));
    }
    if name
        .chars()
        .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(format!("{noun} contain a Windows-illegal character"));
    }
    if is_windows_reserved_name(name) {
        return Err("That name is reserved by Windows".to_owned());
    }
    Ok(name.to_owned())
}

fn normalized_display_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn duplicate_display_path(source_display: &str, leaf: &str) -> String {
    let (stem, extension) = match source_display.rsplit_once('.') {
        Some((stem, extension)) => (stem, extension),
        None => (source_display, ""),
    };
    let parent = stem
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let file = if extension.is_empty() {
        leaf.to_owned()
    } else {
        format!("{leaf}.{extension}")
    };
    if parent.is_empty() {
        file
    } else {
        format!("{parent}/{file}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum NameOperationRoute {
    Rename,
    SaveAsOverlay,
    InPlaceDuplicateConfirmation,
}

pub(in crate::app) fn name_operation_route(operation: TagNameOperation) -> NameOperationRoute {
    match operation {
        TagNameOperation::Rename => NameOperationRoute::Rename,
        TagNameOperation::SaveAsOverlay => NameOperationRoute::SaveAsOverlay,
        TagNameOperation::Duplicate => NameOperationRoute::InPlaceDuplicateConfirmation,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct DuplicateDialogParts {
    pub(in crate::app) prefill: String,
    pub(in crate::app) fixed_parent: String,
    pub(in crate::app) extension: String,
}

pub(in crate::app) fn duplicate_dialog_parts(display: &str) -> DuplicateDialogParts {
    let (stem, extension) = match display.rsplit_once('.') {
        Some((stem, extension)) => (stem, extension.to_owned()),
        None => (display, String::new()),
    };
    let leaf = stem.rsplit(['/', '\\']).next().unwrap_or(stem);
    let fixed_parent = stem
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default();
    DuplicateDialogParts {
        prefill: format!("{leaf}_copy"),
        fixed_parent,
        extension,
    }
}

fn source_entries_display_paths(source: &LoadedSourceData) -> Vec<String> {
    source
        .entries
        .iter()
        .chain(source.all_entries.iter())
        .map(|entry| entry.display_path.clone())
        .collect()
}

fn exact_container_provider(entry: &TagEntry) -> Result<(usize, String), String> {
    match &entry.location {
        TagEntryLocation::Container {
            container,
            rel_path,
        } => Ok((*container, rel_path.clone())),
        _ => Err("Not a Campaign Evolved container tag".to_owned()),
    }
}

/// The `.uasset` wrapper a container tag's `.ubulk` payload belongs to, as the
/// mount recorded it.
#[derive(Clone, Debug)]
pub(in crate::app) struct ResolvedUasset {
    /// Which mounted container actually carries the wrapper.
    pub(in crate::app) container: usize,
    /// The path in its **original case**. The IoStore directory index is
    /// case-sensitive, so this string is only ever one taken from a real entry
    /// — never one this code assembled.
    pub(in crate::app) rel_path: String,
    /// How it was found, for the diagnostic.
    pub(in crate::app) how: &'static str,
}

/// Find the `.uasset` wrapper paired with a `.ubulk` payload.
///
/// Swapping the extension on the payload path and reading that is right almost
/// always and wrong in exactly the cases that matter. A container's directory
/// index is matched byte-for-byte, and the two entries do not have to agree on
/// case: a mod ships no directory index at all, so its paths are *recovered* —
/// from a base container's index when the chunk id is known there, and
/// otherwise from the Zen header's own package name, which carries whatever
/// casing the cook wrote. `objects/characters/Marine/marine-biped.ubulk` and
/// `objects/characters/marine/marine-biped.uasset` are the same package to
/// Unreal (chunk ids hash the lowercased name) and two different keys to the
/// container index — so the swapped string resolves to nothing, in every
/// container, and the copy fails with "path not found".
///
/// So ask the indexes that recorded the real paths first, and only fall back to
/// assembling one. Nothing assembled is ever handed to a read: each step
/// returns a string taken from an entry that exists.
pub(in crate::app) fn resolve_source_uasset(
    containers: &[crate::core::source::MountedContainer],
    packages: &crate::core::source::ContainerPackageIndex,
    target: usize,
    ubulk_rel_path: &str,
) -> Result<ResolvedUasset, String> {
    resolve_source_uasset_in(&MountedPaths(containers), packages, target, ubulk_rel_path)
}

/// The directory-index questions [`resolve_source_uasset`] asks of the mount.
///
/// A seam, so the resolution order can be tested against containers whose
/// `.uasset` and `.ubulk` entries deliberately disagree on case — which is the
/// whole bug, and which no container Baboon can synthesise reproduces, since
/// the only writer available emits no directory index at all.
pub(in crate::app) trait ContainerPaths {
    fn count(&self) -> usize;
    /// Whether `path` is in this container's directory index, matched exactly.
    fn contains(&self, container: usize, path: &str) -> bool;
    /// The container's own spelling of `path`, matched without case.
    fn find_ignoring_case(&self, container: usize, path: &str) -> Option<String>;
}

struct MountedPaths<'a>(&'a [crate::core::source::MountedContainer]);

impl ContainerPaths for MountedPaths<'_> {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn contains(&self, container: usize, path: &str) -> bool {
        self.0
            .get(container)
            .is_some_and(|mounted| mounted.archive.contains(path))
    }

    fn find_ignoring_case(&self, container: usize, path: &str) -> Option<String> {
        self.0
            .get(container)?
            .archive
            .entries()
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(path))
            .map(|entry| entry.path.clone())
    }
}

pub(in crate::app) fn resolve_source_uasset_in(
    containers: &dyn ContainerPaths,
    packages: &crate::core::source::ContainerPackageIndex,
    target: usize,
    ubulk_rel_path: &str,
) -> Result<ResolvedUasset, String> {
    let assembled = ubulk_rel_path
        .strip_suffix(".ubulk")
        .map(|stem| format!("{stem}.uasset"))
        .ok_or("Source container path is not a .ubulk")?;

    // 1. What indexing recorded. `ContainerPackageIndex` is keyed by the
    //    lowercased `/game/...` package name and stores the original-case
    //    container path, which is exactly the provenance this needs.
    if let Some(package) = crate::core::source::container_package_name(&assembled)
        && let Some((container, rel_path)) = packages.lookup(&package)
        && containers.contains(container, rel_path)
    {
        return Ok(ResolvedUasset {
            container,
            rel_path: rel_path.to_owned(),
            how: "package index",
        });
    }

    // 2. The exact swapped path in the container that provides the payload.
    if containers.contains(target, &assembled) {
        return Ok(ResolvedUasset {
            container: target,
            rel_path: assembled,
            how: "same container",
        });
    }

    // 3. and 4. The same path spelt differently, in the providing container
    //    first and then in the layers beneath it — an older mod that shipped a
    //    payload without its wrapper leaves the base game's copy as the only
    //    one there is.
    let search =
        std::iter::once(target).chain(lower_priority_container_indices(target, containers.count()));
    for index in search {
        if let Some(rel_path) = containers.find_ignoring_case(index, &assembled) {
            return Ok(ResolvedUasset {
                container: index,
                rel_path,
                how: if index == target {
                    "same container, different case"
                } else {
                    "lower-priority container"
                },
            });
        }
    }
    Err(format!(
        "No .uasset wrapper for {ubulk_rel_path} in any mounted container (looked for \
         {assembled}, in any case)"
    ))
}

/// Everything a duplicate resolved before it wrote anything, so a failure
/// report names what was actually used rather than what was displayed.
#[derive(Clone, Debug)]
pub(in crate::app) struct DuplicateDiagnostics {
    pub(in crate::app) display_path: String,
    pub(in crate::app) source_container: usize,
    pub(in crate::app) source_container_label: String,
    pub(in crate::app) source_utoc: PathBuf,
    pub(in crate::app) source_ubulk: String,
    pub(in crate::app) source_uasset: String,
    pub(in crate::app) source_uasset_container: String,
    pub(in crate::app) source_uasset_how: &'static str,
    pub(in crate::app) source_package: String,
    pub(in crate::app) package_basename: String,
    pub(in crate::app) destination_package: String,
    pub(in crate::app) destination_uasset: String,
    pub(in crate::app) destination_ubulk: String,
}

impl std::fmt::Display for DuplicateDiagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "duplicate: shown as {shown}\n  source container: #{index} {label} ({utoc})\n  \
             payload: {ubulk}\n  wrapper: {uasset} (in {wrapper_container}, via \
             {how})\n  package: {package} (basename {basename})\n  destination package: \
             {destination}\n  destination wrapper: {destination_uasset}\n  destination payload: \
             {destination_ubulk}",
            shown = self.display_path,
            index = self.source_container,
            label = self.source_container_label,
            utoc = self.source_utoc.display(),
            ubulk = self.source_ubulk,
            uasset = self.source_uasset,
            wrapper_container = self.source_uasset_container,
            how = self.source_uasset_how,
            package = self.source_package,
            basename = self.package_basename,
            destination = self.destination_package,
            destination_uasset = self.destination_uasset,
            destination_ubulk = self.destination_ubulk,
        )
    }
}

fn container_duplicate_paths(
    source_rel_path: &str,
    source_display: &str,
    destination_leaf: &str,
) -> Result<ContainerDuplicatePaths, String> {
    let source_file = source_rel_path
        .rsplit('/')
        .next()
        .ok_or("Source container path is empty")?;
    let (_, group_name) =
        parse_ublock_stem(source_file).ok_or("Source path is not a tagged .ubulk package")?;
    let parent = source_rel_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let stem = format!("{destination_leaf}-{group_name}");
    let ubulk = if parent.is_empty() {
        format!("{stem}.ubulk")
    } else {
        format!("{parent}/{stem}.ubulk")
    };
    let uasset = ubulk
        .strip_suffix(".ubulk")
        .map(|stem| format!("{stem}.uasset"))
        .ok_or("Destination path is not a .ubulk")?;
    let package = container_rel_to_package_path_for_duplicate(&uasset)?;
    Ok(ContainerDuplicatePaths {
        package,
        uasset,
        ubulk,
        display: duplicate_display_path(source_display, destination_leaf),
    })
}

fn container_rel_to_package_path_for_duplicate(rel: &str) -> Result<String, String> {
    let no_extension = rel
        .strip_suffix(".uasset")
        .or_else(|| rel.strip_suffix(".ubulk"))
        .ok_or("Container asset path has no supported extension")?;
    Ok(format!("/Game/{}", super::strip_content_root(no_extension)))
}

fn strip_prefix_case_insensitive<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn container_logical_path(rel_path: &str) -> Option<String> {
    let after = strip_prefix_case_insensitive(rel_path, "Meteorite/Content/Tags/")
        .or_else(|| strip_prefix_case_insensitive(rel_path, "Tags/"))
        .or_else(|| strip_prefix_case_insensitive(rel_path, "Meteorite/Content/"))?;
    let source_file = after.rsplit('/').next()?;
    let (tag_name, _group_longname) = parse_ublock_stem(source_file)?;
    let directory = after.rsplit_once('/').map(|(directory, _)| directory);
    Some(match directory {
        Some(directory) if !directory.is_empty() => format!(
            "{}/{}",
            directory.to_ascii_lowercase(),
            tag_name.to_ascii_lowercase()
        ),
        _ => tag_name.to_ascii_lowercase(),
    })
}

pub(in crate::app) fn container_duplicate_index_key(group_tag: u32, rel_path: &str) -> Option<String> {
    container_logical_path(rel_path)
        .map(|logical| crate::core::source::container_ref_key(group_tag, &logical))
}

fn select_duplicate_bytes(
    stored_bytes: &[u8],
    document: Option<&TagDocument>,
) -> Result<Vec<u8>, String> {
    if let Some(document) = document.filter(|document| document.dirty.is_set()) {
        document
            .tag
            .write_to_bytes()
            .map_err(|error| format!("Could not serialize current edits: {error}"))
    } else {
        Ok(stored_bytes.to_vec())
    }
}

fn loose_duplicate_destination(source_path: &Path, new_leaf: &str) -> Result<PathBuf, String> {
    let parent = source_path
        .parent()
        .ok_or("Source tag has no parent directory")?;
    let mut filename = std::ffi::OsString::from(new_leaf);
    if let Some(extension) = source_path.extension() {
        filename.push(".");
        filename.push(extension);
    }
    Ok(parent.join(filename))
}

fn loose_duplicate_entry(
    source: &TagSource,
    source_entry: &TagEntry,
    source_names: &TagNameIndex,
    destination: &Path,
    new_leaf: &str,
) -> Result<TagEntry, String> {
    match source {
        TagSource::LooseFolder { root, .. } => {
            crate::core::source::loose_file_entry(root, destination, source_names)
                .map_err(|error| format!("Could not register duplicate: {error:#}"))?
                .ok_or_else(|| "The copied file is not a recognized tag".to_owned())
        }
        TagSource::SingleFile { .. } => Ok(TagEntry {
            key: file_entry_key(&destination),
            display_path: duplicate_display_path(&source_entry.display_path, new_leaf),
            group_tag: source_entry.group_tag,
            group_name: source_entry.group_name.clone(),
            location: TagEntryLocation::LooseFile(destination.to_path_buf()),
        }),
        _ => Err("Loose duplicate source is no longer available".to_owned()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerDuplicateCompletion {
    Failed,
    KitClosed,
    Apply,
}

/// Route a finished duplicate on whether its workspace is still **open** —
/// deliberately not on whether the kit's generation still matches.
///
/// The bytes are already in the pak by the time this runs, and a pak rewrite
/// takes seconds; anything the user does meanwhile that bumps the generation
/// (saving another tag, revealing an entry, stashing an edit) would otherwise
/// discard a duplicate that succeeded on disk, leaving it invisible until the
/// whole source is reloaded. Provenance is re-validated against the live source
/// instead, which is the question that actually matters.
fn classify_container_duplicate_completion(
    succeeded: bool,
    kit_open: bool,
) -> ContainerDuplicateCompletion {
    if !succeeded {
        ContainerDuplicateCompletion::Failed
    } else if !kit_open {
        ContainerDuplicateCompletion::KitClosed
    } else {
        ContainerDuplicateCompletion::Apply
    }
}

/// Which mounted container currently provides `target_utoc`.
///
/// Resolved by path, not by the index the job started with. That index is only a
/// position in the mounted list and anything that remounts can reorder it, while
/// the `.utoc` a worker actually wrote to is an identity that cannot drift.
/// Checking the recorded slot first keeps the common case a single comparison.
///
/// This replaces the generation stamp as the staleness test. Refusing to
/// register a copy that is already in the pak does not undo anything — it just
/// hides the tag until the whole source is reloaded — so the question worth
/// asking is "where is that container now", not "has anything changed".
pub(in crate::app) fn container_index_for_utoc(
    source: Option<&LoadedSourceData>,
    recorded_index: usize,
    target_utoc: &Path,
) -> Option<usize> {
    let TagSource::IoStoreContainerSet { containers, .. } = &source?.source else {
        return None;
    };
    if containers
        .get(recorded_index)
        .is_some_and(|target| target.utoc_path == target_utoc)
    {
        return Some(recorded_index);
    }
    containers
        .iter()
        .position(|target| target.utoc_path == target_utoc)
}

fn clear_container_duplicate_running(running: &mut HashSet<KitId>, kit: KitId) {
    running.remove(&kit);
}

fn apply_container_duplicate_source_state(
    source: &mut LoadedSourceData,
    target_container: usize,
    group_tag: u32,
    package: &str,
    uasset_path: &str,
    ubulk_path: &str,
    is_mod: bool,
    entry: &TagEntry,
    tag: &TagFile,
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
            return Err("Duplicate completed against a non-container source".to_owned());
        };
        let index_key = container_duplicate_index_key(group_tag, ubulk_path)
            .ok_or("Duplicate completed with an invalid container destination path")?;
        Arc::make_mut(index).insert(index_key, target_container, ubulk_path.to_owned());
        Arc::make_mut(packages).insert(
            package.to_ascii_lowercase(),
            target_container,
            uasset_path.to_owned(),
        );
        if !is_mod {
            Arc::make_mut(shipped).insert(ubulk_path, target_container);
        }
    }

    let key = entry.key.clone();
    // Sorted, not pushed (upsert_entry keeps a container's list in
    // `natural_key` order): the browser draws a folder in entry-vector order,
    // so a pushed copy would land at the bottom of its folder instead of
    // beside the tag it was duplicated from.
    source.upsert_entry(entry.clone(), pending_folders);
    if let Some(reverse) = source.reverse_dependencies.as_mut() {
        let mut dependencies = Vec::new();
        collect_tag_dependency_refs(tag.root(), &mut dependencies);
        reverse.set_tag_dependencies(key, dependencies);
    }
    Ok(())
}

fn register_clean_duplicate_document(kit: &mut Kit, entry: TagEntry, tag: TagFile) {
    let key = entry.key.clone();
    kit.parsed_tags.insert(key.clone(), TagDocument::clean(tag));
    kit.open_tag_pane(&key);
    kit.selected_key = Some(key);
}

fn lower_priority_container_indices(target: usize, count: usize) -> impl Iterator<Item = usize> {
    (0..target.min(count)).rev()
}

/// Read the effective wrapper without changing the provider of the tag body.
///
/// The path comes from [`resolve_source_uasset`], which asks the mount's own
/// indexes rather than assembling one — so the read is against a path that
/// exists, in the case the container spells it.
fn read_effective_wrapper(
    containers: &[crate::core::source::MountedContainer],
    resolved: &ResolvedUasset,
) -> Result<Vec<u8>, String> {
    containers
        .get(resolved.container)
        .ok_or_else(|| "Container provenance is stale".to_owned())?
        .archive
        .read(&resolved.rel_path)
        .map_err(|error| {
            format!(
                "Could not read the paired asset {} : {error}",
                resolved.rel_path
            )
        })
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut created = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        created = true;
        file.write_all(bytes)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("Could not sync {}: {error}", path.display()))?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(path);
    }
    result
}

fn backup_sibling_path(utoc: &Path, suffix: &str) -> Result<PathBuf, String> {
    let parent = utoc.parent().ok_or("Target UTOC has no parent directory")?;
    let filename = utoc
        .file_name()
        .ok_or("Target UTOC has no filename")?
        .to_string_lossy();
    Ok(parent.join(format!("{filename}{suffix}")))
}

fn reset_readonly_and_remove(path: &Path) {
    if let Ok(mut permissions) = fs::metadata(path).map(|metadata| metadata.permissions()) {
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
    let _ = fs::remove_file(path);
}

/// The first backup slot beside `utoc` that is free.
///
/// Backups are immutable once written — an earlier one is the only record of a
/// state the container can still be walked back to, so it is never overwritten.
/// A container can be written to more than once (duplicate, delete, duplicate
/// again), so each write takes the next free slot rather than failing because
/// the first is taken.
fn next_free_backup_slot(utoc: &Path) -> Result<(PathBuf, PathBuf), String> {
    for attempt in 0..MAX_BACKUP_SLOTS {
        let ordinal = match attempt {
            0 => String::new(),
            _ => format!("-{attempt}"),
        };
        let backup = backup_sibling_path(utoc, &format!("{DUPLICATE_BACKUP_SUFFIX}{ordinal}"))?;
        let manifest = backup_sibling_path(
            utoc,
            &format!("{DUPLICATE_BACKUP_SUFFIX}{ordinal}{DUPLICATE_BACKUP_MANIFEST_TAIL}"),
        )?;
        if !backup.exists() && !manifest.exists() {
            return Ok((backup, manifest));
        }
    }
    Err(format!(
        "{MAX_BACKUP_SLOTS} backups already exist beside {}; move or delete some before writing \
         to this container again",
        utoc.display()
    ))
}

/// Create the immutable sibling backup immediately before in-place mutation.
/// Existing backups are never removed or overwritten.
pub(in crate::app) fn create_duplicate_backup(utoc: &Path) -> Result<DuplicateBackupPaths, String> {
    let original_utoc = fs::read(utoc)
        .map_err(|error| format!("Could not read original UTOC {}: {error}", utoc.display()))?;
    let ucas = utoc.with_extension("ucas");
    let original_ucas_length = fs::metadata(&ucas)
        .map_err(|error| {
            format!(
                "Could not inspect original UCAS {}: {error}",
                ucas.display()
            )
        })?
        .len();
    let original_utoc_filename = utoc
        .file_name()
        .ok_or("Target UTOC has no filename")?
        .to_string_lossy()
        .into_owned();
    let manifest = serde_json::to_vec(&DuplicateBackupManifest {
        version: DUPLICATE_BACKUP_VERSION,
        original_utoc_filename,
        original_ucas_length,
    })
    .map_err(|error| format!("Could not encode duplicate backup manifest: {error}"))?;
    let (backup, manifest_path) = next_free_backup_slot(utoc)?;
    let mut created = Vec::new();
    let result = (|| {
        write_create_new(&backup, &original_utoc)?;
        created.push(backup.clone());
        write_create_new(&manifest_path, &manifest)?;
        created.push(manifest_path.clone());
        for path in [&backup, &manifest_path] {
            let mut permissions = fs::metadata(path)
                .map_err(|error| format!("Could not inspect backup {}: {error}", path.display()))?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).map_err(|error| {
                format!(
                    "Could not make backup read-only {}: {error}",
                    path.display()
                )
            })?;
        }
        Ok(DuplicateBackupPaths {
            utoc: backup.clone(),
            manifest: manifest_path.clone(),
        })
    })();
    if result.is_err() {
        for path in created.into_iter().rev() {
            reset_readonly_and_remove(&path);
        }
    }
    result
}

pub(in crate::app) fn backup_paths_text(backup: &DuplicateBackupPaths) -> String {
    format!(
        "{} (manifest {})",
        backup.utoc.display(),
        backup.manifest.display()
    )
}

fn parse_duplicate_body(
    bytes: &[u8],
    source: &TagSource,
    entry: &TagEntry,
) -> Result<TagFile, String> {
    match source {
        TagSource::LooseFolder {
            game,
            definitions_root,
            ..
        } => crate::core::source::read_tag_from_bytes(
            bytes,
            *game,
            Some(definitions_root),
            entry.group_tag,
        )
        .map_err(|error| format!("Could not parse duplicate bytes: {error:#}")),
        _ => crate::core::source::read_tag_from_bytes(bytes, None, None, entry.group_tag)
            .map_err(|error| format!("Could not parse duplicate bytes: {error:#}")),
    }
}

impl Baboon {
    pub(in crate::app) fn begin_duplicate_tag(&mut self) {
        if self.refuse_read_only_edit(self.active) {
            return;
        }
        let Some(state) = self.tag_ops.rename_tag.as_ref() else {
            return;
        };
        let key = state.key.clone();
        let raw_name = state.new_path_input.clone();
        let old_display = state.old_display.clone();
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let destination_display = duplicate_display_path(&old_display, raw_name.trim());
        let existing = self
            .source()
            .map(source_entries_display_paths)
            .unwrap_or_default();
        let new_leaf =
            match validate_duplicate_leaf_name(&raw_name, &destination_display, &existing) {
                Ok(name) => name,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
        match entry.location {
            TagEntryLocation::LooseFile(_) => {
                self.tag_ops.rename_tag = None;
                match self.duplicate_loose_tag(&entry, &new_leaf) {
                    Ok(()) => {}
                    Err(error) => self.status = error,
                }
            }
            TagEntryLocation::Container { .. } => {
                self.tag_ops.rename_tag = None;
                self.tag_ops.container_duplicate_confirm = Some(ContainerDuplicateConfirm {
                    kit: self.active_kit_id(),
                    key,
                    destination_leaf: new_leaf,
                });
            }
            TagEntryLocation::Monolithic { .. } | TagEntryLocation::NewContainer { .. } => {
                self.status =
                    "Only loose-file and Campaign Evolved container tags can be duplicated"
                        .to_owned();
            }
        }
    }

    fn duplicate_loose_tag(&mut self, entry: &TagEntry, new_leaf: &str) -> Result<(), String> {
        let TagEntryLocation::LooseFile(source_path) = &entry.location else {
            return Err("Only loose-file tags can use the loose duplicate path".to_owned());
        };
        let destination = loose_duplicate_destination(source_path, new_leaf)?;
        let (source_kind, source_names) = {
            let source = self.source().ok_or("No tag source is loaded")?;
            (source.source.clone(), source.names.clone())
        };
        let is_dirty = self.kits[self.active]
            .parsed_tags
            .get(&entry.key)
            .is_some_and(|document| document.dirty.is_set());
        let stored_bytes = if is_dirty {
            Vec::new()
        } else {
            fs::read(source_path)
                .map_err(|error| format!("Could not read {}: {error}", source_path.display()))?
        };
        let bytes = select_duplicate_bytes(
            &stored_bytes,
            self.kits[self.active].parsed_tags.get(&entry.key),
        )?;
        write_create_new(&destination, &bytes)?;
        let parsed = match parse_duplicate_body(&bytes, &source_kind, entry) {
            Ok(tag) => tag,
            Err(error) => {
                reset_readonly_and_remove(&destination);
                return Err(error);
            }
        };
        let duplicate_entry =
            match loose_duplicate_entry(&source_kind, entry, &source_names, &destination, new_leaf)
            {
                Ok(entry) => entry,
                Err(error) => {
                    reset_readonly_and_remove(&destination);
                    return Err(error);
                }
            };
        let duplicate_key = duplicate_entry.key.clone();
        self.register_created_tag(duplicate_entry, parsed);
        // Expand and scroll to the copy so it is visible beside the tag it came
        // from, rather than only selected somewhere in a collapsed tree.
        self.reveal_in_browser(&duplicate_key);
        self.status = format!(
            "Duplicated {} → {}",
            entry.display_path,
            destination.display()
        );
        Ok(())
    }

    pub(in crate::app) fn start_container_duplicate(
        &mut self,
        kit: KitId,
        key: String,
        destination_leaf: String,
        ctx: egui::Context,
    ) {
        if !self.focus_navigation_kit(kit) {
            self.status = "The workspace this duplicate came from is closed".to_owned();
            return;
        }
        if self.tag_ops.container_delete_running.contains(&kit) {
            self.status =
                "A Campaign Evolved delete is already running for this workspace".to_owned();
            return;
        }
        if self.tag_ops.container_duplicate_running.contains(&kit) {
            self.status =
                "A Campaign Evolved duplicate is already running for this workspace".to_owned();
            return;
        }
        let Some(entry) = self.entry_for_key(&key).cloned() else {
            self.status = "Tag is no longer in the source".to_owned();
            return;
        };
        let (target_container, source_rel_path) = match exact_container_provider(&entry) {
            Ok(provider) => provider,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let paths = match container_duplicate_paths(
            &source_rel_path,
            &entry.display_path,
            &destination_leaf,
        ) {
            Ok(paths) => paths,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let existing = self
            .source()
            .map(source_entries_display_paths)
            .unwrap_or_default();
        if let Err(error) =
            validate_duplicate_leaf_name(&destination_leaf, &paths.display, &existing)
        {
            self.status = error;
            return;
        }
        // Everything the write needs is read here, on the UI thread, against
        // the mount as it stands. Resolving the wrapper before the worker
        // starts is what lets a failure be reported with the paths that were
        // actually used rather than the ones that were displayed.
        let (
            root,
            containers,
            target_utoc,
            target_label,
            is_mod,
            entry_count_before,
            body_bytes,
            wrapper_bytes,
            diagnostics,
        ) = {
            let Some(source) = self.source() else {
                self.status = "No source is loaded".to_owned();
                return;
            };
            let TagSource::IoStoreContainerSet {
                root,
                containers,
                packages,
                ..
            } = &source.source
            else {
                self.status = "Source is not a Campaign Evolved container source".to_owned();
                return;
            };
            let Some(target) = containers.get(target_container) else {
                self.status = "Container provenance is stale".to_owned();
                return;
            };
            let resolved = match resolve_source_uasset(
                containers,
                packages,
                target_container,
                &source_rel_path,
            ) {
                Ok(resolved) => resolved,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
            let wrapper = match read_effective_wrapper(containers, &resolved) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
            let is_dirty = self.kits[self.active]
                .parsed_tags
                .get(&key)
                .is_some_and(|document| document.dirty.is_set());
            let stored_bytes = if is_dirty {
                Vec::new()
            } else {
                match target.archive.read(&source_rel_path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        self.status = format!(
                            "Could not read {} from {}: {error}",
                            source_rel_path, target.chunk_label
                        );
                        return;
                    }
                }
            };
            let body = match select_duplicate_bytes(
                &stored_bytes,
                self.kits[self.active].parsed_tags.get(&key),
            ) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
            let diagnostics = DuplicateDiagnostics {
                display_path: entry.display_path.clone(),
                source_container: target_container,
                source_container_label: target.chunk_label.clone(),
                source_utoc: target.utoc_path.clone(),
                source_ubulk: source_rel_path.clone(),
                source_uasset: resolved.rel_path.clone(),
                source_uasset_container: containers
                    .get(resolved.container)
                    .map(|mounted| mounted.chunk_label.clone())
                    .unwrap_or_else(|| "unknown".to_owned()),
                source_uasset_how: resolved.how,
                source_package: crate::core::source::container_package_name(&resolved.rel_path)
                    .unwrap_or_else(|| "unknown".to_owned()),
                package_basename: resolved
                    .rel_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&resolved.rel_path)
                    .to_owned(),
                destination_package: paths.package.clone(),
                destination_uasset: paths.uasset.clone(),
                destination_ubulk: paths.ubulk.clone(),
            };
            (
                root.clone(),
                containers.clone(),
                target.utoc_path.clone(),
                target.chunk_label.clone(),
                target.is_mod,
                target.archive.chunk_count(),
                body,
                wrapper,
                diagnostics,
            )
        };
        eprintln!("{diagnostics}");
        // The in-place writer appends to the `.ucas` and swaps the `.utoc` by
        // rename, neither of which needs a mapping released — and it reads
        // chunks through the mapping while it works, so releasing would break
        // it. The lease is still taken: it refuses a second write to the same
        // container, and it remounts the Unreal package workspace afterwards,
        // whose parsed copy of the TOC the swap makes stale.
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
        self.tag_ops.container_duplicate_running.insert(kit);
        self.status = format!("Duplicating {} in {}…", entry.display_path, target_label);
        let input = ContainerDuplicateWorkerInput {
            root,
            containers,
            target_container,
            wrapper_bytes,
            diagnostics,
            source_key: key,
            group_tag: entry.group_tag,
            group_name: entry
                .group_name
                .clone()
                .unwrap_or_else(|| format_group_tag(entry.group_tag)),
            body_bytes,
            paths,
            target_label,
            is_mod,
            entry_count_before,
            source_display: entry.display_path.clone(),
        };
        // Through spawn_worker so the lease always comes back: a panicking
        // write used to send nothing, leaving the container leased for good.
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::ContainerDuplicateFinished {
                stamp,
                lease: lease_id,
                result: run_container_duplicate(input),
            },
            move |error| WorkerMessage::ContainerDuplicateFinished {
                stamp,
                lease: lease_id,
                result: Err(error),
            },
        );
    }

    pub(in crate::app) fn handle_container_duplicate_finished(
        &mut self,
        stamp: KitStamp,
        lease: ContainerLeaseId,
        result: Result<ContainerDuplicateResult, String>,
        ctx: &egui::Context,
    ) -> bool {
        // Taken and settled before anything else can return early. A write that
        // landed changed the container's `.utoc`, so the Unreal package
        // workspace's parsed copy of it is stale either way.
        if let Some(lease) = self.take_container_write_lease(lease) {
            let outcome = if result.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            };
            self.release_container_write_lease(lease, outcome, ctx);
        }
        let kit_index = self.kit_index(stamp.kit);
        let completion =
            classify_container_duplicate_completion(result.is_ok(), kit_index.is_some());
        clear_container_duplicate_running(&mut self.tag_ops.container_duplicate_running, stamp.kit);
        if completion == ContainerDuplicateCompletion::Failed {
            if let Err(error) = &result {
                self.status = error.clone();
                self.operation_notice = Some(OperationNotice {
                    title: "Duplicate failed".to_owned(),
                    message: error.clone(),
                    failed: true,
                });
            }
            return false;
        }
        let Some(kit_index) = kit_index else {
            // The workspace closed while the pak was being rewritten. The copy
            // is on disk and will be there on the next load; there is no
            // workspace left to show it in, and no status bar that belongs to it.
            return true;
        };
        let result = match result {
            Ok(result) => result,
            Err(_) => unreachable!("failed duplicate was handled above"),
        };
        // Where the container this was written to sits *now*. The tag is in the
        // pak either way, so the only thing that can genuinely stop it being
        // registered is the workspace no longer holding that container at all.
        let Some(target_container) = container_index_for_utoc(
            self.kits[kit_index].source.as_ref(),
            result.target_container,
            &result.target_utoc,
        ) else {
            self.status = format!(
                "Duplicated into {}, but this workspace no longer has that container mounted — \
                 reload the source to see it. Backup: {}",
                result.target_label,
                backup_paths_text(&result.backup)
            );
            return false;
        };
        let mut result = result;
        result.target_container = target_container;
        // The entry addresses its provider positionally, so it has to be
        // corrected alongside the result it was built from.
        if let TagEntryLocation::Container { container, .. } = &mut result.entry.location {
            *container = target_container;
        }
        let display_for_notice = result.entry.display_path.clone();
        // Read before the source borrow: the tree rebuild below has to re-apply
        // the workspace's pending folders, and `source` holds `kits[kit_index]`.
        let folder_seeds = self.kits[kit_index].folder_seeds();
        {
            let Some(source) = self.kits[kit_index].source.as_mut() else {
                self.status = "Duplicate completed after its source was unloaded".to_owned();
                return false;
            };
            let TagSource::IoStoreContainerSet { containers, .. } = &mut source.source else {
                self.status = "Duplicate completed against a non-container source".to_owned();
                return false;
            };
            let Some(target) = containers.get_mut(result.target_container) else {
                self.status = "Duplicate completed with stale container provenance".to_owned();
                return false;
            };
            target.archive = result.archive;
            if let Err(error) = apply_container_duplicate_source_state(
                source,
                result.target_container,
                result.entry.group_tag,
                &result.package,
                &result.uasset_path,
                &result.ubulk_path,
                result.is_mod,
                &result.entry,
                &result.tag,
                &folder_seeds,
            ) {
                self.status = error;
                return false;
            }
        }
        // Recorded before anything else user-visible: this is the only evidence
        // that Baboon authored the copy, and without it the tag can never be
        // deleted again — nor recognised by an export as new content rather
        // than as an edit to whatever it was copied from.
        self.tag_ops.created_tags.record(result.record);
        let ledger_error = self.tag_ops.created_tags.save().err();
        let entry = result.entry;
        let key = entry.key.clone();
        // Stashed straight away, so the copy is in the next Export Mod whether
        // or not anyone edits it. The document stays clean: the bytes are
        // already in the container. A tag that will not re-serialize is simply
        // not stashed — the copy itself is fine, and it is stashed again the
        // moment it is edited.
        if let Ok(bytes) = result.tag.write_to_bytes() {
            self.stash_authored_tag(kit_index, &entry, result.package.clone(), bytes, 0.0);
        }
        self.kits[kit_index].generation = self.kits[kit_index].generation.wrapping_add(1);
        // The field-value index is keyed by entry, so it has to be rebuilt
        // before the next search can see the copy.
        self.kits[kit_index].field_index.invalidate();
        register_clean_duplicate_document(&mut self.kits[kit_index], entry, result.tag);
        // Expand and scroll to the copy, but only when its workspace is the one
        // on screen: revealing forces Folders mode and clears the filter, which
        // has no business happening in a workspace the user moved away from.
        if self.active == kit_index {
            self.reveal_in_browser(&key);
        }
        // A review left open while this ran is now describing a stash that has
        // one more tag in it than it is showing.
        self.refresh_open_mod_review(kit_index);
        self.operation_notice = Some(OperationNotice {
            title: "Tag duplicated".to_owned(),
            message: format!(
                "{} → {}\n\nWritten into {}.\nThe UTOC and UCAS changed; the sibling PAK did \
                 not.\nBackup: {}",
                result.source_key,
                display_for_notice,
                result.target_label,
                backup_paths_text(&result.backup)
            ),
            failed: false,
        });
        self.status = match ledger_error {
            // The copy exists and works; only the record of who made it failed
            // to persist, which costs the user the ability to delete it later.
            Some(error) => format!(
                "Duplicated into {} (UTOC/UCAS changed; PAK unchanged), but the duplicate \
                 record could not be saved ({error}) — this copy cannot be deleted from \
                 Baboon. Backup: {}",
                result.target_label,
                backup_paths_text(&result.backup)
            ),
            None => format!(
                "Duplicated into {} (UTOC/UCAS changed; PAK unchanged). Backup: {}",
                result.target_label,
                backup_paths_text(&result.backup)
            ),
        };
        false
    }
}

fn run_container_duplicate(
    input: ContainerDuplicateWorkerInput,
) -> Result<ContainerDuplicateResult, String> {
    let wrapper = &input.wrapper_bytes;
    TagFile::read_from_bytes(&input.body_bytes)
        .map_err(|error| format!("Could not parse duplicate body before mutation: {error}"))?;
    let target = input
        .containers
        .get(input.target_container)
        .ok_or("Container provenance is stale")?;
    let backup = create_duplicate_backup(&target.utoc_path)?;
    let archive = target.archive.clone();
    let request = blam_tags::iostore::writer::InPlaceTagDuplicate {
        source_uasset: &wrapper,
        tag_bytes: &input.body_bytes,
        destination_package_path: &input.paths.package,
        destination_uasset_path: &input.paths.uasset,
        destination_ubulk_path: &input.paths.ubulk,
    };
    if let Err(error) = blam_tags::iostore::writer::duplicate_tag_in_place_with(
        &archive,
        &target.utoc_path,
        &request,
    ) {
        // The resolved paths ride along: a report that says only "path not
        // found" leaves nobody able to tell which path was looked for or where.
        return Err(format!(
            "Duplicate into {} failed: {error}. Backup kept at {}\n\n{}",
            input.target_label,
            backup_paths_text(&backup),
            input.diagnostics
        ));
    }
    let reopened = crate::core::source::reopen_container_archive(
        &input.root,
        &input.containers,
        input.target_container,
    )
    .map_err(|error| {
        format!(
            "Duplicate wrote {}, but reopening failed: {error}. Backup kept at {}",
            input.target_label,
            backup_paths_text(&backup)
        )
    })?;
    let new_body = reopened.read(&input.paths.ubulk).map_err(|error| {
        format!(
            "Duplicate wrote {}, but the new body could not be read: {error}. Backup kept at {}",
            input.target_label,
            backup_paths_text(&backup)
        )
    })?;
    let tag = TagFile::read_from_bytes(&new_body).map_err(|error| {
        format!(
            "Duplicate wrote {}, but the new body could not be parsed: {error}. Backup kept at {}",
            input.target_label,
            backup_paths_text(&backup)
        )
    })?;
    let chunk_label = target.chunk_label.clone();
    let entry = TagEntry {
        key: crate::core::source::container_entry_key(&chunk_label, &input.paths.ubulk),
        display_path: input.paths.display.clone(),
        group_tag: input.group_tag,
        group_name: Some(input.group_name.clone()),
        location: TagEntryLocation::Container {
            container: input.target_container,
            rel_path: input.paths.ubulk.clone(),
        },
    };
    let record = CreatedTagRecord {
        utoc_path: target.utoc_path.display().to_string(),
        chunk_label: chunk_label.clone(),
        package_id: package_id_for(&input.paths.package),
        package_path: input.paths.package.clone(),
        uasset_path: input.paths.uasset.clone(),
        ubulk_path: input.paths.ubulk.clone(),
        display_path: input.paths.display.clone(),
        group_tag: input.group_tag,
        source_display: input.source_display,
        container_entry_count_before: input.entry_count_before,
        // A duplicate is content Baboon added, so deleting it takes away only
        // what Baboon put there.
        origin: CreatedTagOrigin::Authored,
        created_unix_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default(),
    };
    Ok(ContainerDuplicateResult {
        source_key: input.source_key,
        target_container: input.target_container,
        target_utoc: target.utoc_path.clone(),
        archive: Arc::new(reopened),
        entry,
        tag,
        package: input.paths.package,
        uasset_path: input.paths.uasset,
        ubulk_path: input.paths.ubulk,
        target_label: input.target_label,
        is_mod: input.is_mod,
        backup,
        record,
    })
}

#[cfg(test)]
mod provenance_tests;

#[cfg(test)]
mod tests;
