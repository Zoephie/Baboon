//! Which loose tag files a piece of background work read, so it can be done
//! again when one of them changes. Work turns recording on for its thread
//! with [`record_reads`]; every loose tag read through [`super::read_entry`]
//! is then noted in the shared [`ReadFiles`], with the file as it was when
//! read. A Model Preview keeps one for all the jobs behind it (the model, its
//! textures, overlays, weapons and clips), and reloads when Baboon saves, or
//! the folder watch sees, a file in it that has since changed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::core::document::disk::{DiskCheck, DiskStamp};

/// The files read, by [`file_key`], each with its stamp when read (`None`
/// for one that was missing then, which counts as changed once it appears).
#[derive(Clone, Default)]
pub(crate) struct ReadFiles(Arc<Mutex<HashMap<String, Option<DiskStamp>>>>);

thread_local! {
    static RECORDING: RefCell<Option<ReadFiles>> = const { RefCell::new(None) };
}

/// Recording on this thread until dropped, when what was recording before
/// (if anything) resumes.
pub(crate) struct Recording(Option<ReadFiles>);

impl Drop for Recording {
    fn drop(&mut self) {
        let previous = self.0.take();
        RECORDING.with(|recording| *recording.borrow_mut() = previous);
    }
}

/// Note every loose tag this thread reads in `files`, until the result drops.
pub(crate) fn record_reads(files: &ReadFiles) -> Recording {
    Recording(RECORDING.with(|recording| recording.borrow_mut().replace(files.clone())))
}

/// Called by the tag readers for each loose file they open.
pub(crate) fn note_read(path: &Path) {
    RECORDING.with(|recording| {
        let Some(files) = recording.borrow().as_ref().map(ReadFiles::clone) else {
            return;
        };
        let key = file_key(path);
        let Ok(mut read) = files.0.lock() else {
            return;
        };
        // The first read is the one the work was built from.
        read.entry(key).or_insert_with(|| DiskStamp::of_file(path).ok());
    });
}

/// How two spellings of one file compare: the file system is not case
/// sensitive on Windows, and references, saves and the OS each spell paths
/// their own way.
pub(crate) fn file_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.replace('/', "\\").to_lowercase()
    } else {
        text.into_owned()
    }
}

/// Which files have changed, among those a piece of work may have read.
pub(crate) enum Changed<'a> {
    /// These, which the folder watch named or Baboon just saved.
    Files(&'a [&'a Path]),
    /// Unknown: the watch dropped events, or there is no watch.
    Anything,
}

impl ReadFiles {
    /// Whether a file this read is no longer as it was then. `Anything`
    /// looks at every file read, which costs a disk lookup each.
    pub(crate) fn any_changed(&self, changed: &Changed) -> bool {
        let Ok(read) = self.0.lock() else {
            return false;
        };
        let differs = |path: &Path, stamp: &Option<DiskStamp>| match stamp {
            None => path.is_file(),
            Some(stamp) => !matches!(stamp.check(path), DiskCheck::Same | DiskCheck::Unreadable(_)),
        };
        match changed {
            Changed::Files(paths) => paths
                .iter()
                .any(|path| read.get(&file_key(path)).is_some_and(|stamp| differs(path, stamp))),
            Changed::Anything => read.iter().any(|(key, stamp)| differs(Path::new(key), stamp)),
        }
    }

    /// Whether any file read ends in `.extension`.
    #[cfg(test)]
    pub(crate) fn contains_extension(&self, extension: &str) -> bool {
        let suffix = format!(".{extension}");
        self.0.lock().is_ok_and(|read| read.keys().any(|key| key.ends_with(&suffix)))
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.0.lock().is_ok_and(|read| read.contains_key(&file_key(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("baboon-reads-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_reads_made_while_recording_are_noted() {
        let dir = temp("recording");
        let (shader, bitmap) = (dir.join("rock.shader"), dir.join("rock.bitmap"));
        std::fs::write(&shader, b"shader").unwrap();
        std::fs::write(&bitmap, b"bitmap").unwrap();
        let files = ReadFiles::default();
        note_read(&bitmap);
        {
            let _recording = record_reads(&files);
            note_read(&shader);
        }
        note_read(&bitmap);
        assert!(files.contains(&shader));
        assert!(!files.contains(&bitmap));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_file_counts_as_changed_only_once_it_changes() {
        let dir = temp("changed");
        let shader = dir.join("Rock.shader");
        let missing = dir.join("moss.bitmap");
        let other = dir.join("other.shader");
        std::fs::write(&shader, b"shader").unwrap();
        let files = ReadFiles::default();
        {
            let _recording = record_reads(&files);
            note_read(&shader);
            note_read(&missing);
        }
        let spelled_otherwise = dir.join("rock.shader");
        assert!(!files.any_changed(&Changed::Files(&[&spelled_otherwise])), "nothing changed yet");
        assert!(!files.any_changed(&Changed::Anything));
        std::fs::write(&other, b"other").unwrap();
        assert!(!files.any_changed(&Changed::Files(&[&other])), "a file it never read");

        std::fs::write(&shader, b"shader, saved again").unwrap();
        if cfg!(windows) {
            assert!(files.any_changed(&Changed::Files(&[&spelled_otherwise])), "the same file in other letter case");
        }
        assert!(files.any_changed(&Changed::Files(&[&shader])));

        std::fs::write(&missing, b"bitmap").unwrap();
        assert!(files.any_changed(&Changed::Files(&[&missing])), "a file that was missing appeared");
        assert!(files.any_changed(&Changed::Anything));
        let _ = std::fs::remove_dir_all(dir);
    }
}
