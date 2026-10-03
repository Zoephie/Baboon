//! The clip player: one sound out of a tag's several, picked from a dropdown,
//! played and scrubbed on a timeline.
//! It owns presentation and the clip selection; playback belongs to `audio`.

use super::*;

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

/// Every glyph the player draws, which the tests check the app's fonts have.
#[cfg(test)]
const PLAYER_GLYPHS: [&str; 10] = [
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
    let nothing = egui::Id::new(("clip_player_nothing_to_preview", edit.tag_key, id.as_str()));
    if ui.data(|data| data.get_temp::<bool>(nothing)).is_some() {
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
        None => ui.data_mut(|data| data.insert_temp(nothing, true)),
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

    // The waveform, then the playhead over it.
    let head = x_of(position);
    match waveform {
        Some(waveform) => draw_lanes(ui, &painter, track, waveform, head),
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
    let clip_id = clips[selected].id.clone();
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

    let time_at = |x: f32| (((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64) * duration;
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
                ui.data_mut(|data| data.insert_temp(region_id, (clip_id.clone(), start, end)))
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
    let drag_id = egui::Id::new(("clip_player_drag_anchor", edit.tag_key));
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
        && !ui.ctx().wants_keyboard_input()
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
    response.on_hover_text(if loaded.is_some() || previewed.is_some() {
        "Click to jump \u{00B7} drag to select a region \u{00B7} drag the ruler to scrub"
    } else {
        "Click to play"
    });
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
) {
    let channels = waveform.channels().max(1);
    let lane_height = track.height() / f32::from(channels);
    let pixels_per_point = ui.ctx().pixels_per_point();
    let columns = (track.width() * pixels_per_point).floor().max(1.0) as usize;
    let column_width = track.width() / columns as f32;
    let frames = waveform.frames();
    let frames_per_column = frames as f64 / columns as f64;
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
        for column in 0..columns {
            let start = (column as f64 * frames_per_column) as u64;
            let end = (((column + 1) as f64 * frames_per_column) as u64).max(start + 1);
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
        /// The waveform's bars from the last frame: each rect and its colour.
        bars: Vec<(egui::Rect, egui::Color32)>,
        /// Filled rects from the last frame, with their fill.
        fills: Vec<(egui::Rect, egui::Color32)>,
        /// The clips previews were asked for, in order.
        previewed: Vec<String>,
        /// The tab's preview, as the audio state would hand it over.
        preview: Option<Preview>,
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
                previewed: Vec::new(),
                preview: None,
                timeline: egui::Rect::NOTHING,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            let clips = clips();
            let mut sinks = EditSinks::default();
            let playback = self.playback.clone();
            let preview = self.preview.clone();
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
                        edit.sound_preview = preview.clone();
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
            self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
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
    #[test]
    fn every_player_glyph_is_in_the_app_s_fonts() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        let _ = ctx.run(Default::default(), |_| {});
        let missing: Vec<&str> = PLAYER_GLYPHS
            .into_iter()
            .filter(|glyph| {
                !ctx.fonts(|fonts| fonts.has_glyphs(&egui::FontId::proportional(14.0), glyph))
            })
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
}
