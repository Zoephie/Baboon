//! Chimp extraction: packages, textures, meshes and levels out to files.
//! It owns the export prompts, menus, workers and writers; decoding belongs in `package` and saving edits in `save`.

use super::*;

/// A mesh export waiting on the answer to "textures as well?".
///
/// The destination is already chosen at this point, so the choice can be made
/// against the folder the textures would actually appear in.
pub(in crate::app) struct ChimpMeshTexturePrompt {
    pub(super) kit: KitId,
    pub(in crate::app) package: String,
    /// Read through [`ChimpMeshTexturePrompt::format_label`]; the format itself
    /// is Chimp's own business.
    format: ChimpMeshFormat,
    /// Which image format the textures would be written as, if any are. Asked
    /// here rather than in a second window, because it is only one more choice
    /// about the same export and the answer only matters alongside the two
    /// buttons that ask for textures at all.
    pub(in crate::app) texture_export: ChimpTextureExport,
    pub(in crate::app) path: PathBuf,
}

impl ChimpMeshTexturePrompt {
    /// Where the textures would be written.
    pub(in crate::app) fn texture_directory(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(CHIMP_TEXTURE_DIR)
    }

    pub(in crate::app) fn format_label(&self) -> &'static str {
        self.format.label()
    }

    /// The word "matching textures" would filter on, if one can be derived.
    pub(in crate::app) fn texture_subject(&self) -> Option<String> {
        chimp_mesh_texture_subject(&self.package)
    }

    /// A prompt for `package` as a JMS export into a folder that does not
    /// exist, for tests that draw it.
    #[cfg(test)]
    pub(in crate::app) fn for_test(kit: KitId, package: &str) -> Self {
        Self {
            kit,
            package: package.to_owned(),
            format: ChimpMeshFormat::Jms,
            texture_export: ChimpTextureExport::default(),
            path: PathBuf::from("/no/such/folder/mesh.jms"),
        }
    }
}

/// How a Texture2D extraction should be written.
///
/// All three split a UDIM set into 1001-numbered files and give each block its
/// authored resolution. DDS additionally keeps the cooked pixel format and the
/// whole mip chain; TIFF and PNG are one flat RGBA8 image per block.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ChimpTextureFormat {
    Dds,
    Tiff,
    Png,
}

impl ChimpTextureFormat {
    pub(in crate::app) const ALL: [Self; 3] = [Self::Dds, Self::Tiff, Self::Png];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Dds => "DDS",
            Self::Tiff => "TIFF",
            Self::Png => "PNG",
        }
    }

    /// What choosing this format actually gets you, in the prompt.
    pub(in crate::app) fn summary(self) -> &'static str {
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
pub(in crate::app) struct ChimpTextureExportPrompt {
    pub(super) kit: KitId,
    pub(super) package: String,
    pub(in crate::app) export: ChimpTextureExport,
    /// The export the open document has selected. The extraction loads the
    /// package afresh, so without this it could only ever see export 0.
    pub(super) export_index: Option<usize>,
}

impl ChimpTextureExportPrompt {
    pub(in crate::app) fn name(&self) -> &str {
        self.package.rsplit('/').next().unwrap_or(&self.package)
    }

    #[cfg(test)]
    pub(in crate::app) fn for_test(kit: KitId, package: &str) -> Self {
        Self {
            kit,
            package: package.to_owned(),
            export: ChimpTextureExport::default(),
            export_index: None,
        }
    }
}

/// One entry; the format is chosen in the prompt it opens.
pub(super) fn chimp_texture_export_menu(
    ui: &mut egui::Ui,
    package: &str,
    out: &mut Option<String>,
) {
    if ui.button("Extract Texture2D…").clicked() {
        *out = Some(package.to_owned());
        close_menu(ui);
    }
}

/// How a whole level leaves Baboon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ChimpLevelFormat {
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
pub(in crate::app) enum ChimpLevelPhase {
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
pub(in crate::app) struct ChimpLevelJob {
    /// Never reused, so a finished export can tell whether it is still the
    /// current job. See [`next_chimp_level_job_id`].
    pub(in crate::app) id: u64,
    pub(in crate::app) kit: KitId,
    pub(super) name: String,
    pub(in crate::app) phase: ChimpLevelPhase,
    pub(in crate::app) done: usize,
    pub(in crate::app) total: usize,
    /// When the current phase began, so the estimate is of the work being done
    /// rather than an average across phases that cost different amounts.
    pub(in crate::app) phase_started: Instant,
}

/// A fresh id for a [`ChimpLevelJob`].
pub(in crate::app) fn next_chimp_level_job_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl ChimpLevelJob {
    #[cfg(test)]
    pub(in crate::app) fn for_test(id: u64, kit: KitId) -> Self {
        Self {
            id,
            kit,
            name: "level".to_owned(),
            phase: ChimpLevelPhase::ReadingCells,
            done: 0,
            total: 1,
            phase_started: Instant::now(),
        }
    }

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
pub(in crate::app) fn format_remaining(remaining: Duration) -> String {
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
pub(in crate::app) struct ChimpLevelExportPrompt {
    pub(super) kit: KitId,
    pub(in crate::app) package: String,
    /// The `_Generated_` cells this level is made of.
    pub(in crate::app) cells: Vec<String>,
    pub(in crate::app) format: ChimpLevelFormat,
    /// Full Nanite geometry rather than the coarse fallback proxy.
    pub(in crate::app) nanite: bool,
    pub(in crate::app) split: bool,
    pub(in crate::app) triangles: usize,
    pub(in crate::app) placements: usize,
}

impl ChimpLevelExportPrompt {
    pub(in crate::app) fn format_label(&self) -> &'static str {
        self.format.label()
    }

    /// A prompt for a two-cell level, for tests that draw it.
    #[cfg(test)]
    pub(in crate::app) fn for_test(kit: KitId, package: &str) -> Self {
        let default = SegmentBudget::default();
        Self {
            kit,
            package: package.to_owned(),
            cells: vec![format!("{package}_Generated_0"), format!("{package}_Generated_1")],
            format: ChimpLevelFormat::SegmentedUsd,
            nanite: true,
            split: true,
            triangles: default.triangles,
            placements: default.placements,
        }
    }

    pub(in crate::app) fn format_summary(&self) -> &'static str {
        self.format.summary()
    }

    /// What the exported files are named after.
    pub(in crate::app) fn name(&self) -> String {
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
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        "level".to_owned()
    } else {
        name
    }
}

/// Whether a package is shaped like a World Partition persistent level.
///
/// A cheap name test rather than a search, because it runs while a context menu
/// is open and the real answer means scanning every package in the install.
/// Unreal names a persistent level after the folder that holds it — C10 lives at
/// `.../C10/C10` — so that is what this looks for, and
/// [`chimp_level_cells`] decides for certain once the menu is actually used.
pub(super) fn chimp_looks_like_level(package: &str) -> bool {
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
pub(in crate::app) enum ChimpMeshFormat {
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

/// The bytes of export `index` as the document holds it now.
///
/// `payloads` are the bytes the package was read with. An edit changes the
/// decoded export, not its payload, so for a dirty document those bytes are
/// stale; `serialize` writes the decoded export instead, as rebuilding the
/// package does. An export that never decoded has only its payload. `None`
/// when there is no such export.
fn chimp_export_bytes(
    document: &ChimpDocument,
    index: usize,
    serialize: impl FnOnce(&str, &Export) -> Result<Vec<u8>, String>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(payload) = document.payloads.get(index) else {
        return Ok(None);
    };
    if document.dirty
        && let Some(export) = document.exports.get(index)
        && let (Some(class), Ok(decoded)) = (export.class.as_deref(), &export.decoded)
    {
        return serialize(class, decoded).map(Some);
    }
    Ok(Some(payload.clone()))
}

impl Baboon {
    pub(super) fn extract_chimp_package(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.model.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let bytes = if document.dirty {
            let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
                return;
            };
            match rebuild_chimp_document(world, document) {
                Ok((bytes, _)) => bytes,
                Err(error) => {
                    self.model.status = error;
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
            Ok(()) => self.model.status = format!("Extracted {}", path.display()),
            Err(error) => self.model.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    pub(super) fn extract_chimp_export(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.model.kits[kit_index].chimp.documents.get(package) else {
            return;
        };
        let Some(pane) = self.views[self.model.kits[kit_index].id].chimp.documents.get(package) else {
            return;
        };
        let index = pane
            .selected_export
            .min(document.payloads.len().saturating_sub(1));
        let world = match &self.model.kits[kit_index].chimp.mount {
            ChimpMount::Ready(world) => Some(world),
            _ => None,
        };
        let payload = match chimp_export_bytes(document, index, |class, decoded| {
            let world = world.ok_or("Chimp must be mounted to extract an edited export")?;
            validate_chimp_header(document)?;
            let names = document.header.name_map.copy_raw_names();
            let resolver = world.resolver(&document.header, &document.original, &names);
            write_export_in(class, decoded, world.usmap(), Some(&resolver))
                .map_err(|error| format!("Could not serialize export {index}: {error:#}"))
        }) {
            Ok(Some(payload)) => payload,
            Ok(None) => return,
            Err(error) => {
                self.model.status = error;
                return;
            }
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
            Ok(()) => self.model.status = format!("Extracted {}", path.display()),
            Err(error) => self.model.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    pub(super) fn extract_chimp_json(&mut self, kit_index: usize, package: &str) {
        let Some(document) = self.model.kits[kit_index].chimp.documents.get(package) else {
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
            Ok(()) => self.model.status = format!("Exported {}", path.display()),
            Err(error) => self.model.status = format!("Could not write {}: {error}", path.display()),
        }
    }

    /// Ask which image format to extract a Texture2D as.
    ///
    /// The save dialog comes after the answer, because the format decides both
    /// what the export is — a numbered UDIM set or a single file, a mip chain or
    /// one flat image — and what the picker should be named and filtered for.
    pub(super) fn begin_extract_chimp_texture(&mut self, kit_index: usize, package: &str) {
        if !matches!(self.model.kits[kit_index].chimp.mount, ChimpMount::Ready(_)) {
            return;
        }
        let export_index = self.views[self.model.kits[kit_index].id]
            .chimp
            .documents
            .get(package)
            .map(|pane| pane.selected_export);
        self.dialogs.open(ChimpTextureExportPrompt {
            kit: self.model.kits[kit_index].id,
            package: package.to_owned(),
            // DDS and split UDIM: the pair that round-trips into Unreal.
            export: ChimpTextureExport::default(),
            export_index,
        });
    }

    /// Pick a destination and run the extraction the prompt described.
    pub(in crate::app) fn start_chimp_texture_export(
        &mut self,
        prompt: ChimpTextureExportPrompt,
        ctx: egui::Context,
    ) {
        let ChimpTextureExportPrompt {
            kit,
            package,
            export,
            export_index,
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
        let Some(kit_index) = self.model.kits.iter().position(|entry| entry.id == kit) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return;
        };
        let world = world.clone();
        let tx = self.tx.clone();
        self.model.status = format!("Extracting {package}…");
        spawn_export(&tx, &ctx, move || {
            write_chimp_texture(&world, &package, &path, export, export_index)
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
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
            return;
        };
        let cells = chimp_level_cells(world, package);
        if cells.is_empty() {
            self.model.status = format!("{package} is not a World Partition level");
            return;
        }
        let default = SegmentBudget::default();
        self.dialogs.open(ChimpLevelExportPrompt {
            kit: self.model.kits[kit_index].id,
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

    pub(in crate::app) fn start_chimp_level_export(
        &mut self,
        prompt: ChimpLevelExportPrompt,
        ctx: egui::Context,
    ) {
        let Some(directory) = rfd::FileDialog::new()
            .set_title(format!(
                "Export {} as {}",
                prompt.name(),
                prompt.format_label()
            ))
            .pick_folder()
        else {
            return;
        };
        let Some(kit_index) = self.model.kits.iter().position(|kit| kit.id == prompt.kit) else {
            return;
        };
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
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
        let job = next_chimp_level_job_id();
        self.chimp.chimp_level_job = Some(ChimpLevelJob {
            id: job,
            kit,
            name: name.clone(),
            phase: ChimpLevelPhase::ReadingCells,
            done: 0,
            total: cells.len(),
            phase_started: Instant::now(),
        });
        self.model.status = format!("Exporting {name}…");
        let panic_name = name.clone();
        spawn_worker(
            &self.tx.clone(),
            &ctx.clone(),
            move || {
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
                let left_out = scene.skipped.summary();
                let result = match format {
                    ChimpLevelFormat::SegmentedUsd => write_segmented_usd(
                        &world, &scene, detail, &directory, &name, budget, &stage,
                    )
                    .map_err(|error| error.to_string())
                    .map(|report| {
                        format!(
                            "Exported {name}: {} meshes, {} placements, {} segment(s)",
                            report.prototypes, report.instances, report.segments
                        )
                    }),
                    ChimpLevelFormat::Blender => write_blend_export(
                        &world, &scene, detail, &directory, &name, budget, &stage,
                    )
                    .map_err(|error| error.to_string())
                    .map(|report| {
                        format!(
                            "Exported {name}: {} meshes, {} placements, {} master(s). \
                                 Run build_blend.py in Blender to build them.",
                            report.meshes, report.placements, report.segments
                        )
                    }),
                };
                WorkerMessage::ChimpLevelExportFinished {
                    job,
                    result: result.map(|message| match left_out {
                        Some(left_out) => format!("{message}. {left_out}"),
                        None => message,
                    }),
                }
            },
            move |error| WorkerMessage::ChimpLevelExportFinished {
                job,
                result: Err(format!("Exporting {panic_name} failed: {error}")),
            },
        );
    }

    pub(super) fn begin_extract_chimp_mesh(
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
        if !matches!(self.model.kits[kit_index].chimp.mount, ChimpMount::Ready(_)) {
            return;
        }
        // Asked once the destination is known, so the prompt can say exactly
        // where the textures would land.
        self.dialogs.open(ChimpMeshTexturePrompt {
            kit: self.model.kits[kit_index].id,
            package: package.to_owned(),
            format,
            texture_export: ChimpTextureExport::default(),
            path,
        });
    }

    /// Run a mesh export that has been through the texture prompt.
    pub(in crate::app) fn start_chimp_mesh_export(
        &mut self,
        prompt: ChimpMeshTexturePrompt,
        textures: ChimpTextureScope,
        ctx: egui::Context,
    ) {
        let Some(kit_index) = self.model.kit_index(prompt.kit) else {
            self.model.status = "The workspace this export came from is closed".to_owned();
            return;
        };
        let ChimpMount::Ready(world) = &self.model.kits[kit_index].chimp.mount else {
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
        self.model.status = format!("Extracting {package} as {}…", format.label());
        spawn_export(&tx, &ctx, move || {
            write_chimp_mesh(&world, &package, &path, format, textures, texture_export)
        });
    }
}

/// Where a mesh's textures are written, relative to the mesh itself.
pub(super) const CHIMP_TEXTURE_DIR: &str = "textures2d";

/// How much of a mesh's texture set to export alongside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ChimpTextureScope {
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
        let Ok(document) = load_chimp_package(world, material) else {
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
        // Decoded once, here, and written from the same surfaces: this used
        // to decode a whole document to find out whether it was a texture,
        // then load and decode it all again to write it.
        let Ok(previews) = load_chimp_texture_previews(world, &package) else {
            continue;
        };
        if previews.is_empty() {
            continue;
        }
        if !created {
            if let Err(error) = fs::create_dir_all(directory) {
                failures.push(format!("Could not create {}: {error}", directory.display()));
                return (written, failures);
            }
            created = true;
        }
        let output = directory.join(format!("{leaf}.{}", options.format.extension()));
        match write_chimp_texture_previews(&previews, &package, &output, options, None) {
            Ok(_) => written += 1,
            Err(error) => failures.push(error),
        }
    }
    (written, failures)
}

/// The texture surfaces of `export_index`, or of the first Texture2D when it
/// names none (or names an export that is not a texture).
///
/// The index has to be passed in: the export loads the package afresh, and a
/// fresh document's own selection is always export 0, so reading it here
/// exported the first texture whatever the combo box pointed at.
fn chimp_selected_surfaces<'a>(
    previews: &'a [ChimpTexturePreview],
    package: &str,
    export_index: Option<usize>,
) -> Result<&'a Texture2dSurfaces, String> {
    let selected = selected_texture_preview(previews, export_index)
        .ok_or_else(|| format!("{package} has no Texture2D export"))?;
    selected
        .surfaces
        .as_ref()
        .map_err(|error| format!("{package}: {error}"))
}

fn selected_texture_preview(
    previews: &[ChimpTexturePreview],
    export_index: Option<usize>,
) -> Option<&ChimpTexturePreview> {
    export_index
        .and_then(|index| {
            previews
                .iter()
                .find(|texture| texture.export_index == index)
        })
        .or_else(|| previews.first())
}

/// How a Texture2D should be written out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct ChimpTextureExport {
    pub(in crate::app) format: ChimpTextureFormat,
    /// Split a UDIM set into one numbered file per block. Off writes the whole
    /// set as the single stitched image it was reassembled into, for engines
    /// that have no UDIM support to import it with.
    pub(in crate::app) split_udim: bool,
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
pub(super) fn write_chimp_texture(
    world: &World,
    package: &str,
    output: &Path,
    options: ChimpTextureExport,
    export_index: Option<usize>,
) -> Result<String, String> {
    let previews = load_chimp_texture_previews(world, package)?;
    write_chimp_texture_previews(&previews, package, output, options, export_index)
}

/// Write the chosen texture out of already-decoded previews.
fn write_chimp_texture_previews(
    previews: &[ChimpTexturePreview],
    package: &str,
    output: &Path,
    options: ChimpTextureExport,
    export_index: Option<usize>,
) -> Result<String, String> {
    let ChimpTextureExport { format, split_udim } = options;
    let surfaces = chimp_selected_surfaces(previews, package, export_index)?;
    let stem = output
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "texture".to_owned());
    let directory = output.parent().unwrap_or(Path::new("."));

    // Splitting is only a question for a set with more than one block; a plain
    // texture is one file either way.
    let split = split_udim && surfaces.is_udim();
    let (blocks_x, blocks_y) = if split {
        (
            surfaces.width_in_blocks.max(1),
            surfaces.height_in_blocks.max(1),
        )
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
        ChimpTextureFormat::Png => image::RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "image does not match its dimensions".to_owned())?
            .write_to(
                &mut std::io::BufWriter::new(&mut file),
                image::ImageFormat::Png,
            )
            .map_err(|error| format!("Could not encode {}: {error}", path.display())),
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
        if format_name.get_or_insert_with(|| surface.pixel_format.clone()) != &surface.pixel_format
        {
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
    // Exports only: the document's own mesh preview would decode the same
    // geometry a second time, and render text panes nothing reads.
    let document = load_chimp_package(world, package)?;
    let kind = chimp_mesh_kind(&document.exports)
        .ok_or_else(|| format!("{package} is not a StaticMesh or SkeletalMesh"))?;
    let materials = chimp_material_names(&document.header);
    let mut writer = std::io::BufWriter::new(
        fs::File::create(output)
            .map_err(|error| format!("Could not create {}: {error}", output.display()))?,
    );
    match kind {
        ChimpMeshKind::Skeletal => {
            let mesh = SkeletalMesh::from_package(
                &document.bytes,
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
                &document.bytes,
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

pub(super) fn chimp_mesh_export_menu(
    ui: &mut Ui,
    package: &str,
    requested: &mut Option<(String, ChimpMeshFormat)>,
) {
    let format = right_opening_menu_button(ui, "Extract mesh", 220.0, |ui| {
        style_list_menu(ui);
        for format in [
            ChimpMeshFormat::Jms,
            ChimpMeshFormat::Psk,
            ChimpMeshFormat::Pskx,
        ] {
            if ui.button(format.label()).clicked() {
                return Some(format);
            }
        }
        None
    })
    .inner
    .flatten();
    if let Some(format) = format {
        *requested = Some((package.to_owned(), format));
        close_menu(ui);
    }
}

/// Offered only where it means something: a package with `_Generated_` cells
/// beside it is a World Partition level, and everything else is one package.
pub(super) fn chimp_level_export_menu(
    ui: &mut Ui,
    package: &str,
    requested: &mut Option<(String, ChimpLevelFormat)>,
) {
    let selected = right_opening_menu_button(ui, "Export level", 220.0, |ui| {
        style_list_menu(ui);
        for format in [ChimpLevelFormat::SegmentedUsd, ChimpLevelFormat::Blender] {
            if ui
                .button(format.label())
                .on_hover_text(format.summary())
                .clicked()
            {
                return Some(format);
            }
        }
        None
    })
    .inner
    .flatten();
    if let Some(format) = selected {
        *requested = Some((package.to_owned(), format));
        close_menu(ui);
    }
}

#[cfg(test)]
mod tests;
