//! Which loose tag files a piece of background work read, so it can be done
//! again when one of them changes. Work turns recording on for its thread
//! with [`record_reads`]; every loose tag read through [`super::read_entry`]
//! is then noted in the shared [`ReadFiles`], with the file as it was when
//! read. A Model Preview keeps one for all the jobs behind it (the model, its
//! textures, overlays, weapons and clips), and reloads when Baboon saves, or
//! the folder watch sees, a file in it that has since changed.
//!
//! A [`ReadFiles`] can also carry open tags' unsaved edits ([`Edited`]):
//! a read of one of those files gets the edits instead of the file, and is
//! noted as read from that version of the document.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::core::document::disk::{DiskCheck, DiskStamp};

/// How a file was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadAs {
    /// From disk, as stamped then (`None` for a file missing then, which
    /// counts as changed once it appears).
    Disk(Option<DiskStamp>),
    /// From an open document's unsaved edits, at its content stamp.
    Edits((u64, u64)),
}

/// An open document's unsaved edits, handed to background work to read in
/// place of its file.
pub(crate) struct Edited {
    /// The document's content stamp when the edits were taken.
    pub(crate) stamp: (u64, u64),
    /// `None` when the document could not be written out: the file is read,
    /// but noted at the document's stamp all the same, so the work is not
    /// redone (and fails the same way) until the document changes again.
    pub(crate) bytes: Option<Vec<u8>>,
}

/// An open tag document as [`ReadFiles::edits_out_of_date`] compares it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenDocument {
    pub(crate) stamp: (u64, u64),
    pub(crate) dirty: bool,
}

/// The files read, by [`file_key`], and how; with the unsaved edits reads
/// take in place of their files.
#[derive(Clone, Default)]
pub(crate) struct ReadFiles {
    read: Arc<Mutex<HashMap<String, ReadAs>>>,
    edits: Arc<HashMap<String, Edited>>,
}

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

/// Called by the loose tag reader for each file it opens. Returns the
/// unsaved edits to read instead, when the recording carries them.
pub(crate) fn note_read(path: &Path) -> Option<Vec<u8>> {
    RECORDING.with(|recording| {
        let files = recording.borrow().as_ref().map(ReadFiles::clone)?;
        let key = file_key(path);
        let edited = files.edits.get(&key);
        let mut read = files.read.lock().ok()?;
        // The first read is the one the work was built from.
        read.entry(key).or_insert_with(|| match edited {
            Some(edited) => ReadAs::Edits(edited.stamp),
            None => ReadAs::Disk(DiskStamp::of_file(path).ok()),
        });
        edited.and_then(|edited| edited.bytes.clone())
    })
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
    /// A new list whose reads take `edits` (by [`file_key`]) in place of
    /// their files.
    pub(crate) fn with_edits(edits: HashMap<String, Edited>) -> Self {
        Self { read: Arc::default(), edits: Arc::new(edits) }
    }

    /// The files whose reads take unsaved edits, by [`file_key`].
    #[cfg(test)]
    pub(crate) fn edited_files(&self) -> Vec<String> {
        let mut files: Vec<String> = self.edits.keys().cloned().collect();
        files.sort();
        files
    }

    /// How many files have been read so far.
    pub(crate) fn len(&self) -> usize {
        self.read.lock().map(|read| read.len()).unwrap_or_default()
    }

    /// The files read so far, by [`file_key`].
    pub(crate) fn keys(&self) -> Vec<String> {
        self.read.lock().map(|read| read.keys().cloned().collect()).unwrap_or_default()
    }

    /// Whether a file read from disk is no longer as it was then. `Anything`
    /// looks at every file read, which costs a disk lookup each. Files read
    /// from unsaved edits are [`Self::edits_out_of_date`]'s to judge, and
    /// `skip` (by [`file_key`]) is left out.
    pub(crate) fn any_changed(&self, changed: &Changed, skip: Option<&str>) -> bool {
        let Ok(read) = self.read.lock() else {
            return false;
        };
        let differs = |path: &Path, read_as: &ReadAs| match read_as {
            ReadAs::Edits(_) => false,
            ReadAs::Disk(None) => path.is_file(),
            ReadAs::Disk(Some(stamp)) => !matches!(stamp.check(path), DiskCheck::Same | DiskCheck::Unreadable(_)),
        };
        let named: Option<HashSet<String>> = match changed {
            Changed::Files(paths) => Some(paths.iter().map(|path| file_key(path)).collect()),
            Changed::Anything => None,
        };
        read.iter()
            .filter(|(key, _)| Some(key.as_str()) != skip)
            // A folder renamed or deleted whole is named alone, not the
            // files in it: a file read under it counts as named too.
            .filter(|(key, _)| {
                named.as_ref().is_none_or(|named| {
                    Path::new(key.as_str()).ancestors().any(|path| path.to_str().is_some_and(|path| named.contains(path)))
                })
            })
            .any(|(key, read_as)| differs(Path::new(key.as_str()), read_as))
    }

    /// Whether the open documents, by [`file_key`], now differ from what was
    /// read: a file read from disk whose document now has unsaved edits, or
    /// edits read from a document that has since changed, been reverted,
    /// reloaded or closed. `own` (the previewed tag) is left out. `Some`
    /// names the difference, so a caller can wait for edits to settle.
    pub(crate) fn edits_out_of_date(&self, open: &HashMap<String, OpenDocument>, own: Option<&str>) -> Option<u64> {
        let read = self.read.lock().ok()?;
        let mut differences = 0_u64;
        let mut differs = false;
        for (key, read_as) in read.iter() {
            if Some(key.as_str()) == own {
                continue;
            }
            let document = open.get(key);
            let same = match (read_as, document) {
                (ReadAs::Edits(stamp), Some(document)) => document.stamp == *stamp,
                (ReadAs::Edits(_), None) => false,
                (ReadAs::Disk(_), document) => !document.is_some_and(|document| document.dirty),
            };
            if !same {
                differs = true;
                let mut hasher = DefaultHasher::new();
                (key, document.map(|document| document.stamp)).hash(&mut hasher);
                // Order-free: the map's iteration order is not the point.
                differences ^= hasher.finish();
            }
        }
        differs.then_some(differences)
    }

    /// Whether any file read ends in `.extension`.
    #[cfg(test)]
    pub(crate) fn contains_extension(&self, extension: &str) -> bool {
        let suffix = format!(".{extension}");
        self.read.lock().is_ok_and(|read| read.keys().any(|key| key.ends_with(&suffix)))
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.read.lock().is_ok_and(|read| read.contains_key(&file_key(path)))
    }

    /// How `path` was read, if it was.
    #[cfg(test)]
    pub(crate) fn read_as(&self, path: &Path) -> Option<ReadAs> {
        self.read.lock().ok()?.get(&file_key(path)).copied()
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
        assert!(!files.any_changed(&Changed::Files(&[&spelled_otherwise]), None), "nothing changed yet");
        assert!(!files.any_changed(&Changed::Anything, None));
        std::fs::write(&other, b"other").unwrap();
        assert!(!files.any_changed(&Changed::Files(&[&other]), None), "a file it never read");

        std::fs::write(&shader, b"shader, saved again").unwrap();
        if cfg!(windows) {
            assert!(files.any_changed(&Changed::Files(&[&spelled_otherwise]), None), "the same file in other letter case");
        }
        assert!(files.any_changed(&Changed::Files(&[&shader]), None));

        std::fs::write(&missing, b"bitmap").unwrap();
        assert!(files.any_changed(&Changed::Files(&[&missing]), None), "a file that was missing appeared");
        assert!(files.any_changed(&Changed::Anything, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_of_an_edited_file_takes_the_edits_and_notes_them() {
        let dir = temp("edits");
        let (shader, bitmap) = (dir.join("rock.shader"), dir.join("rock.bitmap"));
        std::fs::write(&shader, b"saved").unwrap();
        std::fs::write(&bitmap, b"bitmap").unwrap();
        let edits = HashMap::from([(file_key(&shader), Edited { stamp: (7, 3), bytes: Some(b"edited".to_vec()) })]);
        let files = ReadFiles::with_edits(edits);
        let _recording = record_reads(&files);
        assert_eq!(note_read(&shader).as_deref(), Some(&b"edited"[..]));
        assert_eq!(note_read(&bitmap), None, "no edits: the file is read");
        assert_eq!(files.read_as(&shader), Some(ReadAs::Edits((7, 3))));
        assert!(matches!(files.read_as(&bitmap), Some(ReadAs::Disk(Some(_)))));
        std::fs::write(&shader, b"saved again elsewhere").unwrap();
        assert!(!files.any_changed(&Changed::Files(&[&shader]), None), "edits read are judged by their document");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edits_out_of_date_follows_the_open_documents() {
        let dir = temp("out-of-date");
        let (shader, bitmap, own) = (dir.join("rock.shader"), dir.join("rock.bitmap"), dir.join("rock.render_model"));
        for path in [&shader, &bitmap, &own] {
            std::fs::write(path, b"tag").unwrap();
        }
        let edits = HashMap::from([(file_key(&shader), Edited { stamp: (7, 3), bytes: Some(b"edited".to_vec()) })]);
        let files = ReadFiles::with_edits(edits);
        {
            let _recording = record_reads(&files);
            for path in [&shader, &bitmap, &own] {
                note_read(path);
            }
        }
        let document = |stamp, dirty| OpenDocument { stamp, dirty };
        let open = |documents: &[(&std::path::PathBuf, OpenDocument)]| -> HashMap<String, OpenDocument> {
            documents.iter().map(|(path, document)| (file_key(path), *document)).collect()
        };
        let own_key = file_key(&own);
        let own = Some(own_key.as_str());

        // As drawn: the shader at its stamp, the bitmap clean, the previewed
        // tag itself edited (its own document's business).
        let drawn = open(&[(&shader, document((7, 3), true)), (&bitmap, document((9, 0), false)), (&own.map(std::path::PathBuf::from).unwrap(), document((1, 5), true))]);
        assert_eq!(files.edits_out_of_date(&drawn, own), None);
        // Saved: same stamp, no longer dirty, still what was drawn.
        assert_eq!(files.edits_out_of_date(&open(&[(&shader, document((7, 3), false))]), own), None);

        let edited_again = files.edits_out_of_date(&open(&[(&shader, document((7, 4), true))]), own);
        let edited_more = files.edits_out_of_date(&open(&[(&shader, document((7, 5), true))]), own);
        assert!(edited_again.is_some() && edited_again != edited_more, "each edit is a new difference");
        assert!(files.edits_out_of_date(&open(&[]), own).is_some(), "closed, its edits discarded");
        assert!(files.edits_out_of_date(&open(&[(&shader, document((8, 0), false))]), own).is_some(), "reloaded from disk");
        let bitmap_edited = open(&[(&shader, document((7, 3), true)), (&bitmap, document((9, 1), true))]);
        assert!(files.edits_out_of_date(&bitmap_edited, own).is_some(), "a file read from disk now has edits");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Explorer names only the folder when one is renamed or sent to the
    /// Recycle Bin whole: the files read inside it count as named.
    #[test]
    fn a_folder_named_alone_stands_for_the_files_read_inside_it() {
        let dir = temp("folder");
        let shaders = dir.join("objects").join("rock").join("shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        let (shader, own) = (shaders.join("rock.shader"), shaders.join("rock.render_model"));
        std::fs::write(&shader, b"shader").unwrap();
        std::fs::write(&own, b"model").unwrap();
        let files = ReadFiles::default();
        {
            let _recording = record_reads(&files);
            note_read(&shader);
            note_read(&own);
        }
        let sibling = dir.join("objects").join("rock").join("shaders2");
        std::fs::create_dir_all(&sibling).unwrap();
        assert!(!files.any_changed(&Changed::Files(&[&shaders]), None), "named, but nothing in it changed");

        std::fs::write(&own, b"model, saved").unwrap();
        let own_key = file_key(&own);
        assert!(!files.any_changed(&Changed::Files(&[&shaders]), Some(&own_key)), "the skipped file is left out");

        std::fs::remove_file(&shader).unwrap();
        assert!(!files.any_changed(&Changed::Files(&[&sibling]), None), "a folder whose name only starts the same");
        assert!(files.any_changed(&Changed::Files(&[&shaders]), None), "its shader is gone");
        assert!(files.any_changed(&Changed::Files(&[&dir.join("objects")]), None), "and so is a grandparent named");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Edits that could not be written out are read from the file, but noted
    /// at the document's stamp, so the work is not redone until it changes.
    #[test]
    fn unwritable_edits_read_the_file_but_note_the_document() {
        let dir = temp("unwritable");
        let shader = dir.join("rock.shader");
        std::fs::write(&shader, b"saved").unwrap();
        let files = ReadFiles::with_edits(HashMap::from([(file_key(&shader), Edited { stamp: (7, 3), bytes: None })]));
        let _recording = record_reads(&files);
        assert_eq!(note_read(&shader), None, "the file is read");
        assert_eq!(files.read_as(&shader), Some(ReadAs::Edits((7, 3))));
        let open = HashMap::from([(file_key(&shader), OpenDocument { stamp: (7, 3), dirty: true })]);
        assert_eq!(files.edits_out_of_date(&open, None), None, "as drawn: nothing to load again");
        let _ = std::fs::remove_dir_all(dir);
    }
}
