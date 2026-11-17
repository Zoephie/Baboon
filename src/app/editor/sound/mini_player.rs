//! The mini player: a small card in the workspace's corner for the sound
//! that is playing while its own player is out of sight, scrolled away or
//! behind another tab, so it can be paused or stopped from anywhere.

use std::collections::VecDeque;
use std::sync::Arc;

use super::{PlayerDeck, deck_id, random_other};

use egui::{Align2, CornerRadius, Frame, RichText, Sense, Vec2};

use crate::app::audio::{AudioCommand, AudioState, PlaybackView, SoundAction, SoundOwner, SoundRequests};
use crate::app::context::Ctx;
use crate::app::editor::dialogs::EditorCommand;
use crate::app::ui_kit::{
    ButtonIcon, foundation_section_bar, icon_button, paint_tag_icon_at, progress_fill, progress_track,
    selectable_icon_button, subtle_dark, text_dark, window_work_area,
};

const WIDTH: f32 = 260.0;
const HEADER_HEIGHT: f32 = 36.0;
/// About the whole card's height, to keep its top in the workspace.
const CARD_HEIGHT: f32 = 170.0;
/// From the workspace's corner.
const INSET: f32 = 16.0;

fn drawn_id() -> egui::Id {
    egui::Id::unique("sound_player_drawn")
}

/// Where the mini player was dragged to: its bottom-right corner's offset
/// from the workspace's, kept for the session.
fn offset_id() -> egui::Id {
    egui::Id::unique("sound_mini_player_offset")
}

fn on_screen_id(owner: &SoundOwner) -> egui::Id {
    egui::Id::unique(("sound_player_on_screen", owner))
}

/// A tag pane is about to draw: nothing of its sound player is in sight yet.
pub(in crate::app) fn sound_player_drawing(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<bool>(drawn_id()));
}

/// A sound card was drawn with some of it in sight.
pub(super) fn mark_player_on_screen(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(drawn_id(), true));
}

/// A tag pane has drawn: whether its sound player showed, for the mini
/// player drawn after every pane this pass.
pub(in crate::app) fn sound_player_drawn(ctx: &egui::Context, owner: &SoundOwner) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        if data.remove_temp::<bool>(drawn_id()).unwrap_or(false) {
            data.insert_temp(on_screen_id(owner), pass);
        }
    });
}

/// Whether the mini player shows: a sound under way — playing, or paused
/// partway — whose own player was not in sight this pass.
fn shown(playback: &PlaybackView, on_screen: bool) -> bool {
    under_way(playback.playing, playback.position, playback.duration) && !on_screen
}

fn under_way(playing: bool, position: f64, duration: f64) -> bool {
    playing || (position > 0.0 && position < duration)
}

/// The mini player, over the workspace's bottom-right corner.
pub(in crate::app) fn draw_sound_mini_player(cx: &Ctx<'_>, audio: &AudioState) {
    let ctx = cx.egui;
    let Some((owner, playback)) = audio.now_playing() else {
        return;
    };
    let on_screen = ctx.data(|data| data.get_temp::<u64>(on_screen_id(&owner)))
        == Some(ctx.cumulative_pass_nr());
    if !shown(&playback, on_screen) {
        return;
    }
    let kit = cx.model.kits.iter().find(|kit| kit.id == owner.kit);
    let entry = kit.and_then(|kit| {
        kit.source
            .as_ref()
            .and_then(|source| source.entry_for_key(&owner.key))
            .or_else(|| kit.active_favorite_entries.iter().find(|entry| entry.key == owner.key))
    });
    let title = entry.map_or_else(|| playback.label.clone(), crate::app::browser::tag_tab_label);
    let group_tag = entry.map(|entry| entry.group_tag);
    // The folder it is in, which the name alone may not tell apart.
    let folder = entry
        .and_then(|entry| crate::core::tag_path::parent(&entry.display_path))
        .map(|folder| format!("{folder}{}", crate::core::tag_path::SEPARATOR));
    let game = kit.and_then(|kit| kit.source.as_ref()).and_then(|source| source.game);

    // The tab's clips, when its player left them, and which one plays.
    let deck = ctx.data(|data| data.get_temp::<Arc<PlayerDeck>>(deck_id(&owner.key)));
    let current = deck.as_ref().and_then(|deck| {
        let id = playback.clip.as_deref()?;
        deck.clips.iter().position(|(clip, _)| clip == id)
    });

    let mut queue = VecDeque::new();
    let mut requests = SoundRequests::new(&mut queue, Some(owner.clone()));
    let area = window_work_area(ctx);
    let offset = ctx
        .data(|data| data.get_temp::<Vec2>(offset_id()))
        .unwrap_or(Vec2::splat(-INSET));
    // Kept in the workspace however the window was resized since.
    let corner = (area.right_bottom() + offset).clamp(
        area.left_top() + Vec2::new(WIDTH, CARD_HEIGHT),
        area.right_bottom(),
    );
    egui::Area::new(egui::Id::unique("sound_mini_player"))
        .order(egui::Order::Middle)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(corner)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).inner_margin(0.0).show(ui, |ui| {
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing.y = 0.0;
                let radius = ui.style().visuals.window_corner_radius;

                // The header: the tag, which brings its tab back, and the
                // handle the card is dragged by.
                let (header, response) =
                    ui.allocate_exact_size(Vec2::new(WIDTH, HEADER_HEIGHT), Sense::click_and_drag());
                if response.dragged() {
                    let moved = corner + response.drag_delta() - area.right_bottom();
                    ctx.data_mut(|data| data.insert_temp(offset_id(), moved));
                }
                ui.painter().rect_filled(
                    header,
                    CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 },
                    foundation_section_bar(),
                );
                let font = egui::TextStyle::Body.resolve(ui.style());
                let color = if response.hovered() { text_dark() } else { ui.visuals().text_color() };
                let galley = ui.painter().layout_no_wrap(title.clone(), font, color);
                let icon = 18.0;
                let gap = if group_tag.is_some() { icon + 8.0 } else { 0.0 };
                let text_width = galley.size().x.min(WIDTH - 24.0 - gap);
                let left = header.center().x - (gap + text_width) / 2.0;
                if group_tag.is_some() {
                    let icon_rect = egui::Rect::from_center_size(
                        egui::pos2(left + icon / 2.0, header.center().y),
                        Vec2::splat(icon),
                    );
                    paint_tag_icon_at(ui, group_tag, game, icon_rect);
                }
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(left + gap, header.center().y - galley.size().y / 2.0),
                    Vec2::new(text_width, galley.size().y),
                );
                ui.painter().with_clip_rect(text_rect).galley(text_rect.min, galley, color);
                let cursor = if response.dragged() {
                    egui::CursorIcon::Grabbing
                } else {
                    egui::CursorIcon::Grab
                };
                if response
                    .on_hover_text(format!("Go to {title}; drag to move"))
                    .on_hover_cursor(cursor)
                    .clicked()
                {
                    cx.send(EditorCommand::RevealTab {
                        kit: owner.kit,
                        key: owner.key.clone(),
                    });
                }

                // The folder, its start elided to fit.
                if let Some(folder) = &folder {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let shown = elide_start(ui, folder, WIDTH - 24.0);
                        ui.add_sized(
                            Vec2::new(WIDTH - 24.0, ui.spacing().interact_size.y),
                            egui::Label::new(RichText::new(shown).color(subtle_dark())).truncate(),
                        )
                        .on_hover_text(folder);
                    });
                }

                // Shuffle, Skip Back, Play, Stop, Skip Forward and Loop,
                // centred, as the player's header has them.
                ui.add_space(10.0);
                let buttons = 6.0 * crate::app::ui_kit::ICON_BUTTON_SIZE.x + 5.0 * 6.0;
                let many = deck.as_ref().is_some_and(|deck| deck.clips.len() > 1);
                let skip = |requests: &mut SoundRequests<'_>, to: Option<usize>| {
                    if let (Some(deck), Some(to)) = (deck.as_ref(), to) {
                        skip_to(ctx, requests, deck, current, to);
                    }
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space((WIDTH - buttons) / 2.0);
                    let deck_len = deck.as_ref().map_or(0, |deck| deck.clips.len());
                    if icon_button(ui, ButtonIcon::Shuffle, "Play a random one", many, text_dark())
                        .clicked()
                    {
                        skip(&mut requests, current.map(|current| random_other(current, deck_len)));
                    }
                    if icon_button(ui, ButtonIcon::SkipBack, "Previous", many, text_dark()).clicked() {
                        skip(&mut requests, step(deck.as_deref(), current, -1));
                    }
                    let (icon, hover) = if playback.playing {
                        (ButtonIcon::Pause, "Pause")
                    } else {
                        (ButtonIcon::Play, "Play")
                    };
                    if icon_button(ui, icon, hover, true, text_dark()).clicked() {
                        requests.push_back(SoundAction::TogglePause);
                    }
                    if icon_button(ui, ButtonIcon::Stop, "Stop", true, text_dark()).clicked() {
                        requests.push_back(SoundAction::Stop);
                    }
                    if icon_button(ui, ButtonIcon::SkipForward, "Next", many, text_dark()).clicked() {
                        skip(&mut requests, step(deck.as_deref(), current, 1));
                    }
                    let looping = audio.looping();
                    if selectable_icon_button(ui, ButtonIcon::Loop, "Loop", looping, true).clicked() {
                        requests.push_back(SoundAction::SetLooping(!looping));
                    }
                });

                // How far through it is; click or drag to move.
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let (rail, response) = ui.allocate_exact_size(
                        Vec2::new(WIDTH - 24.0, 12.0),
                        Sense::click_and_drag(),
                    );
                    let track = egui::Rect::from_center_size(rail.center(), Vec2::new(rail.width(), 4.0));
                    ui.painter().rect_filled(track, 2.0, progress_track());
                    let fraction = if playback.duration > 0.0 {
                        (playback.position / playback.duration).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let mut filled = track;
                    filled.set_right(track.left() + track.width() * fraction);
                    ui.painter().rect_filled(filled, 2.0, progress_fill());
                    // A handle where it stands, while the pointer is on the bar.
                    if response.hovered() || response.dragged() {
                        let at = egui::pos2(filled.right(), track.center().y);
                        ui.painter().circle_filled(at, 6.0, progress_fill());
                        ui.painter().circle_stroke(at, 6.0, ui.visuals().widgets.hovered.fg_stroke);
                    }
                    if let Some(pointer) = response.interact_pointer_pos()
                        && (response.clicked() || response.dragged())
                    {
                        let at = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
                        requests.push_back(SoundAction::Seek(at as f64 * playback.duration));
                    }
                    response
                        .on_hover_text(format!(
                            "{} / {}",
                            clock(playback.position),
                            clock(playback.duration)
                        ))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                });
                ui.add_space(12.0);
            });
        });
    if !queue.is_empty() {
        cx.send(AudioCommand::Queue(queue));
    }
}

/// The clip `by` places from `current` in the player's order, wrapping.
fn step(deck: Option<&PlayerDeck>, current: Option<usize>, by: isize) -> Option<usize> {
    let deck = deck?;
    let rank = deck.order.iter().position(|&index| Some(index) == current)?;
    let len = deck.order.len() as isize;
    Some(deck.order[(rank as isize + by).rem_euclid(len) as usize])
}

/// Play clip `to` for the tab, and select it there, as its own player would.
fn skip_to(
    ctx: &egui::Context,
    requests: &mut SoundRequests<'_>,
    deck: &PlayerDeck,
    current: Option<usize>,
    to: usize,
) {
    if Some(to) == current {
        return;
    }
    let (id, action) = &deck.clips[to];
    // A region belongs to the clip it was drawn on.
    if let Some((from, _)) = current.and_then(|current| deck.clips.get(current))
        && ctx.data(|data| data.get_temp::<(String, f64, f64)>(deck.region_id)).is_some()
    {
        ctx.data_mut(|data| data.remove::<(String, f64, f64)>(deck.region_id));
        requests.push_for_clip(SoundAction::SetRegion(None), from.clone());
    }
    ctx.data_mut(|data| data.insert_temp(deck.selection_id, id.clone()));
    if let Some(action) = action {
        requests.play_clip(action.clone(), id.clone());
    }
}

/// `text` cut from its start, behind an ellipsis, to fit `width`: a path's
/// end says the most about it.
fn elide_start(ui: &egui::Ui, text: &str, width: f32) -> String {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let fits = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
            <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    for start in 1..chars.len() {
        let shown = format!("\u{2026}{}", chars[start..].iter().collect::<String>());
        if fits(&shown) {
            return shown;
        }
    }
    "\u{2026}".to_owned()
}

/// Seconds as m:ss.
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    format!("{}:{:04.1}", (seconds / 60.0).floor() as u64, seconds % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_for_a_sound_under_way() {
        assert!(under_way(true, 0.5, 2.0), "playing");
        assert!(under_way(false, 0.5, 2.0), "paused partway");
        assert!(!under_way(false, 0.0, 2.0), "stopped");
        assert!(!under_way(false, 2.0, 2.0), "finished");
    }

    #[test]
    fn its_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(75.25), "1:15.2");
    }
}
