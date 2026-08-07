//! Tag duplication workflows.
//! Loose files are copied synchronously on the UI thread; Campaign Evolved
//! package duplication is snapshot/worker/UI-result work because it mutates a
//! mounted IoStore container in place.

use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use blam_tags::iostore::parse_ublock_stem;
use serde::Serialize;

const DUPLICATE_BACKUP_SUFFIX: &str = ".baboon-duplicate-backup";
const DUPLICATE_BACKUP_MANIFEST_TAIL: &str = ".manifest.json";
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
    containers: Vec<crate::source::MountedContainer>,
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
pub(super) fn validate_duplicate_leaf_name(
    raw: &str,
    destination_display: &str,
    existing_display_paths: &[String],
) -> Result<String, String> {
    if raw.chars().any(|character| character.is_ascii_control()) {
        return Err("Tag names cannot contain control characters".to_owned());
    }
    if raw.ends_with([' ', '.']) {
        return Err("Tag names cannot end with a space or dot".to_owned());
    }
    let name = raw.trim();
    if name.is_empty() {
        return Err("Enter a new tag name".to_owned());
    }
    if name == "." || name == ".." {
        return Err("Tag names cannot be . or ..".to_owned());
    }
    if name.contains(['/', '\\']) {
        return Err("Enter a leaf name only; the parent folder is fixed".to_owned());
    }
    if name.contains('.') {
        return Err("Tag names cannot contain a dot or extension".to_owned());
    }
    if name.chars().any(|character| character.is_ascii_control()) {
        return Err("Tag names cannot contain control characters".to_owned());
    }
    if name
        .chars()
        .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("Tag names contain a Windows-illegal character".to_owned());
    }
    if is_windows_reserved_leaf(name) {
        return Err("That name is reserved by Windows".to_owned());
    }
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

fn is_windows_reserved_leaf(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
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
pub(super) enum NameOperationRoute {
    Rename,
    SaveAsOverlay,
    InPlaceDuplicateConfirmation,
}

pub(super) fn name_operation_route(operation: TagNameOperation) -> NameOperationRoute {
    match operation {
        TagNameOperation::Rename => NameOperationRoute::Rename,
        TagNameOperation::SaveAsOverlay => NameOperationRoute::SaveAsOverlay,
        TagNameOperation::Duplicate => NameOperationRoute::InPlaceDuplicateConfirmation,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DuplicateDialogParts {
    pub(super) prefill: String,
    pub(super) fixed_parent: String,
    pub(super) extension: String,
}

pub(super) fn duplicate_dialog_parts(display: &str) -> DuplicateDialogParts {
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
pub(super) struct ResolvedUasset {
    /// Which mounted container actually carries the wrapper.
    pub(super) container: usize,
    /// The path in its **original case**. The IoStore directory index is
    /// case-sensitive, so this string is only ever one taken from a real entry
    /// — never one this code assembled.
    pub(super) rel_path: String,
    /// How it was found, for the diagnostic.
    pub(super) how: &'static str,
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
pub(super) fn resolve_source_uasset(
    containers: &[crate::source::MountedContainer],
    packages: &crate::source::ContainerPackageIndex,
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
pub(super) trait ContainerPaths {
    fn count(&self) -> usize;
    /// Whether `path` is in this container's directory index, matched exactly.
    fn contains(&self, container: usize, path: &str) -> bool;
    /// The container's own spelling of `path`, matched without case.
    fn find_ignoring_case(&self, container: usize, path: &str) -> Option<String>;
}

struct MountedPaths<'a>(&'a [crate::source::MountedContainer]);

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

pub(super) fn resolve_source_uasset_in(
    containers: &dyn ContainerPaths,
    packages: &crate::source::ContainerPackageIndex,
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
    if let Some(package) = crate::source::container_package_name(&assembled)
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
pub(super) struct DuplicateDiagnostics {
    pub(super) display_path: String,
    pub(super) source_container: usize,
    pub(super) source_container_label: String,
    pub(super) source_utoc: PathBuf,
    pub(super) source_ubulk: String,
    pub(super) source_uasset: String,
    pub(super) source_uasset_container: String,
    pub(super) source_uasset_how: &'static str,
    pub(super) source_package: String,
    pub(super) package_basename: String,
    pub(super) destination_package: String,
    pub(super) destination_uasset: String,
    pub(super) destination_ubulk: String,
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

pub(super) fn container_duplicate_index_key(group_tag: u32, rel_path: &str) -> Option<String> {
    container_logical_path(rel_path)
        .map(|logical| crate::source::container_ref_key(group_tag, &logical))
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
            crate::source::loose_file_entry(root, destination, source_names)
                .map_err(|error| format!("Could not register duplicate: {error:#}"))?
                .ok_or_else(|| "The copied file is not a recognized tag".to_owned())
        }
        TagSource::SingleFile { .. } => Ok(TagEntry {
            key: format!("file:{}", destination.display()),
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
pub(super) fn container_index_for_utoc(
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
    source.entries.retain(|existing| existing.key != key);
    source.all_entries.retain(|existing| existing.key != key);
    // Sorted, not pushed: the mount sorts by `natural_key` and the browser draws
    // a folder in entry-vector order, so a pushed copy lands at the bottom of
    // its folder instead of beside the tag it was duplicated from.
    crate::source::insert_entry_sorted(&mut source.entries, entry.clone());
    if !source.all_entries.is_empty() {
        crate::source::insert_entry_sorted(&mut source.all_entries, entry.clone());
    }
    source.tree = crate::source::build_tree(&source.entries);
    source.group_tree = crate::source::build_group_tree(if source.all_entries.is_empty() {
        &source.entries
    } else {
        &source.all_entries
    });
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
    containers: &[crate::source::MountedContainer],
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
pub(super) fn create_duplicate_backup(utoc: &Path) -> Result<DuplicateBackupPaths, String> {
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

pub(super) fn backup_paths_text(backup: &DuplicateBackupPaths) -> String {
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
        } => crate::source::read_tag_from_bytes(
            bytes,
            game.as_deref(),
            Some(definitions_root),
            entry.group_tag,
        )
        .map_err(|error| format!("Could not parse duplicate bytes: {error:#}")),
        _ => crate::source::read_tag_from_bytes(bytes, None, None, entry.group_tag)
            .map_err(|error| format!("Could not parse duplicate bytes: {error:#}")),
    }
}

impl Baboon {
    pub(super) fn begin_duplicate_tag(&mut self) {
        let Some(state) = self.rename_tag.as_ref() else {
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
                self.rename_tag = None;
                match self.duplicate_loose_tag(&entry, &new_leaf) {
                    Ok(()) => {}
                    Err(error) => self.status = error,
                }
            }
            TagEntryLocation::Container { .. } => {
                self.rename_tag = None;
                self.container_duplicate_confirm = Some(ContainerDuplicateConfirm {
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
        if self.container_delete_running.contains(&kit) {
            self.status =
                "A Campaign Evolved delete is already running for this workspace".to_owned();
            return;
        }
        if self.container_duplicate_running.contains(&kit) {
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
                source_package: crate::source::container_package_name(&resolved.rel_path)
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
        self.container_duplicate_running.insert(kit);
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
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = run_container_duplicate(input);
            let _ = tx.send(WorkerMessage::ContainerDuplicateFinished {
                stamp,
                lease: lease_id,
                result,
            });
            ctx.request_repaint();
        });
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
        clear_container_duplicate_running(&mut self.container_duplicate_running, stamp.kit);
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
            ) {
                self.status = error;
                return false;
            }
        }
        // Recorded before anything else user-visible: this is the only evidence
        // that Baboon authored the copy, and without it the tag can never be
        // deleted again — nor recognised by an export as new content rather
        // than as an edit to whatever it was copied from.
        self.created_tags.record(result.record);
        let ledger_error = self.created_tags.save().err();
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
    let reopened = crate::source::reopen_container_archive(
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
        key: format!("ublock:{chunk_label}:{}", input.paths.ubulk),
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
        package_id: super::package_id_for(&input.paths.package),
        package_path: input.paths.package.clone(),
        uasset_path: input.paths.uasset.clone(),
        ubulk_path: input.paths.ubulk.clone(),
        display_path: input.paths.display.clone(),
        group_tag: input.group_tag,
        source_display: input.source_display,
        container_entry_count_before: input.entry_count_before,
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
mod provenance_tests {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_fixture(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "baboon-duplicate-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn test_tag() -> TagFile {
        TagFile::new(crate::app::test_definition_path(
            "halo4_mcc/camera_track.json",
        ))
        .unwrap()
    }

    fn test_entry(key: &str, display_path: &str, group_tag: u32) -> TagEntry {
        TagEntry {
            key: key.to_owned(),
            display_path: display_path.to_owned(),
            group_tag,
            group_name: Some("camera_track".to_owned()),
            location: TagEntryLocation::Container {
                container: 0,
                rel_path: format!("Meteorite/Content/Tags/{display_path}.ubulk"),
            },
        }
    }

    fn container_source(entries: Vec<TagEntry>) -> LoadedSourceData {
        let tree = crate::source::build_tree(&entries);
        let group_tree = crate::source::build_group_tree(&entries);
        LoadedSourceData {
            label: "duplicate test containers".to_owned(),
            source: TagSource::IoStoreContainerSet {
                root: PathBuf::from("C:/duplicate-test/Paks"),
                containers: Vec::new(),
                index: Arc::new(crate::source::ContainerTagIndex::default()),
                packages: Arc::new(crate::source::ContainerPackageIndex::default()),
                shipped: Arc::new(crate::source::ShippedTagIndex::default()),
            },
            names: TagNameIndex::default(),
            game: None,
            entries,
            tree,
            group_tree,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        }
    }

    #[test]
    fn duplicate_name_validation_rejects_invalid_and_reserved_leaves() {
        let existing = vec!["objects/source.model".to_owned()];
        for invalid in [
            "", " ", ".", "..", "foo.bar", r"foo\bar", "foo/bar", "foo<bar", "foo*bar", "foo ",
            "foo\t", "CON", "com1", "LPT9",
        ] {
            assert!(
                validate_duplicate_leaf_name(invalid, "objects/new.model", &existing).is_err(),
                "{invalid:?} should be rejected"
            );
        }
        assert_eq!(
            validate_duplicate_leaf_name(" new", "objects/new.model", &existing).unwrap(),
            "new"
        );
    }

    #[test]
    fn duplicate_name_validation_is_case_insensitive_and_includes_source() {
        let existing = vec![
            "Objects/Source.model".to_owned(),
            "objects/existing.model".to_owned(),
        ];
        assert!(
            validate_duplicate_leaf_name("existing", "OBJECTS/EXISTING.model", &existing).is_err()
        );
        assert!(validate_duplicate_leaf_name("Source", "objects/Source.model", &existing).is_err());
    }

    #[test]
    fn duplicate_dialog_prefills_copy_and_keeps_parent_and_extension_fixed() {
        assert_eq!(
            duplicate_dialog_parts("Objects/Old-Biped.BIPED"),
            DuplicateDialogParts {
                prefill: "Old-Biped_copy".to_owned(),
                fixed_parent: "Objects".to_owned(),
                extension: "BIPED".to_owned(),
            }
        );
    }

    #[test]
    fn duplicate_operation_routes_are_distinct_and_preserve_existing_paths() {
        assert_ne!(TagNameOperation::Duplicate, TagNameOperation::SaveAsOverlay);
        assert_ne!(TagNameOperation::Duplicate, TagNameOperation::Rename);
        assert_eq!(
            name_operation_route(TagNameOperation::Duplicate),
            NameOperationRoute::InPlaceDuplicateConfirmation
        );
        assert_eq!(
            name_operation_route(TagNameOperation::SaveAsOverlay),
            NameOperationRoute::SaveAsOverlay
        );
        assert_eq!(
            name_operation_route(TagNameOperation::Rename),
            NameOperationRoute::Rename
        );
    }

    #[test]
    fn duplicate_bytes_keep_stored_bytes_and_do_not_mutate_document_state() {
        let stored = b"stored bytes with original layout";
        assert_eq!(select_duplicate_bytes(stored, None).unwrap(), stored);

        let clean_tag = test_tag();
        let clean_document = TagDocument::clean(clean_tag);
        assert_eq!(
            select_duplicate_bytes(stored, Some(&clean_document)).unwrap(),
            stored
        );

        let dirty_tag = test_tag();
        let dirty_document = TagDocument::modified(dirty_tag);
        let before = dirty_document.tag.write_to_bytes().unwrap();
        let revision = dirty_document.dirty.revision();
        let copied = select_duplicate_bytes(&[], Some(&dirty_document)).unwrap();
        assert_eq!(copied, before);
        assert!(dirty_document.dirty.is_set());
        assert_eq!(dirty_document.dirty.revision(), revision);
        assert_eq!(dirty_document.tag.write_to_bytes().unwrap(), before);
    }

    #[test]
    fn loose_duplicate_destination_and_create_new_preserve_extension_bytes_and_collision() {
        let root = temp_fixture("loose-create-new");
        let parent = root.join("Objects");
        fs::create_dir_all(&parent).unwrap();
        let source = parent.join("Old.BIPED");
        let destination = loose_duplicate_destination(&source, "New_copy").unwrap();
        assert_eq!(destination, parent.join("New_copy.BIPED"));

        let existing = b"keep this destination";
        fs::write(&destination, existing).unwrap();
        assert!(write_create_new(&destination, b"must not replace").is_err());
        assert_eq!(fs::read(&destination).unwrap(), existing);

        reset_readonly_and_remove(&destination);
        let copied = b"exact duplicate bytes\0\x01";
        write_create_new(&destination, copied).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), copied);

        let source_entry = TagEntry {
            key: "file:source".to_owned(),
            display_path: "Old.BIPED".to_owned(),
            group_tag: test_tag().header.group_tag,
            group_name: Some("camera_track".to_owned()),
            location: TagEntryLocation::LooseFile(source.clone()),
        };
        let duplicate_entry = loose_duplicate_entry(
            &TagSource::SingleFile {
                path: source.clone(),
            },
            &source_entry,
            &TagNameIndex::default(),
            &destination,
            "New_copy",
        )
        .unwrap();
        assert_eq!(duplicate_entry.group_tag, source_entry.group_tag);
        assert!(matches!(
            duplicate_entry.location,
            TagEntryLocation::LooseFile(ref path) if path == &destination
        ));

        reset_readonly_and_remove(&destination);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn container_duplicate_paths_preserve_parent_and_group_suffix() {
        let paths = container_duplicate_paths(
            "Meteorite/Content/Tags/Objects/old-biped.ubulk",
            "objects/old.model",
            "new_copy",
        )
        .unwrap();
        assert_eq!(
            paths.uasset,
            "Meteorite/Content/Tags/Objects/new_copy-biped.uasset"
        );
        assert_eq!(
            paths.ubulk,
            "Meteorite/Content/Tags/Objects/new_copy-biped.ubulk"
        );
        assert_eq!(paths.package, "/Game/Tags/Objects/new_copy-biped");
        assert_eq!(paths.display, "objects/new_copy.model");
    }

    #[test]
    fn container_duplicate_index_strips_group_suffix_and_keeps_original_rel_path() {
        let rel_path = "Meteorite/Content/Tags/Objects/new_copy-biped.ubulk";
        let group_tag = u32::from_be_bytes(*b"bipd");
        let logical = container_logical_path(rel_path).unwrap();
        assert_eq!(logical, "objects/new_copy");
        assert_ne!(logical, "objects/new_copy-biped");

        let mut index = crate::source::ContainerTagIndex::default();
        let key = container_duplicate_index_key(group_tag, rel_path).unwrap();
        index.insert(key, 7, rel_path.to_owned());
        assert_eq!(
            index.lookup(group_tag, "objects/new_copy"),
            Some((7, rel_path))
        );
        assert_eq!(
            index.lookup(group_tag, "OBJECTS\\NEW_COPY"),
            Some((7, rel_path))
        );
        assert_eq!(index.lookup(group_tag, "objects/new_copy-biped"), None);
    }

    #[test]
    fn lower_provider_search_is_nearest_first() {
        assert_eq!(
            lower_priority_container_indices(4, 8).collect::<Vec<_>>(),
            vec![3, 2, 1, 0]
        );
        assert_eq!(
            lower_priority_container_indices(0, 8).collect::<Vec<_>>(),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn duplicate_completion_applies_whenever_the_workspace_is_still_open() {
        // The write is already on disk when this runs, and a pak rewrite takes
        // seconds — long enough for routine work to bump the kit's generation.
        // Only a closed workspace may drop the result.
        assert_eq!(
            classify_container_duplicate_completion(true, true),
            ContainerDuplicateCompletion::Apply
        );
        assert_eq!(
            classify_container_duplicate_completion(true, false),
            ContainerDuplicateCompletion::KitClosed
        );
        assert_eq!(
            classify_container_duplicate_completion(false, true),
            ContainerDuplicateCompletion::Failed
        );
    }

    #[test]
    fn a_container_that_is_no_longer_mounted_has_no_index() {
        let source = container_source(vec![]);
        let utoc = Path::new("C:/Game/Paks/pakchunk0-Windows.utoc");
        // A workspace that no longer holds the container a copy was written into
        // is the one case that genuinely cannot register it.
        assert_eq!(container_index_for_utoc(Some(&source), 0, utoc), None);
        assert_eq!(container_index_for_utoc(None, 0, utoc), None);
    }

    #[test]
    fn duplicate_completion_failure_and_closed_kit_paths_clear_guard_without_phantom_state() {
        let group_tag = test_tag().header.group_tag;
        let source_entry = test_entry("source", "objects/source.model", group_tag);
        let destination_entry = test_entry("destination", "objects/new_copy.model", group_tag);

        for (succeeded, current) in [(false, true), (true, false)] {
            let mut running = HashSet::from([KitId(41)]);
            let mut source = container_source(vec![source_entry.clone()]);
            let mut kit = Kit::empty(KitId(41), TagNameIndex::default());
            let destination_tag = test_tag();
            let outcome = classify_container_duplicate_completion(succeeded, current);
            clear_container_duplicate_running(&mut running, KitId(41));

            if outcome == ContainerDuplicateCompletion::Apply {
                apply_container_duplicate_source_state(
                    &mut source,
                    0,
                    group_tag,
                    "/Game/Tags/Objects/new_copy-model",
                    "Meteorite/Content/Tags/Objects/new_copy-model.uasset",
                    "Meteorite/Content/Tags/Objects/new_copy-model.ubulk",
                    false,
                    &destination_entry,
                    &destination_tag,
                )
                .unwrap();
                register_clean_duplicate_document(
                    &mut kit,
                    destination_entry.clone(),
                    destination_tag,
                );
            }

            assert!(running.is_empty());
            assert_eq!(source.entries.len(), 1);
            assert_eq!(source.entries[0].key, source_entry.key);
            let TagSource::IoStoreContainerSet { index, .. } = &source.source else {
                panic!("test source must be a container source");
            };
            assert!(index.lookup(group_tag, "objects/new_copy").is_none());
            assert!(!kit.parsed_tags.contains_key(&destination_entry.key));
            assert!(!kit.open_tabs.contains(&destination_entry.key));
        }
    }

    #[test]
    fn duplicate_completion_success_adds_clean_destination_and_preserves_dirty_source() {
        let group_tag = test_tag().header.group_tag;
        let source_entry = test_entry("source", "objects/source.model", group_tag);
        let destination_entry = test_entry("destination", "objects/new_copy.model", group_tag);
        let mut source = container_source(vec![source_entry.clone()]);
        let mut kit = Kit::empty(KitId(41), TagNameIndex::default());
        let source_tag = test_tag();
        let source_document = TagDocument::modified(source_tag);
        let source_before = source_document.tag.write_to_bytes().unwrap();
        let source_revision = source_document.dirty.revision();
        kit.parsed_tags
            .insert(source_entry.key.clone(), source_document);
        let destination_tag = test_tag();

        apply_container_duplicate_source_state(
            &mut source,
            0,
            group_tag,
            "/Game/Tags/Objects/new_copy-model",
            "Meteorite/Content/Tags/Objects/new_copy-model.uasset",
            "Meteorite/Content/Tags/Objects/new_copy-model.ubulk",
            false,
            &destination_entry,
            &destination_tag,
        )
        .unwrap();
        register_clean_duplicate_document(&mut kit, destination_entry.clone(), destination_tag);

        assert_eq!(source.entries.len(), 2);
        let TagSource::IoStoreContainerSet { index, .. } = &source.source else {
            panic!("test source must be a container source");
        };
        assert_eq!(
            index.lookup(group_tag, "objects/new_copy"),
            Some((0, "Meteorite/Content/Tags/Objects/new_copy-model.ubulk"))
        );
        let source_document = kit.parsed_tags.get(&source_entry.key).unwrap();
        assert!(source_document.dirty.is_set());
        assert_eq!(source_document.dirty.revision(), source_revision);
        assert_eq!(source_document.tag.write_to_bytes().unwrap(), source_before);
        let destination_document = kit.parsed_tags.get(&destination_entry.key).unwrap();
        assert!(!destination_document.dirty.is_set());
        assert!(kit.open_tabs.contains(&destination_entry.key));
        assert_eq!(
            kit.selected_key.as_deref(),
            Some(destination_entry.key.as_str())
        );
    }

    #[test]
    fn duplicate_lands_beside_its_source_rather_than_at_the_end_of_the_folder() {
        // Folders are drawn in entry-vector order under the default Natural
        // sort, so a pushed copy would appear at the bottom of the folder —
        // which is what made a successful duplicate look like it never landed.
        let group_tag = test_tag().header.group_tag;
        let mut source = container_source(vec![
            test_entry("alpha", "objects/alpha.model", group_tag),
            test_entry("source", "objects/source.model", group_tag),
            test_entry("zulu", "objects/zulu.model", group_tag),
        ]);
        let destination_entry = test_entry("destination", "objects/source_copy.model", group_tag);

        apply_container_duplicate_source_state(
            &mut source,
            0,
            group_tag,
            "/Game/Tags/Objects/source_copy-model",
            "Meteorite/Content/Tags/Objects/source_copy-model.uasset",
            "Meteorite/Content/Tags/Objects/source_copy-model.ubulk",
            false,
            &destination_entry,
            &test_tag(),
        )
        .unwrap();

        let order: Vec<&str> = source
            .entries
            .iter()
            .map(|entry| entry.display_path.as_str())
            .collect();
        assert_eq!(
            order,
            [
                "objects/alpha.model",
                "objects/source.model",
                "objects/source_copy.model",
                "objects/zulu.model",
            ]
        );
    }

    #[test]
    fn single_file_registration_updates_browser_source_and_preserves_group() {
        let old_entry = TagEntry {
            key: "file:old".to_owned(),
            display_path: "old.model".to_owned(),
            group_tag: test_tag().header.group_tag,
            group_name: Some("camera_track".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from("old.model")),
        };
        let new_entry = TagEntry {
            key: "file:new".to_owned(),
            display_path: "new.model".to_owned(),
            group_tag: old_entry.group_tag,
            group_name: old_entry.group_name.clone(),
            location: TagEntryLocation::LooseFile(PathBuf::from("new.model")),
        };
        let entries = vec![old_entry];
        let mut source = LoadedSourceData {
            label: "single file".to_owned(),
            source: TagSource::SingleFile {
                path: PathBuf::from("old.model"),
            },
            names: TagNameIndex::default(),
            game: None,
            tree: crate::source::build_tree(&entries),
            group_tree: crate::source::build_group_tree(&entries),
            entries,
            all_entries: Vec::new(),
            reverse_dependencies: None,
            initial_tag: None,
        };

        crate::app::controller::register_created_tag_in_source(&mut source, new_entry.clone());

        assert_eq!(source.entries.len(), 2);
        assert!(
            source.entries.iter().any(|entry| {
                entry.key == new_entry.key && entry.group_tag == new_entry.group_tag
            })
        );
        assert!(source.all_entries.is_empty());
        assert!(source.tree.entries.contains(&1));
        assert!(
            source
                .group_tree
                .children
                .iter()
                .flat_map(|node| node.entries.iter())
                .any(|&index| source.entries[index].key == new_entry.key)
        );
    }

    #[test]
    fn duplicate_backup_is_exact_manifested_and_read_only() {
        let root = temp_fixture("backup");
        fs::create_dir_all(&root).unwrap();
        let utoc = root.join("pakchunk7-WinGDK.utoc");
        let ucas = root.join("pakchunk7-WinGDK.ucas");
        let original = b"exact toc bytes\0\x01".to_vec();
        fs::write(&utoc, &original).unwrap();
        fs::write(&ucas, vec![4u8; 37]).unwrap();
        let backup = create_duplicate_backup(&utoc).unwrap();
        assert_eq!(fs::read(&backup.utoc).unwrap(), original);
        assert_eq!(
            backup.manifest.file_name().unwrap().to_string_lossy(),
            "pakchunk7-WinGDK.utoc.baboon-duplicate-backup.manifest.json"
        );
        assert!(fs::metadata(&backup.utoc).unwrap().permissions().readonly());
        assert!(
            fs::metadata(&backup.manifest)
                .unwrap()
                .permissions()
                .readonly()
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&backup.manifest).unwrap()).unwrap();
        assert_eq!(manifest["version"], DUPLICATE_BACKUP_VERSION);
        assert_eq!(manifest["original_ucas_length"], 37);

        // A container can legitimately be written to more than once — duplicate,
        // delete, duplicate again — so a second backup takes the next slot
        // instead of failing, and never disturbs the first.
        fs::write(&utoc, b"second generation").unwrap();
        let second = create_duplicate_backup(&utoc).unwrap();
        assert_ne!(second.utoc, backup.utoc);
        assert_eq!(fs::read(&backup.utoc).unwrap(), original);
        assert_eq!(fs::read(&second.utoc).unwrap(), b"second generation");
        reset_readonly_and_remove(&second.manifest);
        reset_readonly_and_remove(&second.utoc);
        reset_readonly_and_remove(&backup.manifest);
        reset_readonly_and_remove(&backup.utoc);
        let _ = fs::remove_file(utoc);
        let _ = fs::remove_file(ucas);
        let _ = fs::remove_dir(&root);
    }

    #[test]
    fn duplicate_backup_never_writes_over_an_occupied_slot() {
        // A backup is the only record of a state the container can still be
        // walked back to, so an occupied slot is stepped over, never reused —
        // even when what occupies it is not something Baboon wrote.
        let root = temp_fixture("backup-occupied-slot");
        fs::create_dir_all(&root).unwrap();
        let utoc = root.join("pakchunk8-WinGDK.utoc");
        let ucas = root.join("pakchunk8-WinGDK.ucas");
        fs::write(&utoc, b"original toc").unwrap();
        fs::write(&ucas, b"ucas").unwrap();
        let manifest = backup_sibling_path(&utoc, DUPLICATE_BACKUP_MANIFEST_SUFFIX).unwrap();
        fs::write(&manifest, b"keep").unwrap();

        let backup = create_duplicate_backup(&utoc).unwrap();
        assert_ne!(backup.manifest, manifest);
        assert_eq!(fs::read(&manifest).unwrap(), b"keep");
        assert_eq!(fs::read(&backup.utoc).unwrap(), b"original toc");

        reset_readonly_and_remove(&backup.manifest);
        reset_readonly_and_remove(&backup.utoc);
        let _ = fs::remove_file(manifest);
        let _ = fs::remove_file(utoc);
        let _ = fs::remove_file(ucas);
        let _ = fs::remove_dir(&root);
    }
}
