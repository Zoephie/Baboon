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

use super::super::audio::InlineCodec;
use super::*;

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
        (0..block.len())
            .find_map(|index| find_block_field(&block.element(index)?, "language permutation info"))
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
    } else if compression.contains("ogg") {
        // Halo CE music.
        InlineCodec::OggVorbis
    } else if compression.contains("none") {
        // "none (big endian)" / "none (little endian)"; CE's plain "none" is
        // little-endian.
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
        let tag_rate =
            read_enum_clean(&root, "sample rate").map_or(48_000, |name| h2_rate_for(&name));

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
                let compression = read_enum_clean(&entry, "compression")
                    .unwrap_or_else(|| tag_compression.clone());
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
        !entry
            .compression
            .eq_ignore_ascii_case(&self.tag_compression)
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
    pub(in crate::app) fn samples(
        &self,
        tag: &TagFile,
        entry: &H2Entry,
    ) -> Option<(Vec<u8>, Vec<usize>)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// Read a Halo 2 kit sound, or `None` (with a skip message naming the
    /// variable) when `BLAM_TEST_H2EK` isn't set.
    fn h2_kit_sound(rel: &str) -> Option<TagFile> {
        let defs = crate::core::test_kits::definitions();
        let tag_path = crate::core::test_kits::h2ek_tags().join(rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: set BLAM_TEST_H2EK to a Halo 2 kit's tags ({})", tag_path.display());
            return None;
        }
        let group = u32::from_be_bytes(*b"snd!");
        Some(
            crate::core::source::read_tag_at_path(&tag_path, Some(GameId::Halo2), Some(defs), group)
                .expect("read H2 sound tag"),
        )
    }

    /// Whole-tag H2 extraction end-to-end (skip-if-absent): build the same rows
    /// the player builds and run the real `AudioState::run_extract` for one
    /// language, validating a WAV per permutation at that language's length.
    #[test]
    fn h2_extract_writes_one_wav_per_permutation_in_the_chosen_language() {
        let Some(tag) = h2_kit_sound("sound/dialog/combat/elite_dogmatic/01_alert/seefoe.sound")
        else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let rows = sound_permutation_rows(&tag, Some(&h2));
        let dir = crate::core::test_kits::unique_temp_dir("h2_extract_german");
        let source = RowSource {
            h2: Some(&h2),
            language: Some("german"),
            sound_rel: None,
            multi_pr: false,
        };
        let items = build_extract_items(&tag, &rows, source, &dir, false);
        assert_eq!(items.len(), rows.len(), "every permutation has German");
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: None,
                label: "h2".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        for row in &rows {
            let RowKind::InlineH2 { lpi } = row.kind else {
                panic!("{} should be an H2 language row", row.name);
            };
            let (entry, fallback) = h2.entry_for(lpi, Some("german")).unwrap();
            assert!(!fallback);
            let wav = std::fs::read(dir.join(format!("{}.wav", row.name))).expect("wav written");
            assert_eq!(&wav[0..4], b"RIFF");
            // 16-bit mono: the data chunk is 2 bytes a frame.
            let frames = (wav.len() - 44) / 2;
            let expected = (h2.duration_secs(entry).unwrap() * 48_000.0).round();
            assert_eq!(frames as f64, expected, "{}: German length", row.name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Each permutation plays its own audio in the chosen language. Rows used to
    /// take the Nth blob of a flat list of every language of every permutation,
    /// so `seefoe`'s four rows played permutation `1` in English, Portuguese,
    /// German and French.
    #[test]
    fn h2_rows_play_their_own_permutation_in_the_chosen_language() {
        let Some(tag) = h2_kit_sound("sound/dialog/combat/elite_dogmatic/01_alert/seefoe.sound")
        else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        assert_eq!(
            h2.languages,
            [
                "english",
                "japanese",
                "german",
                "french",
                "spanish",
                "italian",
                "korean",
                "chinese",
                "portuguese"
            ]
        );
        let rows = sound_permutation_rows(&tag, Some(&h2));
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["1", "2", "4", "5"]);
        for (ordinal, row) in rows.iter().enumerate() {
            let RowKind::InlineH2 { lpi } = row.kind else {
                panic!("{} should be an H2 language row", row.name);
            };
            assert_eq!(lpi, ordinal, "{} keeps its own entry", row.name);
            for language in [None, Some("german"), Some("portuguese")] {
                let (entry, fallback) = h2.entry_for(lpi, language).unwrap();
                assert!(!fallback);
                assert_eq!(entry.language, language.unwrap_or("english"));
            }
        }
        // English perm `1` is 0.65 s: 62,400 bytes of 16-bit mono at 48 kHz,
        // the engine's own duration rule.
        let (english, _) = h2.entry_for(0, None).unwrap();
        assert_eq!(h2.duration_secs(english), Some(0.65));
    }

    /// Render the sound player for `tag` with `language` chosen and the clip
    /// `selected` (a clip id) picked, returning every piece of text it paints.
    fn painted_sound_player(
        tag: &TagFile,
        language: Option<&str>,
        selected: Option<&str>,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        if let Some(id) = selected {
            ctx.data_mut(|data| {
                data.insert_temp(clip_selection_id("sound", "test"), id.to_owned())
            });
        }
        let mut painted = Vec::new();
        for _ in 0..2 {
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut sinks = EditSinks::default();
                        let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
                        edit.game = Some(GameId::Halo2);
                        edit.sound_language = language;
                        draw_sound_player(ui, tag, &mut edit);
                    });
                },
            );
            painted = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        painted
    }

    /// The clip id the player gives a permutation row.
    fn clip_id(row: &SoundPermRow, language: Option<&str>) -> String {
        format!(
            "{}:{}:{}",
            row.pr_index,
            row.perm_index,
            language.unwrap_or("")
        )
    }

    /// The player shows what each permutation is — its name and length in the
    /// chosen language, one at a time as it is picked — instead of the pitch
    /// range `|default|`, offers the tag's languages, and describes what that
    /// language plays.
    #[test]
    fn h2_sound_player_shows_lengths_languages_and_the_chosen_format() {
        let Some(tag) = h2_kit_sound("sound/dialog/combat/elite_dogmatic/01_alert/seefoe.sound")
        else {
            return;
        };
        let has = |painted: &[String], text: &str| painted.iter().any(|shown| shown == text);
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let rows = sound_permutation_rows(&tag, Some(&h2));
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["1", "2", "4", "5"]);

        let english = painted_sound_player(&tag, None, None);
        for text in [
            "class: unit_dialog",
            "1",
            "0:00.000 / 0:00.650",
            "\u{1F310} English",
            "opus \u{00B7} mono \u{00B7} 48 kHz",
        ] {
            assert!(has(&english, text), "missing {text:?} in {english:?}");
        }
        assert!(has(&english, "\u{2B07} Extract all (English)"));
        assert!(has(&english, "\u{2B07} All languages"));
        // The third extract button is the selected permutation's own.
        assert!(has(&english, "\u{2B07} 1"), "{english:?}");
        assert!(
            !english.iter().any(|shown| shown.contains("|default|")),
            "a lone default pitch range isn't shown: {english:?}"
        );
        // Picking each permutation shows it, with its own length.
        for row in &rows {
            let painted = painted_sound_player(&tag, None, Some(&clip_id(row, None)));
            let (entry, _) = h2.entry_for(lpi_of(row), None).unwrap();
            let length = format!(
                "0:00.000 / {}",
                format_play_time(h2.duration_secs(entry).unwrap())
            );
            assert!(
                has(&painted, &row.name),
                "{} not shown: {painted:?}",
                row.name
            );
            assert!(
                has(&painted, &length),
                "{} lacks {length:?}: {painted:?}",
                row.name
            );
        }

        let portuguese = painted_sound_player(&tag, Some("portuguese"), None);
        assert!(
            has(&portuguese, "xbox adpcm \u{00B7} mono \u{00B7} 22.05 kHz"),
            "{portuguese:?}"
        );
        assert!(has(&portuguese, "(rate inferred)"));
        assert!(has(&portuguese, "\u{2B07} Extract all (Portuguese)"));

        // Another game's language this tag never had: say so, play English.
        let mexican = painted_sound_player(&tag, Some("mexican"), None);
        assert!(
            has(
                &mexican,
                "Mexican isn't in this tag \u{2014} playing English."
            ),
            "{mexican:?}"
        );
        assert!(has(&mexican, "\u{1F310} English"));
    }

    fn lpi_of(row: &SoundPermRow) -> usize {
        match row.kind {
            RowKind::InlineH2 { lpi } => lpi,
            _ => panic!("{} is not a Halo 2 language row", row.name),
        }
    }

    /// A permutation missing the chosen language says so and plays English,
    /// but a bulk extract leaves it out rather than filing English audio
    /// under `data_japanese`.
    #[test]
    fn h2_permutation_missing_the_language_falls_back_and_is_not_extracted() {
        let Some(tag) = h2_kit_sound("sound/dialog/combat/elite_loose/16_taunt/tnt_elt.sound")
        else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let rows = sound_permutation_rows(&tag, Some(&h2));
        assert_eq!(rows.len(), 3);
        // Shown for the permutation that lacks it, when that one is picked.
        let notes: Vec<bool> = rows
            .iter()
            .map(|row| {
                painted_sound_player(
                    &tag,
                    Some("japanese"),
                    Some(&clip_id(row, Some("japanese"))),
                )
                .iter()
                .any(|shown| shown == "no Japanese \u{00B7} plays English")
            })
            .collect();
        assert_eq!(notes.iter().filter(|note| **note).count(), 1, "{notes:?}");

        let source = RowSource {
            h2: Some(&h2),
            language: Some("japanese"),
            sound_rel: None,
            multi_pr: false,
        };
        let base = std::path::Path::new("data_japanese");
        assert_eq!(build_extract_items(&tag, &rows, source, base, false).len(), 2);
    }

    /// Portuguese is legacy Xbox ADPCM under an Opus tag. It used to go to the
    /// Opus decoder ("no opus packets decoded"); it decodes as ADPCM at the
    /// rate its mouth data implies, not the tag's 48 kHz.
    #[test]
    fn h2_legacy_language_decodes_with_its_own_codec_and_inferred_rate() {
        use super::audio::InlineCodec;
        let Some(tag) = h2_kit_sound("sound/dialog/combat/elite_dogmatic/01_alert/seefoe.sound")
        else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let (entry, _) = h2.entry_for(0, Some("portuguese")).unwrap();
        assert!(matches!(entry.codec, InlineCodec::XboxAdpcm));
        assert_eq!(h2.rate_of(entry), (22_050, H2RateSource::Inferred));
        let (english, _) = h2.entry_for(0, None).unwrap();
        assert_eq!(h2.rate_of(english), (48_000, H2RateSource::Tag));

        let (bytes, offsets) = h2.samples(&tag, entry).unwrap();
        let (codec, channels, rate) = h2.decode_params(entry);
        let pcm = super::audio::decode_inline_chunked(codec, &bytes, &offsets, channels, rate)
            .expect("portuguese decodes");
        assert_eq!(pcm.sample_rate, 22_050);
        let seconds = pcm.frame_count() as f64 / 22_050.0;
        assert!((seconds - h2.duration_secs(entry).unwrap()).abs() < 1e-9);
    }

    /// A permutation's `language permutation info` index isn't its position:
    /// in `marine_jump` the first permutation's audio is entry 6.
    #[test]
    fn h2_permutation_uses_its_stored_index_not_its_position() {
        let Some(tag) = h2_kit_sound("sound/characters/marines/marine_jump.sound") else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let rows = sound_permutation_rows(&tag, Some(&h2));
        assert!(matches!(rows[0].kind, RowKind::InlineH2 { lpi: 6 }));
        assert!(matches!(rows[1].kind, RowKind::InlineH2 { lpi: 0 }));
    }

    /// Halo 2's older layout keeps samples on the permutation, like CE, but names
    /// the codec `compression`. Read through CE's `format`, its Xbox ADPCM was
    /// decoded as PCM noise.
    #[test]
    fn h2_older_layout_reads_compression_not_ce_format() {
        use super::audio::InlineCodec;
        let Some(tag) = h2_kit_sound("sound/characters/footsteps/grunt/dirt/jump_up.sound") else {
            return;
        };
        assert!(H2Sound::read(&tag).is_none(), "no language entries in this layout");
        let rows = sound_permutation_rows(&tag, None);
        assert_eq!(rows.len(), 6);
        for row in &rows {
            assert!(
                matches!(
                    row.kind,
                    RowKind::InlinePermutation {
                        codec: InlineCodec::XboxAdpcm,
                        channels: 1,
                        sample_rate: 44_100,
                    }
                ),
                "{}",
                row.name
            );
        }
    }

    /// Every Halo 2 sound's every language entry decodes, to exactly the length
    /// the engine computes from the tag (`sound_definitions.cpp`: count × ½ ×
    /// encoding factor ÷ rate). This is the gate on the whole model: the entry
    /// choice, the per-entry codec, the inferred legacy rate, and the Opus
    /// decoder's negated last packet all have to be right for the lengths to
    /// agree. Long: run with `--ignored` in release.
    #[test]
    #[ignore]
    fn h2_every_language_entry_decodes_to_the_engine_length() {
        let root = crate::core::test_kits::h2ek_tags();
        if !root.exists() {
            eprintln!("skip: set BLAM_TEST_H2EK");
            return;
        }
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "sound") {
                    files.push(path);
                }
            }
        }
        let total = files.len();
        let (mut tags_with_entries, mut entries, mut unknown_length) = (0usize, 0usize, 0usize);
        let mut failures = Vec::new();
        for path in &files {
            let rel = path.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
            let tag = h2_kit_sound(&rel).unwrap();
            let Some(h2) = H2Sound::read(&tag) else {
                continue;
            };
            tags_with_entries += 1;
            let rows = sound_permutation_rows(&tag, Some(&h2));
            for row in &rows {
                let RowKind::InlineH2 { lpi } = row.kind else {
                    continue;
                };
                for entry in h2.entries(lpi) {
                    entries += 1;
                    let (bytes, offsets) = h2.samples(&tag, entry).unwrap();
                    let (codec, channels, rate) = h2.decode_params(entry);
                    let decoded = match super::audio::decode_inline_chunked(
                        codec, &bytes, &offsets, channels, rate,
                    ) {
                        Ok(pcm) => pcm.frame_count() as f64,
                        Err(error) => {
                            failures.push(format!("{rel} {} {}: {error}", row.name, entry.language));
                            continue;
                        }
                    };
                    let Some(expected) = h2.duration_secs(entry).map(|s| s * f64::from(rate)) else {
                        unknown_length += 1;
                        continue;
                    };
                    if (decoded - expected).abs() > 0.5 {
                        failures.push(format!(
                            "{rel} {} {}: decoded {decoded} frames, engine says {expected}",
                            row.name, entry.language
                        ));
                    }
                }
            }
        }
        eprintln!(
            "{total} sound tags, {tags_with_entries} with language entries, {entries} entries \
             decoded, {unknown_length} without a stored length, {} failures",
            failures.len()
        );
        for failure in failures.iter().take(40) {
            eprintln!("  {failure}");
        }
        assert!(failures.is_empty());
    }

    /// The tag issue #96 reported: cinematic music clicked every 65,520
    /// samples, at each chunk boundary, played or extracted. Every boundary has
    /// to be as smooth as the music around it.
    #[test]
    fn cinematic_music_has_no_click_at_its_chunk_boundaries() {
        let Some(tag) =
            h2_kit_sound("sound/cinematics/01_spacestation/c01_outro/music/c01_outro_01_mus.sound")
        else {
            return;
        };
        let h2 = H2Sound::read(&tag).expect("H2 language entries");
        let (entry, _) = h2.entry_for(0, None).unwrap();
        let (bytes, offsets) = h2.samples(&tag, entry).unwrap();
        assert!(offsets.len() > 1, "the entry is chunked");
        let (codec, channels, rate) = h2.decode_params(entry);
        let pcm = super::audio::decode_inline_chunked(codec, &bytes, &offsets, channels, rate).unwrap();

        let ch = usize::from(pcm.channels);
        let l = |i: usize| f64::from(pcm.samples[i * ch]);
        let d2 = |i: usize| (l(i) - 2.0 * l(i - 1) + l(i - 2)).abs();
        // Every chunk of this entry holds 65,520 frames.
        let mut cut = 65_520;
        let mut checked = 0;
        while cut + 3 < pcm.frame_count() {
            let local: f64 = (cut - 200..cut - 3).map(d2).sum::<f64>() / 197.0;
            let jump = d2(cut) / (local + 1.0);
            assert!(jump < 8.0, "a click at frame {cut}: {jump:.1}x the music around it");
            cut += 65_520;
            checked += 1;
        }
        assert!(checked > 50, "only {checked} boundaries checked");
    }
}
