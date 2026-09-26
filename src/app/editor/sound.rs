//! Sound, dialogue, looping-sound, and material-effects presentation.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

mod h2;
pub(in crate::app) use h2::*;

/// The `sound_classes` (`sncl`) tag group.
pub(in crate::app) fn is_sound_classes_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"sncl"
}

/// Cross-game-normalized overview of a `sound_classes` tag: one row per sound
/// class with its near/far distance and a detail column, reading whichever
/// distance layout the game uses (classic H2/H3/ODST keep `distance bounds`
/// directly on the entry; Reach/H4/H2A nest them under `distance parameters`).
/// Read-only — the full editable field tree still renders below.
pub(in crate::app) fn draw_sound_classes_summary(ui: &mut Ui, tag: &TagFile) {
    let Some(classes) = tag
        .root()
        .field("sound classes")
        .and_then(|field| field.as_block())
    else {
        return;
    };
    let count = classes.len();
    egui::CollapsingHeader::new(
        RichText::new(format!("Sound Classes Overview ({count})"))
            .strong()
            .color(text_dark()),
    )
    .id_salt("sound_classes_overview")
    .default_open(true)
    .show(ui, |ui| {
        if count == 0 {
            ui.label(RichText::new("(no sound classes)").color(subtle_dark()));
            return;
        }
        egui::Grid::new("sound_classes_overview_grid")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for header in ["#", "near", "far", "detail"] {
                    ui.label(RichText::new(header).strong().color(subtle_dark()));
                }
                ui.end_row();
                for index in 0..count {
                    let Some(element) = classes.element(index) else {
                        continue;
                    };
                    let row = sound_class_distance_row(&element);
                    ui.label(RichText::new(format!("{index}")).color(subtle_dark()));
                    ui.label(RichText::new(row.near).color(text_dark()));
                    ui.label(RichText::new(row.far).color(text_dark()));
                    ui.label(RichText::new(row.detail).color(subtle_dark()));
                    ui.end_row();
                }
            });
    });
    ui.add_space(6.0);
}

pub(super) struct SoundClassDistanceRow {
    pub(super) near: String,
    pub(super) far: String,
    pub(super) detail: String,
}

pub(super) fn sound_class_distance_row(element: &TagStruct) -> SoundClassDistanceRow {
    // Modern (Reach/H4/H2A): scalar distances nested under `distance parameters`.
    if let Some(params) = element.descend("distance parameters") {
        let near = read_real_clean(&params, "minimum distance");
        let far = read_real_clean(&params, "maximum distance");
        let mut detail = Vec::new();
        if let Some(attack) = read_real_clean(&params, "attack distance") {
            detail.push(format!("attack {attack:.1}"));
        }
        if let Some(sustain) = read_real_clean(&params, "sustain db") {
            detail.push(format!("sustain {sustain:.1}dB"));
        }
        return SoundClassDistanceRow {
            near: fmt_real_opt(near),
            far: fmt_real_opt(far),
            detail: detail.join(", "),
        };
    }
    // Classic (H2/H3/ODST): `distance bounds` real_bounds directly on the entry.
    if let Some(bounds_name) = find_full_field_name(element, "distance bounds") {
        let bounds = element.read_real_bounds(bounds_name);
        let detail = if let Some(attack_name) = find_full_field_name(element, "attack bounds") {
            let attack = element.read_real_bounds(attack_name);
            format!("attack {:.1}..{:.1}", attack.lower, attack.upper)
        } else if let Some(silence) = element
            .field_names()
            .find(|name| name.to_ascii_lowercase().contains("silence"))
            .and_then(|name| element.read_real(name))
        {
            format!("inner silence {silence:.1}")
        } else {
            String::new()
        };
        return SoundClassDistanceRow {
            near: format!("{:.1}", bounds.lower),
            far: format!("{:.1}", bounds.upper),
            detail,
        };
    }

    SoundClassDistanceRow {
        near: "—".to_owned(),
        far: "—".to_owned(),
        detail: String::new(),
    }
}

/// Resolve a field by its cleaned (display) name — the engine stores names with
/// `:units#tooltip` / `{alias}` suffixes, so a direct `read_*(clean_name)` call
/// would never match. Returns the full stored name to pass to typed readers.
pub(in crate::app) fn find_full_field_name<'a>(
    element: &TagStruct<'a>,
    clean: &str,
) -> Option<&'a str> {
    element
        .field_names()
        .find(|name| clean_field_name(name).eq_ignore_ascii_case(clean))
}

fn read_real_clean(element: &TagStruct, clean: &str) -> Option<f32> {
    element.read_real(find_full_field_name(element, clean)?)
}

fn fmt_real_opt(value: Option<f32>) -> String {
    match value {
        Some(value) => format!("{value:.1}"),
        None => "—".to_owned(),
    }
}

/// The `dialogue` (`udlg`) tag group.
pub(in crate::app) fn is_sound_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"snd!"
}

/// How a permutation row sources its audio: an FMOD bank subsound (Halo 3+),
/// samples stored on the permutation itself (Halo CE, and Halo 2's older
/// layout), or Halo 2's per-language entries (see [`H2Sound`]).
pub(super) enum RowKind {
    Bank,
    /// The permutation's own `samples`. CE stores them in one of four formats
    /// (PCM / Xbox-ADPCM / IMA-ADPCM / Ogg Vorbis), so the codec is read from
    /// the tag rather than assumed.
    InlinePermutation {
        codec: super::audio::InlineCodec,
        channels: u16,
        sample_rate: u32,
    },
    /// Index into the tag's `language permutation info` block.
    InlineH2 {
        lpi: usize,
    },
}

/// One audition row: a permutation's name (which for the bank path is the
/// subsound key) + where its audio comes from.
pub(super) struct SoundPermRow {
    pub(super) pitch_range: String,
    pub(super) name: String,
    pub(super) pr_index: usize,
    pub(super) perm_index: usize,
    pub(super) kind: RowKind,
    pub(super) gain_db: Option<f32>,
    pub(super) skip_fraction: Option<f32>,
    /// Length of the permutation's own samples (`InlinePermutation` only).
    pub(super) inline_bytes: usize,
}

/// Extract a permutation's own `samples` bytes (CE, older Halo 2).
/// Re-navigates from the root so it only clones the played permutation's blob.
pub(super) fn inline_permutation_samples(
    tag: &TagFile,
    pr_index: usize,
    perm_index: usize,
) -> Option<Vec<u8>> {
    let root = tag.root();
    let pitch_ranges = find_block_field(&root, "pitch range")?;
    let pitch_range = pitch_ranges.element(pr_index)?;
    let permutations = find_block_field(&pitch_range, "permutation")?;
    let perm = permutations.element(perm_index)?;
    let full = find_full_field_name(&perm, "samples")?;
    let data = perm.field(full)?.as_data()?;
    (!data.is_empty()).then(|| data.to_vec())
}

/// Read the `file offset` of each `sound_permutation_chunk_block` element in a
/// raw-info-block struct (the block whose elements carry a `file offset` field).
/// H2 splits an entry's audio into ~1.36 s chunks, each an independent stream;
/// these offsets let the decoder slice + concatenate them. Empty if unchunked.
fn chunk_offsets_of(raw_el: &TagStruct) -> Vec<usize> {
    for field in raw_el.fields() {
        let Some(block) = field.as_block() else {
            continue;
        };
        if block.len() == 0 {
            continue;
        }
        let Some(first) = block.element(0) else {
            continue;
        };
        let Some(offset_field) = first.field_names().find(|n| {
            clean_field_name(n)
                .to_ascii_lowercase()
                .contains("file offset")
        }) else {
            continue;
        };
        let mut offsets = Vec::with_capacity(block.len());
        for m in 0..block.len() {
            let Some(el) = block.element(m) else {
                continue;
            };
            let Some(v) = el.read_int_any(offset_field) else {
                continue;
            };
            offsets.push(v.max(0) as usize);
        }
        return offsets;
    }

    Vec::new()
}

/// Codec parameters for samples stored on the permutation itself.
///
/// Halo CE names them `format` / `channel count` / `sample rate`, and uses Ogg
/// for music but Xbox-ADPCM for most effects, so the format must be read (the
/// Ogg decoder chokes on ADPCM bytes). Halo 2's older layout names them
/// `compression` (on the permutation, else the tag) / `encoding` / `sample
/// rate` — read through CE's names, its ADPCM decoded as PCM noise.
pub(super) fn permutation_inline_params(
    root: &TagStruct,
    perm: &TagStruct,
) -> (super::audio::InlineCodec, u16, u32) {
    use super::audio::InlineCodec;
    let read = |element: &TagStruct, clean: &str| {
        find_full_field_name(element, clean).and_then(|full| element.read_enum_name(full))
    };
    let Some(format) = read(root, "format") else {
        let compression = read(perm, "compression")
            .or_else(|| read(root, "compression"))
            .unwrap_or_default();
        let channels = read(root, "encoding").map_or(1, |name| h2_channels_for(&name));
        let rate = read(root, "sample rate").map_or(22_050, |name| h2_rate_for(&name));
        return (h2_codec_for(&compression), channels, rate);
    };
    let format = format.to_ascii_lowercase();
    let codec = if format.contains("ogg") || format.contains("vorbis") {
        InlineCodec::OggVorbis
    } else if format.contains("xbox") || format.contains("ima") {
        // "xbox adpcm" and "ima adpcm" are both IMA-family; the Xbox 0x0069
        // decoder handles CE's 36-byte-block layout.
        InlineCodec::XboxAdpcm
    } else {
        // "pcm" — uncompressed interleaved 16-bit PCM, little-endian on CE.
        InlineCodec::Pcm { big_endian: false }
    };
    let channels = read(root, "channel count")
        .map(|name| {
            if name.to_ascii_lowercase().contains("mono") {
                1
            } else {
                2
            }
        })
        .unwrap_or(1);
    let sample_rate = read(root, "sample rate")
        .map(|name| if name.contains("44") { 44_100 } else { 22_050 })
        .unwrap_or(22_050);
    (codec, channels, sample_rate)
}

/// Seconds of audio in `bytes` of a fixed-rate codec; `None` for Opus and Ogg,
/// whose length only a decode reveals.
fn fixed_rate_duration(
    codec: super::audio::InlineCodec,
    bytes: usize,
    channels: u16,
    sample_rate: u32,
) -> Option<f64> {
    use super::audio::InlineCodec;
    let channels = usize::from(channels.max(1));
    let frames = match codec {
        // 36 bytes per channel per block, 64 samples each.
        InlineCodec::XboxAdpcm => bytes / (36 * channels) * 64,
        InlineCodec::Pcm { .. } => bytes / (2 * channels),
        InlineCodec::Opus | InlineCodec::OggVorbis => return None,
    };
    (sample_rate > 0).then(|| frames as f64 / f64::from(sample_rate))
}

/// Audition panel for a `sound` (`snd!`) tag. Halo 3+ page the actual samples
/// out to the FMOD bank (`<game>/fmod/pc/*.fsb`) — the tag itself carries only
/// zeroed placeholder buffers — so we list the tag's pitch-range/permutation
/// names and play each by resolving its name against the opened banks. Clicking
/// Play/Stop queues an action the app drains after rendering. (Classic CE/H2,
/// whose audio is inline in the tag, aren't handled by this bank path yet and
/// will report "not found in FMOD bank".)
/// Halo 4 `.sound` tags reference Wwise events by name (no inline pitch-range
/// audio). Collect the non-empty event-name string-ids on the tag root.
pub(super) fn h4_event_names(tag: &TagFile) -> Vec<(&'static str, String)> {
    let root = tag.root();
    let mut out = Vec::new();
    for (label, field) in [
        ("Event", "event name"),
        ("Player event", "player event name"),
        ("Fallback event", "fallback event name"),
    ] {
        if let Some(name) = find_full_field_name(&root, field)
            .and_then(|full| root.read_string_id(full))
            .filter(|name| !name.is_empty())
        {
            out.push((label, name));
        }
    }
    out
}

/// Halo 2's languages in schema order (`sound.json`'s `language` enum), for the
/// dialogue and looping players, which can't know what their referenced sounds
/// carry until one is loaded.
const H2_LANGUAGES: [&str; 9] = [
    "english",
    "japanese",
    "german",
    "french",
    "spanish",
    "italian",
    "korean",
    "chinese",
    "portuguese",
];

/// A language the picker offers: the value kept in the shared audio state
/// (`None` = the source's default) and its label.
pub(super) struct LanguageChoice {
    value: Option<String>,
    label: String,
}

/// Localized languages available for the current source. Halo 2 carries its
/// languages in the tag (`h2`), English being the default; Wwise `.pck`
/// subdirs for H4/H2A and FMOD `.fsb` banks for the rest, behind a separate
/// "default". Empty ⇒ single-language.
fn language_choices(edit: &FieldEditContext<'_>, h2: Option<&H2Sound>) -> Vec<LanguageChoice> {
    let h2_choices = |languages: &mut dyn Iterator<Item = &str>| -> Vec<LanguageChoice> {
        languages
            .map(|language| LanguageChoice {
                value: (!language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE))
                    .then(|| language.to_owned()),
                label: language_label(language),
            })
            .collect()
    };
    if let Some(h2) = h2 {
        return if h2.languages.len() > 1 {
            h2_choices(&mut h2.languages.iter().map(String::as_str))
        } else {
            Vec::new()
        };
    }
    if edit.game == Some("halo2_mcc") && edit.ce_sound.is_none() {
        return h2_choices(&mut H2_LANGUAGES.iter().copied());
    }
    let with_default = |languages: Vec<String>| -> Vec<LanguageChoice> {
        if languages.is_empty() {
            return Vec::new();
        }
        std::iter::once(LanguageChoice {
            value: None,
            label: "default".to_owned(),
        })
        .chain(languages.into_iter().map(|language| LanguageChoice {
            label: language.clone(),
            value: Some(language),
        }))
        .collect()
    };
    // Campaign Evolved has no `tags_root` (its tags live in containers) and its
    // languages aren't discoverable from a bank directory — they're named by
    // the event's own cooked data, so take them from the resolved binding.
    // `SFX` is the non-localized bucket, not a language, so it isn't offered.
    if let Some(binding) = edit.ce_sound {
        let langs: Vec<String> = binding
            .languages()
            .into_iter()
            .filter(|l| !l.eq_ignore_ascii_case("SFX"))
            .collect();
        if !langs.is_empty() {
            return with_default(langs);
        }
    }
    let Some(root) = edit.tags_root else {
        return Vec::new();
    };
    with_default(match edit.game {
        Some("halo4_mcc") | Some("halo2amp_mcc") => {
            blam_tags::audio::WwiseBanks::available_languages(root)
        }

        _ => blam_tags::audio::SoundBanks::available_languages(root),
    })
}

/// Shared transport row for every sound-player variant: Stop, a volume slider, a
/// language selector (when the source is localized), and the status line. All
/// changes queue a [`super::audio::SoundAction`] the app drains after rendering.
fn draw_sound_transport(ui: &mut Ui, edit: &mut FieldEditContext<'_>, languages: &[LanguageChoice]) {
    ui.horizontal(|ui| {
        if ui
            .button(RichText::new("\u{25A0} Stop"))
            .on_hover_text("Stop playback")
            .clicked()
        {
            *edit.sound_play_request = Some(super::audio::SoundAction::Stop);
        }
        let mut volume = edit.sound_volume;
        ui.spacing_mut().slider_width = 90.0;
        if ui
            .add(
                egui::Slider::new(&mut volume, 0.0..=1.0)
                    .text(RichText::new("\u{1F50A}").color(subtle_dark()))
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            )
            .on_hover_text("Playback volume")
            .changed()
        {
            *edit.sound_play_request = Some(super::audio::SoundAction::SetVolume(volume));
        }
        // Language selector — picks which localized audio plays and is
        // extracted (to `data_<lang>\`). A language this source lacks shows as
        // the default, which is what plays.
        if !languages.is_empty() {
            let current = edit.sound_language.map(str::to_owned);
            let shown = languages
                .iter()
                .find(|choice| {
                    choice.value.as_deref().map(str::to_ascii_lowercase)
                        == current.as_deref().map(str::to_ascii_lowercase)
                })
                .or_else(|| languages.iter().find(|choice| choice.value.is_none()))
                .or(languages.first());
            let mut selected = shown.and_then(|choice| choice.value.clone());
            let before = selected.clone();
            egui::ComboBox::from_id_salt("sound_language")
                .selected_text(format!(
                    "\u{1F310} {}",
                    shown.map_or("default", |choice| choice.label.as_str())
                ))
                .show_ui(ui, |ui| {
                    for choice in languages {
                        ui.selectable_value(&mut selected, choice.value.clone(), &choice.label);
                    }
                })
                .response
                .on_hover_text(format!(
                    "Language to play and extract ({} available)",
                    languages.len()
                ));
            if selected != before {
                *edit.sound_play_request = Some(super::audio::SoundAction::SetLanguage(selected));
            }
        }
        if let Some(status) = edit.sound_status {
            ui.label(RichText::new(status).color(subtle_dark()));
        }
    });
}

/// The block index a Halo 2 permutation keeps into `language permutation info`
/// (its unnamed `custom short block index`), or `None` when unset.
fn permutation_lpi_index(perm: &TagStruct) -> Option<usize> {
    perm.fields()
        .filter(|field| field.field_type() == TagFieldType::CustomShortBlockIndex)
        .find_map(|field| match field.value() {
            Some(blam_tags::TagFieldData::CustomShortBlockIndex(index)) => {
                usize::try_from(index).ok()
            }
            _ => None,
        })
}

/// Build the audition/extraction rows for a `.sound` tag: every pitch-range
/// permutation with its name and audio source, classified identically for the
/// player and the extractor. Capped so a pathological tag can't stall the UI.
/// `h2` is the tag's [`H2Sound`], when it is one.
pub(super) fn sound_permutation_rows(tag: &TagFile, h2: Option<&H2Sound>) -> Vec<SoundPermRow> {
    let root = tag.root();
    let Some(pitch_ranges) = find_block_field(&root, "pitch range") else {
        return Vec::new();
    };
    const MAX_ROWS: usize = 400;
    // Permutations in tag order: the fallback index into `language permutation
    // info` for a permutation whose own index is unset or out of range.
    let mut ordinal = 0usize;
    let mut rows: Vec<SoundPermRow> = Vec::new();
    for pr_index in 0..pitch_ranges.len() {
        let Some(pitch_range) = pitch_ranges.element(pr_index) else {
            continue;
        };
        let pr_name = find_full_field_name(&pitch_range, "name")
            .and_then(|full| pitch_range.read_string_id(full))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("pitch range {pr_index}"));
        let Some(permutations) = find_block_field(&pitch_range, "permutation") else {
            continue;
        };
        for perm_index in 0..permutations.len() {
            if rows.len() >= MAX_ROWS {
                break;
            }
            let Some(perm) = permutations.element(perm_index) else {
                continue;
            };
            let name = find_full_field_name(&perm, "name")
                .and_then(|full| perm.read_string_id(full))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("#{perm_index}"));
            let inline_bytes = find_full_field_name(&perm, "samples")
                .and_then(|full| perm.field(full))
                .and_then(|field| field.as_data())
                .map_or(0, <[u8]>::len);
            let kind = if inline_bytes > 0 {
                let (codec, channels, sample_rate) = permutation_inline_params(&root, &perm);
                RowKind::InlinePermutation {
                    codec,
                    channels,
                    sample_rate,
                }
            } else if let Some(h2) = h2 {
                let lpi = permutation_lpi_index(&perm)
                    .filter(|&index| !h2.entries(index).is_empty())
                    .unwrap_or(ordinal);
                RowKind::InlineH2 { lpi }
            } else {
                RowKind::Bank
            };
            ordinal += 1;
            rows.push(SoundPermRow {
                pitch_range: pr_name.clone(),
                name,
                pr_index,
                perm_index,
                kind,
                gain_db: find_full_field_name(&perm, "gain").and_then(|full| perm.read_real(full)),
                skip_fraction: find_full_field_name(&perm, "skip fraction")
                    .and_then(|full| perm.read_real(full)),
                inline_bytes,
            });
        }
    }
    rows
}

/// A `.sound` tag's `<tags root>`-relative path without extension (e.g.
/// `.../tags/sound/dialog/.../ambush.sound` → `sound\dialog\...\ambush`), for
/// building the FMOD subsound id. Backslash-normalized; the hash lowercases.
fn sound_tag_rel(abs: &std::path::Path, tags_root: &std::path::Path) -> Option<String> {
    let rel = abs.strip_prefix(tags_root).ok()?;
    Some(rel.with_extension("").to_string_lossy().replace('/', "\\"))
}

/// The engine's `fmod bank subsound id hash` for a Halo 3+ permutation row —
/// the collision-free key that resolves to the exact intended variant. Needs the
/// owning `.sound` tag's rel path (e.g. `sound\dialog\...\ambush`); returns
/// `None` when it's unknown, and playback then falls back to leaf-name lookup.
/// Folds in the pitch-range folder per the engine rule, and hashes the
/// permutation's raw string-id (`row.name`) — matching the tool's own build.
fn row_bank_id(sound_rel: Option<&str>, multi_pr: bool, row: &SoundPermRow) -> Option<u32> {
    let sound_rel = sound_rel?;
    let pitch = blam_tags::audio::fmod_pitch_range_folder(&row.pitch_range, multi_pr);
    Some(blam_tags::audio::fmod_bank_subsound_id_hash(
        sound_rel, pitch, &row.name,
    ))
}

/// What a row plays and extracts from: the Halo 2 language entries and the
/// chosen language, and the FMOD subsound id inputs (see [`row_bank_id`]).
#[derive(Clone, Copy)]
pub(super) struct RowSource<'a> {
    pub(super) h2: Option<&'a H2Sound>,
    /// The chosen language; `None` is the source's default.
    pub(super) language: Option<&'a str>,
    pub(super) sound_rel: Option<&'a str>,
    pub(super) multi_pr: bool,
}

/// The play action for a permutation row (bank subsound / the permutation's own
/// samples / a Halo 2 language entry). `None` if the audio can't be read.
fn row_play_action(
    tag: &TagFile,
    row: &SoundPermRow,
    source: RowSource<'_>,
) -> Option<super::audio::SoundAction> {
    use super::audio::SoundAction;
    match &row.kind {
        RowKind::Bank => Some(SoundAction::Play {
            id: row_bank_id(source.sound_rel, source.multi_pr, row),
            key: row.name.clone(),
            label: row.name.clone(),
        }),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => {
            let bytes = inline_permutation_samples(tag, row.pr_index, row.perm_index)?;

            Some(SoundAction::PlayInline {
                bytes,
                codec: *codec,
                channels: *channels,
                sample_rate: *sample_rate,
                chunk_offsets: Vec::new(),
                label: row.name.clone(),
            })
        }
        RowKind::InlineH2 { lpi } => {
            let h2 = source.h2?;
            let (entry, _) = h2.entry_for(*lpi, source.language)?;
            let (bytes, chunk_offsets) = h2.samples(tag, entry)?;
            let (codec, channels, sample_rate) = h2.decode_params(entry);
            let label = if entry.language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE) {
                row.name.clone()
            } else {
                format!("{} ({})", row.name, language_label(&entry.language))
            };
            Some(SoundAction::PlayInline {
                bytes,
                codec,
                channels,
                sample_rate,
                chunk_offsets,
                label,
            })
        }
    }
}

/// Where a permutation row's audio comes from for extraction. Mirrors
/// [`row_play_action`] but yields file-writing sources. `raw_ce` writes CE's
/// self-contained inline Ogg verbatim (near-lossless) instead of decoding.
/// `exact_language` refuses a Halo 2 row that lacks the chosen language rather
/// than writing another language's audio in its place.
fn row_extract_source(
    tag: &TagFile,
    row: &SoundPermRow,
    source: RowSource<'_>,
    raw_ce: bool,
    exact_language: bool,
) -> Option<ExtractSource> {
    use super::audio::InlineCodec;
    match &row.kind {
        RowKind::Bank => Some(ExtractSource::Bank {
            id: row_bank_id(source.sound_rel, source.multi_pr, row),
            key: row.name.clone(),
        }),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => {
            let bytes = inline_permutation_samples(tag, row.pr_index, row.perm_index)?;
            // Raw passthrough writes the stream verbatim — only meaningful (and
            // only a valid `.ogg`) when the CE format actually is Ogg Vorbis.
            let raw_ogg = raw_ce && matches!(codec, InlineCodec::OggVorbis);
            Some(if raw_ogg {
                ExtractSource::Raw(bytes)
            } else {
                ExtractSource::Inline {
                    bytes,
                    codec: *codec,
                    channels: *channels,
                    sample_rate: *sample_rate,
                    chunk_offsets: Vec::new(),
                }
            })
        }
        RowKind::InlineH2 { lpi } => {
            let h2 = source.h2?;
            let (entry, fallback) = h2.entry_for(*lpi, source.language)?;
            if fallback && exact_language {
                return None;
            }
            let (bytes, chunk_offsets) = h2.samples(tag, entry)?;
            let (codec, channels, sample_rate) = h2.decode_params(entry);
            Some(ExtractSource::Inline {
                bytes,
                codec,
                channels,
                sample_rate,
                chunk_offsets,
            })
        }
    }
}

/// File extension for an extracted row: CE raw passthrough keeps `.ogg`;
/// everything else decodes to `.wav`.
fn row_extract_ext(kind: &RowKind, raw_ce: bool) -> &'static str {
    if raw_ce
        && matches!(
            kind,
            RowKind::InlinePermutation {
                codec: super::audio::InlineCodec::OggVorbis,
                ..
            }
        )
    {
        "ogg"
    } else {
        "wav"
    }
}

/// A compact `(sound class, codec)` readout for the player header.
fn sound_class_and_compression(tag: &TagFile) -> (Option<String>, String) {
    let root = tag.root();
    // CE names it `sound class`; H2 through Reach, `class`.
    let class = find_full_field_name(&root, "sound class")
        .or_else(|| find_full_field_name(&root, "class"))
        .and_then(|full| root.read_enum_name(full))
        .filter(|value| !value.is_empty());
    let compression = find_full_field_name(&root, "compression")
        .and_then(|full| root.read_enum_name(full))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "FMOD Vorbis bank".to_owned());
    (class, compression)
}

/// Whether a pitch-range name is the implicit default (tool writes those files
/// flat, no subfolder). Covers the literal `|default|`, an empty name, and the
/// `pitch range N` placeholder [`sound_permutation_rows`] synthesizes for an
/// unnamed range.
pub(super) fn is_default_pitch_range(name: &str) -> bool {
    let n = name.trim();
    n.is_empty()
        || n.eq_ignore_ascii_case("|default|")
        || n.eq_ignore_ascii_case("default")
        || n.strip_prefix("pitch range ")
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

/// Whether the tag has more than one distinct pitch range (drives subfoldering).
fn rows_span_multiple_pitch_ranges(rows: &[SoundPermRow]) -> bool {
    let mut names: Vec<&str> = rows.iter().map(|row| row.pitch_range.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names.len() > 1
}

/// The `[<pitch range>/]<permutation>.<ext>` path (relative to the tag's data
/// dir) for a permutation — shared by extraction and reimport so they agree on
/// filenames. A subfolder is used when there are multiple ranges or the single
/// range is named (non-default); a lone default range stays flat.
fn perm_relative_path(multi_pr: bool, row: &SoundPermRow, ext: &str) -> std::path::PathBuf {
    let file = format!("{}.{ext}", sanitize_component(&row.name));
    if multi_pr || !is_default_pitch_range(&row.pitch_range) {
        std::path::PathBuf::from(sanitize_component(&row.pitch_range)).join(file)
    } else {
        std::path::PathBuf::from(file)
    }
}

/// Lay each row out under `base` as `[<pitch range>/]<permutation>.<ext>` — the
/// structure `tool.exe`'s sound import consumes (RE-verified from the tool's own
/// exporter). A Halo 2 permutation without the chosen language is left out, not
/// filled with another language's audio.
pub(super) fn build_extract_items(
    tag: &TagFile,
    rows: &[SoundPermRow],
    source: RowSource<'_>,
    base: &std::path::Path,
    raw_ce: bool,
) -> Vec<ExtractItem> {
    let multi_pr = rows_span_multiple_pitch_ranges(rows);
    let source = RowSource { multi_pr, ..source };
    rows.iter()
        .filter_map(|row| {
            let item_source = row_extract_source(tag, row, source, raw_ce, true)?;
            let rel = perm_relative_path(multi_pr, row, row_extract_ext(&row.kind, raw_ce));
            Some(ExtractItem {
                out_path: base.join(rel),
                source: item_source,
            })
        })
        .collect()
}

/// The `data\` root name a Halo 2 language extracts under: English is the
/// default (`data\`), the rest `data_<language>\`.
fn h2_data_language(language: &str) -> Option<&str> {
    (!language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE)).then_some(language)
}

/// Per-game one-line note on the format `tool.exe` requires when reimporting the
/// extracted files (RE-verified from each tool binary). Empty for Wwise games.
fn sound_format_note(game: Option<&str>) -> &'static str {
    match game {
        Some("haloce_mcc") => "16-bit WAV, 22050 or 44100 Hz, mono/stereo",
        Some("halo2_mcc") => "16-bit WAV, 22050/32000/44100/48000 Hz (resampled), mono/stereo",
        Some("halo3_mcc") | Some("halo3odst_mcc") | Some("haloreach_mcc") => {
            "16-bit WAV, 48000 Hz, mono/stereo"
        }
        _ => "",
    }
}

/// Render the Halo 4 Wwise event player: a play button per named event that
/// queues a [`super::audio::SoundAction::PlayEvent`] (resolved against the
/// game's `.pck` banks by the audio layer).
fn draw_wwise_event_player(
    ui: &mut Ui,
    events: &[(&'static str, String)],
    edit: &mut FieldEditContext<'_>,
) {
    let languages = language_choices(edit, None);
    egui::CollapsingHeader::new(
        RichText::new(format!("Sound \u{2014} Wwise event ({})", events.len())).color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit, &languages);
        ui.label(
            RichText::new(
                "Wwise-authored \u{2014} audio lives in sound\\pc\\*.pck; \
                 extract-only (no tool.exe reimport).",
            )
            .color(subtle_dark()),
        );
        egui::Grid::new("wwise_events")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for (label, name) in events {
                    if ui
                        .small_button("\u{25B6}")
                        .on_hover_text("Play this Wwise event from the sound banks")
                        .clicked()
                    {
                        *edit.sound_play_request = Some(super::audio::SoundAction::PlayEvent {
                            event_name: name.clone(),
                            label: name.clone(),
                        });
                    }
                    if ui
                        .small_button("\u{2B07}")
                        .on_hover_text(
                            "Extract this event to WAV (play it once first to load the banks)",
                        )
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Extract Wwise event")
                            .set_file_name(format!("{}.wav", sanitize_component(name)))
                            .save_file()
                        {
                            *edit.sound_extract_request = Some(ExtractRequest {
                                items: vec![ExtractItem {
                                    out_path: path,
                                    source: ExtractSource::Event { name: name.clone() },
                                }],
                                tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                                label: name.clone(),
                            });
                        }
                    }
                    ui.label(RichText::new(*label).color(subtle_dark()));
                    ui.label(RichText::new(name).color(text_dark()));
                    ui.end_row();
                }
            });
    });
}

/// Stand-in for a Campaign Evolved `sound` tag that reaches no Wwise event.
///
/// About a tenth of the game's sound tags are like this — Reach metadata kept
/// for the simulation with no audio asset behind them. Saying so is the whole
/// point: the alternative is a player that looks playable and never is.
fn draw_ce_unbound_note(ui: &mut Ui) {
    egui::CollapsingHeader::new(RichText::new("Sound \u{2014} no audio bound").color(text_dark()))
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "This tag's permutation table is inherited Reach metadata. Campaign \
                 Evolved plays audio through Wwise, and no event is wired to this tag, \
                 so there is nothing to audition or extract.",
                )
                .color(subtle_dark()),
            );
        });
    ui.add_space(6.0);
}

/// Render the Campaign Evolved player.
///
/// CE `sound` tags hold no sample data and — unlike Halo 4 — name no event
/// either, so there is nothing in the tag to play from. The rows here come from
/// the binding the controller already resolved by walking the tag's package
/// imports out to its Wwise event(s); each row is one `.wem` permutation.
fn draw_ce_wwise_player(
    ui: &mut Ui,
    binding: &crate::source::ce_audio::CeSoundBinding,
    edit: &mut FieldEditContext<'_>,
) {
    let languages = binding.languages();
    let localized = !(languages.len() == 1 && languages[0].eq_ignore_ascii_case("SFX"));
    // The global selector is shared with every other game's player, so it may
    // hold a language this tag doesn't carry (or nothing at all). Let the
    // binding pick what it can actually show.
    let selected = binding.language_to_show(edit.sound_language);
    let media = binding.media_for_language(&selected);

    let language_picker = language_choices(edit, None);
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Sound \u{2014} Wwise media ({} permutation{})",
            media.len(),
            if media.len() == 1 { "" } else { "s" }
        ))
        .color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit, &language_picker);
        ui.label(
            RichText::new(if localized {
                "Wwise-authored \u{2014} localized voice; media lives in the \
                 language pak chunks. Extract-only."
            } else {
                "Wwise-authored \u{2014} the tag carries no samples; media lives \
                 in the pak chunks. Extract-only."
            })
            .color(subtle_dark()),
        );
        if localized {
            ui.label(
                RichText::new(format!(
                    "languages: {}  (showing {selected})",
                    languages.join(", ")
                ))
                .color(subtle_dark()),
            );
        }

        // Whole-tag extract. CE media is addressed directly in the pak set, so
        // unlike the Halo 4 event player this needs no prior playback.
        if !media.is_empty()
            && let Some(root) = edit.ce_paks_root
        {
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("\u{2B07} Extract all"))
                    .on_hover_text("Extract every permutation of this language to WAV")
                    .clicked()
                    && let Some(base) = rfd::FileDialog::new()
                        .set_title("Extract Wwise media")
                        .pick_folder()
                {
                    let items = media
                        .iter()
                        .map(|m| ExtractItem {
                            out_path: base
                                .join(format!("{}.wav", sanitize_component(&m.display_name()))),
                            source: ExtractSource::CeMedia {
                                paks_root: root.to_path_buf(),
                                media: Box::new((*m).clone()),
                            },
                        })
                        .collect();
                    *edit.sound_extract_request = Some(ExtractRequest {
                        items,
                        tags_root: None,
                        label: base
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "sound".to_owned()),
                    });
                }
            });
        }

        egui::Grid::new("ce_wwise_media")
            .striped(true)
            .num_columns(5)
            .show(ui, |ui| {
                for m in &media {
                    if ui
                        .small_button("\u{25B6}")
                        .on_hover_text("Play this permutation")
                        .clicked()
                    {
                        match edit.ce_paks_root {
                            Some(root) => {
                                *edit.sound_play_request =
                                    Some(super::audio::SoundAction::PlayCeMedia {
                                        paks_root: root.to_path_buf(),
                                        media: Box::new((*m).clone()),
                                        label: m.display_name(),
                                    });
                            }
                            // Shouldn't happen for a container source, but a
                            // click that does nothing at all is worse than one
                            // that says why.
                            None => {
                                if let Some(status) = edit.status.as_deref_mut() {
                                    "no container source for Wwise media".clone_into(status);
                                }
                            }
                        }
                    }
                    if ui
                        .small_button("\u{2B07}")
                        .on_hover_text("Extract this permutation to WAV")
                        .clicked()
                        && let Some(root) = edit.ce_paks_root
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Extract Wwise media")
                            .set_file_name(format!("{}.wav", sanitize_component(&m.display_name())))
                            .save_file()
                    {
                        *edit.sound_extract_request = Some(ExtractRequest {
                            items: vec![ExtractItem {
                                out_path: path,
                                source: ExtractSource::CeMedia {
                                    paks_root: root.to_path_buf(),
                                    media: Box::new((*m).clone()),
                                },
                            }],
                            tags_root: None,
                            label: m.display_name(),
                        });
                    }
                    ui.label(RichText::new(m.display_name()).color(text_dark()))
                        .on_hover_text(&m.source_name);
                    ui.label(RichText::new(&m.event_name).color(subtle_dark()));
                    ui.label(RichText::new(m.location_label()).color(subtle_dark()));
                    ui.end_row();
                }
            });
    });
}

/// `1` → `mono`, `2` → `stereo`, `4` → `quad`.
fn channel_label(channels: u16) -> String {
    match channels {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        4 => "quad".to_owned(),
        6 => "5.1".to_owned(),
        n => format!("{n} channels"),
    }
}

fn codec_label(codec: super::audio::InlineCodec) -> &'static str {
    use super::audio::InlineCodec;
    match codec {
        InlineCodec::OggVorbis => "ogg vorbis",
        InlineCodec::Opus => "opus",
        InlineCodec::XboxAdpcm => "xbox adpcm",
        InlineCodec::Pcm { .. } => "pcm",
    }
}

/// Why a legacy entry's rate is shown as inferred.
const INFERRED_RATE_HOVER: &str = "Legacy Xbox audio. The tag doesn't record this language's sample \
     rate (the tool writes every language at the tag's rate; this one predates that), so it is \
     recovered from the lip-sync mouth data, which runs at the same pace in every language.";

/// The audio a row plays for `language`, as `(duration, fell back to English)`.
fn row_duration(row: &SoundPermRow, source: RowSource<'_>) -> (Option<f64>, bool) {
    match &row.kind {
        RowKind::Bank => (None, false),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => (
            fixed_rate_duration(*codec, row.inline_bytes, *channels, *sample_rate),
            false,
        ),
        RowKind::InlineH2 { lpi } => {
            let Some((entry, fallback)) = source.h2.and_then(|h2| h2.entry_for(*lpi, source.language))
            else {
                return (None, false);
            };
            (source.h2.and_then(|h2| h2.duration_secs(entry)), fallback)
        }
    }
}

/// Everything a row holds, for its hover: every Halo 2 language with its codec,
/// rate, length and size, or the permutation's own format.
fn row_details(row: &SoundPermRow, h2: Option<&H2Sound>) -> String {
    match &row.kind {
        RowKind::Bank => format!("{}\nplays from the FMOD sound bank", row.name),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => format!(
            "{}\n{} \u{00B7} {} \u{00B7} {} \u{00B7} {}",
            row.name,
            codec_label(*codec),
            channel_label(*channels),
            format_rate(*sample_rate),
            format_bytes(row.inline_bytes)
        ),
        RowKind::InlineH2 { lpi } => {
            let Some(h2) = h2 else {
                return row.name.clone();
            };
            let mut lines = vec![row.name.clone()];
            let mut entries: Vec<&H2Entry> = h2.entries(*lpi).iter().collect();
            entries.sort_by_key(|entry| {
                h2.languages
                    .iter()
                    .position(|language| *language == entry.language)
            });
            for entry in entries {
                let (rate, rate_source) = h2.rate_of(entry);
                let duration = h2
                    .duration_secs(entry)
                    .map(|seconds| format!(" \u{00B7} {seconds:.2} s"))
                    .unwrap_or_default();
                let inferred = match rate_source {
                    H2RateSource::Tag => "",
                    H2RateSource::Inferred => " (inferred)",
                    H2RateSource::Unknown => " (unknown, assumed)",
                };
                lines.push(format!(
                    "{}: {} \u{00B7} {}{inferred}{duration} \u{00B7} {}",
                    language_label(&entry.language),
                    entry.compression,
                    format_rate(rate),
                    format_bytes(entry.sample_bytes)
                ));
            }
            lines.join("\n")
        }
    }
}

/// The class · codec · channels · rate line under the transport, describing
/// what the chosen language actually plays.
fn draw_sound_format_line(ui: &mut Ui, tag: &TagFile, rows: &[SoundPermRow], source: RowSource<'_>) {
    let (class, compression) = sound_class_and_compression(tag);
    ui.horizontal_wrapped(|ui| {
        let dot = |ui: &mut Ui| {
            ui.label(RichText::new("\u{00B7}").color(subtle_dark()));
        };
        if let Some(class) = &class {
            ui.label(RichText::new(format!("class: {class}")).color(subtle_dark()));
            dot(ui);
        }
        let h2_entry = source.h2.and_then(|h2| {
            rows.iter().find_map(|row| match row.kind {
                RowKind::InlineH2 { lpi } => h2.entry_for(lpi, source.language),
                _ => None,
            })
        });
        let (Some(h2), Some((entry, _))) = (source.h2, h2_entry) else {
            let text = match rows.first().map(|row| &row.kind) {
                Some(RowKind::InlinePermutation {
                    codec,
                    channels,
                    sample_rate,
                }) => format!(
                    "{} \u{00B7} {} \u{00B7} {}",
                    codec_label(*codec),
                    channel_label(*channels),
                    format_rate(*sample_rate)
                ),
                _ => format!("codec: {compression}"),
            };
            ui.label(RichText::new(text).color(subtle_dark()));
            return;
        };
        let (rate, rate_source) = h2.rate_of(entry);
        ui.label(
            RichText::new(format!(
                "{} \u{00B7} {} \u{00B7} {}",
                entry.compression,
                channel_label(h2.channels),
                format_rate(rate)
            ))
            .color(subtle_dark()),
        );
        match rate_source {
            H2RateSource::Tag => {}
            H2RateSource::Inferred => {
                ui.label(RichText::new("(rate inferred)").color(ui.visuals().warn_fg_color))
                    .on_hover_text(INFERRED_RATE_HOVER);
            }
            H2RateSource::Unknown => {
                ui.label(
                    RichText::new("(rate unknown \u{2014} assumed)")
                        .color(ui.visuals().warn_fg_color),
                )
                .on_hover_text(INFERRED_RATE_HOVER);
            }
        }
    });
}

pub(in crate::app) fn draw_sound_player(
    ui: &mut Ui,
    tag: &TagFile,
    edit: &mut FieldEditContext<'_>,
) {
    // Campaign Evolved: audio is reached through package imports, not the tag.
    // `Some` means this is a CE sound tag, whatever it resolved to — so the
    // fall-through below (which reads the Reach permutation table and plays it
    // out of an FMOD bank that does not exist here) must never be reached. It
    // rendered a working-looking player whose every button said "no source
    // loaded".
    if let Some(binding) = edit.ce_sound {
        if binding.is_empty() {
            draw_ce_unbound_note(ui);
        } else {
            let binding = binding.clone();
            draw_ce_wwise_player(ui, &binding, edit);
        }
        return;
    }
    // Halo 4: Wwise event reference, no inline pitch-range audio.
    let events = h4_event_names(tag);
    if !events.is_empty() {
        draw_wwise_event_player(ui, &events, edit);
        return;
    }
    let h2 = H2Sound::read(tag);
    let rows = sound_permutation_rows(tag, h2.as_ref());
    if rows.is_empty() {
        return;
    }
    let languages = language_choices(edit, h2.as_ref());
    // A Halo 2 tag plays the chosen language when it has it, else English;
    // the shared choice may be another game's language this tag never had.
    let chosen = edit.sound_language.map(str::to_owned);
    let missing_language = h2
        .as_ref()
        .zip(chosen.as_deref())
        .filter(|(h2, language)| !h2.has_language(language))
        .map(|(_, language)| language.to_owned());
    let language = if missing_language.is_some() {
        None
    } else {
        chosen.as_deref()
    };
    // The raw-passthrough toggle is only meaningful for CE Ogg-format tags
    // (writing the verbatim stream as `.ogg`); ADPCM/PCM tags must decode to WAV.
    let has_inline_ogg = rows.iter().any(|row| {
        matches!(
            row.kind,
            RowKind::InlinePermutation {
                codec: super::audio::InlineCodec::OggVorbis,
                ..
            }
        )
    });
    // Loose `.sound` file path (for the reimport data\ layout + tool tag path).
    let abs_tag_path = edit
        .tag_key
        .strip_prefix("file:")
        .map(std::path::PathBuf::from);
    // This tag's rel path (e.g. `sound\dialog\...\ambush`) + whether it spans
    // multiple pitch ranges — both feed the FMOD subsound id hash.
    let sound_rel = abs_tag_path
        .as_deref()
        .zip(edit.tags_root)
        .and_then(|(abs, root)| sound_tag_rel(abs, root));
    let multi_pr = rows_span_multiple_pitch_ranges(&rows);
    let source = RowSource {
        h2: h2.as_ref(),
        language,
        sound_rel: sound_rel.as_deref(),
        multi_pr,
    };
    // A lone `|default|` pitch range is just the container; name ranges only
    // when there's more than one, or the one there is was named.
    let show_pitch_ranges =
        multi_pr || rows.first().is_some_and(|row| !is_default_pitch_range(&row.pitch_range));
    let localized = h2.as_ref().is_some_and(|h2| h2.languages.len() > 1);
    let language_name = language_label(language.unwrap_or(H2_DEFAULT_LANGUAGE));

    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Sound \u{2014} {} permutation{}",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        ))
        .color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit, &languages);
        draw_sound_format_line(ui, tag, &rows, source);
        if let Some(missing) = &missing_language {
            ui.label(
                RichText::new(format!(
                    "{} isn't in this tag \u{2014} playing English.",
                    language_label(missing)
                ))
                .color(subtle_dark()),
            );
        }

        // Extract. The CE raw-ogg toggle persists per tag. (Reimport is left
        // to the user via the game's tool.exe.)
        let raw_ce_id = ui.make_persistent_id(("sound_raw_ce", edit.tag_key));
        let mut raw_ce = ui.data(|d| d.get_temp::<bool>(raw_ce_id)).unwrap_or(false);
        let data_language = |language: Option<&str>| -> Option<String> {
            if h2.is_some() {
                language.and_then(h2_data_language).map(str::to_owned)
            } else {
                language.map(str::to_owned)
            }
        };
        let base_for = |language: Option<&str>| {
            abs_tag_path
                .as_deref()
                .zip(edit.tags_root)
                .and_then(|(tag_path, root)| {
                    reimport_base_dir_lang(root, tag_path, data_language(language).as_deref())
                })
        };
        let extract_base = base_for(if h2.is_some() {
            language
        } else {
            edit.sound_language
        });
        let format_note = sound_format_note(edit.game);
        let missing_count = rows
            .iter()
            .filter(|row| row_duration(row, source).1)
            .count();
        ui.horizontal(|ui| {
            let mut extract_hover = match &extract_base {
                Some(dir) => format!("Extract every permutation to {}", dir.display()),
                None => "Choose a folder and extract every permutation".to_owned(),
            };
            if missing_count > 0 {
                extract_hover.push_str(&format!(
                    "\n{missing_count} permutation(s) have no {language_name} audio and are skipped"
                ));
            }
            if !format_note.is_empty() {
                extract_hover.push_str(&format!("\nFor tool.exe reimport: {format_note}"));
            }
            let extract_label = if localized {
                format!("\u{2B07} Extract all ({language_name})")
            } else {
                "\u{2B07} Extract all".to_owned()
            };
            if ui
                .button(RichText::new(extract_label))
                .on_hover_text(extract_hover)
                .clicked()
            {
                let base = extract_base.clone().or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Extract sound permutations")
                        .pick_folder()
                });
                if let Some(base) = base {
                    let items = build_extract_items(tag, &rows, source, &base, raw_ce);
                    let label = base
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "sound".to_owned());
                    *edit.sound_extract_request = Some(ExtractRequest {
                        items,
                        tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                        label,
                    });
                }
            }
            if let Some(h2) = h2.as_ref().filter(|_| localized) {
                let all_hover = if extract_base.is_some() {
                    format!(
                        "Extract all {} languages: English to data\\\u{2026}, the others to \
                         data_<language>\\\u{2026}",
                        h2.languages.len()
                    )
                } else {
                    format!(
                        "Choose a folder and extract all {} languages, one subfolder each",
                        h2.languages.len()
                    )
                };
                if ui
                    .button(RichText::new("\u{2B07} All languages"))
                    .on_hover_text(all_hover)
                    .clicked()
                {
                    // Loose tags extract beside the kit's data roots; anywhere
                    // else, into one chosen folder with a subfolder per language.
                    let picked = if extract_base.is_some() {
                        None
                    } else {
                        rfd::FileDialog::new()
                            .set_title("Extract every language")
                            .pick_folder()
                    };
                    if extract_base.is_some() || picked.is_some() {
                        let mut items = Vec::new();
                        for each in &h2.languages {
                            let base = match &picked {
                                Some(folder) => Some(folder.join(each)),
                                None => base_for(Some(each)),
                            };
                            let Some(base) = base else {
                                continue;
                            };
                            let each_source = RowSource {
                                language: Some(each),
                                ..source
                            };
                            items.extend(build_extract_items(tag, &rows, each_source, &base, false));
                        }
                        *edit.sound_extract_request = Some(ExtractRequest {
                            items,
                            tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                            label: "all languages".to_owned(),
                        });
                    }
                }
            }
            if has_inline_ogg {
                ui.checkbox(&mut raw_ce, "raw .ogg").on_hover_text(
                    "Extract CE audio as the tag's original Ogg stream (lossless) \
                     instead of decoding to WAV",
                );
            }
        });
        ui.data_mut(|d| d.insert_temp(raw_ce_id, raw_ce));

        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                egui::Grid::new("sound_permutations")
                    .striped(true)
                    .num_columns(5)
                    .show(ui, |ui| {
                        let mut last_pitch_range: Option<&str> = None;
                        for row in &rows {
                            if show_pitch_ranges && last_pitch_range != Some(row.pitch_range.as_str()) {
                                last_pitch_range = Some(row.pitch_range.as_str());
                                ui.label("");
                                ui.label("");
                                ui.label(
                                    RichText::new(format!("pitch range: {}", row.pitch_range))
                                        .strong()
                                        .color(subtle_dark()),
                                );
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                            }
                            let (duration, fallback) = row_duration(row, source);
                            let play_hover = match row.kind {
                                RowKind::Bank => {
                                    "Play this permutation from the FMOD bank".to_owned()
                                }
                                RowKind::InlineH2 { .. } if localized && !fallback => {
                                    format!("Play this permutation in {language_name}")
                                }
                                RowKind::InlineH2 { .. } if fallback => {
                                    "Play this permutation in English".to_owned()
                                }
                                _ => "Play this permutation".to_owned(),
                            };
                            if ui.small_button("\u{25B6}").on_hover_text(play_hover).clicked() {
                                *edit.sound_play_request = row_play_action(tag, row, source);
                            }
                            if ui
                                .small_button("\u{2B07}")
                                .on_hover_text("Extract this permutation to a file")
                                .clicked()
                            {
                                let ext = row_extract_ext(&row.kind, raw_ce);
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title("Extract permutation")
                                    .set_file_name(format!(
                                        "{}.{ext}",
                                        sanitize_component(&row.name)
                                    ))
                                    .save_file()
                                    && let Some(item_source) =
                                        row_extract_source(tag, row, source, raw_ce, false)
                                {
                                    *edit.sound_extract_request = Some(ExtractRequest {
                                        items: vec![ExtractItem {
                                            out_path: path,
                                            source: item_source,
                                        }],
                                        tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                                        label: row.name.clone(),
                                    });
                                }
                            }
                            ui.label(RichText::new(&row.name).color(text_dark()))
                                .on_hover_text(row_details(row, h2.as_ref()));
                            ui.label(
                                RichText::new(
                                    duration
                                        .map(|seconds| format!("{seconds:.2} s"))
                                        .unwrap_or_default(),
                                )
                                .color(subtle_dark()),
                            );
                            ui.horizontal(|ui| {
                                if fallback {
                                    ui.label(
                                        RichText::new(format!(
                                            "no {language_name} \u{00B7} plays English"
                                        ))
                                        .color(ui.visuals().warn_fg_color),
                                    );
                                }
                                if let Some(gain) = row.gain_db.filter(|gain| gain.abs() >= 0.05) {
                                    ui.label(
                                        RichText::new(format!("gain {gain:+.1} dB"))
                                            .color(subtle_dark()),
                                    );
                                }
                                if let Some(skip) = row.skip_fraction.filter(|skip| *skip > 0.0) {
                                    ui.label(
                                        RichText::new(format!("skip {:.0}%", skip * 100.0))
                                            .color(subtle_dark()),
                                    )
                                    .on_hover_text(
                                        "Fraction of requests for this permutation that are ignored",
                                    );
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
    });
    ui.add_space(6.0);
}

pub(in crate::app) fn is_dialogue_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"udlg"
}

/// First block field whose cleaned name contains `needle` (lowercased).
pub(super) fn find_block_field<'a>(element: &TagStruct<'a>, needle: &str) -> Option<TagBlock<'a>> {
    element.field_names().find_map(|name| {
        if clean_field_name(name).to_ascii_lowercase().contains(needle) {
            element.field(name).and_then(|field| field.as_block())
        } else {
            None
        }
    })
}

/// First field whose cleaned name contains `needle` (lowercased).
pub(super) fn find_field_name_containing<'a>(
    element: &TagStruct<'a>,
    needle: &str,
) -> Option<&'a str> {
    element
        .field_names()
        .find(|name| clean_field_name(name).to_ascii_lowercase().contains(needle))
}

struct DialogueRow {
    name: String,
    sounds: Vec<(u32, String)>,
}

/// A clickable referenced-tag label (filename shown, full path on hover). On
/// click it returns an open request — Alt opens in a floating window.
fn ref_open_label(ui: &mut Ui, group_tag: u32, path: &str) -> Option<OpenTagRequest> {
    let filename = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let clicked = ui
        .add(egui::Label::new(RichText::new(filename).color(text_dark())).sense(Sense::click()))
        .on_hover_text(format!("{path}\n(click to open · Alt: floating)"))
        .clicked();
    clicked.then(|| OpenTagRequest {
        group_tag,

        rel_path: path.to_owned(),
        float: ui.input(|i| i.modifiers.alt),
    })
}

/// Render a row's referenced tags as clickable labels (capped). Returns the
/// first open request triggered this frame.
fn draw_ref_cell(ui: &mut Ui, refs: &[(u32, String)]) -> Option<OpenTagRequest> {
    const SHOWN: usize = 4;
    if refs.is_empty() {
        ui.label(RichText::new("(none)").color(subtle_dark()));
        return None;
    }
    let mut open = None;
    ui.horizontal_wrapped(|ui| {
        for (index, (group_tag, path)) in refs.iter().take(SHOWN).enumerate() {
            if index > 0 {
                ui.label(RichText::new("·").color(subtle_dark()));
            }
            if let Some(request) = ref_open_label(ui, *group_tag, path) {
                open = Some(request);
            }
        }
        if refs.len() > SHOWN {
            ui.label(RichText::new(format!("+{}", refs.len() - SHOWN)).color(subtle_dark()));
        }
    });
    open
}

/// Cross-game-normalized overview of a `dialogue` (`udlg`) tag: one row per
/// vocalization with its identifier and referenced sound(s), each clickable to
/// open the sound tag. Reads whichever layout the game uses — the `sound`
/// reference sits directly on the vocalization (H2/H3/ODST) or nested under a
/// per-vocalization `stimuli` block (Reach/H4/H2A). Classic Halo CE has no
/// vocalization block (fixed per-context fields), so we note that and defer to
/// the field tree.
/// Load a referenced `.sound` tag (from a dialogue or looping container) so its
/// audio can be auditioned/extracted like the primary tag. Classic-aware.
fn load_referenced_sound(
    game: Option<&str>,
    tags_root: Option<&std::path::Path>,
    definitions_root: Option<&std::path::Path>,
    rel_path: &str,
    group: u32,
) -> Option<(TagFile, std::path::PathBuf)> {
    let tags_root = tags_root?;
    let abs = blam_tags::paths::resolve_tag_path(tags_root, rel_path, "sound");
    let tag = crate::source::read_tag_at_path(&abs, game, definitions_root, group).ok()?;
    Some((tag, abs))
}

/// The chosen language when `h2` carries it, else the default — a referenced
/// sound may lack the language the dialogue player was set to.
fn language_for<'a>(h2: Option<&H2Sound>, language: Option<&'a str>) -> Option<&'a str> {
    match h2 {
        Some(h2) => language.filter(|language| h2.has_language(language)),
        None => language,
    }
}

/// The play action for the first playable unit of a (referenced) sound tag:
/// a Wwise event (H4) or the first pitch-range permutation. `sound_rel` is the
/// referenced tag's rel path, used to compute the FMOD subsound id.
fn referenced_sound_play_action(
    tag: &TagFile,
    sound_rel: Option<&str>,
    language: Option<&str>,
) -> Option<super::audio::SoundAction> {
    if let Some((_, name)) = h4_event_names(tag).into_iter().next() {
        return Some(super::audio::SoundAction::PlayEvent {
            event_name: name.clone(),
            label: name,
        });
    }

    let h2 = H2Sound::read(tag);
    let rows = sound_permutation_rows(tag, h2.as_ref());
    let source = RowSource {
        h2: h2.as_ref(),
        language: language_for(h2.as_ref(), language),
        sound_rel,
        multi_pr: rows_span_multiple_pitch_ranges(&rows),
    };
    rows.first()
        .and_then(|row| row_play_action(tag, row, source))
}

/// Extract items for a whole (referenced) sound tag under `base` — Wwise events
/// or pitch-range permutations, matching the primary extractor's layout.
/// `sound_rel` is the referenced tag's rel path (for the FMOD subsound id).
fn referenced_sound_extract_items(
    tag: &TagFile,
    base: &std::path::Path,
    sound_rel: Option<&str>,
    language: Option<&str>,
) -> Vec<ExtractItem> {
    let events = h4_event_names(tag);
    if !events.is_empty() {
        return events
            .iter()
            .map(|(_, name)| ExtractItem {
                out_path: base.join(format!("{}.wav", sanitize_component(name))),
                source: ExtractSource::Event { name: name.clone() },
            })
            .collect();
    }
    let h2 = H2Sound::read(tag);
    let rows = sound_permutation_rows(tag, h2.as_ref());
    let source = RowSource {
        h2: h2.as_ref(),
        language: language_for(h2.as_ref(), language),
        sound_rel,
        multi_pr: false,
    };
    build_extract_items(tag, &rows, source, base, false)
}

/// What a click on a referenced-sound row produced. A container source can only
/// yield `ce_ref`: the referenced tag holds no samples, so the app resolves its
/// Wwise binding after the frame.
#[derive(Default)]
struct ReferencedSoundClick {
    open: Option<OpenTagRequest>,
    play: Option<super::audio::SoundAction>,
    extract: Option<ExtractRequest>,
    ce_ref: Option<CeSoundRefRequest>,
}

impl ReferencedSoundClick {
    /// Fold another row's click into this one. Only one button can be pressed
    /// per frame, so a later row's `Some` simply wins.
    fn take_from(&mut self, other: Self) {
        self.open = other.open.or(self.open.take());
        self.play = other.play.or(self.play.take());
        self.extract = other.extract.or(self.extract.take());
        self.ce_ref = other.ce_ref.or(self.ce_ref.take());
    }

    /// Hand every collected request to the app.
    fn apply(self, edit: &mut FieldEditContext<'_>) {
        if self.open.is_some() {
            *edit.open_request = self.open;
        }
        if self.play.is_some() {
            *edit.sound_play_request = self.play;
        }
        if self.extract.is_some() {
            *edit.sound_extract_request = self.extract;
        }
        if self.ce_ref.is_some() {
            *edit.ce_sound_ref_request = self.ce_ref;
        }
    }
}

/// Render a set of `.sound` refs with a ▶ Play, ⬇ Extract, and the clickable
/// open-label per ref. Shared by the dialogue and sound_looping players; kept out
/// of `edit` so the grid closure needn't borrow it mutably.
///
/// `container_source` marks a Campaign Evolved mount, where there is no tags
/// root to load the referenced tag from — and nothing worth loading if there
/// were, since CE sound tags carry no samples.
fn draw_referenced_sound_cell(
    ui: &mut Ui,
    refs: &[(u32, String)],
    game: Option<&str>,
    tags_root: Option<&std::path::Path>,
    definitions_root: Option<&std::path::Path>,
    language: Option<&str>,
    container_source: bool,
) -> ReferencedSoundClick {
    let mut click = ReferencedSoundClick::default();
    if refs.is_empty() {
        ui.label(RichText::new("(none)").color(subtle_dark()));
        return click;
    }
    ui.vertical(|ui| {
        for (group, path) in refs {
            ui.horizontal(|ui| {
                let is_sound = &group.to_be_bytes() == b"snd!";
                let label = path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
                if is_sound
                    && ui
                        .small_button("\u{25B6}")
                        .on_hover_text("Play this referenced sound")
                        .clicked()
                {
                    if container_source {
                        click.ce_ref = Some(CeSoundRefRequest {
                            group_tag: *group,
                            reference: path.clone(),
                            label: label.clone(),
                            extract: false,
                        });
                    } else if let Some((sound, _)) =
                        load_referenced_sound(game, tags_root, definitions_root, path, *group)
                    {
                        click.play =
                            referenced_sound_play_action(&sound, Some(path.as_str()), language);
                    }
                }
                // Deliberately a second `if`: chaining these would skip drawing
                // the extract button on the frame Play is clicked.
                if is_sound
                    && ui
                        .small_button("\u{2B07}")
                        .on_hover_text(if container_source {
                            "Extract this referenced sound's permutations to a folder"
                        } else {
                            "Extract this referenced sound to its data\\ folder"
                        })
                        .clicked()
                {
                    if container_source {
                        click.ce_ref = Some(CeSoundRefRequest {
                            group_tag: *group,
                            reference: path.clone(),
                            label,
                            extract: true,
                        });
                    } else if let Some((sound, abs)) =
                        load_referenced_sound(game, tags_root, definitions_root, path, *group)
                        && let Some(base) =
                            tags_root.and_then(|root| reimport_base_dir_lang(root, &abs, language))
                    {
                        let items =
                            referenced_sound_extract_items(&sound, &base, Some(path.as_str()), language);
                        click.extract = Some(ExtractRequest {
                            items,
                            tags_root: tags_root.map(std::path::Path::to_path_buf),
                            label,
                        });
                    }
                }
                if let Some(request) = ref_open_label(ui, *group, path) {
                    click.open = Some(request);
                }
            });
        }
    });
    click
}

pub(in crate::app) fn draw_dialogue_summary(
    ui: &mut Ui,
    tag: &TagFile,
    edit: &mut FieldEditContext<'_>,
) {
    let root = tag.root();
    let Some(vocalizations) = find_block_field(&root, "vocali") else {
        ui.label(
            RichText::new(
                "Classic Halo CE dialogue: fixed per-context sound references (no vocalization \
                 block). Edit them in the field tree below.",
            )
            .color(subtle_dark()),
        );
        ui.add_space(6.0);
        return;
    };

    const MAX_ROWS: usize = 600;
    let total = vocalizations.len();
    let mut rows: Vec<DialogueRow> = Vec::new();
    let mut total_sounds = 0usize;
    for index in 0..total.min(MAX_ROWS) {
        let Some(vocal) = vocalizations.element(index) else {
            continue;
        };
        let name = find_field_name_containing(&vocal, "vocali")
            .and_then(|full| vocal.read_string_id(full))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("#{index}"));
        let mut sounds = Vec::new();
        // Direct: a `sound` reference on the vocalization itself.
        if let Some(reference) = find_full_field_name(&vocal, "sound")
            .and_then(|full| vocal.read_tag_ref_with_group(full))
        {
            sounds.push(reference);
        }
        // Nested: each `stimuli` element carries its own `sound` reference.
        if let Some(stimuli) = find_block_field(&vocal, "stimul") {
            for stimulus_index in 0..stimuli.len() {
                if let Some(reference) = stimuli.element(stimulus_index).and_then(|stimulus| {
                    find_full_field_name(&stimulus, "sound")
                        .and_then(|full| stimulus.read_tag_ref_with_group(full))
                }) {
                    sounds.push(reference);
                }
            }
        }
        total_sounds += sounds.len();
        rows.push(DialogueRow { name, sounds });
    }

    let mut clicked = ReferencedSoundClick::default();
    // Copies so the grid closure needn't borrow `edit`.
    let game = edit.game;
    let tags_root = edit.tags_root;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
    let container_source = edit.ce_paks_root.is_some();
    let languages = language_choices(edit, None);
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Dialogue Overview ({total} vocalizations, {total_sounds} sounds)"
        ))
        .strong()
        .color(text_dark()),
    )
    .id_salt("dialogue_overview")
    .default_open(total <= 40)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit, &languages);
        if total == 0 {
            ui.label(RichText::new("(no vocalizations)").color(subtle_dark()));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("dialogue_overview_scroll")
            .max_height(280.0)
            .show(ui, |ui| {
                egui::Grid::new("dialogue_overview_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for header in ["vocalization", "sound(s)"] {
                            ui.label(RichText::new(header).strong().color(subtle_dark()));
                        }

                        ui.end_row();
                        for row in &rows {
                            ui.label(RichText::new(&row.name).color(text_dark()));
                            let click = draw_referenced_sound_cell(
                                ui,
                                &row.sounds,
                                game,
                                tags_root,
                                defs,
                                language,
                                container_source,
                            );
                            clicked.take_from(click);

                            ui.end_row();
                        }
                    });
                if total > MAX_ROWS {
                    ui.label(
                        RichText::new(format!(
                            "… {} more vocalizations not shown",
                            total - MAX_ROWS
                        ))
                        .color(subtle_dark()),
                    );
                }
            });
    });
    clicked.apply(edit);
    ui.add_space(6.0);
}

/// The `sound_looping` (`lsnd`) tag group — a container of `.sound` refs.
pub(in crate::app) fn is_sound_looping_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"lsnd"
}

/// Collect labeled `.sound` refs from a `sound_looping` (`lsnd`) tag: each
/// track's in/loop/out/alt* references, plus each detail sound.
fn sound_looping_refs(tag: &TagFile) -> Vec<(String, u32, String)> {
    let root = tag.root();
    let mut out = Vec::new();
    if let Some(tracks) = find_block_field(&root, "track") {
        for index in 0..tracks.len() {
            let Some(track) = tracks.element(index) else {
                continue;
            };
            let track_name = find_full_field_name(&track, "name")
                .and_then(|full| track.read_string_id(full))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("track {index}"));
            for (label, group, path) in struct_tag_refs_labeled(&track) {
                if &group.to_be_bytes() == b"snd!" {
                    out.push((format!("{track_name} \u{00B7} {label}"), group, path));
                }
            }
        }
    }
    if let Some(details) = find_block_field(&root, "detail sound") {
        for index in 0..details.len() {
            let Some(detail) = details.element(index) else {
                continue;
            };
            for (label, group, path) in struct_tag_refs_labeled(&detail) {
                if &group.to_be_bytes() == b"snd!" {
                    out.push((format!("detail {index} \u{00B7} {label}"), group, path));
                }
            }
        }
    }
    out
}

/// Audition/extract panel for a `sound_looping` (`lsnd`) tag: it carries no audio
/// itself, only `.sound` refs, so we resolve each component sound and reuse the
/// per-sound player (load-on-click, like the dialogue player).
pub(in crate::app) fn draw_sound_looping_player(
    ui: &mut Ui,
    tag: &TagFile,
    edit: &mut FieldEditContext<'_>,
) {
    let refs = sound_looping_refs(tag);
    if refs.is_empty() {
        return;
    }
    let mut clicked = ReferencedSoundClick::default();
    let game = edit.game;
    let tags_root = edit.tags_root;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
    let container_source = edit.ce_paks_root.is_some();
    let languages = language_choices(edit, None);
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Sound Looping \u{2014} {} component sound(s)",
            refs.len()
        ))
        .color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit, &languages);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                egui::Grid::new("sound_looping_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (label, group, path) in &refs {
                            ui.label(RichText::new(label).color(subtle_dark()));
                            let one = [(*group, path.clone())];
                            clicked.take_from(draw_referenced_sound_cell(
                                ui,
                                &one,
                                game,
                                tags_root,
                                defs,
                                language,
                                container_source,
                            ));
                            ui.end_row();
                        }
                    });
            });
    });
    clicked.apply(edit);
    ui.add_space(6.0);
}

/// The `material_effects` (`foot`) tag group.
pub(in crate::app) fn is_material_effects_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"foot"
}

/// All block fields of a struct, paired with their cleaned display label.
pub(super) fn block_fields<'a>(element: &TagStruct<'a>) -> Vec<(String, TagBlock<'a>)> {
    element
        .field_names()
        .filter_map(|name| {
            element
                .field(name)
                .and_then(|field| field.as_block())
                .map(|block| (clean_field_name(name), block))
        })
        .collect()
}

/// Every set tag reference (group, path) on a struct (skips empty references).
/// Value-based so it doesn't depend on the field's name (which varies: `effect`/
/// `sound` in CE vs `tag (effect or sound)`/`secondary tag` in modern games).
fn struct_tag_refs(element: &TagStruct) -> Vec<(u32, String)> {
    element
        .field_names()
        .filter_map(|name| element.read_tag_ref_with_group(name))
        .filter(|(_, path)| !path.is_empty())
        .collect()
}

/// Every set tag reference on a struct paired with its cleaned field-name label
/// (`(label, group, path)`) — used to name a sound_looping track's in/loop/out
/// components.
fn struct_tag_refs_labeled(element: &TagStruct) -> Vec<(String, u32, String)> {
    element
        .field_names()
        .filter_map(|name| {
            element
                .read_tag_ref_with_group(name)
                .filter(|(_, path)| !path.is_empty())
                .map(|(group, path)| (clean_field_name(name), group, path))
        })
        .collect()
}

struct MaterialEffectRow {
    effect: usize,
    block: String,
    name: String,
    tags: Vec<(u32, String)>,
}

/// Cross-game-normalized overview of a `material_effects` (`foot`) tag. Flattens
/// the `effects` block and each effect's per-material sub-blocks into rows of
/// (effect #, block, material name, referenced tag(s)). Deprecated `old
/// materials` sub-blocks are skipped; references are read by value so the
/// CE (`effect`/`sound`) and modern (`tag`/`secondary tag`) field names both
/// work. Each referenced tag is clickable to open it.
pub(in crate::app) fn draw_material_effects_summary(
    ui: &mut Ui,
    tag: &TagFile,
    edit: &mut FieldEditContext<'_>,
) {
    let Some(effects) = find_block_field(&tag.root(), "effect") else {
        return;
    };

    const MAX_ROWS: usize = 600;
    let mut rows: Vec<MaterialEffectRow> = Vec::new();
    let mut total_refs = 0usize;
    'outer: for effect_index in 0..effects.len() {
        let Some(effect) = effects.element(effect_index) else {
            continue;
        };
        for (block_label, materials) in block_fields(&effect) {
            if block_label.to_ascii_lowercase().contains("old") {
                continue; // skip deprecated "old materials (DO NOT USE)" blocks
            }
            for material_index in 0..materials.len() {
                let Some(material) = materials.element(material_index) else {
                    continue;
                };
                let name = find_field_name_containing(&material, "material name")
                    .and_then(|full| material.read_string_id(full))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("#{material_index}"));

                let tags = struct_tag_refs(&material);
                total_refs += tags.len();
                rows.push(MaterialEffectRow {
                    effect: effect_index,
                    block: block_label.clone(),
                    name,
                    tags,
                });
                if rows.len() >= MAX_ROWS {
                    break 'outer;
                }
            }
        }
    }

    let truncated = rows.len() >= MAX_ROWS;
    let mut to_open: Option<OpenTagRequest> = None;
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Material Effects Overview ({} effects, {total_refs} references)",
            effects.len()
        ))
        .strong()
        .color(text_dark()),
    )
    .id_salt("material_effects_overview")
    .default_open(rows.len() <= 40)
    .show(ui, |ui| {
        if rows.is_empty() {
            ui.label(RichText::new("(no material entries)").color(subtle_dark()));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("material_effects_overview_scroll")
            .max_height(280.0)
            .show(ui, |ui| {
                egui::Grid::new("material_effects_overview_grid")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        for header in ["effect", "block", "material", "tag(s)"] {
                            ui.label(RichText::new(header).strong().color(subtle_dark()));
                        }
                        ui.end_row();
                        for row in &rows {
                            ui.label(
                                RichText::new(format!("#{}", row.effect)).color(subtle_dark()),
                            );
                            ui.label(RichText::new(&row.block).color(subtle_dark()));
                            ui.label(RichText::new(&row.name).color(text_dark()));
                            if let Some(request) = draw_ref_cell(ui, &row.tags) {
                                to_open = Some(request);
                            }
                            ui.end_row();
                        }
                    });
                if truncated {
                    ui.label(RichText::new("… more rows not shown").color(subtle_dark()));
                }
            });
    });
    if to_open.is_some() {
        *edit.open_request = to_open;
    }
    ui.add_space(6.0);
}
