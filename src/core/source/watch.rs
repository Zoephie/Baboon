//! Hearing about changes under a loose kit's folder as they happen, through
//! the operating system's file notifications (`notify`). A watch collects
//! the files a burst of changes touched and when it last did; callers wait
//! for the burst to go quiet (Tool and Explorer write in bursts), then take
//! the files and look at the ones they care about themselves.

use std::collections::HashSet;
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
    /// missing some: anything might have changed. Windows reports this as an
    /// error; macOS and Linux as an event asking for a rescan.
    pub(crate) incomplete: bool,
}

/// What the watcher's thread collects for [`FolderWatch::settled`].
#[derive(Default)]
struct Pending {
    last_change: Option<Instant>,
    /// When the OS last reported anything at all, reads included: proof
    /// the watch is still running.
    last_heard: Option<Instant>,
    burst: Burst,
    /// `burst.paths` as a set: a checkout or batch import names tens of
    /// thousands of files, too many to search a list for each.
    seen: HashSet<PathBuf>,
}

impl Pending {
    /// Take one report from the OS. Whether it may be a change.
    fn note(&mut self, event: notify::Result<notify::Event>) -> bool {
        let now = Instant::now();
        self.last_heard = Some(now);
        match event {
            // An error or a rescan request (the OS dropped events when its
            // buffer overflowed) may have hidden a change, so it counts as one.
            Err(_) => self.burst.incomplete = true,
            Ok(event) if event.need_rescan() => self.burst.incomplete = true,
            // Reads are not changes, nor are Git's own records.
            Ok(event)
                if matches!(event.kind, EventKind::Access(_))
                    || (!event.paths.is_empty() && event.paths.iter().all(|path| inside_git(path))) =>
            {
                return false;
            }
            Ok(event) => {
                for path in event.paths {
                    if !inside_git(&path) && self.seen.insert(path.clone()) {
                        self.burst.paths.push(path);
                    }
                }
            }
        }
        self.last_change = Some(now);
        true
    }
}

/// A recursive watch on one folder. Dropping it stops watching.
pub(crate) struct FolderWatch {
    _watcher: notify::RecommendedWatcher,
    pending: Arc<Mutex<Pending>>,
    started: Instant,
}

impl FolderWatch {
    /// Watch `root` and everything under it. `wake` runs on the watcher's
    /// thread after each change, with how long until the burst could settle,
    /// so the caller can come back and call [`FolderWatch::settled`].
    pub(crate) fn start(root: &Path, wake: impl Fn(Duration) + Send + 'static) -> notify::Result<Self> {
        let pending = Arc::new(Mutex::new(Pending::default()));
        let noted = Arc::clone(&pending);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let changed = noted.lock().is_ok_and(|mut pending| pending.note(event));
            if changed {
                wake(WATCH_QUIET + Duration::from_millis(50));
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher, pending, started: Instant::now() })
    }

    /// Whether the OS has reported anything within `within`. Windows can
    /// stop a watch after an error without saying so, so a watch that has
    /// gone quiet may have stopped.
    pub(crate) fn heard_within(&self, within: Duration) -> bool {
        self.pending
            .lock()
            .ok()
            .and_then(|pending| pending.last_heard)
            .is_some_and(|at| at.elapsed() <= within)
    }

    /// Whether the watch has run for all of the last `span` and reported
    /// nothing in it. A change made in that time that it did not report
    /// means it has stopped.
    pub(crate) fn silent_for(&self, span: Duration) -> bool {
        self.started.elapsed() >= span && !self.heard_within(span)
    }

    /// Once per burst, when there were changes and none for `quiet` since:
    /// what changed.
    pub(crate) fn settled(&self, quiet: Duration) -> Option<Burst> {
        let mut pending = self.pending.lock().ok()?;
        match pending.last_change {
            Some(at) if at.elapsed() >= quiet => {
                pending.last_change = None;
                pending.seen.clear();
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
        // macOS's FSEvents can report the folders made just before the watch
        // started a moment late: let them pass first.
        std::thread::sleep(WATCH_QUIET * 3);
        let _ = watch.settled(WATCH_QUIET);

        std::fs::write(root.join(".git").join("index"), b"index").unwrap();
        std::fs::write(git.join("main"), b"ref").unwrap();
        std::thread::sleep(WATCH_QUIET * 3);
        let burst = watch.settled(WATCH_QUIET);
        assert!(burst.is_none(), "Git's own records are ignored, but the watch saw {burst:?}");

        std::fs::write(root.join("warthog.model"), b"tag").unwrap();
        wait_for(&watch, "a tag beside the repository");

        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_rescan_request_means_anything_may_have_changed() {
        let mut pending = Pending::default();
        let rescan = notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        assert!(pending.note(Ok(rescan)), "a rescan request may hide a change");
        assert!(pending.burst.incomplete);
        assert!(pending.burst.paths.is_empty());

        let mut pending = Pending::default();
        let read = notify::Event::new(EventKind::Access(notify::event::AccessKind::Any)).add_path(PathBuf::from("a.model"));
        assert!(!pending.note(Ok(read)), "a read is not a change");
        assert!(pending.last_heard.is_some(), "but it shows the watch is running");
        assert!(pending.last_change.is_none());
    }

    #[test]
    fn a_reported_change_shows_the_watch_is_running() {
        let root = std::env::temp_dir().join(format!("baboon-folder-watch-heard-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let watch = FolderWatch::start(&root, |_| {}).unwrap();
        std::fs::write(root.join("warthog.model"), b"tag").unwrap();
        wait_for(&watch, "the write to settle");
        assert!(watch.heard_within(Duration::from_secs(30)), "the write was heard");

        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
    }
}
