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
pub(in crate::app) mod container_dump_confirm;
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
