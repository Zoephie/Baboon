//! Tag, bitmap, geometry, animation, and source-code export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(super) mod bitmap;
pub(super) mod container_dump;
pub(super) mod geometry;
pub(super) mod import_info;
pub(super) mod json;
pub(super) mod references;
pub(super) mod scripts;
pub(super) mod shader_source;

pub(super) use bitmap::*;
pub(super) use container_dump::*;
pub(super) use geometry::*;
pub(super) use import_info::*;
pub(super) use json::*;
pub(super) use references::*;
pub(super) use scripts::*;
pub(super) use shader_source::*;

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
mod batch_tests {
    use super::*;

    fn batch(written: usize, failures: usize) -> BatchExport {
        BatchExport {
            written,
            tags: written,
            failures: (0..failures)
                .map(|index| format!("objects/t{index}.bitmap: no images"))
                .collect(),
        }
    }

    fn finish(batch: BatchExport) -> anyhow::Result<String> {
        batch.finish("none found", "all failed", |written, tags| {
            format!("Wrote {written} from {tags}")
        })
    }

    /// A partly failed export says which tags failed, not only how many.
    #[test]
    fn a_batch_export_names_what_failed() {
        assert_eq!(finish(batch(4, 0)).unwrap(), "Wrote 4 from 4");
        assert_eq!(
            finish(batch(4, 2)).unwrap(),
            "Wrote 4 from 4; 2 failed: objects/t0.bitmap: no images; \
             objects/t1.bitmap: no images"
        );
        let many = finish(batch(1, 5)).unwrap();
        assert!(many.contains("objects/t2.bitmap") && !many.contains("objects/t3.bitmap"));
        assert!(many.ends_with("; and 2 more"), "{many}");

        let all = finish(batch(0, 2)).unwrap_err().to_string();
        assert!(all.starts_with("all failed: objects/t0.bitmap"), "{all}");
        assert_eq!(finish(batch(0, 0)).unwrap_err().to_string(), "none found");
    }
}
