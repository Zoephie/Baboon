//! What an open tag's file looked like when its document last matched it,
//! and whether the file still looks that way.
//!
//! Baboon reads a tag once and edits it in memory, so a file replaced outside
//! Baboon (Explorer, Tool, another editor) would otherwise go unnoticed until
//! a save rolled it back. The check compares against what was loaded rather
//! than asking whether the file is newer: an Explorer copy keeps the original
//! modified time, so a replacement can look older than the tag it replaced.

use std::fs::Metadata;
use std::hash::Hasher;
use std::io::ErrorKind;
use std::path::Path;
use std::time::SystemTime;

/// A loose tag file as its document last matched it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskStamp {
    size: u64,
    modified: Option<SystemTime>,
    /// A hash of the bytes, when they were to hand. A file whose size or time
    /// moved but whose bytes did not (a touch, a copy of the same file over
    /// itself) is then not a change. After a save only the metadata is
    /// known, so any later change to it counts.
    hash: Option<u64>,
}

/// What a file is now, against a [`DiskStamp`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiskCheck {
    /// Looks as it did.
    Same,
    /// The metadata moved but the bytes are the same: keep the document and
    /// remember the file as it is now, so the next check is quick.
    Restamped(DiskStamp),
    /// The bytes are different. Holds the file as it is now, so a user who
    /// keeps their own edits can take it as the version they are replacing.
    Changed(DiskStamp),
    /// Nothing is at the path.
    Missing,
    /// The file is there but could not be read (locked mid-write, say).
    /// Nothing is decided; the next check looks again.
    Unreadable(String),
}

impl DiskStamp {
    /// The file as `metadata` describes it, holding `bytes`. Take the
    /// metadata before reading: if the file changes in between, the bytes are
    /// the newer ones and the next check finds the same bytes again.
    pub(crate) fn of(metadata: &Metadata, bytes: &[u8]) -> Self {
        Self { size: metadata.len(), modified: metadata.modified().ok(), hash: Some(hash_bytes(bytes)) }
    }

    /// The file at `path` by its metadata alone: what a save just wrote.
    pub(crate) fn of_file(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self { size: metadata.len(), modified: metadata.modified().ok(), hash: None })
    }

    fn same_metadata(&self, metadata: &Metadata) -> bool {
        self.modified.is_some() && self.size == metadata.len() && self.modified == metadata.modified().ok()
    }

    /// Whether the file at `path` still matches. Reads the file only when its
    /// metadata moved and there is a hash to compare.
    pub(crate) fn check(&self, path: &Path) -> DiskCheck {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return DiskCheck::Missing,
            Err(error) if error.kind() == ErrorKind::NotFound => return DiskCheck::Missing,
            Err(error) => return DiskCheck::Unreadable(error.to_string()),
        };
        if self.same_metadata(&metadata) {
            return DiskCheck::Same;
        }
        let Some(hash) = self.hash else {
            return DiskCheck::Changed(Self { size: metadata.len(), modified: metadata.modified().ok(), hash: None });
        };
        match std::fs::read(path) {
            Ok(bytes) if hash_bytes(&bytes) == hash => DiskCheck::Restamped(Self::of(&metadata, &bytes)),
            Ok(bytes) => DiskCheck::Changed(Self::of(&metadata, &bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound => DiskCheck::Missing,
            Err(error) => DiskCheck::Unreadable(error.to_string()),
        }
    }
}

/// Stable within one run, which is all a stamp needs: stamps never outlive
/// the process.
fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn stamp_of(path: &Path) -> DiskStamp {
        let metadata = fs::metadata(path).unwrap();
        DiskStamp::of(&metadata, &fs::read(path).unwrap())
    }

    fn set_modified(path: &Path, time: SystemTime) {
        fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn an_untouched_file_is_the_same() {
        let dir = crate::core::test_kits::unique_temp_dir("disk-stamp-same");
        let path = dir.join("a.tag");
        fs::write(&path, b"one").unwrap();
        assert_eq!(stamp_of(&path).check(&path), DiskCheck::Same);
        let _ = fs::remove_dir_all(dir);
    }

    /// An Explorer copy keeps the source's modified time, so the replacement
    /// can be older than the tag it replaced. It is still a change.
    #[test]
    fn a_replacement_with_an_older_time_is_a_change() {
        let dir = crate::core::test_kits::unique_temp_dir("disk-stamp-older");
        let path = dir.join("a.tag");
        fs::write(&path, b"one").unwrap();
        let stamp = stamp_of(&path);
        fs::write(&path, b"two").unwrap();
        set_modified(&path, SystemTime::now() - Duration::from_secs(86_400 * 365));
        let DiskCheck::Changed(now) = stamp.check(&path) else {
            panic!("a replacement is a change");
        };
        assert_eq!(now.check(&path), DiskCheck::Same, "the change carries the file as it is now");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_touched_file_with_the_same_bytes_is_restamped_not_changed() {
        let dir = crate::core::test_kits::unique_temp_dir("disk-stamp-touch");
        let path = dir.join("a.tag");
        fs::write(&path, b"one").unwrap();
        let stamp = stamp_of(&path);
        set_modified(&path, SystemTime::now() + Duration::from_secs(60));
        let DiskCheck::Restamped(fresh) = stamp.check(&path) else {
            panic!("the same bytes under a new time should restamp");
        };
        assert_eq!(fresh.check(&path), DiskCheck::Same);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_deleted_file_is_missing() {
        let dir = crate::core::test_kits::unique_temp_dir("disk-stamp-missing");
        let path = dir.join("a.tag");
        fs::write(&path, b"one").unwrap();
        let stamp = stamp_of(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(stamp.check(&path), DiskCheck::Missing);
        let _ = fs::remove_dir_all(dir);
    }

    /// After a save only the metadata is known, so a later write of the same
    /// size is still caught by its time.
    #[test]
    fn a_saved_stamp_counts_any_metadata_change() {
        let dir = crate::core::test_kits::unique_temp_dir("disk-stamp-saved");
        let path = dir.join("a.tag");
        fs::write(&path, b"one").unwrap();
        let stamp = DiskStamp::of_file(&path).unwrap();
        assert_eq!(stamp.check(&path), DiskCheck::Same);
        set_modified(&path, SystemTime::now() + Duration::from_secs(60));
        assert!(matches!(stamp.check(&path), DiskCheck::Changed(_)));
        let _ = fs::remove_dir_all(dir);
    }
}
