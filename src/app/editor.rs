use super::*;

use super::sound_extract::{
    ExtractItem, ExtractRequest, ExtractSource, reimport_base_dir_lang, sanitize_component,
};

pub(super) fn draw_entry_header(ui: &mut Ui, entry: &TagEntry, names: &TagNameIndex) {
    ui.heading(RichText::new(&entry.display_path).color(text_dark()));
    ui.horizontal(|ui| {
        ui.label(RichText::new("Group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, entry.group_tag)).color(text_dark()));
        if let Some(name) = &entry.group_name {
            ui.label(RichText::new(name).color(subtle_dark()));
        }
    });
    ui.separator();
}

pub(super) fn draw_tag(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    model_preview: &mut ModelPreviewState,
    model_preview_size: &mut f32,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    let is_object_family = is_object_family_group(entry.group_tag);
    let is_shaderish =
        is_material_tag(entry) || is_material_shader_tag(entry) || is_shader_tag(entry);
    let is_model = is_model_group(entry.group_tag, names);

    draw_tag_metadata(ui, tag, names);
    if !is_object_family {
        draw_object_model_summary(ui, tag, entry, names, edit);
    }
    if is_sound_classes_group(entry.group_tag) {
        draw_sound_classes_summary(ui, tag);
    }
    if is_sound_group(entry.group_tag) {
        draw_sound_player(ui, tag, edit);
    }
    if is_dialogue_group(entry.group_tag) {
        draw_dialogue_summary(ui, tag, edit);
    }
    if is_sound_looping_group(entry.group_tag) {
        draw_sound_looping_player(ui, tag, edit);
    }
    if is_material_effects_group(entry.group_tag) {
        draw_material_effects_summary(ui, tag, edit);
    }

    if is_model {
        draw_model_tag_panel_tabs(ui, model_preview);
    }
    ui.add_space(6.0);

    if is_model && model_preview.active_tab == ModelTagPanelTab::RenderModel {
        draw_model_preview_panel(
            ui,
            tag,
            entry,
            names,
            source,
            model_preview,
            model_preview_size,
            edit,
        );
        return;
    }

    draw_tag_fields_scroll(
        ui,
        tag,
        entry,
        names,
        source,
        rmdf_cache,
        rmop_cache,
        color_popup,
        function_popup,
        expert_mode,
        edit,
        is_object_family,
        is_shaderish,
    );
}

fn draw_model_tag_panel_tabs(ui: &mut Ui, model_preview: &mut ModelPreviewState) {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::Fields,
            "Fields",
        );
        ui.selectable_value(
            &mut model_preview.active_tab,
            ModelTagPanelTab::RenderModel,
            "Render model",
        );
    });
}

fn draw_tag_fields_scroll(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    source: Option<&TagSource>,
    rmdf_cache: &mut HashMap<String, Option<RenderMethodDefinition>>,
    rmop_cache: &mut HashMap<String, Option<RenderMethodOption>>,
    color_popup: &mut Option<MaterialColorPopup>,
    function_popup: &mut Option<FunctionPopup>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    is_object_family: bool,
    is_shaderish: bool,
) {
    let scroll_height = ui.available_height().max(0.0);
    if is_shaderish {
        // The Guerilla-style shader grid is the single editing surface — no
        // separate field tab. The grid's bitmap/scalar/int/function/category
        // cells are editable inline; when the grid can't be built it falls
        // back to the standard editable field tree (inside draw_material_tag).
        ScrollArea::both()
            .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
            .max_height(scroll_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                draw_material_tag(
                    ui,
                    tag,
                    entry,
                    names,
                    source,
                    rmdf_cache,
                    rmop_cache,
                    color_popup,
                    function_popup,
                    expert_mode,
                    edit,
                );
            });
        return;
    }

    ScrollArea::both()
        .id_salt(("tag_scroll", edit.view_scope, edit.tag_key))
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
            if is_object_family {
                draw_inherited_object_fields(ui, tag.root(), names, expert_mode, edit);
            } else {
                draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
            }
        });
}

const TAG_FIELD_SCROLL_MIN_WIDTH: f32 = 980.0;

/// The `sound_classes` (`sncl`) tag group.
pub(super) fn is_sound_classes_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"sncl"
}

/// Cross-game-normalized overview of a `sound_classes` tag: one row per sound
/// class with its near/far distance and a detail column, reading whichever
/// distance layout the game uses (classic H2/H3/ODST keep `distance bounds`
/// directly on the entry; Reach/H4/H2A nest them under `distance parameters`).
/// Read-only — the full editable field tree still renders below.
pub(super) fn draw_sound_classes_summary(ui: &mut Ui, tag: &TagFile) {
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

struct SoundClassDistanceRow {
    near: String,
    far: String,
    detail: String,
}

fn sound_class_distance_row(element: &TagStruct) -> SoundClassDistanceRow {
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
pub(super) fn find_full_field_name<'a>(element: &TagStruct<'a>, clean: &str) -> Option<&'a str> {
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
pub(super) fn is_sound_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"snd!"
}

/// How a permutation row sources its audio: an FMOD bank subsound (Halo 3+), a
/// self-contained inline Ogg blob on the permutation (CE), or an inline
/// Opus/Xbox-ADPCM blob in H2's parallel language-permutation-info block.
enum RowKind {
    Bank,
    InlineOgg,
    InlineH2 { blob: usize },
}

/// One audition row: a permutation's name (which for the bank path is the
/// subsound key) + where its audio comes from.
struct SoundPermRow {
    pitch_range: String,
    name: String,
    pr_index: usize,
    perm_index: usize,
    kind: RowKind,
}

/// Extract a CE permutation's inline `samples` bytes (a self-contained Ogg).
/// Re-navigates from the root so it only clones the played permutation's blob.
fn inline_permutation_samples(
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
fn h2_blobs(tag: &TagFile, want: Option<usize>) -> (usize, Option<Vec<u8>>) {
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
                    if let Some(bytes) = samples {
                        if want == Some(count) {
                            got = Some(bytes.to_vec());
                        }
                        count += 1;
                    }
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
fn h2_blob_chunk_offsets(tag: &TagFile, want: usize) -> Vec<usize> {
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
            if let Some(el) = block.element(m) {
                if let Some(v) = el.read_int_any(offset_field) {
                    offsets.push(v.max(0) as usize);
                }
            }
        }
        return offsets;
    }
    Vec::new()
}

/// Read H2's tag-level inline codec parameters: `compression` → codec,
/// `encoding` → channel count, `sample rate` → Hz (used only by ADPCM; Opus is
/// always 48 kHz).
fn h2_codec_params(tag: &TagFile) -> (super::audio::InlineCodec, u16, u32) {
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

/// Audition panel for a `sound` (`snd!`) tag. Halo 3+ page the actual samples
/// out to the FMOD bank (`<game>/fmod/pc/*.fsb`) — the tag itself carries only
/// zeroed placeholder buffers — so we list the tag's pitch-range/permutation
/// names and play each by resolving its name against the opened banks. Clicking
/// Play/Stop queues an action the app drains after rendering. (Classic CE/H2,
/// whose audio is inline in the tag, aren't handled by this bank path yet and
/// will report "not found in FMOD bank".)
/// Halo 4 `.sound` tags reference Wwise events by name (no inline pitch-range
/// audio). Collect the non-empty event-name string-ids on the tag root.
fn h4_event_names(tag: &TagFile) -> Vec<(&'static str, String)> {
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
fn sound_permutation_rows(tag: &TagFile) -> Vec<SoundPermRow> {
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
                RowKind::InlineOgg
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
        RowKind::InlineOgg => {
            let bytes = inline_permutation_samples(tag, row.pr_index, row.perm_index)?;
            Some(SoundAction::PlayInline {
                bytes,
                codec: InlineCodec::OggVorbis,
                channels: 2,
                sample_rate: 44_100,
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
        RowKind::InlineOgg => {
            let bytes = inline_permutation_samples(tag, row.pr_index, row.perm_index)?;
            Some(if raw_ce {
                ExtractSource::Raw(bytes)
            } else {
                ExtractSource::Inline {
                    bytes,
                    codec: InlineCodec::OggVorbis,
                    channels: 2,
                    sample_rate: 44_100,
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
    if raw_ce && matches!(kind, RowKind::InlineOgg) {
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
fn is_default_pitch_range(name: &str) -> bool {
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
fn build_extract_items(
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

pub(super) fn draw_sound_player(ui: &mut Ui, tag: &TagFile, edit: &mut FieldEditContext<'_>) {
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
    let has_inline_ogg = rows
        .iter()
        .any(|row| matches!(row.kind, RowKind::InlineOgg));
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

pub(super) fn is_dialogue_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"udlg"
}

/// First block field whose cleaned name contains `needle` (lowercased).
fn find_block_field<'a>(element: &TagStruct<'a>, needle: &str) -> Option<TagBlock<'a>> {
    element.field_names().find_map(|name| {
        if clean_field_name(name).to_ascii_lowercase().contains(needle) {
            element.field(name).and_then(|field| field.as_block())
        } else {
            None
        }
    })
}

/// First field whose cleaned name contains `needle` (lowercased).
fn find_field_name_containing<'a>(element: &TagStruct<'a>, needle: &str) -> Option<&'a str> {
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

pub(super) fn draw_dialogue_summary(ui: &mut Ui, tag: &TagFile, edit: &mut FieldEditContext<'_>) {
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
pub(super) fn is_sound_looping_group(group_tag: u32) -> bool {
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
pub(super) fn draw_sound_looping_player(
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
pub(super) fn is_material_effects_group(group_tag: u32) -> bool {
    &group_tag.to_be_bytes() == b"foot"
}

/// All block fields of a struct, paired with their cleaned display label.
fn block_fields<'a>(element: &TagStruct<'a>) -> Vec<(String, TagBlock<'a>)> {
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
pub(super) fn draw_material_effects_summary(
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

/// Object-family tag groups (each derives from `obje` and carries a `.model`
/// reference). For these we surface the connected model at the very top.
pub(super) fn is_object_family_group(group_tag: u32) -> bool {
    matches!(
        &group_tag.to_be_bytes(),
        b"bipd" // biped
            | b"vehi" // vehicle
            | b"weap" // weapon
            | b"eqip" // equipment
            | b"scen" // scenery
            | b"mach" // device_machine
            | b"ctrl" // device_control
            | b"crat" // crate
            | b"bloc" // crate-like block
            | b"ssce" // sound_scenery
            | b"gint" // giant
            | b"proj" // projectile
            | b"obje" // object (base)
    )
}

/// Show the connected `.model` reference at the top of object-family tags
/// (biped, vehicle, weapon, scenery, …) with a working Open button.
pub(super) fn draw_object_model_summary(
    ui: &mut Ui,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    edit: &mut FieldEditContext<'_>,
) {
    if !is_object_family_group(entry.group_tag) {
        return;
    }
    let Some(model) = find_model_reference(tag.root(), names, 0, "") else {
        return;
    };
    let formatted = format_reference_path(names, model.group_tag, &model.rel_path);
    let meta = FieldDisplayMeta {
        label: "model".to_owned(),
        unit: None,
        range: None,
        help: Some("Object model tag reference".to_owned()),
        tag_reference_allowed: Vec::new(),
        read_only: false,
        advanced: false,
    };
    ui.add_space(4.0);
    let import_verb = geometry_import_verb(names, model.group_tag);
    draw_foundation_tag_reference_row(
        ui,
        &meta,
        &formatted,
        Some((model.group_tag, model.rel_path)),
        import_verb,
        0,
        &model.field_path,
        edit,
    );
}

pub(super) struct ModelReferenceInfo {
    pub(super) group_tag: u32,
    pub(super) rel_path: String,
    pub(super) field_path: String,
}

/// Like `find_model_reference` but returns the raw `(group_tag, rel_path)` so
/// the caller can resolve/open the target.
pub(super) fn find_model_reference(
    tag_struct: TagStruct<'_>,
    names: &TagNameIndex,
    depth: usize,
    path_prefix: &str,
) -> Option<ModelReferenceInfo> {
    if depth > 8 {
        return None;
    }
    for field in tag_struct.fields() {
        let field_path = append_field_path(path_prefix, field.name());
        if let Some(value) = field.value() {
            if let TagFieldData::TagReference(reference) = value {
                if let Some((group_tag, path)) = reference.group_tag_and_name.as_ref() {
                    if is_model_group(*group_tag, names) && !path.is_empty() {
                        return Some(ModelReferenceInfo {
                            group_tag: *group_tag,
                            rel_path: path.clone(),
                            field_path,
                        });
                    }
                }
            }
            continue;
        }
        if let Some(nested) = field.as_struct() {
            if let Some(found) = find_model_reference(nested, names, depth + 1, &field_path) {
                return Some(found);
            }
        } else if let Some(block) = field.as_block() {
            for (index, element) in block.iter().take(4).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        } else if let Some(array) = field.as_array() {
            for (index, element) in array.iter().take(8).enumerate() {
                let element_path = format!("{field_path}[{index}]");
                if let Some(found) = find_model_reference(element, names, depth + 1, &element_path)
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

pub(super) fn is_model_group(group_tag: u32, names: &TagNameIndex) -> bool {
    group_tag == u32::from_be_bytes(*b"hlmt")
        || names.name_for(group_tag) == Some("model")
        || group_tag_to_extension(group_tag) == Some("model")
}

pub(super) fn format_reference_path(names: &TagNameIndex, group_tag: u32, path: &str) -> String {
    if let Some(extension) = names
        .name_for(group_tag)
        .or_else(|| group_tag_to_extension(group_tag))
    {
        format!("{path}.{extension}")
    } else {
        format!("{}:{path}", format_group_tag(group_tag))
    }
}

pub(super) fn apply_pending_edits(
    tag: &mut TagFile,
    edits: Vec<PendingFieldEdit>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for edit in edits {
        let result = catch_edit_unwind(|| apply_field_edit(tag, &edit.path, &edit.input));
        match result {
            Ok(()) => {
                *dirty = true;
                status = Some(format!("Edited {}", edit.path));
            }
            Err(error) => {
                status = Some(format!("Edit failed for {}: {error}", edit.path));
            }
        }
    }
    status
}

pub(super) fn apply_block_ops(
    tag: &mut TagFile,
    ops: Vec<BlockOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result = apply_one_block_op(tag, &op);
        match result {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Block edit failed for {}: {error}", op.path));
            }
        }
    }
    status
}

pub(super) fn apply_function_data_ops(
    tag: &mut TagFile,
    ops: Vec<FunctionDataOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result =
            catch_edit_unwind(|| replace_halo2_function_byte_block(tag, &op.block_path, &op.data));
        match result {
            Ok(()) => {
                *dirty = true;
                status = Some(format!("Edited {}", op.block_path));
            }
            Err(error) => {
                status = Some(format!(
                    "Function edit failed for {}: {error}",
                    op.block_path
                ));
            }
        }
    }
    status
}

fn catch_edit_unwind(f: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .map_err(|panic| panic_message(panic))?
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        format!("internal edit panic: {message}")
    } else if let Some(message) = panic.downcast_ref::<&'static str>() {
        format!("internal edit panic: {message}")
    } else {
        "internal edit panic".to_owned()
    }
}

pub(super) fn replace_halo2_function_byte_block(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) -> Result<(), String> {
    if TagFunction::parse(data).is_err()
        && !is_h2_legacy_constant_function_data(data)
        && !is_h2_legacy_editable_function_data(data)
        && !is_damage_effect_vibration_function_data(data)
    {
        return Err("invalid mapping_function data".to_owned());
    }
    let current_len = tag
        .root()
        .field_path(block_path)
        .and_then(|field| field.as_block())
        .map(|block| block.len());
    let Some(current_len) = current_len else {
        return replace_halo2_wrapped_function_byte_block(tag, block_path, data)
            .ok_or_else(|| format!("function byte block not found: {block_path}"))?;
    };
    if current_len == data.len() && current_len > 0 {
        for (index, byte) in data.iter().copied().enumerate() {
            let value = (byte as i8).to_string();
            apply_field_edit(tag, &format!("{block_path}[{index}]/Value"), &value)?;
        }
        return Ok(());
    }
    clear_block(tag, block_path)?;
    for (index, byte) in data.iter().copied().enumerate() {
        add_block_element(tag, block_path)?;
        let value = (byte as i8).to_string();
        apply_field_edit(tag, &format!("{block_path}[{index}]/Value"), &value)?;
    }
    Ok(())
}

fn is_h2_legacy_editable_function_data(data: &[u8]) -> bool {
    data.len() >= 20 && data.len() != 32 && data.first().is_some_and(|kind| *kind <= 10)
}

fn is_damage_effect_vibration_function_data(data: &[u8]) -> bool {
    data.len() == 36
        && data.first().is_some_and(|kind| *kind <= 10)
        && data.get(2).is_some_and(|exponent| *exponent <= 7)
        && data.get(20..24).is_some_and(|bytes| {
            f32::from_le_bytes(bytes.try_into().unwrap_or_default()).is_finite()
        })
        && data.get(24..28).is_some_and(|bytes| {
            f32::from_le_bytes(bytes.try_into().unwrap_or_default()).is_finite()
        })
}

fn replace_halo2_wrapped_function_byte_block(
    tag: &mut TagFile,
    block_path: &str,
    data: &[u8],
) -> Option<Result<(), String>> {
    let wrapper_path = block_path.strip_suffix("/function/data")?;
    let mut root = tag.root_mut();
    let mut wrapper_field = root.field_path_mut(wrapper_path)?;
    let mut wrapper = wrapper_field.as_struct_mut()?;
    let mut result = None;
    wrapper.for_each_field_mut(|mut field| {
        if result.is_some()
            || field.as_ref().name() != "function"
            || field.as_ref().field_type() != TagFieldType::Struct
        {
            return;
        }
        let Some(mut mapping) = field.as_struct_mut() else {
            return;
        };
        let Some(mut data_field) = mapping.field_mut("data") else {
            return;
        };
        let Some(mut block) = data_field.as_block_mut() else {
            return;
        };
        block.clear();
        for byte in data.iter().copied() {
            let index = block.add_element();
            let Some(mut element) = block.element_mut(index) else {
                result = Some(Err("failed to create function byte element".to_owned()));
                return;
            };
            let Some(mut value_field) = element.field_mut("Value") else {
                result = Some(Err("function byte element missing Value field".to_owned()));
                return;
            };
            if let Err(error) = value_field.set(TagFieldData::CharInteger(byte as i8)) {
                result = Some(Err(format!("{error:?}")));
                return;
            }
        }
        result = Some(Ok(()));
    });
    result
}

pub(super) fn apply_h2_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<H2ShaderParamOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_one_h2_shader_param_op(tag, &op)
        }))
        .map_err(panic_message)
        .and_then(|result| result);
        match result {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("H2 shader edit failed: {error}"));
            }
        }
    }
    status
}

pub(super) fn apply_one_h2_shader_param_op(
    tag: &mut TagFile,
    op: &H2ShaderParamOp,
) -> Result<String, String> {
    match op {
        H2ShaderParamOp::EnsureParameter {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
        } => {
            let index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            Ok(format!(
                "Ensured H2 parameter '{}' at {}[{}]",
                parameter_name, parameters_block_path, index
            ))
        }
        H2ShaderParamOp::EnsureAnimationProperty {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            animation_type_index,
            initial_function_data,
        } => {
            let parameter_index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let animation_index = ensure_h2_animation_property(
                tag,
                parameters_block_path,
                parameter_index,
                *animation_type_index,
            )?;
            let data_path = format!(
                "{}[{}]/animation properties[{}]/function/data",
                parameters_block_path, parameter_index, animation_index
            );
            replace_halo2_function_byte_block(tag, &data_path, initial_function_data)?;
            Ok(format!(
                "Created H2 function row '{}' type {}",
                parameter_name, animation_type_index
            ))
        }
        H2ShaderParamOp::EditFunctionData { block_path, data } => {
            replace_halo2_function_byte_block(tag, block_path, data)?;
            Ok(format!("Edited H2 function data at {block_path}"))
        }
        H2ShaderParamOp::EditTemplateBackedValue {
            parameters_block_path,
            parameter_name,
            parameter_type_index,
            field,
            input,
        } => {
            let index = ensure_h2_shader_parameter(
                tag,
                parameters_block_path,
                parameter_name,
                *parameter_type_index,
            )?;
            let path = format!(
                "{}[{}]/{}",
                parameters_block_path,
                index,
                escape_field_path_segment(field)
            );
            apply_field_edit(tag, &path, input)?;
            Ok(format!(
                "Edited H2 parameter '{}' {}",
                parameter_name, field
            ))
        }
        H2ShaderParamOp::SwitchTemplate {
            parameters_block_path,
            allowed_parameter_names,
        } => {
            let allowed = allowed_parameter_names
                .iter()
                .map(|name| name.to_ascii_lowercase())
                .collect::<std::collections::HashSet<_>>();
            let Some(block) = tag
                .root()
                .field_path(parameters_block_path)
                .and_then(|field| field.as_block())
            else {
                return Ok("Updated H2 shader template".to_owned());
            };
            let mut delete_indices = Vec::new();
            for index in 0..block.len() {
                let Some(parameter) = block.element(index) else {
                    continue;
                };
                let name = parameter
                    .read_string_id("name")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if name.is_empty() || !allowed.contains(&name) {
                    delete_indices.push(index);
                }
            }
            let removed = delete_indices.len();
            for index in delete_indices.into_iter().rev() {
                apply_one_block_op(
                    tag,
                    &BlockOp {
                        path: parameters_block_path.clone(),
                        kind: BlockOpKind::Delete(index),
                    },
                )?;
            }
            Ok(format!(
                "Updated H2 shader template; pruned {removed} parameter(s)"
            ))
        }
    }
}

fn ensure_h2_shader_parameter(
    tag: &mut TagFile,
    parameters_block_path: &str,
    parameter_name: &str,
    parameter_type_index: i32,
) -> Result<usize, String> {
    if let Some(index) = h2_shader_parameter_index(tag, parameters_block_path, parameter_name) {
        return Ok(index);
    }
    let index = add_block_element(tag, parameters_block_path)?;
    apply_field_edit(
        tag,
        &format!("{parameters_block_path}[{index}]/name"),
        parameter_name,
    )?;
    apply_field_edit(
        tag,
        &format!("{parameters_block_path}[{index}]/type"),
        &parameter_type_index.to_string(),
    )?;
    Ok(index)
}

fn h2_shader_parameter_index(
    tag: &TagFile,
    parameters_block_path: &str,
    parameter_name: &str,
) -> Option<usize> {
    let block = tag
        .root()
        .field_path(parameters_block_path)
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find_map(|(index, element)| {
        (element.read_string_id("name").as_deref() == Some(parameter_name)).then_some(index)
    })
}

fn ensure_h2_animation_property(
    tag: &mut TagFile,
    parameters_block_path: &str,
    parameter_index: usize,
    animation_type_index: i32,
) -> Result<usize, String> {
    let animation_block_path =
        format!("{parameters_block_path}[{parameter_index}]/animation properties");
    if let Some(index) =
        h2_animation_property_index(tag, &animation_block_path, animation_type_index)
    {
        return Ok(index);
    }
    let index = add_block_element(tag, &animation_block_path)?;
    apply_field_edit(
        tag,
        &format!("{animation_block_path}[{index}]/type"),
        &animation_type_index.to_string(),
    )?;
    Ok(index)
}

fn h2_animation_property_index(
    tag: &TagFile,
    animation_block_path: &str,
    animation_type_index: i32,
) -> Option<usize> {
    let block = tag
        .root()
        .field_path(animation_block_path)
        .and_then(|field| field.as_block())?;
    block.iter().enumerate().find_map(|(index, element)| {
        (element
            .read_int_any("type")
            .and_then(|value| i32::try_from(value).ok())
            == Some(animation_type_index))
        .then_some(index)
    })
}

pub(super) fn apply_one_block_op(tag: &mut TagFile, op: &BlockOp) -> Result<String, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(&op.path)
        .ok_or_else(|| "block path no longer resolves".to_owned())?;
    if let Some(mut block) = field.as_block_mut() {
        return match &op.kind {
            BlockOpKind::Add => {
                let idx = block.add_element();
                Ok(format!("Added element {idx} to {}", op.path))
            }
            BlockOpKind::Insert(i) => {
                block.insert_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Inserted element at {i} in {}", op.path))
            }
            BlockOpKind::Duplicate(i) => {
                let idx = block.duplicate_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Duplicated element {i} → {idx} in {}", op.path))
            }
            BlockOpKind::Delete(i) => {
                block.delete_element(*i).map_err(|e| format!("{e:?}"))?;
                Ok(format!("Deleted element {i} from {}", op.path))
            }
            BlockOpKind::DeleteAll => {
                block.clear();
                Ok(format!("Cleared {}", op.path))
            }
            BlockOpKind::Paste { at, elements } => {
                paste_elements(&mut block, *at, elements)?;
                Ok(format!(
                    "Pasted {} element(s) into {}",
                    elements.len(),
                    op.path
                ))
            }
            BlockOpKind::ReplaceElement { at, elements } => {
                block.delete_element(*at).map_err(|e| format!("{e:?}"))?;
                paste_elements(&mut block, *at, elements)?;
                Ok(format!(
                    "Replaced element {at} with {} element(s) in {}",
                    elements.len(),
                    op.path
                ))
            }
            BlockOpKind::ReplaceBlock { elements } => {
                block.clear();
                paste_elements(&mut block, 0, elements)?;
                Ok(format!(
                    "Replaced {} with {} element(s)",
                    op.path,
                    elements.len()
                ))
            }
        };
    }
    // Arrays are fixed-count: insert/delete can't apply, but an element can be
    // replaced in place with a copied element of the same struct.
    if let Some(mut array) = field.as_array_mut() {
        return match &op.kind {
            BlockOpKind::ReplaceElement { at, elements } => {
                let element = elements
                    .first()
                    .ok_or_else(|| "clipboard has no element".to_owned())?;
                array
                    .replace_element(*at, element)
                    .map_err(|error| format!("{error:?}"))?;
                Ok(format!("Replaced element {at} in {}", op.path))
            }
            _ => Err(
                "arrays are fixed-size — only replacing an element in place is supported"
                    .to_owned(),
            ),
        };
    }
    Err("field is not a block or array".to_owned())
}

/// Insert `elements` consecutively starting at `at`, preserving their order.
fn paste_elements(
    block: &mut blam_tags::TagBlockMut<'_>,
    at: usize,
    elements: &[blam_tags::TagBlockElement],
) -> Result<(), String> {
    for (offset, element) in elements.iter().enumerate() {
        block
            .paste_element(at + offset, element)
            .map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

pub(super) fn apply_field_edit(tag: &mut TagFile, path: &str, input: &str) -> Result<(), String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| "field path no longer resolves".to_owned())?;
    let field_ref = field.as_ref();
    if is_subchunk_backed_field(field_ref.field_type()) && field_ref.value().is_none() {
        return Err("field data is absent in this tag version".to_owned());
    }
    let value = parse_gui_field_value(&field_ref, input)?;
    field.set(value).map_err(|error| format!("{error:?}"))
}

fn is_subchunk_backed_field(field_type: TagFieldType) -> bool {
    matches!(
        field_type,
        TagFieldType::StringId
            | TagFieldType::OldStringId
            | TagFieldType::TagReference
            | TagFieldType::Data
            | TagFieldType::ApiInterop
    )
}

pub(super) fn apply_shader_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader op failed: {error}"));
            }
        }
    }
    status
}

pub(super) fn apply_shader_param_ops(
    tag: &mut TagFile,
    ops: Vec<ShaderParamOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_shader_param_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Shader param op failed: {error}"));
            }
        }
    }
    status
}

pub(super) fn apply_model_variant_ops(
    tag: &mut TagFile,
    ops: Vec<ModelVariantOp>,
    dirty: &mut bool,
) -> Option<String> {
    let mut status = None;
    for op in ops {
        match apply_one_model_variant_op(tag, &op) {
            Ok(msg) => {
                *dirty = true;
                status = Some(msg);
            }
            Err(error) => {
                status = Some(format!("Model variant edit failed: {error}"));
            }
        }
    }
    status
}

fn apply_one_model_variant_op(tag: &mut TagFile, op: &ModelVariantOp) -> Result<String, String> {
    match op {
        ModelVariantOp::Create { name, regions } => {
            let variant_index = add_block_element(tag, "variants")?;
            apply_field_edit(tag, &format!("variants[{variant_index}]/name"), name)?;
            write_model_variant_regions(tag, variant_index, regions)?;
            Ok(format!("Created model variant '{name}'"))
        }
        ModelVariantOp::Update {
            variant_index,
            regions,
        } => {
            ensure_block_element_exists(tag, "variants", *variant_index)?;
            write_model_variant_regions(tag, *variant_index, regions)?;
            Ok(format!("Updated model variant {}", variant_index))
        }
        ModelVariantOp::Drop { variant_index } => {
            let mut root = tag.root_mut();
            let mut field = root
                .field_path_mut("variants")
                .ok_or_else(|| "variants block not found".to_owned())?;
            let mut block = field
                .as_block_mut()
                .ok_or_else(|| "variants is not a block".to_owned())?;
            block
                .delete_element(*variant_index)
                .map_err(|e| format!("{e:?}"))?;
            Ok(format!("Deleted model variant {}", variant_index))
        }
    }
}

fn write_model_variant_regions(
    tag: &mut TagFile,
    variant_index: usize,
    regions: &[ModelVariantRegionChoice],
) -> Result<(), String> {
    let regions_path = format!("variants[{variant_index}]/regions");
    clear_block(tag, &regions_path)?;
    for region in regions {
        let region_index = add_block_element(tag, &regions_path)?;
        apply_field_edit(
            tag,
            &format!("{regions_path}[{region_index}]/region name"),
            &region.region_name,
        )?;
        let permutations_path = format!("{regions_path}[{region_index}]/permutations");
        let permutation_index = add_block_element(tag, &permutations_path)?;
        apply_field_edit(
            tag,
            &format!("{permutations_path}[{permutation_index}]/permutation name"),
            &region.permutation_name,
        )?;
    }
    Ok(())
}

fn ensure_block_element_exists(tag: &TagFile, path: &str, index: usize) -> Result<(), String> {
    let block = tag
        .root()
        .field_path(path)
        .and_then(|field| field.as_block())
        .ok_or_else(|| format!("{path} block not found"))?;
    if index < block.len() {
        Ok(())
    } else {
        Err(format!("{path}[{index}] is out of range"))
    }
}

pub(super) fn add_block_element(tag: &mut TagFile, path: &str) -> Result<usize, String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| format!("{path} block not found"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| format!("{path} is not a block"))?;
    Ok(block.add_element())
}

fn clear_block(tag: &mut TagFile, path: &str) -> Result<(), String> {
    let mut root = tag.root_mut();
    let mut field = root
        .field_path_mut(path)
        .ok_or_else(|| format!("{path} block not found"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| format!("{path} is not a block"))?;
    block.clear();
    Ok(())
}

pub(super) fn apply_one_shader_param_op(
    tag: &mut TagFile,
    op: &ShaderParamOp,
) -> Result<String, String> {
    // Step 1: append a new element to the parameters block.
    let new_idx = {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(&op.parameters_block_path)
            .ok_or_else(|| format!("parameters block not found: {}", op.parameters_block_path))?;
        let mut block = field
            .as_block_mut()
            .ok_or_else(|| format!("not a block: {}", op.parameters_block_path))?;
        block.add_element()
    };

    // Step 2: write parameter name.
    let name_path = format!("{}[{}]/parameter name", op.parameters_block_path, new_idx);
    apply_field_edit(tag, &name_path, &op.parameter_name)?;

    // Step 3: initialise requested fields.
    for initial in &op.initial_fields {
        let field = escape_field_path_segment(&initial.field);
        let field_path = format!("{}[{}]/{}", op.parameters_block_path, new_idx, field);
        apply_field_edit(tag, &field_path, &initial.input)?;
    }

    for animated in &op.animated_parameters {
        let animated_block_path = format!(
            "{}[{}]/animated parameters",
            op.parameters_block_path, new_idx
        );
        apply_one_shader_op(
            tag,
            &ShaderOp {
                animated_block_path,
                output_type_index: animated.output_type_index,
                initial_function_hex: animated.initial_function_hex.clone(),
            },
        )?;
    }

    Ok(format!(
        "Created parameter '{}' at {}[{}]",
        op.parameter_name, op.parameters_block_path, new_idx
    ))
}

pub(super) fn apply_one_shader_op(tag: &mut TagFile, op: &ShaderOp) -> Result<String, String> {
    // Step 1: append one element to the animated-parameters block and capture its index.
    let new_idx = {
        let mut root = tag.root_mut();
        let mut field = root
            .field_path_mut(&op.animated_block_path)
            .ok_or_else(|| {
                format!(
                    "animated params block not found: {}",
                    op.animated_block_path
                )
            })?;
        let mut block = field
            .as_block_mut()
            .ok_or_else(|| format!("not a block: {}", op.animated_block_path))?;
        block.add_element()
    };

    // Step 2: set the output `type` field on the newly created element.
    let type_path = format!("{}[{}]/type", op.animated_block_path, new_idx);
    apply_field_edit(tag, &type_path, &op.output_type_index.to_string())?;

    // Step 3: write the initial `mapping_function` blob into `function/data`.
    let data_path = format!("{}[{}]/function/data", op.animated_block_path, new_idx);
    apply_field_edit(tag, &data_path, &op.initial_function_hex)?;

    Ok(format!(
        "Added animated parameter (type {}) at {}[{}]",
        op.output_type_index, op.animated_block_path, new_idx
    ))
}

pub(super) fn is_editable_tag(entry: &TagEntry, tag: &TagFile) -> bool {
    matches!(entry.location, TagEntryLocation::LooseFile(_))
        && (tag.classic_engine().is_some() || tag.endian == Endian::Le)
}

pub(super) fn append_field_path(prefix: &str, field_name: &str) -> String {
    if prefix.is_empty() {
        field_name.to_owned()
    } else {
        format!("{prefix}/{field_name}")
    }
}

pub(super) fn escape_field_path_segment(field_name: &str) -> String {
    field_name.replace('\\', "\\\\").replace('/', "\\/")
}

pub(super) fn is_text_editable_value(value: &TagFieldData) -> bool {
    !matches!(
        value,
        TagFieldData::Data(_)
            | TagFieldData::ApiInterop(_)
            | TagFieldData::Custom(_)
            | TagFieldData::Point2d(_)
            | TagFieldData::Rectangle2d(_)
            | TagFieldData::RealPoint2d(_)
            | TagFieldData::RealPoint3d(_)
            | TagFieldData::RealVector2d(_)
            | TagFieldData::RealVector3d(_)
            | TagFieldData::RealQuaternion(_)
            | TagFieldData::RealEulerAngles2d(_)
            | TagFieldData::RealEulerAngles3d(_)
            | TagFieldData::RealPlane2d(_)
            | TagFieldData::RealPlane3d(_)
            | TagFieldData::RgbColor(_)
            | TagFieldData::ArgbColor(_)
            | TagFieldData::RealRgbColor(_)
            | TagFieldData::RealArgbColor(_)
            | TagFieldData::RealHsvColor(_)
            | TagFieldData::RealAhsvColor(_)
            | TagFieldData::ShortIntegerBounds(_)
            | TagFieldData::AngleBounds(_)
            | TagFieldData::RealBounds(_)
            | TagFieldData::FractionBounds(_)
    )
}

pub(super) fn parse_gui_field_value(
    field: &TagField<'_>,
    input: &str,
) -> Result<TagFieldData, String> {
    let trimmed = input.trim();
    match field.field_type() {
        TagFieldType::CharInteger => parse_value(trimmed, "i8").map(TagFieldData::CharInteger),
        TagFieldType::ShortInteger => parse_value(trimmed, "i16").map(TagFieldData::ShortInteger),
        TagFieldType::LongInteger => parse_value(trimmed, "i32").map(TagFieldData::LongInteger),
        TagFieldType::Int64Integer => parse_value(trimmed, "i64").map(TagFieldData::Int64Integer),
        TagFieldType::ByteInteger => parse_value(trimmed, "u8").map(TagFieldData::ByteInteger),
        TagFieldType::WordInteger => parse_value(trimmed, "u16").map(TagFieldData::WordInteger),
        TagFieldType::DwordInteger => parse_value(trimmed, "u32").map(TagFieldData::DwordInteger),
        TagFieldType::QwordInteger => parse_value(trimmed, "u64").map(TagFieldData::QwordInteger),
        TagFieldType::Tag => parse_group_tag(trimmed)
            .map(TagFieldData::Tag)
            .ok_or_else(|| "expected 1..=4 ASCII group tag".to_owned()),
        TagFieldType::Angle => parse_value(trimmed, "f32").map(TagFieldData::Angle),
        TagFieldType::ShortIntegerBounds => {
            let (lower, upper) = parse_short_bounds(trimmed, "short bounds")?;
            Ok(TagFieldData::ShortIntegerBounds(
                blam_tags::math::ShortBounds { lower, upper },
            ))
        }
        TagFieldType::AngleBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "angle bounds")?;
            Ok(TagFieldData::AngleBounds(blam_tags::math::AngleBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::RealBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "real bounds")?;
            Ok(TagFieldData::RealBounds(blam_tags::math::RealBounds {
                lower,
                upper,
            }))
        }
        TagFieldType::FractionBounds => {
            let (lower, upper) = parse_float_bounds(trimmed, "fraction bounds")?;
            Ok(TagFieldData::FractionBounds(
                blam_tags::math::FractionBounds { lower, upper },
            ))
        }
        TagFieldType::RealVector2d => {
            let [i, j] = parse_float_channels::<2>(trimmed, "real vector 2d")?;
            Ok(TagFieldData::RealVector2d(blam_tags::math::RealVector2d {
                i,
                j,
            }))
        }
        TagFieldType::RealVector3d => {
            let [i, j, k] = parse_float_channels::<3>(trimmed, "real vector 3d")?;
            Ok(TagFieldData::RealVector3d(blam_tags::math::RealVector3d {
                i,
                j,
                k,
            }))
        }
        TagFieldType::RealPoint2d => {
            let [x, y] = parse_float_channels::<2>(trimmed, "real point 2d")?;
            Ok(TagFieldData::RealPoint2d(blam_tags::math::RealPoint2d {
                x,
                y,
            }))
        }
        TagFieldType::RealPoint3d => {
            let [x, y, z] = parse_float_channels::<3>(trimmed, "real point 3d")?;
            Ok(TagFieldData::RealPoint3d(blam_tags::math::RealPoint3d {
                x,
                y,
                z,
            }))
        }
        TagFieldType::RealQuaternion => {
            let [i, j, k, w] = parse_float_channels::<4>(trimmed, "real quaternion")?;
            Ok(TagFieldData::RealQuaternion(
                blam_tags::math::RealQuaternion { i, j, k, w },
            ))
        }
        TagFieldType::Real => parse_value(trimmed, "f32").map(TagFieldData::Real),
        TagFieldType::RealSlider => parse_value(trimmed, "f32").map(TagFieldData::RealSlider),
        TagFieldType::RealFraction => parse_value(trimmed, "f32").map(TagFieldData::RealFraction),
        TagFieldType::CharEnum => Ok(TagFieldData::CharEnum {
            value: parse_enum_value(field, trimmed)? as i8,
            name: None,
        }),
        TagFieldType::ShortEnum => Ok(TagFieldData::ShortEnum {
            value: parse_enum_value(field, trimmed)? as i16,
            name: None,
        }),
        TagFieldType::LongEnum => Ok(TagFieldData::LongEnum {
            value: parse_enum_value(field, trimmed)?,
            name: None,
        }),
        TagFieldType::ByteFlags => Ok(TagFieldData::ByteFlags {
            value: parse_int_mask(trimmed)? as u8,
            names: Vec::new(),
        }),
        TagFieldType::WordFlags => Ok(TagFieldData::WordFlags {
            value: parse_int_mask(trimmed)? as u16,
            names: Vec::new(),
        }),
        TagFieldType::LongFlags => Ok(TagFieldData::LongFlags {
            value: parse_int_mask(trimmed)? as i32,
            names: Vec::new(),
        }),
        TagFieldType::ByteBlockFlags => {
            Ok(TagFieldData::ByteBlockFlags(parse_int_mask(trimmed)? as u8))
        }
        TagFieldType::WordBlockFlags => {
            Ok(TagFieldData::WordBlockFlags(parse_int_mask(trimmed)? as u16))
        }
        TagFieldType::LongBlockFlags => {
            Ok(TagFieldData::LongBlockFlags(parse_int_mask(trimmed)? as i32))
        }
        TagFieldType::CharBlockIndex => Ok(TagFieldData::CharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::CustomCharBlockIndex => Ok(TagFieldData::CustomCharBlockIndex(
            parse_block_index(trimmed)? as i8,
        )),
        TagFieldType::ShortBlockIndex => Ok(TagFieldData::ShortBlockIndex(parse_block_index(
            trimmed,
        )? as i16)),
        TagFieldType::CustomShortBlockIndex => Ok(TagFieldData::CustomShortBlockIndex(
            parse_block_index(trimmed)? as i16,
        )),
        TagFieldType::LongBlockIndex => {
            Ok(TagFieldData::LongBlockIndex(parse_block_index(trimmed)?))
        }
        TagFieldType::CustomLongBlockIndex => Ok(TagFieldData::CustomLongBlockIndex(
            parse_block_index(trimmed)?,
        )),
        TagFieldType::String => Ok(TagFieldData::String(trimmed.to_owned())),
        TagFieldType::LongString => Ok(TagFieldData::LongString(trimmed.to_owned())),
        TagFieldType::StringId => Ok(TagFieldData::StringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::OldStringId => Ok(TagFieldData::OldStringId(StringIdData {
            string: parse_none_string(trimmed),
        })),
        TagFieldType::TagReference => parse_tag_reference(trimmed).map(TagFieldData::TagReference),
        // Color values: comma-separated floats, written by the color picker
        // swatch. RGB = "r, g, b"; ARGB = "a, r, g, b".
        TagFieldType::RgbColor => {
            let (_, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::RgbColor(blam_tags::math::RgbColor(raw)))
        }
        TagFieldType::ArgbColor => {
            let (a, r, g, b) = parse_rgb_or_argb_color_channels(trimmed)?;
            let raw = ((color_float_to_u8(a) as u32) << 24)
                | ((color_float_to_u8(r) as u32) << 16)
                | ((color_float_to_u8(g) as u32) << 8)
                | color_float_to_u8(b) as u32;
            Ok(TagFieldData::ArgbColor(blam_tags::math::ArgbColor(raw)))
        }
        TagFieldType::RealRgbColor => {
            let [r, g, b] = parse_color_channels::<3>(trimmed)?;
            Ok(TagFieldData::RealRgbColor(blam_tags::math::RealRgbColor {
                red: r,
                green: g,
                blue: b,
            }))
        }
        TagFieldType::RealArgbColor => {
            let [a, r, g, b] = parse_color_channels::<4>(trimmed)?;
            Ok(TagFieldData::RealArgbColor(
                blam_tags::math::RealArgbColor {
                    alpha: a,
                    red: r,
                    green: g,
                    blue: b,
                },
            ))
        }
        // Raw byte blobs (e.g. a `mapping_function` `data` field) are
        // carried through the string edit channel as lowercase hex. The
        // function editor produces these via `push_function_edit`.
        TagFieldType::Data => decode_hex(trimmed).map(TagFieldData::Data),
        _ => Err(format!(
            "editing {} fields is not supported yet",
            field.type_name()
        )),
    }
}

/// Decode a contiguous lowercase/uppercase hex string (no separators)
/// into bytes. Used to ferry function blobs through `PendingFieldEdit`.
pub(super) fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    let s = input.trim();
    if s.len() % 2 != 0 {
        return Err("hex blob must have an even number of digits".to_owned());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| "invalid hex digit".to_owned())?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

/// Encode bytes as a contiguous lowercase hex string.
pub(super) fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
    }
    out
}

pub(super) fn parse_value<T: std::str::FromStr>(input: &str, expected: &str) -> Result<T, String> {
    input
        .parse()
        .map_err(|_| format!("expected {expected} value"))
}

/// Parse exactly `N` comma-separated float channels (used for color values).
pub(super) fn parse_color_channels<const N: usize>(input: &str) -> Result<[f32; N], String> {
    let parts: Vec<f32> = input
        .split(',')
        .map(|part| part.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("expected {N} comma-separated color channels"))?;
    parts
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} comma-separated color channels"))
}

pub(super) fn parse_rgb_or_argb_color_channels(
    input: &str,
) -> Result<(f32, f32, f32, f32), String> {
    let parts = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| parse_value::<f32>(part, "color channel"))
        .collect::<Result<Vec<_>, _>>()?;
    match parts.as_slice() {
        [r, g, b] => Ok((1.0, *r, *g, *b)),
        [a, r, g, b] => Ok((*a, *r, *g, *b)),
        _ => Err("expected 3 or 4 comma-separated color channels".to_owned()),
    }
}

pub(super) fn color_float_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub(super) fn parse_float_channels<const N: usize>(
    input: &str,
    expected: &str,
) -> Result<[f32; N], String> {
    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let values = parts
        .into_iter()
        .map(|part| part.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("expected {N} values for {expected}"))?;
    values
        .try_into()
        .map_err(|_: Vec<f32>| format!("expected {N} values for {expected}"))
}

pub(super) fn parse_float_bounds(input: &str, expected: &str) -> Result<(f32, f32), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(super) fn parse_short_bounds(input: &str, expected: &str) -> Result<(i16, i16), String> {
    let (lower, upper) = parse_bounds_parts(input, expected)?;
    Ok((
        lower
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
        upper
            .parse()
            .map_err(|_| format!("expected {expected} as lower..upper"))?,
    ))
}

pub(super) fn parse_bounds_parts<'a>(
    input: &'a str,
    expected: &str,
) -> Result<(&'a str, &'a str), String> {
    if let Some((lower, upper)) = input.split_once("..") {
        return Ok((lower.trim(), upper.trim()));
    }
    if let Some((lower, upper)) = input.split_once(" to ") {
        return Ok((lower.trim(), upper.trim()));
    }

    let parts = if input.contains(',') {
        input
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
    } else {
        input.split_whitespace().collect::<Vec<_>>()
    };
    let [lower, upper]: [&str; 2] = parts
        .try_into()
        .map_err(|_| format!("expected {expected} as lower..upper"))?;
    Ok((lower, upper))
}

pub(super) fn parse_none_string(input: &str) -> String {
    if input.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        input.to_owned()
    }
}

pub(super) fn parse_block_index(input: &str) -> Result<i32, String> {
    if input.eq_ignore_ascii_case("none") {
        Ok(-1)
    } else {
        parse_value(input, "block index")
    }
}

pub(super) fn parse_int_mask(input: &str) -> Result<u64, String> {
    if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).map_err(|_| "expected integer mask".to_owned())
    } else {
        input
            .parse()
            .map_err(|_| "expected integer mask".to_owned())
    }
}

pub(super) fn parse_enum_value(field: &TagField<'_>, input: &str) -> Result<i32, String> {
    if let Ok(value) = input.parse() {
        return Ok(value);
    }
    if let Some(blam_tags::TagOptions::Enum { names, .. }) = field.options() {
        if let Some((index, _)) = names
            .iter()
            .enumerate()
            .find(|(_, name)| name.eq_ignore_ascii_case(input))
        {
            return Ok(index as i32);
        }
    }
    Err("expected enum name or integer".to_owned())
}

pub(super) fn parse_tag_reference(input: &str) -> Result<TagReferenceData, String> {
    if input.eq_ignore_ascii_case("none") || input.is_empty() {
        return Ok(TagReferenceData {
            group_tag_and_name: None,
        });
    }
    if let Some((group, path)) = input.split_once(':') {
        let group_tag = parse_group_tag(group)
            .ok_or_else(|| "tag reference group must be 1..=4 ASCII chars".to_owned())?;
        return Ok(TagReferenceData {
            group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
        });
    }
    let Some((path, extension)) = input.rsplit_once('.') else {
        return Err("expected <path>.<group> or GROUP:<path>".to_owned());
    };
    let group_tag = extension_to_group_tag(extension)
        .or_else(|| parse_group_tag(extension))
        .ok_or_else(|| format!("unknown tag group {extension:?}"))?;
    Ok(TagReferenceData {
        group_tag_and_name: Some((group_tag, path.replace('/', "\\"))),
    })
}

pub(super) fn field_display_meta(name: &str) -> FieldDisplayMeta {
    let mut text = name.trim().to_owned();
    let advanced = text.ends_with('*');
    if advanced {
        text.pop();
    }
    let read_only = text.ends_with('!');
    if read_only {
        text.pop();
    }
    let (text, help) = match text.split_once('#') {
        Some((label, help)) => (label.trim().to_owned(), Some(help.trim().to_owned())),
        None => (text, None),
    };
    // A `[min,max]` range may sit in the unit slot (`name:[0,1]`) or bare in the
    // name (`max sounds [1,16]`); pull it out so the unit and range are distinct.
    let (text, range) = extract_range_hint(&text);
    let (label, unit) = match text.split_once(':') {
        Some((label, unit)) => (label.trim().to_owned(), Some(unit.trim().to_owned())),
        None => (text.trim().to_owned(), None),
    };
    FieldDisplayMeta {
        label: clean_field_name_basic(&label),
        unit: unit.filter(|unit| !unit.is_empty()),
        range,
        help: help.filter(|help| !help.is_empty()),
        tag_reference_allowed: Vec::new(),
        read_only,
        advanced,
    }
}

/// Split a `[ … ]` range hint out of `text`, returning `(remainder, range)`.
/// The range keeps its brackets (e.g. `[0,+inf]`).
fn extract_range_hint(text: &str) -> (String, Option<String>) {
    match (text.find('['), text.rfind(']')) {
        (Some(open), Some(close)) if open < close => {
            let range = text[open..=close].to_owned();
            let mut rest = text[..open].to_owned();
            rest.push_str(&text[close + 1..]);
            (rest, Some(range))
        }
        _ => (text.to_owned(), None),
    }
}

/// Metadata shown after a field's value: the unit (preferred over the type
/// name), then the `[range]` hint if present.
pub(super) fn field_suffix(meta: &FieldDisplayMeta, type_name: &str) -> String {
    let base = meta
        .unit
        .clone()
        .unwrap_or_else(|| clean_type_name(type_name));
    match &meta.range {
        Some(range) => {
            if base.is_empty() {
                range.clone()
            } else {
                format!("{base} {range}")
            }
        }
        None => base,
    }
}

pub(super) fn draw_field_help(ui: &mut Ui, meta: &FieldDisplayMeta) {
    // Field documentation is shown on hover over the name label (see
    // `foundation_label_cell`); this only surfaces the read-only marker.
    if meta.read_only {
        ui.label(RichText::new("read-only").color(subtle_dark()).small());
    }
}

pub(super) fn enum_option_label(options: &[&str], selected: i64) -> String {
    if selected < 0 {
        return "NONE".to_owned();
    }
    options
        .get(selected as usize)
        .map(|name| format!("{selected}. {name}"))
        .unwrap_or_else(|| selected.to_string())
}

pub(super) fn extension_to_group_tag(extension: &str) -> Option<u32> {
    let fourcc = match extension {
        "material" => "mat",
        "material_shader" => "mats",
        "material_effects" => "foot",
        "object" => "obje",
        "model" => "hlmt",
        "character" => "char",
        "style" => "styl",
        "unit" => "unit",
        "render_model" => "mode",
        "collision_model" => "coll",
        "physics_model" => "phmo",
        "model_animation_graph" => "jmad",
        "biped" => "bipd",
        "vehicle" => "vehi",
        "weapon" => "weap",
        "equipment" => "eqip",
        "item" => "item",
        "giant" => "gint",
        "creature" => "crea",
        "scenery" => "scen",
        "crate" => "crat",
        "bitmap" => "bitm",
        "scenario_structure_bsp" => "sbsp",
        "scenario" => "scnr",
        "projectile" => "proj",
        "effect" => "effe",
        "effect_scenery" => "efsc",
        "damage_effect" => "jpt!",
        "sound" => "snd!",
        "sound_looping" => "lsnd",
        "sound_scenery" => "ssce",
        "dialogue" => "udlg",
        "light" => "ligh",
        "lens_flare" => "lens",
        "camera_track" => "trak",
        "device" => "devi",
        "device_control" => "ctrl",
        "device_machine" => "mach",
        "device_terminal" => "term",
        "globals" => "matg",
        "shader" => "rmsh",
        "shader_terrain" => "rmtr",
        "shader_water" => "rmw ",
        "shader_foliage" => "rmfl",
        "shader_decal" => "rmd ",
        "shader_halogram" => "rmhg",
        "shader_skin" => "rmsk",
        "shader_cortana" => "rmct",
        "shader_custom" => "rmcs",
        "shader_particle" => "rmp ",
        "shader_beam" => "rmb ",
        "shader_contrail" => "rmco",
        "shader_light_volume" => "rmlv",
        _ => return None,
    };
    parse_group_tag(fourcc)
}

pub(super) fn draw_tag_metadata(ui: &mut Ui, tag: &TagFile, names: &TagNameIndex) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Header group:").color(subtle_dark()));
        ui.monospace(RichText::new(group_label(names, tag.group().tag)).color(text_dark()));
        ui.label(RichText::new("Version:").color(subtle_dark()));
        ui.monospace(RichText::new(tag.group().version.to_string()).color(text_dark()));
        ui.label(RichText::new("Endian:").color(subtle_dark()));
        ui.monospace(
            RichText::new(match tag.endian {
                Endian::Le => "LE",
                Endian::Be => "BE",
            })
            .color(text_dark()),
        );
    });
}

pub(super) fn draw_bitmap_tag(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    _color_popup: &mut Option<MaterialColorPopup>,
    preview: &mut BitmapPreviewState,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    draw_tag_metadata(ui, tag, names);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let can_reimport = bitmap_reimport_data_path(entry, edit.tags_root).is_some();
        if ui
            .add_enabled(can_reimport, egui::Button::new("Reimport"))
            .on_hover_text("Run tool bitmaps for this bitmap source path, then reload the tag")
            .clicked()
        {
            *edit.bitmap_reimport = Some(entry.key.clone());
        }
        ui.separator();
        ui.selectable_value(&mut preview.active_tab, BitmapPanelTab::Fields, "Fields");
        ui.selectable_value(
            &mut preview.active_tab,
            BitmapPanelTab::Texture,
            "Texture preview",
        );
    });
    ui.separator();

    match preview.active_tab {
        BitmapPanelTab::Fields => {
            ScrollArea::both()
                .id_salt(("bitmap_fields_scroll", edit.view_scope, edit.tag_key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                    draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
                });
        }
        BitmapPanelTab::Texture => draw_bitmap_preview(ui, ctx, tag, entry, preview),
    }
}

pub(super) fn bitmap_reimport_data_path(
    entry: &TagEntry,
    tags_root: Option<&Path>,
) -> Option<String> {
    let TagEntryLocation::LooseFile(path) = &entry.location else {
        return None;
    };
    let tags_root = tags_root?;
    let rel = path.strip_prefix(tags_root).ok()?;
    let mut source = rel.to_path_buf();
    source.set_extension("");
    Some(source.to_string_lossy().replace('/', "\\"))
}

pub(super) fn draw_bitmap_preview(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    preview: &mut BitmapPreviewState,
) {
    if preview.decoded.is_none() {
        preview.decoded = Some(
            build_bitmap_preview(tag, preview.image_index, preview.mip_index)
                .map_err(|error| error.to_string()),
        );
        preview.texture_dirty = true;
    }

    let Some(decoded) = preview.decoded.as_ref() else {
        return;
    };
    let data = match decoded {
        Ok(data) => data,
        Err(error) => {
            ui.colored_label(Color32::from_rgb(130, 32, 24), error);
            return;
        }
    };

    ui.horizontal(|ui| {
        let red_changed = ui.checkbox(&mut preview.show_red, "Red").changed();
        let green_changed = ui.checkbox(&mut preview.show_green, "Green").changed();
        let blue_changed = ui.checkbox(&mut preview.show_blue, "Blue").changed();
        let alpha_changed = ui.checkbox(&mut preview.show_alpha, "Alpha").changed();
        if red_changed || green_changed || blue_changed || alpha_changed {
            preview.texture_dirty = true;
        }
    });
    // Deferred re-decode: index fields are disjoint from `decoded` so we can
    // write them now, but `decoded = None` must wait until `data`'s borrow ends
    // (applied at the end of the function).
    let mut redecode = false;
    ui.horizontal(|ui| {
        // Image (sequence) selector.
        if data.image_count > 1 {
            ui.label(RichText::new("Image").color(subtle_dark()));
            if ui
                .add_enabled(preview.image_index > 0, egui::Button::new("◀"))
                .clicked()
            {
                preview.image_index -= 1;
                preview.mip_index = 0;
                redecode = true;
            }
            ui.monospace(
                RichText::new(format!("{}/{}", preview.image_index, data.image_count - 1))
                    .color(text_dark()),
            );
            if ui
                .add_enabled(
                    preview.image_index + 1 < data.image_count,
                    egui::Button::new("▶"),
                )
                .clicked()
            {
                preview.image_index += 1;
                preview.mip_index = 0;
                redecode = true;
            }
            ui.separator();
        } else {
            ui.label(RichText::new("Image 0").color(subtle_dark()));
        }
        // Mip-level selector.
        if data.mip_count > 1 {
            ui.label(RichText::new("Mip").color(subtle_dark()));
            if ui
                .add_enabled(preview.mip_index > 0, egui::Button::new("◀"))
                .clicked()
            {
                preview.mip_index -= 1;
                redecode = true;
            }
            ui.monospace(
                RichText::new(format!("{}/{}", preview.mip_index, data.mip_count - 1))
                    .color(text_dark()),
            );
            if ui
                .add_enabled(
                    preview.mip_index + 1 < data.mip_count,
                    egui::Button::new("▶"),
                )
                .clicked()
            {
                preview.mip_index += 1;
                redecode = true;
            }
            ui.separator();
        }
        ui.monospace(RichText::new(format!("{} x {}", data.width, data.height)).color(text_dark()));
        ui.label(RichText::new(&data.format_name).color(subtle_dark()));
        ui.label(RichText::new(&data.type_name).color(subtle_dark()));
        ui.separator();
        ui.label(RichText::new(format!("Zoom {:.0}%", preview.zoom * 100.0)).color(subtle_dark()));
        let (_, zoom_wheel_delta) = combo_box_with_scroll(
            ui,
            egui::ComboBox::from_id_salt(("bitmap_zoom_preset", &entry.key))
                .selected_text("Set…")
                .width(58.0),
            |ui| {
                if ui.selectable_label(false, "Fit").clicked() {
                    preview.zoom_initialized = false; // refit next frame
                    preview.pan = Vec2::ZERO;
                }
                for pct in [25u32, 50, 100, 200, 400] {
                    if ui.selectable_label(false, format!("{pct}%")).clicked() {
                        preview.zoom = pct as f32 / 100.0;
                        preview.zoom_initialized = true;
                        preview.pan = Vec2::ZERO;
                    }
                }
            },
        );
        if let Some(delta) = zoom_wheel_delta {
            let presets = [0u32, 25, 50, 100, 200, 400];
            let current_pct = (preview.zoom * 100.0).round() as u32;
            let current = presets
                .iter()
                .position(|pct| *pct == current_pct)
                .unwrap_or_else(|| {
                    presets
                        .iter()
                        .enumerate()
                        .skip(1)
                        .min_by_key(|(_, pct)| pct.abs_diff(current_pct))
                        .map(|(index, _)| index)
                        .unwrap_or(0)
                });
            if let Some(next) = combo_scroll_next_index(current, presets.len(), delta) {
                if presets[next] == 0 {
                    preview.zoom_initialized = false;
                } else {
                    preview.zoom = presets[next] as f32 / 100.0;
                    preview.zoom_initialized = true;
                }
                preview.pan = Vec2::ZERO;
            }
        }
        if ui.button("Reset zoom").clicked() {
            preview.zoom_initialized = false; // triggers fit-to-view on next frame
            preview.pan = Vec2::ZERO;
        }
        ui.separator();
        ui.label(RichText::new("BG").color(subtle_dark()));
        let (_, bg_wheel_delta) = combo_box_with_scroll(
            ui,
            egui::ComboBox::from_id_salt(("bitmap_bg", &entry.key))
                .selected_text(preview.bg.label())
                .width(86.0),
            |ui| {
                for bg in BitmapPreviewBg::ALL {
                    if ui.selectable_label(preview.bg == bg, bg.label()).clicked() {
                        preview.bg = bg;
                    }
                }
            },
        );
        if let Some(delta) = bg_wheel_delta {
            let current = BitmapPreviewBg::ALL
                .iter()
                .position(|bg| *bg == preview.bg)
                .unwrap_or(0);
            if let Some(next) = combo_scroll_next_index(current, BitmapPreviewBg::ALL.len(), delta)
            {
                preview.bg = BitmapPreviewBg::ALL[next];
            }
        }
    });
    ui.add_space(6.0);

    if preview.texture_dirty || preview.texture.is_none() {
        let rgba = filtered_bitmap_rgba(data, preview);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [data.width as usize, data.height as usize],
            &rgba,
        );
        if let Some(texture) = preview.texture.as_mut() {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            preview.texture = Some(ctx.load_texture(
                format!("bitmap_preview_{}", entry.key),
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        preview.texture_dirty = false;
    }

    let Some(texture) = preview.texture.as_ref() else {
        return;
    };
    let image_size = texture.size_vec2();

    // Allocate the whole remaining area as a fixed canvas and handle pan/zoom
    // manually. Using a ScrollArea here causes the scroll wheel to both zoom
    // (our code) and pan the viewport (egui), which fight and "teleport".
    let canvas_size = ui.available_size();
    let (canvas_rect, canvas_resp) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());

    // Fit zoom = the scale at which the whole texture fits the canvas (never
    // upscaling past 1:1). This is both the initial zoom and the minimum the
    // user can zoom out to — you can't shrink the texture smaller than fit.
    let fit_zoom = if canvas_rect.width() > 1.0
        && canvas_rect.height() > 1.0
        && image_size.x > 0.0
        && image_size.y > 0.0
    {
        let fit_w = canvas_rect.width() / image_size.x;
        let fit_h = canvas_rect.height() / image_size.y;
        fit_w.min(fit_h).min(1.0).max(0.001)
    } else {
        0.001
    };

    // On first load, set zoom to fit and center.
    if !preview.zoom_initialized && fit_zoom > 0.001 {
        preview.zoom = fit_zoom;
        preview.pan = Vec2::ZERO;
        preview.zoom_initialized = true;
    }

    // Scroll-to-zoom, anchored at the cursor (the image pixel under the
    // pointer stays fixed). All math is self-contained in this frame, so
    // there's no one-frame feedback lag.
    if canvas_resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            let old_zoom = preview.zoom;
            let factor = (scroll / 240.0).exp();
            // Floor at fit_zoom so the texture can't be zoomed out smaller
            // than the size where it fully fits the canvas.
            let new_zoom = (old_zoom * factor).clamp(fit_zoom, 32.0);
            if (new_zoom - old_zoom).abs() > f32::EPSILON {
                if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                    // Image top-left in screen space at the current zoom.
                    let center = canvas_rect.center();
                    let img_tl = center + preview.pan - image_size * old_zoom * 0.5;
                    // Pixel coordinate under the cursor.
                    let img_px = (ptr - img_tl) / old_zoom;
                    // Solve for the pan that keeps img_px under the cursor.
                    let new_img_tl = ptr - img_px * new_zoom;
                    preview.pan = new_img_tl - center + image_size * new_zoom * 0.5;
                }
                preview.zoom = new_zoom;
            }
        }
    }

    // Drag to pan.
    if canvas_resp.dragged() {
        preview.pan += canvas_resp.drag_delta();
    }

    // Clamp the pan so the image always covers the canvas — you can't drag
    // into empty background past the image edge. When the image is smaller
    // than the canvas on an axis (e.g. at fit zoom), it stays centered there.
    let draw_size = image_size * preview.zoom;
    let half_extra_x = ((draw_size.x - canvas_rect.width()) * 0.5).max(0.0);
    let half_extra_y = ((draw_size.y - canvas_rect.height()) * 0.5).max(0.0);
    preview.pan.x = preview.pan.x.clamp(-half_extra_x, half_extra_x);
    preview.pan.y = preview.pan.y.clamp(-half_extra_y, half_extra_y);

    // Draw: dark background, then the image clipped to the canvas.
    let painter = ui.painter();
    painter.rect_filled(canvas_rect, 0.0, preview.bg.color());
    painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, grid_line()));

    let img_tl = canvas_rect.center() + preview.pan - draw_size * 0.5;
    let img_rect = egui::Rect::from_min_size(img_tl, draw_size);
    painter.with_clip_rect(canvas_rect).image(
        texture.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    // Under-cursor pixel coordinate + RGBA readout (samples the original
    // decoded pixels, independent of the channel-view toggles).
    if let Some(ptr) = canvas_resp.hover_pos() {
        let img_px = (ptr - img_tl) / preview.zoom;
        let (px, py) = (img_px.x.floor() as i64, img_px.y.floor() as i64);
        if px >= 0 && py >= 0 && (px as u32) < data.width && (py as u32) < data.height {
            let idx = (py as usize * data.width as usize + px as usize) * 4;
            if let Some(rgba) = data.rgba.get(idx..idx + 4) {
                let (r, g, b, a) = (rgba[0], rgba[1], rgba[2], rgba[3]);
                let text = format!("({px}, {py})  R{r} G{g} B{b} A{a}");
                let font = egui::FontId::monospace(12.0);
                let galley = painter.layout_no_wrap(text.clone(), font.clone(), text_dark());
                let pad = 5.0;
                let swatch = 12.0;
                let box_w = pad + swatch + 6.0 + galley.size().x + pad;
                let box_h = galley.size().y.max(swatch) + pad * 2.0;
                let box_min =
                    egui::pos2(canvas_rect.left() + 6.0, canvas_rect.bottom() - box_h - 6.0);
                let box_rect = egui::Rect::from_min_size(box_min, egui::vec2(box_w, box_h));
                painter.rect_filled(box_rect, 3.0, Color32::from_black_alpha(190));
                let swatch_rect = egui::Rect::from_min_size(
                    box_min + egui::vec2(pad, (box_h - swatch) * 0.5),
                    egui::vec2(swatch, swatch),
                );
                painter.rect_filled(swatch_rect, 2.0, Color32::from_rgb(r, g, b));
                painter.rect_stroke(swatch_rect, 2.0, Stroke::new(1.0, grid_line()));
                painter.text(
                    swatch_rect.right_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    text_dark(),
                );
            }
        }
    }

    // Apply a deferred image/mip change now that `data`'s borrow has ended.
    if redecode {
        preview.decoded = None;
        preview.texture_dirty = true;
    }
}

/// Field-aware diff of two same-group tags: walk both root structs in parallel
/// (same layout → same field order) and collect every differing leaf value plus
/// block element-count mismatches. Returns the diffs and whether the cap was hit.
pub(super) fn diff_tags(
    a: &TagFile,
    b: &TagFile,
    names: &TagNameIndex,
    limit: usize,
) -> (Vec<TagFieldDiff>, bool) {
    let mut out = Vec::new();
    diff_structs(&a.root(), &b.root(), "", names, &mut out, limit);
    let truncated = out.len() > limit;
    out.truncate(limit);
    (out, truncated)
}

fn diff_structs(
    a: &TagStruct<'_>,
    b: &TagStruct<'_>,
    path: &str,
    names: &TagNameIndex,
    out: &mut Vec<TagFieldDiff>,
    limit: usize,
) {
    for (fa, fb) in a.fields_all().zip(b.fields_all()) {
        if out.len() > limit {
            return;
        }
        let field_path = append_field_path(path, fa.name());
        if let (Some(ba), Some(bb)) = (fa.as_block(), fb.as_block()) {
            if ba.len() != bb.len() {
                out.push(TagFieldDiff {
                    path: field_path.clone(),
                    a: format!("{} element(s)", ba.len()),
                    b: format!("{} element(s)", bb.len()),
                });
            }
            for i in 0..ba.len().min(bb.len()) {
                if let (Some(ea), Some(eb)) = (ba.element(i), bb.element(i)) {
                    diff_structs(&ea, &eb, &format!("{field_path}[{i}]"), names, out, limit);
                }
            }
        } else if let (Some(aa), Some(ab)) = (fa.as_array(), fb.as_array()) {
            for i in 0..aa.len().min(ab.len()) {
                if let (Some(ea), Some(eb)) = (aa.element(i), ab.element(i)) {
                    diff_structs(&ea, &eb, &format!("{field_path}[{i}]"), names, out, limit);
                }
            }
        } else if let (Some(sa), Some(sb)) = (fa.as_struct(), fb.as_struct()) {
            diff_structs(&sa, &sb, &field_path, names, out, limit);
        } else if let (Some(va), Some(vb)) = (fa.value(), fb.value()) {
            let ta = foundation::format_foundation_scalar_value(names, &va);
            let tb = foundation::format_foundation_scalar_value(names, &vb);
            if ta != tb {
                out.push(TagFieldDiff {
                    path: field_path,
                    a: ta,
                    b: tb,
                });
            }
        }
    }
}

pub(super) fn build_bitmap_preview(
    tag: &TagFile,
    image_index: usize,
    mip_index: usize,
) -> anyhow::Result<BitmapPreviewData> {
    let bitmap = Bitmap::new(tag)?;
    if bitmap.is_empty() {
        anyhow::bail!("bitmap tag has no images");
    }
    let image_count = bitmap.len();
    let image_index = image_index.min(image_count - 1);
    let image = bitmap
        .image(image_index)
        .ok_or_else(|| anyhow::anyhow!("bitmap tag has no image {image_index}"))?;
    let format = image.format()?;
    let base_width = image.width();
    let base_height = image.height();
    if base_width == 0 || base_height == 0 {
        anyhow::bail!("bitmap image has empty dimensions");
    }
    let mip_count = (image.mipmap_levels() as usize).max(1);
    let mip = mip_index.min(mip_count - 1);

    // Walk the face-0 mip chain to this level: offset = Σ smaller-level bytes,
    // dims halve each step (floored at 1). Layout is `[face0_mips … faceN_mips]`,
    // so face 0's chain starts at offset 0.
    let mut offset = 0usize;
    let (mut width, mut height) = (base_width, base_height);
    for _ in 0..mip {
        offset += format.level_bytes(width, height) as usize;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    let mip_len = format.level_bytes(width, height) as usize;

    let pixel_bytes = image.pixel_bytes()?;
    if pixel_bytes.len() < offset + mip_len {
        anyhow::bail!(
            "bitmap image mip {mip} needs {} bytes at offset {offset} but only {} were available",
            mip_len,
            pixel_bytes.len()
        );
    }
    let rgba = decode_to_rgba8(
        format,
        width,
        height,
        &pixel_bytes[offset..offset + mip_len],
        bitmap.p8_palette(),
    )?;
    Ok(BitmapPreviewData {
        width,
        height,
        image_count,
        mip_count,
        format_name: image.format_name().unwrap_or_else(|| format!("{format:?}")),
        type_name: image.type_name().unwrap_or_else(|| "2D texture".to_owned()),
        rgba,
    })
}

pub(super) fn filtered_bitmap_rgba(
    data: &BitmapPreviewData,
    preview: &BitmapPreviewState,
) -> Vec<u8> {
    let alpha_only =
        !preview.show_red && !preview.show_green && !preview.show_blue && preview.show_alpha;
    let mut out = data.rgba.clone();
    for pixel in out.chunks_exact_mut(4) {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if alpha_only {
            pixel[0] = a;
            pixel[1] = a;
            pixel[2] = a;
            pixel[3] = 255;
        } else {
            pixel[0] = if preview.show_red { r } else { 0 };
            pixel[1] = if preview.show_green { g } else { 0 };
            pixel[2] = if preview.show_blue { b } else { 0 };
            pixel[3] = if preview.show_alpha { a } else { 255 };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .unwrap_or_else(|_| "/Users/camden/Halo/halo3_mcc/tags".to_owned());
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
                    decode_subsound(&data, sub.channels, sub.frequency, sub.setup_hash).unwrap();
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
            .unwrap_or_else(|_| "/Users/camden/Halo/halo4_mcc/tags".to_owned());
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

    /// H2 investigation (skip-if-absent): walk the classic `.sound` tag and dump
    /// every non-empty `data` field with its path + size + first bytes, to locate
    /// the inline audio and characterize its framing. Point at a tag with
    /// `SND_TAG` (rel path incl. extension), default an Opus one.
    /// `SND_TAG=sound/ui/pickup_health.sound cargo test -p baboon h2_dump_data -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn h2_dump_data_fields() {
        fn walk(
            st: &blam_tags::TagStruct<'_>,
            path: &str,
            out: &mut Vec<(String, usize, Vec<u8>)>,
            depth: usize,
        ) {
            if depth > 14 {
                return;
            }
            for field in st.fields() {
                let seg = {
                    let n = field.name();
                    if n.is_empty() {
                        "?".to_owned()
                    } else {
                        n.to_owned()
                    }
                };
                let p = format!("{path}/{seg}");
                if let Some(data) = field.as_data() {
                    if !data.is_empty() {
                        out.push((p.clone(), data.len(), data[..data.len().min(24)].to_vec()));
                    }
                } else if let Some(block) = field.as_block() {
                    for i in 0..block.len() {
                        if let Some(el) = block.element(i) {
                            walk(&el, &format!("{p}[{i}]"), out, depth + 1);
                        }
                    }
                } else if let Some(s) = field.as_struct() {
                    walk(&s, &p, out, depth + 1);
                } else if let Some(arr) = field.as_array() {
                    for (i, el) in arr.iter().enumerate() {
                        walk(&el, &format!("{p}<{i}>"), out, depth + 1);
                    }
                }
            }
        }

        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let mut out = Vec::new();
        walk(&tag.root(), "", &mut out, 0);
        eprintln!("=== {rel}: {} non-empty data field(s) ===", out.len());
        for (p, len, head) in &out {
            let hex: String = head
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = head
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            eprintln!("  {len:>8}B  {p}\n            {hex}  |{ascii}|");
        }
        // Re-walk to grab the full bytes of the largest data field and write it
        // out for framing analysis.
        let mut biggest: Option<Vec<u8>> = None;
        fn grab(st: &blam_tags::TagStruct<'_>, best: &mut Option<Vec<u8>>, depth: usize) {
            if depth > 14 {
                return;
            }
            for field in st.fields() {
                if let Some(data) = field.as_data() {
                    if best.as_ref().map_or(true, |b| data.len() > b.len()) {
                        *best = Some(data.to_vec());
                    }
                } else if let Some(block) = field.as_block() {
                    for i in 0..block.len() {
                        if let Some(el) = block.element(i) {
                            grab(&el, best, depth + 1);
                        }
                    }
                } else if let Some(s) = field.as_struct() {
                    grab(&s, best, depth + 1);
                } else if let Some(arr) = field.as_array() {
                    for el in arr.iter() {
                        grab(&el, best, depth + 1);
                    }
                }
            }
        }
        grab(&tag.root(), &mut biggest, 0);
        if let Some(bytes) = biggest {
            let stem = std::path::Path::new(&rel)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("h2");
            let out_path = format!("/tmp/h2_{stem}.bin");
            std::fs::write(&out_path, &bytes).unwrap();
            eprintln!("wrote {} ({} bytes)", out_path, bytes.len());
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
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/sound/sinomatixx_music/b40_extraction_music.sound",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
            .expect("read CE sound tag");
        let bytes = inline_permutation_samples(&tag, 0, 0).expect("inline samples present");
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

    /// End-to-end extraction (skip-if-absent): read a real CE `.sound`, build
    /// the same rows the player builds, and run the actual
    /// `AudioState::run_extract` for both WAV (decoded) and raw `.ogg`
    /// (passthrough), validating each output.
    #[test]
    #[ignore]
    fn ce_extract_writes_wav_and_raw_ogg() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let tag_path = std::path::Path::new(
            "/Users/camden/Halo/haloce_mcc/tags/sound/sinomatixx_music/b40_extraction_music.sound",
        );
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no CE tag/defs");
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(tag_path, Some("haloce_mcc"), Some(defs), group)
            .expect("read CE sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty(), "CE tag should have permutations");

        // Decoded WAV.
        let wav_dir = std::env::temp_dir().join("baboon_ce_extract_wav");
        let _ = std::fs::remove_dir_all(&wav_dir);
        let items = build_extract_items(&tag, &rows, None, &wav_dir, false);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "ce".to_owned(),
        });
        let wav = std::fs::read(wav_dir.join(format!("{}.wav", sanitize_component(&rows[0].name))))
            .expect("wav written");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(wav.len() > 44, "wav should carry samples");

        // Raw .ogg passthrough should be byte-identical to the inline samples.
        let ogg_dir = std::env::temp_dir().join("baboon_ce_extract_ogg");
        let _ = std::fs::remove_dir_all(&ogg_dir);
        let items = build_extract_items(&tag, &rows, None, &ogg_dir, true);
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "ce".to_owned(),
        });
        let ogg = std::fs::read(ogg_dir.join(format!("{}.ogg", sanitize_component(&rows[0].name))))
            .expect("ogg written");
        assert!(ogg.starts_with(b"OggS"), "raw passthrough should be an Ogg");
        let inline =
            inline_permutation_samples(&tag, rows[0].pr_index, rows[0].perm_index).unwrap();
        assert_eq!(ogg, inline, "raw passthrough must be verbatim tag bytes");

        let _ = std::fs::remove_dir_all(&wav_dir);
        let _ = std::fs::remove_dir_all(&ogg_dir);
    }

    /// Whole-tag H2 extraction end-to-end (skip-if-absent): build the same rows
    /// the player builds and run the real `AudioState::run_extract`, validating a
    /// decoded WAV lands for the inline (Opus/ADPCM/PCM) path. `SND_TAG` overrides.
    #[test]
    #[ignore]
    fn h2_extract_writes_wav() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty(), "H2 tag should have permutations");
        let h2_params = Some(h2_codec_params(&tag));
        let dir = std::env::temp_dir().join("baboon_h2_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let items = build_extract_items(&tag, &rows, h2_params, &dir, false);
        let count = items.len();
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: None,
            label: "h2".to_owned(),
        });
        // At least one WAV should have been written with a valid RIFF header.
        let mut found = 0usize;
        for entry in walkdir(&dir) {
            let bytes = std::fs::read(&entry).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WAVE");
            found += 1;
        }
        assert!(found > 0 && found <= count, "wrote {found}/{count} wavs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whole-tag H3/Reach bank extraction end-to-end (skip-if-absent): run the
    /// real `run_extract` Bank path (resolve subsound → decode → WAV) for every
    /// permutation. `SND_TAGS_ROOT`/`SND_TAG` override for Reach/ODST.
    #[test]
    #[ignore]
    fn bank_extract_writes_wav() {
        let root = std::env::var("SND_TAGS_ROOT")
            .unwrap_or_else(|_| "/Users/camden/Halo/halo3_mcc/tags".to_owned());
        let rel = std::env::var("SND_TAG")
            .unwrap_or_else(|_| "sound/visual_fx/ambient_vehicle_destroyed_large.sound".to_owned());
        let tags_root = std::path::Path::new(&root);
        let tag_path = tags_root.join(&rel);
        if !tag_path.exists() {
            eprintln!("skip: no bank tags at {}", tag_path.display());
            return;
        }
        let tag = blam_tags::TagFile::read(&tag_path).expect("read sound tag");
        let rows = sound_permutation_rows(&tag);
        assert!(!rows.is_empty());
        let dir = std::env::temp_dir().join("baboon_bank_extract");
        let _ = std::fs::remove_dir_all(&dir);
        let items = build_extract_items(&tag, &rows, None, &dir, false);
        let mut audio = super::audio::AudioState::default();
        audio.run_extract(ExtractRequest {
            items,
            tags_root: Some(tags_root.to_path_buf()),
            label: "bank".to_owned(),
        });
        let mut found = 0usize;
        for entry in walkdir(&dir) {
            let bytes = std::fs::read(&entry).unwrap();
            assert_eq!(&bytes[0..4], b"RIFF");
            found += 1;
        }
        assert!(found > 0, "wrote no bank WAVs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DIAGNOSTIC (skip-if-absent): dump the full structure of a real H2 sound
    /// tag — every block/element and each data field's size — plus h2_blobs and
    /// decoded durations, to find why long sounds show short. Point via SND_TAG.
    #[test]
    #[ignore]
    fn h2_dump_structure() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel = std::env::var("SND_TAG").unwrap_or_else(|_| "sound/loop.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read tag");
        let (codec, channels, rate) = h2_codec_params(&tag);
        eprintln!("codec={codec:?} channels={channels} rate={rate}");
        let root = tag.root();

        // Permutation names + count.
        if let Some(prs) = find_block_field(&root, "pitch range") {
            for p in 0..prs.len() {
                let pr = prs.element(p).unwrap();
                if let Some(perms) = find_block_field(&pr, "permutation") {
                    eprintln!("pitch range {p}: {} permutations", perms.len());
                    for pm in 0..perms.len() {
                        let perm = perms.element(pm).unwrap();
                        let name = find_full_field_name(&perm, "name")
                            .and_then(|f| perm.read_string_id(f))
                            .unwrap_or_default();
                        // dump any int/enum fields that look like chunk/sample counts
                        let fields: Vec<String> = perm
                            .field_names()
                            .filter(|n| {
                                let c = clean_field_name(n).to_ascii_lowercase();
                                c.contains("sample")
                                    || c.contains("count")
                                    || c.contains("chunk")
                                    || c.contains("first")
                                    || c.contains("index")
                            })
                            .map(|n| {
                                let v = perm
                                    .read_int_any(n)
                                    .map(|x| x.to_string())
                                    .unwrap_or_default();
                                format!("{}={v}", clean_field_name(n))
                            })
                            .collect();
                        eprintln!("  perm {pm} '{name}': {}", fields.join(" "));
                    }
                }
            }
        }

        // Walk extra-info → language perm info → raw info block, dumping data sizes
        // AND any nested blocks (chunked samples live in a nested block).
        for field in root.fields() {
            let Some(block) = field.as_block() else {
                continue;
            };
            for i in 0..block.len() {
                let el = block.element(i).unwrap();
                let Some(lpi) = find_block_field(&el, "language permutation info") else {
                    continue;
                };
                eprintln!(
                    "extra-info[{i}] '{}': lpi={}",
                    clean_field_name(field.name()),
                    lpi.len()
                );
                for j in 0..lpi.len() {
                    let lel = lpi.element(j).unwrap();
                    let Some(raw) = find_block_field(&lel, "raw info block") else {
                        continue;
                    };
                    eprintln!("  lpi[{j}]: raw info block={}", raw.len());
                    for k in 0..raw.len() {
                        let rel = raw.element(k).unwrap();
                        let mut parts = Vec::new();
                        for f in rel.fields() {
                            if let Some(d) = f.as_data() {
                                parts.push(format!(
                                    "data'{}'={}B",
                                    clean_field_name(f.name()),
                                    d.len()
                                ));
                            } else if let Some(b) = f.as_block() {
                                parts.push(format!(
                                    "block'{}'x{}",
                                    clean_field_name(f.name()),
                                    b.len()
                                ));
                                // dump nested block element int values (chunk offsets/sizes)
                                for m in 0..b.len().min(20) {
                                    if let Some(be) = b.element(m) {
                                        let ints: Vec<String> = be
                                            .field_names()
                                            .filter_map(|n| {
                                                be.read_int_any(n).map(|v| v.to_string())
                                            })
                                            .collect();
                                        let datas: Vec<String> = be
                                            .fields()
                                            .filter_map(|bf| {
                                                bf.as_data().map(|d| format!("data={}B", d.len()))
                                            })
                                            .collect();
                                        if !ints.is_empty() || !datas.is_empty() {
                                            parts.push(format!(
                                                "[{m}:{} {}]",
                                                ints.join(","),
                                                datas.join(",")
                                            ));
                                        }
                                    }
                                }
                            } else {
                                let c = clean_field_name(f.name()).to_ascii_lowercase();
                                if c.contains("sample")
                                    || c.contains("count")
                                    || c.contains("size")
                                    || c.contains("compression")
                                    || c.contains("index")
                                {
                                    if let Some(v) = rel.read_int_any(f.name()) {
                                        parts.push(format!("{}={v}", clean_field_name(f.name())));
                                    }
                                }
                            }
                        }
                        eprintln!("    raw[{k}]: {}", parts.join(" "));
                    }
                }
            }
        }

        let (count, _) = h2_blobs(&tag, Some(0));
        eprintln!("h2_blobs count={count}");
        for idx in 0..count {
            let (_, blob) = h2_blobs(&tag, Some(idx));
            let offs = h2_blob_chunk_offsets(&tag, idx);
            if let Some(b) = blob {
                let old = super::audio::decode_inline(codec, &b, channels, rate)
                    .map(|p| p.duration_secs())
                    .unwrap_or(0.0);
                match super::audio::decode_inline_chunked(codec, &b, &offs, channels, rate) {
                    Ok(pcm) => eprintln!(
                        "blob{idx}: {}B, {} chunks -> {:.2}s (was {:.2}s single-chunk)",
                        b.len(),
                        offs.len().max(1),
                        pcm.duration_secs(),
                        old
                    ),
                    Err(e) => eprintln!("blob{idx}: {}B decode err: {e}", b.len()),
                }
            }
        }

        // Verify: decode blob0's chunks independently by slicing at chunk offsets.
        let (_, blob0) = h2_blobs(&tag, Some(0));
        if let Some(b) = blob0 {
            let offs = [0usize, 19234, 38487, b.len()];
            let mut sum = 0.0f32;
            for w in offs.windows(2) {
                if w[1] <= b.len() && w[0] < w[1] {
                    match super::audio::decode_inline(codec, &b[w[0]..w[1]], channels, rate) {
                        Ok(p) => {
                            sum += p.duration_secs();
                            eprintln!(
                                "  chunk [{}..{}] {}B -> {} frames = {:.2}s",
                                w[0],
                                w[1],
                                w[1] - w[0],
                                p.frame_count(),
                                p.duration_secs()
                            );
                        }
                        Err(e) => eprintln!("  chunk [{}..{}] err: {e}", w[0], w[1]),
                    }
                }
            }
            eprintln!("  blob0 all chunks summed = {sum:.2}s");
        }
    }

    /// Regression (skip-if-absent): H2 splits inline audio into per-chunk streams
    /// (`sound_permutation_chunk_block`); the decoder must concatenate all chunks,
    /// not stop at the first. For any multi-chunk blob, the chunked decode must be
    /// longer than the single-stream decode. Default tag = the custom `loop.sound`.
    #[test]
    #[ignore]
    fn h2_chunked_decode_is_full_length() {
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel = std::env::var("SND_TAG").unwrap_or_else(|_| "sound/loop.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read tag");
        let (codec, channels, rate) = h2_codec_params(&tag);
        let (count, _) = h2_blobs(&tag, Some(0));
        let mut checked_multichunk = false;
        for idx in 0..count {
            let (_, blob) = h2_blobs(&tag, Some(idx));
            let offs = h2_blob_chunk_offsets(&tag, idx);
            let Some(bytes) = blob else { continue };
            if offs.len() <= 1 {
                continue; // single-chunk: chunked == single, nothing to prove
            }
            checked_multichunk = true;
            let single = super::audio::decode_inline(codec, &bytes, channels, rate)
                .map(|p| p.frame_count())
                .unwrap_or(0);
            let full = super::audio::decode_inline_chunked(codec, &bytes, &offs, channels, rate)
                .expect("chunked decode")
                .frame_count();
            assert!(
                full > single,
                "blob{idx}: chunked {full} !> single {single} ({} chunks)",
                offs.len()
            );
        }
        assert!(
            checked_multichunk,
            "expected at least one multi-chunk blob to test"
        );
    }

    /// Per-language bank plumbing (skip-if-absent): the FMOD languages are
    /// discovered from the `.fsb` names, and opening a specific language + the
    /// shared sfx bank still resolves an SFX permutation (language bank first,
    /// sfx fallback). Proves `open_pc_language` doesn't break default resolution.
    #[test]
    #[ignore]
    fn fmod_language_selection_resolves() {
        use blam_tags::audio::SoundBanks;
        let tags_root = std::path::Path::new("/Users/camden/Halo/halo3_mcc/tags");
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
        let rows = sound_permutation_rows(&tag);
        let resolved = rows
            .iter()
            .filter(|r| banks.resolve(&r.name).is_some())
            .count();
        assert!(resolved > 0, "no permutations resolved in french+sfx banks");
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

    /// Classic Halo 2 inline audio (skip-if-absent): read the tag, extract the
    /// first inline audio blob + codec params exactly as the player does, and
    /// decode via the matching codec (Opus or Xbox-ADPCM). Point at a tag with
    /// `SND_TAG`; default an Opus one.
    #[test]
    #[ignore]
    fn h2_inline_extracts_and_decodes() {
        use super::audio::InlineCodec;
        use blam_tags::audio::{decode_opus, decode_xbox_adpcm};
        let defs = std::path::Path::new("/Users/camden/Source/blam-tags/definitions");
        let rel =
            std::env::var("SND_TAG").unwrap_or_else(|_| "sound/ui/pickup_health.sound".to_owned());
        let tag_path = std::path::Path::new("/Users/camden/Halo/halo2_mcc/tags").join(&rel);
        if !tag_path.exists() || !defs.exists() {
            eprintln!("skip: no H2 tag/defs ({})", tag_path.display());
            return;
        }
        let group = u32::from_be_bytes(*b"snd!");
        let tag = crate::source::read_tag_at_path(&tag_path, Some("halo2_mcc"), Some(defs), group)
            .expect("read H2 sound tag");
        let (count, blob) = h2_blobs(&tag, Some(0));
        assert!(count > 0, "no H2 inline audio blobs found");
        let bytes = blob.expect("blob 0");
        let (codec, channels, sample_rate) = h2_codec_params(&tag);
        let (codec_name, pcm) = match codec {
            InlineCodec::Opus => ("opus", decode_opus(&bytes, channels)),
            InlineCodec::XboxAdpcm => (
                "xbox-adpcm",
                decode_xbox_adpcm(&bytes, channels, sample_rate),
            ),
            InlineCodec::Pcm { big_endian } => (
                "pcm",
                blam_tags::audio::decode_pcm(&bytes, channels, sample_rate, big_endian),
            ),
            InlineCodec::OggVorbis => unreachable!("H2 is opus/adpcm/pcm"),
        };
        let pcm = pcm.expect("decode H2 inline");
        eprintln!(
            "H2 {rel}: {count} blob(s) codec={codec_name} ch={channels} {sample_rate}Hz \
             -> {} frames ({:.2}s)",
            pcm.frame_count(),
            pcm.duration_secs()
        );
        assert!(pcm.frame_count() > 0);
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

    #[test]
    fn field_meta_separates_range_from_unit_and_suffix_shows_both() {
        // Range in the unit slot: unit is empty, range captured; suffix shows
        // the type (no unit) followed by the range.
        let m = field_display_meta("acceleration scale:[0,+inf]#marine 1.0, grunt 1.4");
        assert_eq!(m.label, "acceleration scale");
        assert_eq!(m.unit, None);
        assert_eq!(m.range.as_deref(), Some("[0,+inf]"));
        assert_eq!(m.help.as_deref(), Some("marine 1.0, grunt 1.4"));
        assert_eq!(field_suffix(&m, "real"), "real [0,+inf]");

        // Range bare in the name (no colon): pulled out of the label.
        let m = field_display_meta("max sounds per tag [1,16]#max sounds");
        assert_eq!(m.label, "max sounds per tag");
        assert_eq!(m.range.as_deref(), Some("[1,16]"));
        assert_eq!(field_suffix(&m, "long_integer"), "long integer [1,16]");

        // Real unit, no range: unit wins over the type, no range appended.
        let m = field_display_meta("preemption time:ms#replaces after this many ms");
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.range, None);
        assert_eq!(field_suffix(&m, "short_integer"), "ms");

        // Unit AND range together: unit first, then range.
        let m = field_display_meta("auto-exposure delay:[0.1-1]seconds#how long");
        assert_eq!(m.unit.as_deref(), Some("seconds"));
        assert_eq!(m.range.as_deref(), Some("[0.1-1]"));
        assert_eq!(field_suffix(&m, "real"), "seconds [0.1-1]");
    }

    #[test]
    fn new_tags_strip_doc_strings_and_explanations_on_write() {
        // The engine strips explanation fields + cleans field names when building
        // a layout from JSON, so a freshly-created tag's embedded blay matches
        // shipped tags — no `#help`/`:units` text, no explanation bodies.
        let tag = TagFile::new("definitions/haloreach_mcc/sound_classes.json").unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(
            !contains(b"attenuating"),
            "must not embed explanation/help text"
        );
        assert!(
            !contains(b"world units"),
            "must not embed `:units` annotations"
        );
        // And it must still round-trip cleanly.
        TagFile::read_from_bytes(&bytes).expect("stripped tag must parse");
    }

    #[test]
    fn block_to_tsv_exports_header_and_one_row_per_element() {
        let mut tag = TagFile::new("definitions/halo2_mcc/model.json").unwrap();
        let mut dirty = false;
        for name in ["alpha", "beta"] {
            apply_model_variant_ops(
                &mut tag,
                vec![ModelVariantOp::Create {
                    name: name.to_owned(),
                    regions: Vec::new(),
                }],
                &mut dirty,
            );
        }
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let tsv = super::block_to_tsv(&variants, &TagNameIndex::default());
        let lines: Vec<&str> = tsv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 element rows");
        assert!(
            lines[0].split('\t').any(|col| col == "name"),
            "header should include the `name` column"
        );
        assert!(tsv.contains("alpha") && tsv.contains("beta"));
    }

    #[test]
    fn model_variant_ops_create_update_and_drop_regions() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/model.json")).unwrap();
        let mut dirty = false;

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Create {
                name: "test".to_owned(),
                regions: vec![ModelVariantRegionChoice {
                    region_name: "body".to_owned(),
                    permutation_name: "default".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Created model variant 'test'"));
        assert!(dirty);
        assert_variant(&tag, 0, "test", "body", "default");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Update {
                variant_index: 0,
                regions: vec![ModelVariantRegionChoice {
                    region_name: "head".to_owned(),
                    permutation_name: "damaged".to_owned(),
                }],
            }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Updated model variant 0"));
        assert_variant(&tag, 0, "test", "head", "damaged");

        let status = apply_model_variant_ops(
            &mut tag,
            vec![ModelVariantOp::Drop { variant_index: 0 }],
            &mut dirty,
        );
        assert_eq!(status.as_deref(), Some("Deleted model variant 0"));
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(variants.len(), 0);
    }

    #[test]
    fn h2_render_model_marker_translation_and_rotation_are_editable() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
        tag.container = test_halo2_render_model_container();
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("marker groups").unwrap();
            let mut marker_groups = field.as_block_mut().unwrap();
            marker_groups.add_element();
        }
        {
            let mut root = tag.root_mut();
            let mut field = root.field_path_mut("marker groups[0]/markers").unwrap();
            let mut markers = field.as_block_mut().unwrap();
            markers.add_element();
        }

        let mut dirty = false;
        let status = apply_pending_edits(
            &mut tag,
            vec![
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/translation".to_owned(),
                    input: "-0.27, 0, 0.73".to_owned(),
                },
                PendingFieldEdit {
                    path: "marker groups[0]/markers[0]/rotation".to_owned(),
                    input: "-0.38, 0, -0.92, 0".to_owned(),
                },
            ],
            &mut dirty,
        );

        assert_eq!(
            status.as_deref(),
            Some("Edited marker groups[0]/markers[0]/rotation")
        );
        assert!(dirty);
        let root = tag.root();
        let translation = root
            .field_path("marker groups[0]/markers[0]/translation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealPoint3d(translation) = translation else {
            panic!("translation should be a real point 3d");
        };
        assert!((translation.x + 0.27).abs() < 0.0001);
        assert!((translation.y - 0.0).abs() < 0.0001);
        assert!((translation.z - 0.73).abs() < 0.0001);

        let rotation = root
            .field_path("marker groups[0]/markers[0]/rotation")
            .unwrap()
            .value()
            .unwrap();
        let TagFieldData::RealQuaternion(rotation) = rotation else {
            panic!("rotation should be a real quaternion");
        };
        assert!((rotation.i + 0.38).abs() < 0.0001);
        assert!((rotation.j - 0.0).abs() < 0.0001);
        assert!((rotation.k + 0.92).abs() < 0.0001);
        assert!((rotation.w - 0.0).abs() < 0.0001);
        assert_h2_render_model_write_atomic_verifies(&tag);
    }

    fn test_halo2_render_model_container() -> blam_tags::file::TagContainer {
        let mut header = vec![0; 64];
        header[36..40].copy_from_slice(b"edom");
        header[56..58].copy_from_slice(&0u16.to_le_bytes());
        header[60..64].copy_from_slice(b"!MLB");
        blam_tags::file::TagContainer::Classic {
            engine: blam_tags::classic::ClassicEngine::Halo2V4,
            header,
        }
    }

    fn assert_h2_render_model_write_atomic_verifies(tag: &TagFile) {
        let mut path = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "baboon_h2_render_model_marker_{}_{}.render_model",
            std::process::id(),
            stamp
        ));
        let _ = std::fs::remove_file(&path);
        tag.write_atomic(&path).unwrap_or_else(|error| {
            panic!(
                "write_atomic verification failed for {}: {error}",
                path.display()
            )
        });
        let _ = std::fs::remove_file(&path);
    }

    fn assert_variant(
        tag: &TagFile,
        variant_index: usize,
        variant_name: &str,
        region_name: &str,
        permutation_name: &str,
    ) {
        let variants = tag
            .root()
            .field("variants")
            .and_then(|field| field.as_block())
            .unwrap();
        let variant = variants.element(variant_index).unwrap();
        assert_eq!(
            variant.read_string_id("name").as_deref(),
            Some(variant_name)
        );
        let regions = variant
            .field("regions")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(regions.len(), 1);
        let region = regions.element(0).unwrap();
        assert_eq!(
            region.read_string_id("region name").as_deref(),
            Some(region_name)
        );
        let permutations = region
            .field("permutations")
            .and_then(|field| field.as_block())
            .unwrap();
        assert_eq!(permutations.len(), 1);
        let permutation = permutations.element(0).unwrap();
        assert_eq!(
            permutation.read_string_id("permutation name").as_deref(),
            Some(permutation_name)
        );
    }
}

#[cfg(test)]
mod tag_diff_tests {
    use super::*;

    #[test]
    fn diff_detects_value_and_block_count_changes() {
        let names = TagNameIndex::default();
        let a = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        let mut b = TagFile::new("definitions/halo3_mcc/sound_classes.json").unwrap();
        // Two freshly-created identical tags must report no differences.
        let (diffs, truncated) = diff_tags(&a, &b, &names, 5000);
        assert!(diffs.is_empty(), "identical tags should have no diffs");
        assert!(!truncated);
        // Adding a block element to one shows up as an element-count difference.
        add_block_element(&mut b, "sound classes").unwrap();
        let (diffs, _) = diff_tags(&a, &b, &names, 5000);
        assert!(
            diffs.iter().any(|d| d.path.contains("sound classes")),
            "block element-count difference should be reported"
        );
    }
}
