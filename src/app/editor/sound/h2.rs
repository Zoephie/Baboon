//! Halo 2's inline, per-language sound audio.
//!
//! A permutation doesn't hold its samples. It holds an index into the tag's
//! `language permutation info` block, and each element there carries one entry
//! per language — each with its own `language`, its own `compression`, the
//! samples, mouth and lip-sync data, and the chunk table. English is the entry
//! every tag has; the rest vary by tag and by permutation.
//!
//! The tool writes every language at the tag's own sample rate (`import_sounds`
//! resamples or refuses), but the tags also carry legacy entries it didn't
//! write: Portuguese Xbox ADPCM, recorded at 22.05, 32 or 44.1 kHz under a
//! 48 kHz tag, with nothing in the tag recording which. Their rate is recovered
//! from the mouth data, which the engine samples at a fixed rate for every
//! language: against the tag's own-rate entries of the same permutation it
//! gives each legacy entry's duration, and so its rate. Across all 14,430 such
//! entries in the Halo 2 kit the estimate lands unambiguously on one of the
//! engine's four rates, and on the same one for every entry of a tag.

use super::*;
use super::super::audio::InlineCodec;

/// The engine's sample rates (`sound_definitions.h`), the only ones a tag can hold.
const H2_SAMPLE_RATES: [u32; 4] = [22_050, 32_000, 44_100, 48_000];

/// The language every Halo 2 sound carries, and the one a missing language
/// falls back to.
pub(in crate::app) const H2_DEFAULT_LANGUAGE: &str = "english";

/// One language's audio for one permutation.
pub(in crate::app) struct H2Entry {
    /// The `language` enum's option name, e.g. `english`.
    pub(in crate::app) language: String,
    /// The `compression` enum's option name, e.g. `opus`.
    pub(in crate::app) compression: String,
    pub(in crate::app) codec: InlineCodec,
    pub(in crate::app) sample_bytes: usize,
    mouth_bytes: usize,
    /// The engine's sample count (16-bit PCM bytes, per `sound_definitions.cpp`).
    stored_count: Option<i64>,
    lpi: usize,
    /// The entry's index in `raw info block`, or `None` for the older layout
    /// whose `language permutation info` element is itself the entry.
    raw: Option<usize>,
}

/// How an entry's sample rate was established.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum H2RateSource {
    /// The tag's `sample rate`, which the tool encodes every entry at.
    Tag,
    /// A legacy entry's rate, recovered from its mouth data.
    Inferred,
    /// A legacy entry whose rate couldn't be recovered; the tag's is assumed.
    Unknown,
}

/// A Halo 2 sound tag's per-language audio, read without copying any samples.
pub(in crate::app) struct H2Sound {
    pub(in crate::app) channels: u16,
    tag_rate: u32,
    tag_compression: String,
    /// `encoding` → the engine's samples-per-byte factor (mono 1, stereo ½,
    /// codec 1, quad ¼ — `sound_definitions.cpp`).
    encoding_factor: f64,
    legacy_rate: Option<u32>,
    groups: Vec<Vec<H2Entry>>,
    /// Languages present anywhere in the tag, in the schema's order.
    pub(in crate::app) languages: Vec<String>,
}

/// The `language permutation info` block of a Halo 2 sound, if it has one.
fn language_permutation_info<'a>(root: &TagStruct<'a>) -> Option<TagBlock<'a>> {
    root.fields().find_map(|field| {
        let block = field.as_block()?;
        (0..block.len()).find_map(|index| {
            find_block_field(&block.element(index)?, "language permutation info")
        })
    })
}

/// The struct holding one entry's fields: a `raw info block` element, or the
/// `language permutation info` element itself in the older layout.
fn entry_struct<'a>(lpi: &TagBlock<'a>, group: usize, raw: Option<usize>) -> Option<TagStruct<'a>> {
    let element = lpi.element(group)?;
    match raw {
        Some(raw) => find_block_field(&element, "raw info block")?.element(raw),
        None => Some(element),
    }
}

/// The codec a `compression` option name denotes.
pub(in crate::app) fn h2_codec_for(compression: &str) -> InlineCodec {
    let compression = compression.to_ascii_lowercase();
    if compression.contains("opus") {
        InlineCodec::Opus
    } else if compression.contains("none") {
        // "none (big endian)" / "none (little endian)".
        InlineCodec::Pcm {
            big_endian: compression.contains("big"),
        }
    } else {
        InlineCodec::XboxAdpcm
    }
}

/// Channel count for an `encoding` option name, matched by name because the
/// option order differs between games (H2: mono, stereo, codec, quad).
pub(in crate::app) fn h2_channels_for(encoding: &str) -> u16 {
    let encoding = encoding.to_ascii_lowercase();
    if encoding.contains("mono") {
        1
    } else if encoding.contains("5.1") {
        6
    } else if encoding.contains("quad") {
        4
    } else {
        2 // stereo, codec
    }
}

/// Hz for a `sample rate` option name (`22kHz`, `44kHz`, `32kHz`, `48kHz`).
pub(in crate::app) fn h2_rate_for(sample_rate: &str) -> u32 {
    if sample_rate.contains("22") {
        22_050
    } else if sample_rate.contains("32") {
        32_000
    } else if sample_rate.contains("44") {
        44_100
    } else {
        48_000
    }
}

fn read_enum_clean(element: &TagStruct, clean: &str) -> Option<String> {
    find_full_field_name(element, clean).and_then(|full| element.read_enum_name(full))
}

/// The `language` enum's position in the schema, to order languages the way
/// the tool lists them.
fn language_order(element: &TagStruct) -> i64 {
    find_full_field_name(element, "language")
        .and_then(|full| element.field(full))
        .and_then(|field| field.value())
        .and_then(|value| match value {
            blam_tags::TagFieldData::CharEnum { value, .. } => Some(i64::from(value)),
            blam_tags::TagFieldData::ShortEnum { value, .. } => Some(i64::from(value)),
            _ => None,
        })
        .unwrap_or(i64::MAX)
}

/// The engine's sample count: the entry's last direct `long integer`, which
/// `sound_definitions.cpp` turns into a duration.
fn stored_count(element: &TagStruct) -> Option<i64> {
    // Filter on the type first: `value()` on the samples would copy them.
    element
        .fields()
        .filter(|field| field.field_type() == TagFieldType::LongInteger)
        .filter_map(|field| match field.value() {
            Some(blam_tags::TagFieldData::LongInteger(value)) => Some(i64::from(value)),
            _ => None,
        })
        .last()
        .filter(|&count| count > 0)
}

fn first_data_len(element: &TagStruct, nth: usize) -> usize {
    element
        .fields()
        .filter_map(|field| field.as_data())
        .nth(nth)
        .map_or(0, <[u8]>::len)
}

impl H2Sound {
    /// Read a Halo 2 sound's per-language audio, or `None` when the tag has no
    /// `language permutation info` block (other games, older Halo 2 layouts).
    pub(in crate::app) fn read(tag: &TagFile) -> Option<Self> {
        let root = tag.root();
        let lpi = language_permutation_info(&root)?;
        let tag_compression = read_enum_clean(&root, "compression").unwrap_or_default();
        let encoding = read_enum_clean(&root, "encoding").unwrap_or_default();
        let channels = h2_channels_for(&encoding);
        let encoding_factor = match channels {
            1 => 1.0,
            4 => 0.25,
            _ if encoding.to_ascii_lowercase().contains("codec") => 1.0,
            _ => 0.5,
        };
        let tag_rate = read_enum_clean(&root, "sample rate").map_or(48_000, |name| h2_rate_for(&name));

        let mut groups = Vec::with_capacity(lpi.len());
        let mut languages: Vec<(i64, String)> = Vec::new();
        for group in 0..lpi.len() {
            let Some(element) = lpi.element(group) else {
                groups.push(Vec::new());
                continue;
            };
            let raws: Vec<Option<usize>> = match find_block_field(&element, "raw info block") {
                Some(raw) => (0..raw.len()).map(Some).collect(),
                None => vec![None],
            };
            let mut entries = Vec::with_capacity(raws.len());
            for raw in raws {
                let Some(entry) = entry_struct(&lpi, group, raw) else {
                    continue;
                };
                let language = read_enum_clean(&entry, "language")
                    .unwrap_or_else(|| H2_DEFAULT_LANGUAGE.to_owned());
                let compression =
                    read_enum_clean(&entry, "compression").unwrap_or_else(|| tag_compression.clone());
                let sample_bytes = first_data_len(&entry, 0);
                if sample_bytes == 0 {
                    continue; // a language slot with no audio in it
                }
                if !languages.iter().any(|(_, name)| *name == language) {
                    languages.push((language_order(&entry), language.clone()));
                }
                entries.push(H2Entry {
                    codec: h2_codec_for(&compression),
                    language,
                    compression,
                    sample_bytes,
                    mouth_bytes: first_data_len(&entry, 1),
                    stored_count: stored_count(&entry),
                    lpi: group,
                    raw,
                });
            }
            groups.push(entries);
        }
        languages.sort();
        let mut sound = Self {
            channels,
            tag_rate,
            tag_compression,
            encoding_factor,
            legacy_rate: None,
            groups,
            languages: languages.into_iter().map(|(_, name)| name).collect(),
        };
        sound.legacy_rate = sound.infer_legacy_rate();
        Some(sound)
    }

    /// Whether `entry` was encoded by something other than the tool, so the
    /// tag's rate isn't its own.
    fn is_legacy(&self, entry: &H2Entry) -> bool {
        !entry.compression.eq_ignore_ascii_case(&self.tag_compression)
    }

    fn frames(&self, entry: &H2Entry) -> Option<f64> {
        entry
            .stored_count
            .map(|count| count as f64 * 0.5 * self.encoding_factor)
    }

    /// The legacy entries' shared rate: each entry's mouth-data duration,
    /// measured against its permutation's tag-rate entries, snapped to the
    /// nearest engine rate; the tag's majority wins.
    fn infer_legacy_rate(&self) -> Option<u32> {
        let mut votes = [0usize; H2_SAMPLE_RATES.len()];
        for entries in &self.groups {
            let reference: Vec<f64> = entries
                .iter()
                .filter(|entry| !self.is_legacy(entry) && entry.mouth_bytes > 0)
                .filter_map(|entry| {
                    let seconds = self.frames(entry)? / f64::from(self.tag_rate);
                    (seconds > 0.0).then(|| entry.mouth_bytes as f64 / seconds)
                })
                .collect();
            if reference.is_empty() {
                continue;
            }
            let mouth_per_second = reference.iter().sum::<f64>() / reference.len() as f64;
            for entry in entries.iter().filter(|entry| self.is_legacy(entry)) {
                let Some(frames) = self.frames(entry) else {
                    continue;
                };
                if entry.mouth_bytes == 0 {
                    continue;
                }
                let implied = frames * mouth_per_second / entry.mouth_bytes as f64;
                let nearest = H2_SAMPLE_RATES
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let da = (implied / f64::from(**a)).ln().abs();
                        let db = (implied / f64::from(**b)).ln().abs();
                        da.total_cmp(&db)
                    })
                    .map(|(index, _)| index);
                if let Some(index) = nearest {
                    votes[index] += 1;
                }
            }
        }
        let (index, count) = votes.iter().enumerate().max_by_key(|(_, count)| **count)?;
        (*count > 0).then_some(H2_SAMPLE_RATES[index])
    }

    /// The sample rate `entry` plays at, and how it was established.
    pub(in crate::app) fn rate_of(&self, entry: &H2Entry) -> (u32, H2RateSource) {
        if !self.is_legacy(entry) {
            return (self.tag_rate, H2RateSource::Tag);
        }
        match self.legacy_rate {
            Some(rate) => (rate, H2RateSource::Inferred),
            None => (self.tag_rate, H2RateSource::Unknown),
        }
    }

    /// Playback length, from the engine's own sample count — or, in the older
    /// `raw info block` that has none, from the size of a fixed-rate codec.
    pub(in crate::app) fn duration_secs(&self, entry: &H2Entry) -> Option<f64> {
        let (rate, _) = self.rate_of(entry);
        match self.frames(entry) {
            Some(frames) => Some(frames / f64::from(rate)),
            None => fixed_rate_duration(entry.codec, entry.sample_bytes, self.channels, rate),
        }
    }

    /// Every language entry of one `language permutation info` element.
    pub(in crate::app) fn entries(&self, lpi: usize) -> &[H2Entry] {
        self.groups.get(lpi).map_or(&[], Vec::as_slice)
    }

    /// The entry to play for `language` (`None` = English), falling back to
    /// English, then to whatever the permutation has. The flag is true when
    /// the entry isn't the language asked for.
    pub(in crate::app) fn entry_for(
        &self,
        lpi: usize,
        language: Option<&str>,
    ) -> Option<(&H2Entry, bool)> {
        let entries = self.entries(lpi);
        let wanted = language.unwrap_or(H2_DEFAULT_LANGUAGE);
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.language.eq_ignore_ascii_case(wanted))
        {
            return Some((entry, false));
        }
        entries
            .iter()
            .find(|entry| entry.language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE))
            .or_else(|| entries.first())
            .map(|entry| (entry, true))
    }

    /// Whether the tag carries `language` at all.
    pub(in crate::app) fn has_language(&self, language: &str) -> bool {
        self.languages
            .iter()
            .any(|name| name.eq_ignore_ascii_case(language))
    }

    /// The samples and chunk offsets of `entry`, copied out of the tag.
    pub(in crate::app) fn samples(&self, tag: &TagFile, entry: &H2Entry) -> Option<(Vec<u8>, Vec<usize>)> {
        let root = tag.root();
        let lpi = language_permutation_info(&root)?;
        let element = entry_struct(&lpi, entry.lpi, entry.raw)?;
        let bytes = element
            .fields()
            .find_map(|field| field.as_data().filter(|data| !data.is_empty()))?
            .to_vec();
        Some((bytes, chunk_offsets_of(&element)))
    }

    /// `(codec, channels, sample rate)` for decoding `entry`.
    pub(in crate::app) fn decode_params(&self, entry: &H2Entry) -> (InlineCodec, u16, u32) {
        (entry.codec, self.channels, self.rate_of(entry).0)
    }
}

/// A language's display name: `english` → `English`.
pub(in crate::app) fn language_label(language: &str) -> String {
    let mut chars = language.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `48000` → `48 kHz`, `22050` → `22.05 kHz`.
pub(in crate::app) fn format_rate(rate: u32) -> String {
    if rate % 1000 == 0 {
        format!("{} kHz", rate / 1000)
    } else {
        let khz = format!("{:.2}", f64::from(rate) / 1000.0);
        format!("{} kHz", khz.trim_end_matches('0').trim_end_matches('.'))
    }
}

/// A byte count as `5.1 KB` / `1.2 MB`.
pub(in crate::app) fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}
