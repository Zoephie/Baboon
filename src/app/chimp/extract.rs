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
}

/// One entry; the format is chosen in the prompt it opens.
pub(super) fn chimp_texture_export_menu(
    ui: &mut egui::Ui,
    package: &str,
    out: &mut Option<String>,
) {
    if ui.button("Extract Texture2D…").clicked() {
        *out = Some(package.to_owned());
        ui.close_menu();
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
    pub(in crate::app) kit: KitId,
    pub(super) name: String,
    pub(in crate::app) phase: ChimpLevelPhase,
    pub(in crate::app) done: usize,
    pub(in crate::app) total: usize,
    /// When the current phase began, so the estimate is of the work being done
    /// rather than an average across phases that cost different amounts.
    pub(in crate::app) phase_started: Instant,
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
pub(super) enum ChimpMeshFormat {
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

impl Baboon {
    pub(super) fn extract_chimp_package(&mut self, kit_index: usize, package: &str) {
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

    pub(super) fn extract_chimp_export(&mut self, kit_index: usize, package: &str) {
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

    pub(super) fn extract_chimp_json(&mut self, kit_index: usize, package: &str) {
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
    pub(super) fn begin_extract_chimp_texture(&mut self, kit_index: usize, package: &str) {
        if !matches!(self.kits[kit_index].chimp.mount, ChimpMount::Ready(_)) {
            return;
        }
        let export_index = self.kits[kit_index]
            .chimp
            .documents
            .get(package)
            .map(|document| document.selected_export);
        self.chimp_texture_export_prompt = Some(ChimpTextureExportPrompt {
            kit: self.kits[kit_index].id,
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
            let result = write_chimp_texture(&world, &package, &path, export, export_index);
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
                WorkerMessage::ExportFinished(result.map(|message| match left_out {
                    Some(left_out) => format!("{message}. {left_out}"),
                    None => message,
                }))
            },
            move |error| {
                WorkerMessage::ExportFinished(Err(format!(
                    "Exporting {panic_name} failed: {error}"
                )))
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
    pub(in crate::app) fn start_chimp_mesh_export(
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
        ui.close_menu();
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
        ui.close_menu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(
            job_at(0, 2334, Duration::from_secs(10))
                .remaining()
                .is_none()
        );
        assert!(
            job_at(3, 2334, Duration::from_millis(200))
                .remaining()
                .is_none()
        );
        // Finished is not "0s left", it is nothing to say.
        assert!(
            job_at(2334, 2334, Duration::from_secs(60))
                .remaining()
                .is_none()
        );
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
        let surfaces = chimp_selected_surfaces(&document.texture_previews, &package, None).unwrap();
        assert_eq!(
            (surfaces.width_in_blocks, surfaces.height_in_blocks),
            (3, 2)
        );

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
            ChimpTextureExport {
                format: ChimpTextureFormat::Png,
                ..Default::default()
            },
            None,
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
        let surfaces = chimp_selected_surfaces(&document.texture_previews, &package, None).unwrap();
        assert!(surfaces.is_virtual, "{package} should be a virtual texture");
        assert!(surfaces.is_udim(), "{package} should be a UDIM set");
        // Tiles were cropped in block space, so the surface is still compressed.
        assert_eq!(surfaces.layers[0].mips[0].pixel_format, "PF_DXT1");

        let directory = std::env::temp_dir().join(format!("baboon-dds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("texture.dds");
        write_chimp_texture(
            &world,
            &package,
            &output,
            ChimpTextureExport::default(),
            None,
        )
        .unwrap();

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
        assert!(
            written.contains(&"texture.1001.dds".to_owned()),
            "{written:?}"
        );

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
                ChimpTextureExport {
                    format,
                    ..Default::default()
                },
                None,
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
        let surfaces = chimp_selected_surfaces(&document.texture_previews, &package, None).unwrap();
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
            None,
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
                ChimpTextureExport {
                    format,
                    ..Default::default()
                },
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
        let fmt = match std::env::var("CE_TEXTURE_FORMAT")
            .unwrap_or_default()
            .as_str()
        {
            "png" => ChimpTextureFormat::Png,
            "tif" => ChimpTextureFormat::Tiff,
            _ => ChimpTextureFormat::Dds,
        };
        let split = std::env::var("CE_TEXTURE_SPLIT").unwrap_or_default() != "0";
        println!(
            "{}",
            write_chimp_texture(
                &world,
                &package,
                &directory.join(format!("t.{}", fmt.extension())),
                ChimpTextureExport {
                    format: fmt,
                    split_udim: split
                },
                None
            )
            .unwrap()
        );
        let mut names: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for path in names {
            let bytes = std::fs::read(&path).unwrap();
            let at =
                |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
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
        let surfaces = chimp_selected_surfaces(&document.texture_previews, &package, None).unwrap();
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
                write_chimp_mesh(
                    &world,
                    &package,
                    &output,
                    format,
                    ChimpTextureScope::None,
                    ChimpTextureExport::default(),
                )
                .unwrap();
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

    fn preview(export_index: usize) -> ChimpTexturePreview {
        ChimpTexturePreview {
            export_index,
            preview: BitmapPreviewState::default(),
            surfaces: Err(format!("export {export_index}")),
        }
    }

    /// The export the user picked is the one that gets written. A package with
    /// two Texture2D exports used to export the first whatever was selected,
    /// because the selection was read off a freshly loaded document.
    #[test]
    fn texture_export_writes_the_selected_texture() {
        let previews = [preview(2), preview(5)];
        let pick = |index| selected_texture_preview(&previews, index).map(|p| p.export_index);

        assert_eq!(pick(Some(5)), Some(5), "the selected texture");
        assert_eq!(pick(Some(2)), Some(2));
        assert_eq!(pick(None), Some(2), "nothing selected: the first texture");
        assert_eq!(
            pick(Some(0)),
            Some(2),
            "a non-texture export selected: the first texture"
        );
        assert_eq!(
            selected_texture_preview(&[], Some(5)).map(|p| p.export_index),
            None
        );
    }
}
