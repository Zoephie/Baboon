//! Source listings: every map id, sounds by class, uncompressed sounds.

use super::*;

impl Baboon {


    /// List every scenario's map id (and name, where it has one).
    pub(in crate::app) fn show_map_ids(&mut self, ctx: &egui::Context) {
        self.show_source_listing(SourceListing::MapIds, ctx);
    }

    /// List every `snd!` tag annotated with its sound class + compression, with a
    /// per-class count summary (mirrors `count-class-sounds` /
    /// `count-all-class-sounds`).
    pub(in crate::app) fn show_sounds_by_class(&mut self, ctx: &egui::Context) {
        self.show_source_listing(SourceListing::SoundsByClass, ctx);
    }

    /// List `snd!` tags stored uncompressed (compression name contains "none"),
    /// mirroring `dump-uncompressed-sounds`.
    pub(in crate::app) fn show_uncompressed_sounds(&mut self, ctx: &egui::Context) {
        self.show_source_listing(SourceListing::UncompressedSounds, ctx);
    }

    /// Run a whole-source listing on a worker and show it when it lands.
    ///
    /// These read every scenario or sound tag in the source, which for sounds
    /// is thousands of full tag parses, and they used to do it on the UI
    /// thread. The results window says what it is reading meanwhile.
    pub(in crate::app) fn show_source_listing(&mut self, listing: SourceListing, ctx: &egui::Context) {
        let kit = self.model.active_kit_id();
        let entries = match self.model.listing_entries() {
            Ok(entries) => entries.to_vec(),
            Err(note) => {
                self.search.query_results = Some(TagQueryResults {
                    kit,
                    title: listing.title().to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(note),
                    ref_target: None,
                });
                return;
            }
        };
        let Some(source) = self.model.source().map(|source| source.source.clone()) else {
            return;
        };
        let wanted = listing.group();
        let count = entries
            .iter()
            .filter(|entry| entry.group_tag.to_be_bytes() == *wanted)
            .count();
        self.search.query_results = Some(TagQueryResults {
            kit,
            title: listing.title().to_owned(),
            entries: Vec::new(),
            annotations: Vec::new(),
            note: Some(format!("Reading {count} tag(s)…")),
            ref_target: None,
        });
        let stamp = self.model.kit_stamp();
        spawn_worker(
            &self.tx,
            ctx,
            move || WorkerMessage::SourceListingReady {
                stamp,
                results: build_source_listing(listing, &source, &entries, stamp.kit),
            },
            move |error| WorkerMessage::SourceListingReady {
                stamp,
                results: TagQueryResults {
                    kit: stamp.kit,
                    title: listing.title().to_owned(),
                    entries: Vec::new(),
                    annotations: Vec::new(),
                    note: Some(error),
                    ref_target: None,
                },
            },
        );
    }

    /// Applies `WorkerMessage::SourceListingReady` if its kit still has the
    /// source it was read from.
    pub(in crate::app) fn handle_source_listing_ready(
        &mut self,
        stamp: KitStamp,
        results: TagQueryResults,
    ) -> bool {
        if self.model.resolve_stamp(stamp).is_none() {
            return true;
        }
        self.search.query_results = Some(results);
        false
    }
}

/// The whole-source listings in the Tools menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum SourceListing {
    MapIds,
    SoundsByClass,
    UncompressedSounds,
}

impl SourceListing {
    fn title(self) -> &'static str {
        match self {
            Self::MapIds => "Scenario map IDs",
            Self::SoundsByClass => "Sounds by class",
            Self::UncompressedSounds => "Uncompressed sounds",
        }
    }

    fn group(self) -> &'static [u8; 4] {
        match self {
            Self::MapIds => b"scnr",
            Self::SoundsByClass | Self::UncompressedSounds => b"snd!",
        }
    }
}

/// Read every tag a listing is about and build its results. Runs on a worker.
pub(in crate::app) fn build_source_listing(
    listing: SourceListing,
    source: &TagSource,
    entries: &[TagEntry],
    kit: KitId,
) -> TagQueryResults {
    let (entries, annotations, note) = match listing {
        SourceListing::MapIds => listing_map_ids(source, entries),
        SourceListing::SoundsByClass => listing_sounds_by_class(scan_sound_tags(source, entries)),
        SourceListing::UncompressedSounds => {
            listing_uncompressed_sounds(scan_sound_tags(source, entries))
        }
    };
    TagQueryResults {
        kit,
        title: format!("{} ({})", listing.title(), entries.len()),
        entries,
        annotations,
        note,
        ref_target: None,
    }
}

type ListingRows = (Vec<TagEntry>, Vec<String>, Option<String>);

pub(in crate::app) fn listing_map_ids(source: &TagSource, listed: &[TagEntry]) -> ListingRows {
    let mut entries = Vec::new();
    let mut annotations = Vec::new();
    for entry in listed {
        if &entry.group_tag.to_be_bytes() != b"scnr" {
            continue;
        }
        let Ok(tag) = crate::core::source::read_entry(source, entry) else {
            continue;
        };
        let root = tag.root();
        if let Some(id) = root.read_int_any("map id") {
            // `map name` carries a `#tooltip` suffix in Reach/H4, so resolve
            // it via the cleaned-name lookup rather than an exact match.
            let name = find_full_field_name(&root, "map name")
                .and_then(|full| root.read_string_id(full))
                .unwrap_or_default();
            annotations.push(if name.is_empty() {
                format!("map id {id}")
            } else {
                format!("map id {id}  ({name})")
            });
            entries.push(entry.clone());
        }
    }
    let note = entries.is_empty().then(|| {
        "No scenario map IDs found (scnr tags only; classic Halo 2 stores them elsewhere)."
            .to_owned()
    });
    (entries, annotations, note)
}

/// Every `snd!` tag's `class` and `compression` enum names, as
/// `(class, compression, entry)`. Shared by both sound listings.
fn scan_sound_tags(source: &TagSource, listed: &[TagEntry]) -> Vec<(String, String, TagEntry)> {
    let mut rows = Vec::new();
    for entry in listed {
        if &entry.group_tag.to_be_bytes() != b"snd!" {
            continue;
        }
        let Ok(tag) = crate::core::source::read_entry(source, entry) else {
            continue;
        };
        let root = tag.root();
        let class = find_full_field_name(&root, "class")
            .and_then(|full| root.read_enum_name(full))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(none)".to_owned());
        let compression = find_full_field_name(&root, "compression")
            .and_then(|full| root.read_enum_name(full))
            .unwrap_or_default();
        rows.push((class, compression, entry.clone()));
    }
    rows
}

pub(in crate::app) fn listing_sounds_by_class(mut rows: Vec<(String, String, TagEntry)>) -> ListingRows {
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.2.display_path.cmp(&b.2.display_path))
    });
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (class, _, _) in &rows {
        *counts.entry(class.as_str()).or_default() += 1;
    }
    let entries: Vec<TagEntry> = rows.iter().map(|(_, _, e)| e.clone()).collect();
    let annotations: Vec<String> = rows
        .iter()
        .map(|(class, comp, _)| {
            if comp.is_empty() {
                format!("[{class}]")
            } else {
                format!("[{class}] {comp}")
            }
        })
        .collect();
    let note = if entries.is_empty() {
        Some("No sound tags found.".to_owned())
    } else {
        let summary = counts
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("{} class(es) \u{2014} {summary}", counts.len()))
    };
    (entries, annotations, note)
}

pub(in crate::app) fn listing_uncompressed_sounds(rows: Vec<(String, String, TagEntry)>) -> ListingRows {
    let mut hits: Vec<(String, String, TagEntry)> = rows
        .into_iter()
        .filter(|(_, comp, _)| comp.to_ascii_lowercase().contains("none"))
        .collect();
    hits.sort_by(|a, b| a.2.display_path.cmp(&b.2.display_path));
    let entries: Vec<TagEntry> = hits.iter().map(|(_, _, e)| e.clone()).collect();
    let annotations: Vec<String> = hits
        .iter()
        .map(|(class, comp, _)| format!("{comp}  [{class}]"))
        .collect();
    let note = entries
        .is_empty()
        .then(|| "No uncompressed sound tags found.".to_owned());
    (entries, annotations, note)
}

impl Model {
    /// Scan every scenario (`scnr`) tag and list its map id (+ map name where
    /// present). Reads `map id` at the scenario root, which covers the modern
    /// engines (H2A/H3/ODST/Reach/H4); classic Halo 2 stores it elsewhere.
    /// Every tag a whole-source listing should walk, or why it cannot yet.
    ///
    /// A container mount enumerates every tag up front, into `entries`, and
    /// leaves `all_entries` empty; a loose folder only has them all once its
    /// background scan is done. The listings read `all_entries` directly, so
    /// on a container they walked nothing, and on a folder mid-scan they
    /// walked nothing too, and both said "none found".
    pub(in crate::app) fn listing_entries(&self) -> Result<&[TagEntry], String> {
        let source = self
            .source()
            .ok_or_else(|| "No source loaded.".to_owned())?;
        if matches!(source.source, TagSource::LooseFolder { .. }) && source.all_entries.is_empty() {
            return Err(
                "The tag index is still being built; try again once indexing finishes.".to_owned(),
            );
        }
        Ok(source.full_entry_set())
    }
}
