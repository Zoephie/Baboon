//! Hearing about changes under a loose kit's folder as they happen, through
//! the operating system's file notifications (`notify`). A watch collects
//! the files a burst of changes touched and when it last did; callers wait
//! for the burst to go quiet (Tool and Explorer write in bursts), then take
//! the files and look at the ones they care about themselves.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

/// How long changes must stop before a burst of them counts as settled.
pub(crate) const WATCH_QUIET: Duration = Duration::from_millis(300);

/// Git's own records, which a repository kept inside the tags folder
/// rewrites on every `git status`. Watching them would make anything that
/// reads Git on a change (Git Review) trigger itself forever.
fn inside_git(path: &Path) -> bool {
    path.components().any(|component| component.as_os_str() == ".git")
}

/// One burst of changes, as [`FolderWatch::settled`] hands it over.
#[derive(Debug, Default)]
pub(crate) struct Burst {
    /// Every path the OS named, each once.
    pub(crate) paths: Vec<PathBuf>,
    /// The OS dropped events (its buffer overflowed), so `paths` may be
    /// missing some: anything might have changed.
    pub(crate) incomplete: bool,
}

/// What the watcher's thread collects for [`FolderWatch::settled`].
#[derive(Default)]
struct Pending {
    last_change: Option<Instant>,
    burst: Burst,
}

/// A recursive watch on one folder. Dropping it stops watching.
pub(crate) struct FolderWatch {
    _watcher: notify::RecommendedWatcher,
    pending: Arc<Mutex<Pending>>,
}

impl FolderWatch {
    /// Watch `root` and everything under it. `wake` runs on the watcher's
    /// thread after each change, with how long until the burst could settle,
    /// so the caller can come back and call [`FolderWatch::settled`].
    pub(crate) fn start(root: &Path, wake: impl Fn(Duration) + Send + 'static) -> notify::Result<Self> {
        let pending = Arc::new(Mutex::new(Pending::default()));
        let noted = Arc::clone(&pending);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Reads are not changes. An error (the OS dropped events when its
            // buffer overflowed) may have hidden one, so it counts as one.
            if let Ok(event) = &event
                && (matches!(event.kind, EventKind::Access(_)) || (!event.paths.is_empty() && event.paths.iter().all(|path| inside_git(path))))
            {
                return;
            }
            if let Ok(mut pending) = noted.lock() {
                pending.last_change = Some(Instant::now());
                match event {
                    Ok(event) => {
                        for path in event.paths {
                            if !inside_git(&path) && !pending.burst.paths.contains(&path) {
                                pending.burst.paths.push(path);
                            }
                        }
                    }
                    Err(_) => pending.burst.incomplete = true,
                }
            }
            wake(WATCH_QUIET + Duration::from_millis(50));
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher, pending })
    }

    /// Once per burst, when there were changes and none for `quiet` since:
    /// what changed.
    pub(crate) fn settled(&self, quiet: Duration) -> Option<Burst> {
        let mut pending = self.pending.lock().ok()?;
        match pending.last_change {
            Some(at) if at.elapsed() >= quiet => {
                pending.last_change = None;
                Some(std::mem::take(&mut pending.burst))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(watch: &FolderWatch, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while watch.settled(WATCH_QUIET).is_none() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_change_under_the_folder_settles_once_after_the_quiet_period() {
        let root = std::env::temp_dir().join(format!("baboon-folder-watch-{}", std::process::id()));
        let nested = root.join("objects").join("vehicles");
        std::fs::create_dir_all(&nested).unwrap();
        let watch = FolderWatch::start(&root, |_| {}).unwrap();
        assert!(watch.settled(WATCH_QUIET).is_none(), "nothing changed yet");

        std::fs::write(nested.join("warthog.model"), b"one").unwrap();
        std::fs::write(nested.join("warthog.model"), b"two").unwrap();
        wait_for(&watch, "the write burst to settle");
        assert!(watch.settled(WATCH_QUIET).is_none(), "a burst settles once");

        std::fs::remove_file(nested.join("warthog.model")).unwrap();
        wait_for(&watch, "the deletion to settle");

        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn changes_inside_git_are_not_changes() {
        let root = std::env::temp_dir().join(format!("baboon-folder-watch-git-{}", std::process::id()));
        let git = root.join(".git").join("refs");
        std::fs::create_dir_all(&git).unwrap();
        let watch = FolderWatch::start(&root, |_| {}).unwrap();

        std::fs::write(root.join(".git").join("index"), b"index").unwrap();
        std::fs::write(git.join("main"), b"ref").unwrap();
        std::thread::sleep(WATCH_QUIET * 3);
        assert!(watch.settled(WATCH_QUIET).is_none(), "Git's own records are ignored");

        std::fs::write(root.join("warthog.model"), b"tag").unwrap();
        wait_for(&watch, "a tag beside the repository");

        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
    }
}
