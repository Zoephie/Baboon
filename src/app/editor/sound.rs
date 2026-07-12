//! Sound, dialogue, looping-sound, and material-effects presentation.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

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

/// How a permutation row sources its audio: an FMOD bank subsound (Halo 3+), an
/// inline blob on the permutation (CE), or an inline Opus/Xbox-ADPCM blob in
/// H2's parallel language-permutation-info block. CE stores the whole
/// per-permutation `samples` stream in one of four formats (PCM / Xbox-ADPCM /
/// IMA-ADPCM / Ogg Vorbis), so its codec/channels/rate are read from the tag
/// rather than assumed to be Ogg.
pub(super) enum RowKind {
    Bank,
    InlineCe {
        codec: super::audio::InlineCodec,
        channels: u16,
        sample_rate: u32,
    },
    InlineH2 {
        blob: usize,
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
}

/// Extract a CE permutation's inline `samples` bytes (a self-contained Ogg).
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

/// Walk an H2 `.sound` tag's inline audio blobs — the first non-empty `data`
/// field (the samples) in each language-permutation-info raw-info entry. Returns
/// the total count, and the `want`-th blob if requested. Counting is a cheap
/// borrow-only walk; pass `want` to clone exactly one blob.
pub(super) fn h2_blobs(tag: &TagFile, want: Option<usize>) -> (usize, Option<Vec<u8>>) {
    let root = tag.root();
    let mut count = 0usize;
    let mut got = None;
    for field in root.fields() {
        let Some(block) = field.as_block() else {
            continue;
        };
        for i in 0..block.len() {
            let Some(el) = block.element(i) else {
                continue;
            };
            let Some(lang_perm_info) = find_block_field(&el, "language permutation info") else {
                continue;
            };
            for j in 0..lang_perm_info.len() {
                let Some(lpi_el) = lang_perm_info.element(j) else {
                    continue;
                };

                let Some(raw_info) = find_block_field(&lpi_el, "raw info block") else {
                    continue;
                };
                for k in 0..raw_info.len() {
                    let Some(raw_el) = raw_info.element(k) else {
                        continue;
                    };
                    let samples = raw_el
                        .fields()
                        .find_map(|f| f.as_data().filter(|d| !d.is_empty()));
                    let Some(bytes) = samples else {
                        continue;
                    };
                    if want == Some(count) {
                        got = Some(bytes.to_vec());
                    }
                    count += 1;
                }
            }
        }
    }
    (count, got)
}

/// The per-chunk byte offsets of the `want`-th H2 inline blob, from its
/// `sound_permutation_chunk_block` (each element's `file offset`). H2 splits a
/// permutation's audio into ~1.36 s chunks, each an independent stream; these
/// offsets let the decoder slice + concatenate them. Empty if unchunked.
/// Mirrors [`h2_blobs`]'s traversal so ordinals line up.
pub(super) fn h2_blob_chunk_offsets(tag: &TagFile, want: usize) -> Vec<usize> {
    let root = tag.root();
    let mut count = 0usize;
    for field in root.fields() {
        let Some(block) = field.as_block() else {
            continue;
        };
        for i in 0..block.len() {
            let Some(el) = block.element(i) else {
                continue;
            };
            let Some(lang_perm_info) = find_block_field(&el, "language permutation info") else {
                continue;
            };
            for j in 0..lang_perm_info.len() {
                let Some(lpi_el) = lang_perm_info.element(j) else {
                    continue;
                };
                let Some(raw_info) = find_block_field(&lpi_el, "raw info block") else {
                    continue;
                };
                for k in 0..raw_info.len() {
                    let Some(raw_el) = raw_info.element(k) else {
                        continue;
                    };
                    let has_samples = raw_el
                        .fields()
                        .any(|f| f.as_data().is_some_and(|d| !d.is_empty()));
                    if !has_samples {
                        continue;
                    }
                    if count == want {
                        return chunk_offsets_of(&raw_el);
                    }
                    count += 1;
                }
            }
        }
    }
    Vec::new()
}

/// Read the `file offset` of each `sound_permutation_chunk_block` element in a
/// raw-info-block struct (the block whose elements carry a `file offset` field).
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

/// Read H2's tag-level inline codec parameters: `compression` → codec,
/// `encoding` → channel count, `sample rate` → Hz (used only by ADPCM; Opus is
/// always 48 kHz).
pub(super) fn h2_codec_params(tag: &TagFile) -> (super::audio::InlineCodec, u16, u32) {
    use super::audio::InlineCodec;
    let root = tag.root();
    let compression = find_full_field_name(&root, "compression")
        .and_then(|full| root.read_enum_name(full))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let codec = if compression.contains("opus") {
        InlineCodec::Opus
    } else if compression.contains("none") {
        // Uncompressed PCM — "none (big endian)" / "none (little endian)".
        InlineCodec::Pcm {
            big_endian: compression.contains("big"),
        }
    } else {
        InlineCodec::XboxAdpcm
    };
    // Channel count from the `encoding` enum by NAME — the enum ordering differs
    // between games (H2: mono,stereo,codec,quad; H3/Reach: mono,stereo,quad,5.1,
    // codec), so an index-based map would be wrong.
    let channels = find_full_field_name(&root, "encoding")
        .and_then(|full| root.read_enum_name(full))
        .map(|name| {
            let n = name.to_ascii_lowercase();
            if n.contains("mono") {
                1
            } else if n.contains("5.1") {
                6
            } else if n.contains("quad") {
                4
            } else {
                2 // stereo, codec
            }
        })
        .unwrap_or(2);
    let sample_rate = find_full_field_name(&root, "sample rate")
        .and_then(|full| root.read_enum_name(full))
        .map(|name| {
            let n = name.to_ascii_lowercase();
            if n.contains("48") {
                48_000
            } else if n.contains("44") {
                44_100
            } else if n.contains("32") {
                32_000
            } else if n.contains("22") {
                22_050
            } else {
                48_000
            }
        })
        .unwrap_or(48_000);
    (codec, channels, sample_rate)
}

/// Read CE's tag-level inline codec parameters: `format` → codec, `channel
/// count` → channels, `sample rate` → Hz. Unlike H2, CE stores the whole stream
/// per permutation and can use any of four formats — Ogg Vorbis is common but
/// weapon/effect sounds are frequently Xbox-ADPCM, so the format must be read
/// (not assumed) or the Ogg decoder chokes on the ADPCM bytes.
pub(super) fn ce_codec_params(tag: &TagFile) -> (super::audio::InlineCodec, u16, u32) {
    use super::audio::InlineCodec;
    let root = tag.root();
    let format = find_full_field_name(&root, "format")
        .and_then(|full| root.read_enum_name(full))
        .unwrap_or_default()
        .to_ascii_lowercase();
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
    let channels = find_full_field_name(&root, "channel count")
        .and_then(|full| root.read_enum_name(full))
        .map(|name| {
            if name.to_ascii_lowercase().contains("mono") {
                1
            } else {
                2
            }
        })
        .unwrap_or(1);
    let sample_rate = find_full_field_name(&root, "sample rate")
        .and_then(|full| root.read_enum_name(full))
        .map(|name| if name.contains("44") { 44_100 } else { 22_050 })
        .unwrap_or(22_050);
    (codec, channels, sample_rate)
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

/// Localized languages available for the current source, by game family (Wwise
/// `.pck` subdirs for H4/H2A, FMOD `.fsb` for the rest). Empty ⇒ single-language.
fn available_sound_languages(edit: &FieldEditContext<'_>) -> Vec<String> {
    let Some(root) = edit.tags_root else {
        return Vec::new();
    };
    match edit.game {
        Some("halo4_mcc") | Some("halo2amp_mcc") => {
            blam_tags::audio::WwiseBanks::available_languages(root)
        }

        _ => blam_tags::audio::SoundBanks::available_languages(root),
    }
}

/// Shared transport row for every sound-player variant: Stop, a volume slider, a
/// language selector (when the source is localized), and the status line. All
/// changes queue a [`super::audio::SoundAction`] the app drains after rendering.
fn draw_sound_transport(ui: &mut Ui, edit: &mut FieldEditContext<'_>) {
    let languages = available_sound_languages(edit);
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
        // Language selector — resolves banks/pcks (and routes extraction to
        // `data_<lang>\`) for the chosen localized language.
        if !languages.is_empty() {
            let current = edit.sound_language.map(str::to_owned);
            let mut selected = current.clone();
            egui::ComboBox::from_id_salt("sound_language")
                .selected_text(format!(
                    "\u{1F310} {}",
                    current.as_deref().unwrap_or("default")
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, "default");
                    for lang in &languages {
                        ui.selectable_value(&mut selected, Some(lang.clone()), lang);
                    }
                });
            if selected != current {
                *edit.sound_play_request = Some(super::audio::SoundAction::SetLanguage(selected));
            }
        }
        if let Some(status) = edit.sound_status {
            ui.label(RichText::new(status).color(subtle_dark()));
        }
    });
}

/// Build the audition/extraction rows for a `.sound` tag: every pitch-range
/// permutation with its name and audio source, classified identically for the
/// player and the extractor. Capped so a pathological tag can't stall the UI.
pub(super) fn sound_permutation_rows(tag: &TagFile) -> Vec<SoundPermRow> {
    let root = tag.root();
    let Some(pitch_ranges) = find_block_field(&root, "pitch range") else {
        return Vec::new();
    };
    const MAX_ROWS: usize = 400;
    // H2 stores audio in a parallel language-permutation-info block (not on the
    // permutation like CE); count its blobs so rows can map to them by order.
    // Gate strictly on an inline-decodable codec: H3/ODST use the SAME nested
    // block but store XMA `samples` inline (undecodable here; the playable audio
    // is FMOD-Vorbis in the bank), so treating them as inline-H2 would decode
    // garbage. Only opus / xbox-adpcm / uncompressed are actually inline audio.
    let comp = find_full_field_name(&root, "compression")
        .and_then(|full| root.read_enum_name(full))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let inline_decodable = !comp.contains("xma")
        && (comp.contains("opus")
            || comp.contains("adpcm")
            || comp.contains("none")
            || comp.contains("pcm"));
    let h2_count = if inline_decodable {
        h2_blobs(tag, None).0
    } else {
        0
    };
    let mut h2_ordinal = 0usize;
    // CE codec/channels/rate (read once, lazily — only CE tags have inline
    // per-permutation `samples`).
    let mut ce_params: Option<(super::audio::InlineCodec, u16, u32)> = None;
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
            let has_inline_samples = find_full_field_name(&perm, "samples")
                .and_then(|full| perm.field(full))
                .and_then(|field| field.as_data())
                .is_some_and(|data| !data.is_empty());
            let kind = if has_inline_samples {
                let (codec, channels, sample_rate) =
                    *ce_params.get_or_insert_with(|| ce_codec_params(tag));
                RowKind::InlineCe {
                    codec,
                    channels,
                    sample_rate,
                }
            } else if h2_count > 0 {
                let blob = h2_ordinal.min(h2_count - 1);
                h2_ordinal += 1;
                RowKind::InlineH2 { blob }
            } else {
                RowKind::Bank
            };
            rows.push(SoundPermRow {
                pitch_range: pr_name.clone(),
                name,
                pr_index,
                perm_index,
                kind,
            });
        }
    }
    rows
}

/// The play action for a permutation row (bank subsound / CE inline Ogg / H2
/// inline blob). Returns `None` if the inline bytes can't be re-extracted.
fn row_play_action(
    tag: &TagFile,
    row: &SoundPermRow,
    h2_params: Option<(super::audio::InlineCodec, u16, u32)>,
) -> Option<super::audio::SoundAction> {
    use super::audio::{InlineCodec, SoundAction};
    match &row.kind {
        RowKind::Bank => Some(SoundAction::Play {
            key: row.name.clone(),
            label: row.name.clone(),
        }),
        RowKind::InlineCe {
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
        RowKind::InlineH2 { blob } => {
            let bytes = h2_blobs(tag, Some(*blob)).1?;
            let (codec, channels, sample_rate) =
                h2_params.unwrap_or((InlineCodec::Opus, 1, 48_000));
            Some(SoundAction::PlayInline {
                bytes,
                codec,
                channels,
                sample_rate,
                chunk_offsets: h2_blob_chunk_offsets(tag, *blob),
                label: row.name.clone(),
            })
        }
    }
}

/// Where a permutation row's audio comes from for extraction. Mirrors
/// [`row_play_action`] but yields file-writing sources. `raw_ce` writes CE's
/// self-contained inline Ogg verbatim (near-lossless) instead of decoding.
fn row_extract_source(
    tag: &TagFile,
    row: &SoundPermRow,
    h2_params: Option<(super::audio::InlineCodec, u16, u32)>,
    raw_ce: bool,
) -> Option<ExtractSource> {
    use super::audio::InlineCodec;
    match &row.kind {
        RowKind::Bank => Some(ExtractSource::Bank {
            key: row.name.clone(),
        }),
        RowKind::InlineCe {
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
        RowKind::InlineH2 { blob } => {
            let bytes = h2_blobs(tag, Some(*blob)).1?;
            let (codec, channels, sample_rate) =
                h2_params.unwrap_or((InlineCodec::Opus, 1, 48_000));
            Some(ExtractSource::Inline {
                bytes,
                codec,
                channels,
                sample_rate,
                chunk_offsets: h2_blob_chunk_offsets(tag, *blob),
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
            RowKind::InlineCe {
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
    let class = find_full_field_name(&root, "sound class")
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
/// exporter).
pub(super) fn build_extract_items(
    tag: &TagFile,
    rows: &[SoundPermRow],
    h2_params: Option<(super::audio::InlineCodec, u16, u32)>,
    base: &std::path::Path,
    raw_ce: bool,
) -> Vec<ExtractItem> {
    let multi_pr = rows_span_multiple_pitch_ranges(rows);
    rows.iter()
        .filter_map(|row| {
            let source = row_extract_source(tag, row, h2_params, raw_ce)?;
            let rel = perm_relative_path(multi_pr, row, row_extract_ext(&row.kind, raw_ce));
            Some(ExtractItem {
                out_path: base.join(rel),
                source,
            })
        })
        .collect()
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
    egui::CollapsingHeader::new(
        RichText::new(format!("Sound \u{2014} Wwise event ({})", events.len())).color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit);
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

pub(in crate::app) fn draw_sound_player(
    ui: &mut Ui,
    tag: &TagFile,
    edit: &mut FieldEditContext<'_>,
) {
    // Halo 4: Wwise event reference, no inline pitch-range audio.
    let events = h4_event_names(tag);
    if !events.is_empty() {
        draw_wwise_event_player(ui, &events, edit);
        return;
    }
    let rows = sound_permutation_rows(tag);
    if rows.is_empty() {
        return;
    }
    // H2 tag-level codec/channels/rate (read once; used by inline H2 rows).
    let is_h2 = rows
        .iter()
        .any(|row| matches!(row.kind, RowKind::InlineH2 { .. }));
    let h2_params = is_h2.then(|| h2_codec_params(tag));
    // The raw-passthrough toggle is only meaningful for CE Ogg-format tags
    // (writing the verbatim stream as `.ogg`); ADPCM/PCM tags must decode to WAV.
    let has_inline_ogg = rows.iter().any(|row| {
        matches!(
            row.kind,
            RowKind::InlineCe {
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

    egui::CollapsingHeader::new(
        RichText::new(format!("Sound \u{2014} {} permutation(s)", rows.len())).color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit);

        // Class / codec readout.
        let (class, codec) = sound_class_and_compression(tag);
        ui.horizontal(|ui| {
            if let Some(class) = &class {
                ui.label(RichText::new(format!("class: {class}")).color(subtle_dark()));
                ui.label(RichText::new("\u{00B7}").color(subtle_dark()));
            }
            ui.label(RichText::new(format!("codec: {codec}")).color(subtle_dark()));
        });

        // Extract-all. The CE raw-ogg toggle persists per tag. (Reimport is left
        // to the user via the game's tool.exe.)
        let raw_ce_id = ui.make_persistent_id(("sound_raw_ce", edit.tag_key));
        let mut raw_ce = ui.data(|d| d.get_temp::<bool>(raw_ce_id)).unwrap_or(false);
        let extract_base =
            abs_tag_path
                .as_deref()
                .zip(edit.tags_root)
                .and_then(|(tag_path, root)| {
                    reimport_base_dir_lang(root, tag_path, edit.sound_language)
                });
        let format_note = sound_format_note(edit.game);
        ui.horizontal(|ui| {
            let extract_hover = match &extract_base {
                Some(dir) => format!("Extract every permutation to {}", dir.display()),
                None => "Choose a folder and extract every permutation".to_owned(),
            };
            let extract_hover = if format_note.is_empty() {
                extract_hover
            } else {
                format!("{extract_hover}\nFor tool.exe reimport: {format_note}")
            };
            if ui
                .button(RichText::new("\u{2B07} Extract all"))
                .on_hover_text(extract_hover)
                .clicked()
            {
                let base = extract_base.clone().or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Extract sound permutations")
                        .pick_folder()
                });
                if let Some(base) = base {
                    let items = build_extract_items(tag, &rows, h2_params, &base, raw_ce);
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
                    .num_columns(4)
                    .show(ui, |ui| {
                        for row in &rows {
                            let hover = match row.kind {
                                RowKind::Bank => "Play this permutation from the FMOD bank",
                                _ => "Play this permutation (inline tag audio)",
                            };
                            if ui.small_button("\u{25B6}").on_hover_text(hover).clicked() {
                                *edit.sound_play_request = row_play_action(tag, row, h2_params);
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
                                {
                                    if let Some(source) =
                                        row_extract_source(tag, row, h2_params, raw_ce)
                                    {
                                        *edit.sound_extract_request = Some(ExtractRequest {
                                            items: vec![ExtractItem {
                                                out_path: path,
                                                source,
                                            }],
                                            tags_root: edit
                                                .tags_root
                                                .map(std::path::Path::to_path_buf),
                                            label: row.name.clone(),
                                        });
                                    }
                                }
                            }
                            ui.label(RichText::new(&row.name).color(text_dark()));
                            ui.label(RichText::new(&row.pitch_range).color(subtle_dark()));
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

/// The play action for the first playable unit of a (referenced) sound tag:
/// a Wwise event (H4) or the first pitch-range permutation.
fn referenced_sound_play_action(tag: &TagFile) -> Option<super::audio::SoundAction> {
    if let Some((_, name)) = h4_event_names(tag).into_iter().next() {
        return Some(super::audio::SoundAction::PlayEvent {
            event_name: name.clone(),
            label: name,
        });
    }

    let rows = sound_permutation_rows(tag);
    let is_h2 = rows
        .iter()
        .any(|row| matches!(row.kind, RowKind::InlineH2 { .. }));
    let h2_params = is_h2.then(|| h2_codec_params(tag));
    rows.first()
        .and_then(|row| row_play_action(tag, row, h2_params))
}

/// Extract items for a whole (referenced) sound tag under `base` — Wwise events
/// or pitch-range permutations, matching the primary extractor's layout.
fn referenced_sound_extract_items(tag: &TagFile, base: &std::path::Path) -> Vec<ExtractItem> {
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
    let rows = sound_permutation_rows(tag);
    let is_h2 = rows
        .iter()
        .any(|row| matches!(row.kind, RowKind::InlineH2 { .. }));
    let h2_params = is_h2.then(|| h2_codec_params(tag));
    build_extract_items(tag, &rows, h2_params, base, false)
}

/// Render a set of `.sound` refs with a ▶ Play, ⬇ Extract, and the clickable
/// open-label per ref. Shared by the dialogue and sound_looping players; kept out
/// of `edit` so the grid closure needn't borrow it mutably.
fn draw_referenced_sound_cell(
    ui: &mut Ui,
    refs: &[(u32, String)],
    game: Option<&str>,
    tags_root: Option<&std::path::Path>,
    definitions_root: Option<&std::path::Path>,
    language: Option<&str>,
) -> (
    Option<OpenTagRequest>,
    Option<super::audio::SoundAction>,
    Option<ExtractRequest>,
) {
    let (mut open, mut play, mut extract) = (None, None, None);
    if refs.is_empty() {
        ui.label(RichText::new("(none)").color(subtle_dark()));
        return (open, play, extract);
    }
    ui.vertical(|ui| {
        for (group, path) in refs {
            ui.horizontal(|ui| {
                let is_sound = &group.to_be_bytes() == b"snd!";
                if is_sound
                    && ui
                        .small_button("\u{25B6}")
                        .on_hover_text("Play this referenced sound")
                        .clicked()
                {
                    if let Some((sound, _)) =
                        load_referenced_sound(game, tags_root, definitions_root, path, *group)
                    {
                        play = referenced_sound_play_action(&sound);
                    }
                }
                if is_sound
                    && ui
                        .small_button("\u{2B07}")
                        .on_hover_text("Extract this referenced sound to its data\\ folder")
                        .clicked()
                {
                    if let Some((sound, abs)) =
                        load_referenced_sound(game, tags_root, definitions_root, path, *group)
                    {
                        if let Some(base) =
                            tags_root.and_then(|root| reimport_base_dir_lang(root, &abs, language))
                        {
                            let items = referenced_sound_extract_items(&sound, &base);
                            let label = path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
                            extract = Some(ExtractRequest {
                                items,
                                tags_root: tags_root.map(std::path::Path::to_path_buf),
                                label,
                            });
                        }
                    }
                }
                if let Some(request) = ref_open_label(ui, *group, path) {
                    open = Some(request);
                }
            });
        }
    });
    (open, play, extract)
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

    let mut to_open: Option<OpenTagRequest> = None;
    let mut to_play: Option<super::audio::SoundAction> = None;
    let mut to_extract: Option<ExtractRequest> = None;
    // Copies so the grid closure needn't borrow `edit`.
    let game = edit.game;
    let tags_root = edit.tags_root;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
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
        draw_sound_transport(ui, edit);
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
                            let (open, play, extract) = draw_referenced_sound_cell(
                                ui,
                                &row.sounds,
                                game,
                                tags_root,
                                defs,
                                language,
                            );
                            if open.is_some() {
                                to_open = open;
                            }
                            if play.is_some() {
                                to_play = play;
                            }
                            if extract.is_some() {
                                to_extract = extract;
                            }

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
    if to_open.is_some() {
        *edit.open_request = to_open;
    }
    if to_play.is_some() {
        *edit.sound_play_request = to_play;
    }
    if to_extract.is_some() {
        *edit.sound_extract_request = to_extract;
    }
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
    let mut to_open: Option<OpenTagRequest> = None;
    let mut to_play: Option<super::audio::SoundAction> = None;
    let mut to_extract: Option<ExtractRequest> = None;
    let game = edit.game;
    let tags_root = edit.tags_root;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Sound Looping \u{2014} {} component sound(s)",
            refs.len()
        ))
        .color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        draw_sound_transport(ui, edit);
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
                            let (open, play, extract) = draw_referenced_sound_cell(
                                ui, &one, game, tags_root, defs, language,
                            );
                            if open.is_some() {
                                to_open = open;
                            }
                            if play.is_some() {
                                to_play = play;
                            }
                            if extract.is_some() {
                                to_extract = extract;
                            }
                            ui.end_row();
                        }
                    });
            });
    });
    if to_open.is_some() {
        *edit.open_request = to_open;
    }
    if to_play.is_some() {
        *edit.sound_play_request = to_play;
    }
    if to_extract.is_some() {
        *edit.sound_extract_request = to_extract;
    }
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
