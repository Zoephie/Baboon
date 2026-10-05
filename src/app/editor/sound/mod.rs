//! Sound, dialogue, looping-sound, and material-effects presentation.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

mod h2;
pub(in crate::app) use h2::*;
mod player;
#[cfg(test)]
pub(super) use player::clip_selection_id;
pub(in crate::app) use player::forget_closed_players;
use player::*;

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

/// A name field's text, whichever way the game stores it: a string id in
/// Halo 2 onward, a fixed 32-character `string` in Halo CE (permutation and
/// pitch-range names), or a long string. Reading only string ids named every
/// Halo CE row by its index (`#0`, `pitch range 0`), and extraction wrote
/// `#0.wav`, losing the names the tag holds.
pub(in crate::app) fn read_name_text(element: &TagStruct<'_>, full: &str) -> Option<String> {
    element
        .read_string_id(full)
        .or_else(|| element.read_string(full))
        .or_else(|| element.read_long_string(full))
        .filter(|name| !name.is_empty())
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

/// A Halo CE permutation's audio with the pieces chained after it: the bytes of
/// each, end to end, and where each starts (empty when there is one piece).
///
/// CE stores a long sound, mostly music, as a chain: the pitch range's first
/// `actual permutation count` permutations are the ones a sound plays, and each
/// names the piece that continues it in `next permutation index`, until -1.
/// 268 pitch ranges in Halo CE's kit chain this way, `long.sound`'s into 122
/// pieces. Each piece is a whole stream of its own, so they are decoded one at
/// a time and joined. A pitch range whose actual count is 0 chains nothing:
/// its `next` fields are unset (0), and every permutation stands alone.
pub(super) fn inline_permutation_chain(
    tag: &TagFile,
    pr_index: usize,
    perm_index: usize,
) -> Option<(Vec<u8>, Vec<usize>)> {
    let root = tag.root();
    let pitch_ranges = find_block_field(&root, "pitch range")?;
    let pitch_range = pitch_ranges.element(pr_index)?;
    let permutations = find_block_field(&pitch_range, "permutation")?;
    let samples = |index: usize| -> Option<Vec<u8>> {
        let perm = permutations.element(index)?;
        let data = perm.field(find_full_field_name(&perm, "samples")?)?.as_data()?;
        (!data.is_empty()).then(|| data.to_vec())
    };
    let mut bytes = samples(perm_index)?;
    let mut offsets = vec![0];
    if actual_permutation_count(&pitch_range) > 0 {
        let mut seen = std::collections::HashSet::from([perm_index]);
        let mut current = perm_index;
        while let Some(next) = next_permutation(&permutations, current)
            && seen.insert(next)
        {
            let Some(piece) = samples(next) else { break };
            offsets.push(bytes.len());
            bytes.extend_from_slice(&piece);
            current = next;
        }
    }
    if offsets.len() == 1 {
        offsets.clear();
    }
    Some((bytes, offsets))
}

/// A CE pitch range's `actual permutation count`: how many of its permutations
/// are sounds rather than pieces chained after one. 0 where it has none.
fn actual_permutation_count(pitch_range: &TagStruct) -> usize {
    find_full_field_name(pitch_range, "actual permutation count")
        .and_then(|full| pitch_range.read_int_any(full))
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0)
}

/// The permutation chained after `index`, if it names one in range.
fn next_permutation(permutations: &TagBlock, index: usize) -> Option<usize> {
    let perm = permutations.element(index)?;
    let next = perm.read_int_any(find_full_field_name(&perm, "next permutation index")?)?;
    usize::try_from(next).ok().filter(|&next| next < permutations.len())
}

/// Read the `file offset` of each `sound_permutation_chunk_block` element in a
/// raw-info-block struct (the block whose elements carry a `file offset` field).
/// H2 splits an entry's audio into ~1.36 s chunks; see
/// `decode_inline_chunked` for how each codec's are joined. Empty if unchunked.
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
/// Halo CE and Halo 2's older layout both name them `compression` (on the
/// permutation, else the tag) / `encoding` / `sample rate`. CE uses Ogg for
/// music but Xbox-ADPCM for most effects, so the compression must be read
/// (the Ogg decoder chokes on ADPCM bytes).
pub(super) fn permutation_inline_params(
    root: &TagStruct,
    perm: &TagStruct,
) -> (super::audio::InlineCodec, u16, u32) {
    let read = |element: &TagStruct, clean: &str| {
        find_full_field_name(element, clean).and_then(|full| element.read_enum_name(full))
    };
    let compression = read(perm, "compression")
        .or_else(|| read(root, "compression"))
        .unwrap_or_default();
    let channels = read(root, "encoding").map_or(1, |name| h2_channels_for(&name));
    let rate = read(root, "sample rate").map_or(22_050, |name| h2_rate_for(&name));
    (h2_codec_for(&compression), channels, rate)
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
            .and_then(|full| read_name_text(&root, full))
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

/// Halo 3-family language catalog from the engine's sound schema. The first
/// value is the FMOD bank stem; the second is the user-facing name.
pub(super) const FMOD_LANGUAGES: [(&str, &str); 12] = [
    ("english", "English"),
    ("japanese", "Japanese"),
    ("german", "German"),
    ("french", "French"),
    ("spanish", "Spanish"),
    ("mexican", "Mexican Spanish"),
    ("italian", "Italian"),
    ("korean", "Korean"),
    ("chinese-traditional", "Chinese (Traditional)"),
    ("chinese-simplified", "Chinese (Simplified)"),
    ("portuguese", "Portuguese"),
    ("polish", "Polish"),
];

/// A language the picker offers: the value kept in the shared audio state
/// (`None` = the source's default) and its label.
pub(super) struct LanguageChoice {
    pub(super) value: Option<String>,
    pub(super) label: String,
    pub(super) available: bool,
    pub(super) unavailable_reason: Option<String>,
}

/// Localized languages available for the current source. Halo 2 carries its
/// languages in the tag (`h2`), English being the default; Wwise `.pck`
/// subdirs for H4/H2A and FMOD `.fsb` banks for the rest, behind a separate
/// "default". Empty ⇒ single-language.
pub(super) fn language_choices(
    edit: &FieldEditContext<'_>,
    h2: Option<&H2Sound>,
) -> Vec<LanguageChoice> {
    let h2_choices = |languages: &mut dyn Iterator<Item = &str>| -> Vec<LanguageChoice> {
        languages
            .map(|language| LanguageChoice {
                value: (!language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE))
                    .then(|| language.to_owned()),
                label: language_label(language),
                available: true,
                unavailable_reason: None,
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
    if edit.game == Some(GameId::Halo2) && edit.ce_sound.is_none() {
        return h2_choices(&mut H2_LANGUAGES.iter().copied());
    }
    let available_choices = |languages: Vec<String>| -> Vec<LanguageChoice> {
        languages
            .into_iter()
            .map(|language| LanguageChoice {
                label: language.clone(),
                value: Some(language),
                available: true,
                unavailable_reason: None,
            })
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
            return available_choices(langs);
        }
    }
    let Some(root) = edit.tags_root else {
        return Vec::new();
    };
    if matches!(
        edit.game,
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach)
    ) {
        let installed = blam_tags::audio::SoundBanks::available_languages(root);
        let bank_dir = root.parent().unwrap_or(root).join("fmod").join("pc");
        return FMOD_LANGUAGES
            .iter()
            .map(|(bank, label)| {
                let installed_name = installed
                    .iter()
                    .find(|candidate| candidate.eq_ignore_ascii_case(bank));
                let available = installed_name.is_some();
                LanguageChoice {
                    value: Some(installed_name.map_or_else(|| (*bank).to_owned(), Clone::clone)),
                    label: (*label).to_owned(),
                    available,
                    unavailable_reason: (!available).then(|| {
                        format!(
                            "Unavailable — {} is not installed",
                            bank_dir.join(format!("{bank}.fsb")).display()
                        )
                    }),
                }
            })
            .collect();
    }
    available_choices(match edit.game {
        Some(GameId::Halo4) | Some(GameId::Halo2Amp) => {
            blam_tags::audio::WwiseBanks::available_languages(root)
        }
        _ => blam_tags::audio::SoundBanks::available_languages(root),
    })
}

/// A slider over `0..=max` read out as a percentage, with `icon` after its
/// value. Dragging stays in the range; a value typed into the box (`150`,
/// `150%`) stands even past it, for the audio state to bound.
fn percent_slider<'a>(value: &'a mut f32, max: f32, icon: &str) -> egui::Slider<'a> {
    egui::Slider::new(value, 0.0..=max)
        .clamping(egui::SliderClamping::Never)
        .text(RichText::new(icon).color(subtle_dark()))
        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
        .custom_parser(|text| {
            text.trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f64>()
                .ok()
                .map(|percent| percent / 100.0)
        })
}

/// Volume, the language choice and the status line, shared by every player.
pub(super) fn draw_sound_output_controls(
    ui: &mut Ui,
    edit: &mut FieldEditContext<'_>,
    languages: &[LanguageChoice],
) {
    let error_status = sound_error_status(edit);
    let mut volume = edit.sound_volume;
    ui.spacing_mut().slider_width = 90.0;
    if ui
        .add(percent_slider(
            &mut volume,
            super::audio::VOLUME_SLIDER_MAX,
            "\u{1F50A}",
        ))
        .on_hover_text("Playback volume; type a value for more than 100%.")
        .changed()
    {
        edit.sound_play_request
            .push_back(super::audio::SoundAction::SetVolume(volume));
    }
    // A slider's label sits after its value, so each icon would otherwise
    // read as the start of the control after it.
    ui.separator();
    let mut speed = edit.sound_speed;
    let response = ui
        .add(percent_slider(
            &mut speed,
            super::audio::SPEED_SLIDER_MAX,
            SPEED_ICON,
        ))
        .on_hover_text(
            "Playback speed; pitch moves with it. Type a value for more than 500%; \
             double-click for 100%.",
        );
    // A slider senses drags only, so its response never reports a click; a
    // double-click is read off the pointer while over it.
    let reset = response.hovered()
        && ui.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
        });
    if reset {
        speed = 1.0;
    }
    if response.changed() || reset {
        edit.sound_play_request
            .push_back(super::audio::SoundAction::SetSpeed(speed));
    }
    // Language selector — picks which localized audio plays and is
    // extracted (to `data_<lang>\`). A language this source lacks shows as
    // the default, which is what plays.
    if !languages.is_empty() {
        ui.separator();
        let current = edit.sound_language.map(str::to_owned);
        let shown = languages
            .iter()
            .find(|choice| {
                choice.available
                    && choice.value.as_deref().map(str::to_ascii_lowercase)
                        == current.as_deref().map(str::to_ascii_lowercase)
            })
            .or_else(|| languages.iter().find(|choice| choice.available))
            .or(languages.first());
        let mut selected = shown.and_then(|choice| choice.value.clone());
        // The selection is shared between editing kits. Commit the visual
        // fallback too: otherwise a language chosen in H2 but absent from
        // H3 continues opening that missing bank while the combo says
        // "default".
        let before = current;
        egui::ComboBox::from_id_salt("sound_language")
            .selected_text(format!(
                "\u{1F310} {}",
                shown.map_or("default", |choice| choice.label.as_str())
            ))
            .show_ui(ui, |ui| {
                for choice in languages {
                    let response = ui
                        .add_enabled_ui(choice.available, |ui| {
                            ui.selectable_value(&mut selected, choice.value.clone(), &choice.label)
                        })
                        .inner;
                    if let Some(reason) = &choice.unavailable_reason {
                        response.on_hover_text(reason);
                    }
                }
            })
            .response
            .on_hover_text(format!(
                "Language to play and extract ({} of {} available)",
                languages.iter().filter(|choice| choice.available).count(),
                languages.len()
            ));
        if selected != before {
            edit.sound_play_request
                .push_back(super::audio::SoundAction::SetLanguage(selected));
        }
    }
    if let Some(status) = edit.sound_status.filter(|_| error_status.is_none()) {
        ui.label(RichText::new(status).color(subtle_dark()));
    }
}

/// A status line that reports a failure, which [`draw_sound_errors`] shows on
/// its own line rather than beside the controls.
fn sound_error_status<'a>(edit: &FieldEditContext<'a>) -> Option<&'a str> {
    edit.sound_status.filter(|status| {
        status.starts_with("FMOD audio unavailable:")
            || status.starts_with("decode failed:")
            || status.starts_with("resolve failed:")
            || status.starts_with("Extraction cancelled")
            || *status == "no audio output device"
    })
}

/// The player's failure line, and the missing-language-bank warning.
pub(super) fn draw_sound_errors(ui: &mut Ui, edit: &FieldEditContext<'_>) {
    let error_status = sound_error_status(edit);
    let missing_fmod_languages = matches!(
        edit.game,
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach)
    )
    .then(|| edit.tags_root)
    .flatten()
    .filter(|root| blam_tags::audio::SoundBanks::available_languages(root).is_empty())
    .map(|root| root.parent().unwrap_or(root).join("fmod").join("pc"));
    if let Some(error) = error_status {
        ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {error}"));
    }
    if let Some(path) = missing_fmod_languages {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "⚠ No localized FMOD language bank is installed in {}. Effects in sfx.fsb can still play, but dialogue requires english.fsb and its matching .fsb.info.",
                path.display()
            ),
        );
    }
}

/// `m:ss.mmm`, the time format the transport shows.
pub(super) fn format_play_time(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{}:{:02}.{:03}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000
    )
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
#[cfg(test)]
pub(super) fn sound_permutation_rows(tag: &TagFile, h2: Option<&H2Sound>) -> Vec<SoundPermRow> {
    sound_permutation_rows_for_game(tag, h2, None)
}

/// Game-aware row classification. H3-family tags carry allocated, zero-filled
/// `samples` placeholders even though their real audio lives in FMOD banks;
/// buffer length alone therefore cannot distinguish them from CE/H2 inline
/// audio.
pub(super) fn sound_permutation_rows_for_game(
    tag: &TagFile,
    h2: Option<&H2Sound>,
    game: Option<GameId>,
) -> Vec<SoundPermRow> {
    let bank_backed = matches!(
        game,
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach)
    );
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
            .and_then(|full| read_name_text(&pitch_range, full))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("pitch range {pr_index}"));
        let Some(permutations) = find_block_field(&pitch_range, "permutation") else {
            continue;
        };
        // Past a CE pitch range's actual count are the pieces its sounds chain
        // into, played as part of them; see `inline_permutation_chain`.
        let actual = actual_permutation_count(&pitch_range);
        for perm_index in 0..permutations.len() {
            if rows.len() >= MAX_ROWS {
                break;
            }
            let Some(perm) = permutations.element(perm_index) else {
                continue;
            };
            let name = find_full_field_name(&perm, "name")
                .and_then(|full| read_name_text(&perm, full))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("#{perm_index}"));
            let inline_bytes = find_full_field_name(&perm, "samples")
                .and_then(|full| perm.field(full))
                .and_then(|field| field.as_data())
                .map_or(0, <[u8]>::len);
            if !bank_backed && h2.is_none() && inline_bytes > 0 && actual > 0 && perm_index >= actual {
                continue;
            }
            let kind = if bank_backed {
                RowKind::Bank
            } else if inline_bytes > 0 {
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

/// Halo 3-family tags can expose fields that resemble Halo 2's inline
/// localization table. Only interpret that layout for an actual Halo 2 source;
/// otherwise theoretical tag languages leak into FMOD bank extraction.
fn h2_sound_for_game(tag: &TagFile, game: Option<GameId>) -> Option<H2Sound> {
    matches!(game, Some(GameId::Halo2))
        .then(|| H2Sound::read(tag))
        .flatten()
}

/// A `.sound` tag's `<tags root>`-relative path without extension (e.g.
/// `.../tags/sound/dialog/.../ambush.sound` → `sound\dialog\...\ambush`), for
/// building the FMOD subsound id. Backslash-normalized; the hash lowercases.
pub(super) fn sound_tag_rel(abs: &std::path::Path, tags_root: &std::path::Path) -> Option<String> {
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

fn is_fmod_language_game(game: Option<GameId>) -> bool {
    matches!(
        game,
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach)
    )
}

fn sound_path_may_have_languages(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("sound/dialog/")
}

/// True when every bank-backed permutation resolves from the shared
/// `sfx.fsb` alone. Passing a deliberately absent language opens only that
/// shared bank, avoiding path/class-name guesses about whether a tag is
/// localized.
fn rows_use_only_shared_fmod_bank(
    rows: &[SoundPermRow],
    sound_rel: Option<&str>,
    banks: &blam_tags::audio::SoundBanks,
) -> bool {
    let multi_pr = rows_span_multiple_pitch_ranges(rows);
    let mut found = false;
    rows.iter()
        .filter(|row| matches!(row.kind, RowKind::Bank))
        .all(|row| {
            found = true;
            let id = row_bank_id(sound_rel, multi_pr, row);
            resolve_sound_bank(banks, id, &row.name).is_some_and(|(bank, _)| {
                banks.bank_paths()[bank]
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("sfx.fsb"))
            })
        })
        && found
}

fn resolve_sound_bank(
    banks: &blam_tags::audio::SoundBanks,
    id: Option<u32>,
    key: &str,
) -> Option<(usize, usize)> {
    match id {
        Some(id) => banks.resolve_by_id(id),
        None => banks.resolve(key),
    }
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
pub(super) fn row_play_action(
    tag: &TagFile,
    row: &SoundPermRow,
    source: RowSource<'_>,
    tags_root: Option<&std::path::Path>,
) -> Option<super::audio::SoundAction> {
    use super::audio::SoundAction;
    match &row.kind {
        RowKind::Bank => Some(SoundAction::Play {
            id: row_bank_id(source.sound_rel, source.multi_pr, row),
            key: row.name.clone(),
            label: row.name.clone(),
            tags_root: tags_root.map(std::path::Path::to_path_buf),
        }),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => {
            let (bytes, chunk_offsets) = inline_permutation_chain(tag, row.pr_index, row.perm_index)?;

            Some(SoundAction::PlayInline {
                bytes,
                codec: *codec,
                channels: *channels,
                sample_rate: *sample_rate,
                chunk_offsets,
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
            language: source.language.map(str::to_owned),
        }),
        RowKind::InlinePermutation {
            codec,
            channels,
            sample_rate,
        } => {
            let (bytes, chunk_offsets) = inline_permutation_chain(tag, row.pr_index, row.perm_index)?;
            // Raw passthrough writes the stream verbatim — only meaningful (and
            // only a valid `.ogg`) when the CE format actually is Ogg Vorbis. A
            // chain's pieces, end to end, are a chained Ogg file.
            let raw_ogg = raw_ce && matches!(codec, InlineCodec::OggVorbis);
            Some(if raw_ogg {
                ExtractSource::Raw(bytes)
            } else {
                ExtractSource::Inline {
                    bytes,
                    codec: *codec,
                    channels: *channels,
                    sample_rate: *sample_rate,
                    chunk_offsets,
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
    let class = find_full_field_name(&root, "class")
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
pub(super) fn rows_span_multiple_pitch_ranges(rows: &[SoundPermRow]) -> bool {
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

/// Build the same reimport-layout extraction an open sound pane would queue,
/// but from a browser entry. This is deliberately UI-free so tag and folder
/// context menus do not have to open documents just to export their audio.
pub(in crate::app) fn browser_sound_extract_items(
    tag: &TagFile,
    abs_tag_path: &std::path::Path,
    layout: &KitLayout,
    game: Option<GameId>,
    selected_language: Option<&str>,
    all_languages: bool,
    shared_fmod_banks: Option<&blam_tags::audio::SoundBanks>,
) -> Vec<ExtractItem> {
    let tags_root = layout.tags.as_path();
    let h2 = h2_sound_for_game(tag, game);
    let sound_rel = sound_tag_rel(abs_tag_path, tags_root);
    let events = h4_event_names(tag);
    let rows = sound_permutation_rows_for_game(tag, h2.as_ref(), game);
    let shared_fmod_audio = is_fmod_language_game(game)
        && shared_fmod_banks.is_some_and(|banks| {
            rows_use_only_shared_fmod_bank(&rows, sound_rel.as_deref(), banks)
        });
    let languages: Vec<Option<String>> = if all_languages {
        if shared_fmod_audio {
            vec![None]
        } else if let Some(h2) = h2.as_ref() {
            if h2.languages.is_empty() {
                vec![None]
            } else {
                h2.languages.iter().cloned().map(Some).collect()
            }
        } else if matches!(game, Some(GameId::HaloCe) | Some(GameId::Halo2)) {
            // Classic inline tags without H2's language table have exactly one
            // stream; repeating it into every bank language would fabricate
            // localizations that do not exist.
            vec![None]
        } else {
            let mut external = match game {
                Some(GameId::Halo4) | Some(GameId::Halo2Amp) => {
                    blam_tags::audio::WwiseBanks::available_languages(tags_root)
                }
                _ => blam_tags::audio::SoundBanks::available_languages(tags_root),
            };
            // Bank selection and output layout are separate concerns: the
            // English audio must be read from the explicit English bank, even
            // though it is written under `data\` rather than `data_english\`.
            // Kits without a localized bank set still use the base FMOD banks.
            let default = external
                .iter()
                .position(|language| is_default_external_language(language))
                .map(|index| Some(external.remove(index)))
                .unwrap_or(None);
            std::iter::once(default)
                .chain(external.into_iter().map(Some))
                .collect()
        }
    } else {
        vec![
            (!shared_fmod_audio)
                .then(|| selected_language.map(str::to_owned))
                .flatten(),
        ]
    };

    let mut items = Vec::new();
    for language in languages {
        let data_language = if h2.is_some() {
            language.as_deref().and_then(h2_data_language)
        } else {
            language
                .as_deref()
                .filter(|language| !is_default_external_language(language))
        };
        let Some(base) = reimport_base_dir_lang(layout, abs_tag_path, data_language) else {
            continue;
        };
        if !events.is_empty() {
            items.extend(events.iter().map(|(_, name)| ExtractItem {
                out_path: base.join(format!("{}.wav", sanitize_component(name))),
                source: ExtractSource::Event {
                    name: name.clone(),
                    language: language.clone(),
                },
            }));
            continue;
        }
        let source = RowSource {
            h2: h2.as_ref(),
            language: language.as_deref(),
            sound_rel: sound_rel.as_deref(),
            multi_pr: rows_span_multiple_pitch_ranges(&rows),
        };
        items.extend(build_extract_items(tag, &rows, source, &base, false));
    }
    items
}

fn is_default_external_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("english") || language.eq_ignore_ascii_case("english(us)")
}

/// The `data\` root name a Halo 2 language extracts under: English is the
/// default (`data\`), the rest `data_<language>\`.
fn h2_data_language(language: &str) -> Option<&str> {
    (!language.eq_ignore_ascii_case(H2_DEFAULT_LANGUAGE)).then_some(language)
}

/// Per-game one-line note on the format `tool.exe` requires when reimporting the
/// extracted files (RE-verified from each tool binary). Empty for Wwise games.
fn sound_format_note(game: Option<GameId>) -> &'static str {
    match game {
        Some(GameId::HaloCe) => "16-bit WAV, 22050 or 44100 Hz, mono/stereo",
        Some(GameId::Halo2) => "16-bit WAV, 22050/32000/44100/48000 Hz (resampled), mono/stereo",
        Some(GameId::Halo3) | Some(GameId::Halo3Odst) | Some(GameId::HaloReach) => {
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
    // Name the field each event comes from only when there is more than one
    // kind of them.
    let grouped = events.iter().any(|(label, _)| *label != events[0].0);
    egui::CollapsingHeader::new(
        RichText::new(format!("Sound \u{2014} Wwise event ({})", events.len())).color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        let language = edit.sound_language.unwrap_or("");
        let clips: Vec<PlayerClip> = events
            .iter()
            .map(|(label, name)| PlayerClip {
                id: format!("event:{name}:{language}"),
                name: name.clone(),
                group: grouped.then(|| (*label).to_owned()),
                duration: None,
            })
            .collect();
        let tags_root = edit.tags_root;
        let selected =
            draw_clip_player(ui, edit, "wwise_event", &clips, &languages, &mut |index| {
                let name = &events[index].1;
                Some(ClipPlay::Action(super::audio::SoundAction::PlayEvent {
                    event_name: name.clone(),
                    label: name.clone(),
                    tags_root: tags_root.map(std::path::Path::to_path_buf),
                }))
            });
        ui.label(
            RichText::new(
                "Wwise-authored \u{2014} audio lives in sound\\pc\\*.pck; \
                 extract-only (no tool.exe reimport).",
            )
            .color(subtle_dark()),
        );
        let (label, name) = &events[selected];
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new(format!("\u{2B07} {name}")))
                .on_hover_text("Extract this event to WAV")
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Extract Wwise event")
                    .set_file_name(format!("{}.wav", sanitize_component(name)))
                    .save_file()
            {
                *edit.sound_extract_request = Some(ExtractRequest {
                    items: vec![ExtractItem {
                        out_path: path,
                        source: ExtractSource::Event {
                            name: name.clone(),
                            language: edit.sound_language.map(str::to_owned),
                        },
                    }],
                    tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                    label: name.clone(),
                });
            }
            ui.label(RichText::new(*label).color(subtle_dark()));
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
/// the binding the app already resolved by walking the tag's package
/// imports out to its Wwise event(s); each row is one `.wem` permutation.
fn draw_ce_wwise_player(
    ui: &mut Ui,
    binding: &crate::core::source::ce_audio::CeSoundBinding,
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
        if media.is_empty() {
            ui.label(RichText::new("(no media in this language)").color(subtle_dark()));
            return;
        }
        // Name the event each file plays for only when there is more than one.
        let grouped = media.iter().any(|m| m.event_name != media[0].event_name);
        let clips: Vec<PlayerClip> = media
            .iter()
            .enumerate()
            .map(|(index, m)| PlayerClip {
                id: format!("ce:{selected}:{index}:{}", m.source_name),
                name: m.display_name(),
                group: grouped.then(|| m.event_name.clone()),
                duration: None,
            })
            .collect();
        let paks_root = edit.ce_paks_root;
        let chosen = draw_clip_player(
            ui,
            edit,
            "ce_media",
            &clips,
            &language_picker,
            &mut |index| {
                paks_root.map(|root| {
                    ClipPlay::Action(super::audio::SoundAction::PlayCeMedia {
                        paks_root: root.to_path_buf(),
                        media: Box::new(media[index].clone()),
                        label: media[index].display_name(),
                    })
                })
            },
        );
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

        // The selected file, then the whole tag. CE media is addressed
        // directly in the pak set, so unlike the Halo 4 event player this
        // needs no prior playback.
        let m = media[chosen];
        ui.horizontal_wrapped(|ui| {
            if let Some(root) = edit.ce_paks_root
                && ui
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

            if ui
                .button(RichText::new(format!("\u{2B07} {}", m.display_name())))
                .on_hover_text(format!(
                    "Extract this permutation to WAV\n{}",
                    m.source_name
                ))
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
                            media: Box::new(m.clone()),
                        },
                    }],
                    tags_root: None,
                    label: m.display_name(),
                });
            }
            ui.label(RichText::new(&m.event_name).color(subtle_dark()));
            ui.label(RichText::new(m.location_label()).color(subtle_dark()));
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
            let Some((entry, fallback)) =
                source.h2.and_then(|h2| h2.entry_for(*lpi, source.language))
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
fn draw_sound_format_line(
    ui: &mut Ui,
    tag: &TagFile,
    rows: &[SoundPermRow],
    source: RowSource<'_>,
) {
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
    let h2 = h2_sound_for_game(tag, edit.game);
    let rows = sound_permutation_rows_for_game(tag, h2.as_ref(), edit.game);
    if rows.is_empty() {
        return;
    }
    // A Halo 2 tag plays the chosen language when it has it, else English;
    // the shared choice may be another game's language this tag never had.
    let chosen = edit.sound_language.map(str::to_owned);
    let missing_language = h2
        .as_ref()
        .zip(chosen.as_deref())
        .filter(|(h2, language)| !h2.has_language(language))
        .map(|(_, language)| language.to_owned());
    let mut language = if missing_language.is_some() {
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
    let abs_tag_path = file_key_path(edit.tag_key).map(std::path::Path::to_path_buf);
    // This tag's rel path (e.g. `sound\dialog\...\ambush`) + whether it spans
    // multiple pitch ranges — both feed the FMOD subsound id hash.
    let sound_rel = abs_tag_path
        .as_deref()
        .zip(edit.tags_root)
        .and_then(|(abs, root)| sound_tag_rel(abs, root));
    let shared_fmod_audio = is_fmod_language_game(edit.game)
        && sound_rel
            .as_deref()
            .is_some_and(|path| !sound_path_may_have_languages(path));
    if shared_fmod_audio {
        language = None;
    }
    let languages = if shared_fmod_audio {
        Vec::new()
    } else {
        language_choices(edit, h2.as_ref())
    };
    let multi_pr = rows_span_multiple_pitch_ranges(&rows);
    let source = RowSource {
        h2: h2.as_ref(),
        language,
        sound_rel: sound_rel.as_deref(),
        multi_pr,
    };
    // A lone `|default|` pitch range is just the container; name ranges only
    // when there's more than one, or the one there is was named.
    let show_pitch_ranges = multi_pr
        || rows
            .first()
            .is_some_and(|row| !is_default_pitch_range(&row.pitch_range));
    let localized = h2.as_ref().is_some_and(|h2| h2.languages.len() > 1)
        || (is_fmod_language_game(edit.game) && !shared_fmod_audio);
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
        let clips: Vec<PlayerClip> = rows
            .iter()
            .map(|row| PlayerClip {
                // The language is part of what plays, so a language change is a
                // different clip rather than the loaded one.
                id: format!(
                    "{}:{}:{}",
                    row.pr_index,
                    row.perm_index,
                    language.unwrap_or("")
                ),
                name: row.name.clone(),
                group: show_pitch_ranges.then(|| format!("pitch range: {}", row.pitch_range)),
                duration: row_duration(row, source).0,
            })
            .collect();
        let tags_root = edit.tags_root;
        let selected = draw_clip_player(ui, edit, "sound", &clips, &languages, &mut |index| {
            row_play_action(tag, &rows[index], source, tags_root).map(ClipPlay::Action)
        });
        let row = &rows[selected];
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
                language
                    .filter(|language| !is_default_external_language(language))
                    .map(str::to_owned)
            }
        };
        let base_for = |language: Option<&str>| {
            abs_tag_path
                .as_deref()
                .zip(edit.kit_layout)
                .and_then(|(tag_path, layout)| {
                    reimport_base_dir_lang(layout, tag_path, data_language(language).as_deref())
                })
        };
        let extract_base = base_for(if shared_fmod_audio {
            None
        } else if h2.is_some() {
            language
        } else {
            edit.sound_language
        });
        let format_note = sound_format_note(edit.game);
        let missing_count = rows
            .iter()
            .filter(|row| row_duration(row, source).1)
            .count();
        let (_, fallback) = row_duration(row, source);
        ui.horizontal_wrapped(|ui| {
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
                            items.extend(build_extract_items(
                                tag,
                                &rows,
                                each_source,
                                &base,
                                false,
                            ));
                        }
                        *edit.sound_extract_request = Some(ExtractRequest {
                            items,
                            tags_root: edit.tags_root.map(std::path::Path::to_path_buf),
                            label: "all languages".to_owned(),
                        });
                    }
                }
            }
            // Third: the selected permutation alone, then what is particular to it.
            if ui
                .button(RichText::new(format!("\u{2B07} {}", row.name)))
                .on_hover_text(format!(
                    "Extract this permutation to a file\n{}",
                    row_details(row, h2.as_ref())
                ))
                .clicked()
            {
                let ext = row_extract_ext(&row.kind, raw_ce);
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Extract permutation")
                    .set_file_name(format!("{}.{ext}", sanitize_component(&row.name)))
                    .save_file()
                    && let Some(item_source) = row_extract_source(tag, row, source, raw_ce, false)
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
            if fallback {
                ui.label(
                    RichText::new(format!("no {language_name} \u{00B7} plays English"))
                        .color(ui.visuals().warn_fg_color),
                );
            }
            if let Some(gain) = row.gain_db.filter(|gain| gain.abs() >= 0.05) {
                ui.label(RichText::new(format!("gain {gain:+.1} dB")).color(subtle_dark()));
            }
            if let Some(skip) = row.skip_fraction.filter(|skip| *skip > 0.0) {
                ui.label(RichText::new(format!("skip {:.0}%", skip * 100.0)).color(subtle_dark()))
                    .on_hover_text("Fraction of requests for this permutation that are ignored");
            }
            if has_inline_ogg {
                ui.checkbox(&mut raw_ce, "raw .ogg").on_hover_text(
                    "Extract CE audio as the tag's original Ogg stream (lossless) \
                     instead of decoding to WAV",
                );
            }
        });
        ui.data_mut(|d| d.insert_temp(raw_ce_id, raw_ce));
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
    game: Option<GameId>,
    tags_root: Option<&std::path::Path>,
    definitions_root: Option<&std::path::Path>,
    rel_path: &str,
    group: u32,
) -> Option<(TagFile, std::path::PathBuf)> {
    let tags_root = tags_root?;
    let abs = blam_tags::paths::resolve_tag_path(tags_root, rel_path, "sound");
    let tag =
        crate::core::source::read_tag_at_path(&abs, game, definitions_root, group)
            .ok()?;
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
    game: Option<GameId>,
    sound_rel: Option<&str>,
    language: Option<&str>,
    tags_root: Option<&std::path::Path>,
) -> Option<super::audio::SoundAction> {
    if let Some((_, name)) = h4_event_names(tag).into_iter().next() {
        return Some(super::audio::SoundAction::PlayEvent {
            event_name: name.clone(),
            label: name,
            tags_root: tags_root.map(std::path::Path::to_path_buf),
        });
    }

    let h2 = h2_sound_for_game(tag, game);
    let rows = sound_permutation_rows_for_game(tag, h2.as_ref(), game);
    let source = RowSource {
        h2: h2.as_ref(),
        language: language_for(h2.as_ref(), language),
        sound_rel,
        multi_pr: rows_span_multiple_pitch_ranges(&rows),
    };
    rows.first()
        .and_then(|row| row_play_action(tag, row, source, tags_root))
}

/// Extract items for a whole (referenced) sound tag under `base` — Wwise events
/// or pitch-range permutations, matching the primary extractor's layout.
/// `sound_rel` is the referenced tag's rel path (for the FMOD subsound id).
fn referenced_sound_extract_items(
    tag: &TagFile,
    game: Option<GameId>,
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
                source: ExtractSource::Event {
                    name: name.clone(),
                    language: language.map(str::to_owned),
                },
            })
            .collect();
    }
    let h2 = h2_sound_for_game(tag, game);
    let rows = sound_permutation_rows_for_game(tag, h2.as_ref(), game);
    let source = RowSource {
        h2: h2.as_ref(),
        language: language_for(h2.as_ref(), language),
        sound_rel,
        multi_pr: false,
    };
    build_extract_items(tag, &rows, source, base, false)
}

/// A referenced sound as a player clip: `id` names it within the player.
fn referenced_clip(id: String, group: Option<String>, name: String) -> PlayerClip {
    PlayerClip {
        id,
        name,
        group,
        duration: None,
    }
}

/// How a referenced sound plays: its tag's first permutation (or Halo 4
/// event), loaded from the tags folder — or, on a Campaign Evolved mount, a
/// reference the app resolves to the tag's Wwise media.
#[allow(clippy::too_many_arguments)]
fn referenced_clip_play(
    group: u32,
    path: &str,
    game: Option<GameId>,
    tags_root: Option<&std::path::Path>,
    definitions_root: Option<&std::path::Path>,
    language: Option<&str>,
    container_source: bool,
) -> Option<ClipPlay> {
    let label = path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
    if container_source {
        return Some(ClipPlay::CeRef(CeSoundRefRequest {
            group_tag: group,
            reference: path.to_owned(),
            label,
            extract: false,
            clip: None,
            preview: false,
        }));
    }
    let (sound, _) = load_referenced_sound(game, tags_root, definitions_root, path, group)?;
    referenced_sound_play_action(&sound, game, Some(path), language, tags_root)
        .map(ClipPlay::Action)
}

/// What a click on a referenced-sound row produced. A container source can only
/// yield `ce_ref`: the referenced tag holds no samples, so the app resolves its
/// Wwise binding after the frame.
#[derive(Default)]
struct ReferencedSoundClick {
    open: Option<OpenTagRequest>,
    /// A row's ▶: the player clip to select and play.
    select: Option<usize>,
    extract: Option<ExtractRequest>,
    ce_ref: Option<CeSoundRefRequest>,
}

impl ReferencedSoundClick {
    /// Fold another row's click into this one. Only one button can be pressed
    /// per frame, so a later row's `Some` simply wins.
    fn take_from(&mut self, other: Self) {
        self.open = other.open.or(self.open.take());
        self.select = other.select.or(self.select.take());
        self.extract = other.extract.or(self.extract.take());
        self.ce_ref = other.ce_ref.or(self.ce_ref.take());
    }

    /// Hand every collected request to the app; a row's ▶ plays through the
    /// player `id_salt` over `clips`.
    fn apply(
        self,
        ctx: &egui::Context,
        edit: &mut FieldEditContext<'_>,
        id_salt: &str,
        clips: &[PlayerClip],
        play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
    ) {
        if self.open.is_some() {
            *edit.open_request = self.open;
        }
        if let Some(index) = self.select {
            play_clip_now(ctx, edit, id_salt, clips, index, play);
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
/// `clips[i]` is the player clip ref `i` plays as; ▶ selects and plays it in
/// the player. Without `clips` there is no ▶ (the player already has one).
///
/// `container_source` marks a Campaign Evolved mount, where there is no tags
/// root to load the referenced tag from — and nothing worth loading if there
/// were, since CE sound tags carry no samples.
#[allow(clippy::too_many_arguments)] // copies out of `edit`, so the grid closure needn't borrow it
fn draw_referenced_sound_cell(
    ui: &mut Ui,
    refs: &[(u32, String)],
    clips: Option<&[Option<usize>]>,
    game: Option<GameId>,
    tags_root: Option<&std::path::Path>,
    kit_layout: Option<&KitLayout>,
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
        for (index, (group, path)) in refs.iter().enumerate() {
            ui.horizontal(|ui| {
                let is_sound = &group.to_be_bytes() == b"snd!";
                let label = path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned();
                if let Some(clip) = clips.and_then(|clips| clips.get(index).copied().flatten())
                    && ui
                        .small_button("\u{25B6}")
                        .on_hover_text("Play this referenced sound in the player")
                        .clicked()
                {
                    click.select = Some(clip);
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
                            clip: None,
                            preview: false,
                        });
                    } else if let Some((sound, abs)) =
                        load_referenced_sound(game, tags_root, definitions_root, path, *group)
                        && let Some(base) = kit_layout
                            .and_then(|layout| reimport_base_dir_lang(layout, &abs, language))
                    {
                        let items = referenced_sound_extract_items(
                            &sound,
                            game,
                            &base,
                            Some(path.as_str()),
                            language,
                        );
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
            .and_then(|full| read_name_text(&vocal, full))
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
    let kit_layout = edit.kit_layout;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
    let container_source = edit.ce_paks_root.is_some();
    let languages = language_choices(edit, None);
    // Every referenced sound is a clip in one player; a row's ▶ plays its
    // sound through it.
    let language_key = language.unwrap_or("");
    let mut clips: Vec<PlayerClip> = Vec::new();
    let mut clip_refs: Vec<(u32, String)> = Vec::new();
    let mut row_clips: Vec<Vec<Option<usize>>> = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let mut ids = Vec::new();
        for (sound_index, (group, path)) in row.sounds.iter().enumerate() {
            if &group.to_be_bytes() != b"snd!" {
                ids.push(None);
                continue;
            }
            ids.push(Some(clips.len()));
            clips.push(referenced_clip(
                format!("ref:{row_index}:{sound_index}:{path}:{language_key}"),
                Some(row.name.clone()),
                path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned(),
            ));
            clip_refs.push((*group, path.clone()));
        }
        row_clips.push(ids);
    }
    let mut play = |index: usize| {
        let (group, path) = &clip_refs[index];
        referenced_clip_play(
            *group,
            path,
            game,
            tags_root,
            defs,
            language,
            container_source,
        )
    };
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Dialogue Overview ({total} vocalizations, {total_sounds} sounds)"
        ))
        .strong()
        .color(text_dark()),
    )
    .id_salt("dialogue_overview")
    .default_open(true)
    .show(ui, |ui| {
        if clips.is_empty() {
            // Nothing to play: just the volume, language and status.
            ui.horizontal(|ui| draw_sound_output_controls(ui, edit, &languages));
            draw_sound_errors(ui, edit);
        } else {
            draw_clip_player(ui, edit, "dialogue", &clips, &languages, &mut play);
        }
        if total == 0 {
            ui.label(RichText::new("(no vocalizations)").color(subtle_dark()));
            return;
        }
        // The player stays in view; the table of every vocalization, which
        // runs to hundreds of rows, starts closed past 40.
        egui::CollapsingHeader::new(
            RichText::new(format!("Vocalizations ({total})")).color(text_dark()),
        )
        .id_salt("dialogue_vocalizations")
        .default_open(total <= 40)
        .show(ui, |ui| {
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
                            for (row, row_clips) in rows.iter().zip(&row_clips) {
                                ui.label(RichText::new(&row.name).color(text_dark()));
                                let click = draw_referenced_sound_cell(
                                    ui,
                                    &row.sounds,
                                    Some(row_clips),
                                    game,
                                    tags_root,
                                    kit_layout,
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
    });
    clicked.apply(&ui.ctx().clone(), edit, "dialogue", &clips, &mut play);
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
                .and_then(|full| read_name_text(&track, full))
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
    let kit_layout = edit.kit_layout;
    let defs = edit.definitions_root;
    let language = edit.sound_language;
    let container_source = edit.ce_paks_root.is_some();
    let languages = language_choices(edit, None);
    // One clip per component sound, listed under its track (or "detail
    // sounds") as the part it plays.
    let language_key = language.unwrap_or("");
    let clips: Vec<PlayerClip> = refs
        .iter()
        .enumerate()
        .map(|(index, (label, _, path))| {
            let (owner, part) = label.split_once(" \u{00B7} ").unwrap_or((label, ""));
            let group = if owner.starts_with("detail ") {
                "detail sounds".to_owned()
            } else {
                owner.to_owned()
            };
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
            referenced_clip(
                format!("ref:{index}:{path}:{language_key}"),
                Some(group),
                format!("{part} \u{00B7} {leaf}"),
            )
        })
        .collect();
    let mut play = |index: usize| {
        let (_, group, path) = &refs[index];
        referenced_clip_play(
            *group,
            path,
            game,
            tags_root,
            defs,
            language,
            container_source,
        )
    };
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "Sound Looping \u{2014} {} component sound(s)",
            refs.len()
        ))
        .color(text_dark()),
    )
    .default_open(true)
    .show(ui, |ui| {
        let selected = draw_clip_player(ui, edit, "looping", &clips, &languages, &mut play);
        // The selected component: open it, or extract it.
        let (_, group, path) = &refs[selected];
        clicked.take_from(draw_referenced_sound_cell(
            ui,
            &[(*group, path.clone())],
            None,
            game,
            tags_root,
            kit_layout,
            defs,
            language,
            container_source,
        ));
    });
    clicked.apply(&ui.ctx().clone(), edit, "looping", &clips, &mut play);
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
                    .and_then(|full| read_name_text(&material, full))
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



#[cfg(test)]
mod tests {
    use super::*;
    use blam_tags::TagFieldData;
    use crate::app::audio::{InlineCodec, SoundAction, SoundOwner, SoundRequest, SoundRequests};
    use std::collections::VecDeque;

    // Editor unit and fixture tests.
    // It owns test-only characterization and does not participate in runtime application behavior.

    /// The extract-layout default-range detector matches the tool's `|default|`
    /// rule plus our synthesized placeholder for unnamed ranges.
    #[test]
    fn default_pitch_range_detection() {
        assert!(is_default_pitch_range(""));
        assert!(is_default_pitch_range("|default|"));
        assert!(is_default_pitch_range("default"));
        assert!(is_default_pitch_range("pitch range 0"));
        assert!(is_default_pitch_range("pitch range 12"));
        assert!(!is_default_pitch_range("close"));
        assert!(!is_default_pitch_range("pitch range x"));
    }

    #[test]
    fn fmod_language_picker_lists_the_catalog_and_disables_missing_banks() {
        let root = std::env::temp_dir().join(format!(
            "baboon-fmod-language-picker-{}",
            std::process::id()
        ));
        let tags = root.join("tags");
        let banks = root.join("fmod/pc");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::create_dir_all(&banks).unwrap();
        std::fs::write(banks.join("english.fsb"), []).unwrap();
        std::fs::write(banks.join("french.fsb"), []).unwrap();

        let mut sinks = EditSinks::default();
        let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
        edit.game = Some(GameId::Halo3);
        edit.tags_root = Some(&tags);
        let choices = language_choices(&edit, None);

        assert_eq!(choices.len(), FMOD_LANGUAGES.len());
        assert_eq!(choices.iter().filter(|choice| choice.available).count(), 2);
        assert!(choices.iter().all(|choice| choice.label != "default"));
        let japanese = choices
            .iter()
            .find(|choice| choice.label == "Japanese")
            .unwrap();
        assert!(!japanese.available);
        assert!(japanese
            .unavailable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("japanese.fsb")));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Campaign Evolved extraction end-to-end (skip-if-absent): resolve a
    /// sound tag's Wwise media exactly as the player does, run it through
    /// `AudioState::run_extract`, and validate the WAV that lands on disk.
    ///
    /// Run with:
    ///   CE_PAKS=/path/to/Meteorite/Content/Paks cargo test ce_extract -- --ignored --nocapture
    #[test]
    #[ignore = "requires a Campaign Evolved install; set CE_PAKS"]
    fn ce_extract_writes_a_valid_wav() {
        use crate::app::audio::AudioState;
        use crate::app::export::sound_extract::{ExtractItem, ExtractRequest, ExtractSource};
        use crate::core::source::ce_audio::{CeSoundMedia, resolve_sound_binding};
        use crate::core::source::{ContainerPackageIndex, MountedContainer, container_package_name};
        use blam_tags::iostore::{IoStoreArchive, usmap::Usmap};
        use std::path::PathBuf;
        use std::sync::Arc;

        let Ok(root) = std::env::var("CE_PAKS") else {
            eprintln!("skip: CE_PAKS not set");
            return;
        };
        let root = PathBuf::from(root);
        if !root.exists() {
            eprintln!("skip: no Campaign Evolved paks at {}", root.display());
            return;
        }

        let mut utocs: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("read paks dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("utoc"))
            })
            .filter(|p| {
                !p.file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("global.utoc"))
            })
            .collect();
        utocs.sort();

        let mut containers = Vec::new();
        let mut packages = ContainerPackageIndex::default();
        for utoc in utocs {
            let Ok(archive) = IoStoreArchive::open(&utoc) else {
                continue;
            };
            let idx = containers.len();
            for e in archive.entries() {
                if let Some(pkg) = container_package_name(&e.path) {
                    packages.insert(pkg, idx, e.path.clone());
                }
            }
            containers.push(MountedContainer {
                utoc_path: utoc.clone(),
                chunk_label: utoc.file_stem().unwrap().to_string_lossy().into_owned(),
                // Nothing on this path layers shipped against modded — it mounts
                // everything to resolve one lookup.
                is_mod: false,
                archive: Arc::new(archive),
            });
        }

        let usmap = Usmap::meteorite().expect("bundled usmap");
        let binding = resolve_sound_binding(
            &containers,
            &packages,
            &usmap,
            "/Game/Tags/sound/scripted/vo_scr_m02halo/m02_00040_cortana-sound",
            None,
        );
        assert!(!binding.is_empty(), "no media resolved");

        let shown = binding.language_to_show(None);
        let media: Vec<CeSoundMedia> = binding
            .media_for_language(&shown)
            .into_iter()
            .cloned()
            .collect();
        assert!(!media.is_empty());

        let dir = std::env::temp_dir().join(format!("baboon_ce_extract_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let items: Vec<ExtractItem> = media
            .iter()
            .map(|m| ExtractItem {
                out_path: dir.join(format!("{}.wav", sanitize_component(&m.display_name()))),
                source: ExtractSource::CeMedia {
                    paks_root: root.clone(),
                    media: Box::new(m.clone()),
                },
            })
            .collect();
        let expected: Vec<PathBuf> = items.iter().map(|i| i.out_path.clone()).collect();

        let mut audio = AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: None,
                label: "ce extract test".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();

        for path in &expected {
            let bytes = std::fs::read(path)
                .unwrap_or_else(|e| panic!("{} not written: {e}", path.display()));
            assert!(bytes.len() > 44, "{} is header-only", path.display());
            assert_eq!(&bytes[0..4], b"RIFF", "{} is not a RIFF", path.display());
            assert_eq!(&bytes[8..12], b"WAVE", "{} is not a WAVE", path.display());
            // Real audio, not a buffer of zeroes.
            let loud = bytes[44..]
                .chunks_exact(2)
                .any(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() > 64);
            assert!(loud, "{} decoded to silence", path.display());
            println!("wrote {} ({} bytes)", path.display(), bytes.len());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end validation of the sound-player glue against real H3 files
    /// (skip-if-absent): extract permutation names exactly as `draw_sound_player`
    /// does, then resolve each against the FMOD banks and decode — the same path
    /// `AudioState::process` takes on a Play click.
    #[test]
    #[ignore]
    fn sound_player_permutations_resolve_and_decode() {
        use blam_tags::audio::{SoundBanks, decode_subsound};
        // Overridable so the same check runs against any game's tags + banks.
        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| crate::core::test_kits::tag_path("halo3_mcc", "").to_owned());
        let rel = std::env::var("SND_TAG")
            .unwrap_or_else(|_| "sound/visual_fx/ambient_vehicle_destroyed_large.sound".to_owned());
        let tags_root = std::path::Path::new(&root);
        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no H3 tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read sound tag");
        let root = tag.root();
        let pitch_ranges = find_block_field(&root, "pitch range").expect("pitch ranges block");
        let mut names = Vec::new();
        for pr_index in 0..pitch_ranges.len() {
            let pitch_range = pitch_ranges.element(pr_index).unwrap();
            let permutations =
                find_block_field(&pitch_range, "permutation").expect("permutations block");
            for perm_index in 0..permutations.len() {
                let perm = permutations.element(perm_index).unwrap();
                if let Some(name) = find_full_field_name(&perm, "name")
                    .and_then(|full| perm.read_string_id(full))
                    .filter(|n| !n.is_empty())
                {
                    names.push(name);
                }
            }
        }
        assert!(!names.is_empty(), "extracted no permutation names");

        let banks = SoundBanks::open_pc(tags_root).expect("open FMOD banks");
        let mut resolved = 0usize;
        for name in &names {
            if let Some((bank_index, sub_index)) = banks.resolve(name) {
                let bank = banks.bank(bank_index);
                let sub = &bank.subsounds[sub_index];
                let data = bank.read_subsound_data(sub_index).unwrap();
                let pcm =
                    decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash, sub.num_samples)
                        .unwrap();
                assert!(pcm.frame_count() > 0, "'{name}' decoded to nothing");
                resolved += 1;
            }
        }
        eprintln!(
            "permutations: {} extracted, {} resolved+decoded",
            names.len(),
            resolved
        );
        assert!(resolved > 0, "no permutation names resolved in the bank");
    }

    /// End-to-end validation of the Halo 4 Wwise glue (skip-if-absent): read a
    /// real `.sound` tag, extract its event name exactly as `draw_sound_player`
    /// does, then resolve+decode it against the game's `.pck` banks — the same
    /// path `AudioState::process` takes on a PlayEvent click.
    #[test]
    #[ignore]
    fn h4_event_resolves_and_decodes() {
        use blam_tags::audio::WwiseBanks;
        let root = std::env::var("H4_TAGS_ROOT")
            .unwrap_or_else(|_| crate::core::test_kits::tag_path("halo4_mcc", "").to_owned());
        let rel = std::env::var("H4_SND_TAG")
            .unwrap_or_else(|_| "sound/ui/m30_a_60_sfx.sound".to_owned());
        let tags_root = std::path::Path::new(&root);

        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no H4 tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read H4 sound tag");
        let events = h4_event_names(&tag);
        assert!(!events.is_empty(), "no event names on the H4 sound tag");
        eprintln!("events: {events:?}");

        let banks = WwiseBanks::open_pc(tags_root).expect("open Wwise banks");
        let mut resolved = 0usize;
        for (_label, name) in &events {
            let pcm = banks.resolve(name).expect("resolve event");
            assert!(pcm.frame_count() > 0, "'{name}' decoded to nothing");
            eprintln!(
                "  {name} -> {}ch {}Hz {} frames",
                pcm.channels,
                pcm.sample_rate,
                pcm.frame_count()
            );
            resolved += 1;
        }
        assert!(resolved > 0);
    }

    /// Coverage audit (skip-if-absent): walk *every* `.sound` tag under a game's
    /// tags tree, compute each permutation's `fmod bank subsound id hash` exactly
    /// as the sound player does, and check it resolves in the FMOD banks. Reports
    /// id-coverage and, for id-misses, whether the legacy name lookup would have
    /// found *anything* — so a miss is attributed to a genuinely absent subsound
    /// vs. a hash/reconstruction gap. Run with:
    ///   SND_TAGS_ROOT=/path/to/haloreach_mcc/tags \
    ///     cargo test fmod_id_resolves_every_permutation -- --ignored --nocapture
    #[test]
    #[ignore]
    fn fmod_id_resolves_every_permutation() {
        use blam_tags::audio::{SoundBanks, fmod_bank_subsound_id_hash, fmod_pitch_range_folder};

        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| crate::core::test_kits::tag_path("haloreach_mcc", "").to_owned());
        let tags_root = std::path::Path::new(&root);
        if !tags_root.exists() {
            eprintln!("skip: no tags at {}", tags_root.display());
            return;
        }
        let language = std::env::var("SND_LANGUAGE").ok();
        let banks = SoundBanks::open_pc_language(tags_root, language.as_deref())
            .expect("open FMOD banks");

        // Recursively collect every .sound tag.
        let mut sound_tags = Vec::new();
        let mut stack = vec![tags_root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "sound") {
                    sound_tags.push(p);
                }
            }
        }
        eprintln!(
            "scanning {} .sound tags under {} for {:?}",
            sound_tags.len(), root, language
        );

        let (mut perms, mut by_id, mut by_name, mut id_miss_name_hit, mut absent, mut no_pr) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let mut hash_gaps: Vec<String> = Vec::new();

        for tag_path in &sound_tags {
            let Ok(tag) = blam_tags::TagFile::read(tag_path) else {
                continue;
            };
            let tag_root = tag.root();
            let Some(pitch_ranges) = find_block_field(&tag_root, "pitch range") else {
                no_pr += 1;
                continue;
            };
            // Tag rel path (backslash, no extension) — the hash input's tag part.
            let rel = tag_path
                .strip_prefix(tags_root)
                .unwrap_or(tag_path)
                .with_extension("")
                .to_string_lossy()
                .replace('/', "\\");
            let multi_pr = pitch_ranges.len() > 1;
            for pr_index in 0..pitch_ranges.len() {
                let Some(pr) = pitch_ranges.element(pr_index) else {
                    continue;
                };
                let pr_name = find_full_field_name(&pr, "name")
                    .and_then(|full| pr.read_string_id(full))
                    .unwrap_or_default();
                let folder = fmod_pitch_range_folder(&pr_name, multi_pr);
                let Some(permutations) = find_block_field(&pr, "permutation") else {
                    continue;
                };
                for perm_index in 0..permutations.len() {
                    let Some(perm) = permutations.element(perm_index) else {
                        continue;
                    };
                    let Some(name) = find_full_field_name(&perm, "name")
                        .and_then(|full| perm.read_string_id(full))
                        .filter(|n| !n.is_empty())
                    else {
                        continue;
                    };
                    perms += 1;
                    let id = fmod_bank_subsound_id_hash(&rel, folder, &name);
                    let id_hit = banks.resolve_by_id(id).is_some();
                    let name_hit = banks.resolve(&name).is_some();
                    if id_hit {
                        by_id += 1;
                    }
                    if name_hit {
                        by_name += 1;
                    }
                    if !id_hit {
                        if name_hit {
                            id_miss_name_hit += 1;
                            if hash_gaps.len() < 30 {
                                hash_gaps
                                    .push(format!("{rel}\\{}#{perm_index} :: {name}", pr_name));
                            }
                        } else {
                            absent += 1;
                        }
                    }
                }
            }
        }

        eprintln!("permutations: {perms}");
        eprintln!(
            "  resolved by id  : {by_id} ({:.2}%)",
            100.0 * by_id as f64 / perms.max(1) as f64
        );
        eprintln!("  resolved by name: {by_name} (legacy, ambiguous)");
        eprintln!("  id-miss, name-hit (potential hash gap): {id_miss_name_hit}");
        eprintln!("  id-miss, name-miss (subsound absent from bank): {absent}");
        eprintln!("  tags without a pitch-range block (Wwise/classic): {no_pr}");
        if !hash_gaps.is_empty() {
            eprintln!("  sample hash gaps:");
            for g in &hash_gaps {
                eprintln!("    {g}");
            }
        }
    }

    /// Classic Halo CE inline audio (skip-if-absent): read the classic `.sound`
    /// tag, extract the permutation's inline `samples` exactly as the player
    /// does, and decode the Ogg Vorbis — the path `AudioState` takes for a
    /// `PlayInline` action.
    #[test]
    #[ignore]
    fn ce_inline_permutation_extracts_and_decodes() {
        use blam_tags::audio::decode_ogg_vorbis;
        let defs = crate::core::test_kits::definitions();
        let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(
            "haloce_mcc",
            "sound/sinomatixx_music/b40_extraction_music.sound",
        ));
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::core::source::read_tag_at_path(tag_path, Some(GameId::HaloCe), Some(defs), group)
            .expect("read CE sound tag");
        let bytes = inline_permutation_chain(&tag, 0, 0).expect("inline samples present").0;
        assert!(
            bytes.starts_with(b"OggS"),
            "CE samples should be an Ogg stream"
        );
        let pcm = decode_ogg_vorbis(&bytes).expect("decode CE ogg");
        eprintln!(
            "CE inline: {} bytes -> {} frames {}ch {}Hz",
            bytes.len(),
            pcm.frame_count(),
            pcm.channels,
            pcm.sample_rate
        );
        assert!(pcm.frame_count() > 0);
    }

    /// Classic Halo CE Xbox-ADPCM weapon sound (skip-if-absent). CE `.sound`
    /// tags aren't always Ogg — weapon/effect sounds are frequently
    /// `format = xbox adpcm`, which has no `OggS` header. Regression for the
    /// `ogg header: NoCapturePatternFound` failure: the codec must come from the
    /// tag's `format` field, and the row must decode via the ADPCM path.
    #[test]
    #[ignore]
    fn ce_inline_xbox_adpcm_extracts_and_decodes() {
        use super::audio::InlineCodec;
        let defs = crate::core::test_kits::definitions();
        let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(
            "haloce_mcc",
            "sound/sfx/weapons/sniper rifle/fire.sound",
        ));
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");

        let tag = crate::core::source::read_tag_at_path(tag_path, Some(GameId::HaloCe), Some(defs), group)
            .expect("read CE sound tag");

        // The tag reports Xbox-ADPCM, mono, 22050 Hz — and carries no Ogg stream.
        let root = tag.root();
        let perm = find_block_field(&root, "pitch range")
            .and_then(|ranges| ranges.element(0))
            .and_then(|range| find_block_field(&range, "permutation"))
            .and_then(|perms| perms.element(0))
            .expect("first permutation");
        let (codec, channels, sample_rate) = permutation_inline_params(&root, &perm);
        assert!(
            matches!(codec, InlineCodec::XboxAdpcm),
            "sniper fire.sound is xbox adpcm, got {codec:?}"
        );
        assert_eq!((channels, sample_rate), (1, 22_050));

        // The player's row must inherit that codec (not assume Ogg).
        let rows = sound_permutation_rows(&tag, None);
        assert!(matches!(
            rows.first().map(|r| &r.kind),
            Some(RowKind::InlinePermutation {
                codec: InlineCodec::XboxAdpcm,
                ..
            })
        ));

        let bytes = inline_permutation_chain(&tag, 0, 0).expect("inline samples present").0;
        assert!(!bytes.starts_with(b"OggS"), "adpcm stream, not Ogg");
        let pcm = super::audio::decode_inline(codec, &bytes, channels, sample_rate)
            .expect("decode CE xbox adpcm");
        eprintln!(
            "CE xbox-adpcm: {} bytes -> {} frames {}ch {}Hz",
            bytes.len(),
            pcm.frame_count(),
            pcm.channels,
            pcm.sample_rate
        );
        assert!(pcm.frame_count() > 0);
    }

    /// End-to-end extraction (skip-if-absent): read a real CE `.sound`, build
    /// the same rows the player builds, and run the actual
    /// `AudioState::run_extract` for both WAV (decoded) and raw `.ogg`
    /// (passthrough), validating each output.
    #[test]
    #[ignore]
    fn ce_extract_writes_wav_and_raw_ogg() {
        let defs = crate::core::test_kits::definitions();
        let tag_path = std::path::Path::new(crate::core::test_kits::tag_path(
            "haloce_mcc",
            "sound/sinomatixx_music/b40_extraction_music.sound",
        ));
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::core::source::read_tag_at_path(tag_path, Some(GameId::HaloCe), Some(defs), group)
            .expect("read CE sound tag");
        let rows = sound_permutation_rows(&tag, None);
        assert!(!rows.is_empty(), "CE tag should have permutations");

        // Decoded WAV.
        let wav_dir = std::env::temp_dir().join("baboon_ce_extract_wav");
        let _ = std::fs::remove_dir_all(&wav_dir);

        let items = build_extract_items(&tag, &rows, RowSource { h2: None, language: None, sound_rel: None, multi_pr: false }, &wav_dir, false);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: None,
                label: "ce".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        let wav = std::fs::read(wav_dir.join(format!("{}.wav", sanitize_component(&rows[0].name))))
            .expect("wav written");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(wav.len() > 44, "wav should carry samples");

        // Raw .ogg passthrough should be byte-identical to the inline samples.
        let ogg_dir = std::env::temp_dir().join("baboon_ce_extract_ogg");
        let _ = std::fs::remove_dir_all(&ogg_dir);
        let items = build_extract_items(&tag, &rows, RowSource { h2: None, language: None, sound_rel: None, multi_pr: false }, &ogg_dir, true);
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: None,
                label: "ce".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        let ogg = std::fs::read(ogg_dir.join(format!("{}.ogg", sanitize_component(&rows[0].name))))
            .expect("ogg written");
        assert!(ogg.starts_with(b"OggS"), "raw passthrough should be an Ogg");
        let inline =
            inline_permutation_chain(&tag, rows[0].pr_index, rows[0].perm_index).unwrap().0;
        assert_eq!(ogg, inline, "raw passthrough must be verbatim tag bytes");

        let _ = std::fs::remove_dir_all(&wav_dir);

        let _ = std::fs::remove_dir_all(&ogg_dir);
    }

    /// Whole-tag H3/Reach bank extraction end-to-end (skip-if-absent): run the
    /// real `run_extract` Bank path (resolve subsound → decode → WAV) for every
    /// permutation. `SND_TAGS_ROOT`/`SND_TAG` override for Reach/ODST.
    #[test]
    #[ignore]
    fn bank_extract_writes_wav() {
        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| crate::core::test_kits::tag_path("halo3_mcc", "").to_owned());
        let rel = std::env::var("SND_TAG")
            .unwrap_or_else(|_| "sound/visual_fx/ambient_vehicle_destroyed_large.sound".to_owned());
        let tags_root = std::path::Path::new(&root);
        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no bank tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read sound tag");
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::Halo3));
        assert!(!rows.is_empty());
        let dir = std::env::temp_dir().join("baboon_bank_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let items = build_extract_items(&tag, &rows, RowSource { h2: None, language: None, sound_rel: None, multi_pr: false }, &dir, false);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: Some(tags_root.to_path_buf()),
                label: "bank".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        let mut found = 0usize;
        for entry in walkdir(&dir) {
            let bytes = std::fs::read(&entry).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            found += 1;
        }
        assert!(found > 0, "wrote no bank WAVs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Per-language bank plumbing (skip-if-absent): the FMOD languages are
    /// discovered from the `.fsb` names, and opening a specific language + the
    /// shared sfx bank still resolves an SFX permutation (language bank first,
    /// sfx fallback). Proves `open_pc_language` doesn't break default resolution.
    #[test]
    #[ignore]
    fn fmod_language_selection_resolves() {
        use blam_tags::audio::SoundBanks;
        let tags_root = std::path::Path::new(crate::core::test_kits::tag_path("halo3_mcc", ""));
        if !tags_root.join("../fmod/pc/sfx.fsb").exists() {
            eprintln!("skip: no H3 fmod banks");
            return;
        }
        let langs = SoundBanks::available_languages(tags_root);

        eprintln!("H3 languages: {langs:?}");
        assert!(
            langs.iter().any(|l| l == "french"),
            "expected localized .fsb languages, got {langs:?}"
        );
        assert!(!langs.iter().any(|l| l == "sfx"), "sfx must be excluded");
        // Open a specific language + sfx; an SFX permutation still resolves.
        let banks =
            SoundBanks::open_pc_language(tags_root, Some("french")).expect("open french+sfx");
        let tag = blam_tags::TagFile::read(
            &tags_root.join("sound/visual_fx/ambient_vehicle_destroyed_large.sound"),
        )
        .expect("read sfx sound tag");
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::Halo3));
        let resolved = rows
            .iter()
            .filter(|r| banks.resolve(&r.name).is_some())
            .count();
        assert!(resolved > 0, "no permutations resolved in french+sfx banks");
    }

    /// Exercise the same row classification, ID construction, queued action,
    /// bank resolution, and async decode used by the sound-player button.
    #[test]
    #[ignore]
    fn h3_sound_player_action_reaches_playback() {
        let tags_root = crate::core::test_kits::h3ek_tags();
        let path = tags_root.join("sound/visual_fx/ambient_vehicle_destroyed_large.sound");
        if !path.exists() {
            eprintln!("skip: set BLAM_TEST_H3EK");
            return;
        }
        let tag = TagFile::read(&path).expect("read H3 sound tag");
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::Halo3));
        assert!(!rows.is_empty(), "sound tag has no permutations");
        assert!(
            rows.iter().all(|row| matches!(row.kind, RowKind::Bank)),
            "H3 permutations must be bank-backed"
        );
        let rel = sound_tag_rel(&path, &tags_root).expect("tag-relative path");
        let source = RowSource {
            h2: None,
            language: None,
            sound_rel: Some(&rel),
            multi_pr: rows_span_multiple_pitch_ranges(&rows),
        };
        let play = row_play_action(&tag, &rows[0], source, Some(&tags_root))
            .expect("play action for H3 row");
        let super::audio::SoundAction::Play { id, key, .. } = &play else {
            panic!("H3 row did not create an FMOD play action");
        };
        let banks = blam_tags::audio::SoundBanks::open_pc(&tags_root).expect("open H3 banks");
        let (bank_index, sub_index) = id
            .and_then(|id| banks.resolve_by_id(id))
            .or_else(|| banks.resolve(key))
            .expect("resolve H3 player row");
        let bank = banks.bank(bank_index);
        let sub = &bank.subsounds[sub_index];
        let data = bank.read_subsound_data(sub_index).expect("read H3 subsound");
        let pcm = blam_tags::audio::decode_subsound(
            &data,
            sub.channels,
            sub.frequency,
            sub.setup_hash,
            sub.num_samples,
        )
        .expect("decode H3 player row");
        let peak = pcm
            .samples
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 64, "resolved H3 player row decoded to silence");
        let duration = pcm.duration_secs();
        let mut audio = super::audio::AudioState::default();
        audio.pending.push_back(play.into());
        audio.process(None, &egui::Context::default());
        audio.wait_for_audio_jobs();
        assert!(
            audio.status.as_deref().is_some_and(|status| status.starts_with('\u{25B6}')),
            "normal player path did not reach playback: {:?}",
            audio.status
        );
        // Keep the ignored manual/integration test's output stream alive long
        // enough for the device to render the queued voice.
        std::thread::sleep(std::time::Duration::from_secs_f32(
            duration.clamp(0.25, 10.0) + 0.5,
        ));
    }

    #[test]
    #[ignore]
    fn h3_extraction_writes_non_silent_pcm() {
        let tags_root = crate::core::test_kits::h3ek_tags();
        let path = tags_root.join("sound/dialog/combat/brute1/23_idle/peeing.sound");
        if !path.exists() {
            eprintln!("skip: set BLAM_TEST_H3EK");
            return;
        }
        let tag = TagFile::read(&path).expect("read H3 sound tag");
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::Halo3));
        let sound_rel = sound_tag_rel(&path, &tags_root).unwrap();
        let out = std::env::temp_dir().join(format!(
            "baboon-h3-extract-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&out);
        let source = RowSource {
            h2: None,
            language: None,
            sound_rel: Some(&sound_rel),
            multi_pr: rows_span_multiple_pitch_ranges(&rows),
        };
        let items = build_extract_items(&tag, &rows, source, &out, false);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: Some(tags_root),
                label: "H3 non-silent regression".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        for wav in walkdir(&out) {
            let bytes = std::fs::read(&wav).unwrap();
            assert!(bytes.len() > 44, "empty WAV: {}", wav.display());
            assert!(
                bytes[44..].chunks_exact(2).any(|sample| sample != [0, 0]),
                "silent WAV: {} ({:?})",
                wav.display(),
                audio.status
            );
        }
        let _ = std::fs::remove_dir_all(out);
    }

    #[test]
    #[ignore]
    fn h3_all_language_extraction_reads_the_explicit_english_bank() {
        use crate::app::export::sound_extract::ExtractSource;

        let tags_root = crate::core::test_kits::h3ek_tags();
        let path = tags_root.join("sound/dialog/combat/brute1/23_idle/peeing.sound");
        if !path.exists() {
            eprintln!("skip: set BLAM_TEST_H3EK");
            return;
        }
        let tag = TagFile::read(&path).expect("read H3 sound tag");
        let shared_banks = blam_tags::audio::SoundBanks::open_pc_language(
            &tags_root,
            Some("__baboon_shared_bank_only__"),
        )
        .expect("open shared H3 FMOD bank");
        let layout = KitLayout::from_tags_folder(&tags_root).expect("kit layout");
        let mut items = browser_sound_extract_items(
            &tag,
            &path,
            &layout,
            Some(GameId::Halo3),
            None,
            true,
            Some(&shared_banks),
        );
        assert!(items.iter().any(|item| matches!(
            &item.source,
            ExtractSource::Bank { language: Some(language), .. }
                if language.eq_ignore_ascii_case("english")
        )));
        assert!(!items.iter().any(|item| matches!(
            item.source,
            ExtractSource::Bank { language: None, .. }
        )));
        let mut item_languages = items
            .iter()
            .filter_map(|item| match &item.source {
                ExtractSource::Bank { language, .. } => language.clone(),
                _ => None,
            })
            .collect::<Vec<_>>();
        item_languages.sort();
        item_languages.dedup();
        assert_eq!(
            item_languages,
            blam_tags::audio::SoundBanks::available_languages(&tags_root),
            "all-languages extraction must follow installed .fsb files"
        );

        let sfx_path = tags_root.join("sound/visual_fx/ambient_vehicle_destroyed_large.sound");
        let sfx_tag = TagFile::read(&sfx_path).expect("read shared H3 sound tag");
        let sfx_items = browser_sound_extract_items(
            &sfx_tag,
            &sfx_path,
            &layout,
            Some(GameId::Halo3),
            Some("french"),
            true,
            Some(&shared_banks),
        );
        assert!(!sfx_items.is_empty());
        assert!(sfx_items.iter().all(|item| matches!(
            item.source,
            ExtractSource::Bank { language: None, .. }
        )));
        assert!(sfx_items.iter().all(|item| item.out_path.starts_with(
            tags_root.parent().unwrap().join("data")
        )));

        let out = std::env::temp_dir().join(format!(
            "baboon-h3-all-languages-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&out);
        for (index, item) in items.iter_mut().enumerate() {
            item.out_path = out.join(format!("{index}.wav"));
        }
        let total = items.len();
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(
            ExtractRequest {
                items,
                tags_root: Some(tags_root),
                label: "H3 all-languages regression".to_owned(),
            },
            &egui::Context::default(),
        );
        audio.wait_for_audio_jobs();
        assert_eq!(walkdir(&out).len(), total, "{:?}", audio.status);
        assert!(
            audio.status.as_deref().is_some_and(|status| status
                .starts_with(&format!("extracted {total}/{total}"))),
            "{:?}",
            audio.status
        );
        let _ = std::fs::remove_dir_all(out);
    }

    /// Recursively collect files under `dir` (small test helper).
    fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn sound_classes_summary_reads_modern_and_classic_layouts() {
        // Modern (Reach): scalar distances nested under "distance parameters".
        let mut tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        add_block_element(&mut tag, "sound classes").unwrap();
        let classes = tag
            .root()
            .field("sound classes")
            .and_then(|field| field.as_block())
            .unwrap();
        let element = classes.element(0).unwrap();
        assert!(
            element.descend("distance parameters").is_some(),
            "Reach nests distances under `distance parameters`"
        );
        assert_ne!(
            sound_class_distance_row(&element).near,
            "—",
            "Reach `minimum distance` field name should resolve"
        );

        // Classic (H3): `distance bounds` real_bounds directly on the entry.
        let mut tag = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        add_block_element(&mut tag, "sound classes").unwrap();
        let classes = tag
            .root()
            .field("sound classes")
            .and_then(|field| field.as_block())
            .unwrap();
        let element = classes.element(0).unwrap();
        assert!(
            element.descend("distance parameters").is_none(),
            "H3 has no `distance parameters` struct"
        );

        assert!(
            element.field("distance bounds").is_some(),
            "H3 keeps `distance bounds` directly on the entry"
        );
        assert_ne!(sound_class_distance_row(&element).near, "—");
    }

    #[test]
    fn material_effects_summary_walks_effects_and_materials_cross_game() {
        // CE: effect → `materials` block with `effect` + `sound` tag references.
        let mut tag = TagFile::new("definitions/haloce_mcc/material_effects.json").unwrap();
        add_block_element(&mut tag, "effects").unwrap();
        add_block_element(&mut tag, "effects[0]/materials").unwrap();
        let materials = tag
            .root()
            .field_path("effects[0]/materials")
            .and_then(|field| field.as_block())
            .unwrap();
        let material = materials.element(0).unwrap();
        assert!(find_full_field_name(&material, "effect").is_some());
        assert!(find_full_field_name(&material, "sound").is_some());

        // Modern (H3): effect → `sounds` block; materials use a `tag (effect or
        // sound)` reference and a `material name` string_id.
        let mut tag = TagFile::new("definitions/halo3_mcc/material_effects.json").unwrap();
        add_block_element(&mut tag, "effects").unwrap();
        let effects = tag
            .root()
            .field("effects")
            .and_then(|field| field.as_block())
            .unwrap();
        let effect = effects.element(0).unwrap();
        let labels: Vec<String> = block_fields(&effect)
            .into_iter()
            .map(|(label, _)| label.to_ascii_lowercase())
            .collect();
        assert!(
            labels.iter().any(|label| label.contains("sound")),
            "modern effect has a `sounds` material sub-block"
        );
        assert!(
            labels.iter().any(|label| label.contains("old")),
            "modern effect still declares the deprecated `old materials` block"
        );
        add_block_element(&mut tag, "effects[0]/sounds").unwrap();
        let sounds = tag
            .root()
            .field_path("effects[0]/sounds")
            .and_then(|field| field.as_block())
            .unwrap();
        let material = sounds.element(0).unwrap();
        assert!(
            material
                .field_names()
                .any(|name| name.contains("tag (effect or sound)")),
            "modern material carries a `tag (effect or sound)` reference"
        );
        assert!(
            find_field_name_containing(&material, "material name").is_some(),
            "modern material carries a `material name` field"
        );
    }

    #[test]
    fn dialogue_summary_detects_direct_vs_nested_and_classic() {
        // Classic CE: no vocalizations block (flat per-context fields).
        let tag = TagFile::new("definitions/haloce_mcc/dialogue.json").unwrap();
        assert!(
            find_block_field(&tag.root(), "vocali").is_none(),
            "CE has no vocalizations block"
        );

        // H3/ODST: `sound` reference directly on the vocalization.
        let mut tag = TagFile::new("definitions/halo3_mcc/dialogue.json").unwrap();
        add_block_element(&mut tag, "vocalizations").unwrap();
        let vocals = tag
            .root()
            .field("vocalizations")
            .and_then(|field| field.as_block())
            .unwrap();
        let vocal = vocals.element(0).unwrap();
        assert!(
            find_full_field_name(&vocal, "sound").is_some(),
            "H3 keeps `sound` directly on the vocalization"
        );
        assert!(
            find_block_field(&vocal, "stimul").is_none(),
            "H3 has no stimuli sub-block"
        );

        // Reach/H4/H2A: `sound` nested under a per-vocalization `stimuli` block.
        let mut tag = TagFile::new("definitions/haloreach_mcc/dialogue.json").unwrap();
        add_block_element(&mut tag, "vocalizations").unwrap();
        let vocals = tag
            .root()
            .field("vocalizations")
            .and_then(|field| field.as_block())
            .unwrap();
        let vocal = vocals.element(0).unwrap();
        assert!(
            find_block_field(&vocal, "stimul").is_some(),
            "Reach nests sounds under a `stimuli` block"
        );
        assert!(
            find_full_field_name(&vocal, "sound").is_none(),
            "Reach vocalization has no direct `sound` field"
        );
    }

    // The sound player and its row model over synthetic tags.
    //
    // Characterization, with no kit: a Halo CE sound carries its audio inline,
    // so a tag built from the definitions with a few permutations of 16-bit PCM
    // is a complete sound — the rows, the play actions, the extraction layout
    // and the player all work from it alone.

    const SAMPLE_RATE: usize = 22_050;

    fn new_tag_for(game: &str, group: &str) -> TagFile {
        TagFile::new(
            crate::core::bundled::locate_definitions_root()
                .join(game)
                .join(format!("{group}.json")),
        )
        .unwrap_or_else(|error| panic!("{game}/{group}.json: {error:?}"))
    }

    /// The 16-bit sample bytes of permutation `index`: a ramp, so every
    /// permutation's bytes differ.
    fn samples(index: usize, frames: usize) -> Vec<u8> {
        (0..frames)
            .flat_map(|frame| ((frame as i16).wrapping_mul(7).wrapping_add(index as i16)).to_be_bytes())
            .collect()
    }

    /// A Halo CE sound: one pitch range of `permutations` permutations, each
    /// `frames` frames of inline mono PCM, named `perm_{index}`.
    fn ce_sound(permutations: usize, frames: usize) -> TagFile {
        let mut tag = new_tag_for("haloce_mcc", "sound");
        let mut root = tag.root_mut();
        let mut field = root.field_path_mut("pitch ranges").expect("pitch ranges");
        let mut ranges = field.as_block_mut().expect("pitch ranges is a block");
        ranges.add_element();
        let mut range = ranges.element_mut(0).expect("the pitch range");
        let mut field = range.field_path_mut("permutations").expect("permutations");
        let mut block = field.as_block_mut().expect("permutations is a block");
        for index in 0..permutations {
            block.add_element();
            let mut permutation = block.element_mut(index).expect("the permutation");
            permutation
                .field_path_mut("name")
                .expect("permutation name")
                .set(TagFieldData::String(format!("perm_{index}")))
                .expect("set the permutation name");
            permutation
                .field_path_mut("samples")
                .expect("permutation samples")
                .set(TagFieldData::Data(samples(index, frames)))
                .expect("set the samples");
        }
        tag
    }

    /// Every permutation of an inline CE sound is a row that plays its own
    /// samples, in tag order, at the tag's format.
    #[test]
    fn a_ce_sound_lists_each_inline_permutation() {
        let tag = ce_sound(3, SAMPLE_RATE);
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::HaloCe));
        assert_eq!(rows.len(), 3);
        for (index, row) in rows.iter().enumerate() {
            // A Halo CE permutation's `name` is a 32-character `string`, not a
            // string id; the row carries it. (Rows used to be named by index,
            // `#0`, `#1`…, and extraction wrote `#0.wav`.)
            assert_eq!(row.name, format!("perm_{index}"));
            assert_eq!((row.pr_index, row.perm_index), (0, index));
            assert_eq!(row.inline_bytes, SAMPLE_RATE * 2);
            match row.kind {
                RowKind::InlinePermutation {
                    codec: InlineCodec::Pcm { big_endian },
                    channels,
                    sample_rate,
                } => {
                    // CE's plain `none` compression is little-endian PCM.
                    assert!(!big_endian);
                    assert_eq!((channels, sample_rate), (1, SAMPLE_RATE as u32));
                }
                _ => panic!("{index}: not an inline permutation"),
            }
        }
        assert!(!rows_span_multiple_pitch_ranges(&rows));
        assert!(is_default_pitch_range(&rows[0].pitch_range));

        // The second permutation plays its own bytes.
        let source = RowSource {
            h2: None,
            language: None,
            sound_rel: None,
            multi_pr: false,
        };
        match row_play_action(&tag, &rows[1], source, None) {
            Some(SoundAction::PlayInline {
                bytes,
                channels,
                sample_rate,
                chunk_offsets,
                label,
                ..
            }) => {
                assert_eq!(bytes, samples(1, SAMPLE_RATE));
                assert_eq!((channels, sample_rate), (1, SAMPLE_RATE as u32));
                assert!(chunk_offsets.is_empty());
                assert_eq!(label, rows[1].name, "labelled as its row");
            }
            _ => panic!("an inline row plays its samples"),
        }
    }

    /// The same permutations under a Halo 3-family game are bank rows: their
    /// `samples` are placeholders there, whatever they hold.
    #[test]
    fn a_bank_game_reads_every_row_as_a_bank_subsound() {
        let tag = ce_sound(2, 16);
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::Halo3));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| matches!(row.kind, RowKind::Bank)));
        let source = RowSource {
            h2: None,
            language: None,
            sound_rel: Some("sound\\test\\thing"),
            multi_pr: false,
        };
        match row_play_action(&tag, &rows[0], source, None) {
            Some(SoundAction::Play { id, key, label, .. }) => {
                // Keyed by the row's name.
                assert_eq!(key, rows[0].name);
                assert_eq!(label, rows[0].name);
                assert_eq!(
                    id,
                    Some(blam_tags::audio::fmod_bank_subsound_id_hash(
                        "sound\\test\\thing",
                        blam_tags::audio::fmod_pitch_range_folder(&rows[0].pitch_range, false),
                        &rows[0].name,
                    ))
                );
            }
            _ => panic!("a bank row plays from the banks"),
        }
    }

    /// Extraction lays a lone default pitch range out flat, one WAV per
    /// permutation, decoding the inline PCM.
    #[test]
    fn ce_extraction_writes_one_wav_per_permutation_flat() {
        let tag = ce_sound(2, 32);
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::HaloCe));
        let source = RowSource {
            h2: None,
            language: None,
            sound_rel: None,
            multi_pr: false,
        };
        let base = std::path::Path::new("data/sound/test");
        let items = build_extract_items(&tag, &rows, source, base, true);
        // One `<permutation name>.wav` per row, flat under `base`.
        let paths: Vec<_> = items.iter().map(|item| item.out_path.clone()).collect();
        let expected = vec![base.join("perm_0.wav"), base.join("perm_1.wav")];
        assert_eq!(paths, expected);
        for (index, item) in items.iter().enumerate() {
            match &item.source {
                ExtractSource::Inline {
                    bytes,
                    channels,
                    sample_rate,
                    ..
                } => {
                    assert_eq!(*bytes, samples(index, 32));
                    assert_eq!((*channels, *sample_rate), (1, SAMPLE_RATE as u32));
                }
                _ => panic!("PCM is decoded even with raw passthrough on"),
            }
        }

        // From the browser: the same layout, under the kit's data folder.
        let root = std::env::temp_dir().join("baboon-sound-synthetic");
        let layout = KitLayout {
            root: root.clone(),
            tags: root.join("tags"),
            data: root.join("data"),
        };
        let items = browser_sound_extract_items(
            &tag,
            &root.join("tags/sound/test/thing.sound"),
            &layout,
            Some(GameId::HaloCe),
            None,
            true,
            None,
        );
        let paths: Vec<_> = items.iter().map(|item| item.out_path.clone()).collect();
        let expected: Vec<_> = rows
            .iter()
            .map(|row| root.join("data/sound/test/thing").join(format!("{}.wav", row.name)))
            .collect();
        assert_eq!(paths, expected);
    }

    /// Draw the CE sound player for a few frames; see [`run_drawing`].
    fn run(tag: &TagFile, clicks: &[(&str, usize)]) -> (Vec<String>, VecDeque<SoundRequest>) {
        run_drawing("haloce_mcc", clicks, &|ui, edit| draw_sound_player(ui, tag, edit))
    }

    /// Draw `draw` for a few frames as `game`, clicking each of `clicks` (the
    /// `nth` painted text starting with `text`, top to bottom) in turn; what the
    /// last frame painted and every sound request queued.
    fn run_drawing(
        game: &'static str,
        clicks: &[(&str, usize)],
        draw: &dyn Fn(&mut Ui, &mut FieldEditContext<'_>),
    ) -> (Vec<String>, VecDeque<SoundRequest>) {
        let ctx = egui::Context::default();
        let mut queued = VecDeque::new();
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, queued: &mut VecDeque<SoundRequest>| {
            time += 1.0 / 60.0;
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 900.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut sinks = EditSinks::default();
                        let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
                        edit.game = GameId::from_id(game);
                        edit.sound_play_request = SoundRequests::new(
                            queued,
                            Some(SoundOwner {
                                kit: crate::app::kits::kit::KitId(1),
                                key: "test".to_owned(),
                            }),
                        );
                        draw(ui, &mut edit);
                    });
                },
            );
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        frame(Vec::new(), &mut queued);
        let mut texts = frame(Vec::new(), &mut queued);
        for &(text, nth) in clicks {
            let mut found: Vec<egui::Rect> = texts
                .iter()
                .filter(|(shown, _)| shown.starts_with(text))
                .map(|(_, rect)| *rect)
                .collect();
            found.sort_by(|a, b| {
                a.top()
                    .total_cmp(&b.top())
                    .then(a.left().total_cmp(&b.left()))
            });
            let pos = found
                .get(nth)
                .unwrap_or_else(|| panic!("no {nth}th {text:?} in {texts:?}"))
                .center();
            for step in 1..=3 {
                let t = step as f32 / 3.0;
                let from = pos - egui::vec2(30.0, 30.0);
                frame(vec![egui::Event::PointerMoved(from + (pos - from) * t)], &mut queued);
            }
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![button(true)], &mut queued);
            frame(vec![button(false)], &mut queued);
            texts = frame(Vec::new(), &mut queued);
        }
        (texts.into_iter().map(|(text, _)| text).collect(), queued)
    }

    /// The player titles itself with the permutation count, lists each clip's
    /// length from its inline samples, asks for the selected clip's waveform,
    /// and plays the selected clip's own bytes.
    #[test]
    fn the_ce_player_previews_and_plays_the_selected_permutation() {
        let tag = ce_sound(3, SAMPLE_RATE);
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::HaloCe));
        let (painted, queued) = run(&tag, &[]);
        for expected in [
            "Sound \u{2014} 3 permutations",
            // The clip's length, read from its inline samples before any decode.
            "0:00.000 / 0:01.000",
            "pcm \u{b7} mono \u{b7} 22.05 kHz",
            "\u{2B07} Extract all",
        ] {
            assert!(
                painted.iter().any(|text| text == expected),
                "{expected}: {painted:?}"
            );
        }
        assert!(
            painted.iter().any(|text| *text == rows[0].name),
            "the first clip is selected: {painted:?}"
        );
        let previews: Vec<&SoundRequest> = queued.iter().filter(|request| request.preview).collect();
        assert!(!previews.is_empty(), "no waveform preview was requested");
        assert!(previews.iter().all(|request| request.clip.as_deref() == Some("0:0:")));
        assert!(
            queued.iter().all(|request| request.preview),
            "drawing alone played something"
        );

        // "▶" is Next on the clip row, then Play on the transport.
        let (_, queued) = run(&tag, &[("\u{25B6}", 1)]);
        let play = queued
            .iter()
            .find(|request| !request.preview)
            .expect("Play queued nothing");
        assert_eq!(play.clip.as_deref(), Some("0:0:"));
        match &play.action {
            SoundAction::PlayInline { bytes, label, .. } => {
                assert_eq!(*bytes, samples(0, SAMPLE_RATE));
                assert_eq!(*label, rows[0].name);
            }
            _ => panic!("an inline CE permutation plays inline"),
        }

        // Next selects the second permutation (and plays nothing while stopped).
        let (painted, queued) = run(&tag, &[("\u{25B6}", 0)]);
        assert!(painted.iter().any(|text| *text == rows[1].name), "{painted:?}");
        assert!(queued.iter().all(|request| request.preview));
        assert!(queued.iter().any(|request| request.clip.as_deref() == Some("0:1:")));
    }

    fn h3_tag(group: &str) -> TagFile {
        new_tag_for("halo3_mcc", group)
    }

    fn add_element(tag: &mut TagFile, path: &str) {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(path)
            .unwrap_or_else(|| panic!("{path} resolves"));
        field
            .as_block_mut()
            .unwrap_or_else(|| panic!("{path} is a block"))
            .add_element();
    }

    fn set_field(tag: &mut TagFile, path: &str, input: &str) {
        crate::core::document::apply::apply_field_edit(tag, path, input)
            .unwrap_or_else(|error| panic!("{path} = {input}: {error}"));
    }

    /// A looping sound lists the sounds its tracks and detail sounds reference,
    /// one clip each, under the part that plays it — the first selected.
    #[test]
    fn a_looping_sound_lists_its_component_sounds() {
        let mut tag = h3_tag("sound_looping");
        add_element(&mut tag, "tracks");
        set_field(&mut tag, "tracks[0]/name", "ambience");
        set_field(&mut tag, "tracks[0]/in", "sound\\test\\loop_in.sound");
        set_field(&mut tag, "tracks[0]/loop", "sound\\test\\loop.sound");
        add_element(&mut tag, "detail sounds");
        set_field(&mut tag, "detail sounds[0]/sound", "sound\\test\\chirp.sound");
        let refs = sound_looping_refs(&tag);
        let labels: Vec<(&str, &str)> = refs
            .iter()
            .map(|(label, _, path)| (label.as_str(), path.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("ambience \u{b7} in", "sound\\test\\loop_in"),
                ("ambience \u{b7} loop", "sound\\test\\loop"),
                ("detail 0 \u{b7} sound", "sound\\test\\chirp"),
            ]
        );
        let (painted, queued) = run_drawing("halo3_mcc", &[], &|ui, edit| {
            draw_sound_looping_player(ui, &tag, edit)
        });
        for expected in [
            "Sound Looping \u{2014} 3 component sound(s)",
            "ambience \u{25B8} in \u{b7} loop_in",
            // Its length is unknown until the referenced tag is decoded.
            "0:00.000 / -:--.---",
        ] {
            assert!(
                painted.iter().any(|text| text == expected),
                "{expected}: {painted:?}"
            );
        }
        assert!(
            queued.iter().all(|request| request.preview),
            "drawing alone played something"
        );
    }

    /// A dialogue tag's overview counts its vocalizations and the sounds they
    /// reference, and a CE dialogue (no vocalization block) says where to edit.
    #[test]
    fn a_dialogue_overview_counts_its_vocalizations() {
        let mut tag = h3_tag("dialogue");
        for (index, (name, sound)) in [("hail", "sound\\dialog\\hail"), ("pain", "")]
            .into_iter()
            .enumerate()
        {
            add_element(&mut tag, "vocalizations");
            set_field(&mut tag, &format!("vocalizations[{index}]/vocalization"), name);
            if !sound.is_empty() {
                set_field(
                    &mut tag,
                    &format!("vocalizations[{index}]/sound"),
                    &format!("{sound}.sound"),
                );
            }
        }
        let (painted, _) = run_drawing("halo3_mcc", &[], &|ui, edit| {
            draw_dialogue_summary(ui, &tag, edit)
        });
        assert!(
            painted
                .iter()
                .any(|text| text.starts_with("Vocalizations (2")),
            "{painted:?}"
        );

        let classic = new_tag_for("haloce_mcc", "dialogue");
        let (painted, _) = run_drawing("haloce_mcc", &[], &|ui, edit| {
            draw_dialogue_summary(ui, &classic, edit)
        });
        assert!(
            painted
                .iter()
                .any(|text| text.starts_with("Classic Halo CE dialogue")),
            "{painted:?}"
        );
    }

    /// A material effects tag lists each material's effect and sound, skipping
    /// the deprecated "old materials" block.
    #[test]
    fn a_material_effects_overview_lists_material_rows() {
        let mut tag = h3_tag("material_effects");
        add_element(&mut tag, "effects");
        add_element(&mut tag, "effects[0]/sounds");
        set_field(&mut tag, "effects[0]/sounds[0]/material name", "metal");
        set_field(
            &mut tag,
            "effects[0]/sounds[0]/tag (effect or sound)",
            "sound\\materials\\metal_hit.sound",
        );
        add_element(&mut tag, "effects[0]/old materials (DO NOT USE)");
        set_field(
            &mut tag,
            "effects[0]/old materials (DO NOT USE)[0]/material name",
            "deprecated_stone",
        );
        let (painted, _) = run_drawing("halo3_mcc", &[], &|ui, edit| {
            draw_material_effects_summary(ui, &tag, edit)
        });
        assert!(painted.iter().any(|text| text == "metal"), "{painted:?}");
        assert!(
            !painted.iter().any(|text| text == "deprecated_stone"),
            "{painted:?}"
        );
    }

    /// The sound classes overview counts the classes the tag defines.
    #[test]
    fn a_sound_classes_overview_counts_its_classes() {
        let mut tag = h3_tag("sound_classes");
        for _ in 0..3 {
            add_element(&mut tag, "sound classes");
        }
        let (painted, _) = run_drawing("halo3_mcc", &[], &|ui, _| {
            draw_sound_classes_summary(ui, &tag)
        });
        assert!(
            painted
                .iter()
                .any(|text| text == "Sound Classes Overview (3)"),
            "{painted:?}"
        );
    }

    /// One Halo CE permutation's inline Ogg, decoded the way the player does.
    fn ce_inline_ogg_frames(rel: &str, permutation: usize) -> Option<usize> {
        let path = crate::core::test_kits::hceek_tags().join(rel);
        if !path.exists() {
            eprintln!("skip: set BLAM_TEST_HCEEK to a Halo CE kit's tags ({})", path.display());
            return None;
        }
        let defs = crate::core::test_kits::definitions();
        let tag = crate::core::source::read_tag_at_path(&path, Some(GameId::HaloCe), Some(defs), u32::from_be_bytes(*b"snd!"))
            .expect("read the CE sound");
        let root = tag.root();
        let ranges = root.field_path("pitch ranges").and_then(|f| f.as_block()).unwrap();
        let range = ranges.element(0).unwrap();
        let permutations = range.field("permutations").and_then(|f| f.as_block()).unwrap();
        let bytes = permutations.element(permutation).unwrap().field("samples").and_then(|f| f.as_data()).unwrap().to_vec();
        let pcm = super::audio::decode_inline(super::audio::InlineCodec::OggVorbis, &bytes, 2, 44_100)
            .expect("the permutation decodes");
        Some(pcm.frame_count())
    }

    /// A Halo CE Ogg permutation plays for the length its stream states:
    /// `bat1_ww`'s seventh piece decoded 448 frames of padding past it, a fade
    /// to silence that clicked where the next piece continues the music. The
    /// lengths are ffmpeg's decode of the same bytes.
    #[test]
    fn a_ce_ogg_permutation_plays_for_its_stated_length() {
        if let Some(frames) = ce_inline_ogg_frames("sound/music/battle1_themes/bat1_ww.sound", 6) {
            assert_eq!(frames, 232_960);
        }
    }

    /// The one Halo CE permutation the Rust Vorbis decoder panics on still
    /// plays, through libvorbis, at ffmpeg's length.
    #[test]
    fn the_ce_ogg_permutation_lewton_cannot_read_still_plays() {
        if let Some(frames) = ce_inline_ogg_frames("sound/music/spooky1/in.sound", 1) {
            assert_eq!(frames, 232_960);
        }
    }

    /// Every FMOD subsound decodes to the length its bank's header gives. The
    /// stream is padded to a whole packet, and nearly every Halo 3 subsound
    /// ran up to 1,024 frames long.
    #[test]
    fn h3_subsounds_decode_to_their_header_length() {
        let bank_path = crate::core::test_kits::h3ek_tags().join("../fmod/pc/sfx.fsb");
        if !bank_path.exists() {
            eprintln!("skip: no Halo 3 FMOD bank at {}", bank_path.display());
            return;
        }
        let bank = blam_tags::audio::fsb5::Fsb5::open(&bank_path).unwrap();
        let step = (bank.subsounds.len() / 200).max(1);
        let mut checked = 0;
        for index in (0..bank.subsounds.len()).step_by(step) {
            let sub = &bank.subsounds[index];
            let data = bank.read_subsound_data(index).unwrap();
            let pcm = blam_tags::audio::decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash, sub.num_samples)
                .unwrap_or_else(|error| panic!("subsound {index}: {error}"));
            assert_eq!(pcm.frame_count(), sub.num_samples as usize, "subsound {index} ({})", sub.name);
            checked += 1;
        }
        assert!(checked >= 100, "only {checked} subsounds checked");
    }

    /// A Halo CE sound stored as a chain is one row, whose audio is every
    /// piece, end to end: here the pitch range's one actual permutation
    /// continues into the other two.
    #[test]
    fn a_chained_ce_sound_is_one_row_playing_every_piece() {
        let mut tag = ce_sound(3, SAMPLE_RATE);
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("pitch ranges[0]/actual permutation count").unwrap();
            field.set(TagFieldData::ShortInteger(1)).unwrap();
            for (index, next) in [(0, 1i16), (1, 2), (2, -1)] {
                let mut field = root
                    .field_path_mut(&format!("pitch ranges[0]/permutations[{index}]/next permutation index"))
                    .unwrap();
                field.set(TagFieldData::ShortInteger(next)).unwrap();
            }
        }

        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::HaloCe));
        assert_eq!(rows.len(), 1, "the chained pieces are not rows of their own");
        assert_eq!(rows[0].name, "perm_0");

        let (bytes, offsets) = inline_permutation_chain(&tag, 0, 0).unwrap();
        let piece = SAMPLE_RATE * 2;
        assert_eq!(offsets, vec![0, piece, 2 * piece]);
        assert_eq!(bytes.len(), 3 * piece);
        let RowKind::InlinePermutation { codec, channels, sample_rate } = rows[0].kind else {
            panic!("not an inline permutation");
        };
        let pcm = super::audio::decode_inline_chunked(codec, &bytes, &offsets, channels, sample_rate).unwrap();
        assert_eq!(pcm.frame_count(), 3 * SAMPLE_RATE, "all three pieces play");
    }

    /// Halo CE's own chained music: `anfast.sound` is one track in 18 pieces,
    /// and was listed as 18 rows, each playing its own fragment.
    #[test]
    fn a_chained_ce_music_track_plays_whole() {
        let path = crate::core::test_kits::hceek_tags().join("sound/music/anfast/anfast.sound");
        if !path.exists() {
            eprintln!("skip: set BLAM_TEST_HCEEK to a Halo CE kit's tags ({})", path.display());
            return;
        }
        let defs = crate::core::test_kits::definitions();
        let tag = crate::core::source::read_tag_at_path(&path, Some(GameId::HaloCe), Some(defs), u32::from_be_bytes(*b"snd!"))
            .unwrap();
        let rows = sound_permutation_rows_for_game(&tag, None, Some(GameId::HaloCe));
        assert_eq!(rows.len(), 1);
        let (bytes, offsets) = inline_permutation_chain(&tag, rows[0].pr_index, rows[0].perm_index).unwrap();
        assert_eq!(offsets.len(), 18, "every piece of the track");
        let RowKind::InlinePermutation { codec, channels, sample_rate } = rows[0].kind else {
            panic!("not an inline permutation");
        };
        let whole = super::audio::decode_inline_chunked(codec, &bytes, &offsets, channels, sample_rate).unwrap();
        let mut ends = offsets.clone();
        ends.push(bytes.len());
        let pieces: usize = ends
            .windows(2)
            .map(|span| super::audio::decode_inline(codec, &bytes[span[0]..span[1]], channels, sample_rate).unwrap().frame_count())
            .sum();
        assert_eq!(whole.frame_count(), pieces);
        assert!(whole.frame_count() > 18 * 200_000, "{} frames", whole.frame_count());
    }
}
