//! Ledger of Campaign Evolved tags this installation created by duplication.
//! It owns the record of what Baboon authored inside a container; container
//! mutation, eligibility presentation, and workflow coordination belong elsewhere.
//!
//! A duplicated container tag is indistinguishable from a shipped one once it is
//! in the pak: it mounts as an ordinary `TagEntryLocation::Container`, the
//! project database files it as `CampaignProjectTagKind::Existing`, and nothing
//! in the container itself records who put it there. Deleting only what Baboon
//! authored therefore needs an explicit, persistent record — this one — kept
//! beside the rest of the installation's state rather than inside the game's
//! files, so a game update or a reinstall never launders a shipped tag into a
//! deletable one.

use super::*;

use serde::{Deserialize, Serialize};
use std::io::Write as _;

const LEDGER_FILE: &str = "campaign_duplicates.json";
const LEDGER_VERSION: u32 = 1;
/// Suffix of the immutable backup Baboon leaves beside a container it writes to.
/// Must match the one `duplicate.rs` allocates its slots from.
const DUPLICATE_BACKUP_SUFFIX: &str = ".baboon-duplicate-backup";
/// `-==--==--==--==-`, the IoStore TOC magic.
const TOC_MAGIC: &[u8; 16] = b"-==--==--==--==-";

/// One tag Baboon duplicated into a container, identified by everything needed
/// to prove the copy on disk is still the one that was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct CreatedTagRecord {
    /// The container the copy was written into, as an absolute path.
    pub(in crate::app) utoc_path: String,
    /// Pack label, e.g. `pakchunk240-WinGDK`. Half of the browser's tag key.
    pub(in crate::app) chunk_label: String,
    pub(in crate::app) package_path: String,
    /// `FPackageId` of `package_path`, stored so a record can be checked
    /// without re-deriving the hash.
    pub(in crate::app) package_id: u64,
    pub(in crate::app) uasset_path: String,
    pub(in crate::app) ubulk_path: String,
    pub(in crate::app) display_path: String,
    pub(in crate::app) group_tag: u32,
    /// The tag this one was copied from, for the confirmation dialog.
    pub(in crate::app) source_display: String,
    /// How many chunks the container held before this copy was written.
    ///
    /// The container itself records nothing about who wrote a chunk, so this is
    /// the only provenance evidence that exists: a chunk at or past this index
    /// is one Baboon appended. `blam-tags` re-checks it before retiring
    /// anything, which is what keeps a delete off the game's own tags.
    #[serde(default)]
    pub(in crate::app) container_entry_count_before: u32,
    pub(in crate::app) created_unix_secs: u64,
}

impl CreatedTagRecord {
    fn addresses(&self, utoc_path: &Path, ubulk_path: &str) -> bool {
        Path::new(&self.utoc_path) == utoc_path
            && self.ubulk_path.eq_ignore_ascii_case(ubulk_path)
    }
}

/// Every duplicate this installation has made, across every game folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(in crate::app) struct CreatedTagLedger {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    tags: Vec<CreatedTagRecord>,
}

impl CreatedTagLedger {
    pub(in crate::app) fn path() -> PathBuf {
        crate::storage::data_path(LEDGER_FILE)
    }

    /// Read the ledger, treating an absent or unreadable file as empty.
    ///
    /// An empty ledger only ever costs the user a greyed-out Delete, whereas
    /// refusing to start over a malformed sidecar would cost them the app.
    pub(in crate::app) fn load() -> Self {
        let path = Self::path();
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub(in crate::app) fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let json = serde_json::to_vec_pretty(&Self {
            version: LEDGER_VERSION,
            tags: self.tags.clone(),
        })
        .map_err(|error| format!("Could not encode the duplicate ledger: {error}"))?;
        let mut file = atomic_write_file::AtomicWriteFile::open(&path)
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        file.write_all(&json)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        file.commit()
            .map_err(|error| format!("Could not save {}: {error}", path.display()))
    }

    /// Record a duplicate, replacing any earlier record for the same container
    /// path. A path can be reused after a delete, and the newer copy is the one
    /// that exists.
    pub(in crate::app) fn record(&mut self, record: CreatedTagRecord) {
        let utoc = PathBuf::from(&record.utoc_path);
        self.tags
            .retain(|existing| !existing.addresses(&utoc, &record.ubulk_path));
        self.tags.push(record);
    }

    /// Drop the record for one copy. Returns whether anything was recorded.
    pub(in crate::app) fn forget(&mut self, utoc_path: &Path, ubulk_path: &str) -> bool {
        let before = self.tags.len();
        self.tags
            .retain(|existing| !existing.addresses(utoc_path, ubulk_path));
        self.tags.len() != before
    }

    pub(in crate::app) fn find(
        &self,
        utoc_path: &Path,
        ubulk_path: &str,
    ) -> Option<&CreatedTagRecord> {
        self.tags
            .iter()
            .find(|existing| existing.addresses(utoc_path, ubulk_path))
    }

    pub(in crate::app) fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }
}

/// How many chunks a container held before Baboon first wrote to it, recovered
/// from the immutable backups it left beside the `.utoc`.
///
/// The ledger only knows about copies made since it existed, and a sidecar can
/// be lost or moved between machines — but a backup is written immediately
/// *before* the first mutation of a container, so its chunk count is exactly the
/// line above which every chunk was appended by Baboon. Taking the lowest count
/// across the backups keeps the answer conservative when an early backup has
/// been deleted: the line only ever moves up, which under-reports what may be
/// deleted rather than over-reporting it.
///
/// Only the 144-byte TOC header is read, and only after checking the magic, so
/// this stays cheap enough to run for every mounted container.
pub(in crate::app) fn container_original_entry_count(utoc: &Path) -> Option<u32> {
    let directory = utoc.parent()?;
    let stem = utoc.file_name()?.to_string_lossy().into_owned();
    let prefix = format!("{stem}{DUPLICATE_BACKUP_SUFFIX}");
    let mut lowest = None;
    for entry in fs::read_dir(directory).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || name.ends_with(".manifest.json") {
            continue;
        }
        if let Some(count) = toc_entry_count(&entry.path()) {
            lowest = Some(lowest.map_or(count, |existing: u32| existing.min(count)));
        }
    }
    lowest
}

/// `entry_count` from a `.utoc` header, or `None` if this is not one.
fn toc_entry_count(path: &Path) -> Option<u32> {
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0u8; 28];
    std::io::Read::read_exact(&mut file, &mut header).ok()?;
    if &header[..16] != TOC_MAGIC {
        return None;
    }
    Some(u32::from_le_bytes(header[24..28].try_into().ok()?))
}

/// Hash a package path to the identity the runtime uses for it.
pub(in crate::app) fn package_id_for(package_path: &str) -> u64 {
    blam_tags::iostore::package::ue_types::FPackageId::from_name(package_path).0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(utoc: &str, ubulk: &str) -> CreatedTagRecord {
        CreatedTagRecord {
            utoc_path: utoc.to_owned(),
            chunk_label: "pakchunk240-WinGDK".to_owned(),
            package_path: "/Game/Tags/objects/copy-biped".to_owned(),
            package_id: package_id_for("/Game/Tags/objects/copy-biped"),
            uasset_path: "Meteorite/Content/Tags/objects/copy-biped.uasset".to_owned(),
            ubulk_path: ubulk.to_owned(),
            display_path: "objects/copy.biped".to_owned(),
            group_tag: 0,
            source_display: "objects/original.biped".to_owned(),
            container_entry_count_before: 4,
            created_unix_secs: 1,
        }
    }

    #[test]
    fn a_record_addresses_one_copy_in_one_container() {
        let ledger = {
            let mut ledger = CreatedTagLedger::default();
            ledger.record(record(
                "C:/Game/Paks/pakchunk240-WinGDK.utoc",
                "Meteorite/Content/Tags/objects/copy-biped.ubulk",
            ));
            ledger
        };

        assert!(
            ledger
                .find(
                    Path::new("C:/Game/Paks/pakchunk240-WinGDK.utoc"),
                    "Meteorite/Content/Tags/objects/COPY-biped.ubulk",
                )
                .is_some(),
            "container paths are matched case-insensitively, as the mount stores them"
        );
        assert!(
            ledger
                .find(
                    Path::new("D:/Other/Paks/pakchunk240-WinGDK.utoc"),
                    "Meteorite/Content/Tags/objects/copy-biped.ubulk",
                )
                .is_none(),
            "the same relative path in another install is a different tag"
        );
    }

    #[test]
    fn recording_the_same_path_twice_keeps_only_the_newer_copy() {
        let mut ledger = CreatedTagLedger::default();
        let utoc = "C:/Game/Paks/pakchunk240-WinGDK.utoc";
        let ubulk = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
        ledger.record(record(utoc, ubulk));
        let mut newer = record(utoc, ubulk);
        newer.created_unix_secs = 99;
        ledger.record(newer);

        assert_eq!(ledger.tags.len(), 1);
        assert_eq!(
            ledger.find(Path::new(utoc), ubulk).unwrap().created_unix_secs,
            99
        );
        assert!(ledger.forget(Path::new(utoc), ubulk));
        assert!(!ledger.forget(Path::new(utoc), ubulk));
        assert!(ledger.is_empty());
    }

    fn write_test_toc(path: &Path, entry_count: u32) {
        let mut bytes = vec![0u8; 144];
        bytes[..16].copy_from_slice(TOC_MAGIC);
        bytes[24..28].copy_from_slice(&entry_count.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn backups_recover_the_chunk_count_a_container_started_with() {
        let root = std::env::temp_dir().join(format!(
            "baboon-backup-provenance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let utoc = root.join("pakchunk0-Windows.utoc");
        write_test_toc(&utoc, 122_810);

        // No backup yet: nothing in the container can be claimed as Baboon's.
        assert_eq!(container_original_entry_count(&utoc), None);

        write_test_toc(
            &root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup"),
            122_804,
        );
        write_test_toc(
            &root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup-1"),
            122_806,
        );
        fs::write(
            root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup.manifest.json"),
            b"{}",
        )
        .unwrap();
        // The earliest backup wins, so a later one cannot narrow the window and
        // strand a copy made before it. Manifests are not TOCs and are skipped.
        assert_eq!(container_original_entry_count(&utoc), Some(122_804));

        // A sibling that is not one of our backups is ignored entirely.
        write_test_toc(&root.join("pakchunk1-Windows.utoc"), 5);
        assert_eq!(container_original_entry_count(&utoc), Some(122_804));

        fs::write(root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup-2"), b"not a toc")
            .unwrap();
        assert_eq!(container_original_entry_count(&utoc), Some(122_804));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_ledger_reads_as_empty_rather_than_failing_startup() {
        assert!(serde_json::from_slice::<CreatedTagLedger>(b"{ not json")
            .unwrap_or_default()
            .is_empty());
    }
}
