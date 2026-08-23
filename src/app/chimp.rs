//! Chimp: the Campaign Evolved Unreal package workspace.
//!
//! Chimp is deliberately scoped to a loaded Campaign Evolved kit. It shares
//! that kit's Paks root but owns its own package index, documents and editor
//! state; none of those concepts are forced through the editing-kit/tag model.

use super::*;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

mod level;
pub(in crate::app) use level::{LevelScene, read_cell_into, read_cells};
mod level_blend;
mod level_export;
mod level_segment;
mod mesh_weld;
pub(in crate::app) use level_export::{scene_to_usd, write_blend_export, write_segmented_usd};
use level_export::{ExportStage, MeshDetail};
use level_segment::SegmentBudget;
use std::io::{Cursor, Write};

use blam_tags::iostore::asset::texture2d::{Texture2dSurfaces, decode_texture2d_surfaces};
use blam_tags::iostore::container::writer::{
    PackageOverride, PackageReplacement, overwrite_package_in_place_with,
    overwrite_packages_in_place_with, write_package_mod_container,
};
use blam_tags::iostore::object::archive::ExportContext;
use blam_tags::iostore::object::edit::{
    count_object_references, default_value_for_type, editable_schema_slots, property_type_for_slot,
    set_property_slot, validate_value_for_type,
};
use blam_tags::iostore::object::export::{Export, ExportBlock, read_export_in, write_export_in};
use blam_tags::iostore::object::hand_written as chimp_hw;
use blam_tags::iostore::object::native::{NativeStruct, PerPlatformValue};
use blam_tags::iostore::object::tail_models::{TailContext, parse_texture_chain_tail};
use blam_tags::iostore::object::usmap::PropertyType;
use blam_tags::iostore::object::value::{FName, PropValue, PropertyBlock};
use blam_tags::iostore::package::builder::{read_payloads, write_package};
use blam_tags::iostore::package::imports::{
    ImportSlot, ImportTarget, import_package_index, import_slot_of, public_export_hash,
    read_import_slots, write_import_slots,
};
use blam_tags::iostore::package::name_map::FMappedName;
use blam_tags::iostore::package::ue_types::{FPackageObjectIndex, FPackageObjectIndexType};
use blam_tags::iostore::package::zen::{
    EExportFilterFlags, EZenPackageVersion, FZenPackageHeader,
};
use blam_tags::iostore::skeletal_mesh::SkeletalMesh;
use blam_tags::iostore::static_mesh::StaticMesh;
use blam_tags::iostore::usmap::Usmap;
use blam_tags::iostore::world::{CE_HEADER_VERSION, CE_TOC_VERSION, PackageProvider, World};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum KitSurface {
    #[default]
    Tags,
    Chimp,
}

/// A mesh export waiting on the answer to "textures as well?".
///
/// The destination is already chosen at this point, so the choice can be made
/// against the folder the textures would actually appear in.
pub(super) struct ChimpMeshTexturePrompt {
    pub(super) kit: KitId,
    pub(super) package: String,
    /// Read through [`ChimpMeshTexturePrompt::format_label`]; the format itself
    /// is Chimp's own business.
    format: ChimpMeshFormat,
    /// Which image format the textures would be written as, if any are. Asked
    /// here rather than in a second window, because it is only one more choice
    /// about the same export and the answer only matters alongside the two
    /// buttons that ask for textures at all.
    pub(super) texture_export: ChimpTextureExport,
    pub(super) path: PathBuf,
}

impl ChimpMeshTexturePrompt {
    /// Where the textures would be written.
    pub(super) fn texture_directory(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(CHIMP_TEXTURE_DIR)
    }

    pub(super) fn format_label(&self) -> &'static str {
        self.format.label()
    }

    /// The word "matching textures" would filter on, if one can be derived.
    pub(super) fn texture_subject(&self) -> Option<String> {
        chimp_mesh_texture_subject(&self.package)
    }
}

pub(super) enum ChimpMount {
    Idle,
    Loading,
    Ready(Arc<World>),
    Failed(String),
}

impl Default for ChimpMount {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpBrowser {
    #[default]
    Folders,
    Groups,
    Archives,
    Packages,
    Files,
}

impl KitSurface {
    pub(super) const TABS: [(Self, &'static str, &'static str); 2] = [
        (Self::Tags, "Tags", "Browse and edit Halo tags"),
        (
            Self::Chimp,
            "Chimp",
            "Browse and edit Unreal Engine packages",
        ),
    ];
}

impl ChimpBrowser {
    const TABS: [(Self, &'static str); 5] = [
        (Self::Folders, "Folders"),
        (Self::Groups, "Groups"),
        (Self::Files, "Pak files"),
        (Self::Archives, "Archives"),
        (Self::Packages, "Packages"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpArchive {
    IoStore(usize),
    Pak(usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpFolderSelection {
    #[default]
    Package,
    File,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpDocumentView {
    #[default]
    Document,
    Texture,
    Mesh,
    Properties,
    Header,
    Metadata,
}

/// How a Texture2D extraction should be written.
///
/// All three split a UDIM set into 1001-numbered files and give each block its
/// authored resolution. DDS additionally keeps the cooked pixel format and the
/// whole mip chain; TIFF and PNG are one flat RGBA8 image per block.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ChimpTextureFormat {
    Dds,
    Tiff,
    Png,
}

impl ChimpTextureFormat {
    pub(super) const ALL: [Self; 3] = [Self::Dds, Self::Tiff, Self::Png];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Dds => "DDS",
            Self::Tiff => "TIFF",
            Self::Png => "PNG",
        }
    }

    /// What choosing this format actually gets you, in the prompt.
    pub(super) fn summary(self) -> &'static str {
        match self {
            Self::Dds => {
                "The bytes the game ships: the cooked pixel format (BC1-BC7 and the \
                 uncompressed formats) with the whole mip chain, not a re-encode. \
                 The only choice that round-trips back into Unreal unchanged."
            }
            Self::Tiff => {
                "One flat RGBA8 image, uncompressed. Larger than PNG, and what most \
                 texture and compositing tools prefer to be handed."
            }
            Self::Png => {
                "One flat RGBA8 image, losslessly compressed. The smallest of the \
                 three and the one anything will open."
            }
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Dds => "dds",
            Self::Tiff => "tif",
            Self::Png => "png",
        }
    }

    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Dds => ("DDS image", &["dds"]),
            Self::Tiff => ("TIFF image", &["tif", "tiff"]),
            Self::Png => ("PNG image", &["png"]),
        }
    }
}

/// A texture export waiting on the answer to "which image format?".
///
/// The format is asked before the save dialog rather than after, because it
/// decides what the export *is* — one file or a numbered UDIM set, compressed
/// mips or a flat image — and the file picker's name and filter follow from it.
pub(super) struct ChimpTextureExportPrompt {
    pub(super) kit: KitId,
    pub(super) package: String,
    pub(super) export: ChimpTextureExport,
}

impl ChimpTextureExportPrompt {
    pub(super) fn name(&self) -> &str {
        self.package.rsplit('/').next().unwrap_or(&self.package)
    }
}

/// One entry; the format is chosen in the prompt it opens.
fn chimp_texture_export_menu(ui: &mut egui::Ui, package: &str, out: &mut Option<String>) {
    if ui.button("Extract Texture2D…").clicked() {
        *out = Some(package.to_owned());
        ui.close_menu();
    }
}

enum ChimpTreeClick {
    Package(String),
    ExtractTexture(String),
    ExtractMesh(String, ChimpMeshFormat),
    ExportLevel(String, ChimpLevelFormat),
    File(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpMeshKind {
    Skeletal,
    Static,
}

/// How a whole level leaves Baboon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChimpLevelFormat {
    /// One shared prototype library plus a file per region.
    SegmentedUsd,
    /// Raw geometry plus a script that builds `.blend` files from it.
    Blender,
}

impl ChimpLevelFormat {
    fn label(self) -> &'static str {
        match self {
            Self::SegmentedUsd => "USD (.usda)",
            Self::Blender => "Blender (.blend)",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::SegmentedUsd => {
                "A prototype library holding every mesh once, and a file per region \
                 that places them. Imports into any DCC that reads USD."
            }
            Self::Blender => {
                "Raw geometry and a script Blender runs to build one .blend per mesh \
                 and a master that places them as linked duplicates."
            }
        }
    }
}

/// Which part of a level export is running.
///
/// Reported separately because they are not comparable: reading cells is
/// thousands of small packages, writing meshes is hundreds of large ones, and a
/// single bar across both would crawl and then leap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChimpLevelPhase {
    ReadingCells,
    WritingMeshes,
    WritingSegments,
}

impl ChimpLevelPhase {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ReadingCells => "Reading cells",
            Self::WritingMeshes => "Writing meshes",
            Self::WritingSegments => "Writing segments",
        }
    }

    /// Where this phase sits, for "step 1 of 3".
    pub(super) fn step(self) -> usize {
        match self {
            Self::ReadingCells => 1,
            Self::WritingMeshes => 2,
            Self::WritingSegments => 3,
        }
    }
}

/// A level export in flight, and how far along it is.
pub(super) struct ChimpLevelJob {
    pub(super) kit: KitId,
    pub(super) name: String,
    pub(super) phase: ChimpLevelPhase,
    pub(super) done: usize,
    pub(super) total: usize,
    /// When the current phase began, so the estimate is of the work being done
    /// rather than an average across phases that cost different amounts.
    pub(super) phase_started: Instant,
}

impl ChimpLevelJob {
    pub(super) fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
    }

    /// How much longer this phase looks like taking, or `None` until there is
    /// enough of it done to say anything honest.
    pub(super) fn remaining(&self) -> Option<Duration> {
        if self.done == 0 || self.done >= self.total {
            return None;
        }
        let elapsed = self.phase_started.elapsed().as_secs_f64();
        // A first few items are not a rate. Guessing from them produces a
        // number that swings by minutes and teaches the user to ignore it.
        if elapsed < 1.5 {
            return None;
        }
        let each = elapsed / self.done as f64;
        Some(Duration::from_secs_f64(
            each * (self.total - self.done) as f64,
        ))
    }
}

/// "about 4m 20s", or "a few seconds" when there is no point being precise.
pub(super) fn format_remaining(remaining: Duration) -> String {
    let seconds = remaining.as_secs();
    if seconds < 10 {
        return "a few seconds".to_owned();
    }
    if seconds < 60 {
        return format!("about {seconds}s");
    }
    let minutes = seconds / 60;
    let rest = seconds % 60;
    if minutes < 10 {
        format!("about {minutes}m {rest:02}s")
    } else {
        format!("about {minutes}m")
    }
}

/// A level waiting on the answer to "how, and split how far?".
pub(super) struct ChimpLevelExportPrompt {
    pub(super) kit: KitId,
    pub(super) package: String,
    /// The `_Generated_` cells this level is made of.
    pub(super) cells: Vec<String>,
    pub(super) format: ChimpLevelFormat,
    /// Full Nanite geometry rather than the coarse fallback proxy.
    pub(super) nanite: bool,
    pub(super) split: bool,
    pub(super) triangles: usize,
    pub(super) placements: usize,
}

impl ChimpLevelExportPrompt {
    pub(super) fn format_label(&self) -> &'static str {
        self.format.label()
    }

    pub(super) fn format_summary(&self) -> &'static str {
        self.format.summary()
    }

    /// What the exported files are named after.
    pub(super) fn name(&self) -> String {
        prim_safe_name(self.package.rsplit('/').next().unwrap_or("level"))
    }

    fn budget(&self) -> SegmentBudget {
        if self.split {
            SegmentBudget {
                triangles: self.triangles.max(1),
                placements: self.placements.max(1),
            }
        } else {
            // Not "no segmentation" but one segment: the same code path, with a
            // ceiling nothing reaches, so there is no second way to export.
            SegmentBudget {
                triangles: usize::MAX,
                placements: usize::MAX,
            }
        }
    }
}

/// A file-name-safe version of a package leaf.
fn prim_safe_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if name.is_empty() {
        "level".to_owned()
    } else {
        name
    }
}

/// What a package's context menu offers.
///
/// One answer, used both to decide whether to attach a menu at all and to decide
/// what goes in it. The browser draws packages in four places, and when the two
/// decisions were written out separately at each of them they drifted: the level
/// entry was added to every menu body, but the guard on two of the sites still
/// only admitted textures and meshes, so on those the entry sat inside a menu
/// that was never built. A menu that offers nothing must not open, and anything
/// it would offer must open it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChimpPackageActions {
    texture: bool,
    mesh: bool,
    level: bool,
}

impl ChimpPackageActions {
    fn of(package: &str, kind: Option<&str>) -> Self {
        Self {
            texture: kind == Some("Texture2D"),
            mesh: matches!(kind, Some("SkeletalMesh" | "StaticMesh")),
            level: chimp_looks_like_level(package),
        }
    }

    fn any(self) -> bool {
        self.texture || self.mesh || self.level
    }
}

/// Whether a package is shaped like a World Partition persistent level.
///
/// A cheap name test rather than a search, because it runs while a context menu
/// is open and the real answer means scanning every package in the install.
/// Unreal names a persistent level after the folder that holds it — C10 lives at
/// `.../C10/C10` — so that is what this looks for, and
/// [`chimp_level_cells`] decides for certain once the menu is actually used.
fn chimp_looks_like_level(package: &str) -> bool {
    let Some((directory, leaf)) = package.rsplit_once('/') else {
        return false;
    };
    let Some((_, folder)) = directory.rsplit_once('/') else {
        return false;
    };
    !leaf.is_empty() && leaf.eq_ignore_ascii_case(folder)
}

/// The cells a World Partition level is made of, empty if this is not one.
///
/// A persistent level sits beside a `_Generated_` folder holding the cells that
/// place its actors — `C10` is 2,334 of them — so the level package itself
/// places almost nothing and finding the cells is what makes it exportable.
fn chimp_level_cells(world: &World, package: &str) -> Vec<String> {
    let Some((directory, _)) = package.rsplit_once('/') else {
        return Vec::new();
    };
    let prefix = format!("{directory}/_Generated_/").to_lowercase();
    let mut cells: Vec<String> = world
        .packages()
        .iter()
        .filter(|record| record.name.to_lowercase().starts_with(&prefix))
        .map(|record| record.name.clone())
        .collect();
    cells.sort();
    cells
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChimpMeshFormat {
    Jms,
    Psk,
    Pskx,
}

impl ChimpMeshFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jms => "jms",
            Self::Psk => "psk",
            Self::Pskx => "pskx",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Jms => "JMS",
            Self::Psk => "ActorX PSK",
            Self::Pskx => "ActorX PSKX",
        }
    }
}

#[derive(Default)]
struct ChimpFolderNode {
    folders: BTreeMap<String, ChimpFolderNode>,
    packages: Vec<ChimpPackageLeaf>,
    files: Vec<ChimpFileLeaf>,
    package_count: usize,
    file_count: usize,
}

struct ChimpPackageLeaf {
    name: String,
    package: usize,
}

struct ChimpFileLeaf {
    name: String,
    file: usize,
}

impl ChimpFolderNode {
    fn insert_package(&mut self, package: usize, path: &str) {
        let mut segments = path
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .peekable();
        let mut node = self;
        node.package_count += 1;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                node.packages.push(ChimpPackageLeaf {
                    name: segment.to_owned(),
                    package,
                });
                return;
            }
            node = node.folders.entry(segment.to_owned()).or_default();
            node.package_count += 1;
        }
    }

    fn insert_file(&mut self, file: usize, path: &str) {
        let normalized = path.replace('\\', "/");
        let mut segments = normalized
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
            .peekable();
        let mut node = self;
        node.file_count += 1;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                node.files.push(ChimpFileLeaf {
                    name: segment.to_owned(),
                    file,
                });
                return;
            }
            node = node.folders.entry(segment.to_owned()).or_default();
            node.file_count += 1;
        }
    }

    fn entry_count(&self) -> usize {
        self.package_count + self.file_count
    }
}

#[derive(Default)]
pub(super) struct ChimpState {
    pub(super) mount: ChimpMount,
    browser: ChimpBrowser,
    pub(super) filter: String,
    filtered_for: Option<String>,
    filtered_archive_for: Option<Option<ChimpArchive>>,
    filtered_packages: Vec<usize>,
    filtered_files: Vec<usize>,
    filtered_groups: BTreeMap<String, Vec<usize>>,
    content_tree: ChimpFolderNode,
    selected_archive: Option<ChimpArchive>,
    package_types: Vec<Option<String>>,
    type_indexing: bool,
    folder_selection: ChimpFolderSelection,
    pub(super) selected_package: Option<String>,
    selected_file: Option<String>,
    pub(super) open_packages: Vec<String>,
    document_tree: Option<egui_tiles::Tree<String>>,
    pub(super) documents: HashMap<String, ChimpDocument>,
    pub(super) loading_packages: HashSet<String>,
    pending_overwrite: Option<String>,
    pending_overwrite_skip_future: bool,
    save_dialog: Option<ChimpSaveDialog>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ChimpSaveMode {
    #[default]
    ExportMod,
    OverwriteSources,
}

struct ChimpSaveDialog {
    mode: ChimpSaveMode,
    name: String,
    folder: PathBuf,
    overwrite_acknowledged: bool,
    pending_close_action: Option<PendingCloseAction>,
}

enum ChimpSaveAction {
    Export(PathBuf),
    Overwrite,
}

pub(super) struct ChimpDocument {
    pub(super) package: String,
    pub(super) provider: PackageProvider,
    pub(super) original: Vec<u8>,
    pub(super) header: FZenPackageHeader,
    pub(super) payloads: Vec<Vec<u8>>,
    pub(super) exports: Vec<ChimpExport>,
    texture_previews: Vec<ChimpTexturePreview>,
    mesh_kind: Option<ChimpMeshKind>,
    mesh_preview: Option<Result<ModelPreviewData, String>>,
    mesh_preview_state: ModelPreviewState,
    pub(super) selected_export: usize,
    pub(super) dirty: bool,
    view: ChimpDocumentView,
    document_text: String,
    document_line_numbers: String,
    document_text_dirty: bool,
    metadata_text: String,
    metadata_line_numbers: String,
    metadata_text_dirty: bool,
    /// Who references each name-map entry and each import slot.
    ///
    /// Cached because it walks every decoded export, and invalidated with the
    /// metadata text — the two go stale together, on any header change.
    header_usage: Option<ChimpHeaderUsage>,
    header_name_filter: String,
    /// The name-map row being edited, if any. Header edits commit explicitly
    /// rather than per keystroke: a rename walks every decoded export and then
    /// rebuilds the whole package for the recovery checkpoint, which is not
    /// something to do between two letters of a word.
    header_name_edit: Option<ChimpNameEdit>,
    header_import_edit: Option<ChimpImportEdit>,
    header_export_edit: Option<ChimpExportEdit>,
    header_identity_edit: Option<ChimpIdentityEdit>,
    header_error: Option<String>,
    /// Who imports this package. Not derived at load: there is no reverse index
    /// in the paks, so answering it means reading every mounted header.
    referrers: ChimpReferrerState,
    /// The mounted containers no longer provide this package, so `provider` no
    /// longer describes anything and nothing may be written back through it.
    /// The document keeps its bytes, so reading and extraction still work.
    orphaned: bool,
}

#[derive(Default)]
enum ChimpReferrerState {
    #[default]
    Idle,
    Scanning,
    Done(ChimpReferrerScan),
}

/// A draft of the package's flags and versioning.
///
/// The version fields are not metadata: `file_version_ue5` gates whether the
/// bulk-data map is serialized at all, and the Zen version decides the header's
/// own shape. That is why nothing here is applied without first writing the
/// package and reading it back.
struct ChimpIdentityEdit {
    /// Free hex, so an unnamed bit can be set without inventing a checkbox for
    /// every flag the engine defines.
    package_flags: String,
    licensee_version: i32,
    is_unversioned: bool,
    zen_version: EZenPackageVersion,
    file_version_ue4: i32,
    file_version_ue5: i32,
}

/// A draft export-map entry.
struct ChimpExportEdit {
    index: usize,
    object_name: String,
    object_flags: u32,
    filter_flags: EExportFilterFlags,
    /// Recompute `public_export_hash` from the new object name.
    ///
    /// Off by default and expert-only, because it cuts the other way from the
    /// desync it fixes: importers hold the *old* hash, so recomputing makes this
    /// package right and every importer wrong.
    recompute_hash: bool,
}

struct ChimpNameEdit {
    index: usize,
    text: String,
    focus: bool,
}

/// A draft import slot.
///
/// Both halves are kept as text because neither is recoverable from what the
/// file stores: a script import is a one-way hash of its object path, and a
/// package import carries only `public_export_hash`. Whatever the user types is
/// the only name either has, and it is never written back as one.
struct ChimpImportEdit {
    /// The slot being edited, or `slots.len()` for one being appended.
    slot: usize,
    kind: ChimpImportKind,
    /// `/Script/...` object path, for a script import.
    script_path: String,
    /// `/Game/...` package path, for a package import.
    package_path: String,
    /// Object name, hashed on commit. Never persisted: the file has no field for
    /// it, and claiming otherwise would be inventing provenance.
    object_name: String,
    /// Public exports of `package_path`, once someone asked for them. Loading
    /// another package to answer "what is hash X called" is worth doing on
    /// request and not on every draw.
    resolved: Option<Result<Vec<(String, u64)>, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChimpImportKind {
    Script,
    Package,
    Null,
}

/// What the Header view reports about references into the package's tables.
///
/// The counts are the point of the view: a name-map entry is addressed by index,
/// so editing one retargets every reference at once, and seeing how many there
/// are — and whether one of them is the package's own identity — is what makes
/// that safe to reason about before anything is editable.
#[derive(Default)]
struct ChimpHeaderUsage {
    /// Per name-map index, parallel to the table.
    names: Vec<ChimpNameUsage>,
    /// Per import slot, how many object properties name it.
    import_references: Vec<usize>,
}

#[derive(Default, Clone)]
struct ChimpNameUsage {
    /// Total references, including the ones `sites` summarises.
    count: usize,
    /// Where they are, deduplicated and capped — a tooltip, not a report.
    sites: Vec<String>,
    /// This entry backs `summary.name`, so it is the package's identity: the
    /// chunk that serves the package is addressed by a hash of this string.
    is_package_identity: bool,
}

impl ChimpNameUsage {
    fn record(&mut self, site: impl Into<String>) {
        self.count += 1;
        let site = site.into();
        if self.sites.len() < 8 && !self.sites.iter().any(|existing| *existing == site) {
            self.sites.push(site);
        }
    }
}

pub(super) struct ChimpTypeIndex {
    package_types: Vec<Option<String>>,
    type_counts: BTreeMap<String, usize>,
    failures: usize,
}

struct ChimpTexturePreview {
    export_index: usize,
    preview: BitmapPreviewState,
    /// Every layer and mip the package stores, kept in its cooked pixel format.
    /// The viewer expands one of these at a time; export writes them untouched.
    surfaces: Result<Texture2dSurfaces, String>,
}

#[derive(Default, Deserialize, Serialize)]
struct ChimpRecoveryManifest {
    source: String,
    packages: HashMap<String, String>,
}

pub(super) struct ChimpExport {
    pub(super) object: String,
    pub(super) class: Option<String>,
    pub(super) decoded: Result<Export, String>,
}

struct ChimpPaneBehavior<'a> {
    app: &'a mut Baboon,
    kit_index: usize,
    close_requests: Vec<String>,
    focused: Option<String>,
    close_all: bool,
    close_all_but: Option<String>,
    extract_texture: Option<String>,
    extract_mesh: Option<(String, ChimpMeshFormat)>,
    export_level: Option<(String, ChimpLevelFormat)>,
}

impl egui_tiles::Behavior<String> for ChimpPaneBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut Ui,
        tile_id: egui_tiles::TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        if ui.input(|input| input.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect())
        {
            self.focused = Some(pane.clone());
        }
        self.app.draw_chimp_document_pane(
            ui,
            self.kit_index,
            pane,
            &format!("chimp_tile_{}", tile_id.0),
        );
        egui_tiles::UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        let dirty = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(pane)
            .is_some_and(|document| document.dirty);
        let label = pane.rsplit('/').next().unwrap_or(pane);
        RichText::new(if dirty {
            format!("• {label}")
        } else {
            label.to_owned()
        })
        .color(text_dark())
        .into()
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) {
            self.close_requests.push(package.clone());
        }
        false
    }

    fn on_tab_button(
        &mut self,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(egui_tiles::Tile::Pane(package)) = tiles.get(tile_id) else {
            return button_response;
        };
        let package = package.clone();
        if button_response.clicked() {
            self.focused = Some(package.clone());
        }
        if button_response.middle_clicked() {
            self.close_requests.push(package.clone());
        }
        let has_texture = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| !document.texture_previews.is_empty());
        let has_mesh = self.app.kits[self.kit_index]
            .chimp
            .documents
            .get(&package)
            .is_some_and(|document| document.mesh_kind.is_some());
        button_response.context_menu(|ui| {
            if ui.button("Close").clicked() {
                self.close_requests.push(package.clone());
                ui.close_menu();
            }
            if ui.button("Close all but this").clicked() {
                self.close_all_but = Some(package.clone());
                ui.close_menu();
            }
            if ui.button("Close all").clicked() {
                self.close_all = true;
                ui.close_menu();
            }
            if has_texture {
                ui.separator();
                chimp_texture_export_menu(ui, &package, &mut self.extract_texture);
            }
            if has_mesh {
                ui.separator();
                ui.menu_button("Extract mesh", |ui| {
                    for format in [
                        ChimpMeshFormat::Jms,
                        ChimpMeshFormat::Psk,
                        ChimpMeshFormat::Pskx,
                    ] {
                        if ui.button(format.label()).clicked() {
                            self.extract_mesh = Some((package.clone(), format));
                            ui.close_menu();
                        }
                    }
                });
            }
            if chimp_looks_like_level(&package) {
                ui.separator();
                chimp_level_export_menu(ui, &package, &mut self.export_level);
            }
        });
        button_response
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        row_type()
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        tiles: &egui_tiles::Tiles<String>,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        let base = if state.active { menu_bar() } else { row_type() };
        let dirty = matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(package))
            if self.app.kits[self.kit_index]
                .chimp
                .documents
                .get(package)
                .is_some_and(|document| document.dirty));
        if dirty {
            chimp_tint_toward(base, Color32::from_rgb(184, 134, 11), 0.20)
        } else {
            base
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<String>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> Color32 {
        text_dark()
    }
}

impl ChimpState {
    fn ensure_document_tree(&mut self, kit: KitId) -> &mut egui_tiles::Tree<String> {
        self.document_tree
            .get_or_insert_with(|| egui_tiles::Tree::empty(chimp_tree_id(kit)))
    }

    fn open_document_pane(&mut self, kit: KitId, package: &str) {
        let tree = self.ensure_document_tree(kit);
        let existing = tree.tiles.iter().find_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(open) if open == package => Some(*id),
            _ => None,
        });
        if let Some(tile_id) = existing {
            tree.make_active(|id, _| id == tile_id);
        } else {
            let tile_id = tree.tiles.insert_pane(package.to_owned());
            match tree.root() {
                Some(root) => {
                    if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root) {
                        container.add_child(tile_id);
                    } else {
                        let tabs = tree.tiles.insert_tab_tile(vec![root, tile_id]);
                        tree.root = Some(tabs);
                    }
                }
                None => tree.root = Some(tile_id),
            }
            tree.make_active(|id, _| id == tile_id);
        }
        self.selected_package = Some(package.to_owned());
        self.sync_open_packages();
    }

    fn close_document_pane(&mut self, package: &str) {
        if let Some(tree) = self.document_tree.as_mut() {
            let tile_id = tree.tiles.iter().find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(open) if open == package => Some(*id),
                _ => None,
            });
            if let Some(tile_id) = tile_id {
                tree.remove_recursively(tile_id);
            }
        }
        self.sync_open_packages();
    }

    fn sync_open_packages(&mut self) {
        self.open_packages = self
            .document_tree
            .as_ref()
            .map(|tree| {
                tree.tiles
                    .tiles()
                    .filter_map(|tile| match tile {
                        egui_tiles::Tile::Pane(package) => Some(package.clone()),
                        egui_tiles::Tile::Container(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self
            .selected_package
            .as_ref()
            .is_some_and(|package| !self.open_packages.contains(package))
        {
            self.selected_package = self.open_packages.first().cloned();
        }
    }

    fn filter_is_current(&self, query: &str) -> bool {
        self.filtered_for.as_deref() == Some(query)
            && self.filtered_archive_for == Some(self.selected_archive)
    }

    fn refresh_filter(&mut self, world: &World) {
        let query = self.filter.trim().to_ascii_lowercase();
        if self.filter_is_current(&query) {
            return;
        }
        let selected_archive = self.selected_archive;
        self.filtered_for = Some(query.clone());
        self.filtered_archive_for = Some(selected_archive);
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.filtered_groups.clear();
        self.filtered_packages.extend(
            world
                .packages()
                .iter()
                .enumerate()
                .filter(|(index, package)| {
                    let archive_matches = match selected_archive {
                        None => true,
                        Some(ChimpArchive::IoStore(container)) => package
                            .providers
                            .iter()
                            .any(|provider| provider.container == container),
                        Some(ChimpArchive::Pak(_)) => false,
                    };
                    let type_matches = self
                        .package_types
                        .get(*index)
                        .and_then(Option::as_deref)
                        .is_some_and(|kind| chimp_contains_query(kind, &query));
                    archive_matches
                        && (query.is_empty()
                            || type_matches
                            || chimp_contains_query(&package.name, &query)
                            || package.providers.iter().any(|provider| {
                                chimp_contains_query(
                                    &world.containers()[provider.container]
                                        .path
                                        .to_string_lossy(),
                                    &query,
                                )
                            }))
                })
                .map(|(index, _)| index),
        );
        self.filtered_files.extend(
            world
                .pak_files()
                .iter()
                .enumerate()
                .filter(|(_, file)| {
                    let archive_matches = match selected_archive {
                        None => true,
                        Some(ChimpArchive::Pak(container)) => file
                            .providers
                            .iter()
                            .any(|provider| provider.container == container),
                        Some(ChimpArchive::IoStore(_)) => false,
                    };
                    archive_matches
                        && (query.is_empty()
                            || chimp_contains_query(&file.path, &query)
                            || file.providers.iter().any(|provider| {
                                chimp_contains_query(
                                    &world.pak_containers()[provider.container]
                                        .path
                                        .to_string_lossy(),
                                    &query,
                                )
                            }))
                })
                .map(|(index, _)| index),
        );
        self.content_tree = ChimpFolderNode::default();
        for &index in &self.filtered_packages {
            self.content_tree
                .insert_package(index, &world.packages()[index].name);
            self.filtered_groups
                .entry(
                    self.package_types
                        .get(index)
                        .and_then(Option::as_deref)
                        .unwrap_or("Unknown")
                        .to_owned(),
                )
                .or_default()
                .push(index);
        }
        for &index in &self.filtered_files {
            self.content_tree
                .insert_file(index, &world.pak_files()[index].path);
        }
    }

    fn reset_filter(&mut self) {
        self.filtered_for = None;
        self.filtered_archive_for = None;
        self.filtered_packages.clear();
        self.filtered_files.clear();
        self.filtered_groups.clear();
        self.content_tree = ChimpFolderNode::default();
    }
}

fn chimp_tree_id(kit: KitId) -> egui::Id {
    egui::Id::new(("chimp_document_tree", kit.0))
}

fn chimp_contains_query(value: &str, query: &str) -> bool {
    let needle = query.as_bytes();
    needle.is_empty()
        || value
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn chimp_tint_toward(base: Color32, accent: Color32, amount: f32) -> Color32 {
    let mix =
        |base: u8, accent: u8| (base as f32 + (accent as f32 - base as f32) * amount).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(base.r(), accent.r()),
        mix(base.g(), accent.g()),
        mix(base.b(), accent.b()),
        base.a(),
    )
}

pub(super) fn load_chimp_document(world: &World, package: &str) -> Result<ChimpDocument, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    decode_chimp_document(world, provider, bytes)
}

/// One package, decoded as far as its exports and no further.
///
/// This is all a reader needs, and it is a small fraction of what loading a
/// *document* costs: [`decode_chimp_document`] goes on to decode previews and
/// render the editor's text panes, which for a World Partition cell is 329ms of
/// work against the 1ms the exports themselves take. Reading a level is 2,334
/// cells, so the difference is a quarter of an hour against seconds.
pub(super) struct ChimpPackage {
    pub(super) header: FZenPackageHeader,
    pub(super) exports: Vec<ChimpExport>,
}

/// Read and decode a package's exports, skipping everything the editor's
/// document pane needs and a reader does not.
pub(super) fn load_chimp_package(world: &World, package: &str) -> Result<ChimpPackage, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    let (header, _, exports) = decode_chimp_exports(world, &provider, &bytes)?;
    Ok(ChimpPackage { header, exports })
}

/// The shared front half of loading a package: header, payloads, exports.
fn decode_chimp_exports(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
) -> Result<(FZenPackageHeader, Vec<Vec<u8>>, Vec<ChimpExport>), String> {
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("Could not parse {}: {error:#}", provider.entry_path))?;
    let payloads = read_payloads(&header, bytes)
        .map_err(|error| format!("Could not split {}: {error:#}", provider.entry_path))?;
    let names = header.name_map.copy_raw_names();
    let resolver = world.resolver(&header, bytes, &names);
    let bulk: Vec<(i64, i64)> = header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let context = ExportContext {
        bulk_data: &bulk,
        resolver: Some(&resolver),
    };
    let exports: Vec<ChimpExport> = header
        .export_map
        .iter()
        .zip(&payloads)
        .map(|(entry, payload)| {
            let object = header.name_map.get(entry.object_name).to_string();
            let class = world.class_key(&header, entry.class_index);
            let decoded = class
                .as_deref()
                .ok_or_else(|| "class could not be resolved".to_owned())
                .and_then(|class| {
                    read_export_in(
                        payload,
                        &names,
                        world.usmap(),
                        class,
                        entry.object_flags,
                        &context,
                    )
                    .map_err(|error| error.to_string())
                });
            ChimpExport {
                object,
                class,
                decoded,
            }
        })
        .collect();
    drop(resolver);
    Ok((header, payloads, exports))
}

fn decode_chimp_document(
    world: &World,
    provider: PackageProvider,
    bytes: Vec<u8>,
) -> Result<ChimpDocument, String> {
    let (header, payloads, exports) = decode_chimp_exports(world, &provider, &bytes)?;
    let names = header.name_map.copy_raw_names();
    let resolver = world.resolver(&header, &bytes, &names);
    let bulk: Vec<(i64, i64)> = header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let texture_previews = decode_chimp_texture_previews(
        world, &provider, &header, &payloads, &names, &resolver, &bulk, &exports,
    );
    let (mesh_kind, mesh_preview, mesh_preview_state) =
        decode_chimp_mesh_preview(world, &provider, &bytes, &header, &exports);
    let initial_view = if !texture_previews.is_empty() {
        ChimpDocumentView::Texture
    } else if mesh_kind.is_some() {
        ChimpDocumentView::Mesh
    } else {
        ChimpDocumentView::default()
    };
    let mut document = ChimpDocument {
        package: header.package_name(),
        provider,
        original: bytes,
        header,
        payloads,
        exports,
        texture_previews,
        mesh_kind,
        mesh_preview,
        mesh_preview_state,
        selected_export: 0,
        dirty: false,
        view: initial_view,
        document_text: String::new(),
        document_line_numbers: String::new(),
        document_text_dirty: true,
        metadata_text: String::new(),
        metadata_line_numbers: String::new(),
        metadata_text_dirty: true,
        header_usage: None,
        header_name_filter: String::new(),
        header_name_edit: None,
        header_import_edit: None,
        header_export_edit: None,
        header_identity_edit: None,
        header_error: None,
        referrers: ChimpReferrerState::Idle,
        orphaned: false,
    };
    refresh_chimp_document_text(&mut document);
    refresh_chimp_metadata_text(&mut document, world);
    Ok(document)
}

/// Whether an imported package is one of the mesh's materials.
///
/// Recognised by the `M_`/`MI_` prefix the game's own content uses. A mesh's
/// import list carries far more than its materials, and this is the same rule
/// the material list written into an exported mesh already relies on — so the
/// textures exported alongside a mesh belong to the materials named in it.
fn is_chimp_material_package(path: &str) -> bool {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.starts_with("MI_") || leaf.starts_with("M_")
}

fn chimp_material_names(header: &FZenPackageHeader) -> Vec<String> {
    header
        .imported_package_names
        .iter()
        .filter(|path| is_chimp_material_package(path))
        .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
        .collect()
}

fn decode_chimp_mesh_preview(
    world: &World,
    provider: &PackageProvider,
    bytes: &[u8],
    header: &FZenPackageHeader,
    exports: &[ChimpExport],
) -> (
    Option<ChimpMeshKind>,
    Option<Result<ModelPreviewData, String>>,
    ModelPreviewState,
) {
    let kind = if exports
        .iter()
        .any(|export| export.class.as_deref() == Some("SkeletalMesh"))
    {
        Some(ChimpMeshKind::Skeletal)
    } else if exports
        .iter()
        .any(|export| export.class.as_deref() == Some("StaticMesh"))
    {
        Some(ChimpMeshKind::Static)
    } else {
        None
    };
    let preview = kind.map(|kind| {
        let header_size = header.summary.header_size as usize;
        let preview = match kind {
            ChimpMeshKind::Skeletal => {
                SkeletalMesh::from_package(bytes, &header.name_map.copy_raw_names(), header_size)
                    .map(chimp_skeletal_mesh_preview)
            }
            ChimpMeshKind::Static => {
                let bulk = world
                    .archives()
                    .get(provider.container)
                    .and_then(|archive| {
                        let chunk = archive.chunk_index_for(&provider.entry_path).ok()?;
                        archive.read_bulk_for(chunk, 0).ok()
                    });
                StaticMesh::from_package_preferring_nanite(bytes, header_size, bulk.as_deref())
                    .map(chimp_static_mesh_preview)
            }
        }
        .map_err(|error| format!("Could not decode mesh geometry: {error:#}"))?;
        Ok(model_preview::standalone_mesh_preview(
            header.package_name(),
            preview,
        ))
    });
    let mut state = ModelPreviewState::default();
    if kind.is_some() {
        state.show_backfaces = true;
        state.region_selections.insert(
            "mesh".to_owned(),
            ModelRegionSelection {
                enabled: true,
                permutation: "default".to_owned(),
            },
        );
    }
    (kind, preview, state)
}

fn chimp_skeletal_mesh_preview(mesh: SkeletalMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    for section in mesh.sections {
        let index_start = section.base_index.min(preview.indices.len() as u32);
        let index_count =
            (section.num_triangles * 3).min(preview.indices.len() as u32 - index_start);
        if index_count > 0 {
            preview.batches.push(RenderModelPreviewBatch {
                region_name: "mesh".to_owned(),
                permutation_name: "default".to_owned(),
                material_index: section.material_index,
                index_start,
                index_count,
            });
        }
    }
    if preview.batches.is_empty() && !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
        });
    }
    preview
}

fn chimp_static_mesh_preview(mesh: StaticMesh) -> RenderModelPreview {
    let mut preview = chimp_mesh_preview_base(
        mesh.vertices
            .iter()
            .map(|vertex| (vertex.position, vertex.normal)),
        mesh.indices,
    );
    if !preview.indices.is_empty() {
        preview.batches.push(RenderModelPreviewBatch {
            region_name: "mesh".to_owned(),
            permutation_name: "default".to_owned(),
            material_index: 0,
            index_start: 0,
            index_count: preview.indices.len() as u32,
        });
    }
    preview
}

fn chimp_mesh_preview_base(
    vertices: impl IntoIterator<Item = ([f32; 3], [f32; 3])>,
    indices: Vec<u32>,
) -> RenderModelPreview {
    let mut preview = RenderModelPreview {
        regions: vec![RenderModelPreviewRegion {
            name: "mesh".to_owned(),
            permutations: vec!["default".to_owned()],
        }],
        indices,
        bounds_min: [f32::INFINITY; 3],
        bounds_max: [f32::NEG_INFINITY; 3],
        ..Default::default()
    };
    for (position, normal) in vertices {
        for axis in 0..3 {
            preview.bounds_min[axis] = preview.bounds_min[axis].min(position[axis]);
            preview.bounds_max[axis] = preview.bounds_max[axis].max(position[axis]);
        }
        preview
            .vertices
            .push(RenderModelPreviewVertex { position, normal });
    }
    if preview.vertices.is_empty() {
        preview.bounds_min = [-1.0; 3];
        preview.bounds_max = [1.0; 3];
    }
    preview
}

fn decode_chimp_texture_previews(
    world: &World,
    provider: &PackageProvider,
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
    names: &[String],
    resolver: &dyn blam_tags::iostore::object::archive::PackageResolver,
    bulk: &[(i64, i64)],
    exports: &[ChimpExport],
) -> Vec<ChimpTexturePreview> {
    let archive = &world.archives()[provider.container];
    let package_chunk = archive.chunk_index_for(&provider.entry_path);
    exports
        .iter()
        .enumerate()
        .filter(|(_, export)| export.class.as_deref() == Some("Texture2D"))
        .map(|(export_index, export)| {
            let decoded = (|| {
                let package_entry = header
                    .export_map
                    .get(export_index)
                    .ok_or_else(|| "Texture export is missing from the package map".to_owned())?;
                let payload = payloads
                    .get(export_index)
                    .ok_or_else(|| "Texture export payload is missing".to_owned())?;
                let export = export.decoded.as_ref().map_err(Clone::clone)?;
                let context = TailContext {
                    bulk_data: bulk,
                    origin: payload.len().saturating_sub(export.tail.len()),
                    usmap: world.usmap(),
                    resolver: Some(resolver),
                    object_flags: package_entry.object_flags,
                };
                let texture = parse_texture_chain_tail(&export.tail, names, context, true)
                    .map_err(|error| error.to_string())?;
                let package_chunk = package_chunk.as_ref().map_err(|error| error.to_string())?;
                decode_texture2d_surfaces(&texture, |bulk_index| {
                    let entry = header
                        .bulk_data
                        .get(bulk_index.max(0) as usize)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data index {bulk_index} is out of range")
                        })?;
                    let chunk = archive.read_bulk_for(*package_chunk, entry.cooked_index as u16)?;
                    let start = usize::try_from(entry.serial_offset)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data offset"))?;
                    let size = usize::try_from(entry.serial_size)
                        .map_err(|_| anyhow::anyhow!("negative bulk-data size"))?;
                    chunk
                        .get(start..start.saturating_add(size))
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            anyhow::anyhow!("bulk-data entry lies outside its sibling chunk")
                        })
                })
                .map_err(|error| error.to_string())
            })();
            let mut preview = BitmapPreviewState::default();
            // Start on the largest mip that a GPU texture can actually hold.
            // Mip 0 of a virtual texture is routinely wider than the driver's
            // limit, and an oversized upload renders as noise rather than
            // failing, so choosing here is the difference between a preview and
            // an apparently corrupt one.
            if let Ok(surfaces) = &decoded {
                preview.mip_index = first_displayable_mip(surfaces, 0);
            }
            preview.decoded = Some(
                decoded
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|surfaces| {
                        chimp_texture_mip_data(surfaces, preview.image_index, preview.mip_index)
                    }),
            );
            ChimpTexturePreview {
                export_index,
                preview,
                surfaces: decoded,
            }
        })
        .collect()
}

/// Largest mip index that is safe to upload as a GPU texture.
///
/// Cooked virtual textures reach 14336 px on their base mip, well past the
/// 8192-16384 limit typical hardware reports. Uploading past that limit does not
/// error — the texture is simply never defined, and the viewer shows garbage —
/// so the base mip is skipped rather than shown wrong. `max_side` of 0 means the
/// limit is not known yet, in which case a conservative cap is applied.
fn first_displayable_mip(surfaces: &Texture2dSurfaces, max_side: usize) -> usize {
    let limit = if max_side == 0 { 8192 } else { max_side } as u32;
    surfaces
        .layers
        .first()
        .and_then(|layer| {
            layer
                .mips
                .iter()
                .position(|mip| mip.width <= limit && mip.height <= limit)
        })
        .unwrap_or(0)
}

/// Expand one layer/mip of a decoded texture into the shared bitmap viewer's
/// model. Layer and mip counts come from the surfaces, so the viewer's existing
/// steppers work without Chimp growing a second viewer.
fn chimp_texture_mip_data(
    surfaces: &Texture2dSurfaces,
    layer_index: usize,
    mip_index: usize,
) -> Result<BitmapPreviewData, String> {
    let layer_count = surfaces.layers.len().max(1);
    let layer_index = layer_index.min(layer_count - 1);
    let layer = surfaces
        .layers
        .get(layer_index)
        .ok_or_else(|| "texture has no layers".to_owned())?;
    let mip_count = layer.mips.len().max(1);
    let mip_index = mip_index.min(mip_count - 1);
    let surface = layer
        .mips
        .get(mip_index)
        .ok_or_else(|| format!("texture layer has no mip {mip_index}"))?;
    // Not `to_rgba8`: a mixed-resolution UDIM set has no tiles at this level for
    // its lower-resolution blocks, and those regions must come from the coarser
    // level rather than show as holes.
    let rgba = surfaces
        .display_rgba8(layer_index, mip_index)
        .map_err(|error| format!("{error:#}"))?;

    let mut type_name = String::from("Texture2D");
    if surfaces.is_virtual {
        type_name.push_str(" • virtual");
    }
    if surfaces.is_udim() {
        type_name.push_str(&format!(
            " • {}x{} UDIM",
            surfaces.width_in_blocks, surfaces.height_in_blocks
        ));
    }
    if layer_count > 1 {
        type_name.push_str(&format!(" • layer {layer_index}"));
    }
    if !surface.missing_tiles.is_empty() {
        // Say so rather than let a magnified region read as a decode fault.
        type_name.push_str(" • some blocks magnified (authored lower-resolution)");
    }
    Ok(BitmapPreviewData {
        width: surface.width,
        height: surface.height,
        image_count: layer_count,
        mip_count,
        format_name: surface.pixel_format.clone(),
        type_name,
        rgba,
    })
}

fn chimp_package_type(world: &World, header: &FZenPackageHeader) -> Option<String> {
    let package_name = header.package_name();
    let package_leaf = package_name.rsplit('/').next().unwrap_or_default();
    let classify = |index: usize| {
        let entry = header.export_map.get(index)?;
        let class = world.class_key(header, entry.class_index)?;
        if class.contains('/') || class.contains('#') {
            return None;
        }
        Some(if class.contains("Blueprint") {
            "Blueprint".to_owned()
        } else {
            class
        })
    };
    let primary = header
        .export_map
        .iter()
        .enumerate()
        .find(|(_, export)| {
            export.outer_index.is_null() && header.name_map.get(export.object_name) == package_leaf
        })
        .and_then(|(index, _)| classify(index));
    primary.or_else(|| {
        let roots: Vec<usize> = header
            .export_map
            .iter()
            .enumerate()
            .filter(|(_, export)| export.outer_index.is_null())
            .map(|(index, _)| index)
            .collect();
        roots
            .iter()
            .filter_map(|index| classify(*index))
            .find(|class| class == "Blueprint")
            .or_else(|| roots.into_iter().find_map(classify))
    })
}

fn index_chimp_package_types(world: &World) -> ChimpTypeIndex {
    // Most Zen package headers fit in one 64 KiB IoStore compression block.
    // Only retry with the former 1 MiB window (and finally the whole package)
    // for the uncommon large header. This avoids decompressing sixteen blocks
    // for every package while preserving the existing fallback behavior.
    const HEADER_PREFIXES: [usize; 2] = [64 * 1024, 1024 * 1024];
    index_chimp_package_types_with_prefixes(world, &HEADER_PREFIXES)
}

fn index_chimp_package_types_with_prefixes(
    world: &World,
    header_prefixes: &[usize],
) -> ChimpTypeIndex {
    let mut package_types = Vec::with_capacity(world.packages().len());
    let mut type_counts = BTreeMap::new();
    let mut failures = 0usize;
    for package in world.packages() {
        let result = (|| {
            let provider = package.active_provider()?;
            let archive = world.archives().get(provider.container)?;
            let mut header = None;
            for &max_bytes in header_prefixes {
                let prefix = archive.read_prefix(&provider.entry_path, max_bytes).ok()?;
                if let Ok(decoded) = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&prefix),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                ) {
                    header = Some(decoded);
                    break;
                }
                if prefix.len() < max_bytes {
                    break;
                }
            }
            let header = header.or_else(|| {
                world.read_provider(provider).ok().and_then(|bytes| {
                    FZenPackageHeader::deserialize(
                        &mut Cursor::new(bytes),
                        None,
                        CE_TOC_VERSION,
                        CE_HEADER_VERSION,
                        None,
                    )
                    .ok()
                })
            })?;
            chimp_package_type(world, &header)
        })();
        if let Some(class) = &result {
            *type_counts.entry(class.clone()).or_insert(0) += 1;
        } else {
            failures += 1;
        }
        package_types.push(result);
    }
    ChimpTypeIndex {
        package_types,
        type_counts,
        failures,
    }
}

/// What a sweep for "who imports this package" found.
#[derive(Clone, Debug, Default)]
pub(super) struct ChimpReferrerScan {
    /// Packages whose import map names the target, in mount order.
    pub(super) referrers: Vec<String>,
    /// How many packages were examined.
    pub(super) scanned: usize,
    /// How many could not be read, and so could not be ruled out.
    ///
    /// Reported rather than swallowed: "nothing imports this" and "nothing I
    /// could read imports this" are different answers, and only one of them
    /// makes renaming safe.
    pub(super) unreadable: usize,
}

/// Find every mounted package whose import map names `target`.
///
/// A package's imports are the only record of the dependency — there is no
/// reverse index in the paks — so answering this at all means reading every
/// package header. That is the same sweep the type index already performs at
/// mount, and it uses the same prefix ladder: most Zen headers fit in one
/// 64 KiB compression block, so the larger reads are the exception.
fn scan_chimp_referrers(world: &World, target: &str) -> ChimpReferrerScan {
    const HEADER_PREFIXES: [usize; 2] = [64 * 1024, 1024 * 1024];
    let mut scan = ChimpReferrerScan::default();
    for package in world.packages() {
        if package.name.eq_ignore_ascii_case(target) {
            continue;
        }
        scan.scanned += 1;
        let header = (|| {
            let provider = package.active_provider()?;
            let archive = world.archives().get(provider.container)?;
            for &max_bytes in &HEADER_PREFIXES {
                let prefix = archive.read_prefix(&provider.entry_path, max_bytes).ok()?;
                if let Ok(decoded) = FZenPackageHeader::deserialize(
                    &mut Cursor::new(&prefix),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                ) {
                    return Some(decoded);
                }
                if prefix.len() < max_bytes {
                    break;
                }
            }
            world.read_provider(provider).ok().and_then(|bytes| {
                FZenPackageHeader::deserialize(
                    &mut Cursor::new(bytes),
                    None,
                    CE_TOC_VERSION,
                    CE_HEADER_VERSION,
                    None,
                )
                .ok()
            })
        })();
        let Some(header) = header else {
            scan.unreadable += 1;
            continue;
        };
        if header
            .imported_package_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(target))
        {
            scan.referrers.push(package.name.clone());
        }
    }
    scan
}

fn rebuild_chimp_document(
    world: &World,
    document: &ChimpDocument,
) -> Result<(Vec<u8>, blam_tags::iostore::container::header::StoreEntry), String> {
    // A remount can leave a document whose package is no longer provided by any
    // mounted container. Its `provider` addresses a container positionally, so
    // writing through it would land the bytes somewhere they never came from.
    if document.orphaned {
        return Err(format!(
            "{} is no longer in the mounted containers, so it cannot be written back. Its bytes \
             are still here and can be extracted.",
            document.package
        ));
    }
    // Before anything is serialized: a header whose references point outside the
    // tables they name would otherwise be caught by a panic in the name map, or
    // not at all.
    validate_chimp_header(document)?;
    let names = document.header.name_map.copy_raw_names();
    let resolver = world.resolver(&document.header, &document.original, &names);
    let mut payloads = document.payloads.clone();
    for (index, export) in document.exports.iter().enumerate() {
        let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded) else {
            continue;
        };
        if let ExportBlock::Reflected(block) = &decoded.block {
            validate_chimp_property_block(class, block, world.usmap()).map_err(|error| {
                format!(
                    "Could not validate {} export {}: {error}",
                    document.package, export.object
                )
            })?;
        }
        payloads[index] =
            write_export_in(class, decoded, world.usmap(), Some(&resolver)).map_err(|error| {
                format!(
                    "Could not serialize {} export {}: {error:#}",
                    document.package, export.object
                )
            })?;
    }
    write_package(&document.header, &payloads, CE_HEADER_VERSION)
        .map_err(|error| format!("Could not rebuild {}: {error:#}", document.package))
}

/// Structural checks over the package header itself, before it is written.
///
/// The property validator beside this one answers "is this value legal for its
/// schema slot". This answers the questions that only arise once the *header*
/// can be edited: whether every reference still lands inside the table it points
/// at, and whether the package still is what the container says it is.
fn validate_chimp_header(document: &ChimpDocument) -> Result<(), String> {
    validate_chimp_header_parts(
        &document.package,
        &document.header,
        document.payloads.len(),
        &document.exports,
    )
}

/// [`validate_chimp_header`] over the pieces it actually reads, so the checks
/// can be exercised without a mounted package behind them.
fn validate_chimp_header_parts(
    package: &str,
    header: &FZenPackageHeader,
    payload_count: usize,
    exports: &[ChimpExport],
) -> Result<(), String> {
    let names = &header.name_map;

    if payload_count != header.export_map.len() {
        return Err(format!(
            "{package} has {payload_count} export payloads for {} export map entries",
            header.export_map.len()
        ));
    }

    // The most important check here. `FNameMap::get` indexes its slice directly,
    // so an out-of-range `FMappedName` is not a validation failure but a panic —
    // it would take the whole application down on the next draw rather than
    // report anything.
    if names.try_get(header.summary.name).is_none() {
        return Err(format!(
            "{package}: the package name points outside the name map"
        ));
    }
    for (index, export) in header.export_map.iter().enumerate() {
        if names.try_get(export.object_name).is_none() {
            return Err(format!(
                "{package}: export {index}'s object name points outside the name map"
            ));
        }
    }
    for (index, cell) in header.cell_export_map.iter().enumerate() {
        if names.try_get(cell.cpp_class_info).is_none() {
            return Err(format!(
                "{package}: cell export {index}'s class name points outside the name map"
            ));
        }
    }

    // The import map has to remain resolvable as slots, because that is the form
    // every object property addresses it in.
    let slots = read_import_slots(header)
        .map_err(|error| format!("{package}: import map is not resolvable: {error:#}"))?;
    for (index, export) in exports.iter().enumerate() {
        let Ok(decoded) = &export.decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &decoded.block else {
            continue;
        };
        if let Some(bad) = first_unresolvable_object_reference(block, slots.len()) {
            return Err(format!(
                "{package}: export {index} references import slot {bad}, but the import map has {} slots",
                slots.len()
            ));
        }
    }

    Ok(())
}

/// The first negative `FPackageIndex` in `block` that names a slot past the end
/// of the import map, if any.
fn first_unresolvable_object_reference(block: &PropertyBlock, slots: usize) -> Option<usize> {
    fn check(value: &PropValue, slots: usize) -> Option<usize> {
        match value.unwrapped() {
            PropValue::Object(index) if *index < 0 => {
                let slot = import_slot_of(*index)?;
                (slot >= slots).then_some(slot)
            }
            PropValue::Array(items) | PropValue::Set(items) => {
                items.iter().find_map(|item| check(item, slots))
            }
            PropValue::Map(pairs) => pairs
                .iter()
                .find_map(|(key, value)| check(key, slots).or_else(|| check(value, slots))),
            PropValue::Struct(nested) => nested.iter().find_map(|(_, value)| check(value, slots)),
            _ => None,
        }
    }
    block.iter().find_map(|(_, value)| check(value, slots))
}

/// Rewrite one name-map entry, and everything that displays it.
///
/// An `FName` is stored as an index, so the rename retargets every reference on
/// disk by itself. What it does not do is update the *resolved* text a decoded
/// value carries — and that is not cosmetic: interning is by string, so editing
/// one of those fields afterwards would fork a fresh entry rather than follow the
/// rename. Refreshing them is part of the operation, not a redraw.
fn apply_chimp_name_rename(
    document: &mut ChimpDocument,
    index: usize,
    text: &str,
) -> Result<(), String> {
    let text = text.trim();

    // The package's own name is its identity, and the container addresses the
    // chunk that serves it by a hash of that string. Renaming it here would
    // leave the package claiming to be something no chunk id matches — which the
    // in-place-rename work exists to do properly, moving the header, the chunk
    // id and the store entry together.
    if index == document.header.summary.name.index() as usize {
        return Err(
            "This entry is the package's own name. Renaming a package has to move its chunk id \
             and container-header entry with it, so it is a rename of the package rather than an \
             edit of this table."
                .to_owned(),
        );
    }

    document
        .header
        .name_map
        .rename(index, text)
        .map_err(|error| format!("{error:#}"))?;

    // Every export's cached display name, as `decode_chimp_exports` derived it.
    for (export, entry) in document
        .exports
        .iter_mut()
        .zip(document.header.export_map.iter())
    {
        if entry.object_name.index() as usize == index {
            export.object = document
                .header
                .name_map
                .try_get(entry.object_name)
                .map(|name| name.to_string())
                .unwrap_or_default();
        }
    }

    // Then the resolved text inside decoded property values. The composition
    // has to match the reader's: a non-zero number renders as `_{number - 1}`.
    let base = document
        .header
        .name_map
        .names()
        .get(index)
        .cloned()
        .unwrap_or_default();
    for export in &mut document.exports {
        let Ok(decoded) = &mut export.decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &mut decoded.block else {
            continue;
        };
        block.visit_names_mut(&mut |name| {
            if name.index as usize != index {
                return;
            }
            let text = if name.number != 0 {
                format!("{base}_{}", name.number - 1)
            } else {
                base.clone()
            };
            *name = FName::new(name.index, name.number, text);
        });
    }

    Ok(())
}

/// Replace, or append, one import slot and rebuild the four arrays it feeds.
///
/// `write_import_slots` rederives `import_map`, `imported_packages`,
/// `imported_package_names` and `imported_public_export_hashes` from the slot
/// list, preserving slot order — which is what keeps every `FPackageIndex` in
/// every export payload pointing where it did. Appending is safe for the same
/// reason: it cannot renumber an existing slot.
fn apply_chimp_import_slot(
    document: &mut ChimpDocument,
    index: usize,
    slot: ImportSlot,
) -> Result<(), String> {
    let mut slots = read_import_slots(&document.header)
        .map_err(|error| format!("Import map is not resolvable: {error:#}"))?;
    if index > slots.len() {
        return Err(format!(
            "Import slot {index} is past the end of a {}-slot import map",
            slots.len()
        ));
    }
    if index == slots.len() {
        slots.push(slot);
    } else {
        slots[index] = slot;
    }
    write_import_slots(&mut document.header, &slots)
        .map_err(|error| format!("Could not rewrite the import map: {error:#}"))
}

/// Write a candidate header and read it straight back, reporting anything that
/// did not survive the trip.
///
/// This is the parity policy's "reopen coverage" applied per edit rather than
/// only in tests, and it exists because the version fields are format selectors:
/// lowering `file_version_ue5` stops the bulk-data map being written at all, and
/// nothing about the edit itself would say so. One rebuild is cheap — the
/// recovery checkpoint already performs one on every commit.
fn chimp_header_survives_reopen(
    header: &FZenPackageHeader,
    payloads: &[Vec<u8>],
) -> Result<(), String> {
    let (bytes, _) = write_package(header, payloads, CE_HEADER_VERSION)
        .map_err(|error| format!("The edited header could not be written: {error:#}"))?;
    let reopened = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("The edited header did not reopen: {error:#}"))?;

    let mut drift: Vec<String> = Vec::new();
    let mut check = |field: &str, intended: String, got: String| {
        if intended != got {
            drift.push(format!("{field} was written as {intended} and read back as {got}"));
        }
    };
    check(
        "package flags",
        format!("0x{:08X}", header.summary.package_flags),
        format!("0x{:08X}", reopened.summary.package_flags),
    );
    check(
        "unversioned",
        header.is_unversioned.to_string(),
        reopened.is_unversioned.to_string(),
    );
    // An unversioned package carries no versioning block at all — `serialize`
    // omits it and the reader synthesises one heuristically — so comparing those
    // fields would report drift on every package Campaign Evolved ships. They
    // are only real once the block is actually written.
    if !header.is_unversioned {
        check(
            "Zen version",
            format!("{:?}", header.versioning_info.zen_version),
            format!("{:?}", reopened.versioning_info.zen_version),
        );
        check(
            "UE4 file version",
            header
                .versioning_info
                .package_file_version
                .file_version_ue4
                .to_string(),
            reopened
                .versioning_info
                .package_file_version
                .file_version_ue4
                .to_string(),
        );
        check(
            "UE5 file version",
            header
                .versioning_info
                .package_file_version
                .file_version_ue5
                .to_string(),
            reopened
                .versioning_info
                .package_file_version
                .file_version_ue5
                .to_string(),
        );
        check(
            "licensee version",
            header.versioning_info.licensee_version.to_string(),
            reopened.versioning_info.licensee_version.to_string(),
        );
    }
    // Counts rather than contents: these are the tables a version change can
    // quietly stop writing.
    check(
        "bulk data entries",
        header.bulk_data.len().to_string(),
        reopened.bulk_data.len().to_string(),
    );
    check(
        "name map entries",
        header.name_map.len().to_string(),
        reopened.name_map.len().to_string(),
    );
    check(
        "import slots",
        header.import_map.len().to_string(),
        reopened.import_map.len().to_string(),
    );
    check(
        "export map entries",
        header.export_map.len().to_string(),
        reopened.export_map.len().to_string(),
    );

    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "The package does not read back as it was written, so the change was not applied:\n  {}",
            drift.join("\n  ")
        ))
    }
}

/// Apply a drafted identity/versioning change, but only if it survives a write
/// and reopen.
fn apply_chimp_identity_edit(
    document: &mut ChimpDocument,
    edit: &ChimpIdentityEdit,
) -> Result<(), String> {
    let text = edit.package_flags.trim();
    let text = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    let package_flags = u32::from_str_radix(text, 16)
        .map_err(|_| format!("{:?} is not a 32-bit hex value", edit.package_flags))?;

    let mut candidate = document.header.clone();
    candidate.summary.package_flags = package_flags;
    candidate.is_unversioned = edit.is_unversioned;
    // Kept in step because `serialize` derives it rather than reading it, and a
    // header that disagreed with itself would reopen as the other one.
    candidate.summary.has_versioning_info = u32::from(!edit.is_unversioned);
    candidate.versioning_info.zen_version = edit.zen_version;
    candidate.versioning_info.licensee_version = edit.licensee_version;
    candidate.versioning_info.package_file_version.file_version_ue4 = edit.file_version_ue4;
    candidate.versioning_info.package_file_version.file_version_ue5 = edit.file_version_ue5;

    chimp_header_survives_reopen(&candidate, &document.payloads)?;
    document.header = candidate;
    Ok(())
}

/// Apply a drafted export-map entry.
///
/// `object_flags` is the reason this is not three independent setters. It is not
/// inert metadata: it is fed to `read_export_in`, and `RF_CLASS_DEFAULT_OBJECT`
/// selects a different native tail branch — so changing it changes what the same
/// bytes *mean*. The export is therefore round-tripped through the new flags and
/// the edit is refused if it no longer decodes, rather than left to fail at save
/// time with the old decode still in hand.
fn apply_chimp_export_edit(
    world: &World,
    document: &mut ChimpDocument,
    edit: &ChimpExportEdit,
) -> Result<(), String> {
    let index = edit.index;
    if index >= document.header.export_map.len() {
        return Err(format!("Export {index} is no longer in this package"));
    }
    if edit.object_name.trim().is_empty() {
        return Err("An export needs an object name".to_owned());
    }

    // Re-decode first, so a refusal leaves the document untouched.
    if edit.object_flags != document.header.export_map[index].object_flags
        && let Some(redecoded) = chimp_redecode_export(world, document, index, edit.object_flags)?
    {
        document.exports[index].decoded = Ok(redecoded);
    }
    apply_chimp_export_metadata(document, edit);
    Ok(())
}

/// The half of an export edit that needs nothing but the document: the name,
/// the filter flags, and the optional hash recompute.
///
/// Split out because `object_flags` is the only field whose change has to be
/// proven against a decoder first, and everything else would otherwise be
/// untestable without a mounted world behind it.
fn apply_chimp_export_metadata(document: &mut ChimpDocument, edit: &ChimpExportEdit) {
    let index = edit.index;
    let object_name = edit.object_name.trim();
    let entry = &mut document.header.export_map[index];
    entry.object_flags = edit.object_flags;
    entry.filter_flags = edit.filter_flags;
    // `store` interns rather than renames: an existing entry is reused, a new
    // one appended, and the trailing `_N` lands in the reference's number field
    // where it belongs.
    entry.object_name = document.header.name_map.store(object_name);
    if edit.recompute_hash {
        let resolved = document
            .header
            .name_map
            .try_get(entry.object_name)
            .map(|name| name.to_string())
            .unwrap_or_else(|| object_name.to_owned());
        document.header.export_map[index].public_export_hash = public_export_hash(&resolved);
    }

    document.exports[index].object = document
        .header
        .name_map
        .try_get(document.header.export_map[index].object_name)
        .map(|name| name.to_string())
        .unwrap_or_default();
}

/// Round-trip one export through `object_flags` to see whether it still decodes.
///
/// Serializing the *current* decoded value first is what keeps unsaved property
/// edits: re-reading `payloads[index]` would decode the bytes as they were on
/// disk and silently discard them. `Ok(None)` means there was nothing decoded to
/// re-interpret, so the flag is the only thing changing.
fn chimp_redecode_export(
    world: &World,
    document: &ChimpDocument,
    index: usize,
    object_flags: u32,
) -> Result<Option<Export>, String> {
    let export = &document.exports[index];
    let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded) else {
        return Ok(None);
    };
    let names = document.header.name_map.copy_raw_names();
    let resolver = world.resolver(&document.header, &document.original, &names);
    let payload = write_export_in(class, decoded, world.usmap(), Some(&resolver))
        .map_err(|error| format!("Could not re-serialize export {index}: {error:#}"))?;
    let bulk: Vec<(i64, i64)> = document
        .header
        .bulk_data
        .iter()
        .map(|entry| (entry.serial_offset, entry.serial_size))
        .collect();
    let context = ExportContext {
        bulk_data: &bulk,
        resolver: Some(&resolver),
    };
    read_export_in(
        &payload,
        &names,
        world.usmap(),
        class,
        object_flags,
        &context,
    )
    .map(Some)
    .map_err(|error| {
        format!(
            "Export {index} no longer decodes with object flags 0x{object_flags:08X}: {error:#}"
        )
    })
}

/// The public exports of a mounted package, as `(object name, hash)`.
///
/// `public_export_hash` is one-way, so a hash cannot be turned back into the
/// name it came from. This reads the package that actually holds the export and
/// matches on the hash — the difference between a usable picker and a raw 64-bit
/// field.
fn chimp_public_exports_of(world: &World, package: &str) -> Result<Vec<(String, u64)>, String> {
    let record = world
        .package(package)
        .ok_or_else(|| format!("{package} is not mounted"))?;
    let provider = record
        .active_provider()
        .cloned()
        .ok_or_else(|| format!("{package} has no active provider"))?;
    let bytes = world
        .read_provider(&provider)
        .map_err(|error| error.to_string())?;
    let header = FZenPackageHeader::deserialize(
        &mut Cursor::new(&bytes),
        None,
        CE_TOC_VERSION,
        CE_HEADER_VERSION,
        None,
    )
    .map_err(|error| format!("{package} did not parse: {error:#}"))?;
    let mut exports: Vec<(String, u64)> = header
        .export_map
        .iter()
        .filter(|export| export.public_export_hash != 0)
        .filter_map(|export| {
            header
                .name_map
                .try_get(export.object_name)
                .map(|name| (name.to_string(), export.public_export_hash))
        })
        .collect();
    exports.sort();
    exports.dedup();
    Ok(exports)
}

/// Export object names this entry backs whose `public_export_hash` would no
/// longer match, so the view can say so before the rename happens.
///
/// The hash is how *other* packages address an export. Renaming the object it
/// names does not update theirs, so the two drift apart — recoverable, but only
/// if someone knows it happened.
fn chimp_export_hash_desyncs(
    header: &FZenPackageHeader,
    index: usize,
    text: &str,
) -> Vec<usize> {
    header
        .export_map
        .iter()
        .enumerate()
        .filter(|(_, export)| export.object_name.index() as usize == index)
        .filter(|(_, export)| {
            let resolved = if export.object_name.number != 0 {
                format!("{text}_{}", export.object_name.number - 1)
            } else {
                text.to_owned()
            };
            export.public_export_hash != public_export_hash(&resolved)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Recompute who references each name-map entry and each import slot.
fn refresh_chimp_header_usage(document: &mut ChimpDocument) {
    let mut names = vec![ChimpNameUsage::default(); document.header.name_map.len()];
    let mut record = |mapped: FMappedName, site: &str| {
        if let Some(usage) = names.get_mut(mapped.index() as usize) {
            usage.record(site);
        }
    };

    record(document.header.summary.name, "package name");
    for (index, export) in document.header.export_map.iter().enumerate() {
        record(export.object_name, &format!("export {index} object name"));
    }
    for (index, cell) in document.header.cell_export_map.iter().enumerate() {
        record(cell.cpp_class_info, &format!("cell export {index} class"));
    }
    if let Some(usage) = names.get_mut(document.header.summary.name.index() as usize) {
        usage.is_package_identity = true;
    }

    // Property values. `visit_names_mut` is the crate's only traversal over
    // every shape a name can hide behind, and nothing is mutated here — taking
    // it by `&mut` costs nothing and avoids a second copy of a match whose whole
    // value is that it is exhaustive.
    for index in 0..document.exports.len() {
        let Ok(decoded) = &mut document.exports[index].decoded else {
            continue;
        };
        let ExportBlock::Reflected(block) = &mut decoded.block else {
            continue;
        };
        let site = format!("export {index} properties");
        block.visit_names_mut(&mut |name| {
            if let Some(usage) = names.get_mut(name.index as usize) {
                usage.record(&site);
            }
        });
    }

    // Import slots, by the `FPackageIndex` an object property would carry.
    let slot_count = document.header.import_map.len();
    let mut import_references = vec![0usize; slot_count];
    for slot in 0..slot_count {
        let package_index = import_package_index(slot);
        for export in &document.exports {
            let Ok(decoded) = &export.decoded else {
                continue;
            };
            let ExportBlock::Reflected(block) = &decoded.block else {
                continue;
            };
            import_references[slot] += count_object_references(block, package_index);
        }
    }

    document.header_usage = Some(ChimpHeaderUsage {
        names,
        import_references,
    });
}

fn validate_chimp_property_block(
    class: &str,
    block: &PropertyBlock,
    usmap: &Usmap,
) -> Result<(), String> {
    for entry in &block.entries {
        let Some(slot) = entry.slot else {
            continue;
        };
        let ty = property_type_for_slot(class, slot, usmap).map_err(|error| error.to_string())?;
        validate_value_for_type(&ty, &entry.value)
            .map_err(|error| format!("{}: {error}", entry.name))?;
        if let (PropertyType::Struct(nested), PropValue::Struct(value)) = (&ty, &entry.value) {
            validate_chimp_property_block(nested, value, usmap)?;
        }
    }
    Ok(())
}

fn load_chimp_usmap(path: Option<&Path>) -> Result<Usmap, String> {
    let Some(path) = path else {
        return Usmap::meteorite()
            .map_err(|error| format!("Could not parse bundled Campaign Evolved USMAP: {error:#}"));
    };
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read USMAP {}: {error}", path.display()))?;
    Usmap::parse(&bytes)
        .map_err(|error| format!("Could not parse USMAP {}: {error:#}", path.display()))
}

impl Baboon {
    pub(super) fn apply_chimp_usmap_path(&mut self, path: Option<PathBuf>, ctx: egui::Context) {
        if let Err(error) = load_chimp_usmap(path.as_deref()) {
            self.status = error;
            return;
        }
        if self
            .kits
            .iter()
            .any(|kit| kit.chimp.documents.values().any(|document| document.dirty))
        {
            self.status =
                "Build or discard modified Chimp packages before changing the USMAP.".to_owned();
            return;
        }

        self.chimp_usmap_path = path;
        self.chimp_usmap_path_input = self
            .chimp_usmap_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let remount: Vec<usize> = self
            .kits
            .iter()
            .enumerate()
            .filter_map(|(index, kit)| {
                matches!(
                    kit.source.as_ref().map(|source| &source.source),
                    Some(TagSource::IoStoreContainerSet { .. })
                )
                .then_some(index)
            })
            .collect();
        for &index in &remount {
            self.kits[index].chimp = ChimpState::default();
            self.begin_chimp_mount(index, ctx.clone());
        }
        self.status = match &self.chimp_usmap_path {
            Some(path) if remount.is_empty() => {
                format!("Chimp USMAP set to {}", path.display())
            }
            Some(path) => format!("Chimp USMAP set to {}; remounting Chimp", path.display()),
            None if remount.is_empty() => {
                "Chimp will use the bundled Campaign Evolved USMAP".to_owned()
            }
            None => "Using the bundled Campaign Evolved USMAP; remounting Chimp".to_owned(),
        };
    }

    pub(super) fn commit_chimp_usmap_path_input(&mut self, ctx: egui::Context) {
        let trimmed = self.chimp_usmap_path_input.trim();
        let path = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
        self.apply_chimp_usmap_path(path, ctx);
    }

    pub(super) fn choose_chimp_usmap_path(&mut self, ctx: egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Select Chimp USMAP")
            .add_filter("Unreal mappings", &["usmap"]);
        if let Some(directory) = self
            .chimp_usmap_path
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|path| path.is_dir())
        {
            dialog = dialog.set_directory(directory);
        }
        if let Some(path) = dialog.pick_file() {
            self.apply_chimp_usmap_path(Some(path), ctx);
        }
    }

    /// What the Unreal package workspace is busy with, for a message that has
    /// to tell the user why a container cannot be replaced. Deliberately
    /// specific: "the workspace is open" is not something anyone can act on,
    /// while "still indexing package types" is.
    pub(in crate::app) fn chimp_activity(&self, kit_index: usize) -> String {
        let Some(kit) = self.kits.get(kit_index) else {
            return "mounted".to_owned();
        };
        if matches!(kit.chimp.mount, ChimpMount::Loading) {
            return "still mounting".to_owned();
        }
        if kit.chimp.type_indexing {
            return "indexing package types".to_owned();
        }
        if !kit.chimp.loading_packages.is_empty() {
            return "loading a package".to_owned();
        }
        if self.chimp_level_job.is_some() {
            return "exporting a level".to_owned();
        }
        "mounted".to_owned()
    }

    pub(super) fn begin_chimp_mount(&mut self, kit_index: usize, ctx: egui::Context) {
        if !self.enable_chimp {
            return;
        }
        let Some(source) = self.kits.get(kit_index).and_then(|kit| kit.source.as_ref()) else {
            return;
        };
        let TagSource::IoStoreContainerSet { root, .. } = &source.source else {
            return;
        };
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let root = root.clone();
        let usmap_path = self.chimp_usmap_path.clone();
        self.kits[kit_index].chimp.mount = ChimpMount::Loading;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = (|| {
                let usmap = load_chimp_usmap(usmap_path.as_deref())?;
                // Keep startup to container discovery + the lightweight package
                // index. Generated Blueprint schema recovery is intentionally
                // lazy/future work; doing the whole corpus here would leave the
                // workspace saying "loading" while reading every package.
                let world = World::open(&root, usmap).map_err(|error| error.to_string())?;
                Ok(Arc::new(world))
            })();
            let mounted = result.as_ref().ok().cloned();
            let _ = tx.send(WorkerMessage::ChimpMounted { stamp, result });
            if let Some(world) = mounted {
                let index = index_chimp_package_types(&world);
                let _ = tx.send(WorkerMessage::ChimpTypesIndexed { stamp, index });
            }
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_chimp_mounted(
        &mut self,
        stamp: KitStamp,
        result: Result<Arc<World>, String>,
        ctx: egui::Context,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[index].chimp.reset_filter();
        match result {
            Ok(world) => {
                let packages = world.packages().len();
                let files = world.pak_files().len();
                self.kits[index].chimp.mount = ChimpMount::Ready(world.clone());
                self.kits[index].chimp.type_indexing = true;
                self.kits[index].chimp.package_types.clear();
                self.status =
                    format!("Chimp indexed {packages} Unreal packages and {files} pak files");
                self.reconcile_chimp_providers(index, &world);
                self.restore_chimp_recovery(index, &world);
                self.finish_pending_chimp_session_restore(index, ctx);
            }
            Err(error) => {
                self.kits[index].chimp.mount = ChimpMount::Failed(error.clone());
                self.status = format!("Chimp could not open: {error}");
            }
        }
        false
    }

    /// Re-resolve every open document's provider against the world that just
    /// mounted.
    ///
    /// A `PackageProvider` addresses its container *positionally*, and a remount
    /// rebuilds that list — so after one, an open document's provider names
    /// whatever now sits at that index. It is read by `rebuild_chimp_document`
    /// and by both save paths, which is how an edit could be written into a
    /// container it never came from.
    ///
    /// This is not new to any one feature: every remount already has the
    /// problem — changing the USMAP, overwriting source paks, and the write
    /// lease's own remount all leave the providers behind. The panes keep
    /// drawing either way, because a `ChimpDocument` holds its own bytes.
    fn reconcile_chimp_providers(&mut self, kit_index: usize, world: &World) {
        let mut orphaned = Vec::new();
        for (package, document) in &mut self.kits[kit_index].chimp.documents {
            match world
                .package(package)
                .and_then(|record| record.active_provider())
            {
                Some(provider) => {
                    document.provider = provider.clone();
                    document.orphaned = false;
                }
                // Left addressing its old container would be worse than saying
                // so: the document still holds its bytes, and extraction still
                // works, but nothing may be written back through a provider
                // that no longer describes anything.
                None => {
                    document.orphaned = true;
                    orphaned.push(package.clone());
                }
            }
        }
        if !orphaned.is_empty() {
            orphaned.sort();
            self.status = format!(
                "{} open Chimp package(s) are no longer in the mounted containers: {}",
                orphaned.len(),
                orphaned.join(", ")
            );
        }
    }

    fn finish_pending_chimp_session_restore(&mut self, kit_index: usize, ctx: egui::Context) {
        let packages = std::mem::take(&mut self.kits[kit_index].pending_restore_chimp_packages);
        if packages.is_empty() {
            self.kits[kit_index].pending_restore_active_chimp_package = None;
            return;
        }
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let mut queued = 0usize;
        let mut missing = 0usize;
        for package in packages {
            if world.package(&package).is_none() {
                missing += 1;
                continue;
            }
            if self.kits[kit_index].documents_contains_chimp(&package) {
                let kit = self.kits[kit_index].id;
                self.kits[kit_index].chimp.open_document_pane(kit, &package);
            } else {
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            queued += 1;
        }
        if self.kits[kit_index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[kit_index]
                .pending_restore_active_chimp_package
                .take()
            && self.kits[kit_index].documents_contains_chimp(&active)
        {
            let kit = self.kits[kit_index].id;
            self.kits[kit_index].chimp.selected_package = Some(active.clone());
            self.kits[kit_index].chimp.open_document_pane(kit, &active);
        }
        if queued > 0 || missing > 0 {
            self.status = match (queued, missing) {
                (queued, 0) => format!("Reopening {queued} Chimp package(s)"),
                (0, missing) => format!("Could not find {missing} saved Chimp package(s)"),
                (queued, missing) => format!(
                    "Reopening {queued} Chimp package(s); {missing} saved package(s) are missing"
                ),
            };
        }
    }

    pub(super) fn handle_chimp_types_indexed(
        &mut self,
        stamp: KitStamp,
        type_index: ChimpTypeIndex,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let classified = type_index
            .package_types
            .len()
            .saturating_sub(type_index.failures);
        let kinds = type_index.type_counts.len();
        let chimp = &mut self.kits[index].chimp;
        chimp.package_types = type_index.package_types;
        chimp.type_indexing = false;
        chimp.reset_filter();
        self.status =
            format!("Chimp classified {classified} packages into {kinds} Unreal file types");
        false
    }

    fn chimp_recovery_dir(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        let digest = Sha256::digest(root.to_string_lossy().as_bytes());
        let key: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(crate::storage::data_path(&format!("chimp-recovery-{key}")))
    }

    fn load_chimp_recovery_manifest(
        &self,
        kit_index: usize,
    ) -> Option<(PathBuf, ChimpRecoveryManifest)> {
        let directory = self.chimp_recovery_dir(kit_index)?;
        let text = fs::read_to_string(directory.join("manifest.json")).ok()?;
        let manifest = serde_json::from_str(&text).ok()?;
        Some((directory, manifest))
    }

    fn restore_chimp_recovery(&mut self, kit_index: usize, world: &Arc<World>) {
        let Some((directory, manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return;
        };
        let expected_source = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        if manifest.source != expected_source {
            return;
        }
        let mut restored = 0usize;
        for (package, filename) in manifest.packages {
            let Some(provider) = world
                .package(&package)
                .and_then(|record| record.active_provider())
                .cloned()
            else {
                continue;
            };
            let Ok(bytes) = fs::read(directory.join(filename)) else {
                continue;
            };
            let Ok(mut document) = decode_chimp_document(world, provider.clone(), bytes) else {
                continue;
            };
            // The recovery file contains the edited view of the package. Keep
            // the mounted source bytes as the discard baseline so a restored
            // edit can still be returned to the actual shipped package.
            let Ok(source_bytes) = world.read_provider(&provider) else {
                continue;
            };
            document.original = source_bytes;
            document.dirty = true;
            self.kits[kit_index]
                .chimp
                .documents
                .insert(package.clone(), document);
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            restored += 1;
        }
        if restored > 0 {
            self.status = format!("Chimp recovered {restored} unsaved package edit(s)");
        }
    }

    fn checkpoint_chimp_document(&mut self, kit_index: usize, package: &str) {
        let Some(directory) = self.chimp_recovery_dir(kit_index) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let Ok((bytes, _)) = rebuild_chimp_document(world, document) else {
            return;
        };
        if fs::create_dir_all(&directory).is_err() {
            return;
        }
        let digest = Sha256::digest(package.as_bytes());
        let filename = format!(
            "{}.uasset",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        if fs::write(directory.join(&filename), bytes).is_err() {
            return;
        }
        let source = self.kits[kit_index]
            .source
            .as_ref()
            .map(|source| source.source.root_path().display().to_string())
            .unwrap_or_default();
        let mut manifest = self
            .load_chimp_recovery_manifest(kit_index)
            .map(|(_, manifest)| manifest)
            .unwrap_or_default();
        manifest.source = source;
        manifest.packages.insert(package.to_owned(), filename);
        if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
            let _ = fs::write(directory.join("manifest.json"), bytes);
        }
    }

    fn clear_chimp_recovery_packages(
        &self,
        kit_index: usize,
        packages: &[String],
    ) -> Result<(), String> {
        let Some((directory, mut manifest)) = self.load_chimp_recovery_manifest(kit_index) else {
            return Ok(());
        };
        let mut removed_files = Vec::new();
        for package in packages {
            if let Some(filename) = manifest.packages.remove(package) {
                removed_files.push(filename);
            }
        }
        if manifest.packages.is_empty() {
            match fs::remove_file(directory.join("manifest.json")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "Could not remove Chimp recovery manifest {}: {error}",
                        directory.display()
                    ));
                }
            }
        } else {
            let bytes = serde_json::to_vec_pretty(&manifest)
                .map_err(|error| format!("Could not encode Chimp recovery manifest: {error}"))?;
            fs::write(directory.join("manifest.json"), bytes).map_err(|error| {
                format!(
                    "Could not update Chimp recovery manifest {}: {error}",
                    directory.display()
                )
            })?;
        }
        // The manifest is the recovery authority. Once it no longer names the
        // packages, stale payload cleanup cannot make discarded edits return.
        for filename in removed_files {
            let _ = fs::remove_file(directory.join(filename));
        }
        if manifest.packages.is_empty() {
            let _ = fs::remove_dir(&directory);
        }
        Ok(())
    }

    pub(super) fn chimp_dirty_packages(&self, kit_index: usize) -> Vec<String> {
        sorted_unique_dirty_chimp_keys(
            self.kits[kit_index]
                .chimp
                .documents
                .iter()
                .map(|(key, document)| (key.as_str(), document.dirty)),
        )
    }

    pub(super) fn open_chimp_discard_prompt(
        &mut self,
        kit_index: usize,
        packages: Vec<String>,
        pending_action: Option<PendingCloseAction>,
        error: Option<String>,
    ) {
        if packages.is_empty() {
            self.status = "Chimp has no modified packages".to_owned();
            return;
        }
        self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
            kit: self.kits[kit_index].id,
            packages,
            pending_action,
            error,
        });
    }

    fn discard_chimp_packages(
        &mut self,
        kit_index: usize,
        packages: &[String],
    ) -> Result<usize, String> {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return Err(
                "Chimp is not mounted; the original package data is unavailable".to_owned(),
            );
        };
        let world = world.clone();
        let mut restored = Vec::new();
        for package in packages {
            let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
                continue;
            };
            if !document.dirty {
                continue;
            }
            let selected_export = document.selected_export;
            let view = document.view;
            let mut replacement = load_chimp_document(&world, package)
                .map_err(|error| format!("Could not restore {package}: {error}"))?;
            replacement.selected_export =
                selected_export.min(replacement.exports.len().saturating_sub(1));
            replacement.view = view;
            restored.push((package.clone(), replacement));
        }

        self.clear_chimp_recovery_packages(kit_index, packages)?;
        let restored_count = restored.len();
        for (package, document) in restored {
            self.kits[kit_index]
                .chimp
                .documents
                .insert(package, document);
        }
        Ok(restored_count)
    }

    fn begin_chimp_open_package(&mut self, kit_index: usize, package: String, ctx: egui::Context) {
        if self.kits[kit_index].documents_contains_chimp(&package) {
            let kit_id = self.kits[kit_index].id;
            self.kits[kit_index]
                .chimp
                .open_document_pane(kit_id, &package);
            return;
        }
        self.kits[kit_index].chimp.selected_package = Some(package.clone());
        if !self.kits[kit_index]
            .chimp
            .loading_packages
            .insert(package.clone())
        {
            return;
        }
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = load_chimp_document(&world, &package);
            let _ = tx.send(WorkerMessage::ChimpPackageLoaded {
                stamp,
                package,
                result,
            });
            ctx.request_repaint();
        });
    }

    /// Start a sweep for the packages that import `package`.
    ///
    /// On a worker because there is no reverse index in the paks: the only way
    /// to answer it is to read every mounted header, which is the same cost as
    /// the type index at mount and far too much for a draw.
    fn begin_chimp_referrer_scan(&mut self, kit_index: usize, package: String, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) else {
            return;
        };
        if matches!(document.referrers, ChimpReferrerState::Scanning) {
            return;
        }
        document.referrers = ChimpReferrerState::Scanning;
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let scan = scan_chimp_referrers(&world, &package);
            let _ = tx.send(WorkerMessage::ChimpReferrersScanned {
                stamp,
                package,
                scan,
            });
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_chimp_referrers_scanned(
        &mut self,
        stamp: KitStamp,
        package: String,
        scan: ChimpReferrerScan,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        if let Some(document) = self.kits[index].chimp.documents.get_mut(&package) {
            document.referrers = ChimpReferrerState::Done(scan);
        }
        false
    }

    pub(super) fn handle_chimp_package_loaded(
        &mut self,
        stamp: KitStamp,
        package: String,
        result: Result<ChimpDocument, String>,
    ) -> bool {
        let Some(index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[index].chimp.loading_packages.remove(&package);
        match result {
            Ok(document) => {
                self.kits[index]
                    .chimp
                    .documents
                    .insert(package.clone(), document);
                let kit_id = self.kits[index].id;
                self.kits[index].chimp.open_document_pane(kit_id, &package);
            }
            Err(error) => self.status = error,
        }
        if self.kits[index].chimp.loading_packages.is_empty()
            && let Some(active) = self.kits[index].pending_restore_active_chimp_package.take()
            && self.kits[index].documents_contains_chimp(&active)
        {
            let kit_id = self.kits[index].id;
            self.kits[index].chimp.selected_package = Some(active.clone());
            self.kits[index].chimp.open_document_pane(kit_id, &active);
        }
        false
    }

    pub(super) fn draw_chimp_workspace(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        kit_index: usize,
    ) {
        chimp_workspace_toolbar(ui, |ui| {
            let packages = self.chimp_dirty_packages(kit_index);
            let icon = button_icon_image(ui, ButtonIcon::Garbage, text_dark(), 16.0);
            let response = ui.add_enabled(!packages.is_empty(), egui::Button::image(icon));
            if response
                .on_hover_text(
                    "Discard every modified Chimp package in this workspace and restore the original source data",
                )
                .on_disabled_hover_text("This workspace has no modified Chimp packages")
                .clicked()
            {
                self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                    kit: self.kits[kit_index].id,
                    packages,
                    pending_action: None,
                    error: None,
                });
            }
        });
        self.draw_chimp_level_progress(ui, kit_index);
        ui.add_space(4.0);
        let ready = matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_));
        egui::SidePanel::left(egui::Id::new((
            "chimp_package_browser",
            self.kits[kit_index].id.0,
        )))
        .resizable(true)
        .default_width(360.0)
        .frame(
            Frame::none()
                .fill(left_panel())
                .inner_margin(egui::Margin::same(8.0)),
        )
        .show_inside(ui, |ui| {
            self.draw_chimp_browser(ui, ctx, kit_index);
        });
        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(editor_bg())
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show_inside(ui, |ui| {
                if ready {
                    match self.kits[kit_index].chimp.browser {
                        ChimpBrowser::Folders => {
                            match self.kits[kit_index].chimp.folder_selection {
                                ChimpFolderSelection::Package => {
                                    self.draw_chimp_tiles(ui, ctx, kit_index)
                                }
                                ChimpFolderSelection::File => self.draw_chimp_file(ui, kit_index),
                            }
                        }
                        ChimpBrowser::Groups => self.draw_chimp_tiles(ui, ctx, kit_index),
                        ChimpBrowser::Packages => self.draw_chimp_tiles(ui, ctx, kit_index),
                        ChimpBrowser::Archives => {
                            ui.centered_and_justified(|ui| {
                                ui.label("Select an archive to browse its folder hierarchy.");
                            });
                        }
                        ChimpBrowser::Files => self.draw_chimp_file(ui, kit_index),
                    }
                } else {
                    self.draw_chimp_mount_status(ui, kit_index);
                }
            });
    }

    /// A bar for the export running in this workspace, if one is.
    ///
    /// A level export is minutes of work, and without this the window simply
    /// sits there — the one thing a user cannot tell from a frozen-looking
    /// screen is the difference between working and stuck.
    fn draw_chimp_level_progress(&mut self, ui: &mut Ui, kit_index: usize) {
        let kit = self.kits[kit_index].id;
        let Some(job) = self.chimp_level_job.as_ref().filter(|job| job.kit == kit) else {
            return;
        };
        let phase = job.phase;
        let (done, total) = (job.done, job.total);
        let fraction = job.fraction();
        let remaining = job.remaining();
        let name = job.name.clone();

        ui.add_space(4.0);
        egui::Frame::none()
            .fill(row_type())
            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Exporting {name}"))
                            .color(text_dark())
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Only ever an estimate, and said as one.
                        if let Some(remaining) = remaining {
                            ui.label(
                                RichText::new(format!("{} left", format_remaining(remaining)))
                                    .color(subtle_dark())
                                    .small(),
                            );
                        }
                    });
                });
                ui.add_space(3.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_height(10.0)
                        .text(
                            RichText::new(format!(
                                "{} ({}/3) — {done}/{total}",
                                phase.label(),
                                phase.step()
                            ))
                            .small(),
                        ),
                );
            });
        ui.add_space(2.0);
    }

    fn draw_chimp_mount_status(&mut self, ui: &mut Ui, kit_index: usize) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.heading("Chimp");
            ui.add_space(8.0);
            match &self.kits[kit_index].chimp.mount {
                ChimpMount::Idle => {
                    ui.label("The Unreal package index has not been started.");
                    if ui.button("Start Chimp").clicked() {
                        self.begin_chimp_mount(kit_index, ui.ctx().clone());
                    }
                }
                ChimpMount::Loading => {
                    ui.spinner();
                    ui.label("Discovering containers and indexing Unreal packages…");
                    ui.label(
                        RichText::new(
                            "Campaign Evolved tag editing remains available while this runs.",
                        )
                        .color(subtle_dark()),
                    );
                }
                ChimpMount::Failed(error) => {
                    ui.colored_label(Color32::from_rgb(210, 80, 80), error);
                    if ui.button("Retry").clicked() {
                        self.begin_chimp_mount(kit_index, ui.ctx().clone());
                    }
                }
                ChimpMount::Ready(_) => {}
            }
        });
    }

    fn draw_chimp_browser(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        ui.horizontal(|ui| {
            for (browser, label) in ChimpBrowser::TABS {
                ui.selectable_value(&mut self.kits[kit_index].chimp.browser, browser, label);
            }
            if matches!(self.kits[kit_index].chimp.mount, ChimpMount::Loading) {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.kits[kit_index].chimp.filter)
                .hint_text("Search package or container…")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            self.kits[kit_index].chimp.reset_filter();
        }
        ui.add_space(4.0);

        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => {
                self.draw_chimp_mount_status(ui, kit_index);
                return;
            }
        };
        self.kits[kit_index].chimp.refresh_filter(&world);
        // Container diagnostics are not surfaced here. A mount routinely skips
        // archives that carry nothing Chimp reads, and reporting that above the
        // browser on every view described the mount rather than anything the
        // reader can act on. The Archives tab still lists what it could not
        // open, which is where that question is actually being asked.
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Archives {
            self.draw_chimp_archives(ui, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Files {
            self.draw_chimp_pak_files(ui, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Folders {
            self.draw_chimp_folders(ui, ctx, &world, kit_index);
            return;
        }
        if self.kits[kit_index].chimp.browser == ChimpBrowser::Groups {
            self.draw_chimp_groups(ui, ctx, &world, kit_index);
            return;
        }
        let indices = self.kits[kit_index].chimp.filtered_packages.clone();
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let mut extract_texture = None;
        let mut extract_mesh = None;
        let mut export_level = None;
        egui::ScrollArea::vertical()
            .id_salt(("chimp_packages", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, indices.len(), |ui, range| {
                for row in range {
                    let package = &world.packages()[indices[row]];
                    let active = package.active_provider();
                    let overridden = package.providers.len() > 1;
                    let mut label = package.name.clone();
                    if overridden {
                        label.push_str("  ⧉");
                    }
                    let response =
                        ui.selectable_label(selected.as_deref() == Some(&package.name), label);
                    let response = if let Some(provider) = active {
                        response.on_hover_text(format!(
                            "{}\n{}\n{} provider(s)",
                            package.name,
                            world.containers()[provider.container].path.display(),
                            package.providers.len()
                        ))
                    } else {
                        response
                    };
                    let actions = ChimpPackageActions::of(
                        &package.name,
                        self.kits[kit_index]
                            .chimp
                            .package_types
                            .get(indices[row])
                            .and_then(Option::as_deref),
                    );
                    if actions.any() {
                        response.context_menu(|ui| {
                            if actions.texture {
                                chimp_texture_export_menu(
                                    ui,
                                    &package.name,
                                    &mut extract_texture,
                                );
                            }
                            if actions.mesh {
                                chimp_mesh_export_menu(ui, &package.name, &mut extract_mesh);
                            }
                            if actions.level {
                                chimp_level_export_menu(ui, &package.name, &mut export_level);
                            }
                        });
                    }
                    if response.clicked() {
                        self.begin_chimp_open_package(kit_index, package.name.clone(), ctx.clone());
                    }
                }
            });
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_groups(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        world: &World,
        kit_index: usize,
    ) {
        if self.kits[kit_index].chimp.type_indexing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Indexing Unreal package types…")
                        .small()
                        .color(subtle_dark()),
                );
            });
            ui.add_space(4.0);
        }

        let groups = &self.kits[kit_index].chimp.filtered_groups;
        let selected = self.kits[kit_index].chimp.selected_package.clone();
        let mut open_package = None;
        let mut extract_texture = None;
        let mut extract_mesh = None;
        let mut export_level = None;
        if groups.is_empty() && !self.kits[kit_index].chimp.type_indexing {
            ui.label(RichText::new("No matching Unreal packages.").color(subtle_dark()));
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt(("chimp_groups", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (kind, indices) in groups {
                    egui::CollapsingHeader::new(format!("{kind}  ·  {}", indices.len()))
                        .id_salt(("chimp_group", self.kits[kit_index].id.0, &kind))
                        .default_open(false)
                        .show(ui, |ui| {
                            for &index in indices {
                                let package = &world.packages()[index];
                                let label =
                                    package.name.rsplit('/').next().unwrap_or(&package.name);
                                let response = ui
                                    .selectable_label(
                                        selected.as_deref() == Some(&package.name),
                                        label,
                                    )
                                    .on_hover_text(&package.name);
                                let actions =
                                    ChimpPackageActions::of(&package.name, Some(kind.as_str()));
                                if actions.any() {
                                    response.context_menu(|ui| {
                                        if actions.texture {
                                            chimp_texture_export_menu(
                                                ui,
                                                &package.name,
                                                &mut extract_texture,
                                            );
                                        }
                                        if actions.mesh {
                                            chimp_mesh_export_menu(
                                                ui,
                                                &package.name,
                                                &mut extract_mesh,
                                            );
                                        }
                                        if actions.level {
                                            chimp_level_export_menu(
                                                ui,
                                                &package.name,
                                                &mut export_level,
                                            );
                                        }
                                    });
                                }
                                if response.clicked() {
                                    open_package = Some(package.name.clone());
                                }
                            }
                        });
                }
            });
        if let Some(package) = open_package {
            self.begin_chimp_open_package(kit_index, package, ctx.clone());
        }
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_archives(&mut self, ui: &mut Ui, world: &World, kit_index: usize) {
        let selected = self.kits[kit_index].chimp.selected_archive;
        egui::ScrollArea::vertical()
            .id_salt(("chimp_archives", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if ui
                    .selectable_label(selected.is_none(), "All mounted archives")
                    .clicked()
                {
                    let chimp = &mut self.kits[kit_index].chimp;
                    chimp.selected_archive = None;
                    chimp.browser = ChimpBrowser::Folders;
                    chimp.folder_selection = ChimpFolderSelection::Package;
                    chimp.filter.clear();
                    chimp.reset_filter();
                }
                ui.add_space(4.0);
                ui.label(RichText::new("IoStore").strong());
                for container in world.containers() {
                    let name = container
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("container.utoc");
                    let response = ui
                        .selectable_label(
                            selected == Some(ChimpArchive::IoStore(container.index)),
                            format!("{name}  ·  {} packages", container.package_count),
                        )
                        .on_hover_text(format!(
                            "{}\nMount order: {}{}",
                            container.path.display(),
                            container.read_order,
                            if container.recovered_directory_index {
                                "\nRecovered directory index"
                            } else {
                                ""
                            }
                        ));
                    if response.clicked() {
                        let chimp = &mut self.kits[kit_index].chimp;
                        chimp.selected_archive = Some(ChimpArchive::IoStore(container.index));
                        chimp.browser = ChimpBrowser::Folders;
                        chimp.folder_selection = ChimpFolderSelection::Package;
                        chimp.filter.clear();
                        chimp.reset_filter();
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Legacy pak").strong());
                for container in world.pak_containers() {
                    let name = container
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("container.pak");
                    let response = ui
                        .selectable_label(
                            selected == Some(ChimpArchive::Pak(container.index)),
                            format!("{name}  ·  {} files", container.file_count),
                        )
                        .on_hover_text(format!(
                            "{}\nMount order: {}",
                            container.path.display(),
                            container.read_order
                        ));
                    if response.clicked() {
                        let chimp = &mut self.kits[kit_index].chimp;
                        chimp.selected_archive = Some(ChimpArchive::Pak(container.index));
                        chimp.browser = ChimpBrowser::Folders;
                        chimp.folder_selection = ChimpFolderSelection::File;
                        chimp.filter.clear();
                        chimp.reset_filter();
                    }
                }
                if !world.diagnostics().is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Unavailable or empty").strong());
                    for diagnostic in world.diagnostics() {
                        let name = diagnostic
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("archive");
                        ui.colored_label(Color32::from_rgb(210, 150, 70), name)
                            .on_hover_text(format!(
                                "{}\n{}",
                                diagnostic.path.display(),
                                diagnostic.message
                            ));
                    }
                }
            });
    }

    fn draw_chimp_folders(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        world: &World,
        kit_index: usize,
    ) {
        let selected_archive = self.kits[kit_index].chimp.selected_archive;
        if let Some(archive) = selected_archive {
            ui.horizontal(|ui| {
                let path = match archive {
                    ChimpArchive::IoStore(index) => &world.containers()[index].path,
                    ChimpArchive::Pak(index) => &world.pak_containers()[index].path,
                };
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive");
                ui.label(RichText::new(name).strong());
                if ui.small_button("Show all").clicked() {
                    let chimp = &mut self.kits[kit_index].chimp;
                    chimp.selected_archive = None;
                    chimp.reset_filter();
                }
            });
            ui.separator();
        }
        let selected_package = self.kits[kit_index].chimp.selected_package.clone();
        let selected_file = self.kits[kit_index].chimp.selected_file.clone();
        let package_types = self.kits[kit_index].chimp.package_types.clone();
        let clicked = egui::ScrollArea::vertical()
            .id_salt(("chimp_folders", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                draw_chimp_folder_node(
                    ui,
                    &self.kits[kit_index].chimp.content_tree,
                    world,
                    &package_types,
                    selected_package.as_deref(),
                    selected_file.as_deref(),
                    "",
                )
            })
            .inner;
        match clicked {
            Some(ChimpTreeClick::Package(package)) => {
                self.kits[kit_index].chimp.folder_selection = ChimpFolderSelection::Package;
                self.begin_chimp_open_package(kit_index, package, ctx.clone());
            }
            Some(ChimpTreeClick::ExtractTexture(package)) => {
                self.begin_extract_chimp_texture(kit_index, &package);
            }
            Some(ChimpTreeClick::ExportLevel(package, format)) => {
                self.begin_export_chimp_level(kit_index, &package, format);
            }
            Some(ChimpTreeClick::ExtractMesh(package, format)) => {
                self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
            }
            Some(ChimpTreeClick::File(file)) => {
                let chimp = &mut self.kits[kit_index].chimp;
                chimp.folder_selection = ChimpFolderSelection::File;
                chimp.selected_file = Some(file);
            }
            None => {}
        }
    }

    fn draw_chimp_pak_files(&mut self, ui: &mut Ui, world: &World, kit_index: usize) {
        let indices = self.kits[kit_index].chimp.filtered_files.clone();
        let selected = self.kits[kit_index].chimp.selected_file.clone();
        egui::ScrollArea::vertical()
            .id_salt(("chimp_pak_files", self.kits[kit_index].id.0))
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, indices.len(), |ui, range| {
                for row in range {
                    let file = &world.pak_files()[indices[row]];
                    let active = file.active_provider();
                    let mut label = file.path.clone();
                    if file.providers.len() > 1 {
                        label.push_str("  ⧉");
                    }
                    let response =
                        ui.selectable_label(selected.as_deref() == Some(&file.path), label);
                    let response = if let Some(provider) = active {
                        response.on_hover_text(format!(
                            "{}\n{}\n{} provider(s)",
                            file.path,
                            world.pak_containers()[provider.container].path.display(),
                            file.providers.len()
                        ))
                    } else {
                        response
                    };
                    if response.clicked() {
                        self.kits[kit_index].chimp.selected_file = Some(file.path.clone());
                    }
                }
            });
    }

    fn draw_chimp_file(&mut self, ui: &mut Ui, kit_index: usize) {
        let Some(path) = self.kits[kit_index].chimp.selected_file.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a file from a legacy .pak container.");
            });
            return;
        };
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let Some(file) = world.pak_file(&path) else {
            return;
        };
        let Some(provider) = file.active_provider() else {
            return;
        };
        let container = &world.pak_containers()[provider.container];
        ui.heading(&file.path);
        ui.label(
            RichText::new(format!(
                "{} • {} provider(s)",
                container.path.display(),
                file.providers.len()
            ))
            .color(subtle_dark()),
        );
        ui.add_space(8.0);
        ui.label("Legacy-pak entries are exposed as raw files.");
        ui.label(
            RichText::new(
                "Wwise banks/media and other staged data can be extracted; Unreal package property editing uses the Packages view.",
            )
            .color(subtle_dark()),
        );
        if ui.button("Extract file…").clicked() {
            let suggested = std::path::Path::new(&file.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("extracted.bin");
            let Some(output) = rfd::FileDialog::new()
                .set_title("Extract legacy-pak file")
                .set_file_name(suggested)
                .save_file()
            else {
                return;
            };
            match world
                .read_pak_provider(provider)
                .and_then(|bytes| fs::write(&output, bytes).map_err(anyhow::Error::from))
            {
                Ok(()) => self.status = format!("Extracted {}", output.display()),
                Err(error) => {
                    self.status = format!("Could not extract {}: {error:#}", output.display())
                }
            }
        }
    }

    /// Close one Chimp package pane and drop its document.
    ///
    /// Returns `false` when the package was kept because it holds unsaved
    /// edits. Chimp has no save-changes prompt of its own — a dirty package
    /// simply refuses to close — so the caller reports that, since one blocked
    /// package in a "close all" is not worth a message per package.
    pub(in crate::app) fn close_chimp_package(&mut self, kit_index: usize, package: &str) -> bool {
        if self.kits[kit_index]
            .chimp
            .documents
            .get(package)
            .is_some_and(|document| document.dirty)
        {
            return false;
        }
        self.kits[kit_index].chimp.close_document_pane(package);
        self.kits[kit_index].chimp.documents.remove(package);
        true
    }

    fn draw_chimp_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context, kit_index: usize) {
        let Some(mut tree) = self.kits[kit_index].chimp.document_tree.take() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a package to inspect it.");
            });
            return;
        };
        if tree.is_empty() {
            self.kits[kit_index].chimp.document_tree = Some(tree);
            ui.centered_and_justified(|ui| {
                ui.label("Select a package to inspect it.");
            });
            return;
        }

        let mut behavior = ChimpPaneBehavior {
            app: self,
            kit_index,
            close_requests: Vec::new(),
            focused: None,
            close_all: false,
            close_all_but: None,
            extract_texture: None,
            extract_mesh: None,
            export_level: None,
        };
        tree.ui(&mut behavior, ui);
        let close_requests = std::mem::take(&mut behavior.close_requests);
        let focused = behavior.focused.take();
        let close_all = behavior.close_all;
        let close_all_but = behavior.close_all_but.take();
        let extract_texture = behavior.extract_texture.take();
        let extract_mesh = behavior.extract_mesh.take();
        let export_level = behavior.export_level.take();
        self.kits[kit_index].chimp.document_tree = Some(tree);
        self.kits[kit_index].chimp.sync_open_packages();
        if let Some(package) = focused {
            self.kits[kit_index].chimp.selected_package = Some(package);
        }

        let mut requested = if close_all {
            self.kits[kit_index].chimp.open_packages.clone()
        } else if let Some(keep) = close_all_but {
            self.kits[kit_index]
                .chimp
                .open_packages
                .iter()
                .filter(|package| *package != &keep)
                .cloned()
                .collect()
        } else {
            close_requests
        };
        requested.sort();
        requested.dedup();
        let mut blocked = false;
        for package in requested {
            if !self.close_chimp_package(kit_index, &package) {
                blocked = true;
            }
        }
        if blocked {
            self.status = "Save or discard modified Chimp packages before closing them.".to_owned();
        }
        if let Some(package) = extract_texture {
            self.begin_extract_chimp_texture(kit_index, &package);
        }
        if let Some((package, format)) = extract_mesh {
            self.begin_extract_chimp_mesh(kit_index, &package, format, ctx.clone());
        }
        if let Some((package, format)) = export_level {
            self.begin_export_chimp_level(kit_index, &package, format);
        }
    }

    fn draw_chimp_document_pane(
        &mut self,
        ui: &mut Ui,
        kit_index: usize,
        package: &str,
        scope: &str,
    ) {
        if !self.kits[kit_index].chimp.documents.contains_key(package) {
            ui.label("This package is no longer loaded.");
            return;
        }
        let package = package.to_owned();

        let mut save_mod = false;
        let mut extract_package = false;
        let mut extract_json = false;
        let mut extract_export = false;
        {
            let document = self.kits[kit_index]
                .chimp
                .documents
                .get_mut(&package)
                .expect("checked above");
            ui.horizontal(|ui| {
                ui.heading(&document.package);
                ui.separator();
                save_mod = ui
                    .add_enabled(document.dirty, egui::Button::new("Save Chimp changes…"))
                    .on_hover_text("Save every modified Chimp package in one operation")
                    .clicked();
                extract_package = ui.button("Extract package…").clicked();
                extract_json = ui.button("Export JSON…").clicked();
                extract_export = ui.button("Extract selected export…").clicked();
            });
        }
        if save_mod {
            self.open_chimp_save_dialog(kit_index);
        }
        if extract_package {
            self.extract_chimp_package(kit_index, &package);
        }
        if extract_json {
            self.extract_chimp_json(kit_index, &package);
        }
        if extract_export {
            self.extract_chimp_export(kit_index, &package);
        }

        // Read before the document borrow: `document` borrows this kit, and the
        // preference lives on the application.
        let expert = self.expert_mode;
        let mut scan_referrers = false;
        let world = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => world.clone(),
            _ => return,
        };
        let document = self.kits[kit_index]
            .chimp
            .documents
            .get_mut(&package)
            .expect("checked above");
        let container = &world.containers()[document.provider.container];
        ui.label(
            RichText::new(format!(
                "{} exports • {} imports • {} bytes • {}",
                document.header.export_map.len(),
                document.header.import_map.len(),
                document.original.len(),
                container.path.display()
            ))
            .color(subtle_dark()),
        );
        if document.orphaned {
            ui.label(
                RichText::new(
                    "No mounted container provides this package any more. It can still be read and \
                     extracted, but not written back.",
                )
                .color(Color32::from_rgb(170, 130, 60)),
            );
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut document.view, ChimpDocumentView::Document, "Document")
                .on_hover_text("Readable JSON representation of the complete decoded package");
            if !document.texture_previews.is_empty() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Texture, "Texture")
                    .on_hover_text("Decoded Texture2D image preview");
            }
            if document.mesh_kind.is_some() {
                ui.selectable_value(&mut document.view, ChimpDocumentView::Mesh, "Mesh")
                    .on_hover_text("Decoded Unreal mesh in Baboon's 3D viewer");
            }
            ui.selectable_value(
                &mut document.view,
                ChimpDocumentView::Properties,
                "Properties",
            )
            .on_hover_text("Inspect exports and edit supported reflected scalar properties");
            ui.selectable_value(&mut document.view, ChimpDocumentView::Header, "Header")
                .on_hover_text("The package's name map, imports and exports, and what uses each");
            ui.selectable_value(&mut document.view, ChimpDocumentView::Metadata, "Metadata")
                .on_hover_text("Package dependencies and physical archive providers");
        });
        ui.separator();

        let changed = match document.view {
            ChimpDocumentView::Document => {
                if document.document_text_dirty {
                    refresh_chimp_document_text(document);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_document_text", scope.to_owned(), package.clone()),
                    "Decoded Unreal package document",
                    "Copy JSON",
                    &document.document_text,
                    &document.document_line_numbers,
                );
                false
            }
            ChimpDocumentView::Texture => {
                draw_chimp_texture_preview(ui, document);
                false
            }
            ChimpDocumentView::Mesh => {
                match document.mesh_preview.as_ref() {
                    Some(Ok(preview)) => model_preview::draw_standalone_mesh_preview(
                        ui,
                        preview,
                        &mut document.mesh_preview_state,
                    ),
                    Some(Err(error)) => {
                        ui.colored_label(Color32::from_rgb(150, 56, 44), error);
                    }
                    None => {
                        ui.label(RichText::new("No mesh geometry found.").color(subtle_dark()));
                    }
                }
                false
            }
            ChimpDocumentView::Properties => {
                egui::SidePanel::left(egui::Id::new((
                    "chimp_exports",
                    scope.to_owned(),
                    package.clone(),
                )))
                .resizable(true)
                .default_width(220.0)
                .show_inside(ui, |ui| {
                    ui.label(RichText::new("Exports").strong());
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (index, export) in document.exports.iter().enumerate() {
                            let supported = export.decoded.is_ok();
                            let label =
                                format!("{}  {}", if supported { "●" } else { "○" }, export.object);
                            if ui
                                .selectable_label(document.selected_export == index, label)
                                .on_hover_text(export.class.as_deref().unwrap_or("Unknown class"))
                                .clicked()
                            {
                                document.selected_export = index;
                            }
                        }
                    });
                });
                egui::CentralPanel::default()
                    .show_inside(ui, |ui| {
                        draw_chimp_export_editor(ui, document, world.usmap())
                    })
                    .inner
            }
            ChimpDocumentView::Header => {
                draw_chimp_header_view(ui, document, &world, expert, &mut scan_referrers)
            }
            ChimpDocumentView::Metadata => {
                if document.metadata_text_dirty {
                    refresh_chimp_metadata_text(document, &world);
                }
                draw_chimp_json_document(
                    ui,
                    ("chimp_metadata_text", scope.to_owned(), package.clone()),
                    "Decoded package metadata",
                    "Copy metadata JSON",
                    &document.metadata_text,
                    &document.metadata_line_numbers,
                );
                false
            }
        };
        if changed {
            document.dirty = true;
            document.document_text_dirty = true;
            document.metadata_text_dirty = true;
            // Reference counts are derived from the same header the metadata
            // text is, so they go stale at exactly the same moment.
            document.header_usage = None;
        }
        let _ = document;
        if changed {
            self.checkpoint_chimp_document(kit_index, &package);
        }
        if scan_referrers {
            let ctx = ui.ctx().clone();
            self.begin_chimp_referrer_scan(kit_index, package.clone(), ctx);
        }
    }

    fn chimp_default_output_folder(&self, kit_index: usize) -> Option<PathBuf> {
        let root = match &self.kits.get(kit_index)?.source.as_ref()?.source {
            TagSource::IoStoreContainerSet { root, .. } => root,
            _ => return None,
        };
        Some(
            self.chimp_output_dir
                .clone()
                .unwrap_or_else(|| root.clone()),
        )
    }

    pub(super) fn open_chimp_save_dialog(&mut self, kit_index: usize) {
        self.open_chimp_save_dialog_with_pending(kit_index, None);
    }

    pub(super) fn open_chimp_save_dialog_for_close(
        &mut self,
        kit_index: usize,
        action: PendingCloseAction,
    ) -> bool {
        self.open_chimp_save_dialog_with_pending(kit_index, Some(action))
    }

    pub(super) fn has_chimp_save_dialog(&self) -> bool {
        self.kits.iter().any(|kit| kit.chimp.save_dialog.is_some())
    }

    fn open_chimp_save_dialog_with_pending(
        &mut self,
        kit_index: usize,
        pending_close_action: Option<PendingCloseAction>,
    ) -> bool {
        let dirty = self.kits[kit_index]
            .chimp
            .documents
            .values()
            .filter(|document| document.dirty)
            .count();
        if dirty == 0 {
            self.status = "Chimp has no modified packages to save".to_owned();
            return false;
        }
        let Some(folder) = self.chimp_default_output_folder(kit_index) else {
            self.status = "Chimp does not have a Paks output folder".to_owned();
            return false;
        };
        self.kits[kit_index].chimp.save_dialog = Some(ChimpSaveDialog {
            mode: ChimpSaveMode::ExportMod,
            name: "ChimpMod".to_owned(),
            folder,
            overwrite_acknowledged: false,
            pending_close_action,
        });
        true
    }

    pub(super) fn draw_chimp_discard_window(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.chimp_discard_prompt.as_ref() else {
            return;
        };
        let kit = prompt.kit;
        let packages = prompt.packages.clone();
        let pending_action = prompt.pending_action.clone();
        let error = prompt.error.clone();
        let mut open = true;
        let mut discard = false;
        let mut save = false;
        let mut cancel = false;

        egui::Window::new("Discard Chimp changes?")
            .id(egui::Id::new("chimp_discard_changes"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(if pending_action.is_some() {
                        "The following modified Chimp packages must be saved or discarded before closing."
                    } else {
                        "Every listed Chimp package will return to its original source data."
                    })
                    .color(text_dark()),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for package in &packages {
                            ui.label(RichText::new(package).color(text_dark()).monospace());
                        }
                    });
                if let Some(error) = error.as_deref() {
                    ui.add_space(6.0);
                    ui.colored_label(Color32::from_rgb(180, 48, 40), error);
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This removes the unsaved recovery copy. Exported mods and source PAK changes already saved are not affected, and this cannot be undone.",
                    )
                    .color(Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if pending_action.is_some()
                        && ui.button("Save Chimp Changes…").clicked()
                    {
                        save = true;
                    }
                    if ui.button("Discard Changes").clicked() {
                        discard = true;
                    }
                });
            });

        if !open || cancel {
            self.chimp_discard_prompt = None;
            return;
        }

        let Some(index) = self.resolve_kit(kit) else {
            self.chimp_discard_prompt = None;
            return;
        };

        if save {
            let Some(action) = pending_action else {
                return;
            };
            self.chimp_discard_prompt = None;
            if !self.open_chimp_save_dialog_for_close(index, action.clone()) {
                self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                    kit,
                    packages,
                    pending_action: Some(action),
                    error: Some(self.status.clone()),
                });
            }
        } else if discard {
            self.chimp_discard_prompt = None;
            match self.discard_chimp_packages(index, &packages) {
                Ok(count) => {
                    self.active = index;
                    self.status = format!("Discarded {count} modified Chimp package(s)");
                    if let Some(action) = pending_action {
                        self.request_close_action(action, ctx);
                    }
                }
                Err(error) => {
                    self.chimp_discard_prompt = Some(ChimpDiscardPrompt {
                        kit,
                        packages,
                        pending_action,
                        error: Some(error),
                    });
                }
            }
        }
    }

    pub(super) fn draw_chimp_save_window(&mut self, ctx: &egui::Context) {
        let Some(kit_index) = self
            .kits
            .iter()
            .position(|kit| kit.chimp.save_dialog.is_some())
        else {
            return;
        };
        let dirty_packages = self.chimp_dirty_packages(kit_index);
        let source_containers: Vec<PathBuf> = match &self.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => {
                let mut paths: Vec<_> = dirty_packages
                    .iter()
                    .filter_map(|package| {
                        let document = self.kits[kit_index].chimp.documents.get(package)?;
                        world
                            .containers()
                            .get(document.provider.container)
                            .map(|container| container.path.clone())
                    })
                    .collect();
                paths.sort();
                paths.dedup();
                paths
            }
            _ => Vec::new(),
        };
        let mut close = false;
        let mut action = None;
        let expert_mode = self.expert_mode;
        let dialog = self.kits[kit_index]
            .chimp
            .save_dialog
            .as_mut()
            .expect("checked above");
        // Overwriting the installed game's own containers is an expert-mode
        // route. A dialog left on that mode when expert mode is turned off
        // would still act on it, so the mode is corrected here rather than only
        // hidden below.
        if !expert_mode && dialog.mode == ChimpSaveMode::OverwriteSources {
            dialog.mode = ChimpSaveMode::ExportMod;
            dialog.overwrite_acknowledged = false;
        }
        egui::Window::new("Save Chimp changes")
            .id(egui::Id::new("chimp_save_changes"))
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} modified Unreal package(s) will be saved together.",
                    dirty_packages.len()
                ));
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for package in &dirty_packages {
                            ui.label(package);
                        }
                    });
                ui.separator();
                // Without expert mode there is one route, so it is stated
                // rather than offered as a choice of one.
                if expert_mode {
                    let mode_before = dialog.mode;
                    ui.radio_value(
                        &mut dialog.mode,
                        ChimpSaveMode::ExportMod,
                        "Export mod (recommended)",
                    );
                    ui.radio_value(
                        &mut dialog.mode,
                        ChimpSaveMode::OverwriteSources,
                        "Overwrite source PAKs",
                    );
                    if dialog.mode != mode_before {
                        dialog.overwrite_acknowledged = false;
                    }
                } else {
                    ui.label(
                        RichText::new(
                            "Saved as a mod, leaving the installed game untouched. Overwriting \
                             the game's own PAKs needs expert mode.",
                        )
                        .color(subtle_dark())
                        .small(),
                    );
                }
                ui.separator();

                let mut can_save;
                match dialog.mode {
                    ChimpSaveMode::ExportMod => {
                        ui.horizontal(|ui| {
                            ui.label("Mod name");
                            ui.text_edit_singleline(&mut dialog.name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Destination");
                            ui.label(dialog.folder.display().to_string());
                            if ui.button("Browse…").clicked()
                                && let Some(folder) = rfd::FileDialog::new()
                                    .set_title("Choose Chimp mod folder")
                                    .set_directory(&dialog.folder)
                                    .pick_folder()
                            {
                                dialog.folder = folder;
                                dialog.overwrite_acknowledged = false;
                            }
                        });
                        let stem = chimp_mod_stem(&dialog.name);
                        can_save = !sanitize_mod_name(&dialog.name).is_empty();
                        if can_save {
                            ui.label(format!("Output: {stem}.utoc / .ucas / .pak"));
                        } else {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                "Enter a file-safe mod name.",
                            );
                        }
                        let existing =
                            chimp_existing_triplet(&dialog.folder.join(format!("{stem}.utoc")));
                        if !existing.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(210, 120, 80),
                                format!("This will replace: {}", existing.join(", ")),
                            );
                            ui.checkbox(
                                &mut dialog.overwrite_acknowledged,
                                "Replace the existing mod container",
                            );
                            can_save &= dialog.overwrite_acknowledged;
                        }
                    }
                    ChimpSaveMode::OverwriteSources => {
                        ui.colored_label(
                            Color32::from_rgb(190, 72, 56),
                            "This replaces package indexes in the installed game containers.",
                        );
                        for path in &source_containers {
                            ui.label(path.display().to_string());
                        }
                        ui.checkbox(
                            &mut dialog.overwrite_acknowledged,
                            "I understand these source containers will be modified",
                        );
                        can_save = dialog.overwrite_acknowledged && !source_containers.is_empty();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    let label = match dialog.mode {
                        ChimpSaveMode::ExportMod => "Export mod",
                        ChimpSaveMode::OverwriteSources => "Overwrite source PAKs",
                    };
                    if ui.add_enabled(can_save, egui::Button::new(label)).clicked() {
                        action = Some(match dialog.mode {
                            ChimpSaveMode::ExportMod => ChimpSaveAction::Export(
                                dialog
                                    .folder
                                    .join(format!("{}.utoc", chimp_mod_stem(&dialog.name))),
                            ),
                            ChimpSaveMode::OverwriteSources => ChimpSaveAction::Overwrite,
                        });
                    }
                });
            });
        let pending_close_action = dialog.pending_close_action.clone();
        if close || action.is_some() {
            self.kits[kit_index].chimp.save_dialog = None;
        }
        match action {
            Some(ChimpSaveAction::Export(output)) => {
                self.chimp_output_dir = output.parent().map(Path::to_path_buf);
                self.export_chimp_mod_to(kit_index, output, ctx.clone());
                if let Some(action) = pending_close_action {
                    self.finish_chimp_close_after_save(kit_index, action, ctx);
                }
            }
            // Guarded here as well as in the dialog: this is the one action in
            // the app that edits the installed game's own containers, and it
            // should not be reachable by any route expert mode has not opened.
            Some(ChimpSaveAction::Overwrite) if !self.expert_mode => {
                self.status =
                    "Overwriting the game's own PAKs needs expert mode — save this as a mod \
                     instead"
                        .to_owned();
            }
            Some(ChimpSaveAction::Overwrite) => {
                self.overwrite_all_dirty_chimp_packages(kit_index, ctx.clone());
                if let Some(action) = pending_close_action {
                    self.finish_chimp_close_after_save(kit_index, action, ctx);
                }
            }
            None => {}
        }
    }

    fn finish_chimp_close_after_save(
        &mut self,
        kit_index: usize,
        action: PendingCloseAction,
        ctx: &egui::Context,
    ) {
        let packages = self.chimp_dirty_packages(kit_index);
        if packages.is_empty() {
            self.request_close_action(action, ctx);
        } else {
            self.open_chimp_discard_prompt(
                kit_index,
                packages,
                Some(action),
                Some(self.status.clone()),
            );
        }
    }

    fn export_chimp_mod_to(&mut self, kit_index: usize, output: PathBuf, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let mut rebuilt = Vec::new();
        for (document_key, document) in self.kits[kit_index]
            .chimp
            .documents
            .iter()
            .filter(|(_, document)| document.dirty)
        {
            match rebuild_chimp_document(&world, document) {
                Ok((bytes, store)) => rebuilt.push((
                    document_key.clone(),
                    document.provider.clone(),
                    bytes,
                    store,
                )),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        if rebuilt.is_empty() {
            self.status = "Chimp has no modified packages to build".to_owned();
            return;
        }
        if let Some(parent) = output.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.status = format!("Could not create {}: {error}", parent.display());
            return;
        }
        let temporary = output.with_file_name(format!(
            "{}.building.utoc",
            output
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("Chimp_P")
        ));
        let overrides: Vec<PackageOverride<'_>> = rebuilt
            .iter()
            .map(|(_, provider, bytes, store)| PackageOverride {
                archive: &world.archives()[provider.container],
                uasset_path: &provider.entry_path,
                bytes: bytes.clone(),
                store: store.clone(),
            })
            .collect();
        let override_count = overrides.len();
        let built_packages: Vec<String> = rebuilt
            .iter()
            .map(|(package, _, _, _)| package.clone())
            .collect();
        let written = write_package_mod_container(&overrides, &temporary);
        drop(overrides);
        if let Err(error) = written {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not build {}: {error}", output.display());
            return;
        }
        let validation = (|| -> Result<(), String> {
            let archive = blam_tags::iostore::IoStoreArchive::open(&temporary)
                .map_err(|error| format!("Could not reopen temporary mod: {error}"))?;
            for (_, provider, bytes, _) in &rebuilt {
                let source = &world.archives()[provider.container];
                let source_index = source
                    .chunk_index_for(&provider.entry_path)
                    .map_err(|error| error.to_string())?;
                let chunk_id = source
                    .chunk_id(source_index)
                    .map_err(|error| error.to_string())?;
                let saved_index = archive
                    .find_chunk(&chunk_id)
                    .ok_or_else(|| format!("Temporary mod is missing {}", provider.entry_path))?;
                let saved = archive
                    .read_chunk(saved_index)
                    .map_err(|error| error.to_string())?;
                if saved != *bytes {
                    return Err(format!(
                        "Temporary mod did not preserve {} exactly",
                        provider.entry_path
                    ));
                }
            }
            Ok(())
        })();
        if let Err(error) = validation {
            remove_chimp_triplet(&temporary);
            self.status = error;
            return;
        }

        // The active Chimp World (and possibly Baboon's tag mount, and possibly
        // a second workspace on the same install) can have the existing output
        // memory-mapped. The replacement is finished at a staging path first;
        // taking the lease idles every Chimp mount that covers it, unmapping
        // drops the tag mounts, and only then is the triplet swapped with a
        // rollback copy. The shipped game containers are never touched.
        drop(world);
        let mut lease =
            match self.acquire_container_write_lease(&output, ContainerWriteMode::Replace) {
                Ok(lease) => lease,
                Err(failure) => {
                    remove_chimp_triplet(&temporary);
                    self.status = failure.to_string();
                    return;
                }
            };
        if let Err(failure) = self.unmap_leased_containers(&mut lease) {
            remove_chimp_triplet(&temporary);
            self.status = failure.to_string();
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, &ctx);
            return;
        }
        let replaced = replace_chimp_triplet(&temporary, &output);
        let report = self.release_container_write_lease(
            lease,
            if replaced.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            },
            &ctx,
        );
        let reopen_failures = report.reopen_failures;
        match replaced {
            Ok(()) => {
                if let Err(error) = self.clear_chimp_recovery_packages(kit_index, &built_packages) {
                    self.status = format!(
                        "Built {} but could not clear Chimp recovery: {error}",
                        output.display()
                    );
                    return;
                }
                for (package, _, _, _) in rebuilt {
                    if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) {
                        document.dirty = false;
                    }
                }
                self.status = format!(
                    "Built {} modified Unreal package(s) into {}",
                    override_count,
                    output.display()
                );
                if !reopen_failures.is_empty() {
                    self.status.push_str(&format!(
                        "; {} tag mount(s) could not be reopened",
                        reopen_failures.len()
                    ));
                }
            }
            Err(error) => {
                self.status = format!("Could not install {}: {error}", output.display());
            }
        }
        // Remounting is the lease's job now — it knows which workspaces it
        // idled, which is not necessarily this one: an output outside the
        // game's `Paks` was never mapped and never needed idling.
        let _ = ctx;
    }

    fn overwrite_all_dirty_chimp_packages(&mut self, kit_index: usize, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let mut rebuilt = Vec::new();
        for (document_key, document) in self.kits[kit_index]
            .chimp
            .documents
            .iter()
            .filter(|(_, document)| document.dirty)
        {
            match rebuild_chimp_document(&world, document) {
                Ok((bytes, store)) => rebuilt.push((
                    document_key.clone(),
                    document.provider.clone(),
                    bytes,
                    store,
                )),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        if rebuilt.is_empty() {
            self.status = "Chimp has no modified packages to overwrite".to_owned();
            return;
        }

        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, (_, provider, _, _)) in rebuilt.iter().enumerate() {
            groups.entry(provider.container).or_default().push(index);
        }
        let mut originals = BTreeMap::new();
        for &container in groups.keys() {
            let path = world.containers()[container].path.clone();
            match fs::read(&path) {
                Ok(bytes) => {
                    originals.insert(container, (path, bytes));
                }
                Err(error) => {
                    self.status =
                        format!("Could not stage rollback for {}: {error}", path.display());
                    return;
                }
            }
        }

        let mut failure = None;
        for (&container, indices) in &groups {
            let replacements: Vec<_> = indices
                .iter()
                .map(|&index| {
                    let (_, provider, bytes, store) = &rebuilt[index];
                    PackageReplacement {
                        uasset_path: &provider.entry_path,
                        rebuilt_bytes: bytes,
                        store,
                    }
                })
                .collect();
            let path = &originals[&container].0;
            if let Err(error) =
                overwrite_packages_in_place_with(&world.archives()[container], path, &replacements)
            {
                failure = Some(format!("Could not overwrite {}: {error}", path.display()));
                break;
            }
        }

        if let Some(mut error) = failure {
            let mut rollback_failures = Vec::new();
            for (path, bytes) in originals.values() {
                if let Err(rollback_error) = fs::write(path, bytes) {
                    rollback_failures.push(format!("{}: {rollback_error}", path.display()));
                }
            }
            if !rollback_failures.is_empty() {
                error.push_str(&format!(
                    "; rollback also failed for {}",
                    rollback_failures.join(", ")
                ));
            }
            self.status = error;
            drop(world);
            self.begin_chimp_mount(kit_index, ctx);
            return;
        }

        let packages: Vec<String> = rebuilt
            .iter()
            .map(|(package, _, _, _)| package.clone())
            .collect();
        for (package, _, bytes, _) in rebuilt {
            if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(&package) {
                if let Ok(payloads) = read_payloads(&document.header, &bytes) {
                    document.payloads = payloads;
                }
                document.original = bytes;
            }
        }
        if let Err(error) = self.clear_chimp_recovery_packages(kit_index, &packages) {
            self.status = format!(
                "Overwrote the source packages, but could not clear Chimp recovery: {error}"
            );
            drop(world);
            self.begin_chimp_mount(kit_index, ctx);
            return;
        }
        for package in &packages {
            if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(package) {
                document.dirty = false;
            }
        }
        self.status = format!(
            "Overwrote {} modified Unreal package(s) across {} source container(s)",
            packages.len(),
            groups.len()
        );
        drop(world);
        self.begin_chimp_mount(kit_index, ctx);
    }

    fn overwrite_chimp_package(&mut self, kit_index: usize, package: &str, ctx: egui::Context) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let provider = document.provider.clone();
        let (bytes, store) = match rebuild_chimp_document(&world, document) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let archive = &world.archives()[provider.container];
        let path = world.containers()[provider.container].path.clone();
        let source_chunk = match archive
            .chunk_index_for(&provider.entry_path)
            .and_then(|index| archive.chunk_id(index))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                self.status = format!("Could not resolve {package} in {}: {error}", path.display());
                return;
            }
        };
        if let Err(error) =
            overwrite_package_in_place_with(archive, &path, &provider.entry_path, &bytes, &store)
        {
            self.status = format!("Could not overwrite {package}: {error}");
            return;
        }
        let verified = blam_tags::iostore::IoStoreArchive::open(&path)
            .and_then(|archive| {
                let index = archive.find_chunk(&source_chunk).ok_or(
                    blam_tags::iostore::IoStoreError::Package(
                        "saved package chunk is absent after reopening",
                    ),
                )?;
                archive.read_chunk(index)
            })
            .is_ok_and(|saved| saved == bytes);
        if !verified {
            self.status = format!(
                "{package} was written, but validation failed; the package remains marked modified"
            );
            return;
        }
        if let Err(error) = self.accept_chimp_package_save(kit_index, package, bytes) {
            self.status = format!("Saved {package}, but could not clear Chimp recovery: {error}");
            return;
        }
        self.status = format!("Saved {package} into {}", path.display());
        drop(world);
        self.begin_chimp_mount(kit_index, ctx);
    }

    fn save_chimp_package_to_folder(
        &mut self,
        kit_index: usize,
        package: &str,
        ctx: egui::Context,
    ) {
        let stem = chimp_package_container_stem(package);
        let Some(chosen) = rfd::FileDialog::new()
            .set_title("Save Chimp package container")
            .add_filter("Unreal IoStore container", &["utoc"])
            .set_file_name(format!("{stem}.utoc"))
            .save_file()
        else {
            return;
        };
        let output = chosen.with_extension("utoc");
        let folder = output.parent().unwrap_or(Path::new(".")).to_path_buf();
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let provider = document.provider.clone();
        let (bytes, store) = match rebuild_chimp_document(&world, document) {
            Ok(rebuilt) => rebuilt,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if let Err(error) = fs::create_dir_all(&folder) {
            self.status = format!("Could not create {}: {error}", folder.display());
            return;
        }
        let temporary = folder.join(format!("{stem}.building.utoc"));
        let source_archive = &world.archives()[provider.container];
        let source_chunk = match source_archive
            .chunk_index_for(&provider.entry_path)
            .and_then(|index| source_archive.chunk_id(index))
        {
            Ok(chunk) => chunk,
            Err(error) => {
                self.status = format!("Could not resolve {package}: {error}");
                return;
            }
        };
        let override_ = PackageOverride {
            archive: source_archive,
            uasset_path: &provider.entry_path,
            bytes: bytes.clone(),
            store,
        };
        if let Err(error) = write_package_mod_container(&[override_], &temporary) {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not build {}: {error}", output.display());
            return;
        }
        let validated = blam_tags::iostore::IoStoreArchive::open(&temporary)
            .and_then(|archive| {
                let index = archive.find_chunk(&source_chunk).ok_or(
                    blam_tags::iostore::IoStoreError::Package(
                        "saved package chunk is absent from the new container",
                    ),
                )?;
                archive.read_chunk(index)
            })
            .is_ok_and(|saved| saved == bytes);
        if !validated {
            remove_chimp_triplet(&temporary);
            self.status = format!("Could not validate the rebuilt package container for {package}");
            return;
        }

        drop(world);
        let mut lease =
            match self.acquire_container_write_lease(&output, ContainerWriteMode::Replace) {
                Ok(lease) => lease,
                Err(failure) => {
                    remove_chimp_triplet(&temporary);
                    self.status = failure.to_string();
                    return;
                }
            };
        if let Err(failure) = self.unmap_leased_containers(&mut lease) {
            remove_chimp_triplet(&temporary);
            self.status = failure.to_string();
            self.release_container_write_lease(lease, ContainerWriteOutcome::Unchanged, &ctx);
            return;
        }
        let replaced = replace_chimp_triplet(&temporary, &output);
        let report = self.release_container_write_lease(
            lease,
            if replaced.is_ok() {
                ContainerWriteOutcome::Committed
            } else {
                ContainerWriteOutcome::Unchanged
            },
            &ctx,
        );
        let reopen_failures = report.reopen_failures;
        match replaced {
            Ok(()) => {
                if let Err(error) = self.accept_chimp_package_save(kit_index, package, bytes) {
                    self.status =
                        format!("Saved {package}, but could not clear Chimp recovery: {error}");
                    return;
                }
                self.status = format!("Saved {package} as {}", output.display());
                if !reopen_failures.is_empty() {
                    self.status.push_str(&format!(
                        "; {} tag mount(s) could not be reopened",
                        reopen_failures.len()
                    ));
                }
            }
            Err(error) => {
                self.status = format!("Could not install {}: {error}", output.display());
            }
        }
        // Remounted by the lease, which knows what it idled.
        let _ = ctx;
    }

    fn accept_chimp_package_save(
        &mut self,
        kit_index: usize,
        package: &str,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(package) {
            if let Ok(payloads) = read_payloads(&document.header, &bytes) {
                document.payloads = payloads;
            }
            document.original = bytes;
        }
        self.clear_chimp_recovery_packages(kit_index, &[package.to_owned()])?;
        if let Some(document) = self.kits[kit_index].chimp.documents.get_mut(package) {
            document.dirty = false;
        }
        Ok(())
    }

    pub(super) fn draw_chimp_overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(kit_index) = self
            .kits
            .iter()
            .position(|kit| kit.chimp.pending_overwrite.is_some())
        else {
            return;
        };
        let package = self.kits[kit_index]
            .chimp
            .pending_overwrite
            .clone()
            .expect("checked above");
        let source = self.kits[kit_index]
            .chimp
            .documents
            .get(&package)
            .and_then(|document| match &self.kits[kit_index].chimp.mount {
                ChimpMount::Ready(world) => world
                    .containers()
                    .get(document.provider.container)
                    .map(|container| container.path.clone()),
                _ => None,
            });
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Overwrite Unreal package?")
            .id(egui::Id::new("chimp_overwrite_confirm"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.colored_label(
                    Color32::from_rgb(190, 72, 56),
                    "This modifies the selected game container in place.",
                );
                ui.label(RichText::new(&package).strong());
                if let Some(path) = &source {
                    ui.label(path.display().to_string());
                }
                ui.label("Baboon appends the rebuilt chunks and atomically updates the UTOC.");
                ui.checkbox(
                    &mut self.kits[kit_index].chimp.pending_overwrite_skip_future,
                    "Don't ask again (changeable in Settings)",
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Overwrite package").clicked() {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.kits[kit_index].chimp.pending_overwrite = None;
        } else if confirm {
            if self.kits[kit_index].chimp.pending_overwrite_skip_future {
                self.confirm_container_overwrite = false;
            }
            self.kits[kit_index].chimp.pending_overwrite = None;
            self.overwrite_chimp_package(kit_index, &package, ctx.clone());
        }
    }

    fn extract_chimp_package(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let bytes = if document.dirty {
            let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
                return;
            };
            match rebuild_chimp_document(world, document) {
                Ok((bytes, _)) => bytes,
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        } else {
            document.original.clone()
        };
        let suggested = format!("{}.uasset", package.rsplit('/').next().unwrap_or("package"));
        let Some(path) = rfd::FileDialog::new()
            .set_title("Extract Unreal package")
            .set_file_name(&suggested)
            .save_file()
        else {
            return;
        };
        match fs::write(&path, bytes) {
            Ok(()) => self.status = format!("Extracted {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    fn extract_chimp_export(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let index = document
            .selected_export
            .min(document.payloads.len().saturating_sub(1));
        let Some(payload) = document.payloads.get(index) else {
            return;
        };
        let name = document
            .exports
            .get(index)
            .map(|export| export.object.as_str())
            .unwrap_or("export");
        let Some(path) = rfd::FileDialog::new()
            .set_title("Extract raw Unreal export")
            .set_file_name(format!("{name}.bin"))
            .save_file()
        else {
            return;
        };
        match fs::write(&path, payload) {
            Ok(()) => self.status = format!("Extracted {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    fn extract_chimp_json(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let value = chimp_document_json(document);
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export Unreal property dump")
            .set_file_name(format!(
                "{}.json",
                package.rsplit('/').next().unwrap_or("package")
            ))
            .save_file()
        else {
            return;
        };
        match serde_json::to_vec_pretty(&value)
            .and_then(|bytes| fs::write(&path, bytes).map_err(serde_json::Error::io))
        {
            Ok(()) => self.status = format!("Exported {}", path.display()),
            Err(error) => self.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    /// Ask which image format to extract a Texture2D as.
    ///
    /// The save dialog comes after the answer, because the format decides both
    /// what the export is — a numbered UDIM set or a single file, a mip chain or
    /// one flat image — and what the picker should be named and filtered for.
    fn begin_extract_chimp_texture(&mut self, kit_index: usize, package: &str) {
        if !matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_)) {
            return;
        }
        self.chimp_texture_export_prompt = Some(ChimpTextureExportPrompt {
            kit: self.kits[kit_index].id,
            package: package.to_owned(),
            // DDS and split UDIM: the pair that round-trips into Unreal.
            export: ChimpTextureExport::default(),
        });
    }

    /// Pick a destination and run the extraction the prompt described.
    pub(super) fn start_chimp_texture_export(
        &mut self,
        prompt: ChimpTextureExportPrompt,
        ctx: egui::Context,
    ) {
        let ChimpTextureExportPrompt {
            kit,
            package,
            export,
        } = prompt;
        let format = export.format;
        let leaf = package.rsplit('/').next().unwrap_or("texture").to_owned();
        let (filter, extensions) = format.filter();
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("Extract Texture2D as {}", format.label()))
            .add_filter(filter, extensions)
            .set_file_name(format!("{leaf}.{}", format.extension()))
            .save_file()
        else {
            return;
        };
        let Some(kit_index) = self.kits.iter().position(|entry| entry.id == kit) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let tx = self.tx.clone();
        self.status = format!("Extracting {package}…");
        thread::spawn(move || {
            let result = write_chimp_texture(&world, &package, &path, export);
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    /// Offer to export a World Partition level, once it is known to be one.
    ///
    /// The prompt comes before the folder dialog rather than after, because how
    /// a level is split changes what the export *is* — a folder of regions or a
    /// single file — and that is not a thing to discover afterwards.
    pub(super) fn begin_export_chimp_level(
        &mut self,
        kit_index: usize,
        package: &str,
        format: ChimpLevelFormat,
    ) {
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let cells = chimp_level_cells(world, package);
        if cells.is_empty() {
            self.status = format!("{package} is not a World Partition level");
            return;
        }
        let default = SegmentBudget::default();
        self.chimp_level_export_prompt = Some(ChimpLevelExportPrompt {
            kit: self.kits[kit_index].id,
            package: package.to_owned(),
            cells,
            format,
            // The fallback is a proxy built for hardware that cannot run
            // Nanite; anyone exporting a level wants the geometry the game has.
            nanite: true,
            // A whole level in one USD does not import - measured - but a
            // Blender master is placements and pointers and opens as one file.
            split: matches!(format, ChimpLevelFormat::SegmentedUsd),
            triangles: default.triangles,
            placements: default.placements,
        });
    }

    pub(super) fn start_chimp_level_export(
        &mut self,
        prompt: ChimpLevelExportPrompt,
        ctx: egui::Context,
    ) {
        let Some(directory) = rfd::FileDialog::new()
            .set_title(format!("Export {} as {}", prompt.name(), prompt.format_label()))
            .pick_folder()
        else {
            return;
        };
        let Some(kit_index) = self.kits.iter().position(|kit| kit.id == prompt.kit) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let tx = self.tx.clone();
        let kit = prompt.kit;
        let name = prompt.name();
        let budget = prompt.budget();
        let detail = if prompt.nanite {
            MeshDetail::Nanite
        } else {
            MeshDetail::Fallback
        };
        let ChimpLevelExportPrompt { cells, format, .. } = prompt;
        self.chimp_level_job = Some(ChimpLevelJob {
            kit,
            name: name.clone(),
            phase: ChimpLevelPhase::ReadingCells,
            done: 0,
            total: cells.len(),
            phase_started: Instant::now(),
        });
        self.status = format!("Exporting {name}…");
        thread::spawn(move || {
            let total = cells.len();
            let report = |phase, done, total| {
                let _ = tx.send(WorkerMessage::ChimpLevelProgress {
                    kit,
                    phase,
                    done,
                    total,
                });
                ctx.request_repaint();
            };
            // Measured over all 2,334 cells of C10: 1.7s on one thread, 0.7s
            // on four, and 0.8s on sixteen. Past four the threads contend for
            // more than they win, and the whole walk is a second either way.
            let threads = std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(4)
                .min(4);
            let scene = read_cells(&world, &cells, threads, &|done| {
                report(ChimpLevelPhase::ReadingCells, done, total)
            });
            report(ChimpLevelPhase::ReadingCells, total, total);
            let stage = |stage, done, total| {
                report(
                    match stage {
                        ExportStage::Meshes => ChimpLevelPhase::WritingMeshes,
                        ExportStage::Segments => ChimpLevelPhase::WritingSegments,
                    },
                    done,
                    total,
                )
            };
            let result = match format {
                ChimpLevelFormat::SegmentedUsd => {
                    write_segmented_usd(&world, &scene, detail, &directory, &name, budget, &stage)
                        .map_err(|error| error.to_string())
                        .map(|report| {
                            format!(
                                "Exported {name}: {} meshes, {} placements, {} segment(s)",
                                report.prototypes, report.instances, report.segments
                            )
                        })
                }
                ChimpLevelFormat::Blender => {
                    write_blend_export(&world, &scene, detail, &directory, &name, budget, &stage)
                        .map_err(|error| error.to_string())
                        .map(|report| {
                            format!(
                                "Exported {name}: {} meshes, {} placements, {} master(s). \
                                 Run build_blend.py in Blender to build them.",
                                report.meshes, report.placements, report.segments
                            )
                        })
                }
            };
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }

    fn begin_extract_chimp_mesh(
        &mut self,
        kit_index: usize,
        package: &str,
        format: ChimpMeshFormat,
        _ctx: egui::Context,
    ) {
        let suggested = format!(
            "{}.{}",
            package.rsplit('/').next().unwrap_or("mesh"),
            format.extension()
        );
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("Extract mesh as {}", format.label()))
            .add_filter(format.label(), &[format.extension()])
            .set_file_name(&suggested)
            .save_file()
        else {
            return;
        };
        if !matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_)) {
            return;
        }
        // Asked once the destination is known, so the prompt can say exactly
        // where the textures would land.
        self.chimp_mesh_texture_prompt = Some(ChimpMeshTexturePrompt {
            kit: self.kits[kit_index].id,
            package: package.to_owned(),
            format,
            texture_export: ChimpTextureExport::default(),
            path,
        });
    }

    /// Run a mesh export that has been through the texture prompt.
    pub(super) fn start_chimp_mesh_export(
        &mut self,
        prompt: ChimpMeshTexturePrompt,
        textures: ChimpTextureScope,
        ctx: egui::Context,
    ) {
        let Some(kit_index) = self.kit_index(prompt.kit) else {
            self.status = "The workspace this export came from is closed".to_owned();
            return;
        };
        let ChimpMount::Ready(world) = &self.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let ChimpMeshTexturePrompt {
            package,
            format,
            texture_export,
            path,
            ..
        } = prompt;
        let tx = self.tx.clone();
        self.status = format!("Extracting {package} as {}…", format.label());
        thread::spawn(move || {
            let result =
                write_chimp_mesh(&world, &package, &path, format, textures, texture_export);
            let _ = tx.send(WorkerMessage::ExportFinished(result));
            ctx.request_repaint();
        });
    }
}

/// Where a mesh's textures are written, relative to the mesh itself.
pub(super) const CHIMP_TEXTURE_DIR: &str = "textures2d";

/// How much of a mesh's texture set to export alongside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChimpTextureScope {
    None,
    /// Only textures whose name carries the mesh's own subject.
    Matching,
    /// Everything the materials reference, shared master inputs included.
    All,
}

/// The word a mesh's own textures are expected to carry, taken from its name.
///
/// Materials reach a long way — a weapon's material graph pulls in the shared
/// master inputs, detail maps and lookup tables the whole game uses, so
/// exporting everything a mesh's materials touch runs to dozens of textures when
/// the user wanted the handful belonging to this model.
///
/// The subject is the first meaningful segment of the mesh's name:
/// `SM_AssaultRifle_GunBody_M_Default` is about `assaultrifle`, and its own
/// textures are the ones that say so. A leading segment of two characters or
/// fewer is an asset-type prefix (`SM_`, `SK_`), not the subject, so it is
/// skipped.
pub(super) fn chimp_mesh_texture_subject(package: &str) -> Option<String> {
    let leaf = package.rsplit('/').next().unwrap_or(package).to_lowercase();
    let mut segments = leaf.split('_').filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    let subject = if first.len() <= 2 {
        segments.next().unwrap_or(first)
    } else {
        first
    };
    (!subject.is_empty()).then(|| subject.to_owned())
}

/// Every Texture2D package reachable from a mesh's materials.
///
/// A mesh does not reference textures itself: it references materials, and the
/// materials reference the textures. Materials are recognised the same way the
/// material list written into the mesh file recognises them, by the `M_`/`MI_`
/// prefix the game's own content uses, so the textures exported alongside a mesh
/// are the ones belonging to the materials named in it.
///
/// The result is deduplicated and keeps its discovery order, so a texture shared
/// by several materials is written once.
fn chimp_mesh_texture_packages(world: &World, header: &FZenPackageHeader) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut textures = Vec::new();
    for material in header
        .imported_package_names
        .iter()
        .filter(|path| is_chimp_material_package(path))
    {
        let Ok(document) = load_chimp_document(world, material) else {
            continue;
        };
        for candidate in &document.header.imported_package_names {
            if seen.insert(candidate.clone()) {
                textures.push(candidate.clone());
            }
        }
    }
    textures
}

/// Write every texture a mesh's materials reference into `directory`.
///
/// Candidates are whatever the materials import, which includes plenty that are
/// not textures at all — an import that does not resolve to a decodable
/// Texture2D is simply not one, and is skipped rather than reported as a
/// failure. Anything that *is* a texture and still could not be written is
/// collected, because that is a real loss the caller should hear about.
fn write_chimp_mesh_textures(
    world: &World,
    header: &FZenPackageHeader,
    directory: &Path,
    subject: Option<&str>,
    options: ChimpTextureExport,
) -> (usize, Vec<String>) {
    let packages = chimp_mesh_texture_packages(world, header);
    let mut written = 0usize;
    let mut failures = Vec::new();
    let mut created = false;
    for package in packages {
        let leaf = package.rsplit('/').next().unwrap_or("texture");
        // Filtered before loading: decoding a texture only to discard it is the
        // expensive half of the export.
        if let Some(subject) = subject
            && !leaf.to_lowercase().contains(subject)
        {
            continue;
        }
        let Ok(document) = load_chimp_document(world, &package) else {
            continue;
        };
        if document.texture_previews.is_empty() {
            continue;
        }
        if !created {
            if let Err(error) = fs::create_dir_all(directory) {
                failures.push(format!(
                    "Could not create {}: {error}",
                    directory.display()
                ));
                return (written, failures);
            }
            created = true;
        }
        let output = directory.join(format!("{leaf}.{}", options.format.extension()));
        match write_chimp_texture(world, &package, &output, options) {
            Ok(_) => written += 1,
            Err(error) => failures.push(error),
        }
    }
    (written, failures)
}

/// The texture surfaces of the export the user has selected.
///
/// Export used to take whichever Texture2D export happened to decode first,
/// which is the wrong one whenever a package holds more than one and the combo
/// box is pointing at another.
fn chimp_selected_surfaces<'a>(
    document: &'a ChimpDocument,
    package: &str,
) -> Result<&'a Texture2dSurfaces, String> {
    let selected = document
        .texture_previews
        .iter()
        .find(|texture| texture.export_index == document.selected_export)
        .or_else(|| document.texture_previews.first())
        .ok_or_else(|| format!("{package} has no Texture2D export"))?;
    selected
        .surfaces
        .as_ref()
        .map_err(|error| format!("{package}: {error}"))
}

/// How a Texture2D should be written out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChimpTextureExport {
    pub(super) format: ChimpTextureFormat,
    /// Split a UDIM set into one numbered file per block. Off writes the whole
    /// set as the single stitched image it was reassembled into, for engines
    /// that have no UDIM support to import it with.
    pub(super) split_udim: bool,
}

impl Default for ChimpTextureExport {
    fn default() -> Self {
        Self {
            format: ChimpTextureFormat::Dds,
            split_udim: true,
        }
    }
}

/// Write every layer of a Texture2D, splitting a UDIM virtual texture into
/// 1001-numbered files unless asked for one stitched image.
///
/// All three formats split the same way and pick the same base level per block,
/// so a set exported as PNG lines up with the same set exported as DDS. Only
/// what each file carries differs: DDS keeps the cooked pixel format and the
/// whole mip chain, while TIFF and PNG are one flat RGBA8 image.
fn write_chimp_texture(
    world: &World,
    package: &str,
    output: &Path,
    options: ChimpTextureExport,
) -> Result<String, String> {
    let ChimpTextureExport { format, split_udim } = options;
    let document = load_chimp_document(world, package)?;
    let surfaces = chimp_selected_surfaces(&document, package)?;
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "texture".to_owned());
    let directory = output.parent().unwrap_or(Path::new("."));

    // Splitting is only a question for a set with more than one block; a plain
    // texture is one file either way.
    let split = split_udim && surfaces.is_udim();
    let (blocks_x, blocks_y) = if split {
        (surfaces.width_in_blocks.max(1), surfaces.height_in_blocks.max(1))
    } else {
        (1, 1)
    };
    let mut written = Vec::new();
    let mut failures = Vec::new();

    for (layer_index, layer) in surfaces.layers.iter().enumerate() {
        let layer_suffix = if surfaces.layers.len() > 1 {
            format!(".layer{layer_index}")
        } else {
            String::new()
        };
        if !split {
            let name = format!("{stem}{layer_suffix}.{}", format.extension());
            let path = directory.join(&name);
            match write_stitched_layer(surfaces, layer_index, layer, format, &path) {
                Ok(()) => written.push(name),
                Err(error) => failures.push(format!("{name}: {error}")),
            }
            continue;
        }
        // Decoding a base level is expensive - mip 0 of a large virtual texture
        // is tens of megabytes - and blocks sharing a level can share the work.
        let mut decoded_levels: HashMap<usize, Vec<u8>> = HashMap::new();
        for block_y in 0..blocks_y {
            for block_x in 0..blocks_x {
                let name = if blocks_x * blocks_y > 1 {
                    // `1001 + row * 10 + column`, with row counted down from the
                    // top of the reassembled image. Unreal derives a block's
                    // coordinates straight from the number - `BlockX` is
                    // `(udim - 1001) % 10` and `BlockY` is `(udim - 1001) / 10`
                    // - and lays `BlockY` downward in a texture whose V axis
                    // already points down, so block row 0 is the top row. The
                    // upward-counting v of a DCC's UDIM grid is that same
                    // mapping seen through the opposite V convention, not a
                    // second one to correct for: flipping here is what makes a
                    // multi-row set import with its rows swapped.
                    let udim = 1001 + block_y * 10 + block_x;
                    format!("{stem}{layer_suffix}.{udim}.{}", format.extension())
                } else {
                    format!("{stem}{layer_suffix}.{}", format.extension())
                };
                let path = directory.join(&name);
                let result = match format {
                    ChimpTextureFormat::Dds => {
                        write_dds_block(layer, blocks_x, blocks_y, block_x, block_y, &path)
                    }
                    ChimpTextureFormat::Tiff | ChimpTextureFormat::Png => write_flat_block(
                        layer,
                        blocks_x,
                        blocks_y,
                        block_x,
                        block_y,
                        format,
                        &mut decoded_levels,
                        &path,
                    ),
                };
                match result {
                    Ok(()) => written.push(name),
                    Err(error) => failures.push(format!("{name}: {error}")),
                }
            }
        }
    }

    if written.is_empty() {
        return Err(format!(
            "Could not export {package}: {}",
            failures.join("; ")
        ));
    }
    let mut message = format!(
        "Extracted {package} to {} ({} file{})",
        directory.display(),
        written.len(),
        if written.len() == 1 { "" } else { "s" }
    );
    if !failures.is_empty() {
        message.push_str(&format!("; skipped {}", failures.join("; ")));
    }
    Ok(message)
}

/// The finest level that holds every tile of this UDIM block and still divides
/// evenly into the block grid.
///
/// A mixed-resolution UDIM set starts some blocks lower down the chain, so this
/// is the level at which the block is at its authored resolution.
fn block_base_level(
    layer: &blam_tags::iostore::asset::texture2d::TextureLayerSurfaces,
    blocks_x: u32,
    blocks_y: u32,
    block_x: u32,
    block_y: u32,
) -> Option<usize> {
    layer.mips.iter().position(|surface| {
        surface.data.is_ok()
            && surface.covers_block(blocks_x, blocks_y, block_x, block_y)
            && surface.width % blocks_x == 0
            && surface.height % blocks_y == 0
    })
}

/// Write one UDIM block of one layer as a single flat RGBA8 image.
#[allow(clippy::too_many_arguments)]
fn write_flat_block(
    layer: &blam_tags::iostore::asset::texture2d::TextureLayerSurfaces,
    blocks_x: u32,
    blocks_y: u32,
    block_x: u32,
    block_y: u32,
    format: ChimpTextureFormat,
    decoded_levels: &mut HashMap<usize, Vec<u8>>,
    path: &Path,
) -> Result<(), String> {
    let level = block_base_level(layer, blocks_x, blocks_y, block_x, block_y)
        .ok_or_else(|| "no mip covers this block".to_owned())?;
    let surface = &layer.mips[level];
    let rgba = match decoded_levels.entry(level) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(surface.to_rgba8().map_err(|error| format!("{error:#}"))?)
        }
    };

    let block_width = surface.width / blocks_x;
    let block_height = surface.height / blocks_y;
    if block_width == 0 || block_height == 0 {
        return Err("block has no pixels at this mip".to_owned());
    }
    let origin_x = (block_x * block_width) as usize;
    let origin_y = (block_y * block_height) as usize;
    let stride = surface.width as usize * 4;
    let mut cropped = Vec::with_capacity(block_width as usize * block_height as usize * 4);
    for row in 0..block_height as usize {
        let start = (origin_y + row) * stride + origin_x * 4;
        let end = start + block_width as usize * 4;
        cropped.extend_from_slice(
            rgba.get(start..end)
                .ok_or_else(|| "mip is shorter than its dimensions".to_owned())?,
        );
    }

    write_flat_image(&cropped, block_width, block_height, format, path)
}

/// Write a whole layer as one file, keeping every UDIM block in the single
/// stitched image the tiles were reassembled into.
///
/// For DDS this stays lossless — the cooked format and the whole mip chain —
/// but only while every block carries every level. A mixed-resolution set has
/// gaps at its finest levels, and filling them means magnifying from the level
/// below, which cannot be expressed in compressed blocks; that case writes one
/// RGBA8 level instead so the image is complete rather than holed.
fn write_stitched_layer(
    surfaces: &Texture2dSurfaces,
    layer_index: usize,
    layer: &blam_tags::iostore::asset::texture2d::TextureLayerSurfaces,
    format: ChimpTextureFormat,
    path: &Path,
) -> Result<(), String> {
    let has_gaps = layer
        .mips
        .iter()
        .any(|surface| !surface.missing_tiles.is_empty());
    if matches!(format, ChimpTextureFormat::Dds) && !has_gaps {
        return write_dds_block(layer, 1, 1, 0, 0, path);
    }

    let rgba = surfaces
        .display_rgba8(layer_index, 0)
        .map_err(|error| format!("{error:#}"))?;
    let (width, height) = (surfaces.width, surfaces.height);
    if !matches!(format, ChimpTextureFormat::Dds) {
        return write_flat_image(&rgba, width, height, format, path);
    }

    let dxgi = blam_tags::bitmap::dds::ue_dxgi_format("PF_R8G8B8A8")
        .ok_or_else(|| "RGBA8 has no DDS equivalent".to_owned())?;
    let mut file = fs::File::create(path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    blam_tags::bitmap::dds::write_dds_dxgi(
        &mut file,
        dxgi,
        width,
        height,
        1,
        1,
        None,
        Some(32),
        &rgba,
    )
    .map_err(|error| format!("Could not encode {}: {error}", path.display()))
}

fn write_flat_image(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: ChimpTextureFormat,
    path: &Path,
) -> Result<(), String> {
    let mut file = fs::File::create(path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    match format {
        ChimpTextureFormat::Png => {
            image::RgbaImage::from_raw(width, height, rgba.to_vec())
                .ok_or_else(|| "image does not match its dimensions".to_owned())?
                .write_to(
                    &mut std::io::BufWriter::new(&mut file),
                    image::ImageFormat::Png,
                )
                .map_err(|error| format!("Could not encode {}: {error}", path.display()))
        }
        _ => blam_tags::bitmap::tiff::write_rgba8_tiff(&mut file, width, height, rgba)
            .map_err(|error| format!("Could not encode {}: {error}", path.display())),
    }
}

/// Write one UDIM block of one layer as a DDS with its whole mip chain.
///
/// Mips whose block dimensions no longer divide by the UDIM grid are dropped
/// rather than approximated: below that size the cooked data no longer holds one
/// block per region, so cropping would invent pixels.
fn write_dds_block(
    layer: &blam_tags::iostore::asset::texture2d::TextureLayerSurfaces,
    blocks_x: u32,
    blocks_y: u32,
    block_x: u32,
    block_y: u32,
    path: &Path,
) -> Result<(), String> {
    let mut format_name: Option<String> = None;
    let mut dimensions: Option<(u32, u32)> = None;
    let mut payload = Vec::new();
    let mut levels = 0u32;

    for surface in &layer.mips {
        let Ok(data) = &surface.data else {
            // A leading mip that cannot be read is skipped so the rest still
            // export, but a gap part-way down would leave the chain claiming
            // levels it does not contain.
            if levels > 0 {
                break;
            }
            continue;
        };
        // A UDIM set can author its blocks at different resolutions, so a
        // half-size block has no tiles at the finest levels. Start that block's
        // chain where its data actually begins: the file then carries the
        // resolution it was drawn at, rather than a magnified guess.
        if !surface.covers_block(blocks_x, blocks_y, block_x, block_y) {
            if levels > 0 {
                break;
            }
            continue;
        }
        // Every mip in one file must share a format; a fallback mip that came
        // out RGBA8 cannot sit in a BC7 chain.
        if format_name.get_or_insert_with(|| surface.pixel_format.clone()) != &surface.pixel_format {
            break;
        }
        let (unit_x, unit_y, unit_bytes) =
            blam_tags::iostore::asset::texture2d::ue_format_info(&surface.pixel_format)
                .ok_or_else(|| format!("{} has no DDS block size", surface.pixel_format))?;
        let surface_bw = surface.width.div_ceil(unit_x);
        let surface_bh = surface.height.div_ceil(unit_y);
        if surface_bw % blocks_x != 0 || surface_bh % blocks_y != 0 {
            break;
        }
        let block_bw = surface_bw / blocks_x;
        let block_bh = surface_bh / blocks_y;
        if block_bw == 0 || block_bh == 0 {
            break;
        }
        let stride = unit_bytes as usize;
        let origin_bx = (block_x * block_bw) as usize;
        let origin_by = (block_y * block_bh) as usize;
        for row in 0..block_bh as usize {
            let start = ((origin_by + row) * surface_bw as usize + origin_bx) * stride;
            let end = start + block_bw as usize * stride;
            let row_bytes = data
                .get(start..end)
                .ok_or_else(|| "mip is shorter than its dimensions".to_owned())?;
            payload.extend_from_slice(row_bytes);
        }
        if dimensions.is_none() {
            dimensions = Some((block_bw * unit_x, block_bh * unit_y));
        }
        levels += 1;
    }

    let format_name = format_name.ok_or_else(|| "no mip decoded".to_owned())?;
    let (width, height) = dimensions.ok_or_else(|| "no mip decoded".to_owned())?;
    let dxgi = blam_tags::bitmap::dds::ue_dxgi_format(&format_name)
        .ok_or_else(|| format!("{format_name} has no DDS equivalent"))?;
    let (unit_x, unit_y, unit_bytes) =
        blam_tags::iostore::asset::texture2d::ue_format_info(&format_name)
            .ok_or_else(|| format!("{format_name} has no DDS block size"))?;
    let compressed = unit_x > 1 || unit_y > 1;

    let mut file = fs::File::create(path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    blam_tags::bitmap::dds::write_dds_dxgi(
        &mut file,
        dxgi,
        width,
        height,
        levels,
        1,
        compressed.then_some((unit_x, unit_y, unit_bytes)),
        (!compressed).then_some(unit_bytes * 8),
        &payload,
    )
    .map_err(|error| format!("Could not encode {}: {error}", path.display()))
}

fn write_chimp_mesh(
    world: &World,
    package: &str,
    output: &Path,
    format: ChimpMeshFormat,
    textures: ChimpTextureScope,
    texture_export: ChimpTextureExport,
) -> Result<String, String> {
    let document = load_chimp_document(world, package)?;
    let kind = document
        .mesh_kind
        .ok_or_else(|| format!("{package} is not a StaticMesh or SkeletalMesh"))?;
    let materials = chimp_material_names(&document.header);
    let mut writer = std::io::BufWriter::new(
        fs::File::create(output)
            .map_err(|error| format!("Could not create {}: {error}", output.display()))?,
    );
    match kind {
        ChimpMeshKind::Skeletal => {
            let mesh = SkeletalMesh::from_package(
                &document.original,
                &document.header.name_map.copy_raw_names(),
                document.header.summary.header_size as usize,
            )
            .map_err(|error| format!("Could not decode {package}: {error:#}"))?;
            match format {
                ChimpMeshFormat::Jms => chimp_skeletal_mesh_to_jms(&mesh, &materials)
                    .write(&mut writer, 8213)
                    .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Psk => blam_tags::iostore::actorx::write_skeletal_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Psk,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Pskx => blam_tags::iostore::actorx::write_skeletal_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Pskx,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
            }
        }
        ChimpMeshKind::Static => {
            let archive = &world.archives()[document.provider.container];
            let bulk = archive
                .chunk_index_for(&document.provider.entry_path)
                .ok()
                .and_then(|chunk| archive.read_bulk_for(chunk, 0).ok());
            let mesh = StaticMesh::from_package_preferring_nanite(
                &document.original,
                document.header.summary.header_size as usize,
                bulk.as_deref(),
            )
            .map_err(|error| format!("Could not decode {package}: {error:#}"))?;
            match format {
                ChimpMeshFormat::Jms => chimp_static_mesh_to_jms(&mesh, &materials)
                    .write(&mut writer, 8213)
                    .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Psk => blam_tags::iostore::actorx::write_static_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Psk,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
                ChimpMeshFormat::Pskx => blam_tags::iostore::actorx::write_static_mesh(
                    &mesh,
                    &materials,
                    blam_tags::iostore::actorx::ActorXFormat::Pskx,
                    &mut writer,
                )
                .map_err(|error| error.to_string())?,
            }
        }
    }
    writer
        .flush()
        .map_err(|error| format!("Could not finish {}: {error}", output.display()))?;
    drop(writer);
    let mut message = format!(
        "Extracted {package} as {} to {}",
        format.label(),
        output.display()
    );
    if textures != ChimpTextureScope::None {
        let directory = output
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(CHIMP_TEXTURE_DIR);
        let subject = match textures {
            ChimpTextureScope::Matching => chimp_mesh_texture_subject(package),
            _ => None,
        };
        // The mesh is already written, so a texture that fails is reported
        // beside a successful export rather than turning the whole thing into a
        // failure the user has to redo.
        let (written, failures) = write_chimp_mesh_textures(
            world,
            &document.header,
            &directory,
            subject.as_deref(),
            texture_export,
        );
        let scope = match &subject {
            Some(subject) => format!(" matching \"{subject}\""),
            None => String::new(),
        };
        message.push_str(&match (written, failures.len()) {
            (0, 0) => format!(
                ", but no texture{scope} could be read from its materials — nothing was written \
                 to {}",
                directory.display()
            ),
            (_, 0) => format!(
                ", with {written} texture(s){scope} in {}",
                directory.display()
            ),
            (_, count) => format!(
                ", with {written} texture(s){scope} in {} — {count} failed: {}",
                directory.display(),
                failures.join("; ")
            ),
        });
    }
    Ok(message)
}

/// Canonical Unreal skeletal-mesh to JMS conversion shared by Chimp's direct
/// exporter and Campaign Evolved tag-model extraction.
pub(in crate::app) fn chimp_skeletal_mesh_to_jms(
    mesh: &SkeletalMesh,
    material_names: &[String],
) -> blam_tags::jms::JmsFile {
    blam_tags::iostore::actorx::skeletal_mesh_to_jms(mesh, material_names)
}

/// Canonical Unreal static-mesh to JMS conversion shared by Chimp's direct
/// exporter and Campaign Evolved tag-model extraction.
pub(in crate::app) fn chimp_static_mesh_to_jms(
    mesh: &StaticMesh,
    material_names: &[String],
) -> blam_tags::jms::JmsFile {
    blam_tags::iostore::actorx::static_mesh_to_jms(mesh, material_names)
}

fn draw_chimp_folder_node(
    ui: &mut Ui,
    node: &ChimpFolderNode,
    world: &World,
    package_types: &[Option<String>],
    selected_package: Option<&str>,
    selected_file: Option<&str>,
    parent: &str,
) -> Option<ChimpTreeClick> {
    let mut clicked = None;
    for (name, child) in &node.folders {
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        let child_clicked =
            egui::CollapsingHeader::new(format!("{name}  ·  {}", child.entry_count()))
                .id_salt(("chimp_folder", path.clone()))
                .show(ui, |ui| {
                    draw_chimp_folder_node(
                        ui,
                        child,
                        world,
                        package_types,
                        selected_package,
                        selected_file,
                        &path,
                    )
                })
                .body_returned
                .flatten();
        if clicked.is_none() {
            clicked = child_clicked;
        }
    }
    for leaf in &node.packages {
        let package = &world.packages()[leaf.package];
        let mut label = leaf.name.clone();
        if package.providers.len() > 1 {
            label.push_str("  ⧉");
        }
        let response = ui.selectable_label(selected_package == Some(package.name.as_str()), label);
        let response = if let Some(provider) = package.active_provider() {
            response.on_hover_text(format!(
                "{}\n{}\n{} provider(s)",
                package.name,
                world.containers()[provider.container].path.display(),
                package.providers.len()
            ))
        } else {
            response
        };
        let package_type = package_types.get(leaf.package).and_then(Option::as_deref);
        let actions = ChimpPackageActions::of(&package.name, package_type);
        if actions.any() {
            response.context_menu(|ui| {
                if actions.texture {
                    let mut request = None;
                    chimp_texture_export_menu(ui, &package.name, &mut request);
                    if let Some(name) = request {
                        clicked = Some(ChimpTreeClick::ExtractTexture(name));
                    }
                }
                if actions.mesh {
                    ui.menu_button("Extract mesh", |ui| {
                        for format in [
                            ChimpMeshFormat::Jms,
                            ChimpMeshFormat::Psk,
                            ChimpMeshFormat::Pskx,
                        ] {
                            if ui.button(format.label()).clicked() {
                                clicked =
                                    Some(ChimpTreeClick::ExtractMesh(package.name.clone(), format));
                                ui.close_menu();
                            }
                        }
                    });
                }
                if actions.level {
                    let mut requested = None;
                    chimp_level_export_menu(ui, &package.name, &mut requested);
                    if let Some((package, format)) = requested {
                        clicked = Some(ChimpTreeClick::ExportLevel(package, format));
                    }
                }
            });
        }
        if response.clicked() {
            clicked = Some(ChimpTreeClick::Package(package.name.clone()));
        }
    }
    for leaf in &node.files {
        let file = &world.pak_files()[leaf.file];
        let mut label = leaf.name.clone();
        if file.providers.len() > 1 {
            label.push_str("  ⧉");
        }
        let response = ui.selectable_label(selected_file == Some(file.path.as_str()), label);
        let response = if let Some(provider) = file.active_provider() {
            response.on_hover_text(format!(
                "{}\n{}\n{} provider(s)",
                file.path,
                world.pak_containers()[provider.container].path.display(),
                file.providers.len()
            ))
        } else {
            response
        };
        if response.clicked() {
            clicked = Some(ChimpTreeClick::File(file.path.clone()));
        }
    }
    clicked
}

fn chimp_mesh_export_menu(
    ui: &mut Ui,
    package: &str,
    requested: &mut Option<(String, ChimpMeshFormat)>,
) {
    ui.menu_button("Extract mesh", |ui| {
        for format in [
            ChimpMeshFormat::Jms,
            ChimpMeshFormat::Psk,
            ChimpMeshFormat::Pskx,
        ] {
            if ui.button(format.label()).clicked() {
                *requested = Some((package.to_owned(), format));
                ui.close_menu();
            }
        }
    });
}

/// Offered only where it means something: a package with `_Generated_` cells
/// beside it is a World Partition level, and everything else is one package.
fn chimp_level_export_menu(
    ui: &mut Ui,
    package: &str,
    requested: &mut Option<(String, ChimpLevelFormat)>,
) {
    ui.menu_button("Export level", |ui| {
        for format in [ChimpLevelFormat::SegmentedUsd, ChimpLevelFormat::Blender] {
            if ui
                .button(format.label())
                .on_hover_text(format.summary())
                .clicked()
            {
                *requested = Some((package.to_owned(), format));
                ui.close_menu();
            }
        }
    });
}

use super::controller::{
    ContainerWriteMode, ContainerWriteOutcome, container_triplet as triplet,
    remove_container_triplet,
};

fn chimp_mod_stem(name: &str) -> String {
    let sanitized = sanitize_mod_name(name);
    if sanitized
        .get(sanitized.len().saturating_sub(2)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case("_p"))
    {
        sanitized
    } else {
        format!("{sanitized}_P")
    }
}

fn chimp_existing_triplet(path: &Path) -> Vec<String> {
    triplet(path)
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("container file")
                .to_owned()
        })
        .collect()
}

fn chimp_package_container_stem(package: &str) -> String {
    let leaf = package
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("Chimp");
    let mut stem = leaf
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        stem.push_str("Chimp");
    }
    if !stem.ends_with("_P") {
        stem.push_str("_P");
    }
    stem
}

fn remove_chimp_triplet(path: &Path) {
    remove_container_triplet(path);
}

fn replace_chimp_triplet(temporary: &Path, output: &Path) -> Result<(), String> {
    super::controller::swap_container_triplet(temporary, output)
        .map_err(|failure| failure.to_string())
}

impl Kit {
    fn documents_contains_chimp(&self, package: &str) -> bool {
        self.chimp.documents.contains_key(package)
    }
}

fn draw_chimp_texture_preview(ui: &mut Ui, document: &mut ChimpDocument) {
    let options: Vec<(usize, String)> = document
        .texture_previews
        .iter()
        .map(|preview| {
            (
                preview.export_index,
                document
                    .exports
                    .get(preview.export_index)
                    .map(|export| export.object.clone())
                    .unwrap_or_else(|| format!("Export {}", preview.export_index)),
            )
        })
        .collect();
    let mut selected_export = if options
        .iter()
        .any(|(index, _)| *index == document.selected_export)
    {
        document.selected_export
    } else {
        options.first().map(|(index, _)| *index).unwrap_or(0)
    };
    if options.len() > 1 {
        let selected_label = options
            .iter()
            .find(|(index, _)| *index == selected_export)
            .map(|(_, label)| label.as_str())
            .unwrap_or("Texture export");
        egui::ComboBox::from_id_salt(("chimp_texture_export", document.package.clone()))
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (index, label) in &options {
                    ui.selectable_value(&mut selected_export, *index, label);
                }
            });
    }
    document.selected_export = selected_export;
    let Some(preview) = document
        .texture_previews
        .iter_mut()
        .find(|preview| preview.export_index == selected_export)
    else {
        ui.label("This package has no Texture2D export to preview.");
        return;
    };
    let texture_key = format!(
        "chimp_texture_{}_{}",
        document.package, preview.export_index
    );
    let ctx = ui.ctx().clone();

    // The shared viewer clears `decoded` when the layer or mip stepper moves;
    // refill it from the surfaces already in hand rather than re-reading the
    // package. This mirrors `draw_bitmap_preview` for classic bitmap tags.
    if preview.preview.decoded.is_none() {
        preview.preview.decoded = Some(match &preview.surfaces {
            Ok(surfaces) => chimp_texture_mip_data(
                surfaces,
                preview.preview.image_index,
                preview.preview.mip_index,
            ),
            Err(error) => Err(error.clone()),
        });
        preview.preview.texture_dirty = true;
    }

    if let Ok(surfaces) = &preview.surfaces {
        let max_side = ctx.input(|input| input.max_texture_side);
        let smallest = first_displayable_mip(surfaces, max_side);
        if preview.preview.mip_index < smallest {
            // Only reachable if the driver reports a smaller limit than the
            // conservative one used at decode time.
            preview.preview.mip_index = smallest;
            preview.preview.decoded = None;
        }
        if let Some(layer) = surfaces.layers.first()
            && smallest > 0
            && let Some(base) = layer.mips.first()
        {
            ui.label(
                RichText::new(format!(
                    "Mip 0 is {}x{}, larger than this display can upload ({max_side} px); showing mip {smallest} and smaller. Export writes every mip at full size.",
                    base.width, base.height
                ))
                .color(subtle_dark()),
            );
        }
    }

    draw_bitmap_preview_data(ui, &ctx, &texture_key, &mut preview.preview, true, "Layer");
}

#[derive(Clone, Copy)]
struct ChimpJsonPalette {
    plain: Color32,
    key: Color32,
    string: Color32,
    path: Color32,
    number: Color32,
    literal: Color32,
    punctuation: Color32,
}

impl ChimpJsonPalette {
    fn for_dark_mode(dark_mode: bool) -> Self {
        if dark_mode {
            Self {
                plain: Color32::from_rgb(210, 214, 222),
                key: Color32::from_rgb(244, 191, 92),
                string: Color32::from_rgb(190, 235, 125),
                path: Color32::from_rgb(232, 157, 222),
                number: Color32::from_rgb(242, 139, 130),
                literal: Color32::from_rgb(105, 190, 255),
                punctuation: Color32::from_rgb(105, 210, 225),
            }
        } else {
            Self {
                plain: Color32::from_rgb(45, 50, 58),
                key: Color32::from_rgb(145, 91, 0),
                string: Color32::from_rgb(55, 112, 15),
                path: Color32::from_rgb(154, 52, 136),
                number: Color32::from_rgb(185, 62, 48),
                literal: Color32::from_rgb(0, 104, 178),
                punctuation: Color32::from_rgb(0, 112, 128),
            }
        }
    }
}

#[derive(Default)]
struct ChimpJsonHighlighter;

impl egui::util::cache::ComputerMut<(&egui::FontId, &str, bool), egui::text::LayoutJob>
    for ChimpJsonHighlighter
{
    fn compute(
        &mut self,
        (font_id, text, dark_mode): (&egui::FontId, &str, bool),
    ) -> egui::text::LayoutJob {
        chimp_json_layout_job(text, font_id.clone(), dark_mode)
    }
}

fn chimp_json_highlight(ui: &Ui, text: &str) -> egui::text::LayoutJob {
    type HighlightCache =
        egui::util::cache::FrameCache<egui::text::LayoutJob, ChimpJsonHighlighter>;

    let font_id = TextStyle::Monospace.resolve(ui.style());
    let dark_mode = ui.visuals().dark_mode;
    ui.ctx().memory_mut(|memory| {
        memory
            .caches
            .cache::<HighlightCache>()
            .get((&font_id, text, dark_mode))
    })
}

fn chimp_json_layout_job(
    text: &str,
    font_id: egui::FontId,
    dark_mode: bool,
) -> egui::text::LayoutJob {
    let palette = ChimpJsonPalette::for_dark_mode(dark_mode);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let bytes = text.as_bytes();
    let mut offset = 0;
    let mut active_key = String::new();

    while offset < bytes.len() {
        let start = offset;
        let (end, color) = match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    match bytes[offset] {
                        b'\\' => offset = (offset + 2).min(bytes.len()),
                        b'"' => {
                            offset += 1;
                            break;
                        }
                        _ => offset += 1,
                    }
                }
                let mut after = offset;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    if offset >= start + 2 {
                        active_key = text[start + 1..offset - 1].to_owned();
                    }
                    (offset, palette.key)
                } else {
                    let path_value = active_key.to_ascii_lowercase().contains("path")
                        || active_key.to_ascii_lowercase().contains("package");
                    (
                        offset,
                        if path_value {
                            palette.path
                        } else {
                            palette.string
                        },
                    )
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                offset += 1;
                (offset, palette.punctuation)
            }
            b'-' | b'0'..=b'9' => {
                offset += 1;
                while offset < bytes.len()
                    && matches!(
                        bytes[offset],
                        b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                    )
                {
                    offset += 1;
                }
                (offset, palette.number)
            }
            _ if text[start..].starts_with("true") => {
                offset += 4;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("false") => {
                offset += 5;
                (offset, palette.literal)
            }
            _ if text[start..].starts_with("null") => {
                offset += 4;
                (offset, palette.literal)
            }
            byte if byte.is_ascii_whitespace() => {
                offset += 1;
                while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
                    offset += 1;
                }
                (offset, palette.plain)
            }
            _ => {
                offset += text[start..].chars().next().map_or(1, char::len_utf8);
                (offset, palette.plain)
            }
        };
        job.append(
            &text[start..end],
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}

fn chimp_line_numbers(text: &str) -> String {
    let count = text.lines().count().max(1);
    let width = count.to_string().len();
    (1..=count)
        .map(|line| format!("{line:>width$}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn draw_chimp_json_document(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    title: &str,
    copy_label: &str,
    text: &str,
    line_numbers: &str,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(subtle_dark()));
        if ui.small_button(copy_label).clicked() {
            ui.output_mut(|output| output.copied_text = text.to_owned());
        }
        ui.label(
            RichText::new(format!("{} lines", text.lines().count().max(1)))
                .small()
                .color(subtle_dark()),
        );
    });
    egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let highlighted = chimp_json_highlight(ui, text);
            let font_id = TextStyle::Monospace.resolve(ui.style());
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                Frame::none()
                    .fill(ui.visuals().faint_bg_color)
                    .inner_margin(egui::Margin {
                        left: 6.0,
                        right: 8.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(line_numbers)
                                    .font(font_id.clone())
                                    .color(ui.visuals().weak_text_color()),
                            )
                            .selectable(false),
                        );
                    });
                Frame::none()
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(highlighted).selectable(true));
                    });
            });
        });
}

fn draw_chimp_export_editor(ui: &mut Ui, document: &mut ChimpDocument, usmap: &Usmap) -> bool {
    let Some(export) = document.exports.get_mut(document.selected_export) else {
        ui.label("This package has no exports.");
        return false;
    };
    ui.heading(&export.object);
    ui.label(
        RichText::new(export.class.as_deref().unwrap_or("Unknown class")).color(subtle_dark()),
    );
    ui.add_space(6.0);
    let class = export.class.clone().unwrap_or_default();
    match &mut export.decoded {
        Ok(decoded) => match &mut decoded.block {
            ExportBlock::Reflected(block) => {
                ui.label(
                    RichText::new(
                        "Filled circle = editable scalar; outlined values are preserved read-only.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        draw_chimp_property_block(
                            ui,
                            block,
                            &class,
                            &mut document.header.name_map,
                            usmap,
                            0,
                        )
                    })
                    .inner
            }
            ExportBlock::NotSerialized => {
                ui.label("This class has no reflected property block.");
                ui.label("Its native payload is preserved byte-for-byte.");
                false
            }
            ExportBlock::Unreflected(block) => {
                ui.label("Reflection data for this class is not available.");
                ui.label(format!(
                    "{} untyped bytes are preserved byte-for-byte.",
                    block.rest.len()
                ));
                false
            }
        },
        Err(error) => {
            ui.colored_label(Color32::from_rgb(210, 120, 80), error);
            ui.label("The raw export remains available for extraction and is never rewritten.");
            false
        }
    }
}

fn draw_chimp_property_block(
    ui: &mut Ui,
    block: &mut PropertyBlock,
    class: &str,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    let existing: std::collections::BTreeSet<u32> = block
        .entries
        .iter()
        .filter_map(|entry| entry.slot.map(|slot| slot.index))
        .collect();
    if !class.is_empty()
        && let Ok(slots) = editable_schema_slots(class, usmap)
    {
        let omitted: Vec<_> = slots
            .into_iter()
            .filter(|(_, slot, ty)| {
                !existing.contains(&slot.index) && default_value_for_type(ty, usmap).is_ok()
            })
            .collect();
        if !omitted.is_empty() {
            ui.menu_button(
                format!("Add omitted property… ({})", omitted.len()),
                |ui| {
                    ui.set_max_height(360.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (name, slot, ty) in &omitted {
                            let label = if slot.array_index == 0 {
                                name.clone()
                            } else {
                                format!("{name}[{}]", slot.array_index)
                            };
                            if ui.button(label).on_hover_text(format!("{ty:?}")).clicked()
                                && let Ok(value) = default_value_for_type(ty, usmap)
                                && set_property_slot(
                                    block,
                                    class,
                                    name,
                                    slot.array_index,
                                    value,
                                    usmap,
                                )
                                .is_ok()
                            {
                                changed = true;
                                ui.close_menu();
                            }
                        }
                    });
                },
            );
            ui.separator();
        }
    }
    for (index, entry) in block.entries.iter_mut().enumerate() {
        let id = ui.make_persistent_id((depth, index, entry.name.as_ref()));
        let declared = entry
            .slot
            .and_then(|slot| property_type_for_slot(class, slot, usmap).ok());
        let label = match entry.slot.map(|slot| slot.array_index) {
            Some(array_index) if array_index > 0 => {
                format!("{}[{array_index}]", entry.name)
            }
            _ => entry.name.to_string(),
        };
        ui.horizontal_top(|ui| {
            ui.set_min_height(24.0);
            ui.label(RichText::new(label).strong()).on_hover_text(
                declared
                    .as_ref()
                    .map(|ty| format!("{ty:?}"))
                    .unwrap_or_else(|| "Native field without a USMAP slot".to_owned()),
            );
            changed |= chimp_property_value_cell(ui, |ui| {
                draw_chimp_value(
                    ui,
                    id,
                    &mut entry.value,
                    declared.as_ref(),
                    names,
                    usmap,
                    depth,
                )
            })
            .inner;
        });
        ui.separator();
    }
    changed
}

fn chimp_property_value_cell<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    // `Ui::with_layout` inherits the parent's full remaining cross-axis size.
    // In a vertical ScrollArea that made one compact scalar editor consume the
    // entire viewport and then centered the widget inside it. Start each value
    // cell at one normal row instead; `allocate_ui_with_layout` still expands
    // when a nested struct or container genuinely needs additional height.
    let row_height = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_height),
        egui::Layout::right_to_left(egui::Align::Min),
        add_contents,
    )
}

fn chimp_workspace_toolbar<R>(
    ui: &mut Ui,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let row_height = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), row_height),
        egui::Layout::right_to_left(egui::Align::Center),
        add_contents,
    )
}

fn sorted_unique_dirty_chimp_keys<'a>(
    documents: impl IntoIterator<Item = (&'a str, bool)>,
) -> Vec<String> {
    let mut packages = documents
        .into_iter()
        .filter_map(|(key, dirty)| dirty.then(|| key.to_owned()))
        .collect::<Vec<_>>();
    packages.sort();
    packages.dedup();
    packages
}

fn draw_chimp_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut PropValue,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    if let Some(PropertyType::Optional(inner)) = declared
        && !matches!(value, PropValue::Unset)
    {
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.small_button("Unset").clicked() {
                *value = PropValue::Unset;
                changed = true;
            } else {
                changed |= draw_chimp_value(
                    ui,
                    id.with("optional"),
                    value,
                    Some(inner),
                    names,
                    usmap,
                    depth,
                );
            }
        });
        return changed;
    }
    match value {
        PropValue::Bool(value) => ui.checkbox(value, "").changed(),
        PropValue::Int(value) => draw_chimp_integer(ui, value, declared, usmap),
        PropValue::Float(value) => ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
        PropValue::Str(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                value.set_text(text);
            }
            changed
        }
        PropValue::Name(value) => {
            let mut text = value.to_string();
            let changed = ui
                .add(egui::TextEdit::singleline(&mut text).id(id))
                .changed();
            if changed {
                *value = blam_tags::iostore::object::edit::intern_name(names, &text);
            }
            changed
        }
        PropValue::Object(value) => ui
            .add(egui::DragValue::new(value).prefix("Object "))
            .on_hover_text("FPackageIndex: negative = import, positive = export, zero = None")
            .changed(),
        PropValue::SoftObject(value) => {
            let mut changed = false;
            egui::CollapsingHeader::new("Soft object path")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= draw_chimp_fname(ui, "Package", &mut value.package, names);
                    changed |= draw_chimp_fname(ui, "Asset", &mut value.asset, names);
                    let mut sub_path = value.sub_path.to_string();
                    ui.horizontal(|ui| {
                        ui.label("Sub-path");
                        if ui.text_edit_singleline(&mut sub_path).changed() {
                            value.sub_path.set_text(sub_path);
                            changed = true;
                        }
                    });
                });
            changed
        }
        PropValue::Struct(block) => {
            let nested_class = match declared {
                Some(PropertyType::Struct(name)) => name.as_str(),
                _ => "",
            };
            egui::CollapsingHeader::new(format!("Struct ({} properties)", block.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    draw_chimp_property_block(ui, block, nested_class, names, usmap, depth + 1)
                })
                .body_returned
                .unwrap_or(false)
        }
        PropValue::Array(values) => draw_chimp_sequence(
            ui,
            id,
            "Array",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Array(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            true,
        ),
        PropValue::Set(values) => draw_chimp_sequence(
            ui,
            id,
            "Set",
            values,
            declared.and_then(|ty| match ty {
                PropertyType::Set(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            names,
            usmap,
            depth,
            false,
        ),
        PropValue::Map(values) => draw_chimp_map(ui, id, values, declared, names, usmap, depth),
        PropValue::WithRemovals { removals, inner } => {
            let removal_type = match declared {
                Some(PropertyType::Map(key, _)) | Some(PropertyType::Set(key)) => {
                    Some(key.as_ref())
                }
                _ => None,
            };
            let mut changed = false;
            egui::CollapsingHeader::new("Delta-serialized container")
                .id_salt(id)
                .show(ui, |ui| {
                    let mut replace_whole = removals.is_none();
                    if ui
                        .checkbox(&mut replace_whole, "Replace whole container")
                        .changed()
                    {
                        *removals = (!replace_whole).then(Vec::new);
                        changed = true;
                    }
                    if let Some(removals) = removals {
                        changed |= draw_chimp_sequence(
                            ui,
                            id.with("removals"),
                            "Removed values",
                            removals,
                            removal_type,
                            names,
                            usmap,
                            depth + 1,
                            true,
                        );
                    }
                    changed |= draw_chimp_value(
                        ui,
                        id.with("inner"),
                        inner,
                        declared,
                        names,
                        usmap,
                        depth + 1,
                    );
                });
            changed
        }
        PropValue::Native(value) => draw_chimp_native(ui, id, value),
        PropValue::HandWritten(value) => {
            draw_chimp_hand_written(ui, id, value, names, usmap, depth)
        }
        PropValue::Delegate { object, function } => {
            let mut changed = false;
            egui::CollapsingHeader::new("Delegate")
                .id_salt(id)
                .show(ui, |ui| {
                    changed |= ui
                        .add(egui::DragValue::new(object).prefix("Object "))
                        .changed();
                    changed |= draw_chimp_fname(ui, "Function", function, names);
                });
            changed
        }
        PropValue::MulticastDelegate(values) => {
            draw_chimp_multicast_delegate(ui, id, values, names)
        }
        PropValue::FieldPath { path, owner } => {
            let mut changed = ui
                .add(egui::DragValue::new(owner).prefix("Owner "))
                .changed();
            egui::CollapsingHeader::new(format!("Field path ({} segments)", path.len()))
                .id_salt(id)
                .show(ui, |ui| {
                    let mut remove = None;
                    for (index, segment) in path.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= draw_chimp_fname(ui, &format!("[{index}]"), segment, names);
                            if ui.small_button("−").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(index) = remove {
                        path.remove(index);
                        changed = true;
                    }
                    if ui.small_button("+ segment").clicked() {
                        path.push(blam_tags::iostore::object::value::FName::none());
                        changed = true;
                    }
                });
            changed
        }
        PropValue::Unset => {
            if let Some(PropertyType::Optional(inner)) = declared
                && ui.button("Set value").clicked()
            {
                match default_value_for_type(inner, usmap) {
                    Ok(default) => {
                        *value = default;
                        return true;
                    }
                    Err(error) => {
                        ui.colored_label(Color32::from_rgb(210, 120, 80), error.to_string());
                    }
                }
            } else {
                ui.label("Unset");
            }
            false
        }
        PropValue::Raw(bytes) => {
            ui.colored_label(
                Color32::from_rgb(190, 150, 70),
                format!("{} untyped bytes · preserved read-only", bytes.len()),
            );
            false
        }
    }
}

fn draw_chimp_fname(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FName,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut text = value.to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(&mut text).changed() {
            *value = blam_tags::iostore::object::edit::intern_name(names, &text);
            changed = true;
        }
    });
    changed
}

fn draw_chimp_fstr(
    ui: &mut Ui,
    label: &str,
    value: &mut blam_tags::iostore::object::value::FStr,
) -> bool {
    let mut text = value.to_string();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(&mut text).changed() {
            value.set_text(text);
            changed = true;
        }
    });
    changed
}

fn draw_chimp_integer(
    ui: &mut Ui,
    value: &mut i64,
    declared: Option<&PropertyType>,
    usmap: &Usmap,
) -> bool {
    let ty = match declared {
        Some(PropertyType::Enum { inner, enum_name }) => {
            if let Some(definition) = usmap.enums.iter().find(|item| item.name == *enum_name) {
                let selected = definition
                    .values
                    .iter()
                    .find(|(candidate, _)| *candidate == *value as u64)
                    .map(|(_, name)| name.as_str())
                    .unwrap_or("Unknown");
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(format!("{selected} ({})", *value as u64))
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            Some(inner.as_ref())
        }
        Some(PropertyType::Byte {
            enum_name: Some(enum_name),
        }) => {
            if let Some(definition) = usmap.enums.iter().find(|item| item.name == *enum_name) {
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(
                        definition
                            .values
                            .iter()
                            .find(|(candidate, _)| *candidate == *value as u64)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| value.to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for (candidate, name) in &definition.values {
                            if ui
                                .selectable_label(*value as u64 == *candidate, name)
                                .clicked()
                            {
                                *value = *candidate as i64;
                                changed = true;
                            }
                        }
                    });
                return changed;
            }
            declared
        }
        other => other,
    };
    match ty {
        Some(PropertyType::UInt64) => {
            let mut unsigned = *value as u64;
            let changed = ui.add(egui::DragValue::new(&mut unsigned)).changed();
            if changed {
                *value = unsigned as i64;
            }
            changed
        }
        Some(PropertyType::UInt32) => ui
            .add(egui::DragValue::new(value).range(0..=u32::MAX as i64))
            .changed(),
        Some(PropertyType::UInt16) => ui
            .add(egui::DragValue::new(value).range(0..=u16::MAX as i64))
            .changed(),
        Some(PropertyType::Int8) => ui
            .add(egui::DragValue::new(value).range(i8::MIN as i64..=i8::MAX as i64))
            .changed(),
        Some(PropertyType::Int16) => ui
            .add(egui::DragValue::new(value).range(i16::MIN as i64..=i16::MAX as i64))
            .changed(),
        Some(PropertyType::Int) => ui
            .add(egui::DragValue::new(value).range(i32::MIN as i64..=i32::MAX as i64))
            .changed(),
        Some(PropertyType::Byte { .. }) => ui
            .add(egui::DragValue::new(value).range(0..=u8::MAX as i64))
            .changed(),
        _ => ui.add(egui::DragValue::new(value)).changed(),
    }
}

#[derive(Clone, Copy)]
enum ChimpListAction {
    Remove(usize),
    Duplicate(usize),
    MoveUp(usize),
    MoveDown(usize),
}

fn draw_chimp_list<T: Clone>(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<T>,
    default: Option<T>,
    mut draw: impl FnMut(&mut Ui, usize, &mut T) -> bool,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(format!("{label} ({})", values.len()))
        .id_salt(id)
        .show(ui, |ui| {
            let mut action = None;
            for (index, value) in values.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.label(format!("[{index}]"));
                        if ui.small_button("↑").clicked() {
                            action = Some(ChimpListAction::MoveUp(index));
                        }
                        if ui.small_button("↓").clicked() {
                            action = Some(ChimpListAction::MoveDown(index));
                        }
                        if ui.small_button("Duplicate").clicked() {
                            action = Some(ChimpListAction::Duplicate(index));
                        }
                        if ui.small_button("Remove").clicked() {
                            action = Some(ChimpListAction::Remove(index));
                        }
                    });
                    changed |= draw(ui, index, value);
                    ui.separator();
                });
            }
            if let Some(default) = default
                && ui.small_button("+ Add").clicked()
            {
                values.push(default);
                changed = true;
            }
            match action {
                Some(ChimpListAction::Remove(index)) => {
                    values.remove(index);
                    changed = true;
                }
                Some(ChimpListAction::Duplicate(index)) => {
                    values.insert(index + 1, values[index].clone());
                    changed = true;
                }
                Some(ChimpListAction::MoveUp(index)) if index > 0 => {
                    values.swap(index, index - 1);
                    changed = true;
                }
                Some(ChimpListAction::MoveDown(index)) if index + 1 < values.len() => {
                    values.swap(index, index + 1);
                    changed = true;
                }
                _ => {}
            }
        });
    changed
}

fn draw_chimp_sequence(
    ui: &mut Ui,
    id: egui::Id,
    label: &str,
    values: &mut Vec<PropValue>,
    inner_type: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
    allow_duplicates: bool,
) -> bool {
    let default = inner_type.and_then(|ty| default_value_for_type(ty, usmap).ok());
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, label, values, default, |ui, index, value| {
        draw_chimp_value(
            ui,
            id.with(index),
            value,
            inner_type,
            names,
            usmap,
            depth + 1,
        )
    });
    if !allow_duplicates {
        let mut duplicate = false;
        for left in 0..values.len() {
            duplicate |= values[left + 1..]
                .iter()
                .any(|right| values[left].semantic_eq(right));
        }
        if duplicate {
            *values = before;
            changed = false;
            ui.colored_label(
                Color32::from_rgb(210, 120, 80),
                "Sets cannot contain duplicate values.",
            );
        }
    }
    changed
}

fn draw_chimp_map(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(PropValue, PropValue)>,
    declared: Option<&PropertyType>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let (key_type, value_type) = match declared {
        Some(PropertyType::Map(key, value)) => (Some(key.as_ref()), Some(value.as_ref())),
        _ => (None, None),
    };
    let default = key_type
        .and_then(|key| default_value_for_type(key, usmap).ok())
        .zip(value_type.and_then(|value| default_value_for_type(value, usmap).ok()));
    let before = values.clone();
    let mut changed = draw_chimp_list(ui, id, "Map", values, default, |ui, index, pair| {
        let mut changed = false;
        ui.label(RichText::new("Key").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "key")),
            &mut pair.0,
            key_type,
            names,
            usmap,
            depth + 1,
        );
        ui.label(RichText::new("Value").strong());
        changed |= draw_chimp_value(
            ui,
            id.with((index, "value")),
            &mut pair.1,
            value_type,
            names,
            usmap,
            depth + 1,
        );
        changed
    });
    let mut duplicate = false;
    for left in 0..values.len() {
        duplicate |= values[left + 1..]
            .iter()
            .any(|right| values[left].0.semantic_eq(&right.0));
    }
    if duplicate {
        *values = before;
        changed = false;
        ui.colored_label(
            Color32::from_rgb(210, 120, 80),
            "Maps cannot contain duplicate keys.",
        );
    }
    changed
}

fn draw_chimp_multicast_delegate(
    ui: &mut Ui,
    id: egui::Id,
    values: &mut Vec<(i32, blam_tags::iostore::object::value::FName)>,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    draw_chimp_list(
        ui,
        id,
        "Multicast delegate",
        values,
        Some((0, blam_tags::iostore::object::value::FName::none())),
        |ui, _, (object, function)| {
            let mut changed = ui
                .add(egui::DragValue::new(object).prefix("Object "))
                .changed();
            changed |= draw_chimp_fname(ui, "Function", function, names);
            changed
        },
    )
}

fn draw_chimp_f64_values(ui: &mut Ui, values: &mut [f64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_f32_values(ui: &mut Ui, values: &mut [f32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(
                    egui::DragValue::new(value)
                        .speed(0.01)
                        .prefix(format!("{index}: ")),
                )
                .changed();
        }
    });
    changed
}

fn draw_chimp_i64_values(ui: &mut Ui, values: &mut [i64]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_i32_values(ui: &mut Ui, values: &mut [i32]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (index, value) in values.iter_mut().enumerate() {
            changed |= ui
                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                .changed();
        }
    });
    changed
}

fn draw_chimp_native(ui: &mut Ui, id: egui::Id, value: &mut NativeStruct) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Native struct")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                NativeStruct::Vec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec4d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec2d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::TwoVec3d(values) => draw_chimp_f64_values(ui, values),
                NativeStruct::Vec3f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Vec2f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4f(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::LinearColor(values) => draw_chimp_f32_values(ui, values),
                NativeStruct::Mat4d(values) => draw_chimp_f64_values(ui, values.as_mut()),
                NativeStruct::Ints(values) => draw_chimp_i64_values(ui, values),
                NativeStruct::Guid(values) => {
                    let mut changed = false;
                    ui.horizontal_wrapped(|ui| {
                        for (index, value) in values.iter_mut().enumerate() {
                            changed |= ui
                                .add(egui::DragValue::new(value).prefix(format!("{index}: ")))
                                .changed();
                        }
                    });
                    changed
                }
                NativeStruct::Color(values) => {
                    let mut changed = false;
                    for (label, value) in ["B", "G", "R", "A"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::Box3d { min, max, is_valid } => {
                    ui.label("Minimum");
                    let mut changed = draw_chimp_f64_values(ui, min);
                    ui.label("Maximum");
                    changed |= draw_chimp_f64_values(ui, max);
                    changed |= ui
                        .add(egui::DragValue::new(is_valid).prefix("Valid: "))
                        .changed();
                    changed
                }
                NativeStruct::RichCurveKey {
                    interp_mode,
                    tangent_mode,
                    tangent_weight_mode,
                    values,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(interp_mode).prefix("Interpolation: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_mode).prefix("Tangent: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(tangent_weight_mode).prefix("Weight: "))
                        .changed();
                    changed |= draw_chimp_f32_values(ui, values);
                    changed
                }
                NativeStruct::FontCharacter {
                    start_u,
                    start_v,
                    size_u,
                    size_v,
                    texture_index,
                    vertical_offset,
                } => {
                    let mut changed = false;
                    for (label, value) in [
                        ("Start U", start_u),
                        ("Start V", start_v),
                        ("Size U", size_u),
                        ("Size V", size_v),
                        ("Vertical offset", vertical_offset),
                    ] {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed |= ui
                        .add(egui::DragValue::new(texture_index).prefix("Texture: "))
                        .changed();
                    changed
                }
                NativeStruct::PackedBits(value) => ui
                    .add(egui::DragValue::new(value).prefix("Bits: "))
                    .changed(),
                NativeStruct::I32(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::I64(value) => ui.add(egui::DragValue::new(value)).changed(),
                NativeStruct::FrameRange {
                    lower_kind,
                    lower,
                    upper_kind,
                    upper,
                } => {
                    let mut changed = ui
                        .add(egui::DragValue::new(lower_kind).prefix("Lower kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(lower).prefix("Lower: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper_kind).prefix("Upper kind: "))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(upper).prefix("Upper: "))
                        .changed();
                    changed
                }
                NativeStruct::EvaluationKey(values) => {
                    let mut changed = false;
                    for (label, value) in ["Sequence", "Track", "Section"].into_iter().zip(values) {
                        changed |= ui
                            .add(egui::DragValue::new(value).prefix(format!("{label}: ")))
                            .changed();
                    }
                    changed
                }
                NativeStruct::PerPlatform { cooked, value } => {
                    let mut changed = ui.checkbox(cooked, "Cooked").changed();
                    changed |= match value {
                        PerPlatformValue::Int(value) => {
                            ui.add(egui::DragValue::new(value)).changed()
                        }
                        PerPlatformValue::Float(value) => {
                            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                        }
                        PerPlatformValue::Bool(value) => ui.checkbox(value, "Value").changed(),
                        PerPlatformValue::FrameRate(numerator, denominator) => {
                            let mut inner = ui
                                .add(egui::DragValue::new(numerator).prefix("Numerator: "))
                                .changed();
                            inner |= ui
                                .add(egui::DragValue::new(denominator).prefix("Denominator: "))
                                .changed();
                            inner
                        }
                    };
                    changed
                }
                NativeStruct::EmptySerializer => {
                    ui.label("No serialized fields");
                    false
                }
            };
        });
    changed
}

fn draw_chimp_optional_f32(ui: &mut Ui, label: &str, value: &mut Option<f32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0.0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value).speed(0.01)).changed();
    }
    changed
}

fn draw_chimp_optional_i32(ui: &mut Ui, label: &str, value: &mut Option<i32>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_optional_u64(ui: &mut Ui, label: &str, value: &mut Option<u64>) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, label).changed();
    if present && value.is_none() {
        *value = Some(0);
    } else if !present {
        *value = None;
    }
    if let Some(value) = value {
        changed |= ui.add(egui::DragValue::new(value)).changed();
    }
    changed
}

fn draw_chimp_tree_entry(ui: &mut Ui, value: &mut chimp_hw::TreeEntry) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.start).prefix("Start: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.size).prefix("Size: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.capacity).prefix("Capacity: "))
        .changed();
    changed
}

fn draw_chimp_tree_node(ui: &mut Ui, value: &mut chimp_hw::TreeNode) -> bool {
    let mut changed = false;
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower_kind).prefix("Lower kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_lower).prefix("Lower: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper_kind).prefix("Upper kind: "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut value.range_upper).prefix("Upper: "))
        .changed();
    for (label, item) in [
        ("Parent children", &mut value.parent_children_handle),
        ("Parent index", &mut value.parent_index),
        ("Children id", &mut value.children_id),
        ("Data id", &mut value.data_id),
    ] {
        changed |= ui
            .add(egui::DragValue::new(item).prefix(format!("{label}: ")))
            .changed();
    }
    changed
}

fn draw_chimp_shader_value(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::ShaderValueType,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.kind).prefix("Kind: "))
        .changed();
    changed |= ui
        .checkbox(&mut value.is_dynamic_array, "Dynamic array")
        .changed();
    match &mut value.body {
        chimp_hw::ShaderValueTypeBody::Struct { name, elements } => {
            changed |= draw_chimp_fname(ui, "Struct", name, names);
            changed |= draw_chimp_list(
                ui,
                id.with("elements"),
                "Elements",
                elements,
                None,
                |ui, index, (name, value)| {
                    let mut item_changed = draw_chimp_fname(ui, "Name", name, names);
                    item_changed |=
                        draw_chimp_shader_value(ui, id.with(("element", index)), value, names);
                    item_changed
                },
            );
        }
        chimp_hw::ShaderValueTypeBody::Dimension { dimension, counts } => {
            changed |= ui
                .add(egui::DragValue::new(dimension).prefix("Dimension: "))
                .changed();
            changed |= draw_chimp_list(
                ui,
                id.with("counts"),
                "Counts",
                counts,
                Some(0),
                |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
            );
        }
    }
    changed
}

fn draw_chimp_text_argument(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextFormatArgument,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    match value {
        chimp_hw::TextFormatArgument::Int(value) => ui.add(egui::DragValue::new(value)).changed(),
        chimp_hw::TextFormatArgument::UInt(value) | chimp_hw::TextFormatArgument::Gender(value) => {
            ui.add(egui::DragValue::new(value)).changed()
        }
        chimp_hw::TextFormatArgument::Float(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Double(value) => {
            ui.add(egui::DragValue::new(value).speed(0.01)).changed()
        }
        chimp_hw::TextFormatArgument::Text(value) => {
            draw_chimp_text(ui, id.with("text"), value, names)
        }
    }
}

fn draw_chimp_text(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::TextValue,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
) -> bool {
    let mut changed = ui
        .add(egui::DragValue::new(&mut value.flags).prefix("Flags: "))
        .changed();
    match &mut value.history {
        chimp_hw::TextHistory::None { culture_invariant } => {
            let mut present = culture_invariant.is_some();
            if ui
                .checkbox(&mut present, "Culture-invariant string")
                .changed()
            {
                *culture_invariant = present.then(Default::default);
                changed = true;
            }
            if let Some(text) = culture_invariant {
                changed |= draw_chimp_fstr(ui, "Value", text);
            }
        }
        chimp_hw::TextHistory::Base {
            namespace,
            key,
            source,
        } => {
            changed |= draw_chimp_fstr(ui, "Namespace", namespace);
            changed |= draw_chimp_fstr(ui, "Key", key);
            changed |= draw_chimp_fstr(ui, "Source", source);
        }
        chimp_hw::TextHistory::StringTableEntry { table_id, key } => {
            changed |= draw_chimp_fname(ui, "Table", table_id, names);
            changed |= draw_chimp_fstr(ui, "Key", key);
        }
        chimp_hw::TextHistory::OrderedFormat {
            source_fmt,
            arguments,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, argument| draw_chimp_text_argument(ui, id.with(index), argument, names),
            );
        }
        chimp_hw::TextHistory::NamedFormat {
            kind,
            source_fmt,
            arguments,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= draw_chimp_text(ui, id.with("source"), source_fmt, names);
            changed |= draw_chimp_list(
                ui,
                id.with("args"),
                "Arguments",
                arguments,
                None,
                |ui, index, (name, argument)| {
                    let mut item_changed = draw_chimp_fstr(ui, "Name", name);
                    item_changed |= draw_chimp_text_argument(ui, id.with(index), argument, names);
                    item_changed
                },
            );
        }
        chimp_hw::TextHistory::AsNumber {
            kind,
            currency_code,
            source_value,
            options,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            if let Some(currency) = currency_code {
                changed |= draw_chimp_fstr(ui, "Currency", currency);
            }
            changed |= draw_chimp_text_argument(ui, id.with("value"), source_value, names);
            if let Some(options) = options {
                changed |= ui
                    .checkbox(&mut options.always_sign, "Always sign")
                    .changed();
                changed |= ui
                    .checkbox(&mut options.use_grouping, "Use grouping")
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut options.rounding_mode).prefix("Rounding: "))
                    .changed();
                for (label, field) in [
                    ("Minimum integral", &mut options.minimum_integral_digits),
                    ("Maximum integral", &mut options.maximum_integral_digits),
                    ("Minimum fractional", &mut options.minimum_fractional_digits),
                    ("Maximum fractional", &mut options.maximum_fractional_digits),
                ] {
                    changed |= ui
                        .add(egui::DragValue::new(field).prefix(format!("{label}: ")))
                        .changed();
                }
            }
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::AsDateTime {
            kind,
            source_date_time,
            date_style,
            time_style,
            custom_pattern,
            time_zone,
            target_culture,
        } => {
            changed |= ui
                .add(egui::DragValue::new(kind).prefix("Kind: "))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(source_date_time).prefix("Ticks: "))
                .changed();
            for (label, value) in [("Date style", date_style), ("Time style", time_style)] {
                let mut present = value.is_some();
                if ui.checkbox(&mut present, label).changed() {
                    *value = present.then_some(0);
                    changed = true;
                }
                if let Some(value) = value {
                    changed |= ui.add(egui::DragValue::new(value)).changed();
                }
            }
            if let Some(pattern) = custom_pattern {
                changed |= draw_chimp_fstr(ui, "Pattern", pattern);
            }
            changed |= draw_chimp_fstr(ui, "Time zone", time_zone);
            changed |= draw_chimp_fstr(ui, "Culture", target_culture);
        }
        chimp_hw::TextHistory::Transform {
            source_text,
            transform_type,
        } => {
            changed |= draw_chimp_text(ui, id.with("source"), source_text, names);
            changed |= ui
                .add(egui::DragValue::new(transform_type).prefix("Transform: "))
                .changed();
        }
        chimp_hw::TextHistory::TextGenerator {
            generator_type_id,
            contents,
        } => {
            changed |= draw_chimp_fname(ui, "Generator", generator_type_id, names);
            ui.label(match contents {
                Some(bytes) => format!("{} generator bytes · preserved read-only", bytes.len()),
                None => "No generator payload".to_owned(),
            });
        }
    }
    changed
}

fn draw_chimp_sampler(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::WeightedRandomSampler,
) -> bool {
    let mut changed = draw_chimp_list(
        ui,
        id.with("prob"),
        "Probabilities",
        &mut value.prob,
        Some(0.0),
        |ui, _, value| ui.add(egui::DragValue::new(value).speed(0.01)).changed(),
    );
    changed |= draw_chimp_list(
        ui,
        id.with("alias"),
        "Aliases",
        &mut value.alias,
        Some(0),
        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
    );
    changed |= ui
        .add(egui::DragValue::new(&mut value.total_weight).prefix("Total weight: "))
        .changed();
    changed
}

fn draw_chimp_property_bag_type(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagPropertyType,
) -> bool {
    use chimp_hw::PropertyBagPropertyType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [
                T::None,
                T::Bool,
                T::Byte,
                T::Int32,
                T::Int64,
                T::Float,
                T::Double,
                T::Name,
                T::String,
                T::Text,
                T::Enum,
                T::Struct,
                T::Object,
                T::SoftObject,
                T::Class,
                T::SoftClass,
                T::UInt32,
                T::UInt64,
            ] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown type is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_property_bag_container(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::PropertyBagContainerType,
) -> bool {
    use chimp_hw::PropertyBagContainerType as T;
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for candidate in [T::None, T::Array, T::Set] {
                changed |= ui
                    .selectable_value(value, candidate, format!("{candidate:?}"))
                    .changed();
            }
            if matches!(value, T::Unknown(_)) {
                ui.label("Unknown container is preserved read-only");
            }
        });
    changed
}

fn draw_chimp_hand_written(
    ui: &mut Ui,
    id: egui::Id,
    value: &mut chimp_hw::HandWritten,
    names: &mut blam_tags::iostore::package::name_map::FNameMap,
    usmap: &Usmap,
    depth: usize,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Typed Unreal structure")
        .id_salt(id)
        .show(ui, |ui| {
            changed |= match value {
                chimp_hw::HandWritten::MaterialLayersTree(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("nodes"),
                        "Nodes",
                        &mut value.nodes,
                        Some([0; 4]),
                        |ui, _, values| {
                            let mut changed = false;
                            for value in values {
                                changed |= ui.add(egui::DragValue::new(value)).changed();
                            }
                            changed
                        },
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("payloads"),
                        "Payloads",
                        &mut value.payloads,
                        Some([0; 2]),
                        |ui, _, values| {
                            values.iter_mut().fold(false, |changed, value| {
                                ui.add(egui::DragValue::new(value)).changed() || changed
                            })
                        },
                    );
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.root).prefix("Root: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MovieSceneInlineValue(value) => {
                    let mut inner = draw_chimp_fstr(ui, "Type", &mut value.type_name);
                    if let Some(payload) = &mut value.payload {
                        let class = value.type_name.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::EvaluationTree(value) => {
                    let mut inner = draw_chimp_tree_node(ui, &mut value.root);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_entries"),
                        "Child entries",
                        &mut value.child_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("child_nodes"),
                        "Child nodes",
                        &mut value.child_nodes,
                        None,
                        |ui, _, value| draw_chimp_tree_node(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("data_entries"),
                        "Data entries",
                        &mut value.data_entries,
                        Some(chimp_hw::TreeEntry {
                            start: 0,
                            size: 0,
                            capacity: 0,
                        }),
                        |ui, _, value| draw_chimp_tree_entry(ui, value),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("items"),
                        "Items",
                        &mut value.items,
                        None,
                        |ui, _, value| match value {
                            chimp_hw::TreeItem::EntityAndMetaDataIndex { entity, meta_data } => {
                                ui.add(egui::DragValue::new(entity).prefix("Entity: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(meta_data).prefix("Metadata: "))
                                        .changed()
                            }
                            chimp_hw::TreeItem::SubSequence { sequence_id, flags } => {
                                ui.add(egui::DragValue::new(sequence_id).prefix("Sequence: "))
                                    .changed()
                                    | ui.add(egui::DragValue::new(flags).prefix("Flags: "))
                                        .changed()
                            }
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::ShaderValueType(value) => {
                    draw_chimp_shader_value(ui, id, value, names)
                }
                chimp_hw::HandWritten::PerQualityLevel(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.default_bits).prefix("Default bits: "))
                        .changed();
                    inner |= draw_chimp_list(
                        ui,
                        id.with("overrides"),
                        "Overrides",
                        &mut value.overrides,
                        Some((0, 0)),
                        |ui, _, (quality, bits)| {
                            ui.add(egui::DragValue::new(quality).prefix("Quality: "))
                                .changed()
                                | ui.add(egui::DragValue::new(bits).prefix("Bits: "))
                                    .changed()
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::FontData(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.font_face_asset).prefix("Face asset: "),
                        )
                        .changed();
                    let mut inline = value.inline_face.is_some();
                    if ui.checkbox(&mut inline, "Inline face").changed() {
                        value.inline_face = inline.then(|| chimp_hw::InlineFontFace {
                            filename: Default::default(),
                            hinting: 0,
                            loading_policy: 0,
                        });
                        inner = true;
                    }
                    if let Some(face) = &mut value.inline_face {
                        inner |= draw_chimp_fstr(ui, "Filename", &mut face.filename);
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.hinting).prefix("Hinting: "))
                            .changed();
                        inner |= ui
                            .add(egui::DragValue::new(&mut face.loading_policy).prefix("Loading: "))
                            .changed();
                    }
                    inner |= ui
                        .add(egui::DragValue::new(&mut value.sub_face_index).prefix("Sub-face: "))
                        .changed();
                    inner
                }
                chimp_hw::HandWritten::MaterialOverrideNanite(value) => {
                    let mut inner = ui.checkbox(&mut value.cooked, "Cooked").changed();
                    inner |= draw_chimp_optional_i32(
                        ui,
                        "Override material",
                        &mut value.override_material,
                    );
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.properties,
                        "MaterialOverrideNanite",
                        names,
                        usmap,
                        depth + 1,
                    );
                    inner
                }
                chimp_hw::HandWritten::TimeWarpVariant(value) => match value {
                    chimp_hw::TimeWarpVariant::Literal(value) => {
                        ui.add(egui::DragValue::new(value).speed(0.01)).changed()
                    }
                    chimp_hw::TimeWarpVariant::Typed {
                        kind,
                        object,
                        payload,
                    } => {
                        let mut inner = ui
                            .add(egui::DragValue::new(kind).prefix("Kind: "))
                            .changed();
                        inner |= draw_chimp_optional_i32(ui, "Object", object);
                        if let Some(payload) = payload {
                            inner |=
                                draw_chimp_property_block(ui, payload, "", names, usmap, depth + 1);
                        }
                        inner
                    }
                },
                chimp_hw::HandWritten::LocatorFragment(value) => {
                    let mut inner =
                        draw_chimp_fname(ui, "Fragment type", &mut value.fragment_type, names);
                    if let Some(payload) = &mut value.payload {
                        let class = value.fragment_type.to_string();
                        inner |=
                            draw_chimp_property_block(ui, payload, &class, names, usmap, depth + 1);
                    }
                    inner
                }
                chimp_hw::HandWritten::Text(value) => draw_chimp_text(ui, id, value, names),
                chimp_hw::HandWritten::MovieSceneChannel(value) => {
                    let mut inner = ui
                        .add(
                            egui::DragValue::new(&mut value.pre_infinity_extrap)
                                .prefix("Pre extrapolation: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.post_infinity_extrap)
                                .prefix("Post extrapolation: "),
                        )
                        .changed();
                    ui.label(format!("{} time bytes · preserved", value.times.data.len()));
                    ui.label(format!(
                        "{} value bytes · preserved",
                        value.values.data.len()
                    ));
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.default_value)
                                .speed(0.01)
                                .prefix("Default: "),
                        )
                        .changed();
                    inner |= ui
                        .checkbox(&mut value.has_default_value, "Has default")
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_numerator)
                                .prefix("Tick numerator: "),
                        )
                        .changed();
                    inner |= ui
                        .add(
                            egui::DragValue::new(&mut value.tick_resolution_denominator)
                                .prefix("Tick denominator: "),
                        )
                        .changed();
                    inner |= ui.checkbox(&mut value.show_curve, "Show curve").changed();
                    inner
                }
                chimp_hw::HandWritten::PcgPoint(value) => {
                    let mut inner = draw_chimp_f64_values(ui, &mut value.transform);
                    inner |= draw_chimp_optional_f32(ui, "Density", &mut value.density);
                    for (label, point) in [
                        ("Bounds minimum", &mut value.bounds_min),
                        ("Bounds maximum", &mut value.bounds_max),
                    ] {
                        let mut present = point.is_some();
                        if ui.checkbox(&mut present, label).changed() {
                            *point = present.then_some([0.0; 3]);
                            inner = true;
                        }
                        if let Some(point) = point {
                            inner |= draw_chimp_f64_values(ui, point);
                        }
                    }
                    let mut color = value.color.is_some();
                    if ui.checkbox(&mut color, "Color").changed() {
                        value.color = color.then_some([0.0; 4]);
                        inner = true;
                    }
                    if let Some(color) = &mut value.color {
                        inner |= draw_chimp_f64_values(ui, color);
                    }
                    inner |= draw_chimp_optional_f32(ui, "Steepness", &mut value.steepness);
                    inner |= draw_chimp_optional_i32(ui, "Seed", &mut value.seed);
                    inner |=
                        draw_chimp_optional_u64(ui, "Metadata entry", &mut value.metadata_entry);
                    inner
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingLod(value) => {
                    draw_chimp_sampler(ui, id, value)
                }
                chimp_hw::HandWritten::SkeletalMeshSamplingRegion(value) => {
                    let mut inner = draw_chimp_list(
                        ui,
                        id.with("triangles"),
                        "Triangle indices",
                        &mut value.triangle_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_list(
                        ui,
                        id.with("bones"),
                        "Bone indices",
                        &mut value.bone_indices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner |= draw_chimp_sampler(ui, id.with("sampler"), &mut value.sampler);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("vertices"),
                        "Vertices",
                        &mut value.vertices,
                        Some(0),
                        |ui, _, value| ui.add(egui::DragValue::new(value)).changed(),
                    );
                    inner
                }
                chimp_hw::HandWritten::NiagaraVariable(value) => {
                    let mut inner = draw_chimp_fname(ui, "Name", &mut value.name, names);
                    inner |= draw_chimp_property_block(
                        ui,
                        &mut value.type_def,
                        "NiagaraTypeDefinition",
                        names,
                        usmap,
                        depth + 1,
                    );
                    match &mut value.payload {
                        chimp_hw::NiagaraPayload::None => {}
                        chimp_hw::NiagaraPayload::Offset(value) => {
                            inner |= ui
                                .add(egui::DragValue::new(value).prefix("Offset: "))
                                .changed()
                        }
                        chimp_hw::NiagaraPayload::VarData(bytes) => {
                            ui.label(format!(
                                "{} variable-data bytes · preserved read-only",
                                bytes.len()
                            ));
                        }
                    }
                    inner
                }
                chimp_hw::HandWritten::NiagaraGpuParamInfo(value) => {
                    let mut inner = draw_chimp_fstr(ui, "HLSL symbol", &mut value.hlsl_symbol);
                    inner |= draw_chimp_fstr(ui, "DI class", &mut value.di_class_name);
                    inner |= draw_chimp_list(
                        ui,
                        id.with("functions"),
                        "Generated functions",
                        &mut value.generated_functions,
                        None,
                        |ui, index, function| {
                            let mut item = draw_chimp_fname(
                                ui,
                                "Definition",
                                &mut function.definition_name,
                                names,
                            );
                            item |= draw_chimp_fstr(ui, "Instance", &mut function.instance_name);
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "specifiers")),
                                "Specifiers",
                                &mut function.specifiers,
                                Some((
                                    blam_tags::iostore::object::value::FName::none(),
                                    blam_tags::iostore::object::value::FName::none(),
                                )),
                                |ui, _, (name, value)| {
                                    draw_chimp_fname(ui, "Name", name, names)
                                        | draw_chimp_fname(ui, "Value", value, names)
                                },
                            );
                            let default_reference = chimp_hw::NiagaraVariableCommonReference {
                                name: blam_tags::iostore::object::value::FName::none(),
                                underlying_type: 0,
                            };
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "inputs")),
                                "Variadic inputs",
                                &mut function.variadic_inputs,
                                Some(default_reference.clone()),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item |= draw_chimp_list(
                                ui,
                                id.with((index, "outputs")),
                                "Variadic outputs",
                                &mut function.variadic_outputs,
                                Some(default_reference),
                                |ui, _, value| {
                                    draw_chimp_fname(ui, "Name", &mut value.name, names)
                                        | ui.add(
                                            egui::DragValue::new(&mut value.underlying_type)
                                                .prefix("Type: "),
                                        )
                                        .changed()
                                },
                            );
                            item
                        },
                    );
                    inner
                }
                chimp_hw::HandWritten::InstancedPropertyBag(value) => {
                    let mut inner = ui
                        .add(egui::DragValue::new(&mut value.serial_size).prefix("Serial size: "))
                        .changed();
                    if let Some(descriptors) = &mut value.descriptors {
                        let default_descriptor = chimp_hw::PropertyBagDesc {
                            value_type_object: 0,
                            id: Default::default(),
                            name: blam_tags::iostore::object::value::FName::none(),
                            value_type: chimp_hw::PropertyBagPropertyType::None,
                            container_types: Vec::new(),
                        };
                        inner |= draw_chimp_list(
                            ui,
                            id.with("descriptors"),
                            "Descriptors",
                            descriptors,
                            Some(default_descriptor),
                            |ui, index, descriptor| {
                                let mut item = ui
                                    .add(
                                        egui::DragValue::new(&mut descriptor.value_type_object)
                                            .prefix("Type object: "),
                                    )
                                    .changed();
                                item |= draw_chimp_fname(ui, "Name", &mut descriptor.name, names);
                                ui.label("ID");
                                item |= draw_chimp_i32_values(ui, &mut descriptor.id.0);
                                ui.horizontal(|ui| {
                                    ui.label("Type");
                                    item |= draw_chimp_property_bag_type(
                                        ui,
                                        id.with((index, "type")),
                                        &mut descriptor.value_type,
                                    );
                                });
                                item |= draw_chimp_list(
                                    ui,
                                    id.with((index, "containers")),
                                    "Containers",
                                    &mut descriptor.container_types,
                                    Some(chimp_hw::PropertyBagContainerType::None),
                                    |ui, container, value| {
                                        draw_chimp_property_bag_container(
                                            ui,
                                            id.with((index, container)),
                                            value,
                                        )
                                    },
                                );
                                item
                            },
                        );
                    }
                    if let Some(values) = &mut value.values {
                        inner |= draw_chimp_property_block(ui, values, "", names, usmap, depth + 1);
                    }
                    inner
                }
            };
        });
    changed
}

fn chimp_class_display_name(class: Option<&str>) -> &str {
    class
        .and_then(|class| {
            class
                .rsplit(|character| character == '.' || character == '/')
                .find(|segment| !segment.is_empty())
        })
        .unwrap_or("Unknown")
}

fn chimp_object_index_json(
    document: &ChimpDocument,
    world: &World,
    index: FPackageObjectIndex,
) -> Value {
    let object_path = match index.kind() {
        FPackageObjectIndexType::ScriptImport => world
            .class_path(index.raw_index())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::PackageImport => index
            .package_import()
            .and_then(|reference| {
                let package = document
                    .header
                    .imported_package_names
                    .get(reference.imported_package_index as usize)?;
                let hash = document
                    .header
                    .imported_public_export_hashes
                    .get(reference.imported_public_export_hash_index as usize)?;
                Some(format!("{package}#{hash:016X}"))
            })
            .unwrap_or_else(|| format!("package import #{:016X}", index.raw_index())),
        FPackageObjectIndexType::Export => document
            .header
            .export_map
            .get(index.raw_index() as usize)
            .map(|export| {
                format!(
                    "{}.{}",
                    document.package,
                    document.header.name_map.get(export.object_name)
                )
            })
            .unwrap_or_else(|| format!("export #{}", index.raw_index())),
        FPackageObjectIndexType::Null => "None".to_owned(),
    };
    json!({
        "Kind": format!("{:?}", index.kind()),
        "RawIndex": format!("0x{:016X}", index.raw_index()),
        "ObjectPath": object_path,
    })
}

fn chimp_document_json(document: &ChimpDocument) -> Value {
    json!({
        "Package": document.package,
        "Source": document.provider.entry_path,
        "Summary": {
            "Exports": document.header.export_map.len(),
            "Imports": document.header.import_map.len(),
            "Names": document.header.name_map.copy_raw_names().len(),
            "OriginalSize": document.original.len(),
        },
        "Imports": document.header.imported_package_names,
        "ExternalDependencies": document.header.external_package_dependencies
            .iter()
            .map(|dependency| format!("{dependency:?}"))
            .collect::<Vec<_>>(),
        "Exports": document.exports.iter().map(|export| {
            json!({
                "Type": chimp_class_display_name(export.class.as_deref()),
                "Name": export.object,
                "Class": export.class,
                "Properties": export.decoded.as_ref().ok()
                    .and_then(Export::properties)
                    .map(chimp_block_json),
                "DecodeError": export.decoded.as_ref().err(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn chimp_metadata_json(document: &ChimpDocument, world: &World) -> Value {
    let header = &document.header;
    let summary = &header.summary;
    let record = world.package(&document.package);
    json!({
        "Summary": {
            "Package": document.package,
            "SourcePackage": header.source_package_name(),
            "PackageFlags": format!("0x{:08X}", summary.package_flags),
            "HeaderSize": summary.header_size,
            "CookedHeaderSize": summary.cooked_header_size,
            "OriginalSize": document.original.len(),
            "HasVersioningInfo": summary.has_versioning_info != 0,
            "IsUnversioned": header.is_unversioned,
            "ContainerHeaderVersion": format!("{:?}", header.container_header_version),
            "ExportCount": header.export_map.len(),
            "ImportCount": header.import_map.len(),
            "NameCount": header.name_map.copy_raw_names().len(),
            "BulkDataCount": header.bulk_data.len(),
            "ImportedPackageCount": header.imported_package_names.len(),
            "ExternalDependencyCount": header.external_package_dependencies.len(),
        },
        "Versioning": {
            "ZenVersion": format!("{:?}", header.versioning_info.zen_version),
            "FileVersionUE4": header.versioning_info.package_file_version.file_version_ue4,
            "FileVersionUE5": header.versioning_info.package_file_version.file_version_ue5,
            "LicenseeVersion": header.versioning_info.licensee_version,
            "CustomVersions": header.versioning_info.custom_versions.iter().map(|version| {
                json!({
                    "Key": format!("{:?}", version.key),
                    "Version": version.version,
                })
            }).collect::<Vec<_>>(),
        },
        "SectionOffsets": summary.section_offsets().iter().map(|(section, offset)| {
            json!({
                "Section": section,
                "Offset": offset,
            })
        }).collect::<Vec<_>>(),
        "NameMap": header.name_map.copy_raw_names(),
        "ImportedPackageNames": header.imported_package_names,
        "ImportedPublicExportHashes": header.imported_public_export_hashes.iter()
            .map(|hash| format!("0x{hash:016X}"))
            .collect::<Vec<_>>(),
        "ImportMap": header.import_map.iter().enumerate().map(|(index, object)| {
            json!({
                "Index": index,
                "Object": chimp_object_index_json(document, world, *object),
            })
        }).collect::<Vec<_>>(),
        "ExportMap": header.export_map.iter().enumerate().map(|(index, export)| {
            json!({
                "Index": index,
                "ObjectName": header.name_map.get(export.object_name).to_string(),
                "Class": world.class_key(header, export.class_index),
                "Outer": chimp_object_index_json(document, world, export.outer_index),
                "Super": chimp_object_index_json(document, world, export.super_index),
                "Template": chimp_object_index_json(document, world, export.template_index),
                "SerialOffset": export.cooked_serial_offset,
                "SerialSize": export.cooked_serial_size,
                "PublicExportHash": format!("0x{:016X}", export.public_export_hash),
                "ObjectFlags": format!("0x{:08X}", export.object_flags),
                "FilterFlags": format!("{:?}", export.filter_flags),
            })
        }).collect::<Vec<_>>(),
        "BulkDataMap": header.bulk_data.iter().enumerate().map(|(index, entry)| {
            json!({
                "Index": index,
                "SerialOffset": entry.serial_offset,
                "DuplicateSerialOffset": entry.duplicate_serial_offset,
                "SerialSize": entry.serial_size,
                "Flags": format!("0x{:08X}", entry.flags),
                "CookedIndex": entry.cooked_index,
            })
        }).collect::<Vec<_>>(),
        "ExternalPackageDependencies": header.external_package_dependencies.iter()
            .map(|dependency| {
                json!({
                    "FromPackageId": format!("0x{:016X}", dependency.from_package_id.0),
                    "ExternalArcs": dependency.external_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromImportIndex": arc.from_import_index,
                            "FromCommandType": format!("{:?}", arc.from_command_type),
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                    "LegacyArcs": dependency.legacy_dependency_arcs.iter().map(|arc| {
                        json!({
                            "FromExportBundleIndex": arc.from_export_bundle_index,
                            "ToExportBundleIndex": arc.to_export_bundle_index,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        "ShaderMapHashes": header.shader_map_hashes.iter()
            .map(|hash| format!("{hash:?}"))
            .collect::<Vec<_>>(),
        "PhysicalProviders": record.map(|record| {
            record.providers.iter().rev().map(|provider| {
                let container = &world.containers()[provider.container];
                json!({
                    "Active": record.active_provider() == Some(provider),
                    "Container": container.path.display().to_string(),
                    "EntryPath": provider.entry_path,
                    "ReadOrder": provider.read_order,
                    "RecoveredDirectoryIndex": container.recovered_directory_index,
                })
            }).collect::<Vec<_>>()
        }).unwrap_or_default(),
    })
}

fn refresh_chimp_document_text(document: &mut ChimpDocument) {
    document.document_text = serde_json::to_string_pretty(&chimp_document_json(document))
        .unwrap_or_else(|error| format!("Could not render package document: {error}"));
    document.document_line_numbers = chimp_line_numbers(&document.document_text);
    document.document_text_dirty = false;
}

/// The package header as structured rows: identity, the name map, the import
/// map, and the export map, each with who references it.
///
/// Read-only for now. The Metadata view beside it stays as it is and remains the
/// exhaustive dump — this one is the part a person can act on, and the counts
/// are what make an edit's blast radius visible before there is anything to
/// edit.
fn draw_chimp_header_view(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    world: &World,
    expert_mode: bool,
    scan_referrers: &mut bool,
) -> bool {
    if document.header_usage.is_none() {
        refresh_chimp_header_usage(document);
    }
    // Collected during the draw and applied after it: the rename needs `&mut`
    // access to the very header and exports the rows are reading from.
    let mut edits = ChimpHeaderEdits::default();
    draw_chimp_header_sections(ui, document, world, expert_mode, &mut edits);
    // Passed out rather than started here: the scan needs the application, and
    // this call is holding a mutable borrow of one of its documents.
    *scan_referrers = edits.scan_referrers;

    if edits.start_identity {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_export_edit = None;
        let versioning = &document.header.versioning_info;
        document.header_identity_edit = Some(ChimpIdentityEdit {
            package_flags: format!("{:08X}", document.header.summary.package_flags),
            licensee_version: versioning.licensee_version,
            is_unversioned: document.header.is_unversioned,
            zen_version: versioning.zen_version,
            file_version_ue4: versioning.package_file_version.file_version_ue4,
            file_version_ue5: versioning.package_file_version.file_version_ue5,
        });
        document.header_error = None;
    }
    if edits.commit_identity
        && let Some(edit) = document.header_identity_edit.take()
    {
        match apply_chimp_identity_edit(document, &edit) {
            Ok(()) => {
                document.header_error = None;
                return true;
            }
            Err(error) => {
                document.header_error = Some(error);
                document.header_identity_edit = Some(edit);
            }
        }
    }
    if let Some(index) = edits.start_export {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_identity_edit = None;
        document.header_export_edit = document.header.export_map.get(index).map(|entry| {
            ChimpExportEdit {
                index,
                object_name: document
                    .header
                    .name_map
                    .try_get(entry.object_name)
                    .map(|name| name.to_string())
                    .unwrap_or_default(),
                object_flags: entry.object_flags,
                filter_flags: entry.filter_flags,
                recompute_hash: false,
            }
        });
        document.header_error = None;
    }
    if edits.commit_export
        && let Some(edit) = document.header_export_edit.take()
    {
        match apply_chimp_export_edit(world, document, &edit) {
            Ok(()) => {
                document.header_error = None;
                return true;
            }
            Err(error) => {
                document.header_error = Some(error);
                document.header_export_edit = Some(edit);
            }
        }
    }
    if let Some((index, text)) = edits.start_name {
        document.header_import_edit = None;
        document.header_name_edit = Some(ChimpNameEdit {
            index,
            text,
            focus: true,
        });
        document.header_error = None;
    }
    if let Some((slot, current)) = edits.start_import {
        document.header_name_edit = None;
        document.header_import_edit = Some(chimp_import_edit_for(slot, &current, world));
        document.header_error = None;
    }
    if edits.cancel {
        document.header_name_edit = None;
        document.header_import_edit = None;
        document.header_export_edit = None;
        document.header_identity_edit = None;
        document.header_error = None;
    }
    if let Some((index, text)) = edits.commit_name {
        match apply_chimp_name_rename(document, index, &text) {
            Ok(()) => {
                document.header_name_edit = None;
                document.header_error = None;
                return true;
            }
            Err(error) => document.header_error = Some(error),
        }
    }
    if let Some((index, slot)) = edits.commit_import {
        match apply_chimp_import_slot(document, index, slot) {
            Ok(()) => {
                document.header_import_edit = None;
                document.header_error = None;
                return true;
            }
            Err(error) => document.header_error = Some(error),
        }
    }
    false
}

/// What one draw of the Header view asked for, applied once its borrows are
/// gone.
#[derive(Default)]
struct ChimpHeaderEdits {
    start_name: Option<(usize, String)>,
    commit_name: Option<(usize, String)>,
    start_import: Option<(usize, ImportSlot)>,
    commit_import: Option<(usize, ImportSlot)>,
    start_export: Option<usize>,
    commit_export: bool,
    start_identity: bool,
    commit_identity: bool,
    scan_referrers: bool,
    cancel: bool,
}

/// Seed a draft from the slot as it stands, so opening the editor shows what is
/// there rather than an empty form.
fn chimp_import_edit_for(slot: usize, current: &ImportSlot, world: &World) -> ChimpImportEdit {
    let mut edit = ChimpImportEdit {
        slot,
        kind: ChimpImportKind::Null,
        script_path: String::new(),
        package_path: String::new(),
        object_name: String::new(),
        resolved: None,
    };
    match current {
        ImportSlot::Script(index) => {
            edit.kind = ChimpImportKind::Script;
            // Only the mount can name a script hash; an unknown one is left
            // blank rather than filled with something invented.
            edit.script_path = world.class_path(index.raw_index()).unwrap_or_default().to_owned();
        }
        ImportSlot::Package(target) => {
            edit.kind = ChimpImportKind::Package;
            edit.package_path = target.package.clone();
            // The object name is not in the file — only its hash is. Recover it
            // from the package that holds the export, and leave it blank when
            // that is not possible rather than guessing.
            if let Ok(exports) = chimp_public_exports_of(world, &target.package) {
                edit.object_name = exports
                    .iter()
                    .find(|(_, hash)| *hash == target.object_hash)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                edit.resolved = Some(Ok(exports));
            }
        }
        ImportSlot::Null => {}
    }
    edit
}

fn draw_chimp_header_sections(
    ui: &mut Ui,
    document: &mut ChimpDocument,
    world: &World,
    expert_mode: bool,
    edits: &mut ChimpHeaderEdits,
) {
    let ChimpHeaderEdits {
        start_name: start,
        commit_name: commit,
        start_import,
        commit_import,
        start_export,
        commit_export,
        start_identity,
        commit_identity,
        scan_referrers,
        cancel,
    } = edits;
    let usage = document
        .header_usage
        .as_ref()
        .expect("refreshed by the caller");
    let header = &document.header;

    egui::ScrollArea::vertical()
        .id_salt(("chimp_header_view", document.package.clone()))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::CollapsingHeader::new("Identity")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(mut edit) = document.header_identity_edit.take() {
                        draw_chimp_identity_panel(
                            ui,
                            &mut edit,
                            expert_mode,
                            document.header_error.as_deref(),
                            commit_identity,
                            cancel,
                        );
                        ui.add_space(4.0);
                        document.header_identity_edit = Some(edit);
                    } else if ui.button("Edit flags and versioning...").clicked() {
                        *start_identity = true;
                    }
                    egui::Grid::new("chimp_header_identity")
                        .num_columns(2)
                        .spacing([18.0, 4.0])
                        .show(ui, |ui| {
                            header_row(ui, "Package", &document.package);
                            header_row(
                                ui,
                                "Package flags",
                                &format!("0x{:08X}", header.summary.package_flags),
                            );
                            header_row(ui, "Unversioned", &header.is_unversioned.to_string());
                            header_row(
                                ui,
                                "Zen version",
                                &format!("{:?}", header.versioning_info.zen_version),
                            );
                            header_row(
                                ui,
                                "Engine version",
                                &format!(
                                    "UE4 {} · UE5 {}",
                                    header.versioning_info.package_file_version.file_version_ue4,
                                    header.versioning_info.package_file_version.file_version_ue5
                                ),
                            );
                        });
                });

            let names = header.name_map.names();
            egui::CollapsingHeader::new(format!("Name map ({})", names.len()))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filter").color(subtle_dark()).small());
                        ui.add(
                            egui::TextEdit::singleline(&mut document.header_name_filter)
                                .desired_width(240.0)
                                .hint_text("substring"),
                        );
                    });
                    // The editor sits above the list rather than inside it: the
                    // rows are virtualised, so an in-row editor would scroll out
                    // from under the user mid-edit.
                    // Taken out rather than borrowed in place: the panel reads
                    // the rest of the document (to price the edit) while it
                    // writes the draft, and those cannot be the same borrow.
                    if let Some(mut edit) = document.header_name_edit.take() {
                        let current = names.get(edit.index).cloned().unwrap_or_default();
                        let entry_usage = usage.names.get(edit.index).cloned().unwrap_or_default();
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("Editing entry {} · {current}", edit.index))
                                    .strong(),
                            );
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut edit.text)
                                    .id(egui::Id::new(("chimp_name_edit", edit.index)))
                                    .desired_width(320.0)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.focus {
                                response.request_focus();
                                edit.focus = false;
                            }
                            let submitted = response.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter));

                            // The blast radius, before the change rather than
                            // after it. Editing by index is the useful semantic
                            // precisely because it moves everything at once,
                            // which is also what makes it worth seeing first.
                            ui.label(
                                RichText::new(match entry_usage.count {
                                    0 => "Nothing references this entry.".to_owned(),
                                    1 => "1 reference will follow this rename:".to_owned(),
                                    n => format!("{n} references will follow this rename:"),
                                })
                                .color(subtle_dark())
                                .small(),
                            );
                            for site in &entry_usage.sites {
                                ui.label(
                                    RichText::new(format!("    • {site}"))
                                        .color(subtle_dark())
                                        .small(),
                                );
                            }
                            // Renaming the object an export is named after does
                            // not update the hash other packages address it by.
                            // Recoverable, but only if someone knows.
                            let desyncs =
                                chimp_export_hash_desyncs(header, edit.index, edit.text.trim());
                            if !desyncs.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "Export {} is named after this entry. Its public export \
                                         hash will no longer match the new name, so packages that \
                                         import it by hash keep resolving the old one.",
                                        desyncs
                                            .iter()
                                            .map(usize::to_string)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ))
                                    .color(Color32::from_rgb(170, 130, 60))
                                    .small(),
                                );
                            }

                            if let Some(error) = document.header_error.as_deref() {
                                ui.label(
                                    RichText::new(error)
                                        .color(Color32::from_rgb(150, 56, 44))
                                        .small(),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Apply").clicked() || submitted {
                                    *commit = Some((edit.index, edit.text.clone()));
                                }
                                if ui.button("Cancel").clicked() {
                                    *cancel = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        document.header_name_edit = Some(edit);
                    }

                    let filter = document.header_name_filter.trim().to_ascii_lowercase();
                    let rows: Vec<usize> = names
                        .iter()
                        .enumerate()
                        .filter(|(_, name)| {
                            filter.is_empty() || name.to_ascii_lowercase().contains(&filter)
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if rows.is_empty() {
                        ui.label(RichText::new("No matching names").color(subtle_dark()));
                        return;
                    }
                    let row_height = ui.spacing().interact_size.y;
                    // Virtualised: a large package's name map runs to thousands
                    // of entries, and this view is drawn every frame.
                    egui::ScrollArea::vertical()
                        .id_salt("chimp_header_names")
                        .max_height(260.0)
                        .show_rows(ui, row_height, rows.len(), |ui, range| {
                            egui::Grid::new("chimp_header_name_rows")
                                .num_columns(3)
                                .spacing([14.0, 2.0])
                                .show(ui, |ui| {
                                    for &index in &rows[range] {
                                        let usage = usage
                                            .names
                                            .get(index)
                                            .cloned()
                                            .unwrap_or_default();
                                        ui.label(
                                            RichText::new(format!("{index}"))
                                                .color(subtle_dark())
                                                .monospace(),
                                        );
                                        let mut label = RichText::new(&names[index]).monospace();
                                        if usage.is_package_identity {
                                            label = label.color(foundation_blue());
                                        } else if usage.count == 0 {
                                            label = label.color(subtle_dark());
                                        }
                                        // The package's own name is not editable
                                        // here — see `apply_chimp_name_rename`.
                                        // Shown as a plain label so there is no
                                        // affordance to reach for.
                                        if usage.is_package_identity {
                                            ui.label(label)
                                                .on_hover_text(name_usage_tooltip(&usage));
                                        } else if ui
                                            .add(
                                                egui::Label::new(label)
                                                    .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text(name_usage_tooltip(&usage))
                                            .clicked()
                                        {
                                            *start = Some((index, names[index].clone()));
                                        }
                                        ui.label(
                                            RichText::new(match usage.count {
                                                0 => "unreferenced".to_owned(),
                                                1 => "1 reference".to_owned(),
                                                n => format!("{n} references"),
                                            })
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                });

            let slots = read_import_slots(header);
            egui::CollapsingHeader::new(format!("Import map ({})", header.import_map.len()))
                .default_open(true)
                .show(ui, |ui| match &slots {
                    Ok(slots) => {
                        if let Some(mut edit) = document.header_import_edit.take() {
                            draw_chimp_import_editor(
                                ui,
                                &mut edit,
                                world,
                                document.header_error.as_deref(),
                                commit_import,
                                cancel,
                            );
                            ui.add_space(4.0);
                            document.header_import_edit = Some(edit);
                        }
                        egui::Grid::new("chimp_header_imports")
                            .num_columns(4)
                            .spacing([14.0, 2.0])
                            .show(ui, |ui| {
                                for (index, slot) in slots.iter().enumerate() {
                                    ui.label(
                                        RichText::new(format!("{index}"))
                                            .color(subtle_dark())
                                            .monospace(),
                                    );
                                    let (kind, target) = import_slot_display(slot, world);
                                    ui.label(RichText::new(kind).color(subtle_dark()).small());
                                    if ui
                                        .add(
                                            egui::Label::new(RichText::new(target).monospace())
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Retarget this import slot")
                                        .clicked()
                                    {
                                        *start_import = Some((index, slot.clone()));
                                    }
                                    let references = usage
                                        .import_references
                                        .get(index)
                                        .copied()
                                        .unwrap_or_default();
                                    ui.label(
                                        RichText::new(match references {
                                            0 => "unreferenced".to_owned(),
                                            1 => "1 property".to_owned(),
                                            n => format!("{n} properties"),
                                        })
                                        .color(subtle_dark())
                                        .small(),
                                    );
                                    ui.end_row();
                                }
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("+ Add import slot").clicked() {
                                *start_import = Some((slots.len(), ImportSlot::Null));
                            }
                            ui.label(
                                RichText::new(
                                    "Slots cannot be removed: every object property names one by \
                                     position, so dropping one would shift every reference above \
                                     it.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        });
                    }
                    Err(error) => {
                        ui.label(
                            RichText::new(format!("Import map is not resolvable: {error:#}"))
                                .color(Color32::from_rgb(150, 56, 44)),
                        );
                    }
                });

            egui::CollapsingHeader::new(format!("Export map ({})", header.export_map.len()))
                .default_open(false)
                .show(ui, |ui| {
                    if let Some(mut edit) = document.header_export_edit.take() {
                        draw_chimp_export_edit_panel(
                            ui,
                            &mut edit,
                            header,
                            expert_mode,
                            document.header_error.as_deref(),
                            commit_export,
                            cancel,
                        );
                        ui.add_space(4.0);
                        document.header_export_edit = Some(edit);
                    }
                    egui::Grid::new("chimp_header_exports")
                        .num_columns(4)
                        .spacing([14.0, 2.0])
                        .show(ui, |ui| {
                            for (index, export) in header.export_map.iter().enumerate() {
                                ui.label(
                                    RichText::new(format!("{index}"))
                                        .color(subtle_dark())
                                        .monospace(),
                                );
                                if ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(
                                                header
                                                    .name_map
                                                    .try_get(export.object_name)
                                                    .map(|name| name.to_string())
                                                    .unwrap_or_else(|| {
                                                        "<bad name reference>".to_owned()
                                                    }),
                                            )
                                            .monospace(),
                                        )
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Edit this export's name and flags")
                                    .clicked()
                                {
                                    *start_export = Some(index);
                                }
                                ui.label(
                                    RichText::new(
                                        world
                                            .class_key(header, export.class_index)
                                            .unwrap_or_else(|| "Unknown class".to_owned()),
                                    )
                                    .color(subtle_dark())
                                    .small(),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "flags 0x{:08X} · hash 0x{:016X}",
                                        export.object_flags, export.public_export_hash
                                    ))
                                    .color(subtle_dark())
                                    .small(),
                                );
                                ui.end_row();
                            }
                        });
                });

            // The inverse of the import map, and the only question here that
            // cannot be answered from this package alone.
            egui::CollapsingHeader::new("Referenced by")
                .default_open(false)
                .show(ui, |ui| match &document.referrers {
                    ChimpReferrerState::Idle => {
                        ui.label(
                            RichText::new(
                                "The paks carry no reverse index, so this means reading every \
                                 mounted package header once.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                        if ui.button("Find packages that import this").clicked() {
                            *scan_referrers = true;
                        }
                    }
                    ChimpReferrerState::Scanning => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Reading package headers...")
                                    .color(subtle_dark())
                                    .small(),
                            );
                        });
                    }
                    ChimpReferrerState::Done(scan) => {
                        ui.label(
                            RichText::new(match scan.referrers.len() {
                                0 => format!("No hard import, of {} packages read", scan.scanned),
                                1 => format!("1 package of {} imports this", scan.scanned),
                                n => format!("{n} packages of {} import this", scan.scanned),
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                        if scan.referrers.is_empty() {
                            // Deliberately not phrased as "nothing references
                            // this". A soft object reference is FName indices
                            // inside an export's serial data, resolved by name
                            // at runtime; it never appears in the import table
                            // this scan reads, and the Zen summary has no
                            // soft-reference section to read instead. So a zero
                            // here rules out one kind of reference, not all of
                            // them, and it is not evidence that moving the
                            // package is safe.
                            ui.label(
                                RichText::new(
                                    "Soft references live in export data and are not counted.",
                                )
                                .color(subtle_dark())
                                .small(),
                            );
                        }
                        if scan.unreadable > 0 {
                            // "Nothing imports this" and "nothing I could read
                            // imports this" are different answers, and only one
                            // of them makes a rename safe.
                            ui.label(
                                RichText::new(format!(
                                    "{} package(s) could not be read and are not ruled out.",
                                    scan.unreadable
                                ))
                                .color(Color32::from_rgb(170, 130, 60))
                                .small(),
                            );
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("chimp_header_referrers")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for referrer in &scan.referrers {
                                    ui.label(RichText::new(referrer).monospace());
                                }
                            });
                        if ui.button("Scan again").clicked() {
                            *scan_referrers = true;
                        }
                    }
                });

            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Serial offsets, sizes and section offsets are recomputed when the package is \
                     written, so they are not shown here — the Metadata view carries them as they \
                     stand.",
                )
                .color(subtle_dark())
                .small(),
            );
        });
}

/// The draft panel for the package's flags and versioning.
fn draw_chimp_identity_panel(
    ui: &mut Ui,
    edit: &mut ChimpIdentityEdit,
    expert_mode: bool,
    error: Option<&str>,
    commit: &mut bool,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new("Flags and versioning").strong());

        ui.label(RichText::new("Package flags").color(subtle_dark()).small());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut edit.package_flags)
                    .desired_width(120.0)
                    .font(egui::TextStyle::Monospace),
            );
            // The two values measured across every shipped Campaign Evolved tag
            // package, so the common cases need no hex at all.
            if ui
                .button("0x80002200")
                .on_hover_text("Every shipped tag group except the five cooked per level")
                .clicked()
            {
                edit.package_flags = "80002200".to_owned();
            }
            if ui
                .button("0x88002200")
                .on_hover_text("The _Generated_ level groups, which carry PKG_CookGenerated")
                .clicked()
            {
                edit.package_flags = "88002200".to_owned();
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Licensee version").color(subtle_dark()).small());
            ui.add(egui::DragValue::new(&mut edit.licensee_version));
        });

        ui.add_space(4.0);
        if expert_mode {
            ui.label(
                RichText::new(
                    "These decide the header's shape, not just what it says. Lowering the UE5 \
                     file version stops the bulk-data map being written at all.",
                )
                .color(Color32::from_rgb(170, 130, 60))
                .small(),
            );
            ui.checkbox(&mut edit.is_unversioned, "Unversioned");
            if edit.is_unversioned {
                // Worth stating rather than leaving the fields looking live: an
                // unversioned package stores no versioning block, so the reader
                // synthesises these and nothing typed below is kept.
                ui.label(
                    RichText::new(
                        "An unversioned package carries no versioning block, so the fields below \
                         are not stored — the loader infers them. Uncheck this to make them real.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Zen version").color(subtle_dark()).small());
                for option in [
                    EZenPackageVersion::Initial,
                    EZenPackageVersion::DataResourceTable,
                    EZenPackageVersion::ImportedPackageNames,
                    EZenPackageVersion::ExportDependencies,
                ] {
                    ui.selectable_value(&mut edit.zen_version, option, format!("{option:?}"));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("File version UE4").color(subtle_dark()).small());
                ui.add(egui::DragValue::new(&mut edit.file_version_ue4));
                ui.label(RichText::new("UE5").color(subtle_dark()).small());
                ui.add(egui::DragValue::new(&mut edit.file_version_ue5));
            });
        } else {
            ui.label(
                RichText::new(
                    "Versioning fields are Expert mode only: they select the header's format, so \
                     a wrong one produces a package that writes cleanly and loads as something \
                     else.",
                )
                .color(subtle_dark())
                .small(),
            );
        }

        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Applying writes the package and reads it back first. If anything does not \
                 survive that, the change is refused rather than saved.",
            )
            .color(subtle_dark())
            .small(),
        );
        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                *commit = true;
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

/// The five `EObjectFlags` this crate names, as `(bit, label)`.
const CHIMP_OBJECT_FLAG_BITS: [(u32, &str); 5] = [
    (0x0000_0001, "Public"),
    (0x0000_0002, "Standalone"),
    (0x0000_0008, "Transactional"),
    (0x0000_0010, "ClassDefaultObject"),
    (0x0000_0020, "ArchetypeObject"),
];

/// The draft panel for one export-map entry.
fn draw_chimp_export_edit_panel(
    ui: &mut Ui,
    edit: &mut ChimpExportEdit,
    header: &FZenPackageHeader,
    expert_mode: bool,
    error: Option<&str>,
    commit: &mut bool,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new(format!("Export {}", edit.index)).strong());

        ui.label(RichText::new("Object name").color(subtle_dark()).small());
        ui.add(
            egui::TextEdit::singleline(&mut edit.object_name)
                .desired_width(320.0)
                .font(egui::TextStyle::Monospace),
        );

        // The engine resolves an asset as `Package.Object`, so an export named
        // after the package leaf is the one that makes the package loadable
        // under its own path.
        let leaf = header
            .package_name()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let was_leaf = header
            .export_map
            .get(edit.index)
            .and_then(|entry| header.name_map.try_get(entry.object_name))
            .is_some_and(|name| *name == leaf);
        if was_leaf && edit.object_name.trim() != leaf {
            ui.label(
                RichText::new(format!(
                    "This export is named after the package ({leaf}). The engine resolves the \
                     asset as {}.{leaf} — renaming it here leaves nothing at that path.",
                    header.package_name()
                ))
                .color(Color32::from_rgb(170, 130, 60))
                .small(),
            );
        }

        let stored_hash = header
            .export_map
            .get(edit.index)
            .map(|entry| entry.public_export_hash)
            .unwrap_or_default();
        let new_hash = public_export_hash(edit.object_name.trim());
        if stored_hash != new_hash {
            ui.label(
                RichText::new(format!(
                    "Public export hash 0x{stored_hash:016X} was computed from the old name. \
                     Other packages import this export by that hash and are not updated either \
                     way."
                ))
                .color(subtle_dark())
                .small(),
            );
            if expert_mode {
                ui.checkbox(
                    &mut edit.recompute_hash,
                    format!("Recompute it as 0x{new_hash:016X} (importers keep the old one)"),
                );
            }
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Object flags").color(subtle_dark()).small());
        ui.horizontal_wrapped(|ui| {
            for (bit, label) in CHIMP_OBJECT_FLAG_BITS {
                let mut set = edit.object_flags & bit != 0;
                if ui.checkbox(&mut set, label).changed() {
                    if set {
                        edit.object_flags |= bit;
                    } else {
                        edit.object_flags &= !bit;
                    }
                }
            }
        });
        let named: u32 = CHIMP_OBJECT_FLAG_BITS.iter().map(|(bit, _)| bit).sum();
        let remainder = edit.object_flags & !named;
        ui.label(
            RichText::new(format!(
                "0x{:08X}{}",
                edit.object_flags,
                if remainder != 0 {
                    format!(" · 0x{remainder:08X} unnamed, preserved")
                } else {
                    String::new()
                }
            ))
            .color(subtle_dark())
            .small(),
        );
        ui.label(
            RichText::new(
                "Object flags decide how the payload is read, not just how it is labelled. \
                 Applying re-reads this export and refuses the change if it no longer decodes.",
            )
            .color(subtle_dark())
            .small(),
        );

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Filter flags").color(subtle_dark()).small());
            for option in [
                EExportFilterFlags::None,
                EExportFilterFlags::NotForClient,
                EExportFilterFlags::NotForServer,
            ] {
                ui.selectable_value(&mut edit.filter_flags, option, format!("{option:?}"));
            }
        });

        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                *commit = true;
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

/// The draft panel for one import slot.
fn draw_chimp_import_editor(
    ui: &mut Ui,
    edit: &mut ChimpImportEdit,
    world: &World,
    error: Option<&str>,
    commit: &mut Option<(usize, ImportSlot)>,
    cancel: &mut bool,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(RichText::new(format!("Import slot {}", edit.slot)).strong());
        ui.horizontal(|ui| {
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Script, "Script");
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Package, "Package");
            ui.selectable_value(&mut edit.kind, ChimpImportKind::Null, "Null");
        });

        match edit.kind {
            ChimpImportKind::Script => {
                ui.label(
                    RichText::new("Object path, e.g. /Script/Engine.StaticMesh")
                        .color(subtle_dark())
                        .small(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut edit.script_path)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.label(
                    RichText::new(
                        "Stored as a one-way hash of this path. If the module is not mounted, \
                         Baboon cannot name it back — it will read as unknown.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
            }
            ChimpImportKind::Package => {
                ui.label(
                    RichText::new("Package path, e.g. /Game/Art/Meshes/SM_Crate")
                        .color(subtle_dark())
                        .small(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut edit.package_path)
                        .desired_width(420.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Object name").color(subtle_dark()).small());
                    ui.add(
                        egui::TextEdit::singleline(&mut edit.object_name)
                            .desired_width(260.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui
                        .button("List exports")
                        .on_hover_text("Read the target package and list what it exports publicly")
                        .clicked()
                    {
                        edit.resolved = Some(chimp_public_exports_of(world, &edit.package_path));
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Stored as hash 0x{:016X}",
                        public_export_hash(edit.object_name.trim())
                    ))
                    .color(subtle_dark())
                    .small(),
                );
                match &edit.resolved {
                    Some(Ok(exports)) if exports.is_empty() => {
                        ui.label(
                            RichText::new("That package exports nothing publicly.")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                    Some(Ok(exports)) => {
                        let mut pick = None;
                        egui::ScrollArea::vertical()
                            .id_salt("chimp_import_exports")
                            .max_height(140.0)
                            .show(ui, |ui| {
                                for (name, hash) in exports {
                                    if ui
                                        .selectable_label(
                                            public_export_hash(edit.object_name.trim()) == *hash,
                                            RichText::new(name).monospace(),
                                        )
                                        .clicked()
                                    {
                                        pick = Some(name.clone());
                                    }
                                }
                            });
                        if let Some(name) = pick {
                            edit.object_name = name;
                        }
                    }
                    Some(Err(error)) => {
                        ui.label(
                            RichText::new(error)
                                .color(Color32::from_rgb(150, 56, 44))
                                .small(),
                        );
                    }
                    None => {}
                }
            }
            ChimpImportKind::Null => {
                ui.label(
                    RichText::new("The slot resolves to nothing. Properties naming it read as None.")
                        .color(subtle_dark())
                        .small(),
                );
            }
        }

        ui.label(
            RichText::new(
                "Applying rebuilds the imported-package list, which is sorted by package id — the \
                 Metadata view will show it reordered. Dependency arcs are not rebuilt.",
            )
            .color(subtle_dark())
            .small(),
        );
        if let Some(error) = error {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(150, 56, 44))
                    .small(),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                let slot = match edit.kind {
                    ChimpImportKind::Script => Some(ImportSlot::Script(
                        FPackageObjectIndex::create_script_import(edit.script_path.trim()),
                    )),
                    ChimpImportKind::Package => Some(ImportSlot::Package(ImportTarget {
                        package: edit.package_path.trim().to_owned(),
                        object_hash: public_export_hash(edit.object_name.trim()),
                    })),
                    ChimpImportKind::Null => Some(ImportSlot::Null),
                };
                if let Some(slot) = slot {
                    *commit = Some((edit.slot, slot));
                }
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
    });
}

fn header_row(ui: &mut Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).color(subtle_dark()).small());
    ui.label(RichText::new(value).monospace());
    ui.end_row();
}

fn name_usage_tooltip(usage: &ChimpNameUsage) -> String {
    let mut lines = Vec::new();
    if usage.is_package_identity {
        lines.push(
            "This entry is the package's own name. The container addresses the package by a hash \
             of it, so changing it here would not move the chunk that serves it."
                .to_owned(),
        );
    }
    match usage.count {
        0 => lines.push("Nothing references this entry.".to_owned()),
        _ => {
            lines.push(format!("Referenced {} time(s) by:", usage.count));
            lines.extend(usage.sites.iter().map(|site| format!("  • {site}")));
            if usage.sites.len() < usage.count && usage.sites.len() == 8 {
                lines.push("  • …".to_owned());
            }
        }
    }
    lines.join("\n")
}

fn import_slot_display(slot: &ImportSlot, world: &World) -> (String, String) {
    match slot {
        ImportSlot::Script(index) => (
            "script".to_owned(),
            world
                .class_path(index.raw_index())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("unknown to this mount (0x{:016X})", index.raw_index())),
        ),
        ImportSlot::Package(target) => (
            "package".to_owned(),
            format!("{}#{:016X}", target.package, target.object_hash),
        ),
        ImportSlot::Null => ("null".to_owned(), "None".to_owned()),
    }
}

fn refresh_chimp_metadata_text(document: &mut ChimpDocument, world: &World) {
    document.metadata_text = serde_json::to_string_pretty(&chimp_metadata_json(document, world))
        .unwrap_or_else(|error| format!("Could not render package metadata: {error}"));
    document.metadata_line_numbers = chimp_line_numbers(&document.metadata_text);
    document.metadata_text_dirty = false;
}

fn chimp_block_json(block: &PropertyBlock) -> Value {
    Value::Object(
        block
            .iter()
            .map(|(name, value)| (name.to_owned(), chimp_value_json(value)))
            .collect(),
    )
}

fn chimp_value_json(value: &PropValue) -> Value {
    match value {
        PropValue::Bool(value) => json!(value),
        PropValue::Int(value) => json!(value),
        PropValue::Float(value) => json!(value),
        PropValue::Name(value) => json!(value.to_string()),
        PropValue::Str(value) => json!(value.to_string()),
        PropValue::Object(value) => json!({"object_index": value}),
        PropValue::SoftObject(value) => json!({
            "package": value.package.to_string(),
            "asset": value.asset.to_string(),
            "sub_path": value.sub_path.to_string(),
        }),
        PropValue::Array(values) | PropValue::Set(values) => {
            Value::Array(values.iter().map(chimp_value_json).collect())
        }
        PropValue::Map(values) => Value::Array(
            values
                .iter()
                .map(|(key, value)| {
                    json!({
                        "key": chimp_value_json(key),
                        "value": chimp_value_json(value),
                    })
                })
                .collect(),
        ),
        PropValue::Struct(block) => chimp_block_json(block),
        PropValue::Raw(bytes) => json!({"unknown_bytes": bytes.len()}),
        other => json!({"type": format!("{other:?}")}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::iostore::package::name_map::{EMappedNameType, FNameMap};
    use blam_tags::iostore::package::zen::{
        EExportCommandType, FDependencyBundleHeader, FExportBundleEntry, FExportMapEntry,
    };

    /// A header with `names` interned and one export named after the first.
    fn header_with_names(names: &[&str]) -> FZenPackageHeader {
        let mut header = FZenPackageHeader {
            container_header_version: CE_HEADER_VERSION,
            is_unversioned: true,
            ..Default::default()
        };
        header.name_map = FNameMap::create(EMappedNameType::Package);
        for name in names {
            header.name_map.store(name);
        }
        header.summary.name = FMappedName::create(0, EMappedNameType::Package, 0);
        header.export_map = vec![FExportMapEntry {
            cooked_serial_offset: 0,
            cooked_serial_size: 0,
            object_name: FMappedName::create(0, EMappedNameType::Package, 0),
            outer_index: FPackageObjectIndex::default(),
            class_index: FPackageObjectIndex::default(),
            super_index: FPackageObjectIndex::default(),
            template_index: FPackageObjectIndex::default(),
            public_export_hash: 0,
            object_flags: 0,
            filter_flags: EExportFilterFlags::None,
            padding: [0; 3],
        }];
        // The reader enforces a Create and a Serialize command per export, plus
        // a dependency bundle header. A header without them is not a package —
        // which the reopen gate rightly refuses, so the fixture has to be one.
        for command_type in [EExportCommandType::Create, EExportCommandType::Serialize] {
            header.export_bundle_entries.push(FExportBundleEntry {
                local_export_index: 0,
                command_type,
            });
        }
        header
            .dependency_bundle_headers
            .push(FDependencyBundleHeader::default());
        header
    }

    /// `FNameMap::get` indexes its slice directly, so a reference past the end
    /// of the table is a panic rather than an error — it would take the whole
    /// application down on the next draw. This is the check that turns that into
    /// a refused save.
    #[test]
    fn a_name_reference_past_the_end_of_the_table_is_refused_not_fatal() {
        let mut header = header_with_names(&["Warthog"]);
        assert!(validate_chimp_header_parts("pkg", &header, 1, &[]).is_ok());

        header.summary.name = FMappedName::create(7, EMappedNameType::Package, 0);
        let error = validate_chimp_header_parts("pkg", &header, 1, &[]).unwrap_err();
        assert!(error.contains("package name points outside"), "{error}");

        let mut header = header_with_names(&["Warthog"]);
        header.export_map[0].object_name = FMappedName::create(7, EMappedNameType::Package, 0);
        let error = validate_chimp_header_parts("pkg", &header, 1, &[]).unwrap_err();
        assert!(error.contains("export 0's object name"), "{error}");
    }

    /// `write_package` pairs payloads with export map entries positionally and
    /// errors on a mismatch; catching it here names the package instead.
    #[test]
    fn a_payload_count_that_does_not_match_the_export_map_is_refused() {
        let header = header_with_names(&["Warthog"]);
        let error = validate_chimp_header_parts("pkg", &header, 0, &[]).unwrap_err();
        assert!(error.contains("0 export payloads for 1"), "{error}");
    }

    /// An object property addresses an import by position, so a reference past
    /// the end of the import map would resolve to nothing at load.
    #[test]
    fn an_object_reference_past_the_end_of_the_import_map_is_refused() {
        let block = PropertyBlock {
            entries: vec![blam_tags::iostore::object::value::PropertyEntry {
                name: "Thing".into(),
                // `-1` is slot 0, `-4` is slot 3 — past an empty import map.
                value: PropValue::Object(-4),
                slot: None,
            }],
            layout: blam_tags::iostore::object::value::BlockLayout::Unversioned {
                schema_len: 1,
                leading_empty: 0,
            },
        };
        assert_eq!(first_unresolvable_object_reference(&block, 0), Some(3));
        assert_eq!(first_unresolvable_object_reference(&block, 4), None);
    }

    /// A document with two names, an export named after the second, and one
    /// reflected property holding `FName`s that point at it.
    fn rename_fixture() -> ChimpDocument {
        use blam_tags::iostore::object::value::{BlockLayout, PropertyEntry};

        let mut header = header_with_names(&["Warthog", "Material"]);
        // The export is named after entry 1, leaving entry 0 as the package's
        // own identity — the one the guard refuses.
        header.export_map[0].object_name = FMappedName::create(1, EMappedNameType::Package, 0);
        header.export_map[0].public_export_hash = public_export_hash("Material");

        let block = PropertyBlock {
            entries: vec![
                PropertyEntry {
                    name: "Plain".into(),
                    value: PropValue::Name(FName::new(1, 0, "Material")),
                    slot: None,
                },
                PropertyEntry {
                    name: "Numbered".into(),
                    // Number 3 renders as `_2`, which is how the reader composes
                    // it — the refresh has to reproduce that, not just the base.
                    value: PropValue::Array(vec![PropValue::Name(FName::new(1, 3, "Material_2"))]),
                    slot: None,
                },
                PropertyEntry {
                    name: "Other".into(),
                    value: PropValue::Name(FName::new(0, 0, "Warthog")),
                    slot: None,
                },
            ],
            layout: BlockLayout::Unversioned {
                schema_len: 3,
                leading_empty: 0,
            },
        };

        ChimpDocument {
            package: "/Game/Test/Thing".to_owned(),
            provider: PackageProvider {
                container: 0,
                entry_path: "Content/Test/Thing.uasset".to_owned(),
                read_order: 0,
            },
            original: Vec::new(),
            header,
            payloads: vec![Vec::new()],
            exports: vec![ChimpExport {
                object: "Material".to_owned(),
                class: None,
                decoded: Ok(Export {
                    block: ExportBlock::Reflected(block),
                    trailer: blam_tags::iostore::object::export::Trailer::NoGuid,
                    tail: Vec::new(),
                }),
            }],
            texture_previews: Vec::new(),
            mesh_kind: None,
            mesh_preview: None,
            mesh_preview_state: Default::default(),
            selected_export: 0,
            dirty: false,
            view: ChimpDocumentView::Header,
            document_text: String::new(),
            document_line_numbers: String::new(),
            document_text_dirty: false,
            metadata_text: String::new(),
            metadata_line_numbers: String::new(),
            metadata_text_dirty: false,
            header_usage: None,
            header_name_filter: String::new(),
            header_name_edit: None,
            header_import_edit: None,
            header_export_edit: None,
            header_identity_edit: None,
            header_error: None,
            referrers: ChimpReferrerState::Idle,
            orphaned: false,
        }
    }

    /// The package's own name is its identity, and the chunk that serves it is
    /// addressed by a hash of that string — so renaming it here would leave the
    /// package claiming to be something no chunk id matches.
    #[test]
    fn renaming_the_packages_own_name_is_refused() {
        let mut document = rename_fixture();
        let error = apply_chimp_name_rename(&mut document, 0, "Scorpion").unwrap_err();
        assert!(error.contains("package's own name"), "{error}");
        assert_eq!(document.header.name_map.names(), ["Warthog", "Material"]);
    }

    /// The silent-fork case. An `FName` is written as an index, so the rename
    /// lands on disk either way — but the resolved text a decoded value carries
    /// is what a later edit interns *by string*, so a stale one would fork a new
    /// entry instead of following the rename.
    #[test]
    fn a_rename_refreshes_every_resolved_name_including_numbered_ones() {
        let mut document = rename_fixture();
        apply_chimp_name_rename(&mut document, 1, "Surface").unwrap();

        assert_eq!(document.header.name_map.names(), ["Warthog", "Surface"]);
        // The export's cached display name.
        assert_eq!(document.exports[0].object, "Surface");

        let mut seen = Vec::new();
        let Ok(decoded) = &mut document.exports[0].decoded else {
            panic!("fixture decodes");
        };
        let ExportBlock::Reflected(block) = &mut decoded.block else {
            panic!("fixture is reflected");
        };
        block.visit_names_mut(&mut |name| seen.push(name.to_string()));
        seen.sort();
        // `Surface_2` proves the number suffix was recomposed, not dropped, and
        // `Warthog` proves an unrelated entry was left alone.
        assert_eq!(seen, ["Surface", "Surface_2", "Warthog"]);
    }

    /// Renaming the entry an export is named after does not update the hash
    /// other packages import it by, so the view has to say so first.
    #[test]
    fn an_export_hash_desync_is_reported_before_the_rename() {
        let document = rename_fixture();
        assert_eq!(
            chimp_export_hash_desyncs(&document.header, 1, "Surface"),
            vec![0]
        );
        // Renaming it back to what the hash was computed from is no desync, and
        // an entry no export is named after never is.
        assert!(chimp_export_hash_desyncs(&document.header, 1, "Material").is_empty());
        assert!(chimp_export_hash_desyncs(&document.header, 0, "Anything").is_empty());
    }

    /// Retargeting a slot must leave every other slot where it was: an object
    /// property names an import by position, so a reordered list would silently
    /// repoint properties nobody edited.
    #[test]
    fn retargeting_one_import_slot_leaves_the_others_in_place() {
        let mut document = rename_fixture();
        let slots = vec![
            ImportSlot::Script(FPackageObjectIndex::create_script_import("/Script/Engine.Actor")),
            ImportSlot::Package(ImportTarget {
                package: "/Game/One".to_owned(),
                object_hash: public_export_hash("One"),
            }),
            ImportSlot::Null,
        ];
        write_import_slots(&mut document.header, &slots).unwrap();

        apply_chimp_import_slot(
            &mut document,
            1,
            ImportSlot::Package(ImportTarget {
                package: "/Game/Two".to_owned(),
                object_hash: public_export_hash("Two"),
            }),
        )
        .unwrap();

        let after = read_import_slots(&document.header).unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0], slots[0], "slot 0 moved");
        assert_eq!(after[2], ImportSlot::Null, "slot 2 moved");
        let ImportSlot::Package(target) = &after[1] else {
            panic!("slot 1 is a package import");
        };
        assert_eq!(target.package, "/Game/Two");
        assert_eq!(target.object_hash, public_export_hash("Two"));
    }

    /// Appending cannot renumber anything, which is why it is offered while
    /// removal is not.
    #[test]
    fn an_appended_slot_lands_at_the_end_and_is_refused_past_it() {
        let mut document = rename_fixture();
        write_import_slots(&mut document.header, &[ImportSlot::Null]).unwrap();

        apply_chimp_import_slot(&mut document, 1, ImportSlot::Null).unwrap();
        assert_eq!(read_import_slots(&document.header).unwrap().len(), 2);

        let error = apply_chimp_import_slot(&mut document, 5, ImportSlot::Null).unwrap_err();
        assert!(error.contains("past the end"), "{error}");
        assert_eq!(read_import_slots(&document.header).unwrap().len(), 2);
    }

    /// The stored hash is case-folded, so the object-name box cannot be used to
    /// tell two spellings apart — worth pinning, because the editor shows the
    /// hash it computed and a user could reasonably expect casing to matter.
    #[test]
    fn an_import_object_hash_ignores_the_case_it_was_typed_in() {
        let hash = public_export_hash("Crate");
        assert_eq!(public_export_hash("crate"), hash);
        assert_eq!(public_export_hash("CRATE"), hash);
        assert_ne!(public_export_hash("Crates"), hash);
    }

    /// Object flags decide how a payload is *read*, so a wrong one is not a
    /// mislabel — it is a different interpretation of the same bytes. The named
    /// bits have to be the ones the engine uses.
    #[test]
    fn the_named_object_flag_bits_match_the_engine_values() {
        let named: Vec<(u32, &str)> = CHIMP_OBJECT_FLAG_BITS.to_vec();
        assert_eq!(named[0], (0x0000_0001, "Public"));
        assert_eq!(named[1], (0x0000_0002, "Standalone"));
        assert_eq!(named[2], (0x0000_0008, "Transactional"));
        // The one that actually changes decoding, via the native tail branch.
        assert_eq!(named[3], (0x0000_0010, "ClassDefaultObject"));
        assert_eq!(named[4], (0x0000_0020, "ArchetypeObject"));

        // Campaign Evolved's measured tag flags decompose into these, with
        // nothing unnamed left over: 0xb is Public + Standalone + Transactional.
        let all: u32 = CHIMP_OBJECT_FLAG_BITS.iter().map(|(bit, _)| bit).sum();
        assert_eq!(0xb_u32 & !all, 0, "0xb is fully named");
        assert_eq!(0x1_u32 & !all, 0, "the generated-group value is fully named");
    }

    /// Interning rather than renaming is the difference between "this export is
    /// now called X" and "everything called Y is now called X".
    fn export_edit(document: &ChimpDocument, object_name: &str) -> ChimpExportEdit {
        ChimpExportEdit {
            index: 0,
            object_name: object_name.to_owned(),
            object_flags: document.header.export_map[0].object_flags,
            filter_flags: document.header.export_map[0].filter_flags,
            recompute_hash: false,
        }
    }

    /// Interning rather than renaming is the difference between "this export is
    /// now called X" and "everything called Y is now called X" — the second is
    /// what the name-map row does, and confusing the two would silently move
    /// every other reference to that entry.
    #[test]
    fn renaming_an_export_interns_a_name_instead_of_rewriting_the_table() {
        let mut document = rename_fixture();
        let before = document.header.name_map.names().to_vec();

        let edit = export_edit(&document, "Surface");
        apply_chimp_export_metadata(&mut document, &edit);

        // `Material` is still entry 1, untouched, and `Surface` was appended.
        assert_eq!(&document.header.name_map.names()[..before.len()], &before[..]);
        assert_eq!(document.header.name_map.names().last().unwrap(), "Surface");
        assert_eq!(document.exports[0].object, "Surface");
    }

    /// Off by default, because it cuts both ways: recomputing makes this package
    /// self-consistent and every importer holding the old hash wrong.
    #[test]
    fn an_export_hash_moves_only_when_the_recompute_is_asked_for() {
        let mut document = rename_fixture();
        let original = document.header.export_map[0].public_export_hash;
        assert_eq!(original, public_export_hash("Material"));

        let edit = export_edit(&document, "Surface");
        apply_chimp_export_metadata(&mut document, &edit);
        assert_eq!(
            document.header.export_map[0].public_export_hash, original,
            "the hash must not follow the name on its own"
        );

        let mut edit = export_edit(&document, "Surface");
        edit.recompute_hash = true;
        apply_chimp_export_metadata(&mut document, &edit);
        assert_eq!(
            document.header.export_map[0].public_export_hash,
            public_export_hash("Surface")
        );
    }

    /// A trailing `_N` belongs in the reference's number field, and `store` is
    /// what puts it there — so the round trip has to give the name back whole.
    #[test]
    fn an_export_name_with_a_number_suffix_round_trips_through_the_reference() {
        let mut document = rename_fixture();
        let edit = export_edit(&document, "Surface_3");
        apply_chimp_export_metadata(&mut document, &edit);

        assert_eq!(document.exports[0].object, "Surface_3");
        // Stored as base + number, not as a literal entry.
        assert_eq!(document.header.name_map.names().last().unwrap(), "Surface");
        assert_eq!(document.header.export_map[0].object_name.number, 4);
    }

    fn identity_edit(document: &ChimpDocument) -> ChimpIdentityEdit {
        let versioning = &document.header.versioning_info;
        ChimpIdentityEdit {
            package_flags: format!("{:08X}", document.header.summary.package_flags),
            licensee_version: versioning.licensee_version,
            is_unversioned: document.header.is_unversioned,
            zen_version: versioning.zen_version,
            file_version_ue4: versioning.package_file_version.file_version_ue4,
            file_version_ue5: versioning.package_file_version.file_version_ue5,
        }
    }

    /// The gate has to pass the thing it is guarding, or it is just a refusal.
    #[test]
    fn a_flag_change_that_reopens_as_itself_is_applied() {
        let mut document = rename_fixture();
        let mut edit = identity_edit(&document);
        edit.package_flags = "80002200".to_owned();

        apply_chimp_identity_edit(&mut document, &edit).unwrap();
        assert_eq!(document.header.summary.package_flags, 0x8000_2200);

        // `0x` is accepted as well as bare hex, since the view shows it both ways.
        let mut edit = identity_edit(&document);
        edit.package_flags = "0x88002200".to_owned();
        apply_chimp_identity_edit(&mut document, &edit).unwrap();
        assert_eq!(document.header.summary.package_flags, 0x8800_2200);
    }

    /// Campaign Evolved's packages are unversioned, and an unversioned package
    /// stores no versioning block — the loader infers one. So a version edit
    /// there cannot persist, and the gate must not pretend it did.
    #[test]
    fn versioning_fields_are_not_stored_while_a_package_is_unversioned() {
        let document = rename_fixture();
        assert!(document.header.is_unversioned, "the fixture is CE-shaped");

        let mut candidate = document.header.clone();
        candidate.versioning_info.licensee_version = 7;
        candidate.versioning_info.package_file_version.file_version_ue5 = 1;

        // The gate passes, because those fields are simply not written...
        chimp_header_survives_reopen(&candidate, &document.payloads).unwrap();

        // ...and this is what actually comes back.
        let (bytes, _) =
            write_package(&candidate, &document.payloads, CE_HEADER_VERSION).unwrap();
        let reopened = FZenPackageHeader::deserialize(
            &mut Cursor::new(&bytes),
            None,
            CE_TOC_VERSION,
            CE_HEADER_VERSION,
            None,
        )
        .unwrap();
        assert_eq!(reopened.versioning_info.licensee_version, 0);
        assert_ne!(
            reopened.versioning_info.package_file_version.file_version_ue5,
            1
        );
    }

    /// A refused edit must leave the document exactly as it was — the candidate
    /// is written and reopened before anything is assigned.
    #[test]
    fn an_unparseable_flag_value_changes_nothing() {
        let mut document = rename_fixture();
        let before = document.header.summary.package_flags;
        let mut edit = identity_edit(&document);
        edit.package_flags = "not hex".to_owned();

        let error = apply_chimp_identity_edit(&mut document, &edit).unwrap_err();
        assert!(error.contains("hex value"), "{error}");
        assert_eq!(document.header.summary.package_flags, before);
    }

    /// Turning versioning on makes the block real, and `has_versioning_info` is
    /// derived on write — a draft that moved one without the other would reopen
    /// as the opposite thing.
    #[test]
    fn turning_versioning_on_makes_the_fields_persist() {
        let mut document = rename_fixture();
        let mut edit = identity_edit(&document);
        edit.is_unversioned = false;
        edit.licensee_version = 7;
        edit.file_version_ue5 = 1013;

        apply_chimp_identity_edit(&mut document, &edit).unwrap();
        assert!(!document.header.is_unversioned);
        assert_eq!(document.header.summary.has_versioning_info, 1);
        assert_eq!(document.header.versioning_info.licensee_version, 7);

        // Now that the block is written, the gate is comparing real fields — so
        // the same edit round-trips instead of being silently dropped.
        chimp_header_survives_reopen(&document.header, &document.payloads).unwrap();

        let mut edit = identity_edit(&document);
        edit.is_unversioned = true;
        apply_chimp_identity_edit(&mut document, &edit).unwrap();
        assert!(document.header.is_unversioned);
        assert_eq!(document.header.summary.has_versioning_info, 0);
    }

    /// The gate's first job is catching a header that cannot be read back at
    /// all — which is exactly what an export with no bundle commands is.
    #[test]
    fn the_reopen_gate_refuses_a_header_that_cannot_be_read_back() {
        let document = rename_fixture();
        let mut candidate = document.header.clone();
        candidate.export_bundle_entries.clear();

        let error = chimp_header_survives_reopen(&candidate, &document.payloads).unwrap_err();
        assert!(error.contains("did not reopen"), "{error}");
    }

    fn reopen(bytes: &[u8]) -> FZenPackageHeader {
        FZenPackageHeader::deserialize(
            &mut Cursor::new(bytes),
            None,
            CE_TOC_VERSION,
            CE_HEADER_VERSION,
            None,
        )
        .expect("the package reopens")
    }

    /// The fixture, put through one write and reopen so its summary holds real
    /// section offsets.
    ///
    /// A hand-built header starts with every offset and `header_size` at zero —
    /// a state no file is ever in, because the writer fills them. Round-trip
    /// properties are about well-formed headers, and reopening is how one is
    /// obtained; comparing against the unwritten form would only measure the
    /// fixture.
    fn normalized_fixture() -> ChimpDocument {
        let mut document = rename_fixture();
        let (bytes, _) =
            write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
        document.header = reopen(&bytes);
        document
    }

    /// The control every other round-trip rests on.
    ///
    /// Without it, "the edit read back" would be equally true of a writer that
    /// quietly rearranged half the header — the edited field would survive and
    /// everything around it would have moved. Writing, reopening and writing
    /// again has to reach the same bytes, which is what separates rebuilt from
    /// rearranged.
    #[test]
    fn writing_a_package_is_a_fixpoint_over_reopening_it() {
        let document = normalized_fixture();
        let (first, _) =
            write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
        let (second, _) =
            write_package(&reopen(&first), &document.payloads, CE_HEADER_VERSION).unwrap();
        assert_eq!(first, second, "an unedited package must rebuild identically");
    }

    /// A rename has to survive the writer, not just the in-memory table.
    #[test]
    fn a_renamed_entry_survives_a_write_and_reopen() {
        let mut document = rename_fixture();
        let before = document.header.name_map.names().to_vec();
        apply_chimp_name_rename(&mut document, 1, "Surface").unwrap();

        let (bytes, _) =
            write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
        let reopened = reopen(&bytes);

        let mut expected = before;
        expected[1] = "Surface".to_owned();
        assert_eq!(reopened.name_map.names(), expected.as_slice());
        // And the export that referenced it now resolves to the new text.
        assert_eq!(
            reopened
                .name_map
                .try_get(reopened.export_map[0].object_name)
                .as_deref(),
            Some("Surface")
        );
    }

    /// Retargeting rebuilds four parallel arrays from the slot list, so the
    /// proof it worked is that the slots come back as they were set — not that
    /// the arrays look plausible.
    #[test]
    fn a_retargeted_import_survives_a_write_and_reopen() {
        let mut document = normalized_fixture();
        let slots = vec![
            ImportSlot::Script(FPackageObjectIndex::create_script_import("/Script/Engine.Actor")),
            ImportSlot::Package(ImportTarget {
                package: "/Game/One".to_owned(),
                object_hash: public_export_hash("One"),
            }),
        ];
        write_import_slots(&mut document.header, &slots).unwrap();
        apply_chimp_import_slot(
            &mut document,
            1,
            ImportSlot::Package(ImportTarget {
                package: "/Game/Two".to_owned(),
                object_hash: public_export_hash("Two"),
            }),
        )
        .unwrap();

        let (bytes, _) =
            write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
        let after = read_import_slots(&reopen(&bytes)).unwrap();

        assert_eq!(after[0], slots[0]);
        assert_eq!(
            after[1],
            ImportSlot::Package(ImportTarget {
                package: "/Game/Two".to_owned(),
                object_hash: public_export_hash("Two"),
            })
        );
    }

    /// Export metadata is written verbatim, so this is the check that it is not
    /// being recomputed out from under the edit.
    #[test]
    fn edited_export_metadata_survives_a_write_and_reopen() {
        let mut document = rename_fixture();
        let mut edit = export_edit(&document, "Surface");
        edit.filter_flags = EExportFilterFlags::NotForServer;
        edit.recompute_hash = true;
        apply_chimp_export_metadata(&mut document, &edit);

        let (bytes, _) =
            write_package(&document.header, &document.payloads, CE_HEADER_VERSION).unwrap();
        let reopened = reopen(&bytes);
        let entry = &reopened.export_map[0];

        assert_eq!(
            reopened.name_map.try_get(entry.object_name).as_deref(),
            Some("Surface")
        );
        assert_eq!(entry.filter_flags, EExportFilterFlags::NotForServer);
        assert_eq!(entry.public_export_hash, public_export_hash("Surface"));
    }

    /// "Nothing imports this" and "nothing I could read imports this" are
    /// different answers, and a rename is only safe under the first. A scan
    /// that swallowed unreadable packages would report the safe one.
    #[test]
    fn a_referrer_scan_keeps_what_it_could_not_rule_out_separate_from_what_it_cleared() {
        let clean = ChimpReferrerScan {
            referrers: Vec::new(),
            scanned: 100,
            unreadable: 0,
        };
        let partial = ChimpReferrerScan {
            referrers: Vec::new(),
            scanned: 100,
            unreadable: 3,
        };
        // Both found nothing; only one of them looked everywhere.
        assert!(clean.referrers.is_empty() && partial.referrers.is_empty());
        assert_eq!(clean.unreadable, 0);
        assert_ne!(partial.unreadable, 0);
    }

    #[test]
    fn a_mesh_import_is_a_material_by_the_prefix_the_game_uses() {
        assert!(is_chimp_material_package("/Game/Art/Materials/MI_Brute_Body"));
        assert!(is_chimp_material_package("/Game/Art/Materials/M_Master"));
        // Everything else a mesh imports - skeletons, physics, engine content,
        // and the textures themselves - is not a material.
        assert!(!is_chimp_material_package("/Game/Art/Textures/T_Brute_D"));
        assert!(!is_chimp_material_package("/Game/Art/SK_Brute"));
        assert!(!is_chimp_material_package("/Script/Engine"));
        // The prefix is on the leaf, not anywhere in the path.
        assert!(!is_chimp_material_package("/Game/M_Things/SK_Brute"));
    }

    #[test]
    fn a_mesh_name_names_the_model_its_textures_belong_to() {
        // The asset-type prefix is not the subject; the segment after it is.
        assert_eq!(
            chimp_mesh_texture_subject("/Game/Art/SM_AssaultRifle_GunBody_M_Default").as_deref(),
            Some("assaultrifle")
        );
        assert_eq!(
            chimp_mesh_texture_subject("/Game/Art/SK_Brute").as_deref(),
            Some("brute")
        );
        // A name that leads with its subject keeps it.
        assert_eq!(
            chimp_mesh_texture_subject("/Game/Art/AssaultRifle_Body").as_deref(),
            Some("assaultrifle")
        );
        assert_eq!(
            chimp_mesh_texture_subject("/Game/Art/Warthog").as_deref(),
            Some("warthog")
        );
        // A prefix and nothing else still has to answer something, or the
        // filter would match every texture in the game.
        assert_eq!(
            chimp_mesh_texture_subject("/Game/Art/SM_").as_deref(),
            Some("sm")
        );
        assert_eq!(chimp_mesh_texture_subject("").as_deref(), None);
    }

    #[test]
    fn the_subject_matches_a_models_textures_and_not_the_shared_ones() {
        let subject = chimp_mesh_texture_subject("/Game/Art/SM_AssaultRifle_GunBody_M_Default")
            .expect("subject");
        let matches = |leaf: &str| leaf.to_lowercase().contains(&subject);
        assert!(matches("T_AssaultRifle_GunBody_D"));
        assert!(matches("T_assaultrifle_ORM"));
        assert!(matches("T_AssaultRifle_Decal_01"));
        // The shared master inputs a material graph drags in are exactly what
        // this is here to leave out.
        assert!(!matches("T_MasterNoise_01"));
        assert!(!matches("T_Default_Normal"));
        assert!(!matches("T_Brute_D"));
    }

    #[test]
    fn textures_land_beside_the_mesh_they_belong_to() {
        let prompt = ChimpMeshTexturePrompt {
            kit: KitId(1),
            package: "/Game/Art/SK_Brute".to_owned(),
            format: ChimpMeshFormat::Pskx,
            texture_export: ChimpTextureExport::default(),
            path: PathBuf::from("C:/exports/brute.pskx"),
        };
        assert_eq!(
            prompt.texture_directory(),
            PathBuf::from("C:/exports").join(CHIMP_TEXTURE_DIR)
        );
        assert_eq!(prompt.format_label(), "ActorX PSKX");
    }

    #[test]
    fn chimp_discard_uses_dirty_document_keys_for_prompts() {
        assert_eq!(
            sorted_unique_dirty_chimp_keys([
                ("/Game/ZetaRequestedKey", true),
                ("/Game/AlphaRequestedKey", true),
                ("/Game/CleanRequestedKey", false),
            ]),
            ["/Game/AlphaRequestedKey", "/Game/ZetaRequestedKey"]
        );
    }

    #[test]
    fn chimp_workspace_toolbar_does_not_consume_the_editor_viewport() {
        let context = egui::Context::default();
        let mut toolbar_height = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_200.0, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let toolbar = chimp_workspace_toolbar(ui, |ui| {
                        let _ = ui.button("Discard");
                    });
                    toolbar_height = Some(toolbar.response.rect.height());
                });
            },
        );

        let toolbar_height = toolbar_height.expect("the Chimp toolbar was rendered");
        assert!(
            toolbar_height <= 40.0,
            "the Chimp toolbar expanded to {toolbar_height}px"
        );
    }

    #[test]
    fn chimp_scalar_property_row_does_not_consume_the_scroll_viewport() {
        let context = egui::Context::default();
        let mut value = 0_i64;
        let mut row_height = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_200.0, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let row = ui.horizontal_top(|ui| {
                            ui.set_min_height(24.0);
                            ui.label("NumReplicatedProperties");
                            chimp_property_value_cell(ui, |ui| {
                                ui.add(egui::DragValue::new(&mut value));
                            });
                        });
                        row_height = Some(row.response.rect.height());
                    });
                });
            },
        );

        let row_height = row_height.expect("the property row was rendered");
        assert!(
            row_height <= 40.0,
            "a scalar property row expanded to {row_height}px"
        );
    }

    #[test]
    fn chimp_mod_names_are_sanitized_and_priority_suffixed() {
        assert_eq!(chimp_mod_stem("My Cool Mod"), "My-Cool-Mod_P");
        assert_eq!(chimp_mod_stem("Already_p"), "Already_p");
        assert_eq!(chimp_mod_stem("../../unsafe"), "unsafe_P");
    }

    #[test]
    fn chimp_mod_stem_rejects_an_empty_name_at_the_dialog_boundary() {
        assert!(sanitize_mod_name(" ! ").is_empty());
        assert_eq!(chimp_mod_stem(" ! "), "_P");
    }

    fn preview_normal_alignment(preview: &ModelPreviewData) -> (f32, f32) {
        // Unreal's source winding is left-handed, so the sign relative to this
        // right-handed cross product is expected to be negative. Magnitude is
        // the useful regression signal: broken packed normals are incoherent.
        let mut signed = 0.0;
        let mut absolute = 0.0;
        let mut count = 0usize;
        for triangle in preview.preview.indices.chunks_exact(3) {
            let Some(a) = preview.preview.vertices.get(triangle[0] as usize) else {
                continue;
            };
            let Some(b) = preview.preview.vertices.get(triangle[1] as usize) else {
                continue;
            };
            let Some(c) = preview.preview.vertices.get(triangle[2] as usize) else {
                continue;
            };
            let [a, b, c] = [a.position, b.position, c.position];
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let length = (face[0] * face[0] + face[1] * face[1] + face[2] * face[2]).sqrt();
            if length <= 1.0e-6 {
                continue;
            }
            let face = [face[0] / length, face[1] / length, face[2] / length];
            for normal in [
                preview.preview.vertices[triangle[0] as usize].normal,
                preview.preview.vertices[triangle[1] as usize].normal,
                preview.preview.vertices[triangle[2] as usize].normal,
            ] {
                let dot = face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2];
                signed += dot;
                absolute += dot.abs();
                count += 1;
            }
        }
        if count == 0 {
            return (0.0, 0.0);
        }
        (signed / count as f32, absolute / count as f32)
    }

    fn job_at(done: usize, total: usize, elapsed: Duration) -> ChimpLevelJob {
        ChimpLevelJob {
            kit: KitId(0),
            name: "C10".to_owned(),
            phase: ChimpLevelPhase::ReadingCells,
            done,
            total,
            phase_started: Instant::now() - elapsed,
        }
    }

    #[test]
    fn a_progress_estimate_waits_until_it_has_something_to_go_on() {
        // A rate from the first few items swings by minutes and teaches the
        // user to ignore the number, so there is no number until then.
        assert!(job_at(0, 2334, Duration::from_secs(10)).remaining().is_none());
        assert!(job_at(3, 2334, Duration::from_millis(200)).remaining().is_none());
        // Finished is not "0s left", it is nothing to say.
        assert!(job_at(2334, 2334, Duration::from_secs(60)).remaining().is_none());
    }

    #[test]
    fn a_progress_estimate_extrapolates_the_rate_so_far() {
        // Half done after 60s means about 60s left.
        let remaining = job_at(1_000, 2_000, Duration::from_secs(60))
            .remaining()
            .expect("half way through is enough to estimate from");
        assert!(
            (55..=65).contains(&remaining.as_secs()),
            "estimated {}s",
            remaining.as_secs()
        );
    }

    #[test]
    fn a_fraction_stays_inside_the_bar() {
        assert_eq!(job_at(0, 0, Duration::from_secs(1)).fraction(), 0.0);
        assert_eq!(job_at(1_167, 2_334, Duration::from_secs(1)).fraction(), 0.5);
        // A count past the total would draw outside the bar.
        assert_eq!(job_at(9_999, 2_334, Duration::from_secs(1)).fraction(), 1.0);
    }

    #[test]
    fn time_left_is_said_at_the_precision_it_is_known_to() {
        assert_eq!(format_remaining(Duration::from_secs(3)), "a few seconds");
        assert_eq!(format_remaining(Duration::from_secs(42)), "about 42s");
        assert_eq!(format_remaining(Duration::from_secs(260)), "about 4m 20s");
        // Past ten minutes the seconds are noise.
        assert_eq!(format_remaining(Duration::from_secs(1_500)), "about 25m");
    }

    #[test]
    fn a_level_offers_a_menu_wherever_it_is_browsed() {
        // The bug this exists for: the level entry was added to every menu
        // body, but two of the four sites still only attached a menu for
        // textures and meshes - so on those the entry sat inside a menu that
        // was never built, and right-clicking a level did nothing at all.
        // A level is a `World`, which is neither.
        let level = ChimpPackageActions::of("/Game/Levels/Halo1/Solo/C10/C10", Some("World"));
        assert!(level.level);
        assert!(level.any(), "a level must open a menu");
        assert!(!level.texture && !level.mesh);
    }

    #[test]
    fn anything_the_menu_would_offer_opens_it() {
        // The invariant that keeps the guard and the contents from drifting:
        // `any()` is true exactly when at least one entry would be drawn.
        for (package, kind) in [
            ("/Game/Levels/Halo1/Solo/C10/C10", Some("World")),
            ("/Game/Meshes/SM_Rock", Some("StaticMesh")),
            ("/Game/Characters/SK_Elite", Some("SkeletalMesh")),
            ("/Game/Textures/T_Bark", Some("Texture2D")),
            ("/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ", Some("World")),
            ("/Game/Blueprints/BP_Door", Some("Blueprint")),
            ("/Game/Misc/Thing", None),
        ] {
            let actions = ChimpPackageActions::of(package, kind);
            assert_eq!(
                actions.any(),
                actions.texture || actions.mesh || actions.level,
                "{package} would draw entries into a menu that does not open"
            );
        }
    }

    #[test]
    fn a_package_with_nothing_to_offer_opens_no_menu() {
        assert!(!ChimpPackageActions::of("/Game/Blueprints/BP_Door", Some("Blueprint")).any());
        // A cell is a World too, and there are 2,334 of them: offering to
        // export each one as a level would be noise.
        assert!(
            !ChimpPackageActions::of(
                "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ",
                Some("World")
            )
            .any()
        );
    }

    #[test]
    fn a_persistent_level_is_recognised_by_its_folder() {
        // Unreal names a persistent level after the folder holding it, which is
        // what the menu test keys on before the cells are searched for.
        assert!(chimp_looks_like_level("/Game/Levels/Halo1/Solo/C10/C10"));
        assert!(chimp_looks_like_level("/Game/Levels/Halo1/Solo/c10/C10"));
        // A cell, a mesh, and anything else is one package.
        assert!(!chimp_looks_like_level(
            "/Game/Levels/Halo1/Solo/C10/_Generated_/043ATWPYEEJ"
        ));
        assert!(!chimp_looks_like_level("/Game/Meshes/Rocks/SM_Rock_A"));
        assert!(!chimp_looks_like_level("/Game"));
        assert!(!chimp_looks_like_level(""));
    }

    #[test]
    fn an_export_names_its_files_after_the_level() {
        let prompt = ChimpLevelExportPrompt {
            kit: KitId(0),
            package: "/Game/Levels/Halo1/Solo/C10/C10".to_owned(),
            cells: Vec::new(),
            format: ChimpLevelFormat::SegmentedUsd,
            nanite: true,
            split: true,
            triangles: 30_000_000,
            placements: 50_000,
        };
        assert_eq!(prompt.name(), "C10");
        assert_eq!(prompt.budget().triangles, 30_000_000);
        assert_eq!(prompt.budget().placements, 50_000);
    }

    #[test]
    fn not_splitting_is_one_segment_rather_than_another_exporter() {
        // Turning the split off has to go down the same path, or there are two
        // ways to write a level and only one of them stays tested.
        let prompt = ChimpLevelExportPrompt {
            kit: KitId(0),
            package: "/Game/Levels/X/Small/Small".to_owned(),
            cells: Vec::new(),
            format: ChimpLevelFormat::Blender,
            nanite: false,
            split: false,
            triangles: 30_000_000,
            placements: 50_000,
        };
        assert_eq!(prompt.budget().triangles, usize::MAX);
        assert_eq!(prompt.budget().placements, usize::MAX);
    }

    #[test]
    fn chimp_is_idle_and_unfiltered_by_default() {
        let state = ChimpState::default();
        assert!(matches!(state.mount, ChimpMount::Idle));
        assert_eq!(state.browser, ChimpBrowser::Folders);
        assert!(state.filter.is_empty());
        assert!(state.open_packages.is_empty());
        assert!(
            !state.filter_is_current(""),
            "the initial empty query must populate the browser once"
        );
    }

    #[test]
    fn chimp_browser_tabs_follow_the_asset_browsing_order() {
        assert_eq!(
            ChimpBrowser::TABS,
            [
                (ChimpBrowser::Folders, "Folders"),
                (ChimpBrowser::Groups, "Groups"),
                (ChimpBrowser::Files, "Pak files"),
                (ChimpBrowser::Archives, "Archives"),
                (ChimpBrowser::Packages, "Packages"),
            ]
        );
    }

    #[test]
    fn campaign_evolved_surface_tabs_put_tags_before_chimp() {
        assert_eq!(KitSurface::TABS[0].0, KitSurface::Tags);
        assert_eq!(KitSurface::TABS[0].1, "Tags");
        assert_eq!(KitSurface::TABS[1].0, KitSurface::Chimp);
        assert_eq!(KitSurface::TABS[1].1, "Chimp");
    }

    #[test]
    fn chimp_search_matching_is_case_insensitive_without_allocating_per_package() {
        assert!(chimp_contains_query("SM_SpiritDropShip_Body", "spirit"));
        assert!(chimp_contains_query("Texture2D", "texture2d"));
        assert!(!chimp_contains_query("StaticMesh", "skeletal"));
    }

    #[test]
    fn bundled_chimp_usmap_loads_and_invalid_custom_file_is_rejected() {
        assert!(load_chimp_usmap(None).is_ok());
        let path =
            std::env::temp_dir().join(format!("baboon-invalid-{}.usmap", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"not a usmap").unwrap();
        let error = load_chimp_usmap(Some(&path)).err().expect("invalid USMAP");
        assert!(error.contains("Could not parse USMAP"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install plus CE_PAKS and CE_USMAP"]
    fn real_custom_usmap_mounts_and_decodes_a_package() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let path = PathBuf::from(std::env::var_os("CE_USMAP").expect("set CE_USMAP"));
        let world = World::open(root, load_chimp_usmap(Some(&path)).unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .find(|package| {
                package
                    .name
                    .to_ascii_lowercase()
                    .contains("sm_spiritdropship_body")
            })
            .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
        let document = load_chimp_document(&world, &package.name).unwrap();
        assert!(!document.exports.is_empty());
        assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));
    }

    #[test]
    fn chimp_document_tree_tracks_open_close_and_selection() {
        let mut state = ChimpState::default();
        let kit = KitId(7);
        state.open_document_pane(kit, "/Game/Textures/A");
        state.open_document_pane(kit, "/Game/Textures/B");
        assert_eq!(state.open_packages.len(), 2);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/B"));
        assert!(
            state
                .document_tree
                .as_ref()
                .is_some_and(|tree| !tree.is_empty())
        );

        state.close_document_pane("/Game/Textures/B");
        assert_eq!(state.open_packages, ["/Game/Textures/A"]);
        assert_eq!(state.selected_package.as_deref(), Some("/Game/Textures/A"));
        state.close_document_pane("/Game/Textures/A");
        assert!(state.open_packages.is_empty());
        assert!(state.selected_package.is_none());
    }

    #[test]
    fn package_tree_groups_every_path_segment_and_counts_descendants() {
        let mut tree = ChimpFolderNode::default();
        tree.insert_package(0, "/Game/UI/Menu");
        tree.insert_package(1, "/Game/UI/Hud");
        tree.insert_package(2, "/Engine/Config");
        tree.insert_file(0, "../../../Meteorite/Content/Audio/menu.bnk");
        assert_eq!(tree.package_count, 3);
        assert_eq!(tree.file_count, 1);
        assert_eq!(tree.entry_count(), 4);
        let game = tree.folders.get("Game").unwrap();
        assert_eq!(game.package_count, 2);
        let ui = game.folders.get("UI").unwrap();
        assert_eq!(ui.package_count, 2);
        assert_eq!(
            ui.packages
                .iter()
                .map(|leaf| leaf.name.as_str())
                .collect::<Vec<_>>(),
            ["Menu", "Hud"]
        );
        let engine = &tree.folders["Engine"];
        assert_eq!(engine.package_count, 1);
        assert_eq!(engine.packages[0].name, "Config");
        assert_eq!(
            tree.folders["Meteorite"].folders["Content"].folders["Audio"].files[0].name,
            "menu.bnk"
        );
    }

    #[test]
    fn raw_values_are_identified_without_embedding_binary_in_json() {
        assert_eq!(
            chimp_value_json(&PropValue::Raw(vec![1, 2, 3])),
            json!({"unknown_bytes": 3})
        );
    }

    #[test]
    fn readable_documents_are_the_default_view() {
        assert_eq!(ChimpDocumentView::default(), ChimpDocumentView::Document);
        assert_eq!(
            chimp_class_display_name(Some("/Script/Engine.BlueprintGeneratedClass")),
            "BlueprintGeneratedClass"
        );
        assert_eq!(chimp_class_display_name(None), "Unknown");
    }

    #[test]
    fn json_documents_have_line_numbers_and_semantic_colours() {
        let text = "{\n  \"Name\": \"Probe\",\n  \"ObjectPath\": \"/Game/Probe.0\",\n  \"Count\": 3,\n  \"Enabled\": true,\n  \"Missing\": null\n}";
        assert_eq!(chimp_line_numbers(text), "1\n2\n3\n4\n5\n6\n7");
        let job = chimp_json_layout_job(text, egui::FontId::monospace(12.0), true);
        assert_eq!(job.text, text);
        let mut colors = Vec::new();
        for section in &job.sections {
            if !colors.contains(&section.format.color) {
                colors.push(section.format.color);
            }
        }
        assert!(
            colors.len() >= 7,
            "keys, strings, paths, numbers, literals, punctuation, and whitespace need distinct colours"
        );
    }

    #[test]
    fn output_triplet_replacement_replaces_all_files_and_cleans_backups() {
        let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let incoming = directory.join("incoming.utoc");
        let output = directory.join("Chimp_P.utoc");
        for file in triplet(&incoming) {
            std::fs::write(file, b"new").unwrap();
        }
        for file in triplet(&output) {
            std::fs::write(file, b"old").unwrap();
        }
        replace_chimp_triplet(&incoming, &output).unwrap();
        for file in triplet(&output) {
            assert_eq!(std::fs::read(file).unwrap(), b"new");
        }
        assert!(!output.with_extension("utoc.previous").exists());
        assert!(!output.with_extension("ucas.previous").exists());
        assert!(!output.with_extension("pak.previous").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovery_manifest_round_trips_package_files() {
        let manifest = ChimpRecoveryManifest {
            source: "Paks".to_owned(),
            packages: HashMap::from([("/Game/UI/Probe".to_owned(), "012345.uasset".to_owned())]),
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let restored: ChimpRecoveryManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.source, "Paks");
        assert_eq!(
            restored.packages.get("/Game/UI/Probe").map(String::as_str),
            Some("012345.uasset")
        );
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_package_rebuilds_into_a_readable_overlay() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let mut browser = ChimpState::default();
        browser.refresh_filter(&world);
        assert_eq!(browser.filtered_packages.len(), world.packages().len());
        assert_eq!(browser.filtered_files.len(), world.pak_files().len());
        assert_eq!(
            browser.content_tree.package_count,
            browser.filtered_packages.len()
        );
        assert_eq!(
            browser.content_tree.file_count,
            browser.filtered_files.len()
        );
        let container = world
            .containers()
            .iter()
            .find(|container| container.package_count > 0)
            .expect("an IoStore container with packages");
        browser.selected_archive = Some(ChimpArchive::IoStore(container.index));
        browser.reset_filter();
        browser.refresh_filter(&world);
        assert!(!browser.filtered_packages.is_empty());
        assert!(browser.filtered_packages.iter().all(|&index| {
            world.packages()[index]
                .providers
                .iter()
                .any(|provider| provider.container == container.index)
        }));
        let pak = world
            .pak_containers()
            .iter()
            .find(|container| container.file_count > 0)
            .expect("a legacy pak with files");
        browser.selected_archive = Some(ChimpArchive::Pak(pak.index));
        browser.reset_filter();
        browser.refresh_filter(&world);
        assert!(browser.filtered_packages.is_empty());
        assert!(!browser.filtered_files.is_empty());
        assert!(browser.filtered_files.iter().all(|&index| {
            world.pak_files()[index]
                .providers
                .iter()
                .any(|provider| provider.container == pak.index)
        }));
        assert!(
            !world.pak_files().is_empty(),
            "the real mount should index legacy .pak files too"
        );
        let package = world
            .packages()
            .iter()
            .find(|package| package.name.starts_with("/Game/"))
            .expect("a /Game package")
            .name
            .clone();
        let document = load_chimp_document(&world, &package).unwrap();
        assert_eq!(document.view, ChimpDocumentView::Document);
        assert!(!document.document_text_dirty);
        assert!(
            !document.exports.is_empty(),
            "the real package should expose at least one readable export"
        );
        let readable: Value =
            serde_json::from_str(&document.document_text).expect("readable document is valid JSON");
        assert_eq!(readable["Package"], package);
        assert_eq!(
            readable["Exports"].as_array().map(Vec::len),
            Some(document.exports.len())
        );
        let export = readable["Exports"]
            .as_array()
            .and_then(|exports| exports.first())
            .expect("the JSON document should contain its first export");
        assert!(export.get("Type").is_some());
        assert!(export.get("Name").is_some());
        assert!(export.get("Properties").is_some());
        assert_eq!(
            document.document_line_numbers.lines().count(),
            document.document_text.lines().count()
        );
        let metadata: Value =
            serde_json::from_str(&document.metadata_text).expect("metadata document is valid JSON");
        assert_eq!(metadata["Summary"]["Package"], package);
        assert_eq!(
            metadata["NameMap"].as_array().map(Vec::len),
            Some(document.header.name_map.copy_raw_names().len())
        );
        assert_eq!(
            metadata["ExportMap"].as_array().map(Vec::len),
            Some(document.header.export_map.len())
        );
        assert!(
            metadata["PhysicalProviders"]
                .as_array()
                .is_some_and(|providers| !providers.is_empty())
        );
        assert_eq!(
            document.metadata_line_numbers.lines().count(),
            document.metadata_text.lines().count()
        );
        let (bytes, store) = rebuild_chimp_document(&world, &document).unwrap();
        FZenPackageHeader::deserialize(
            &mut Cursor::new(&bytes),
            Some(store.clone()),
            CE_TOC_VERSION,
            CE_HEADER_VERSION,
            None,
        )
        .expect("rebuilt package parses");

        let directory = std::env::temp_dir().join(format!("baboon-chimp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("ChimpTest_P.utoc");
        let override_ = PackageOverride {
            archive: &world.archives()[document.provider.container],
            uasset_path: &document.provider.entry_path,
            bytes: bytes.clone(),
            store,
        };
        write_package_mod_container(&[override_], &output).unwrap();
        let mut overlay = blam_tags::iostore::IoStoreArchive::open(&output).unwrap();
        let bases: Vec<&blam_tags::iostore::IoStoreArchive> = world.archives().iter().collect();
        overlay.recover_entries(&bases, Some("Meteorite/Content/"));
        assert_eq!(overlay.read(&document.provider.entry_path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_fast_type_index_matches_legacy_window() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let fast = index_chimp_package_types_with_prefixes(&world, &[64 * 1024, 1024 * 1024]);
        let legacy = index_chimp_package_types_with_prefixes(&world, &[1024 * 1024]);
        assert_eq!(fast.package_types, legacy.package_types);
        assert_eq!(fast.type_counts, legacy.type_counts);
        assert_eq!(fast.failures, legacy.failures);
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_file_types_filter_and_texture_preview() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let index = index_chimp_package_types(&world);
        assert_eq!(index.package_types.len(), world.packages().len());
        for expected in ["Blueprint", "SkeletalMesh", "StaticMesh", "Texture2D"] {
            assert!(
                index.type_counts.contains_key(expected),
                "real package index should contain {expected}"
            );
        }

        let mut browser = ChimpState {
            package_types: index.package_types,
            filter: "Texture2D".to_owned(),
            ..Default::default()
        };
        browser.refresh_filter(&world);
        assert!(!browser.filtered_packages.is_empty());
        let textures = &browser.filtered_groups["Texture2D"];
        assert!(!textures.is_empty());
        assert!(
            textures
                .iter()
                .all(|index| { browser.package_types[*index].as_deref() == Some("Texture2D") })
        );

        let target = std::env::var("CE_TEXTURE_PACKAGE").ok();
        let package_index = match target.as_deref() {
            Some(target) => {
                let target = target.to_ascii_lowercase();
                textures
                    .iter()
                    .copied()
                    .find(|index| {
                        world.packages()[*index]
                            .name
                            .to_ascii_lowercase()
                            .contains(&target)
                    })
                    .unwrap_or_else(|| panic!("target Texture2D package {target:?} was not found"))
            }
            None => textures[0],
        };
        let package = world.packages()[package_index].name.clone();
        let document = load_chimp_document(&world, &package).unwrap();
        assert_eq!(document.view, ChimpDocumentView::Texture);
        let decoded = document
            .texture_previews
            .iter()
            .find_map(|preview| {
                preview
                    .preview
                    .decoded
                    .as_ref()
                    .and_then(|decoded| decoded.as_ref().ok())
            })
            .unwrap_or_else(|| panic!("{package} should decode at least one Texture2D preview"));
        assert_eq!(
            decoded.rgba.len(),
            decoded.width as usize * decoded.height as usize * 4
        );
        if package
            .to_ascii_lowercase()
            .ends_with("/t_odst_williams_default_d")
        {
            assert_eq!((decoded.width, decoded.height), (4096, 1024));
            assert!(
                decoded
                    .rgba
                    .chunks_exact(4)
                    .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2]),
                "target virtual texture should contain decoded colour data"
            );
        }
        // A UDIM set writes one numbered file per block rather than the single
        // path chosen, so check the directory rather than that exact name.
        let directory =
            std::env::temp_dir().join(format!("baboon-chimp-texture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_texture(
            &world,
            &package,
            &directory.join("texture.tif"),
            ChimpTextureExport { format: ChimpTextureFormat::Tiff, ..Default::default() },
        )
        .unwrap();
        let written: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(!written.is_empty(), "Texture2D extraction wrote nothing");
        for path in &written {
            let bytes = std::fs::read(path).unwrap();
            assert!(
                bytes.starts_with(b"II*") || bytes.starts_with(b"MM\0*"),
                "{} should be a TIFF file",
                path.display()
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A multi-row UDIM set numbers its rows downward from the top of the
    /// reassembled image, because that is the block coordinate Unreal recovers
    /// from the number. Flipping is the mistake, so pin the direction against a
    /// set whose rows are told apart by their authored resolution.
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn udim_rows_are_numbered_downward_from_the_top() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .map(|package| package.name.clone())
            .find(|name| name.to_ascii_lowercase().ends_with("t_elite_minor_armor_n"))
            .expect("elite minor armour normal");
        let document = load_chimp_document(&world, &package).unwrap();
        let surfaces = chimp_selected_surfaces(&document, &package).unwrap();
        assert_eq!((surfaces.width_in_blocks, surfaces.height_in_blocks), (3, 2));

        // In the middle column this set is full size on the top row and half
        // size on the bottom, so the pair (1002, 1012) fixes the direction.
        let layer = &surfaces.layers[0];
        let top_middle = block_base_level(layer, 3, 2, 1, 0).unwrap();
        let bottom_middle = block_base_level(layer, 3, 2, 1, 1).unwrap();
        assert_ne!(
            top_middle, bottom_middle,
            "this fixture only pins the direction if the two rows differ"
        );

        let directory = std::env::temp_dir().join(format!("baboon-udim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_texture(
            &world,
            &package,
            &directory.join("t.png"),
            ChimpTextureExport { format: ChimpTextureFormat::Png, ..Default::default() },
        )
        .unwrap();
        let side = |name: &str| {
            let bytes = std::fs::read(directory.join(name)).unwrap();
            u32::from_be_bytes(bytes[16..20].try_into().unwrap())
        };
        // Top row is 100x, the row below it 101x.
        assert_eq!(
            side("t.1002.png"),
            layer.mips[top_middle].width / 3,
            "1002 should be the top-middle block"
        );
        assert_eq!(
            side("t.1012.png"),
            layer.mips[bottom_middle].width / 3,
            "1012 should be the block below 1002"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Exports a real UDIM virtual texture and checks the DDS files it produces.
    ///
    /// This is the end-to-end check for the whole texture path: the VT tiles are
    /// reassembled while still compressed, split back into UDIM blocks, and
    /// written with their full mip chain.
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_udim_virtual_texture_extracts_to_numbered_dds_files() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let target = std::env::var("CE_TEXTURE_PACKAGE")
            .unwrap_or_else(|_| "T_MI_FloodTank_Default_D".to_owned())
            .to_ascii_lowercase();
        let package = world
            .packages()
            .iter()
            .map(|package| package.name.clone())
            .find(|name| name.to_ascii_lowercase().ends_with(&target))
            .unwrap_or_else(|| panic!("no Texture2D package ending in {target:?}"));

        let document = load_chimp_document(&world, &package).unwrap();
        let surfaces = chimp_selected_surfaces(&document, &package).unwrap();
        assert!(surfaces.is_virtual, "{package} should be a virtual texture");
        assert!(surfaces.is_udim(), "{package} should be a UDIM set");
        // Tiles were cropped in block space, so the surface is still compressed.
        assert_eq!(surfaces.layers[0].mips[0].pixel_format, "PF_DXT1");

        let directory = std::env::temp_dir().join(format!("baboon-dds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("texture.dds");
        write_chimp_texture(&world, &package, &output, ChimpTextureExport::default()).unwrap();

        let mut written: Vec<String> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        assert_eq!(
            written.len(),
            (surfaces.width_in_blocks * surfaces.height_in_blocks) as usize,
            "one DDS per UDIM block: {written:?}"
        );
        assert!(written.contains(&"texture.1001.dds".to_owned()), "{written:?}");

        let bytes = std::fs::read(directory.join("texture.1001.dds")).unwrap();
        assert_eq!(&bytes[0..4], b"DDS ");
        let read_u32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let (height, width) = (read_u32(12), read_u32(16));
        let mip_count = read_u32(28);
        // One block of a 7x1 grid over 7168x1024 is a square 1024 page.
        assert_eq!(
            (width, height),
            (
                surfaces.width / surfaces.width_in_blocks,
                surfaces.height / surfaces.height_in_blocks
            )
        );
        assert!(mip_count > 1, "expected a mip chain, found {mip_count}");
        // fourcc "DX10" at the pixel-format block, then the DXGI format.
        assert_eq!(&bytes[84..88], b"DX10");
        assert_eq!(read_u32(128), 71, "PF_DXT1 should write DXGI BC1_UNORM");
        std::fs::remove_dir_all(&directory).unwrap();

        // PNG and TIFF split identically, at the same per-block resolution, so a
        // set exported one way lines up with the same set exported another.
        for (format, extension, magic) in [
            (ChimpTextureFormat::Png, "png", &b"\x89PNG"[..]),
            (ChimpTextureFormat::Tiff, "tif", &b"II*"[..]),
        ] {
            std::fs::create_dir_all(&directory).unwrap();
            write_chimp_texture(
                &world,
                &package,
                &directory.join(format!("texture.{extension}")),
                ChimpTextureExport { format, ..Default::default() },
            )
            .unwrap();
            let mut flat: Vec<String> = std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            flat.sort();
            assert_eq!(flat.len(), written.len(), "{extension}: {flat:?}");
            assert!(
                flat.contains(&format!("texture.1001.{extension}")),
                "{flat:?}"
            );
            let first = std::fs::read(directory.join(format!("texture.1001.{extension}"))).unwrap();
            assert!(first.starts_with(magic), "{extension} magic");
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// Turning the split off writes one stitched image covering every UDIM
    /// block, for an engine that cannot import a numbered set.
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn unsplit_udim_exports_one_stitched_image() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .map(|package| package.name.clone())
            .find(|name| name.to_ascii_lowercase().ends_with("t_elite_minor_armor_n"))
            .expect("elite minor armour normal");
        let document = load_chimp_document(&world, &package).unwrap();
        let surfaces = chimp_selected_surfaces(&document, &package).unwrap();
        assert!(surfaces.is_udim());

        let directory =
            std::env::temp_dir().join(format!("baboon-stitched-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        write_chimp_texture(
            &world,
            &package,
            &directory.join("t.png"),
            ChimpTextureExport {
                format: ChimpTextureFormat::Png,
                split_udim: false,
            },
        )
        .unwrap();
        let written: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(written, vec!["t.png".to_owned()], "one file, not a set");

        let bytes = std::fs::read(directory.join("t.png")).unwrap();
        let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        // The whole atlas, at the full reassembled size.
        assert_eq!((side(16), side(20)), (surfaces.width, surfaces.height));
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A mesh's textures follow the format chosen in the prompt, not a fixed
    /// TIFF, and land beside the mesh in `textures2d`.
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn mesh_textures_use_the_chosen_image_format() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let index = index_chimp_package_types(&world);
        let mesh = world
            .packages()
            .iter()
            .zip(&index.package_types)
            .find(|(package, kind)| {
                kind.as_deref() == Some("SkeletalMesh")
                    && !chimp_mesh_texture_packages(
                        &world,
                        &load_chimp_document(&world, &package.name).unwrap().header,
                    )
                    .is_empty()
            })
            .map(|(package, _)| package.name.clone())
            .expect("a skeletal mesh whose materials reference textures");

        for (format, extension) in [
            (ChimpTextureFormat::Png, "png"),
            (ChimpTextureFormat::Dds, "dds"),
        ] {
            let directory =
                std::env::temp_dir().join(format!("baboon-meshtex-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            write_chimp_mesh(
                &world,
                &mesh,
                &directory.join("mesh.pskx"),
                ChimpMeshFormat::Pskx,
                ChimpTextureScope::All,
                ChimpTextureExport { format, ..Default::default() },
            )
            .unwrap();
            let textures: Vec<_> = std::fs::read_dir(directory.join(CHIMP_TEXTURE_DIR))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert!(!textures.is_empty(), "{mesh} wrote no textures");
            assert!(
                textures.iter().all(|name| name.ends_with(extension)),
                "expected only .{extension}: {textures:?}"
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    /// Scratch helper: export a texture to DDS and report each file's header.
    /// `CE_TEXTURE_PACKAGE` picks it, `CE_TEXTURE_DDS_DIR` says where.
    #[test]
    #[ignore = "manual check; set CE_PAKS and CE_TEXTURE_DDS_DIR"]
    fn real_texture_to_dds_report() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let directory =
            std::path::PathBuf::from(std::env::var("CE_TEXTURE_DDS_DIR").expect("set dir"));
        std::fs::create_dir_all(&directory).unwrap();
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let target = std::env::var("CE_TEXTURE_PACKAGE")
            .unwrap_or_else(|_| "T_Elite_Minor_Armor_N".to_owned())
            .to_ascii_lowercase();
        let package = world
            .packages()
            .iter()
            .map(|package| package.name.clone())
            .find(|name| name.to_ascii_lowercase().ends_with(&target))
            .unwrap_or_else(|| panic!("no package ending in {target:?}"));
        let fmt = match std::env::var("CE_TEXTURE_FORMAT").unwrap_or_default().as_str() { "png" => ChimpTextureFormat::Png, "tif" => ChimpTextureFormat::Tiff, _ => ChimpTextureFormat::Dds };
        let split = std::env::var("CE_TEXTURE_SPLIT").unwrap_or_default() != "0";
        println!("{}", write_chimp_texture(&world, &package, &directory.join(format!("t.{}", fmt.extension())), ChimpTextureExport { format: fmt, split_udim: split }).unwrap());
        let mut names: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for path in names {
            let bytes = std::fs::read(&path).unwrap();
            let at = |offset: usize| {
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
            };
            println!(
                "  {}: {}x{} mips={} dxgi={} bytes={}",
                path.file_name().unwrap().to_string_lossy(),
                at(16),
                at(12),
                at(28),
                at(128),
                bytes.len()
            );
        }
    }

    /// Scratch helper: dump a decoded mip to PNG so it can be eyeballed.
    /// `CE_TEXTURE_PACKAGE` picks the texture, `CE_TEXTURE_MIP` the level and
    /// `CE_TEXTURE_PNG` the output path.
    #[test]
    #[ignore = "manual visual check; set CE_PAKS and CE_TEXTURE_PNG"]
    fn real_texture_mip_to_png() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let out = std::env::var("CE_TEXTURE_PNG").expect("set CE_TEXTURE_PNG");
        let level: usize = std::env::var("CE_TEXTURE_MIP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let target = std::env::var("CE_TEXTURE_PACKAGE")
            .unwrap_or_else(|_| "T_MI_FloodTank_Default_D".to_owned())
            .to_ascii_lowercase();
        let package = world
            .packages()
            .iter()
            .map(|package| package.name.clone())
            .find(|name| name.to_ascii_lowercase().ends_with(&target))
            .unwrap_or_else(|| panic!("no package ending in {target:?}"));
        let document = load_chimp_document(&world, &package).unwrap();
        let surfaces = chimp_selected_surfaces(&document, &package).unwrap();
        let data = chimp_texture_mip_data(surfaces, 0, level).unwrap();
        println!(
            "{package}: {}x{} {} ({})",
            data.width, data.height, data.format_name, data.type_name
        );
        image::RgbaImage::from_raw(data.width, data.height, data.rgba)
            .unwrap()
            .save(&out)
            .unwrap();
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_meshes_preview_and_extract_to_jms_and_actorx() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let type_index = index_chimp_package_types(&world);

        for (type_name, expected_kind) in [
            ("SkeletalMesh", ChimpMeshKind::Skeletal),
            ("StaticMesh", ChimpMeshKind::Static),
        ] {
            let preferred_suffix = match expected_kind {
                ChimpMeshKind::Skeletal => "/SK_Elite_Common_Body",
                ChimpMeshKind::Static => "",
            };
            let mut candidates = world
                .packages()
                .iter()
                .enumerate()
                .filter(|(package_index, _)| {
                    type_index
                        .package_types
                        .get(*package_index)
                        .and_then(Option::as_deref)
                        == Some(type_name)
                })
                .map(|(_, package)| package)
                .collect::<Vec<_>>();
            candidates.sort_by_key(|package| {
                !package
                    .name
                    .to_ascii_lowercase()
                    .ends_with(&preferred_suffix.to_ascii_lowercase())
            });
            let package = candidates
                .into_iter()
                .find_map(|package| {
                    let document = load_chimp_document(&world, &package.name).ok()?;
                    document.mesh_preview.as_ref()?.as_ref().ok()?;
                    Some(package.name.clone())
                })
                .unwrap_or_else(|| panic!("no decodable {type_name} package"));
            let document = load_chimp_document(&world, &package).unwrap();
            assert_eq!(document.view, ChimpDocumentView::Mesh);
            assert_eq!(document.mesh_kind, Some(expected_kind));
            let preview = document.mesh_preview.as_ref().unwrap().as_ref().unwrap();
            assert!(!preview.preview.vertices.is_empty());
            assert!(!preview.preview.indices.is_empty());
            assert!(!preview.preview.batches.is_empty());
            let (signed_alignment, absolute_alignment) = preview_normal_alignment(preview);
            eprintln!(
                "{package}: winding-signed normal alignment {signed_alignment:.3}, magnitude {absolute_alignment:.3}"
            );
            assert!(
                absolute_alignment > 0.45,
                "{package} normals do not follow the decoded surface ({absolute_alignment:.3})"
            );

            let formats = if expected_kind == ChimpMeshKind::Skeletal
                && preview.preview.vertices.len() <= 65_536
            {
                vec![
                    ChimpMeshFormat::Jms,
                    ChimpMeshFormat::Psk,
                    ChimpMeshFormat::Pskx,
                ]
            } else {
                vec![ChimpMeshFormat::Jms, ChimpMeshFormat::Pskx]
            };
            for format in formats {
                let output = std::env::temp_dir().join(format!(
                    "baboon-chimp-mesh-{}.{}",
                    uuid::Uuid::new_v4(),
                    format.extension()
                ));
                write_chimp_mesh(&world, &package, &output, format, ChimpTextureScope::None, ChimpTextureExport::default()).unwrap();
                let bytes = std::fs::read(&output).unwrap();
                match format {
                    ChimpMeshFormat::Jms => assert!(bytes.starts_with(b";### VERSION ###")),
                    ChimpMeshFormat::Psk => {
                        assert!(bytes.windows(8).any(|window| window == b"FACE0000"))
                    }
                    ChimpMeshFormat::Pskx => {
                        assert!(bytes.windows(8).any(|window| window == b"FACE3200"))
                    }
                }
                std::fs::remove_file(output).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn real_spiritdropship_nanite_export_is_complete() {
        let root = std::env::var_os("CE_PAKS").expect("set CE_PAKS");
        let world = World::open(root, Usmap::meteorite().unwrap()).unwrap();
        let package = world
            .packages()
            .iter()
            .find(|package| {
                package
                    .name
                    .to_ascii_lowercase()
                    .contains("sm_spiritdropship_body")
            })
            .unwrap_or_else(|| panic!("SM_SpiritDropShip_Body was not found"));
        let document = load_chimp_document(&world, &package.name).unwrap();
        assert_eq!(document.mesh_kind, Some(ChimpMeshKind::Static));

        let archive = &world.archives()[document.provider.container];
        let chunk = archive
            .chunk_index_for(&document.provider.entry_path)
            .expect("static mesh package has an IoStore chunk");
        let bulk = archive
            .read_bulk_for(chunk, 0)
            .expect("Nanite static mesh has readable bulk data");
        let resources = blam_tags::iostore::nanite::NaniteResources::parse(
            &document.original,
            document.header.summary.header_size as usize,
        )
        .expect("static mesh has Nanite resources");
        let nanite =
            blam_tags::iostore::nanite::decode_nanite(&document.original, &bulk, &resources);
        let mut miswound_triangles = 0usize;
        let mut duplicate_index_triangles = 0usize;
        let mut zero_area_triangles = 0usize;
        for triangle in &nanite.triangles {
            if triangle[0] == triangle[1]
                || triangle[1] == triangle[2]
                || triangle[0] == triangle[2]
            {
                duplicate_index_triangles += 1;
            }
            let [a, b, c] = triangle.map(|index| nanite.positions[index as usize]);
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if face[0] * face[0] + face[1] * face[1] + face[2] * face[2] <= 1.0e-12 {
                zero_area_triangles += 1;
            }
            let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
                let normal = nanite.normals[*index as usize];
                total[0] += normal[0];
                total[1] += normal[1];
                total[2] += normal[2];
                total
            });
            if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
                miswound_triangles += 1;
            }
        }
        let converted = StaticMesh::from_nanite(&nanite);
        let preview = document
            .mesh_preview
            .as_ref()
            .and_then(|preview| preview.as_ref().ok())
            .expect("Nanite static mesh has a Chimp preview");
        assert_eq!(preview.preview.vertices.len(), converted.vertices.len());
        assert_eq!(preview.preview.indices.len(), converted.indices.len());
        let mut converted_miswound_triangles = 0usize;
        let mut severe_uv_stretch_triangles = 0usize;
        let mut maximum_uv_per_cm = 0.0f32;
        let mut long_uv_edge_triangles = 0usize;
        let mut negative_wrap_span_triangles = 0usize;
        let mut maximum_uv_edge = 0.0f32;
        for triangle in converted.indices.chunks_exact(3) {
            let a = converted.vertices[triangle[0] as usize].position;
            let b = converted.vertices[triangle[1] as usize].position;
            let c = converted.vertices[triangle[2] as usize].position;
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let normal = triangle.iter().fold([0.0; 3], |mut total, index| {
                let normal = converted.vertices[*index as usize].normal;
                total[0] += normal[0];
                total[1] += normal[1];
                total[2] += normal[2];
                total
            });
            if face[0] * normal[0] + face[1] * normal[1] + face[2] * normal[2] > 0.0 {
                converted_miswound_triangles += 1;
            }
            let mut severe_uv_stretch = false;
            let mut long_uv_edge = false;
            for [left, right] in [[0usize, 1usize], [1, 2], [2, 0]] {
                let left = &converted.vertices[triangle[left] as usize];
                let right = &converted.vertices[triangle[right] as usize];
                let dx = left.position[0] - right.position[0];
                let dy = left.position[1] - right.position[1];
                let dz = left.position[2] - right.position[2];
                let position_distance_squared = dx * dx + dy * dy + dz * dz;
                if position_distance_squared <= 1.0e-12 {
                    continue;
                }
                let du = left.uv[0] - right.uv[0];
                let dv = left.uv[1] - right.uv[1];
                let uv_edge = (du * du + dv * dv).sqrt();
                maximum_uv_edge = maximum_uv_edge.max(uv_edge);
                long_uv_edge |= uv_edge > 2.0;
                let uv_per_cm = ((du * du + dv * dv) / position_distance_squared).sqrt();
                maximum_uv_per_cm = maximum_uv_per_cm.max(uv_per_cm);
                severe_uv_stretch |= uv_per_cm > 10.0;
            }
            severe_uv_stretch_triangles += usize::from(severe_uv_stretch);
            long_uv_edge_triangles += usize::from(long_uv_edge);
            for axis in 0..2 {
                let coordinates = [triangle[0], triangle[1], triangle[2]]
                    .map(|index| converted.vertices[index as usize].uv[axis]);
                let minimum = coordinates.iter().copied().fold(f32::INFINITY, f32::min);
                let maximum = coordinates
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max);
                if minimum < -1.0 && maximum < 0.0 && maximum - minimum > 1.0 {
                    negative_wrap_span_triangles += 1;
                    break;
                }
            }
        }
        eprintln!(
            "UV wrap diagnostics: long_edges={long_uv_edge_triangles}, negative_wrap_spans={negative_wrap_span_triangles}, maximum_edge={maximum_uv_edge}"
        );
        let mut position_ids = std::collections::HashMap::<[u32; 3], u32>::new();
        let mut canonical_vertices = Vec::with_capacity(converted.vertices.len());
        for vertex in &converted.vertices {
            let key = vertex
                .position
                .map(|value| if value == 0.0 { 0 } else { value.to_bits() });
            let next = position_ids.len() as u32;
            canonical_vertices.push(*position_ids.entry(key).or_insert(next));
        }
        let mut edges = Vec::with_capacity(converted.indices.len());
        for triangle in converted.indices.chunks_exact(3) {
            let ids = [
                canonical_vertices[triangle[0] as usize],
                canonical_vertices[triangle[1] as usize],
                canonical_vertices[triangle[2] as usize],
            ];
            if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                continue;
            }
            for [a, b] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
                let [lo, hi] = if a < b { [a, b] } else { [b, a] };
                edges.push((u64::from(lo) << 32) | u64::from(hi));
            }
        }
        edges.sort_unstable();
        let mut boundary_edges = Vec::new();
        let mut cursor = 0usize;
        while cursor < edges.len() {
            let edge = edges[cursor];
            let mut end = cursor + 1;
            while end < edges.len() && edges[end] == edge {
                end += 1;
            }
            if end - cursor == 1 {
                boundary_edges.push(edge);
            }
            cursor = end;
        }
        let mut boundary_adjacency = std::collections::HashMap::<u32, Vec<u32>>::new();
        for edge in &boundary_edges {
            let a = (edge >> 32) as u32;
            let b = *edge as u32;
            boundary_adjacency.entry(a).or_default().push(b);
            boundary_adjacency.entry(b).or_default().push(a);
        }
        let mut visited = std::collections::HashSet::new();
        let mut triangular_boundary_loops = 0usize;
        let mut triangular_hole_edges = std::collections::HashMap::<u64, u32>::new();
        for &start in boundary_adjacency.keys() {
            if !visited.insert(start) {
                continue;
            }
            let mut stack = vec![start];
            let mut vertices = 0usize;
            let mut degree_sum = 0usize;
            let mut component = Vec::new();
            while let Some(vertex) = stack.pop() {
                vertices += 1;
                component.push(vertex);
                let neighbours = &boundary_adjacency[&vertex];
                degree_sum += neighbours.len();
                for &neighbour in neighbours {
                    if visited.insert(neighbour) {
                        stack.push(neighbour);
                    }
                }
            }
            if vertices == 3 && degree_sum == 6 {
                triangular_boundary_loops += 1;
                for index in 0..3 {
                    let a = component[index];
                    let b = component[(index + 1) % 3];
                    let third = component[(index + 2) % 3];
                    let [lo, hi] = if a < b { [a, b] } else { [b, a] };
                    triangular_hole_edges.insert((u64::from(lo) << 32) | u64::from(hi), third);
                }
            }
        }
        let mut paired_degenerate_triangles = 0usize;
        let mut paired_zero_area_holes = std::collections::HashSet::<[u32; 3]>::new();
        for triangle in converted.indices.chunks_exact(3) {
            let ids = [
                canonical_vertices[triangle[0] as usize],
                canonical_vertices[triangle[1] as usize],
                canonical_vertices[triangle[2] as usize],
            ];
            let mut distinct = ids;
            distinct.sort_unstable();
            let distinct_len = if distinct[0] == distinct[2] {
                1
            } else if distinct[0] == distinct[1] || distinct[1] == distinct[2] {
                2
            } else {
                3
            };
            if distinct_len == 2 {
                let a = distinct[0];
                let b = distinct[2];
                let edge = (u64::from(a) << 32) | u64::from(b);
                paired_degenerate_triangles +=
                    usize::from(triangular_hole_edges.contains_key(&edge));
            }

            let positions = [
                converted.vertices[triangle[0] as usize].position,
                converted.vertices[triangle[1] as usize].position,
                converted.vertices[triangle[2] as usize].position,
            ];
            let ab = [
                positions[1][0] - positions[0][0],
                positions[1][1] - positions[0][1],
                positions[1][2] - positions[0][2],
            ];
            let ac = [
                positions[2][0] - positions[0][0],
                positions[2][1] - positions[0][1],
                positions[2][2] - positions[0][2],
            ];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2] > 1.0e-12 {
                continue;
            }
            for [x, y] in [[ids[0], ids[1]], [ids[1], ids[2]], [ids[2], ids[0]]] {
                if x == y {
                    continue;
                }
                let [lo, hi] = if x < y { [x, y] } else { [y, x] };
                let edge = (u64::from(lo) << 32) | u64::from(hi);
                if let Some(&third) = triangular_hole_edges.get(&edge) {
                    let mut hole = [lo, hi, third];
                    hole.sort_unstable();
                    paired_zero_area_holes.insert(hole);
                }
            }
        }
        eprintln!(
            "{}: input_triangles={}, decoded_triangles={}, duplicate_index_triangles={}, zero_area_triangles={}, miswound_before={}, miswound_after={}, severe_uv_stretch_triangles={}, maximum_uv_per_cm={}, boundary_edges={}, triangular_boundary_loops={}, paired_degenerate_triangles={}, paired_zero_area_holes={}, unresolved_vertices={}",
            package.name,
            resources.num_input_triangles,
            nanite.triangles.len(),
            duplicate_index_triangles,
            zero_area_triangles,
            miswound_triangles,
            converted_miswound_triangles,
            severe_uv_stretch_triangles,
            maximum_uv_per_cm,
            boundary_edges.len(),
            triangular_boundary_loops,
            paired_degenerate_triangles,
            paired_zero_area_holes.len(),
            nanite.unresolved_vertices,
        );
        assert_eq!(nanite.unresolved_vertices, 0);
        assert_eq!(
            nanite.triangles.len(),
            resources.num_input_triangles as usize
        );
        assert!(
            miswound_triangles > 0,
            "fixture should exercise the regression"
        );
        assert_eq!(converted_miswound_triangles, 0);
        assert_eq!(
            severe_uv_stretch_triangles, 0,
            "repaired Nanite faces must remain on their local UV seams"
        );
        assert!(maximum_uv_per_cm < 5.0);
        assert_eq!(
            negative_wrap_span_triangles, 0,
            "negative repeating UV faces should be split at wrap boundaries"
        );
        assert_eq!(long_uv_edge_triangles, 0);
        assert!(
            triangular_boundary_loops <= 1,
            "the Nanite repair should remove the mass triangular-hole pattern"
        );

        let expected_faces = converted
            .indices
            .chunks_exact(3)
            .filter(|triangle| {
                triangle[0] != triangle[1]
                    && triangle[1] != triangle[2]
                    && triangle[0] != triangle[2]
            })
            .count();
        let jms = blam_tags::iostore::actorx::static_mesh_to_jms(&converted, &[]);
        assert_eq!(jms.triangles.len(), expected_faces);
        let mut jms_bytes = Vec::new();
        jms.write(&mut jms_bytes, 8213).unwrap();
        assert!(jms_bytes.starts_with(b";### VERSION ###"));
        let output = std::env::temp_dir().join(format!(
            "baboon-spiritdropship-{}.pskx",
            uuid::Uuid::new_v4()
        ));
        write_chimp_mesh(
            &world,
            &package.name,
            &output,
            ChimpMeshFormat::Pskx,
            ChimpTextureScope::None,
            ChimpTextureExport::default(),
        )
        .unwrap();
        let bytes = std::fs::read(&output).unwrap();
        let face_chunk = bytes
            .windows(8)
            .position(|window| window == b"FACE3200")
            .expect("PSKX contains its 32-bit face chunk");
        let face_count =
            i32::from_le_bytes(bytes[face_chunk + 28..face_chunk + 32].try_into().unwrap())
                as usize;
        assert_eq!(face_count, expected_faces);
        std::fs::remove_file(output).unwrap();
    }
}
