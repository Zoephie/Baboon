//! Bulk extraction of a Campaign Evolved container set's shipped tags to disk.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

use std::sync::atomic::AtomicUsize;

use anyhow::Context as _;

/// What a finished (or cancelled) bulk extraction actually did.
///
/// Every count is reported rather than folded into a pass/fail, because the
/// interesting answer is usually the one in the middle: a run that wrote most of
/// the game and skipped a hundred mod-only tags is a success, and saying only
/// "done" hides the hundred.
pub(in crate::app) struct ContainerDumpReport {
    pub(in crate::app) written: usize,
    /// Tags no shipped pack carries — a mod added them, so there is no game copy
    /// to extract.
    pub(in crate::app) skipped: usize,
    pub(in crate::app) failed: usize,
    pub(in crate::app) bytes: u64,
    pub(in crate::app) cancelled: bool,
    /// The first few failures, in full. The rest are only counted: a report that
    /// lists forty thousand broken tags is a report nobody reads.
    pub(in crate::app) failures: Vec<String>,
}

/// How many individual failures the report quotes before it just counts them.
const REPORTED_FAILURES: usize = 20;

/// Write every tag the game's own packs ship into `output`, laid out like an
/// editing kit.
///
/// Deliberately reads the *shipped* payload rather than what [`read_entry`]
/// would load: a mod mounted over the game answers "what does the game run?",
/// and this question is "what does the game ship?". Tags only a mod provides
/// have no shipped copy and are counted as skipped rather than failed.
///
/// The `.ubulk` payload is already a byte-complete self-describing tag, so the
/// bytes go to disk untouched — no parse, no re-serialize, and nothing that
/// could lose a field Baboon does not model.
pub(in crate::app) fn dump_shipped_container_tags(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> anyhow::Result<ContainerDumpReport> {
    let targets: Vec<&TagEntry> = entries
        .iter()
        .filter(|entry| matches!(entry.location, TagEntryLocation::Container { .. }))
        .collect();
    let total = targets.len();
    if total == 0 {
        anyhow::bail!("this workspace has no container tags to extract");
    }
    fs::create_dir_all(output)
        .with_context(|| format!("failed to create {}", output.display()))?;

    // Reads decompress a container chunk and writes hit the filesystem, so this
    // is worth spreading — but only so far. Past a handful of threads the disk
    // is the limit and they only contend, the same shape the level export
    // measured at four.
    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8)
        .min(total);
    let chunk_size = total.div_ceil(worker_count).max(1);

    let done = AtomicUsize::new(0);
    let written = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);

    progress(0, total);

    let mut failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .chunks(chunk_size)
            .map(|chunk| {
                let (done, written, skipped, bytes) = (&done, &written, &skipped, &bytes);
                scope.spawn(move || {
                    let mut failures = Vec::new();
                    for entry in chunk {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        match write_shipped_entry(source, entry, output) {
                            Ok(Some(size)) => {
                                written.fetch_add(1, Ordering::Relaxed);
                                bytes.fetch_add(size, Ordering::Relaxed);
                            }
                            Ok(None) => {
                                skipped.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(error) => {
                                failures.push(format!("{}: {error}", entry.display_path))
                            }
                        }
                        // One message per tag would be tens of thousands of
                        // repaints for a bar that moves less than a pixel.
                        let now = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if now == total || now % 256 == 0 {
                            progress(now, total);
                        }
                    }
                    failures
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .flatten()
            .collect()
    });

    let cancelled = cancel.load(Ordering::Relaxed);
    let written = written.load(Ordering::Relaxed);
    let skipped = skipped.load(Ordering::Relaxed);
    let failed = failures.len();
    progress(done.load(Ordering::Relaxed), total);

    // Only a run that got nowhere is an error. A partial one has files on disk
    // and a tally worth reading, and reporting that as a failure would throw
    // both away.
    if written == 0 && failed > 0 {
        anyhow::bail!(
            "failed to extract any tag: {}",
            failures
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    failures.truncate(REPORTED_FAILURES);
    Ok(ContainerDumpReport {
        written,
        skipped,
        failed,
        bytes: bytes.load(Ordering::Relaxed),
        cancelled,
        failures,
    })
}

/// `Ok(None)` when no shipped pack carries this tag; otherwise the byte count.
fn write_shipped_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<Option<u64>> {
    let Some(bytes) = crate::source::read_shipped_entry_bytes(source, entry)? else {
        return Ok(None);
    };
    let relative = safe_relative_path(&entry.display_path)
        .with_context(|| format!("no usable file name for {}", entry.display_path))?;
    let path = output.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, &bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(Some(bytes.len() as u64))
}

/// A display path turned into a relative path safe to join onto an output root.
///
/// Display paths come from container entries rather than from a filesystem, so
/// nothing has ever checked them against one. Two things bite on Windows: a
/// group with no friendly name falls back to the space-padded FOURCC, which
/// makes a file name ending in a space that cannot be created; and a `..` in a
/// path joined onto the user's chosen folder writes outside it.
fn safe_relative_path(display_path: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    let mut any = false;
    for component in display_path.split(['/', '\\']) {
        let component = component.trim().trim_end_matches([' ', '.']);
        if component.is_empty() || component == ".." || component.contains(':') {
            continue;
        }
        path.push(component);
        any = true;
    }
    any.then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_the_display_path() {
        assert_eq!(
            safe_relative_path("levels/solo/c10/c10.scenario"),
            Some(PathBuf::from("levels").join("solo").join("c10").join("c10.scenario"))
        );
    }

    #[test]
    fn trims_a_padded_fourcc_extension() {
        // `format_group_tag` pads a three-letter group to four, and a file name
        // ending in a space cannot be created on Windows.
        assert_eq!(
            safe_relative_path("objects/foo.mat "),
            Some(PathBuf::from("objects").join("foo.mat"))
        );
    }

    #[test]
    fn refuses_to_escape_the_output_root() {
        assert_eq!(
            safe_relative_path("../../windows/system32/foo.scenario"),
            Some(PathBuf::from("windows").join("system32").join("foo.scenario"))
        );
        assert_eq!(
            safe_relative_path("c:/absolute/foo.scenario"),
            Some(PathBuf::from("absolute").join("foo.scenario"))
        );
    }

    #[test]
    fn rejects_a_path_with_nothing_left() {
        assert_eq!(safe_relative_path("../.."), None);
        assert_eq!(safe_relative_path(""), None);
    }
}
