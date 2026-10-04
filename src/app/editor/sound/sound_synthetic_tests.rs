//! The sound player and its row model over synthetic tags.
//!
//! Characterization, with no kit: a Halo CE sound carries its audio inline,
//! so a tag built from the definitions with a few permutations of 16-bit PCM
//! is a complete sound — the rows, the play actions, the extraction layout
//! and the player all work from it alone.

use super::*;
use crate::app::audio::{InlineCodec, SoundAction, SoundOwner, SoundRequest, SoundRequests};
use blam_tags::TagFieldData;
use std::collections::VecDeque;

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
                            kit: crate::app::kit::KitId(1),
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
    crate::app::apply_field_edit(tag, path, input)
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
