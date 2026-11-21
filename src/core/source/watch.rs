//! Hearing about changes under a loose kit's folder as they happen, through
//! the operating system's file notifications (`notify`). A watch says only
//! that *something* under the folder changed and when it last did; callers
//! wait for the burst to go quiet (Tool and Explorer write in bursts) and
//! then look at the files they care about themselves.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

/// How long changes must stop before a burst of them counts as settled.
pub(crate) const WATCH_QUIET: Duration = Duration::from_millis(300);

/// A recursive watch on one folder. Dropping it stops watching.
pub(crate) struct FolderWatch {
    _watcher: notify::RecommendedWatcher,
    last_change: Arc<Mutex<Option<Instant>>>,
}

impl FolderWatch {
    /// Watch `root` and everything under it. `wake` runs on the watcher's
    /// thread after each change, with how long until the burst could settle,
    /// so the caller can come back and call [`FolderWatch::settled`].
    pub(crate) fn start(root: &Path, wake: impl Fn(Duration) + Send + 'static) -> notify::Result<Self> {
        let last_change = Arc::new(Mutex::new(None));
        let noted = Arc::clone(&last_change);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Reads are not changes. An error (the OS dropped events when its
            // buffer overflowed) may have hidden one, so it counts as one.
            if let Ok(event) = &event
                && matches!(event.kind, EventKind::Access(_))
            {
                return;
            }
            if let Ok(mut last) = noted.lock() {
                *last = Some(Instant::now());
            }
            wake(WATCH_QUIET + Duration::from_millis(50));
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher, last_change })
    }

    /// True once per burst: there were changes, and none for `quiet` since.
    pub(crate) fn settled(&self, quiet: Duration) -> bool {
        let Ok(mut last) = self.last_change.lock() else {
            return false;
        };
        match *last {
            Some(at) if at.elapsed() >= quiet => {
                *last = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(watch: &FolderWatch, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !watch.settled(WATCH_QUIET) {
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
        assert!(!watch.settled(WATCH_QUIET), "nothing changed yet");

        std::fs::write(nested.join("warthog.model"), b"one").unwrap();
        std::fs::write(nested.join("warthog.model"), b"two").unwrap();
        wait_for(&watch, "the write burst to settle");
        assert!(!watch.settled(WATCH_QUIET), "a burst settles once");

        std::fs::remove_file(nested.join("warthog.model")).unwrap();
        wait_for(&watch, "the deletion to settle");

        drop(watch);
        let _ = std::fs::remove_dir_all(&root);
    }
}
