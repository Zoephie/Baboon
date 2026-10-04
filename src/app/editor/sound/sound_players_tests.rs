//! The reference players — a looping sound's components, a dialogue tag's
//! vocalizations — play through the clip player: one selection, one
//! playhead, and plays stamped with the clip they are for.
//!
//! Kit-gated: set `BLAM_TEST_H3EK` to the Halo 3 kit's `tags` folder.

use super::*;
use crate::app::audio::{SoundOwner, SoundRequest, SoundRequests};
use std::collections::VecDeque;

/// The plays among queued requests. The language selector also commits
/// the language it shows (`SetLanguage`) whenever it is drawn, and an idle
/// player asks for its waveform preview; neither is what these tests are
/// about.
fn plays(queued: &VecDeque<SoundRequest>) -> Vec<&SoundRequest> {
    queued
        .iter()
        .filter(|request| {
            !request.preview
                && !matches!(
                    request.action,
                    crate::app::audio::SoundAction::SetLanguage(_)
                )
        })
        .collect()
}

fn h3_tag(rel: &str) -> Option<(std::path::PathBuf, TagFile)> {
    let root = crate::test_kits::h3ek_tags();
    let path = root.join(rel);
    if !path.is_file() {
        eprintln!("skipping: {rel} not present under {}", root.display());
        return None;
    }
    Some((root, TagFile::read(&path).expect("read H3 tag")))
}

/// Draw `draw` for a few frames, clicking each of `clicks` (the `nth` painted
/// text starting with `text`, top to bottom) in turn; what it painted and
/// what it queued.
fn run(
    root: &std::path::Path,
    clicks: &[(&str, usize)],
    draw: &dyn Fn(&mut Ui, &mut FieldEditContext<'_>),
) -> (Vec<String>, VecDeque<SoundRequest>, egui::Context) {
    let ctx = egui::Context::default();
    let mut queued = VecDeque::new();
    let frame = |events: Vec<egui::Event>, queued: &mut VecDeque<SoundRequest>| {
        let output = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut sinks = EditSinks::default();
                    let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
                    edit.game = Some(GameId::Halo3);
                    edit.tags_root = Some(root);
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
            .unwrap_or_else(|| panic!("no {nth}th {text:?}"))
            .center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            vec![egui::Event::PointerMoved(pos), button(true)],
            &mut queued,
        );
        frame(vec![button(false)], &mut queued);
        texts = frame(Vec::new(), &mut queued);
    }
    (
        texts.into_iter().map(|(text, _)| text).collect(),
        queued,
        ctx,
    )
}

/// A looping sound lists its component sounds under their tracks, and Play
/// plays the selected one — its first permutation, from the FMOD banks —
/// stamped with its clip.
#[test]
fn a_looping_sound_plays_its_components_through_the_clip_player() {
    let rel = "sound/game_sfx/ui/main_menu_music/main_menu_music.sound_looping";
    let Some((root, tag)) = h3_tag(rel) else {
        return;
    };
    let draw =
        |ui: &mut Ui, edit: &mut FieldEditContext<'_>| draw_sound_looping_player(ui, &tag, edit);
    let (painted, queued, _) = run(&root, &[], &draw);
    assert!(
        painted
            .iter()
            .any(|text| text == "delta_menu \u{25B8} in \u{00B7} in"),
        "the first component is not selected under its track: {painted:?}"
    );
    assert!(plays(&queued).is_empty(), "drawing alone played something");

    // "▶" is Next on the clip row, then Play on the transport.
    let (_, queued, _) = run(&root, &[("\u{25B6}", 1)], &draw);
    let request = *plays(&queued).first().expect("Play queued nothing");
    assert!(
        request
            .clip
            .as_deref()
            .is_some_and(|clip| clip.starts_with("ref:0:")),
        "{:?}",
        request.clip
    );
    assert!(
        matches!(request.action, crate::app::audio::SoundAction::Play { .. }),
        "an H3 component plays from the banks"
    );
}

/// The dialogue overview's ▶ selects that sound in the player and plays it,
/// under the same clip the player's dropdown has for it.
#[test]
fn a_dialogue_row_plays_through_the_player() {
    let Some((root, tag)) = h3_tag("sound/dialog/combat/arbiter.dialogue") else {
        return;
    };
    let draw = |ui: &mut Ui, edit: &mut FieldEditContext<'_>| draw_dialogue_summary(ui, &tag, edit);
    // Open the table, then: ▶ number 0 is Next and 1 is Play; 2 is the
    // first row's.
    let (painted, queued, ctx) = run(&root, &[("Vocalizations (", 0), ("\u{25B6}", 2)], &draw);
    assert!(
        painted
            .iter()
            .any(|text| text.starts_with("Vocalizations (")),
        "the player is not shown above a closed table: {painted:?}"
    );
    let request = *plays(&queued).first().expect("the row's ▶ queued nothing");
    let clip = request.clip.clone().expect("the row's play names no clip");
    assert!(clip.starts_with("ref:0:0:"), "{clip}");
    let selected = ctx.data(|data| data.get_temp::<String>(clip_selection_id("dialogue", "test")));
    assert_eq!(
        selected.as_deref(),
        Some(clip.as_str()),
        "the player did not select the row's sound"
    );
}

/// On a Campaign Evolved mount a referenced sound resolves after the frame, so
/// its play is a reference request — and it carries the clip along.
#[test]
fn a_campaign_evolved_reference_carries_its_clip() {
    let path = "sound\\dialog\\x\\line";
    let play = referenced_clip_play(
        u32::from_be_bytes(*b"snd!"),
        path,
        None,
        None,
        None,
        None,
        true,
    );
    assert!(matches!(play, Some(ClipPlay::CeRef(_))));

    let mut sinks = EditSinks::default();
    let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
    let clips = [referenced_clip(
        "ref:0:x".to_owned(),
        None,
        "line".to_owned(),
    )];
    play_clip_now(
        &egui::Context::default(),
        &mut edit,
        "dialogue",
        &clips,
        0,
        &mut |_| {
            referenced_clip_play(
                u32::from_be_bytes(*b"snd!"),
                path,
                None,
                None,
                None,
                None,
                true,
            )
        },
    );
    let request = edit
        .ce_sound_ref_request
        .as_ref()
        .expect("no reference request");
    assert_eq!(request.clip.as_deref(), Some("ref:0:x"));
    assert_eq!(request.reference, path);
    assert!(!request.extract);
}

/// Opening a looping sound previews its selected component: the request the
/// player makes resolves through the real FMOD banks to a decoded waveform,
/// with nothing played.
#[test]
fn opening_a_sound_previews_its_waveform_from_the_banks() {
    let rel = "sound/game_sfx/ui/main_menu_music/main_menu_music.sound_looping";
    let Some((root, tag)) = h3_tag(rel) else {
        return;
    };
    let draw =
        |ui: &mut Ui, edit: &mut FieldEditContext<'_>| draw_sound_looping_player(ui, &tag, edit);
    let (_, queued, _) = run(&root, &[], &draw);
    let mut audio = crate::app::audio::AudioState::default();
    let previews: Vec<SoundRequest> = queued
        .into_iter()
        .filter(|request| request.preview)
        .collect();
    assert!(!previews.is_empty(), "opening the player previewed nothing");
    let owner = previews[0].owner.clone().unwrap();
    audio.pending.extend(previews);
    let ctx = egui::Context::default();
    while !audio.pending.is_empty() {
        audio.process(Some(&root), &ctx);
    }
    audio.wait_for_audio_jobs();
    match &audio.preview_for(&owner).expect("no preview").state {
        crate::app::audio::PreviewState::Ready(waveform) => {
            assert!(waveform.frames() > 1000, "{} frames", waveform.frames());
        }
        crate::app::audio::PreviewState::Failed(reason) => panic!("preview failed: {reason}"),
        crate::app::audio::PreviewState::Pending => panic!("preview never landed"),
    }
    assert!(
        audio.playback(Some(&owner)).is_none(),
        "previewing played the sound"
    );
}
