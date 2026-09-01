//! The Blam! pane's import workflow: routing an asset's tool source folders
//! through the blam-tags importers off the UI thread, and filing the built
//! tags into the kit.
//! It owns application actions and async coordination; presentation lives in `ui/blam.rs`, folder detection in `app/blam.rs`, and the importers in `blam-tags`.

use super::*;

/// Everything the worker needs, snapshotted on the UI thread so the job never
/// reads `Baboon` state.
struct BlamImportJob {
    /// The asset's folder under `data`, absolute.
    data_dir: PathBuf,
    /// The kit's `tags` folder, absolute.
    tags_root: PathBuf,
    /// The asset folder relative to `data`, forward slashes.
    asset_rel: String,
    /// Last component of `asset_rel`: what the built tags are named.
    asset_name: String,
    /// `definitions/<game>`, where the tag schemas live.
    schema_dir: PathBuf,
    render: bool,
    prt: bool,
    collision: bool,
    physics: bool,
    structure: bool,
}

impl Baboon {
    /// Start the Blam! import for one kit's ticked pipelines. Runs on a worker
    /// thread; progress and the result come back through [`WorkerMessage`].
    pub(in crate::app) fn begin_blam_import(&mut self, kit_index: usize, ctx: egui::Context) {
        if self.kits[kit_index].blam.running {
            return;
        }
        let asset_rel = self.kits[kit_index]
            .blam
            .asset_path
            .trim()
            .replace('\\', "/")
            .trim_matches('/')
            .to_owned();
        if asset_rel.is_empty() {
            self.kits[kit_index].blam.status = "Pick an asset data folder first".to_owned();
            return;
        }
        let Some(kit_root) = self.editing_kit_root_for(kit_index) else {
            self.kits[kit_index].blam.status =
                "This workspace has no loose editing kit to import into".to_owned();
            return;
        };
        let Some(tags_root) = self.loaded_tags_root_for(kit_index) else {
            self.kits[kit_index].blam.status =
                "This workspace has no loose tags folder to import into".to_owned();
            return;
        };
        let Some(game) = self.kits[kit_index]
            .source
            .as_ref()
            .and_then(|source| source.game.clone())
        else {
            self.kits[kit_index].blam.status =
                "This workspace's game is unknown, so no schemas can be chosen".to_owned();
            return;
        };
        let asset_name = asset_rel
            .rsplit('/')
            .next()
            .unwrap_or(asset_rel.as_str())
            .to_owned();
        let blam = &self.kits[kit_index].blam;
        let job = BlamImportJob {
            data_dir: kit_root.join("data").join(&asset_rel),
            tags_root,
            asset_rel,
            asset_name,
            schema_dir: locate_definitions_root().join(&game),
            render: blam.import_render,
            prt: blam.import_prt,
            collision: blam.import_collision,
            physics: blam.import_physics,
            structure: blam.import_structure,
        };
        let stamp = KitStamp {
            kit: self.kits[kit_index].id,
            generation: self.kits[kit_index].generation,
        };
        let tx = self.tx.clone();
        let mut ticked = Vec::new();
        if job.render {
            ticked.push(if job.prt { "render + PRT" } else { "render" });
        }
        if job.collision {
            ticked.push("collision");
        }
        if job.physics {
            ticked.push("physics");
        }
        if job.structure {
            ticked.push("structure");
        }
        let blam = &mut self.kits[kit_index].blam;
        blam.running = true;
        blam.status = "Importing…".to_owned();
        blam.log.clear();
        blam.push_log(
            BlamLogKind::Info,
            format!("Importing {} — {}", job.asset_rel, ticked.join(", ")),
        );
        thread::spawn(move || {
            let (outcomes, created) = run_blam_import(&job, &tx, stamp, &ctx);
            let _ = tx.send(WorkerMessage::BlamImportFinished {
                stamp,
                outcomes,
                created,
            });
            ctx.request_repaint();
        });
    }

    /// Returns true when the message was stale (its kit closed or reloaded).
    pub(super) fn handle_blam_import_progress(
        &mut self,
        stamp: KitStamp,
        kind: BlamLogKind,
        message: String,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        let blam = &mut self.kits[kit_index].blam;
        if kind == BlamLogKind::Info {
            blam.status = message.clone();
        }
        blam.push_log(kind, message);
        false
    }

    /// Returns true when the message was stale (its kit closed or reloaded).
    pub(super) fn handle_blam_import_finished(
        &mut self,
        stamp: KitStamp,
        outcomes: Vec<(String, Result<String, String>)>,
        created: Vec<(TagEntry, TagFile)>,
    ) -> bool {
        let Some(kit_index) = self.resolve_stamp(stamp) else {
            return true;
        };
        self.kits[kit_index].blam.running = false;
        let created_count = created.len();
        let folder_seeds = self.kits[kit_index].folder_seeds();
        for (entry, tag) in created {
            let key = entry.key.clone();
            if let Some(source) = self.kits[kit_index].source.as_mut() {
                register_created_tag_in_source(source, entry, &folder_seeds);
            }
            // A re-imported tag that is open would keep showing — and could
            // save — the bytes from before the import; replace the document
            // with what is now on disk. Any unsaved edits to it are already
            // overwritten on disk, which is what Import was asked to do.
            if self.kits[kit_index].parsed_tags.contains_key(&key) {
                self.kits[kit_index]
                    .parsed_tags
                    .insert(key, TagDocument::clean(tag));
            }
        }
        if created_count > 0 {
            // The source's content changed: stale caches and in-flight work
            // keyed to the old revision must not answer for the new one.
            self.kits[kit_index].generation = self.kits[kit_index].generation.wrapping_add(1);
            self.kits[kit_index].field_index.invalidate();
        }
        let mut failures = 0usize;
        for (label, result) in &outcomes {
            let blam = &mut self.kits[kit_index].blam;
            match result {
                Ok(summary) => blam.push_log(BlamLogKind::Good, format!("{label}: {summary}")),
                Err(error) => {
                    failures += 1;
                    blam.push_log(BlamLogKind::Error, format!("{label} FAILED: {error}"));
                }
            }
        }
        let tally = if outcomes.is_empty() {
            "Nothing was ticked, so nothing ran".to_owned()
        } else if failures == 0 {
            format!("Imported {created_count} tag(s)")
        } else {
            format!("Imported {created_count} tag(s), {failures} pipeline(s) failed")
        };
        let blam = &mut self.kits[kit_index].blam;
        blam.push_log(
            if failures == 0 && created_count > 0 {
                BlamLogKind::Good
            } else {
                BlamLogKind::Info
            },
            tally.clone(),
        );
        blam.status = tally.clone();
        if kit_index == self.active {
            self.status = format!("Blam! — {tally}");
        }
        false
    }
}

fn progress(tx: &Sender<WorkerMessage>, stamp: KitStamp, ctx: &egui::Context, message: String) {
    let _ = tx.send(WorkerMessage::BlamImportProgress {
        stamp,
        kind: BlamLogKind::Info,
        message,
    });
    ctx.request_repaint();
}

/// Append `({seconds}s)` so every finished pipeline says what it cost.
fn timed(summary: String, started: std::time::Instant) -> String {
    format!("{summary} ({:.1}s)", started.elapsed().as_secs_f32())
}

/// Run every ticked pipeline in kit order. One pipeline failing does not stop
/// the others: each reports its own outcome.
fn run_blam_import(
    job: &BlamImportJob,
    tx: &Sender<WorkerMessage>,
    stamp: KitStamp,
    ctx: &egui::Context,
) -> (Vec<(String, Result<String, String>)>, Vec<(TagEntry, TagFile)>) {
    let mut outcomes = Vec::new();
    let mut created = Vec::new();

    if job.render {
        let label = if job.prt { "render + PRT" } else { "render" };
        let started = std::time::Instant::now();
        let result = import_jms_pipeline(job, tx, stamp, ctx, "render", "render_model", |jms, schema| {
            let mut options = blam_tags::render_import::RenderOptions::default();
            if !job.prt {
                options.prt_samples = None;
            }
            if let Some(samples) = options.prt_samples {
                progress(
                    tx,
                    stamp,
                    ctx,
                    format!("render_model: building, solving ambient PRT at {samples} rays a vertex…"),
                );
            } else {
                progress(tx, stamp, ctx, "render_model: building (no PRT)…".to_owned());
            }
            let (tag, report) =
                blam_tags::render_import::render_model_from_jms(jms, schema, &options)
                    .map_err(|error| error.to_string())?;
            Ok((
                tag,
                format!(
                    "{} meshes, {} regions, {} materials, {} nodes",
                    report.meshes, report.regions, report.materials, report.nodes
                ),
            ))
        })
        .map(|(summary, entry, tag)| {
            created.push((entry, tag));
            timed(summary, started)
        });
        outcomes.push((label.to_owned(), result));
    }
    if job.collision {
        let started = std::time::Instant::now();
        let result = import_jms_pipeline(
            job,
            tx,
            stamp,
            ctx,
            "collision",
            "collision_model",
            |jms, schema| {
                progress(
                    tx,
                    stamp,
                    ctx,
                    "collision_model: building the collision BSP…".to_owned(),
                );
                let (tag, report) = blam_tags::collision_import::collision_model_from_jms(
                    jms,
                    schema,
                    &blam_tags::collision_import::CollisionOptions::default(),
                )
                .map_err(|error| error.to_string())?;
                Ok((
                    tag,
                    format!(
                        "{} regions, {} materials, {} surfaces",
                        report.regions, report.materials, report.surfaces
                    ),
                ))
            },
        )
        .map(|(summary, entry, tag)| {
            created.push((entry, tag));
            timed(summary, started)
        });
        outcomes.push(("collision".to_owned(), result));
    }
    if job.physics {
        let started = std::time::Instant::now();
        let result = import_jms_pipeline(
            job,
            tx,
            stamp,
            ctx,
            "physics",
            "physics_model",
            |jms, schema| {
                progress(
                    tx,
                    stamp,
                    ctx,
                    "physics_model: building rigid bodies and shapes…".to_owned(),
                );
                let (tag, report) = blam_tags::physics_import::physics_model_from_jms(
                    jms,
                    schema,
                    &blam_tags::physics_import::PhysicsOptions::default(),
                )
                .map_err(|error| error.to_string())?;
                Ok((
                    tag,
                    format!(
                        "{} rigid bodies, {} materials, {} regions",
                        report.rigid_bodies, report.materials, report.regions
                    ),
                ))
            },
        )
        .map(|(summary, entry, tag)| {
            created.push((entry, tag));
            timed(summary, started)
        });
        outcomes.push(("physics".to_owned(), result));
    }
    if job.structure {
        let started = std::time::Instant::now();
        let result = import_structures(job, tx, stamp, ctx, &mut created)
            .map(|summary| timed(summary, started));
        outcomes.push(("structure".to_owned(), result));
    }
    (outcomes, created)
}

/// Pick the JMS the kit's own filing implies: the folder's only `.jms`, or
/// among several the one named after the asset.
fn jms_source(dir: &Path, asset_name: &str) -> Result<PathBuf, String> {
    let mut files = source_files(dir, "jms")?;
    match files.len() {
        0 => Err(format!("no .jms file in {}", dir.display())),
        1 => Ok(files.remove(0)),
        n => files
            .iter()
            .find(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(asset_name))
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{} holds {n} .jms files and none is named {asset_name}.jms — \
                     merging several JMS files is not supported yet",
                    dir.display()
                )
            }),
    }
}

/// Every file in `dir` with `extension`, sorted so runs are deterministic.
fn source_files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        })
        .collect();
    files.sort();
    Ok(files)
}

/// One JMS pipeline: find the source, parse it, build the tag through
/// `build`, verify the bytes parse back, and file it as
/// `tags\<asset>\<asset_name>.<group>`.
fn import_jms_pipeline(
    job: &BlamImportJob,
    tx: &Sender<WorkerMessage>,
    stamp: KitStamp,
    ctx: &egui::Context,
    folder: &str,
    group: &str,
    build: impl FnOnce(&blam_tags::jms::JmsFile, &Path) -> Result<(TagFile, String), String>,
) -> Result<(String, TagEntry, TagFile), String> {
    let dir = job.data_dir.join(folder);
    if !dir.is_dir() {
        return Err(format!("no {folder} folder in {}", job.data_dir.display()));
    }
    let source = jms_source(&dir, &job.asset_name)?;
    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    progress(tx, stamp, ctx, format!("{group}: parsing {file_name}…"));
    let schema = schema_path(&job.schema_dir, group)?;
    let text = std::fs::read_to_string(&source)
        .map_err(|error| format!("could not read {}: {error}", source.display()))?;
    let (jms, version) =
        blam_tags::jms::JmsFile::parse(&text).map_err(|error| format!("{file_name}: {error}"))?;
    progress(
        tx,
        stamp,
        ctx,
        format!(
            "{group}: {file_name} is JMS v{version}, {} vertices, {} triangles",
            jms.vertices.len(),
            jms.triangles.len()
        ),
    );
    let (tag, summary) = build(&jms, &schema).map_err(|error| format!("{file_name}: {error}"))?;
    progress(
        tx,
        stamp,
        ctx,
        format!("{group}: verifying and writing {}/{}.{group}…", job.asset_rel, job.asset_name),
    );
    let (entry, tag) = file_tag(job, tag, &job.asset_name, group)?;
    Ok((summary, entry, tag))
}

/// The structure pipeline: every `.ass` in the folder becomes its own
/// `scenario_structure_bsp` named after the file, since a level can carry
/// several BSPs.
fn import_structures(
    job: &BlamImportJob,
    tx: &Sender<WorkerMessage>,
    stamp: KitStamp,
    ctx: &egui::Context,
    created: &mut Vec<(TagEntry, TagFile)>,
) -> Result<String, String> {
    let dir = job.data_dir.join("structure");
    if !dir.is_dir() {
        return Err(format!("no structure folder in {}", job.data_dir.display()));
    }
    let sources = source_files(&dir, "ass")?;
    if sources.is_empty() {
        return Err(format!("no .ass file in {}", dir.display()));
    }
    let schema = schema_path(&job.schema_dir, "scenario_structure_bsp")?;
    let mut summaries = Vec::new();
    for source in sources {
        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(job.asset_name.as_str())
            .to_owned();
        progress(
            tx,
            stamp,
            ctx,
            format!("scenario_structure_bsp: parsing {file_name}…"),
        );
        let text = std::fs::read_to_string(&source)
            .map_err(|error| format!("could not read {}: {error}", source.display()))?;
        let (ass, version) = blam_tags::ass_parse::parse(&text)
            .map_err(|error| format!("{file_name}: {error}"))?;
        progress(
            tx,
            stamp,
            ctx,
            format!("scenario_structure_bsp: {file_name} is ASS v{version}, building the sealed world…"),
        );
        let (tag, report) = blam_tags::sbsp_import::structure_bsp_from_ass(&ass, &schema)
            .map_err(|error| format!("{file_name}: {error}"))?;
        progress(
            tx,
            stamp,
            ctx,
            format!(
                "scenario_structure_bsp: verifying and writing {}/{stem}.scenario_structure_bsp…",
                job.asset_rel
            ),
        );
        let (entry, tag) = file_tag(job, tag, &stem, "scenario_structure_bsp")?;
        summaries.push(format!(
            "{stem}: {} vertices, {} triangles, {} portals",
            report.vertices, report.triangles, report.portals_written
        ));
        created.push((entry, tag));
    }
    Ok(summaries.join(", "))
}

fn schema_path(schema_dir: &Path, group: &str) -> Result<PathBuf, String> {
    let schema = schema_dir.join(format!("{group}.json"));
    if !schema.is_file() {
        return Err(format!("schema not found: {}", schema.display()));
    }
    Ok(schema)
}

/// Serialise, prove the bytes parse back, and write the tag where the kit
/// files it. The parsed-back tag is what gets registered, so the document in
/// the editor is exactly what is on disk.
fn file_tag(
    job: &BlamImportJob,
    tag: TagFile,
    stem: &str,
    group: &str,
) -> Result<(TagEntry, TagFile), String> {
    let bytes = tag
        .write_to_bytes()
        .map_err(|error| format!("the built {group} would not serialise: {error}"))?;
    let reread = TagFile::read_from_bytes(&bytes)
        .map_err(|error| format!("the built {group} would not parse back: {error}"))?;
    let display_path = format!("{}/{stem}.{group}", job.asset_rel);
    let output = job.tags_root.join(&job.asset_rel).join(format!("{stem}.{group}"));
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(&output, &bytes)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    let entry = TagEntry {
        key: display_path.clone(),
        display_path,
        group_tag: reread.header.group_tag,
        group_name: Some(group.to_owned()),
        location: TagEntryLocation::LooseFile(output),
    };
    Ok((entry, reread))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Root of a real H3EK install, via `BLAM_TEST_H3EK_ROOT`. Skips (like the
    /// other kit-dependent tests) when absent.
    fn h3ek_root() -> Option<PathBuf> {
        let root = PathBuf::from(std::env::var("BLAM_TEST_H3EK_ROOT").ok()?);
        root.join("data").is_dir().then_some(root)
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir =
            std::env::temp_dir().join(format!("baboon-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_job(job: &BlamImportJob) -> (Vec<(String, Result<String, String>)>, Vec<(TagEntry, TagFile)>) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let stamp = KitStamp {
            kit: KitId(0),
            generation: 0,
        };
        run_blam_import(job, &tx, stamp, &egui::Context::default())
    }

    fn assert_written_and_rereadable(created: &[(TagEntry, TagFile)]) {
        for (entry, _tag) in created {
            let TagEntryLocation::LooseFile(path) = &entry.location else {
                panic!("an imported tag must be a loose file");
            };
            assert!(path.is_file(), "{} was not written", path.display());
            TagFile::read(path)
                .unwrap_or_else(|error| panic!("{} does not re-read: {error}", path.display()));
        }
    }

    /// The full worker pass over a stock asset: render, collision and physics
    /// from the kit's own source files, written into a scratch tags root so
    /// the real kit is untouched. PRT stays off here for speed — the solver
    /// itself is proven in blam-tags' own corpus tests.
    #[test]
    fn a_stock_asset_imports_all_three_model_tags() {
        let Some(root) = h3ek_root() else {
            eprintln!("skipping: set BLAM_TEST_H3EK_ROOT to a real H3EK install");
            return;
        };
        let asset = "objects/vehicles/ghost_aa";
        let data_dir = root.join("data").join(asset);
        if !data_dir.join("render").is_dir() {
            eprintln!("skipping: no {asset} sources in this kit's data");
            return;
        }
        let tags_root = scratch_dir("blam-import");
        let job = BlamImportJob {
            data_dir,
            tags_root: tags_root.clone(),
            asset_rel: asset.to_owned(),
            asset_name: "ghost_aa".to_owned(),
            schema_dir: locate_definitions_root().join("halo3_mcc"),
            render: true,
            prt: false,
            collision: true,
            physics: true,
            structure: false,
        };
        let (outcomes, created) = run_job(&job);
        for (label, result) in &outcomes {
            assert!(result.is_ok(), "{label} failed: {result:?}");
        }
        assert_eq!(created.len(), 3, "render, collision and physics tags");
        assert_written_and_rereadable(&created);
        assert!(
            tags_root
                .join(asset)
                .join("ghost_aa.render_model")
                .is_file(),
            "the render_model must be filed where the kit files it",
        );
        std::fs::remove_dir_all(&tags_root).unwrap();
    }

    /// The structure pipeline routes every `.ass` in the folder to its own
    /// BSP, named after the file. One small shipped level file, staged into a
    /// scratch data folder so only that one imports.
    #[test]
    fn a_shipped_level_ass_becomes_a_structure_bsp() {
        let Some(root) = h3ek_root() else {
            eprintln!("skipping: set BLAM_TEST_H3EK_ROOT to a real H3EK install");
            return;
        };
        let source = root.join("data/levels/solo/070_waste/structure/070_bsp_011.ass");
        if !source.is_file() {
            eprintln!("skipping: no 070_bsp_011.ass in this kit's data");
            return;
        }
        let scratch = scratch_dir("blam-sbsp");
        let data_dir = scratch.join("data/levels/test_level");
        std::fs::create_dir_all(data_dir.join("structure")).unwrap();
        std::fs::copy(&source, data_dir.join("structure/070_bsp_011.ass")).unwrap();
        let tags_root = scratch.join("tags");
        let job = BlamImportJob {
            data_dir,
            tags_root: tags_root.clone(),
            asset_rel: "levels/test_level".to_owned(),
            asset_name: "test_level".to_owned(),
            schema_dir: locate_definitions_root().join("halo3_mcc"),
            render: false,
            prt: false,
            collision: false,
            physics: false,
            structure: true,
        };
        let (outcomes, created) = run_job(&job);
        for (label, result) in &outcomes {
            assert!(result.is_ok(), "{label} failed: {result:?}");
        }
        assert_eq!(created.len(), 1);
        assert!(
            tags_root
                .join("levels/test_level/070_bsp_011.scenario_structure_bsp")
                .is_file(),
            "the BSP must be named after its .ass, not the level folder",
        );
        assert_written_and_rereadable(&created);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
