//! Getting things out of tags: raw tags, bitmaps and their source images,
//! sounds, geometry, animations, import info, shader and script sources,
//! JSON and reference dumps, one tag at a time or a whole folder or
//! container, and the windows that choose what and where.

use super::*;

pub(super) mod bitmap;
pub(super) mod container_dump;
pub(super) mod geometry;
pub(super) mod import_info;
pub(super) mod json;
pub(super) mod references;
pub(super) mod scripts;
pub(super) mod shader_source;
pub(in crate::app) mod sound_extract;
pub(in crate::app) mod extract_target_window;
pub(in crate::app) use extract_target_window::draw_extract_target_window;
pub(in crate::app) mod container_dump_confirm;
pub(in crate::app) use container_dump_confirm::draw_container_dump_confirm_window;
pub(in crate::app) mod extract;

pub(super) use bitmap::*;
pub(super) use container_dump::*;
pub(super) use geometry::*;
pub(super) use import_info::*;
pub(super) use json::*;
pub(super) use references::*;
pub(super) use scripts::*;
pub(super) use shader_source::*;
pub(in crate::app) use sound_extract::*;
pub(in crate::app) use extract::*;

/// What a batch export over many tags managed.
pub(super) struct BatchExport {
    /// Files written, over every tag.
    pub(super) written: usize,
    /// Tags that wrote something.
    pub(super) tags: usize,
    /// `"path: error"` for each tag that failed.
    pub(super) failures: Vec<String>,
}

/// Run `write` over `entries`, collecting what failed rather than stopping
/// at it. `write` returns how many files the tag produced.
pub(super) fn export_each<'e>(
    entries: impl IntoIterator<Item = &'e TagEntry>,
    mut write: impl FnMut(&TagEntry) -> anyhow::Result<usize>,
) -> BatchExport {
    let mut batch = BatchExport {
        written: 0,
        tags: 0,
        failures: Vec::new(),
    };
    for entry in entries {
        match write(entry) {
            Ok(count) => {
                batch.written += count;
                batch.tags += 1;
            }
            Err(error) => batch
                .failures
                .push(format!("{}: {error:#}", entry.display_path)),
        }
    }
    batch
}

impl BatchExport {
    /// The status line: an error when nothing was written (`all_failed` with
    /// the failures, or `none_found`), otherwise `message(written, tags)` and
    /// which tags failed. It used to report only how many had.
    pub(super) fn finish(
        self,
        none_found: &str,
        all_failed: &str,
        message: impl FnOnce(usize, usize) -> String,
    ) -> anyhow::Result<String> {
        if self.written == 0 && !self.failures.is_empty() {
            anyhow::bail!("{all_failed}: {}", self.failures.join("; "));
        }
        if self.written == 0 {
            anyhow::bail!("{none_found}");
        }
        let mut line = message(self.written, self.tags);
        if !self.failures.is_empty() {
            line.push_str(&failure_summary(&self.failures));
        }
        Ok(line)
    }
}

/// `"; 5 failed: a: …; b: …; c: …; and 2 more"` — enough to act on without
/// swamping a status line.
fn failure_summary(failures: &[String]) -> String {
    const SHOWN: usize = 3;
    let mut summary = format!(
        "; {} failed: {}",
        failures.len(),
        failures[..failures.len().min(SHOWN)].join("; ")
    );
    if failures.len() > SHOWN {
        summary.push_str(&format!("; and {} more", failures.len() - SHOWN));
    }
    summary
}

pub(super) fn extract_raw_tag(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    tag.write(output)?;
    Ok(format!("Extracted raw tag {}", output.display()))
}

#[cfg(test)]
mod batch_tests;

#[cfg(test)]
mod bitmap_source_extract_tests;
pub(in crate::app) mod state;
pub(in crate::app) use state::*;

/// Export: the container dump and its confirmation, the extract target window,
/// and a sound extraction waiting to start.
pub(in crate::app) struct ExportFeature {
    /// Mandatory confirmation for a bulk extraction of every shipped tag.
    pub(in crate::app) container_dump_confirm: Option<ContainerDumpConfirm>,
    /// The one bulk container extraction allowed to run at a time.
    pub(in crate::app) container_dump_job: Option<ContainerDumpJob>,
    /// The Extract Geometry / Extract Animations target window, if open.
    pub(in crate::app) extract_target: Option<ExtractTargetPrompt>,
    /// Pending sound-extraction batch (decode + write), drained by the audio layer.
    pub(in crate::app) pending_sound_extract: Option<ExtractRequest>,
}

/// What export can be asked to do.
pub(in crate::app) enum ExportCommand {
    /// Extract `scope` of the containers of `kit` into `output`. The
    /// extraction reads the active kit's source, so the handler returns to
    /// that workspace first and drops the run if it has closed.
    StartContainerDump {
        kit: KitId,
        output: PathBuf,
        scope: ContainerDumpScope,
    },
    /// Extract the geometry or animations of the tag at `key` for `target`'s
    /// tools, starting with the folder picker.
    Extract {
        kind: ExtractKind,
        key: String,
        target: blam_tags::game::Game,
    },
}

impl Baboon {
    pub(in crate::app) fn apply_export_command(&mut self, command: ExportCommand, ctx: &egui::Context) {
        match command {
            ExportCommand::StartContainerDump { kit, output, scope } => {
                if self.focus_navigation_kit(kit) {
                    self.start_container_dump(kit, output, scope, ctx.clone());
                }
            }
            ExportCommand::Extract { kind, key, target } => match kind {
                ExtractKind::Geometry => self.begin_extract_geometry(key, target, ctx.clone()),
                ExtractKind::Animation => self.begin_extract_animation(key, target, ctx.clone()),
            },
        }
    }
}
