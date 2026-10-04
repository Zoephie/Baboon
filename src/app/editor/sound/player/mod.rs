//! The clip player: one sound out of a tag's several, picked from a dropdown,
//! played and scrubbed on a timeline.
//! It owns presentation and the clip selection; playback belongs to `audio`.

use super::*;

use std::collections::HashSet;

use crate::app::audio::{PlaybackView, Preview, PreviewState, SoundAction};

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

/// How a clip is played: an action for the audio queue, or — for a sound a
/// Campaign Evolved tag references — a request the app resolves to one after
/// the frame.
pub(super) enum ClipPlay {
    Action(SoundAction),
    CeRef(CeSoundRefRequest),
}

/// The random-pick button. A shuffle sign: `⤨`, the obvious choice, is in
/// none of the app's fonts and drew as an empty box.
const RANDOM_ICON: &str = "\u{1F500}";

/// The playback speed slider's label.
pub(super) const SPEED_ICON: &str = "\u{23E9}";

/// The zoom-out button: a minus sign, matching the plus beside it.
const ZOOM_OUT_ICON: &str = "\u{2212}";

/// Every glyph the player draws, which the tests check the app's fonts have.
#[cfg(test)]
const PLAYER_GLYPHS: [&str; 12] = [
    SPEED_ICON,
    ZOOM_OUT_ICON,
    "\u{25C0}",
    "\u{25B6}",
    RANDOM_ICON,
    "\u{23F8}",
    "\u{25A0}",
    "\u{27F2}",
    "\u{2B07}",
    "\u{25B8}",
    "\u{1F310}",
    "\u{1F50A}",
];

/// Height of the time ruler above the timeline's track.
const RULER_HEIGHT: f32 = 14.0;
/// Height of the track for mono and stereo, and before a clip is loaded.
const TRACK_HEIGHT: f32 = 64.0;
/// Height of the overview strip shown under the lanes while zoomed in.
const OVERVIEW_HEIGHT: f32 = 14.0;
/// Height of each lane past two channels.
const SURROUND_LANE_HEIGHT: f32 = 16.0;

/// The track's height for a sound of `channels` channels.
fn track_height(channels: Option<u16>) -> f32 {
    match channels {
        Some(channels) if channels > 2 => SURROUND_LANE_HEIGHT * f32::from(channels),
        _ => TRACK_HEIGHT,
    }
}

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
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
) -> usize {
    register_player(ui.ctx(), id_salt, edit.tag_key);
    // Whether a text field had the keyboard as the frame began. Asked after
    // the transport row is drawn, the answer misses an Enter that confirmed a
    // typed volume or speed: the box gives the keyboard up while handling it.
    let keyboard_busy = ui.ctx().egui_wants_keyboard_input();
    let selection_id = clip_selection_id(id_salt, edit.tag_key);
    let stored = ui.data(|data| data.get_temp::<String>(selection_id));
    // Listed, stepped through and defaulted in display order: groups as the
    // tag has them, names numerically within each (1, 2 … 10, not 1, 10, 2).
    let order = display_order(clips);
    let mut selected = stored
        .and_then(|id| clips.iter().position(|clip| clip.id == id))
        .or_else(|| order.first().copied())
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
    let rank = order
        .iter()
        .position(|&index| index == selected)
        .unwrap_or(0);
    ui.horizontal(|ui| {
        let many = clips.len() > 1;
        if ui
            .add_enabled(many, egui::Button::new("\u{25C0}"))
            .on_hover_text("Previous")
            .clicked()
        {
            choose = Some(order[(rank + order.len() - 1) % order.len()]);
        }
        let width = (ui.available_width() - 90.0).clamp(160.0, 420.0);
        egui::ComboBox::from_id_salt(("clip_player_combo", id_salt))
            .width(width)
            .selected_text(clip_title(clip))
            .show_ui(ui, |ui| {
                let mut group: Option<&str> = None;
                for &index in &order {
                    let each = &clips[index];
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
            choose = Some(order[(rank + 1) % order.len()]);
        }
        if ui
            .add_enabled(many, egui::Button::new(RANDOM_ICON))
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
        // A region belongs to the clip it was drawn on.
        if stored_region(ui, edit.tag_key, &clips[selected].id).is_some() {
            ui.data_mut(|data| data.remove::<(String, f64, f64)>(clip_region_id(edit.tag_key)));
            edit.sound_play_request
                .push_for_clip(SoundAction::SetRegion(None), clips[selected].id.clone());
        }
        if playing && !queued_play_for(edit, clips, index) {
            queue_play(edit, clips, index, play);
        }
        selected = index;
        ui.data_mut(|data| data.insert_temp(selection_id, clips[selected].id.clone()));
    }
    let clip = &clips[selected];
    let loaded = loaded.filter(|playback| playback.clip.as_deref() == Some(clip.id.as_str()));

    // Decode the selected clip for its waveform before anything plays.
    let preview = edit
        .sound_preview
        .clone()
        .filter(|preview| preview.clip == clip.id);
    if loaded.is_none() && preview.is_none() {
        request_preview(ui, edit, clips, selected, play);
    }
    let preview_length = match preview.as_ref().map(|preview| &preview.state) {
        Some(PreviewState::Ready(waveform)) => Some(waveform.duration_secs()),
        _ => None,
    };

    draw_timeline(ui, edit, clips, selected, loaded, preview.as_ref(), play);

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
            .on_hover_text("Stop, rewind and clear the region (Enter)")
            .clicked()
        {
            stop(ui, edit, &clip.id);
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
            None => (0.0, clip.duration.or(preview_length)),
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
        // Zoom: around the playhead when the clip is loaded, else the middle
        // of what is shown.
        let waveform = loaded.map(|playback| playback.waveform.clone()).or(
            match preview.as_ref().map(|preview| &preview.state) {
                Some(PreviewState::Ready(waveform)) => Some(waveform.clone()),
                _ => None,
            },
        );
        if let (Some(waveform), Some(duration)) = (waveform, duration) {
            let width = ui.available_width().max(1.0);
            let min_span = min_view_span(Some(waveform.pcm().sample_rate), width);
            let view = load_view(ui, edit.tag_key, &clip.id, duration, min_span);
            let around = loaded.map_or(view.start + view.span / 2.0, |playback| playback.position);
            let mut zoomed = None;
            if ui.button(ZOOM_OUT_ICON).on_hover_text("Zoom out").clicked() {
                zoomed = Some(view.zoomed(0.5, around, duration, min_span));
            }
            if ui
                .button("+")
                .on_hover_text("Zoom in (Ctrl/Cmd + wheel)")
                .clicked()
            {
                zoomed = Some(view.zoomed(2.0, around, duration, min_span));
            }
            if ui
                .add_enabled(!view.whole(duration), egui::Button::new("Fit"))
                .on_hover_text("Show the whole sound")
                .clicked()
            {
                zoomed = Some(View::fit(duration));
            }
            if let Some(view) = zoomed {
                store_view(ui, edit.tag_key, &clip.id, view, duration);
            }
        }
        if let Some((start, end)) = stored_region(ui, edit.tag_key, &clip.id) {
            ui.label(
                RichText::new(format!(
                    "{} \u{2013} {} ({})",
                    format_play_time(start),
                    format_play_time(end),
                    format_play_time(end - start)
                ))
                .monospace()
                .color(foundation_blue()),
            )
            .on_hover_text(
                "The region played; \u{27F2} loops it. Esc or a click outside clears it.",
            );
        }
        ui.add_space(8.0);
        draw_sound_output_controls(ui, edit, languages);
    });
    draw_sound_errors(ui, edit);

    // Space plays or pauses and Enter stops, in the focused tab, when no text
    // field has the keyboard.
    if edit.sound_has_focus && !keyboard_busy && !ui.ctx().egui_wants_keyboard_input() {
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
            stop(ui, edit, &clips[selected].id);
        }
    }
    selected
}

/// Where a tab's player keeps the id of its selected clip. Not tied to the
/// widget's place in the layout: the tab and the player are what it belongs to.
pub(in crate::app::editor) fn clip_selection_id(id_salt: &str, tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_selection", id_salt, tag_key))
}

/// Select clip `index` in the player `id_salt` of this tab and play it — for a
/// list beside the player (the dialogue overview) whose rows play through it.
pub(super) fn play_clip_now(
    ctx: &egui::Context,
    edit: &mut FieldEditContext<'_>,
    id_salt: &str,
    clips: &[PlayerClip],
    index: usize,
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
) {
    let Some(clip) = clips.get(index) else {
        return;
    };
    ctx.data_mut(|data| {
        data.insert_temp(clip_selection_id(id_salt, edit.tag_key), clip.id.clone())
    });
    queue_play(edit, clips, index, play);
}

/// The order clips are listed in: each group where it first appears, and
/// within a group by [`natural_cmp`] on the name.
fn display_order(clips: &[PlayerClip]) -> Vec<usize> {
    let mut groups: Vec<Option<&str>> = Vec::new();
    for clip in clips {
        if !groups.contains(&clip.group.as_deref()) {
            groups.push(clip.group.as_deref());
        }
    }
    let mut order: Vec<usize> = (0..clips.len()).collect();
    order.sort_by(|&a, &b| {
        let group = |index: usize| {
            groups
                .iter()
                .position(|g| *g == clips[index].group.as_deref())
        };
        group(a)
            .cmp(&group(b))
            .then_with(|| natural_cmp(&clips[a].name, &clips[b].name))
    });
    order
}

/// Compare names the way people count: runs of digits by their value, the
/// rest case-insensitively, so `2` comes before `10` and `pain2` before
/// `pain10`. Names that only differ in case or leading zeros fall back to
/// plain order, so the result is total.
pub(super) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let ordering = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (Some(c), Some(d)) => {
                let ordering = c.to_lowercase().cmp(d.to_lowercase());
                if ordering != Ordering::Equal {
                    return ordering;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// `group ▸ name`, or the name alone.
fn clip_title(clip: &PlayerClip) -> String {
    match &clip.group {
        Some(group) => format!("{group} \u{25B8} {}", clip.name),
        None => clip.name.clone(),
    }
}

/// Stop and rewind, clearing the region on `clip`: a stop starts over.
fn stop(ui: &Ui, edit: &mut FieldEditContext<'_>, clip: &str) {
    if stored_region(ui, edit.tag_key, clip).is_some() {
        ui.data_mut(|data| data.remove::<(String, f64, f64)>(clip_region_id(edit.tag_key)));
        edit.sound_play_request
            .push_for_clip(SoundAction::SetRegion(None), clip.to_owned());
    }
    edit.sound_play_request.push_back(SoundAction::Stop);
}

/// Pause or resume the selected clip if it is the one loaded, else play it.
fn toggle_or_play(
    edit: &mut FieldEditContext<'_>,
    clips: &[PlayerClip],
    selected: usize,
    loaded: Option<&PlaybackView>,
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
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
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
) {
    match play(index) {
        Some(ClipPlay::Action(action)) => edit
            .sound_play_request
            .play_clip(action, clips[index].id.clone()),
        Some(ClipPlay::CeRef(mut request)) => {
            request.clip = Some(clips[index].id.clone());
            *edit.ce_sound_ref_request = Some(request);
        }
        None => {}
    }
}

/// Ask for clip `index` to be decoded for its preview. A clip with nothing to
/// play is remembered, so it is not asked for again every frame.
fn request_preview(
    ui: &Ui,
    edit: &mut FieldEditContext<'_>,
    clips: &[PlayerClip],
    index: usize,
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
) {
    let id = clips[index].id.clone();
    let nothing = nothing_to_preview_id(edit.tag_key);
    let known = ui
        .data(|data| data.get_temp::<HashSet<String>>(nothing))
        .unwrap_or_default();
    if known.contains(&id) {
        return;
    }
    match play(index) {
        Some(ClipPlay::Action(action)) => edit.sound_play_request.preview_clip(action, id),
        Some(ClipPlay::CeRef(mut request)) => {
            // One reference request a frame; a click's play wins it.
            if edit.ce_sound_ref_request.is_none() {
                request.clip = Some(id);
                request.preview = true;
                *edit.ce_sound_ref_request = Some(request);
            }
        }
        None => ui.data_mut(|data| {
            data.get_temp_mut_or_default::<HashSet<String>>(nothing)
                .insert(id);
        }),
    }
}

/// Whether a play for `index` was already queued this frame (by Random).
fn queued_play_for(edit: &FieldEditContext<'_>, clips: &[PlayerClip], index: usize) -> bool {
    let id = clips[index].id.as_str();
    edit.sound_play_request.queued_clip(id)
        || edit
            .ce_sound_ref_request
            .as_ref()
            .is_some_and(|request| request.clip.as_deref() == Some(id))
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
    preview: Option<&Preview>,
    play: &mut dyn FnMut(usize) -> Option<ClipPlay>,
) {
    let previewed = match preview.map(|preview| &preview.state) {
        Some(PreviewState::Ready(waveform)) => Some(waveform),
        _ => None,
    };
    let duration = loaded
        .map(|playback| playback.duration)
        .or(clips[selected].duration)
        .or(previewed.map(|waveform| waveform.duration_secs()))
        .unwrap_or(0.0);
    let position = loaded.map_or(0.0, |playback| playback.position);
    // The loaded sound's, else the preview's: unplayed, the playhead at 0.
    let waveform = loaded.map(|playback| &playback.waveform).or(previewed);
    let size = Vec2::new(
        ui.available_width().max(120.0),
        RULER_HEIGHT + track_height(waveform.map(|waveform| waveform.channels())),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let track =
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + RULER_HEIGHT), rect.max);
    painter.rect(
        track,
        3.0,
        foundation_input(),
        egui::Stroke::new(1.0, foundation_input_edge()),
        egui::StrokeKind::Middle,
    );

    // The part of the clip shown: all of it until zoomed. While playing it
    // pages along with the playhead, unless panned or zoomed by hand.
    let clip_id = clips[selected].id.clone();
    let min_span = min_view_span(
        waveform.map(|waveform| waveform.pcm().sample_rate),
        track.width(),
    );
    let mut view = load_view(ui, edit.tag_key, &clip_id, duration, min_span);
    let playing = loaded.is_some_and(|playback| playback.playing);
    let was_playing_id = was_playing_id(edit.tag_key);
    if playing
        && !ui
            .data(|data| data.get_temp::<bool>(was_playing_id))
            .unwrap_or(false)
    {
        view.follow = true;
    }
    ui.data_mut(|data| data.insert_temp(was_playing_id, playing));
    if playing
        && view.follow
        && !view.whole(duration)
        && (position < view.start || position > view.start + view.span)
    {
        view.start = position - view.span * 0.05;
        view = view.clamped(duration, min_span);
    }

    let x_of = |seconds: f64| {
        if view.span > 0.0 {
            track.left() + ((seconds - view.start) / view.span) as f32 * track.width()
        } else {
            track.left()
        }
    };

    // Ruler: a tick every "nice" step that leaves room for its label.
    if duration > 0.0 && view.span > 0.0 {
        let step = ruler_step(view.span, track.width());
        let mut tick = (view.start / step).ceil() * step;
        while tick <= view.start + view.span + 1e-9 {
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

    // The waveform, then the playhead over it.
    let head = x_of(position);
    match waveform {
        Some(waveform) => draw_lanes(ui, &painter, track, waveform, head, view),
        None => {
            let note = match preview.map(|preview| &preview.state) {
                Some(PreviewState::Pending) => "Loading waveform\u{2026}".to_owned(),
                Some(PreviewState::Failed(reason)) => reason.clone(),
                _ => "Play to see the waveform".to_owned(),
            };
            painter.text(
                track.center(),
                egui::Align2::CENTER_CENTER,
                note,
                egui::FontId::proportional(11.0),
                subtle_dark(),
            );
        }
    }
    // The region: a band over the lanes, its edges marked.
    let region_id = clip_region_id(edit.tag_key);
    let mut region = stored_region(ui, edit.tag_key, &clip_id);
    if let Some((start, end)) = region {
        let band = egui::Rect::from_min_max(
            egui::pos2(x_of(start), track.top()),
            egui::pos2(x_of(end), track.bottom()),
        );
        painter.rect_filled(band, 0.0, foundation_blue().gamma_multiply(0.18));
        for x in [band.left(), band.right()] {
            painter.line_segment(
                [egui::pos2(x, track.top()), egui::pos2(x, track.bottom())],
                egui::Stroke::new(1.5, foundation_blue()),
            );
        }
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

    let time_at = |x: f32| {
        view.start + (((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64) * view.span
    };
    let edge_grab = 5.0;
    let near = |x: f32, seconds: f64| (x - x_of(seconds)).abs() <= edge_grab;
    // The ruler (and the playhead's head) scrub; the lanes select.
    let in_ruler = |pointer: egui::Pos2| pointer.y < track.top() || (pointer.x - head).abs() <= 4.0;
    let (pressed, moved, origin) = ui.input(|input| {
        (
            input.pointer.any_pressed(),
            input.pointer.delta() != Vec2::ZERO,
            input.pointer.press_origin(),
        )
    });
    let set_region = |edit: &mut FieldEditContext<'_>, region: Option<(f64, f64)>| {
        match region {
            Some((start, end)) => {
                ui.data_mut(|data| data.insert_temp(region_id, (clip_id.clone(), start, end)));
            }
            None => ui.data_mut(|data| data.remove::<(String, f64, f64)>(region_id)),
        }
        edit.sound_play_request
            .push_for_clip(SoundAction::SetRegion(region), clip_id.clone());
    };

    // Scrubbing: seek when the button goes down on the ruler and whenever the
    // pointer moves while it is held — not again on release, which would pull
    // a playing sound back by the time the click took.
    let scrubbing = origin.is_some_and(in_ruler);
    if let Some(pointer) = response.interact_pointer_pos()
        && scrubbing
        && loaded.is_some()
        && response.is_pointer_button_down_on()
        && (pressed || moved)
    {
        edit.sound_play_request
            .push_back(SoundAction::Seek(time_at(pointer.x)));
    }

    // Selecting: a drag across the lanes draws a region from where it began,
    // or moves the edge it began on.
    let drag_id = drag_anchor_id(edit.tag_key);
    if response.drag_started()
        && let Some(origin) = origin.filter(|origin| !in_ruler(*origin))
        && duration > 0.0
    {
        let anchor = match region {
            Some((start, end)) if near(origin.x, start) => end,
            Some((start, end)) if near(origin.x, end) => start,
            _ => time_at(origin.x),
        };
        ui.data_mut(|data| data.insert_temp(drag_id, anchor));
    }
    if let Some(anchor) = ui.data(|data| data.get_temp::<f64>(drag_id)) {
        if let Some(pointer) = response.interact_pointer_pos()
            && response.dragged()
        {
            let other = time_at(pointer.x);
            let drawn = (anchor.min(other), anchor.max(other));
            if region != Some(drawn) {
                region = Some(drawn);
                set_region(edit, region);
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<f64>(drag_id));
            // Too narrow to mean anything: a click, not a region.
            if let Some((start, end)) = region
                && x_of(end) - x_of(start) < 3.0
            {
                region = None;
                set_region(edit, None);
            }
        }
    }

    // A click on the lanes: off the region clears it; then seek there, or
    // play from there.
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && !in_ruler(pointer)
    {
        let seconds = time_at(pointer.x);
        if region.is_some_and(|(start, end)| seconds < start || seconds > end) {
            region = None;
            set_region(edit, None);
        }
        if loaded.is_some() {
            edit.sound_play_request
                .push_back(SoundAction::Seek(seconds));
        } else {
            queue_play(edit, clips, selected, play);
            // A previewed clip plays at once, so it can start where clicked.
            if previewed.is_some() {
                edit.sound_play_request
                    .push_back(SoundAction::Seek(seconds));
            }
        }
    } else if response.clicked() && loaded.is_none() {
        queue_play(edit, clips, selected, play);
    }

    // Escape clears the region, in the focused tab.
    if region.is_some()
        && edit.sound_has_focus
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        set_region(edit, None);
    }

    if let Some(pointer) = response.hover_pos()
        && let Some((start, end)) = region
        && !in_ruler(pointer)
        && (near(pointer.x, start) || near(pointer.x, end))
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    // Ctrl/Cmd + wheel (or a pinch) zooms around the pointer; a sideways
    // scroll (Shift + wheel) pans. A plain wheel is left to the page.
    if let Some(pointer) = response.hover_pos()
        && duration > 0.0
    {
        let (zoom, pan) = ui.input(|input| (input.zoom_delta(), input.smooth_scroll_delta.x));
        if zoom != 1.0 {
            view = view.zoomed(f64::from(zoom), time_at(pointer.x), duration, min_span);
        }
        if pan != 0.0 && !view.whole(duration) {
            view.start -= f64::from(pan / track.width()) * view.span;
            view.follow = false;
            view = view.clamped(duration, min_span);
        }
    }
    store_view(ui, edit.tag_key, &clip_id, view, duration);
    if let Some(waveform) = waveform.filter(|_| !view.whole(duration))
        && let Some(moved) = draw_overview(ui, waveform, duration, view)
    {
        store_view(
            ui,
            edit.tag_key,
            &clip_id,
            moved.clamped(duration, min_span),
            duration,
        );
    }

    response.on_hover_text(if loaded.is_some() || previewed.is_some() {
        "Click to jump \u{00B7} drag to select a region \u{00B7} drag the ruler to scrub \u{00B7} \
         Ctrl/Cmd + wheel zooms, Shift + wheel scrolls"
    } else {
        "Click to play"
    });
}

/// The part of a clip the timeline shows, in seconds. `follow` pages it
/// along with the playhead while playing; panning or zooming by hand turns
/// it off until playing starts again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct View {
    pub(super) start: f64,
    pub(super) span: f64,
    pub(super) follow: bool,
}

impl View {
    fn fit(duration: f64) -> Self {
        Self {
            start: 0.0,
            span: duration.max(0.0),
            follow: true,
        }
    }

    /// Kept within the clip and between `min_span` and the whole of it.
    fn clamped(self, duration: f64, min_span: f64) -> Self {
        let span = self.span.clamp(min_span.min(duration), duration.max(0.0));
        Self {
            start: self.start.clamp(0.0, (duration - span).max(0.0)),
            span,
            follow: self.follow,
        }
    }

    fn zoomed(self, factor: f64, around: f64, duration: f64, min_span: f64) -> Self {
        let fraction = if self.span > 0.0 {
            ((around - self.start) / self.span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let span = (self.span / factor).clamp(min_span.min(duration), duration);
        Self {
            start: around - fraction * span,
            span,
            follow: false,
        }
        .clamped(duration, min_span)
    }

    fn whole(&self, duration: f64) -> bool {
        self.span >= duration - 1e-9
    }
}

/// The clips of a tab's player that have nothing to preview.
fn nothing_to_preview_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_nothing_to_preview", tag_key))
}

fn was_playing_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_was_playing", tag_key))
}

fn drag_anchor_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_drag_anchor", tag_key))
}

/// Every player drawn, as (player, tab), so a closed tab's can be forgotten.
fn players_id() -> egui::Id {
    egui::Id::new("clip_players")
}

fn register_player(ctx: &egui::Context, id_salt: &str, tag_key: &str) {
    ctx.data_mut(|data| {
        let players = data.get_temp_mut_or_default::<HashSet<(String, String)>>(players_id());
        if !players.contains(&(id_salt.to_owned(), tag_key.to_owned())) {
            players.insert((id_salt.to_owned(), tag_key.to_owned()));
        }
    });
}

/// Forget what each closed tab's player kept — its selection, region, view
/// and the rest — once `is_open` says the tab is gone. Run every frame,
/// like the audio state's own check, so every way a tab closes counts.
pub(in crate::app) fn forget_closed_players(ctx: &egui::Context, is_open: impl Fn(&str) -> bool) {
    let closed: Vec<(String, String)> = ctx.data(|data| {
        data.get_temp::<HashSet<(String, String)>>(players_id())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, tag_key)| !is_open(tag_key))
            .collect()
    });
    if closed.is_empty() {
        return;
    }
    ctx.data_mut(|data| {
        for (id_salt, tag_key) in &closed {
            data.remove::<String>(clip_selection_id(id_salt, tag_key));
            data.remove::<(String, f64, f64)>(clip_region_id(tag_key));
            data.remove::<(String, View)>(clip_view_id(tag_key));
            data.remove::<HashSet<String>>(nothing_to_preview_id(tag_key));
            data.remove::<bool>(was_playing_id(tag_key));
            data.remove::<f64>(drag_anchor_id(tag_key));
        }
        let players = data.get_temp_mut_or_default::<HashSet<(String, String)>>(players_id());
        for player in &closed {
            players.remove(player);
        }
    });
}

fn clip_view_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_view", tag_key))
}

/// The view a tab's player has on `clip`: the whole clip until zoomed.
pub(super) fn load_view(ui: &Ui, tag_key: &str, clip: &str, duration: f64, min_span: f64) -> View {
    ui.data(|data| data.get_temp::<(String, View)>(clip_view_id(tag_key)))
        .filter(|(view_clip, _)| view_clip == clip)
        .map(|(_, view)| view.clamped(duration, min_span))
        .unwrap_or_else(|| View::fit(duration))
}

/// Keep a zoomed or scrolled view. The whole clip is not kept: it is what a
/// clip shows by default, and a length not known yet (a referenced sound
/// before its preview decodes) would otherwise be kept as a zero-length view
/// that the real length later clamps to the narrowest zoom.
fn store_view(ui: &Ui, tag_key: &str, clip: &str, view: View, duration: f64) {
    ui.data_mut(|data| {
        if duration <= 0.0 || view.whole(duration) {
            data.remove::<(String, View)>(clip_view_id(tag_key));
        } else {
            data.insert_temp(clip_view_id(tag_key), (clip.to_owned(), view));
        }
    });
}

/// The narrowest view: about eight points a sample, or a millisecond when
/// the rate is not known yet.
fn min_view_span(sample_rate: Option<u32>, width: f32) -> f64 {
    match sample_rate {
        Some(rate) if rate > 0 => f64::from(width.max(1.0) / 8.0) / f64::from(rate),
        _ => 0.001,
    }
}

/// Where a tab's player keeps its region: the clip it is on, and its start
/// and end in seconds.
fn clip_region_id(tag_key: &str) -> egui::Id {
    egui::Id::new(("clip_player_region", tag_key))
}

/// The region a tab's player has on `clip`, if any.
fn stored_region(ui: &Ui, tag_key: &str, clip: &str) -> Option<(f64, f64)> {
    ui.data(|data| data.get_temp::<(String, f64, f64)>(clip_region_id(tag_key)))
        .filter(|(region_clip, _, _)| region_clip == clip)
        .map(|(_, start, end)| (start, end))
}

/// One lane per channel across `track`: for each physical pixel column, a
/// bar from the column's lowest to highest sample and a brighter core of its
/// RMS, played columns (left of `head`) in the accent and the rest muted.
fn draw_lanes(
    ui: &Ui,
    painter: &egui::Painter,
    track: egui::Rect,
    waveform: &crate::app::audio::Waveform,
    head: f32,
    view: View,
) {
    let channels = waveform.channels().max(1);
    let lane_height = track.height() / f32::from(channels);
    let pixels_per_point = ui.ctx().pixels_per_point();
    let columns = (track.width() * pixels_per_point).floor().max(1.0) as usize;
    let column_width = track.width() / columns as f32;
    let rate = f64::from(waveform.pcm().sample_rate.max(1));
    let first = view.start * rate;
    let last = ((view.start + view.span) * rate).min(waveform.frames() as f64);
    let frames_per_column = (last - first).max(0.0) / columns as f64;
    let labels = crate::app::audio::channel_labels(channels);
    let played = foundation_blue();
    let unplayed = subtle_dark().gamma_multiply(0.55);
    let mut mesh = egui::Mesh::default();
    for channel in 0..channels as usize {
        let lane = egui::Rect::from_min_size(
            egui::pos2(track.left(), track.top() + lane_height * channel as f32),
            Vec2::new(track.width(), lane_height),
        );
        let middle = lane.center().y;
        let half = lane_height * 0.45;
        painter.line_segment(
            [
                egui::pos2(lane.left(), middle),
                egui::pos2(lane.right(), middle),
            ],
            egui::Stroke::new(1.0, grid_line().gamma_multiply(0.6)),
        );
        if channel > 0 {
            painter.line_segment(
                [
                    egui::pos2(lane.left(), lane.top()),
                    egui::pos2(lane.right(), lane.top()),
                ],
                egui::Stroke::new(1.0, grid_line()),
            );
        }
        if frames_per_column < 1.0 {
            // Closer than a sample a pixel: a line through the samples, and
            // a dot on each once they are far enough apart to tell.
            draw_samples(painter, waveform, channel, track, lane, first, last, head);
            continue;
        }
        for column in 0..columns {
            let start = (first + column as f64 * frames_per_column) as u64;
            let end = ((first + (column + 1) as f64 * frames_per_column) as u64).max(start + 1);
            let Some(peak) = waveform.peak(channel, start, end) else {
                break;
            };
            let left = track.left() + column as f32 * column_width;
            let y = |sample: f32| middle - sample / 32768.0 * half;
            let color = if left < head { played } else { unplayed };
            mesh.add_colored_rect(
                egui::Rect::from_min_max(
                    egui::pos2(left, y(f32::from(peak.max))),
                    egui::pos2(
                        left + column_width,
                        y(f32::from(peak.min)).max(y(f32::from(peak.max)) + 1.0 / pixels_per_point),
                    ),
                ),
                color.gamma_multiply(0.55),
            );
            let rms = peak.rms() * 32768.0;
            if rms > 0.0 {
                mesh.add_colored_rect(
                    egui::Rect::from_min_max(
                        egui::pos2(left, y(rms.min(f32::from(peak.max).max(0.0)))),
                        egui::pos2(
                            left + column_width,
                            y(-rms.min(-f32::from(peak.min).max(0.0))),
                        ),
                    ),
                    color,
                );
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    // Labels last, over the waveform.
    if channels > 1 {
        for (channel, label) in labels.iter().enumerate() {
            painter.text(
                egui::pos2(
                    track.left() + 4.0,
                    track.top() + lane_height * channel as f32 + 1.0,
                ),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(9.0),
                subtle_dark(),
            );
        }
    }
}

/// One channel's samples from frame `first` to `last` as a line across
/// `track`, in `lane`; past six points a sample, with a dot on each.
#[allow(clippy::too_many_arguments)]
fn draw_samples(
    painter: &egui::Painter,
    waveform: &crate::app::audio::Waveform,
    channel: usize,
    track: egui::Rect,
    lane: egui::Rect,
    first: f64,
    last: f64,
    head: f32,
) {
    let pcm = waveform.pcm();
    let channels = pcm.channels as usize;
    let span = (last - first).max(1e-9);
    let middle = lane.center().y;
    let half = lane.height() * 0.45;
    let point = |frame: u64| {
        let sample = pcm.samples[frame as usize * channels + channel];
        egui::pos2(
            track.left() + ((frame as f64 - first) / span) as f32 * track.width(),
            middle - f32::from(sample) / 32768.0 * half,
        )
    };
    let from = first.floor().max(0.0) as u64;
    let to = (last.ceil() as u64 + 1).min(waveform.frames());
    let color = |x: f32| {
        if x < head {
            foundation_blue()
        } else {
            subtle_dark()
        }
    };
    let dots = track.width() as f64 / span >= 6.0;
    let mut previous: Option<egui::Pos2> = None;
    for frame in from..to {
        let here = point(frame);
        if let Some(previous) = previous {
            painter.line_segment([previous, here], egui::Stroke::new(1.5, color(here.x)));
        }
        if dots {
            painter.circle_filled(here, 2.0, color(here.x));
        }
        previous = Some(here);
    }
}

/// The whole clip in a strip under the lanes, with the part shown boxed:
/// drag the box to scroll, click elsewhere to centre the view there. The
/// view the strip moved to, if it did.
fn draw_overview(
    ui: &mut Ui,
    waveform: &crate::app::audio::Waveform,
    duration: f64,
    view: View,
) -> Option<View> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(120.0), OVERVIEW_HEIGHT),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, foundation_input());
    let columns = rect.width().max(1.0) as usize;
    let frames_per_column = waveform.frames() as f64 / columns as f64;
    let mut mesh = egui::Mesh::default();
    for column in 0..columns {
        let start = (column as f64 * frames_per_column) as u64;
        let end = (((column + 1) as f64 * frames_per_column) as u64).max(start + 1);
        let loudest = (0..waveform.channels() as usize)
            .filter_map(|channel| waveform.peak(channel, start, end))
            .map(|peak| f32::from(peak.max).max(-f32::from(peak.min)) / 32768.0)
            .fold(0.0, f32::max);
        let x = rect.left() + column as f32;
        let half = rect.height() * 0.45 * loudest;
        mesh.add_colored_rect(
            egui::Rect::from_min_max(
                egui::pos2(x, rect.center().y - half),
                egui::pos2(x + 1.0, rect.center().y + half.max(0.5)),
            ),
            subtle_dark().gamma_multiply(0.55),
        );
    }
    painter.add(egui::Shape::mesh(mesh));
    let x_of = |seconds: f64| rect.left() + (seconds / duration) as f32 * rect.width();
    let shown = egui::Rect::from_min_max(
        egui::pos2(x_of(view.start), rect.top()),
        egui::pos2(
            x_of(view.start + view.span).max(x_of(view.start) + 3.0),
            rect.bottom(),
        ),
    );
    painter.rect(
        shown,
        2.0,
        foundation_blue().gamma_multiply(0.2),
        egui::Stroke::new(1.0, foundation_blue()),
        egui::StrokeKind::Middle,
    );
    let response =
        response.on_hover_text("The whole sound: drag the box to scroll, click to centre");
    let mut moved = None;
    if response.dragged() {
        let shift = f64::from(response.drag_delta().x / rect.width()) * duration;
        moved = Some(View {
            start: view.start + shift,
            follow: false,
            ..view
        });
    } else if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let at = f64::from((pointer.x - rect.left()) / rect.width()) * duration;
        moved = Some(View {
            start: at - view.span / 2.0,
            follow: false,
            ..view
        });
    }
    moved
}

/// The ruler's tick spacing: the smallest of a set of round steps that
/// keeps ticks at least ~70 points apart.
pub(super) fn ruler_step(duration: f64, width: f32) -> f64 {
    const STEPS: [f64; 17] = [
        0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0,
        30.0, 60.0,
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
    } else if step >= 0.01 {
        2
    } else {
        3
    };
    format!("{seconds:.decimals$}")
}

#[cfg(test)]
mod tests;
