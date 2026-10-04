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
/// Must match the one the `duplicate` module allocates its slots from.
const DUPLICATE_BACKUP_SUFFIX: &str = ".baboon-duplicate-backup";
/// `-==--==--==--==-`, the IoStore TOC magic.
const TOC_MAGIC: &[u8; 16] = b"-==--==--==--==-";

/// Why a container tag is in this ledger — which decides whether deleting it
/// would destroy something the game shipped.
///
/// Being in the ledger used to mean exactly one thing, so the question never
/// arose. Renaming in place breaks that: a renamed tag's chunks are appended
/// like any other, and the chunk-index threshold that stands in for provenance
/// cannot tell a copy Baboon made from a shipped tag Baboon moved. Both sit past
/// the line. Only this field can separate them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum CreatedTagOrigin {
    /// Baboon put this content in the container — a duplicate, or a tag created
    /// from scratch. Deleting it removes only what Baboon added.
    ///
    /// The default, so every row written before this field existed reads as what
    /// it actually was: at the time, a duplicate was the only thing the ledger
    /// could hold.
    #[default]
    Authored,
    /// The chunks are Baboon's, but the tag is not: one the game ships was
    /// renamed, so its payload was re-emitted at a new path and the original
    /// retired. There is no other copy of it, and deleting it would be
    /// destroying shipped content through a path built to protect it.
    RenamedFromShipped,
    /// A value this build does not know, written by a newer one. Kept as it was
    /// so saving the ledger writes it back unchanged, and treated as not
    /// Baboon's to delete or rename: only `Authored` grants that.
    ///
    /// Without it, one such value failed the whole file's parse, the ledger
    /// loaded empty, and the next save erased every record in it.
    Unrecognized(String),
}

impl CreatedTagOrigin {
    fn as_str(&self) -> &str {
        match self {
            CreatedTagOrigin::Authored => "Authored",
            CreatedTagOrigin::RenamedFromShipped => "RenamedFromShipped",
            CreatedTagOrigin::Unrecognized(value) => value,
        }
    }
}

impl Serialize for CreatedTagOrigin {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CreatedTagOrigin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "Authored" => CreatedTagOrigin::Authored,
            "RenamedFromShipped" => CreatedTagOrigin::RenamedFromShipped,
            _ => CreatedTagOrigin::Unrecognized(value),
        })
    }
}

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
    /// Whether deleting this tag would remove content Baboon added, or content
    /// the game shipped that Baboon merely moved. Defaulted rather than
    /// versioned: an older file has no `origin` and every row in it predates
    /// renaming, so `Authored` is not a fallback but the truth. An older build
    /// reading a newer file ignores the field, which is the safe direction —
    /// that build cannot rename, so it can never have written the other value.
    #[serde(default)]
    pub(in crate::app) origin: CreatedTagOrigin,
    pub(in crate::app) created_unix_secs: u64,
}

impl CreatedTagRecord {
    fn addresses(&self, utoc_path: &Path, ubulk_path: &str) -> bool {
        Path::new(&self.utoc_path) == utoc_path && self.ubulk_path.eq_ignore_ascii_case(ubulk_path)
    }
}

/// Every duplicate this installation has made, across every game folder.
#[derive(Clone, Debug, Default)]
pub(in crate::app) struct CreatedTagLedger {
    tags: Vec<CreatedTagRecord>,
    /// Rows this build could not read as a record (a newer build's shape),
    /// written back as they were so a save never drops them.
    unparsed: Vec<serde_json::Value>,
    /// Why the file on disk could not be read at all. While set, `save`
    /// refuses: writing would replace every record in it with this session's.
    load_error: Option<String>,
}

/// The file as stored. Rows are read one by one so one this build cannot
/// read costs only that row, never the file.
#[derive(Deserialize)]
struct LedgerFileIn {
    #[serde(default)]
    tags: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct LedgerFileOut {
    version: u32,
    tags: Vec<serde_json::Value>,
}

impl CreatedTagLedger {
    pub(in crate::app) fn path() -> PathBuf {
        crate::storage::data_path(LEDGER_FILE)
    }

    /// Read the ledger, treating an absent file as empty.
    ///
    /// An empty ledger only ever costs the user a greyed-out Delete, whereas
    /// refusing to start over a malformed sidecar would cost them the app. A
    /// file that exists but cannot be read is still loaded as empty, but
    /// remembered as such, so `save` leaves it alone.
    pub(in crate::app) fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub(in crate::app) fn load_from(path: &Path) -> Self {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                return Self {
                    load_error: Some(format!("could not read it: {error}")),
                    ..Self::default()
                };
            }
        };
        Self::from_bytes(&bytes)
    }

    /// The ledger stored as `bytes`; a file that is not a ledger at all reads
    /// as empty with `load_error` set.
    fn from_bytes(bytes: &[u8]) -> Self {
        let file = match serde_json::from_slice::<LedgerFileIn>(bytes) {
            Ok(file) => file,
            Err(error) => {
                return Self {
                    load_error: Some(format!("could not parse it: {error}")),
                    ..Self::default()
                };
            }
        };
        let mut ledger = Self::default();
        for row in file.tags {
            match serde_json::from_value::<CreatedTagRecord>(row.clone()) {
                Ok(record) => ledger.tags.push(record),
                Err(_) => ledger.unparsed.push(row),
            }
        }
        ledger
    }

    pub(in crate::app) fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub(in crate::app) fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(error) = &self.load_error {
            return Err(format!(
                "The duplicate ledger {} was not updated: when Baboon started it {error}. \
                 It was left as it is so the records in it are not lost.",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mut tags = Vec::with_capacity(self.tags.len() + self.unparsed.len());
        for record in &self.tags {
            tags.push(
                serde_json::to_value(record)
                    .map_err(|error| format!("Could not encode the duplicate ledger: {error}"))?,
            );
        }
        tags.extend(self.unparsed.iter().cloned());
        let json = serde_json::to_vec_pretty(&LedgerFileOut {
            version: LEDGER_VERSION,
            tags,
        })
        .map_err(|error| format!("Could not encode the duplicate ledger: {error}"))?;
        let mut file = atomic_write_file::AtomicWriteFile::open(path)
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

    /// Re-file a record after its tag was renamed inside the container.
    ///
    /// `moved` says where the tag is now. Its `origin` is *ignored*, and the
    /// answer decided here instead, from whether the old path was already in the
    /// ledger — because the caller is the one place that must not be trusted to
    /// get it right, and both ways of getting it wrong are bad. A tag Baboon
    /// authored that lost its record becomes undeletable; a tag the game ships
    /// that gains an `Authored` one becomes deletable, which is the invariant
    /// the whole ledger exists to hold.
    ///
    /// A tag keeps `Authored` across any number of renames, and one that started
    /// as shipped never launders itself back — including by being renamed to a
    /// path some earlier copy once used, since any record at the destination is
    /// dropped rather than inherited.
    pub(in crate::app) fn record_rename(&mut self, old_ubulk_path: &str, moved: CreatedTagRecord) {
        let utoc = PathBuf::from(&moved.utoc_path);
        let previous = self
            .tags
            .iter()
            .find(|existing| existing.addresses(&utoc, old_ubulk_path))
            .cloned();
        let moved = CreatedTagRecord {
            origin: previous
                .as_ref()
                .map_or(CreatedTagOrigin::RenamedFromShipped, |previous| {
                    previous.origin.clone()
                }),
            // Carried from the record being replaced rather than taken from the
            // caller: the renamed chunks were appended later still, so the line
            // already recorded stays true and stays the conservative one.
            container_entry_count_before: previous
                .as_ref()
                .map_or(moved.container_entry_count_before, |previous| {
                    previous.container_entry_count_before
                }),
            created_unix_secs: previous
                .as_ref()
                .map_or(moved.created_unix_secs, |previous| {
                    previous.created_unix_secs
                }),
            source_display: previous.as_ref().map_or(moved.source_display, |previous| {
                previous.source_display.clone()
            }),
            ..moved
        };
        self.tags.retain(|existing| {
            !existing.addresses(&utoc, old_ubulk_path)
                && !existing.addresses(&utoc, &moved.ubulk_path)
        });
        self.tags.push(moved);
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
            origin: CreatedTagOrigin::Authored,
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

    const UTOC: &str = "C:/Game/Paks/pakchunk240-WinGDK.utoc";

    fn renamed_to(ubulk: &str) -> CreatedTagRecord {
        CreatedTagRecord {
            // Deliberately the wrong answer, to prove the caller is not the one
            // deciding: `record_rename` overwrites this from the ledger.
            origin: CreatedTagOrigin::Authored,
            ..record(UTOC, ubulk)
        }
    }

    /// A tag the game ships has no ledger record, so the rename is the first
    /// thing the ledger ever hears about it — and it must not conclude from the
    /// silence that Baboon authored it.
    #[test]
    fn renaming_a_tag_the_ledger_never_knew_marks_it_as_shipped() {
        let mut ledger = CreatedTagLedger::default();
        let new = "Meteorite/Content/Tags/objects/renamed-biped.ubulk";
        ledger.record_rename(
            "Meteorite/Content/Tags/objects/shipped-biped.ubulk",
            renamed_to(new),
        );

        let record = ledger.find(Path::new(UTOC), new).expect("recorded");
        assert_eq!(record.origin, CreatedTagOrigin::RenamedFromShipped);
    }

    /// And the converse: a copy Baboon made keeps its authorship across the
    /// move, along with the provenance line the delete path checks.
    #[test]
    fn renaming_a_copy_carries_its_authorship_and_its_provenance_line() {
        let mut ledger = CreatedTagLedger::default();
        let old = "Meteorite/Content/Tags/objects/copy-biped.ubulk";
        let new = "Meteorite/Content/Tags/objects/moved-biped.ubulk";
        ledger.record(record(UTOC, old));
        ledger.record_rename(
            old,
            CreatedTagRecord {
                container_entry_count_before: 9999,
                ..renamed_to(new)
            },
        );

        assert!(ledger.find(Path::new(UTOC), old).is_none());
        let record = ledger.find(Path::new(UTOC), new).expect("recorded");
        assert_eq!(record.origin, CreatedTagOrigin::Authored);
        assert_eq!(
            record.container_entry_count_before, 4,
            "the line already recorded is the conservative one and is kept"
        );
        assert_eq!(ledger.tags.len(), 1);
    }

    /// Renaming twice must not launder a shipped tag into an authored one, and
    /// a stale record sitting at the destination must not be inherited either —
    /// that is the same laundering with an extra step.
    #[test]
    fn a_shipped_tag_stays_shipped_however_far_it_is_moved() {
        let mut ledger = CreatedTagLedger::default();
        let first = "Meteorite/Content/Tags/objects/once-biped.ubulk";
        let second = "Meteorite/Content/Tags/objects/twice-biped.ubulk";
        // A copy Baboon made once lived where the shipped tag is about to land.
        ledger.record(record(UTOC, second));
        ledger.record_rename(
            "Meteorite/Content/Tags/objects/shipped-biped.ubulk",
            renamed_to(first),
        );
        ledger.record_rename(first, renamed_to(second));

        let record = ledger.find(Path::new(UTOC), second).expect("recorded");
        assert_eq!(
            record.origin,
            CreatedTagOrigin::RenamedFromShipped,
            "the record already at the destination is dropped, not inherited"
        );
        assert_eq!(ledger.tags.len(), 1);
    }

    /// Every row written before this field existed was a duplicate, because a
    /// duplicate was the only thing the ledger could hold. `Authored` is the
    /// truth about those rows rather than a fallback — and it has to survive
    /// reading a file that predates the field, or every existing copy silently
    /// stops being deletable.
    #[test]
    fn a_ledger_file_with_no_origin_reads_as_authored() {
        let legacy = serde_json::json!({
            "version": 1,
            "tags": [{
                "utoc_path": UTOC,
                "chunk_label": "pakchunk240-WinGDK",
                "package_path": "/Game/Tags/objects/copy-biped",
                "package_id": 7,
                "uasset_path": "Meteorite/Content/Tags/objects/copy-biped.uasset",
                "ubulk_path": "Meteorite/Content/Tags/objects/copy-biped.ubulk",
                "display_path": "objects/copy.biped",
                "group_tag": 0,
                "source_display": "objects/original.biped",
                "container_entry_count_before": 4,
                "created_unix_secs": 1
            }]
        });
        let ledger = CreatedTagLedger::from_bytes(&serde_json::to_vec(&legacy).unwrap());
        assert!(ledger.load_error.is_none(), "a pre-origin ledger still reads");
        let record = ledger
            .find(
                Path::new(UTOC),
                "Meteorite/Content/Tags/objects/copy-biped.ubulk",
            )
            .expect("the row survived");
        assert_eq!(record.origin, CreatedTagOrigin::Authored);
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
            ledger
                .find(Path::new(utoc), ubulk)
                .unwrap()
                .created_unix_secs,
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

        fs::write(
            root.join("pakchunk0-Windows.utoc.baboon-duplicate-backup-2"),
            b"not a toc",
        )
        .unwrap();
        assert_eq!(container_original_entry_count(&utoc), Some(122_804));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_ledger_reads_as_empty_rather_than_failing_startup() {
        let ledger = CreatedTagLedger::from_bytes(b"{ not json");
        assert!(ledger.is_empty());
        assert!(ledger.load_error.is_some(), "and is remembered as unread");
    }

    fn scratch_ledger(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "baboon-ledger-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join(LEDGER_FILE)
    }

    /// A newer build may write an `origin` this one does not know. That used
    /// to fail the whole file's parse, so the ledger loaded empty and the next
    /// save (any duplicate, delete or rename) erased every record in it.
    #[test]
    fn an_unknown_origin_costs_nothing_and_survives_a_save() {
        let path = scratch_ledger("future-origin");
        let mut future = serde_json::to_value(record(UTOC, "future.ubulk")).unwrap();
        future["origin"] = serde_json::json!("ImportedFromMod");
        // A row whose shape this build cannot read at all is kept raw.
        let alien = serde_json::json!({ "utoc_path": 7, "shape": "from the future" });
        let file = serde_json::json!({
            "version": 2,
            "tags": [future, serde_json::to_value(record(UTOC, "known.ubulk")).unwrap(), alien],
        });
        fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

        let mut ledger = CreatedTagLedger::load_from(&path);
        let found = ledger.find(Path::new(UTOC), "future.ubulk").cloned();
        ledger.record(record(UTOC, "new.ubulk"));
        ledger.save_to(&path).unwrap();
        let reloaded: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let _ = fs::remove_dir_all(path.parent().unwrap());

        assert_eq!(
            found.map(|record| record.origin),
            Some(CreatedTagOrigin::Unrecognized("ImportedFromMod".to_owned()))
        );
        let rows = reloaded["tags"].as_array().unwrap();
        assert_eq!(rows.len(), 4, "{rows:#?}");
        assert!(rows.iter().any(|row| row["origin"] == "ImportedFromMod"));
        assert!(rows.contains(&alien));
        assert!(rows.iter().any(|row| row["ubulk_path"] == "known.ubulk"));
    }

    /// A file that cannot be parsed at all is never written over.
    #[test]
    fn a_ledger_that_failed_to_load_is_not_saved_over() {
        let path = scratch_ledger("truncated");
        let original = b"{ \"version\": 1, \"tags\": [ { \"utoc_path\": \"C:/";
        fs::write(&path, original).unwrap();

        let mut ledger = CreatedTagLedger::load_from(&path);
        ledger.record(record(UTOC, "new.ubulk"));
        let saved = ledger.save_to(&path);
        let on_disk = fs::read(&path).unwrap();
        let _ = fs::remove_dir_all(path.parent().unwrap());

        assert!(saved.is_err(), "the save is refused and says why");
        assert_eq!(on_disk, original, "the file is left exactly as it was");
    }
}
