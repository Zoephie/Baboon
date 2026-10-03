//! The clip player: one sound out of a tag's several, picked from a dropdown,
//! played and scrubbed on a timeline.
//! It owns presentation and the clip selection; playback belongs to `audio`.

use super::*;

use crate::app::audio::{PlaybackView, SoundAction};

/// One clip the player can choose: a permutation, an event, a media file.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlayerClip {
    /// Names the clip to `audio`, so the player can tell whether it is the
    /// one loaded. Unique within the player.
    pub(super) id: String,
    pub(super) name: String,
    /// The heading it is listed under (a pitch range), if any.
    pub(super) group: Option<String>,
    /// Its length, when known before it is decoded.
    pub(super) duration: Option<f64>,
}

/// Height of the time ruler above the timeline's track.
const RULER_HEIGHT: f32 = 14.0;
/// Height of the track, where phase 3's waveform lanes go.
const TRACK_HEIGHT: f32 = 26.0;

/// Draw the player over `clips` and return the index of the selected one.
///
/// `play` builds the action that plays a clip; it is only called for a clip
/// being played, since building one can copy the clip's audio. The selection
/// is kept per tab, by clip id, so it survives the rows being rebuilt.
pub(super) fn draw_clip_player(
    ui: &mut Ui,
    edit: &mut FieldEditContext<'_>,
    id_salt: &str,
    clips: &[PlayerClip],
    languages: &[LanguageChoice],
    play: &mut dyn FnMut(usize) -> Option<SoundAction>,
) -> usize {
    let selection_id = clip_selection_id(id_salt, edit.tag_key);
    let stored = ui.data(|data| data.get_temp::<String>(selection_id));
    let mut selected = stored
        .and_then(|id| clips.iter().position(|clip| clip.id == id))
        .unwrap_or(0);
    let Some(clip) = clips.get(selected) else {
        return 0;
    };

    let playback = edit.sound_playback.clone();
    let loaded = playback
        .as_ref()
        .filter(|playback| playback.clip.as_deref() == Some(clip.id.as_str()));
    let playing = playback.as_ref().is_some_and(|playback| playback.playing);
    let mut choose: Option<usize> = None;

    // Clip choice: previous, the dropdown, next, random.
    ui.horizontal(|ui| {
        let many = clips.len() > 1;
        if ui
            .add_enabled(many, egui::Button::new("\u{25C0}"))
            .on_hover_text("Previous")
            .clicked()
        {
            choose = Some((selected + clips.len() - 1) % clips.len());
        }
        let width = (ui.available_width() - 90.0).clamp(160.0, 420.0);
        egui::ComboBox::from_id_salt(("clip_player_combo", id_salt))
            .width(width)
            .selected_text(clip_title(clip))
            .show_ui(ui, |ui| {
                let mut group: Option<&str> = None;
                for (index, each) in clips.iter().enumerate() {
                    if each.group.as_deref() != group {
                        group = each.group.as_deref();
                        if let Some(heading) = group {
                            ui.label(RichText::new(heading).strong().color(subtle_dark()));
                        }
                    }
                    let text = match each.duration {
                        Some(seconds) => format!("{}    {}", each.name, format_play_time(seconds)),
                        None => each.name.clone(),
                    };
                    if ui.selectable_label(index == selected, text).clicked() {
                        choose = Some(index);
                    }
                }
            });
        if ui
            .add_enabled(many, egui::Button::new("\u{25B6}"))
            .on_hover_text("Next")
            .clicked()
        {
            choose = Some((selected + 1) % clips.len());
        }
        if ui
            .add_enabled(many, egui::Button::new("\u{2928}"))
            .on_hover_text("Play a random one, as the game picks")
            .clicked()
        {
            let next = random_other(selected, clips.len());
            choose = Some(next);
            // Random always plays: it is how the game would sound.
            queue_play(edit, clips, next, play);
        }
    });

    // Changing clip while this tab's sound plays carries on with the new one.
    if let Some(index) = choose.filter(|index| *index != selected) {
        if playing && !queued_play_for(edit, clips, index) {
            queue_play(edit, clips, index, play);
        }
        selected = index;
        ui.data_mut(|data| data.insert_temp(selection_id, clips[selected].id.clone()));
    }
    let clip = &clips[selected];
    let loaded = loaded.filter(|playback| playback.clip.as_deref() == Some(clip.id.as_str()));

    draw_timeline(ui, edit, clips, selected, loaded, play);

    // Transport.
    ui.horizontal(|ui| {
        let (icon, hover) = if loaded.is_some_and(|playback| playback.playing) {
            ("\u{23F8}", "Pause (Space)")
        } else {
            ("\u{25B6}", "Play (Space)")
        };
        if ui.button(icon).on_hover_text(hover).clicked() {
            toggle_or_play(edit, clips, selected, loaded, play);
        }
        if ui
            .button("\u{25A0}")
            .on_hover_text("Stop and rewind (Enter)")
            .clicked()
        {
            edit.sound_play_request.push_back(SoundAction::Stop);
        }
        let mut looping = edit.sound_looping;
        if ui
            .toggle_value(&mut looping, "\u{27F2}")
            .on_hover_text("Loop")
            .changed()
        {
            edit.sound_play_request
                .push_back(SoundAction::SetLooping(looping));
        }
        let (position, duration) = match loaded {
            Some(playback) => (playback.position, Some(playback.duration)),
            None => (0.0, clip.duration),
        };
        ui.label(
            RichText::new(format!(
                "{} / {}",
                format_play_time(position),
                duration.map_or_else(|| "-:--.---".to_owned(), format_play_time)
            ))
            .monospace()
            .color(text_dark()),
        );
        ui.add_space(8.0);
        draw_sound_output_controls(ui, edit, languages);
    });
    draw_sound_errors(ui, edit);

    // Space plays or pauses and Enter stops, in the focused tab, when no text
    // field has the keyboard.
    if edit.sound_has_focus && !ui.ctx().wants_keyboard_input() {
        let (space, enter) = ui.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::Space),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        if space {
            toggle_or_play(edit, clips, selected, loaded, play);
        }
        if enter {
            edit.sound_play_request.push_back(SoundAction::Stop);
        }
    }
    selected
}

/// Where a tab's player keeps the id of its selected clip. Not tied to the
/// widget's place in the layout: the tab and the player are what it belongs to.
pub(in crate::app::editor) fn clip_selection_id(id_salt: &str, tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_selection", id_salt, tag_key))
}

/// `group ▸ name`, or the name alone.
fn clip_title(clip: &PlayerClip) -> String {
    match &clip.group {
        Some(group) => format!("{group} \u{25B8} {}", clip.name),
        None => clip.name.clone(),
    }
}

/// Pause or resume the selected clip if it is the one loaded, else play it.
fn toggle_or_play(
    edit: &mut FieldEditContext<'_>,
    clips: &[PlayerClip],
    selected: usize,
    loaded: Option<&PlaybackView>,
    play: &mut dyn FnMut(usize) -> Option<SoundAction>,
) {
    if loaded.is_some() {
        edit.sound_play_request.push_back(SoundAction::TogglePause);
    } else {
        queue_play(edit, clips, selected, play);
    }
}

fn queue_play(
    edit: &mut FieldEditContext<'_>,
    clips: &[PlayerClip],
    index: usize,
    play: &mut dyn FnMut(usize) -> Option<SoundAction>,
) {
    if let Some(action) = play(index) {
        edit.sound_play_request
            .play_clip(action, clips[index].id.clone());
    }
}

/// Whether a play for `index` was already queued this frame (by Random).
fn queued_play_for(edit: &FieldEditContext<'_>, clips: &[PlayerClip], index: usize) -> bool {
    edit.sound_play_request.queued_clip(&clips[index].id)
}

/// Another index than `current` out of `len`, at random.
fn random_other(current: usize, len: usize) -> usize {
    use std::hash::{BuildHasher, Hasher};
    if len < 2 {
        return current;
    }
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos()),
    );
    let step = 1 + (hasher.finish() % (len as u64 - 1)) as usize;
    (current + step) % len
}

/// The timeline: a ruler and a track with the playhead. A click jumps there
/// and a drag scrubs, the sound carrying on from wherever the playhead is
/// put; on a clip that is not loaded yet, a click plays it.
fn draw_timeline(
    ui: &mut Ui,
    edit: &mut FieldEditContext<'_>,
    clips: &[PlayerClip],
    selected: usize,
    loaded: Option<&PlaybackView>,
    play: &mut dyn FnMut(usize) -> Option<SoundAction>,
) {
    let duration = loaded
        .map(|playback| playback.duration)
        .or(clips[selected].duration)
        .unwrap_or(0.0);
    let position = loaded.map_or(0.0, |playback| playback.position);
    let size = Vec2::new(ui.available_width().max(120.0), RULER_HEIGHT + TRACK_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let track =
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + RULER_HEIGHT), rect.max);
    painter.rect(
        track,
        3.0,
        foundation_input(),
        egui::Stroke::new(1.0, foundation_input_edge()),
    );

    let x_of = |seconds: f64| {
        if duration > 0.0 {
            track.left() + (seconds / duration).clamp(0.0, 1.0) as f32 * track.width()
        } else {
            track.left()
        }
    };

    // Ruler: a tick every "nice" step that leaves room for its label.
    if duration > 0.0 {
        let step = ruler_step(duration, track.width());
        let mut tick = 0.0;
        while tick <= duration + 1e-9 {
            let x = x_of(tick);
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + RULER_HEIGHT - 4.0),
                    egui::pos2(x, rect.top() + RULER_HEIGHT),
                ],
                egui::Stroke::new(1.0, grid_line()),
            );
            painter.text(
                egui::pos2(x + 2.0, rect.top()),
                egui::Align2::LEFT_TOP,
                format_ruler_time(tick, step),
                egui::FontId::proportional(10.0),
                subtle_dark(),
            );
            tick += step;
        }
    }

    // Played part, then the playhead.
    let head = x_of(position);
    if loaded.is_some() && head > track.left() {
        painter.rect_filled(
            egui::Rect::from_min_max(track.min, egui::pos2(head, track.bottom())),
            3.0,
            foundation_blue().gamma_multiply(0.35),
        );
    }
    let head_color = foundation_jump_cyan();
    painter.line_segment(
        [
            egui::pos2(head, track.top()),
            egui::pos2(head, track.bottom()),
        ],
        egui::Stroke::new(2.0, head_color),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(head - 4.0, track.top() - 5.0),
            egui::pos2(head + 4.0, track.top() - 5.0),
            egui::pos2(head, track.top()),
        ],
        head_color,
        egui::Stroke::NONE,
    ));

    // Hover: where a click would land.
    if let Some(pointer) = response.hover_pos().filter(|_| duration > 0.0) {
        let x = pointer.x.clamp(track.left(), track.right());
        painter.line_segment(
            [egui::pos2(x, track.top()), egui::pos2(x, track.bottom())],
            egui::Stroke::new(1.0, text_dark().gamma_multiply(0.4)),
        );
    }

    let pressed_at = |pointer: egui::Pos2| {
        (((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64) * duration
    };
    // Seek when the button goes down and whenever the pointer moves while it
    // is held — not again on release, which would pull a playing sound back
    // by the time the click took.
    let (pressed, moved) = ui.input(|input| {
        (
            input.pointer.any_pressed(),
            input.pointer.delta() != Vec2::ZERO,
        )
    });
    if let Some(pointer) = response.interact_pointer_pos() {
        if loaded.is_some() {
            if response.is_pointer_button_down_on() && (pressed || moved) {
                edit.sound_play_request
                    .push_back(SoundAction::Seek(pressed_at(pointer)));
            }
        } else if response.clicked() {
            queue_play(edit, clips, selected, play);
        }
    }
    response.on_hover_text(if loaded.is_some() {
        "Click to jump, drag to scrub"
    } else {
        "Click to play"
    });
}

/// The ruler's tick spacing: the smallest of a set of round steps that
/// keeps ticks at least ~70 points apart.
pub(super) fn ruler_step(duration: f64, width: f32) -> f64 {
    const STEPS: [f64; 14] = [
        0.01, 0.02, 0.05, 0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0,
    ];
    let ticks = (width / 70.0).max(1.0) as f64;
    STEPS
        .into_iter()
        .find(|step| duration / step <= ticks)
        .unwrap_or(duration / ticks)
}

/// A ruler label: seconds with as many decimals as the step needs, or
/// `m:ss` past a minute.
fn format_ruler_time(seconds: f64, step: f64) -> String {
    if seconds >= 60.0 {
        return format!("{}:{:02}", (seconds / 60.0) as u64, (seconds % 60.0) as u64);
    }
    let decimals = if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    };
    format!("{seconds:.decimals$}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::audio::{SoundOwner, SoundRequest, SoundRequests};
    use std::collections::VecDeque;

    fn clips() -> Vec<PlayerClip> {
        ["a", "b", "c"]
            .into_iter()
            .map(|name| PlayerClip {
                id: format!("id-{name}"),
                name: name.to_owned(),
                group: None,
                duration: Some(2.0),
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
                timeline: egui::Rect::NOTHING,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let clips = clips();
            let mut sinks = EditSinks::default();
            let playback = self.playback.clone();
            let focused = self.focused;
            let queued = &mut self.queued;
            let timeline = &mut self.timeline;
            let output = self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut edit = FieldEditContext::read_only(&mut sinks, "test", "test");
                        edit.sound_play_request = SoundRequests::new(queued, Some(owner()));
                        edit.sound_playback = playback.clone();
                        edit.sound_has_focus = focused;
                        let top = ui.cursor().top();
                        draw_clip_player(ui, &mut edit, "test", &clips, &[], &mut |index| {
                            Some(SoundAction::PlayEvent {
                                event_name: clips[index].name.clone(),
                                label: clips[index].name.clone(),
                                tags_root: None,
                            })
                        });
                        // The timeline is the second row, under the clip row.
                        let _ = top;
                    });
                },
            );
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
            self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
            self.frame(vec![button(false)]);
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

        fn take(&mut self) -> Vec<(Option<String>, String)> {
            self.queued
                .drain(..)
                .map(|request| {
                    let action = match request.action {
                        SoundAction::PlayEvent { event_name, .. } => format!("play {event_name}"),
                        SoundAction::TogglePause => "toggle".to_owned(),
                        SoundAction::Stop => "stop".to_owned(),
                        SoundAction::Seek(seconds) => format!("seek {seconds:.1}"),
                        _ => "other".to_owned(),
                    };
                    (request.clip, action)
                })
                .collect()
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
        let random = h.find("\u{2928}", 0);
        h.click(random);
        let queued = h.take();
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert_ne!(
            queued[0].0.as_deref(),
            Some("id-a"),
            "random picked the current clip"
        );
    }

    #[test]
    fn the_timeline_seeks_on_a_click_and_scrubs_on_a_drag() {
        let mut h = Harness::new();
        h.playback = Some(loaded("id-a", true));
        h.frame(Vec::new());
        h.frame(Vec::new());
        let track = h.timeline;
        assert!(track.width() > 100.0, "timeline not found: {track:?}");
        let at =
            |fraction: f32| egui::pos2(track.left() + track.width() * fraction, track.center().y);
        h.click(at(0.5));
        assert_eq!(h.take(), [(None, "seek 1.0".to_owned())]);

        // A drag seeks on every frame it moves: the sound scrubs.
        let press = egui::Event::PointerButton {
            pos: at(0.25),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(vec![egui::Event::PointerMoved(at(0.25)), press]);
        h.playback = Some(PlaybackView {
            position: 0.5,
            ..loaded("id-a", true)
        });
        h.frame(vec![egui::Event::PointerMoved(at(0.75))]);
        let seeks: Vec<String> = h.take().into_iter().map(|(_, action)| action).collect();
        assert_eq!(seeks, ["seek 0.5", "seek 1.5"]);
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
}
