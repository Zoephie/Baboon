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
        crate::core::storage::data_path(LEDGER_FILE)
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
mod tests;
