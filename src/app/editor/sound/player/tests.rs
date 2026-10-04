use super::*;
use crate::app::audio::{SoundOwner, SoundRequest, SoundRequests};
use std::collections::VecDeque;

fn clips_of(duration: Option<f64>) -> Vec<PlayerClip> {
    ["a", "b", "c"]
        .into_iter()
        .map(|name| PlayerClip {
            id: format!("id-{name}"),
            name: name.to_owned(),
            group: None,
            duration,
        })
        .collect()
}

fn owner() -> SoundOwner {
    SoundOwner {
        kit: crate::app::kit::KitId(1),
        key: "test".to_owned(),
    }
}

/// What the player draws and queues over a few frames.
struct Harness {
    ctx: egui::Context,
    playback: Option<PlaybackView>,
    focused: bool,
    queued: VecDeque<SoundRequest>,
    /// Painted text and where, from the last frame.
    texts: Vec<(String, egui::Rect)>,
    /// The waveform's bars from the last frame: each rect and its colour.
    bars: Vec<(egui::Rect, egui::Color32)>,
    /// Filled rects from the last frame, with their fill.
    fills: Vec<(egui::Rect, egui::Color32)>,
    /// Line segments painted in the last frame.
    segments: usize,
    /// The vertical ones among them, as their x and y range.
    verticals: Vec<(f32, f32, f32)>,
    /// The input clock: a 60th of a second a frame, so a hover tooltip
    /// waits its delay as it would for a real pointer.
    time: f64,
    /// The clips previews were asked for, in order.
    previewed: Vec<String>,
    /// The tab's preview, as the audio state would hand it over.
    preview: Option<Preview>,
    /// Playback speed, as the audio state would hand it over.
    speed: f32,
    /// Whether the clips' lengths are known before they decode (a
    /// sound's permutations) or not (a dialogue's referenced sounds).
    lengths_known: bool,
    timeline: egui::Rect,
}

impl Harness {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            playback: None,
            focused: true,
            queued: VecDeque::new(),
            texts: Vec::new(),
            bars: Vec::new(),
            fills: Vec::new(),
            segments: 0,
            verticals: Vec::new(),
            time: 0.0,
            previewed: Vec::new(),
            preview: None,
            speed: 1.0,
            lengths_known: true,
            timeline: egui::Rect::NOTHING,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let time = self.time;
        let clips = clips_of(self.lengths_known.then_some(2.0));
        let mut sinks = EditSinks::default();
        let playback = self.playback.clone();
        let preview = self.preview.clone();
        let speed = self.speed;
        let focused = self.focused;
        let queued = &mut self.queued;
        let timeline = &mut self.timeline;
        let output = crate::app::run_ui_test(
            &self.ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 400.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
                    edit.sound_play_request = SoundRequests::new(queued, Some(owner()));
                    edit.sound_playback = playback.clone();
                    edit.sound_preview = preview.clone();
                    edit.sound_speed = speed;
                    edit.sound_has_focus = focused;
                    let top = ui.cursor().top();
                    draw_clip_player(ui, &mut edit, "test", &clips, &[], &mut |index| {
                        Some(ClipPlay::Action(SoundAction::PlayEvent {
                            event_name: clips[index].name.clone(),
                            label: clips[index].name.clone(),
                            tags_root: None,
                        }))
                    });
                    // The timeline is the second row, under the clip row.
                    let _ = top;
                });
            },
        );
        self.bars = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) => Some(mesh),
                _ => None,
            })
            .flat_map(|mesh| {
                // `add_colored_rect` adds four vertices per rect.
                mesh.vertices
                    .chunks(4)
                    .map(|quad| {
                        let rect = egui::Rect::from_points(
                            &quad.iter().map(|v| v.pos).collect::<Vec<_>>(),
                        );
                        (rect, quad[0].color)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        self.verticals = output
            .shapes
            .iter()
            .filter_map(|clipped| match clipped.shape {
                egui::Shape::LineSegment { points: [a, b], .. } if a.x == b.x => {
                    Some((a.x, a.y.min(b.y), a.y.max(b.y)))
                }
                _ => None,
            })
            .collect();
        self.segments = output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::LineSegment { .. }))
            .count();
        self.fills = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) => Some((rect.rect, rect.fill)),
                _ => None,
            })
            .collect();
        self.texts = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect();
        // The track is the widest stroke-filled rect: find it through the
        // ruler's first label, which sits on its left edge.
        if let Some((_, zero)) = self
            .texts
            .iter()
            .find(|(text, _)| text == "0.0" || text == "0.00" || text == "0")
        {
            *timeline = egui::Rect::from_min_max(
                egui::pos2(zero.left() - 2.0, zero.top() + RULER_HEIGHT),
                egui::pos2(800.0 - 8.0, zero.top() + RULER_HEIGHT + TRACK_HEIGHT),
            );
        }
    }

    /// The `nth` (top to bottom, left to right) painted text equal to `text`.
    fn find(&self, text: &str, nth: usize) -> egui::Pos2 {
        let mut found: Vec<egui::Rect> = self
            .texts
            .iter()
            .filter(|(shown, _)| shown == text)
            .map(|(_, rect)| *rect)
            .collect();
        found.sort_by(|a, b| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        });
        found
            .get(nth)
            .unwrap_or_else(|| panic!("no {nth}th {text:?} in {:?}", self.texts))
            .center()
    }

    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // Slide onto the target over a few frames, as a real pointer
        // does: egui hit-tests a press against where the pointer was, and
        // counts a single jump as no movement at all — which shows a
        // widget's hover tooltip at once, and the press then lands on it.
        for offset in [6.0, 3.0, 0.0] {
            self.frame(vec![egui::Event::PointerMoved(
                pos - egui::vec2(offset, 0.0),
            )]);
        }
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
    }

    /// Press at `from`, move to `to` over a few frames, release there.
    fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        for step in 1..=4 {
            let pos = from + (to - from) * (step as f32 / 4.0);
            self.frame(vec![egui::Event::PointerMoved(pos)]);
        }
        self.frame(vec![button(to, false)]);
        self.frame(Vec::new());
    }

    /// A point `fraction` of the way along the track, on the lanes.
    fn lanes_at(&self, fraction: f32) -> egui::Pos2 {
        let track = self.timeline;
        egui::pos2(track.left() + track.width() * fraction, track.center().y)
    }

    /// A point `fraction` of the way along the ruler.
    fn ruler_at(&self, fraction: f32) -> egui::Pos2 {
        let track = self.timeline;
        egui::pos2(
            track.left() + track.width() * fraction,
            track.top() - RULER_HEIGHT / 2.0,
        )
    }

    /// Click into the value box at `at`, replace its text with `text`
    /// and press Enter.
    fn type_into(&mut self, at: egui::Pos2, text: &str) {
        self.click(at);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        self.frame(vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
        self.frame(vec![egui::Event::Text(text.to_owned())]);
        self.frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        self.frame(Vec::new());
    }

    fn key(&mut self, key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![event(true), event(false)]);
    }

    /// The plays and transport actions queued since the last call;
    /// previews are counted apart, by [`Self::previews`].
    fn take(&mut self) -> Vec<(Option<String>, String)> {
        let (previews, rest): (Vec<_>, Vec<_>) =
            self.queued.drain(..).partition(|request| request.preview);
        self.previewed
            .extend(previews.into_iter().filter_map(|request| request.clip));
        rest.into_iter()
            .map(|request| {
                let action = match request.action {
                    SoundAction::PlayEvent { event_name, .. } => format!("play {event_name}"),
                    SoundAction::TogglePause => "toggle".to_owned(),
                    SoundAction::Stop => "stop".to_owned(),
                    SoundAction::Seek(seconds) => format!("seek {seconds:.1}"),
                    SoundAction::SetRegion(Some((start, end))) => {
                        format!("region {start:.1}-{end:.1}")
                    }
                    SoundAction::SetRegion(None) => "region none".to_owned(),
                    SoundAction::SetSpeed(speed) => format!("speed {speed:.2}"),
                    SoundAction::SetVolume(volume) => format!("volume {volume:.2}"),
                    _ => "other".to_owned(),
                };
                (request.clip, action)
            })
            .collect()
    }
}

/// `channels` channels of `frames` frames at 1 kHz, channel `c` holding
/// `sample(c, frame)`.
fn sound(channels: u16, frames: usize, sample: impl Fn(usize, usize) -> i16) -> PlaybackView {
    PlaybackView {
        duration: frames as f64 / 1000.0,
        waveform: std::sync::Arc::new(crate::app::audio::Waveform::new(std::sync::Arc::new(
            blam_tags::audio::DecodedPcm {
                samples: (0..frames)
                    .flat_map(|f| (0..channels as usize).map(move |c| (f, c)))
                    .map(|(f, c)| sample(c, f))
                    .collect(),
                channels,
                sample_rate: 1000,
            },
        ))),
        ..loaded("id-a", false)
    }
}

fn loaded(clip: &str, playing: bool) -> PlaybackView {
    PlaybackView {
        label: clip.to_owned(),
        clip: Some(clip.to_owned()),
        position: 0.0,
        duration: 2.0,
        playing,
        looping: false,
        waveform: std::sync::Arc::new(crate::app::audio::Waveform::new(std::sync::Arc::new(
            blam_tags::audio::DecodedPcm {
                samples: vec![0; 2 * 1000],
                channels: 2,
                sample_rate: 1000,
            },
        ))),
    }
}

#[test]
fn play_plays_the_selected_clip_and_toggles_it_once_loaded() {
    let mut h = Harness::new();
    h.frame(Vec::new());
    h.frame(Vec::new());
    // "▶" is Next on the clip row, then Play on the transport row.
    let play = h.find("\u{25B6}", 1);
    h.click(play);
    assert_eq!(h.take(), [(Some("id-a".to_owned()), "play a".to_owned())]);

    h.playback = Some(loaded("id-a", true));
    h.frame(Vec::new());
    let pause = h.find("\u{23F8}", 0);
    h.click(pause);
    assert_eq!(h.take(), [(None, "toggle".to_owned())]);
}

#[test]
fn next_switches_playback_while_playing_and_only_selects_when_not() {
    let mut h = Harness::new();
    h.frame(Vec::new());
    h.frame(Vec::new());
    let next = h.find("\u{25B6}", 0);
    h.click(next);
    assert!(h.take().is_empty(), "nothing playing: Next only selects");
    h.frame(Vec::new());
    assert!(h.texts.iter().any(|(text, _)| text == "b"), "b is selected");

    h.playback = Some(loaded("id-b", true));
    h.frame(Vec::new());
    h.click(next);
    assert_eq!(h.take(), [(Some("id-c".to_owned()), "play c".to_owned())]);
}

#[test]
fn random_plays_another_clip() {
    let mut h = Harness::new();
    h.frame(Vec::new());
    h.frame(Vec::new());
    let random = h.find(RANDOM_ICON, 0);
    h.click(random);
    let queued = h.take();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_ne!(
        queued[0].0.as_deref(),
        Some("id-a"),
        "random picked the current clip"
    );
}

/// A click on the lanes seeks once; dragging the ruler scrubs, seeking
/// on every frame it moves.
#[test]
fn the_lanes_seek_on_a_click_and_the_ruler_scrubs() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", true));
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(
        h.timeline.width() > 100.0,
        "timeline not found: {:?}",
        h.timeline
    );
    h.click(h.lanes_at(0.5));
    assert_eq!(h.take(), [(None, "seek 1.0".to_owned())]);

    let press = egui::Event::PointerButton {
        pos: h.ruler_at(0.25),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    };
    h.frame(vec![egui::Event::PointerMoved(h.ruler_at(0.25)), press]);
    h.playback = Some(PlaybackView {
        position: 0.5,
        ..loaded("id-a", true)
    });
    let to = h.ruler_at(0.75);
    h.frame(vec![egui::Event::PointerMoved(to)]);
    let seeks: Vec<String> = h.take().into_iter().map(|(_, action)| action).collect();
    assert_eq!(seeks, ["seek 0.5", "seek 1.5"]);
}

/// A drag across the lanes selects a region of this clip, drawn and read
/// out; a drag too narrow to mean anything is a click.
#[test]
fn dragging_the_lanes_selects_a_region() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", false));
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    let queued = h.take();
    assert_eq!(
        queued.last(),
        Some(&(Some("id-a".to_owned()), "region 0.5-1.5".to_owned())),
        "{queued:?}"
    );
    assert!(
        h.texts
            .iter()
            .any(|(text, _)| text == "0:00.500 \u{2013} 0:01.500 (0:01.000)")
    );
    let band = foundation_blue().gamma_multiply(0.18);
    let drawn = h
        .fills
        .iter()
        .find(|(_, fill)| *fill == band)
        .map(|(rect, _)| *rect)
        .expect("no region band drawn");
    assert!(
        (drawn.left() - h.lanes_at(0.25).x).abs() < 2.0
            && (drawn.right() - h.lanes_at(0.75).x).abs() < 2.0,
        "the band does not cover the region: {drawn:?}"
    );

    // Grabbing an edge moves that edge.
    h.drag(h.lanes_at(0.75), h.lanes_at(0.5));
    assert_eq!(
        h.take().last().map(|(_, action)| action.as_str()),
        Some("region 0.5-1.0")
    );

    // A click outside clears it and seeks.
    h.click(h.lanes_at(0.9));
    assert_eq!(
        h.take(),
        [
            (Some("id-a".to_owned()), "region none".to_owned()),
            (None, "seek 1.8".to_owned()),
        ]
    );
}

/// Escape clears the region in the focused tab, and so does picking
/// another clip: a region belongs to the clip it was drawn on.
#[test]
fn escape_and_another_clip_clear_the_region() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", false));
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    h.take();
    h.key(egui::Key::Escape);
    assert_eq!(
        h.take(),
        [(Some("id-a".to_owned()), "region none".to_owned())]
    );

    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    h.take();
    let next = h.find("\u{25B6}", 0);
    h.click(next);
    assert_eq!(
        h.take(),
        [(Some("id-a".to_owned()), "region none".to_owned())]
    );
}

#[test]
fn space_toggles_only_in_the_focused_tab() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", true));
    h.frame(Vec::new());
    h.key(egui::Key::Space);
    assert_eq!(h.take(), [(None, "toggle".to_owned())]);

    h.focused = false;
    h.key(egui::Key::Space);
    assert!(h.take().is_empty(), "an unfocused tab took Space");

    h.focused = true;
    h.key(egui::Key::Enter);
    assert_eq!(h.take(), [(None, "stop".to_owned())]);
}

fn lane_labels(h: &Harness) -> Vec<String> {
    let mut labels: Vec<(f32, String)> = h
        .texts
        .iter()
        .filter(|(text, _)| ["M", "L", "R", "C", "LFE", "Ls", "Rs"].contains(&text.as_str()))
        .map(|(text, rect)| (rect.top(), text.clone()))
        .collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    labels.into_iter().map(|(_, text)| text).collect()
}

/// One lane per channel, labelled in WAVE order; mono is one unlabelled
/// lane, and 5.1 gets six shorter ones.
#[test]
fn the_track_has_a_lane_per_channel() {
    for (channels, labels) in [
        (1, vec![]),
        (2, vec!["L", "R"]),
        (6, vec!["L", "R", "C", "LFE", "Ls", "Rs"]),
    ] {
        let mut h = Harness::new();
        h.playback = Some(sound(channels, 2000, |_, _| 8000));
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert_eq!(lane_labels(&h), labels, "{channels} channel(s)");
        let tallest = h
            .bars
            .iter()
            .map(|(rect, _)| rect.bottom())
            .fold(0.0, f32::max);
        let shortest = h
            .bars
            .iter()
            .map(|(rect, _)| rect.top())
            .fold(f32::MAX, f32::min);
        let expected = if channels > 2 {
            16.0 * f32::from(channels)
        } else {
            64.0
        };
        assert!(
            tallest - shortest <= expected,
            "{channels}: bars span {}",
            tallest - shortest
        );
    }
}

/// Each lane draws its own channel: silence stays flat while a full-scale
/// channel fills its lane, and the bars left of the playhead are drawn
/// played.
#[test]
fn each_lane_draws_its_channel_and_the_played_part() {
    let mut h = Harness::new();
    // Left silent, right a full-scale square wave.
    let mut view = sound(2, 2000, |c, f| {
        if c == 0 {
            0
        } else if f % 2 == 0 {
            30000
        } else {
            -30000
        }
    });
    view.position = 1.0; // halfway through the 2 s
    h.playback = Some(view);
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(!h.bars.is_empty(), "no waveform drawn");
    let labels: Vec<&(String, egui::Rect)> = h
        .texts
        .iter()
        .filter(|(text, _)| text == "L" || text == "R")
        .collect();
    let lane_split = labels.iter().find(|(text, _)| text == "R").unwrap().1.top() - 1.0;
    let height = |upper: bool| {
        h.bars
            .iter()
            .filter(|(rect, _)| (rect.center().y < lane_split) == upper)
            .map(|(rect, _)| rect.height())
            .fold(0.0, f32::max)
    };
    assert!(
        height(true) < 2.0,
        "the silent left lane drew {}",
        height(true)
    );
    assert!(
        height(false) > 20.0,
        "the loud right lane drew only {}",
        height(false)
    );

    let middle =
        h.bars.iter().map(|(rect, _)| rect.center().x).sum::<f32>() / h.bars.len() as f32;
    let played = foundation_blue();
    let is_played =
        |color: &egui::Color32| *color == played || *color == played.gamma_multiply(0.55);
    let left_played = h
        .bars
        .iter()
        .filter(|(rect, _)| rect.right() < middle - 20.0)
        .all(|(_, color)| is_played(color));
    let right_unplayed = h
        .bars
        .iter()
        .filter(|(rect, _)| rect.left() > middle + 20.0)
        .all(|(_, color)| !is_played(color));
    assert!(left_played, "bars before the playhead are not drawn played");
    assert!(right_unplayed, "bars after the playhead are drawn played");
}

/// A glyph the fonts lack draws as an empty box; ⤨ did.
///
/// This compares what each glyph draws with what a character no font has
/// draws, rather than asking `Fonts::has_glyphs`: egui 0.36 answers that
/// by checking that the face owning the character is not the face of the
/// replacement box, so every glyph of the emoji font that also supplies
/// the box (◀ ▶ 🔀 🌐 🔊 among them) is reported missing though it draws.
#[test]
fn every_player_glyph_is_in_the_app_s_fonts() {
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::app::foundation_fonts());
    let _ = crate::app::run_ui_test(&ctx, Default::default(), |_| {});
    // Where in the font atlas each glyph of `text` comes from.
    let drawn = |text: &str| -> Vec<([u16; 2], [u16; 2])> {
        let galley = ctx.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            )
        });
        galley
            .rows
            .iter()
            .flat_map(|row| row.glyphs.iter())
            .map(|glyph| (glyph.uv_rect.min, glyph.uv_rect.max))
            .collect()
    };
    // U+0378 and U+0379 are unassigned, so no font has them: both draw
    // the replacement box, and the check below must call them missing.
    let replacement = drawn("\u{378}");
    assert_eq!(replacement.len(), 1);
    assert_eq!(drawn("\u{379}"), replacement);
    let is_missing = |glyph: &str| drawn(glyph).iter().any(|uv| *uv == replacement[0]);
    assert!(is_missing("\u{379}"), "the check sees a missing glyph");
    let missing: Vec<&str> = PLAYER_GLYPHS
        .into_iter()
        .filter(|glyph| is_missing(glyph))
        .collect();
    assert!(missing.is_empty(), "no glyph for {missing:?}");
}

#[test]
fn names_sort_the_way_people_count() {
    let mut names = vec![
        "10", "2", "1", "15", "11", "pain10", "pain2", "Pain3", "a", "01",
    ];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        names,
        [
            "01", "1", "2", "10", "11", "15", "a", "pain2", "Pain3", "pain10"
        ]
    );
}

/// The dropdown lists 1, 2 … 10, and Previous/Next step through that
/// order; groups keep the order the tag gives them.
#[test]
fn the_dropdown_and_next_follow_natural_order() {
    let clip = |name: &str, group: &str| PlayerClip {
        id: format!("{group}/{name}"),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        duration: None,
    };
    let clips = [
        clip("1", "b"),
        clip("10", "b"),
        clip("11", "b"),
        clip("2", "b"),
        clip("x", "a"),
    ];
    let names: Vec<&str> = display_order(&clips)
        .into_iter()
        .map(|index| clips[index].name.as_str())
        .collect();
    assert_eq!(names, ["1", "2", "10", "11", "x"]);
}

fn ready_preview(view: PlaybackView) -> Preview {
    Preview {
        clip: "id-a".to_owned(),
        state: PreviewState::Ready(view.waveform),
    }
}

/// An idle player asks for its selected clip's preview, and only until
/// one is on its way; a loaded clip needs none.
#[test]
fn an_idle_player_asks_for_its_selected_clip_s_preview_once() {
    let mut h = Harness::new();
    h.frame(Vec::new());
    h.take();
    assert_eq!(h.previewed, ["id-a"]);

    h.preview = Some(Preview {
        clip: "id-a".to_owned(),
        state: PreviewState::Pending,
    });
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.take();
    assert_eq!(h.previewed, ["id-a"], "asked again while one was pending");

    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", false));
    h.frame(Vec::new());
    h.take();
    assert!(h.previewed.is_empty(), "a loaded clip was previewed");
}

/// The preview's waveform shows before anything plays, unplayed; while
/// it decodes, and if it cannot, the track says so.
#[test]
fn the_preview_s_waveform_shows_before_playing() {
    let mut h = Harness::new();
    h.preview = Some(ready_preview(sound(2, 2000, |_, f| {
        if f % 2 == 0 { 20000 } else { -20000 }
    })));
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(!h.bars.is_empty(), "no waveform before playing");
    assert!(
        h.bars.iter().all(|(_, color)| *color != foundation_blue()
            && *color != foundation_blue().gamma_multiply(0.55)),
        "an unplayed preview is drawn as played"
    );

    h.preview = Some(Preview {
        clip: "id-a".to_owned(),
        state: PreviewState::Pending,
    });
    h.frame(Vec::new());
    assert!(
        h.texts
            .iter()
            .any(|(text, _)| text == "Loading waveform\u{2026}")
    );

    h.preview = Some(Preview {
        clip: "id-a".to_owned(),
        state: PreviewState::Failed("no source loaded".to_owned()),
    });
    h.frame(Vec::new());
    assert!(h.texts.iter().any(|(text, _)| text == "no source loaded"));
}

/// A previewed clip plays from where its timeline is clicked.
#[test]
fn clicking_a_previewed_timeline_plays_from_there() {
    let mut h = Harness::new();
    h.preview = Some(ready_preview(sound(1, 2000, |_, _| 1000)));
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.take();
    let track = h.timeline;
    let at = egui::pos2(track.left() + track.width() * 0.75, track.center().y);
    h.click(at);
    assert_eq!(
        h.take(),
        [
            (Some("id-a".to_owned()), "play a".to_owned()),
            (None, "seek 1.5".to_owned()),
        ]
    );
}

fn stored_view(h: &Harness) -> Option<View> {
    h.ctx
        .data(|data| data.get_temp::<(String, View)>(clip_view_id("test")))
        .map(|(_, view)| view)
}

#[test]
fn zooming_keeps_the_time_under_the_pointer() {
    let whole = View::fit(10.0);
    let zoomed = whole.zoomed(4.0, 5.0, 10.0, 0.01);
    assert!((zoomed.span - 2.5).abs() < 1e-9);
    // 5 s was halfway across, and still is.
    assert!(((5.0 - zoomed.start) / zoomed.span - 0.5).abs() < 1e-9);
    assert!(!zoomed.follow, "a zoom by hand stops following");
    // Near the end, the view stays inside the clip.
    let end = whole.zoomed(4.0, 9.9, 10.0, 0.01);
    assert!(end.start + end.span <= 10.0 + 1e-9);
    // Never narrower than the narrowest, never wider than the clip.
    assert!((whole.zoomed(1e9, 5.0, 10.0, 0.01).span - 0.01).abs() < 1e-9);
    assert!((zoomed.zoomed(1e-9, 5.0, 10.0, 0.01).span - 10.0).abs() < 1e-9);
}

fn wheel(delta: egui::Vec2, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel {
        phase: egui::TouchPhase::Move,
        unit: egui::MouseWheelUnit::Point,
        delta,
        modifiers,
    }
}

/// A pinch zooms around the pointer, and the ruler and clicks follow the
/// view; a plain wheel is left to the page; Shift + wheel pans; the
/// overview strip shows only while zoomed in.
#[test]
fn the_timeline_zooms_and_pans() {
    let mut h = Harness::new();
    h.playback = Some(sound(1, 2000, |_, f| (f % 100) as i16 * 100));
    h.frame(Vec::new());
    h.frame(Vec::new());
    let middle = h.lanes_at(0.5);

    // A plain wheel scrolls the page, not the view.
    h.frame(vec![
        egui::Event::PointerMoved(middle),
        wheel(egui::vec2(0.0, -40.0), egui::Modifiers::NONE),
    ]);
    h.frame(Vec::new());
    // Nothing kept is the whole clip.
    assert!(
        stored_view(&h).is_none(),
        "a plain wheel zoomed: {:?}",
        stored_view(&h)
    );

    // Pinch-zoom by 4 around the middle: 0.75 s – 1.25 s.
    h.frame(vec![
        egui::Event::PointerMoved(middle),
        egui::Event::Zoom(4.0),
    ]);
    h.frame(Vec::new());
    let view = stored_view(&h).expect("no view stored");
    assert!(
        (view.span - 0.5).abs() < 1e-6 && (view.start - 0.75).abs() < 1e-6,
        "{view:?}"
    );
    assert!(
        h.texts.iter().any(|(text, _)| text == "0.80"),
        "the ruler did not follow: {:?}",
        h.texts
    );

    // A click at the track's quarter point is a quarter of the way into the view.
    h.take();
    h.click(h.lanes_at(0.25));
    let seeks: Vec<String> = h.take().into_iter().map(|(_, action)| action).collect();
    assert_eq!(seeks, ["seek 0.9"]);

    // Shift + wheel pans later in the clip.
    for _ in 0..8 {
        h.frame(vec![
            egui::Event::PointerMoved(middle),
            wheel(egui::vec2(0.0, -40.0), egui::Modifiers::SHIFT),
        ]);
    }
    for _ in 0..30 {
        h.frame(Vec::new());
    }
    let panned = stored_view(&h).unwrap();
    assert!(
        panned.start != view.start,
        "Shift + wheel did not pan: {panned:?}"
    );
    assert!(
        (panned.span - view.span).abs() < 1e-9,
        "panning changed the zoom"
    );
}

/// The overview strip appears once zoomed in, and dragging its box
/// scrolls the view.
#[test]
fn the_overview_strip_scrolls_a_zoomed_view() {
    let mut h = Harness::new();
    h.playback = Some(sound(1, 2000, |_, f| (f % 100) as i16 * 100));
    h.frame(Vec::new());
    h.frame(Vec::new());
    let strip = |h: &Harness| {
        h.fills
            .iter()
            .find(|(rect, fill)| {
                rect.height() == OVERVIEW_HEIGHT && *fill == foundation_input()
            })
            .map(|(rect, _)| *rect)
    };
    assert!(strip(&h).is_none(), "an overview strip on an unzoomed view");
    h.ctx.data_mut(|data| {
        data.insert_temp(
            clip_view_id("test"),
            (
                "id-a".to_owned(),
                View {
                    start: 0.0,
                    span: 0.5,
                    follow: false,
                },
            ),
        )
    });
    h.frame(Vec::new());
    h.frame(Vec::new());
    let strip = strip(&h).expect("no overview strip while zoomed");
    // The box covers the first quarter; drag it right by a quarter.
    let from = egui::pos2(strip.left() + strip.width() * 0.125, strip.center().y);
    let to = egui::pos2(strip.left() + strip.width() * 0.375, strip.center().y);
    h.drag(from, to);
    let view = stored_view(&h).unwrap();
    assert!((view.start - 0.5).abs() < 0.05, "{view:?}");
}

/// While playing, the view pages along with the playhead; once panned by
/// hand it stays put.
#[test]
fn a_zoomed_view_follows_the_playhead_until_panned() {
    let mut h = Harness::new();
    let mut view = sound(1, 2000, |_, _| 1000);
    view.playing = true;
    view.position = 1.6;
    h.playback = Some(view.clone());
    h.ctx.data_mut(|data| {
        data.insert_temp(
            clip_view_id("test"),
            (
                "id-a".to_owned(),
                View {
                    start: 0.0,
                    span: 0.5,
                    follow: true,
                },
            ),
        )
    });
    h.frame(Vec::new());
    let followed = stored_view(&h).unwrap();
    assert!(
        followed.start <= 1.6 && 1.6 <= followed.start + followed.span,
        "{followed:?}"
    );

    h.ctx.data_mut(|data| {
        data.insert_temp(
            clip_view_id("test"),
            (
                "id-a".to_owned(),
                View {
                    start: 0.0,
                    span: 0.5,
                    follow: false,
                },
            ),
        )
    });
    h.frame(Vec::new());
    assert_eq!(
        stored_view(&h).unwrap().start,
        0.0,
        "a panned view followed"
    );
}

/// Closer than a sample a pixel, the lanes draw the samples as a line.
#[test]
fn a_close_zoom_draws_the_samples() {
    let mut h = Harness::new();
    h.playback = Some(sound(1, 2000, |_, f| if f % 2 == 0 { 8000 } else { -8000 }));
    h.ctx.data_mut(|data| {
        data.insert_temp(
            clip_view_id("test"),
            (
                "id-a".to_owned(),
                View {
                    start: 1.0,
                    span: 0.02,
                    follow: false,
                },
            ),
        )
    });
    h.frame(Vec::new());
    h.frame(Vec::new());
    // (The overview strip under the track still draws its own bars.)
    let track = h.timeline;
    assert!(
        !h.bars.iter().any(|(rect, _)| track.contains(rect.center())),
        "bars in the track at a close zoom"
    );
    assert!(h.segments >= 19, "only {} sample segments", h.segments);
}

/// Ctrl/Cmd + wheel zooms (egui turns it into a zoom, as it does a pinch).
#[test]
fn command_wheel_zooms() {
    let mut h = Harness::new();
    h.playback = Some(sound(1, 2000, |_, f| (f % 100) as i16 * 100));
    h.frame(Vec::new());
    h.frame(Vec::new());
    let middle = h.lanes_at(0.5);
    for _ in 0..3 {
        h.frame(vec![
            egui::Event::PointerMoved(middle),
            wheel(egui::vec2(0.0, 40.0), egui::Modifiers::COMMAND),
        ]);
    }
    let view = stored_view(&h).expect("no view stored");
    assert!(view.span < 2.0, "Ctrl/Cmd + wheel did not zoom: {view:?}");
}

/// Closing a tab forgets what its player kept; another tab's stays.
#[test]
fn closing_a_tab_forgets_its_player() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", false));
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    let next = h.find("\u{25B6}", 0);
    h.click(next);
    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    let kept = |h: &Harness| {
        h.ctx.data(|data| {
            (
                data.get_temp::<String>(clip_selection_id("test", "test"))
                    .is_some(),
                data.get_temp::<(String, f64, f64)>(clip_region_id("test"))
                    .is_some(),
            )
        })
    };
    assert_eq!(kept(&h), (true, true), "nothing was kept to forget");

    forget_closed_players(&h.ctx, |tag_key| tag_key == "test");
    assert_eq!(kept(&h), (true, true), "an open tab's player was forgotten");

    forget_closed_players(&h.ctx, |_| false);
    assert_eq!(kept(&h), (false, false), "a closed tab's player was kept");
}

/// Stop clears the region as well as rewinding, from the button or Enter.
#[test]
fn stop_clears_the_region() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", true));
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    h.take();
    let stop = h.find("\u{25A0}", 0);
    h.click(stop);
    assert_eq!(
        h.take(),
        [
            (Some("id-a".to_owned()), "region none".to_owned()),
            (None, "stop".to_owned()),
        ]
    );
    assert!(
        !h.texts.iter().any(|(text, _)| text.contains('\u{2013}')),
        "the region is still read out"
    );

    h.drag(h.lanes_at(0.25), h.lanes_at(0.75));
    h.take();
    h.key(egui::Key::Enter);
    assert_eq!(
        h.take(),
        [
            (Some("id-a".to_owned()), "region none".to_owned()),
            (None, "stop".to_owned()),
        ]
    );

    // With no region, Stop only stops.
    h.click(stop);
    assert_eq!(h.take(), [(None, "stop".to_owned())]);
}

/// The speed slider reads out a percentage, and a double-click puts it
/// back to 100%.
#[test]
fn the_speed_slider_shows_a_percentage_and_resets() {
    let mut h = Harness::new();
    h.speed = 2.5;
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(
        h.texts.iter().any(|(text, _)| text == SPEED_ICON),
        "no speed slider"
    );
    let value = h.find("250%", 0);
    h.take();
    // Double-click the slider's track, just left of its value.
    let track = value - egui::vec2(60.0, 0.0);
    for offset in [6.0, 3.0, 0.0] {
        h.frame(vec![egui::Event::PointerMoved(
            track - egui::vec2(offset, 0.0),
        )]);
    }
    let button = |pressed| egui::Event::PointerButton {
        pos: track,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.frame(vec![button(true)]);
    h.frame(vec![button(false)]);
    h.frame(vec![button(true)]);
    h.frame(vec![button(false)]);
    let speeds: Vec<String> = h
        .take()
        .into_iter()
        .map(|(_, action)| action)
        .filter(|action| action.starts_with("speed"))
        .collect();
    assert_eq!(
        speeds.last().map(String::as_str),
        Some("speed 1.00"),
        "{speeds:?}"
    );
}

/// Volume and speed are parted by a divider, so each slider's icon (drawn
/// after its value) reads as its own; without languages there is no
/// second one.
#[test]
fn a_divider_parts_volume_from_speed() {
    let mut h = Harness::new();
    h.frame(Vec::new());
    h.frame(Vec::new());
    let volume_icon = h.find("\u{1F50A}", 0);
    let speed_icon = h.find(SPEED_ICON, 0);
    let between: Vec<f32> = h
        .verticals
        .iter()
        .filter(|(x, top, bottom)| {
            *x > volume_icon.x
                && *x < speed_icon.x
                && *top <= volume_icon.y
                && *bottom >= volume_icon.y
        })
        .map(|(x, _, _)| *x)
        .collect();
    // The speed slider sits between them too; only its divider is a
    // full-height vertical line in that row.
    assert_eq!(
        between.len(),
        1,
        "dividers between volume and speed: {between:?}"
    );
    let after_speed = h
        .verticals
        .iter()
        .filter(|(x, top, bottom)| {
            *x > speed_icon.x && *top <= speed_icon.y && *bottom >= speed_icon.y
        })
        .count();
    assert_eq!(
        after_speed, 0,
        "a divider before a language picker that is not there"
    );
}

/// A clip whose length is unknown until its preview decodes (a dialogue's
/// referenced sound) opens showing all of it, not the narrowest zoom.
#[test]
fn a_clip_of_unknown_length_opens_unzoomed() {
    let mut h = Harness::new();
    h.lengths_known = false;
    h.preview = Some(Preview {
        clip: "id-a".to_owned(),
        state: PreviewState::Pending,
    });
    h.frame(Vec::new());
    h.frame(Vec::new());
    h.preview = Some(ready_preview(sound(1, 2000, |_, f| (f % 100) as i16 * 100)));
    h.frame(Vec::new());
    h.frame(Vec::new());
    assert!(
        stored_view(&h).is_none(),
        "a view was kept: {:?}",
        stored_view(&h)
    );
    assert!(
        h.texts.iter().any(|(text, _)| text == "2.0"),
        "the ruler does not reach the end: {:?}",
        h.texts.iter().map(|(text, _)| text).collect::<Vec<_>>()
    );
}

/// A percentage typed into the volume or speed box is taken as a
/// percentage, may go past the slider's end, and Enter confirming it
/// does not reach the player's Enter (stop).
#[test]
fn typed_percentages_stand_and_enter_does_not_stop() {
    let mut h = Harness::new();
    h.playback = Some(loaded("id-a", true));
    h.frame(Vec::new());
    h.frame(Vec::new());
    let volume = h.find("100%", 0);
    let speed = h.find("100%", 1);
    h.take();

    h.type_into(volume, "50");
    let queued = h.take();
    assert_eq!(
        queued.last().map(|(_, action)| action.as_str()),
        Some("volume 0.50"),
        "{queued:?}"
    );
    assert!(
        !queued.iter().any(|(_, action)| action == "stop"),
        "Enter stopped playback: {queued:?}"
    );

    h.type_into(volume, "250");
    let queued = h.take();
    assert_eq!(
        queued.last().map(|(_, action)| action.as_str()),
        Some("volume 2.50"),
        "{queued:?}"
    );

    h.type_into(speed, "750%");
    let queued = h.take();
    assert_eq!(
        queued.last().map(|(_, action)| action.as_str()),
        Some("speed 7.50"),
        "{queued:?}"
    );
    assert!(
        !queued.iter().any(|(_, action)| action == "stop"),
        "Enter stopped playback: {queued:?}"
    );
}

/// Dragging the speed slider stays within its range, whatever was typed.
#[test]
fn dragging_the_speed_slider_stops_at_its_end() {
    let mut h = Harness::new();
    h.speed = 7.5;
    h.frame(Vec::new());
    h.frame(Vec::new());
    let value = h.find("750%", 0);
    h.take();
    // From the slider's track, left of the value box, far past its end.
    let track = value - egui::vec2(60.0, 0.0);
    h.drag(
        track - egui::vec2(20.0, 0.0),
        track + egui::vec2(300.0, 0.0),
    );
    let speeds: Vec<f32> = h
        .take()
        .into_iter()
        .filter_map(|(_, action)| action.strip_prefix("speed ").map(|v| v.parse().unwrap()))
        .collect();
    assert!(!speeds.is_empty(), "the drag set no speed");
    assert!(speeds.iter().all(|speed| *speed <= 5.0), "{speeds:?}");
    assert_eq!(speeds.last(), Some(&5.0));
}
