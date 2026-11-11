//! A bitmap's sequences and sprites in its preview: the Sequences sidebar,
//! what the canvas draws over the image, picking a sprite on it, and playing a
//! sequence.
//!
//! What the tags hold and how the game reads them is set out in
//! `docs/bitmap-sequence-viewer-research.md`. In short: a processed sprite's
//! rect is a fraction of its image, spacing included, and its registration
//! point an offset from the rect's top-left; a manual sprite is a window in
//! repeat space, read only by decals and GPU particles.

use super::*;
use blam_tags::bitmap::{BitmapDefinition, BitmapSequence, BitmapSprite, BitmapType};
use eframe::egui::{Pos2, Rect};
use std::sync::Arc;

use crate::app::ui_kit::dock::{
    SidebarLimits, draw_sidebar_resize_handle, in_rect, sidebar_scroll_gutter, split_sidebar,
};

/// More sequences than this, and the Sequences card offers a filter.
const FILTER_FROM: usize = 32;

/// The sidebar's limits; the canvas keeps at least the rest.
const SIDEBAR_LIMITS: SidebarLimits = SidebarLimits {
    min: MIN_BITMAP_SIDEBAR_WIDTH,
    max: MAX_BITMAP_SIDEBAR_WIDTH,
    min_main: 320.0,
    gap: 8.0,
};

/// The tag's sequences, read again only when the document's `stamp` (its id
/// and revision) moves, or none when it has neither block. A selection an
/// edit took away goes with it.
pub(in crate::app) fn tag_sequences(
    preview: &mut BitmapPreviewState,
    tag: &TagFile,
    stamp: (u64, u64),
) -> Option<Arc<BitmapDefinition>> {
    if preview.sequences.as_ref().is_none_or(|(read, _)| *read != stamp) {
        preview.sequences = Some((stamp, Arc::new(BitmapDefinition::from_tag(tag))));
    }
    let definition = preview.sequences.as_ref().map(|(_, definition)| definition.clone())?;
    if preview
        .sprite_selection
        .is_some_and(|selection| !selection_exists(&definition, selection))
    {
        preview.sprite_selection = None;
        preview.sprite_playback = None;
    }
    (!definition.sequences.is_empty() || !definition.manual_sequences.is_empty())
        .then_some(definition)
}

/// The sequences of `block`.
pub(in crate::app) fn block_sequences(
    definition: &BitmapDefinition,
    block: SequenceBlock,
) -> &[BitmapSequence] {
    match block {
        SequenceBlock::Processed => &definition.sequences,
        SequenceBlock::Manual => &definition.manual_sequences,
    }
}

fn selection_exists(definition: &BitmapDefinition, selection: SpriteSelection) -> bool {
    block_sequences(definition, selection.block)
        .get(selection.sequence)
        .is_some_and(|sequence| selection.frame.is_none_or(|frame| frame < sequence.frame_count()))
}

/// The selected sprite, where a sprite is selected.
pub(in crate::app) fn selected_sprite(
    definition: &BitmapDefinition,
    selection: Option<SpriteSelection>,
) -> Option<&BitmapSprite> {
    let selection = selection?;
    block_sequences(definition, selection.block)
        .get(selection.sequence)?
        .sprites
        .get(selection.frame?)
}

/// The image a selection shows: a frame's own; for a whole sequence, the
/// shown one if any of its sprites is on it, else its first frame's.
pub(in crate::app) fn selection_image(
    definition: &BitmapDefinition,
    selection: SpriteSelection,
    shown: usize,
) -> Option<usize> {
    let sequence = block_sequences(definition, selection.block).get(selection.sequence)?;
    if selection.block == SequenceBlock::Manual {
        // Manual windows lie on their sprite's image, wrapped.
        let sprite = sequence.sprites.get(selection.frame.unwrap_or(0))?;
        return Some(sprite.bitmap_index.max(0) as usize);
    }
    match selection.frame {
        Some(frame) => Some(sequence.frame_bitmap(frame)),
        None if sequence.sprites.iter().any(|sprite| sprite.bitmap_index as usize == shown) => {
            Some(shown)
        }
        None => (sequence.frame_count() > 0).then(|| sequence.frame_bitmap(0)),
    }
}

/// Selects `selection`, moving to the image it shows when that is another;
/// the preview decodes it and shows the image before until it lands.
pub(in crate::app) fn apply_selection(
    preview: &mut BitmapPreviewState,
    definition: &BitmapDefinition,
    selection: Option<SpriteSelection>,
    image_count: usize,
) {
    preview.sprite_selection = selection;
    let Some(selection) = selection else {
        preview.sprite_playback = None;
        return;
    };
    let Some(image) = selection_image(definition, selection, preview.image_index) else {
        return;
    };
    if image < image_count {
        preview.image_index = image;
    }
}

/// The frame a playing sequence is on at `now`, and how long until the next.
pub(in crate::app) fn playback_frame(
    playback: SpritePlayback,
    fps: u32,
    frame_count: usize,
    now: f64,
) -> (usize, f64) {
    let period = 1.0 / f64::from(fps.max(1));
    let elapsed = (now - playback.started).max(0.0);
    let steps = (elapsed / period).floor();
    let frame = (playback.frame + steps as usize) % frame_count.max(1);
    (frame, period - (elapsed - steps * period))
}

/// Where the selected sequence's playback has got to, if it is playing: the
/// selection to show now. Asks for a repaint at the next frame.
pub(in crate::app) fn playback_selection(
    ui: &Ui,
    preview: &BitmapPreviewState,
    definition: &BitmapDefinition,
) -> Option<SpriteSelection> {
    let playback = preview.sprite_playback?;
    let selection = preview.sprite_selection?;
    let count = block_sequences(definition, selection.block)
        .get(selection.sequence)?
        .frame_count();
    if count == 0 {
        return None;
    }
    let now = ui.input(|input| input.time);
    let (frame, wait) = playback_frame(playback, preview.sprite_view.playback_fps, count, now);
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.001)));
    Some(SpriteSelection {
        frame: Some(frame),
        ..selection
    })
}

// ---------------------------------------------------------------------------
// Geometry: what the canvas draws, kept apart from drawing it.
// ---------------------------------------------------------------------------

/// `uv` across the image drawn at `image`.
pub(in crate::app) fn uv_on_screen(image: Rect, [u, v]: [f32; 2]) -> Pos2 {
    egui::pos2(image.left() + u * image.width(), image.top() + v * image.height())
}

/// A `[left, right, top, bottom]` rect across the image drawn at `image`,
/// the right way round even where it is mirrored.
pub(in crate::app) fn uv_rect_on_screen(image: Rect, [left, right, top, bottom]: [f32; 4]) -> Rect {
    Rect::from_two_pos(uv_on_screen(image, [left, top]), uv_on_screen(image, [right, bottom]))
}

/// A sprite's rect less `spacing` pixels on each side of an image `width`
/// by `height`, as the game's HUD and antennas take it; none where the tag
/// stores no spacing or nothing is left.
pub(in crate::app) fn content_rect(
    sprite: &BitmapSprite,
    spacing: i16,
    width: u16,
    height: u16,
) -> Option<[f32; 4]> {
    if spacing <= 0 || width == 0 || height == 0 {
        return None;
    }
    let (x, y) = (f32::from(spacing) / f32::from(width), f32::from(spacing) / f32::from(height));
    let [left, right, top, bottom] = sprite.rect();
    let inset = [left + x, right - x, top + y, bottom - y];
    (inset[1] > inset[0] && inset[3] > inset[2]).then_some(inset)
}

/// A repeat-space window folded back onto the image: up to four pieces,
/// where it crosses an edge, each `[left, right, top, bottom]` within
/// `[0, 1]`. A mirrored window covers the same pieces.
pub(in crate::app) fn wrapped_window(rect: [f32; 4]) -> Vec<[f32; 4]> {
    fn spans(a: f32, b: f32) -> Vec<[f32; 2]> {
        let (low, high) = (a.min(b), a.max(b));
        if !(high - low).is_finite() {
            return Vec::new();
        }
        if high - low >= 1.0 {
            return vec![[0.0, 1.0]];
        }
        let start = low - low.floor();
        let end = start + (high - low);
        if end <= 1.0 {
            vec![[start, end]]
        } else {
            vec![[start, 1.0], [0.0, end - 1.0]]
        }
    }
    let [left, right, top, bottom] = rect;
    let mut pieces = Vec::new();
    for [x0, x1] in spans(left, right) {
        for [y0, y1] in spans(top, bottom) {
            pieces.push([x0, x1, y0, y1]);
        }
    }
    pieces
}

/// The axes a window is mirrored on: `(x, y)`.
pub(in crate::app) fn mirrored(sprite: &BitmapSprite) -> (bool, bool) {
    (sprite.left > sprite.right, sprite.top > sprite.bottom)
}

/// The processed sprite under `uv` on image `image`: the smallest whose rect
/// holds it, as `(sequence, sprite)`.
pub(in crate::app) fn sprite_at(
    sequences: &[BitmapSequence],
    image: usize,
    [u, v]: [f32; 2],
) -> Option<(usize, usize)> {
    let mut best: Option<((usize, usize), f32)> = None;
    for (sequence_index, sequence) in sequences.iter().enumerate() {
        for (sprite_index, sprite) in sequence.sprites.iter().enumerate() {
            if sprite.bitmap_index as usize != image {
                continue;
            }
            let [left, right, top, bottom] = sprite.rect();
            let (x0, x1, y0, y1) = (left.min(right), left.max(right), top.min(bottom), top.max(bottom));
            if u < x0 || u > x1 || v < y0 || v > y1 {
                continue;
            }
            let area = (x1 - x0) * (y1 - y0);
            if best.is_none_or(|(_, smallest)| area < smallest) {
                best = Some(((sequence_index, sprite_index), area));
            }
        }
    }
    best.map(|(found, _)| found)
}

/// The zoom and pan that fit `rect` (a fraction of an image `image_size`
/// pixels) into a canvas `canvas` points across with a margin, centred. Pan
/// is the image centre's offset from the canvas centre, as the canvas keeps
/// it.
pub(in crate::app) fn frame_rect(image_size: Vec2, canvas: Vec2, rect: [f32; 4]) -> (f32, Vec2) {
    let [left, right, top, bottom] = rect;
    let size = Vec2::new((right - left).abs() * image_size.x, (bottom - top).abs() * image_size.y);
    let zoom = if size.x > 0.0 && size.y > 0.0 {
        (canvas.x / (size.x * 1.25)).min(canvas.y / (size.y * 1.25)).clamp(0.25, 32.0)
    } else {
        1.0
    };
    let centre = Vec2::new((left + right) * 0.5 * image_size.x, (top + bottom) * 0.5 * image_size.y);
    (zoom, image_size * zoom * 0.5 - centre * zoom)
}

/// The pan that puts `uv` of an image `image_size` pixels at the canvas
/// centre at `zoom`.
pub(in crate::app) fn pan_to(image_size: Vec2, zoom: f32, [u, v]: [f32; 2]) -> Vec2 {
    image_size * zoom * 0.5 - Vec2::new(u * image_size.x, v * image_size.y) * zoom
}

/// The selection's rect to frame: its sprite, or its sequence's sprites on
/// image `image` together.
pub(in crate::app) fn selection_rect(
    definition: &BitmapDefinition,
    selection: SpriteSelection,
    image: usize,
) -> Option<[f32; 4]> {
    let sequence = block_sequences(definition, selection.block).get(selection.sequence)?;
    let sprites: Vec<&BitmapSprite> = match selection.frame {
        Some(frame) => sequence.sprites.get(frame).into_iter().collect(),
        None => sequence
            .sprites
            .iter()
            .filter(|sprite| sprite.bitmap_index as usize == image)
            .collect(),
    };
    let rects: Vec<[f32; 4]> = sprites
        .iter()
        .flat_map(|sprite| match selection.block {
            SequenceBlock::Processed => vec![sprite.rect()],
            SequenceBlock::Manual => wrapped_window(sprite.rect()),
        })
        .collect();
    rects.into_iter().reduce(|a, b| {
        [
            a[0].min(a[1]).min(b[0].min(b[1])),
            a[0].max(a[1]).max(b[0].max(b[1])),
            a[2].min(a[3]).min(b[2].min(b[3])),
            a[2].max(a[3]).max(b[2].max(b[3])),
        ]
    })
}

// ---------------------------------------------------------------------------
// The canvas overlay.
// ---------------------------------------------------------------------------

/// What the canvas needs to draw over the image and pick on it.
pub(in crate::app) struct SpriteCanvas<'a> {
    pub(in crate::app) definition: &'a BitmapDefinition,
    /// The shown image.
    pub(in crate::app) image: usize,
}

/// Selected-sprite amber, the same on every background.
const SELECTED: Color32 = Color32::from_rgb(255, 190, 40);
/// The content rect and the image's registration point.
const SECONDARY: Color32 = Color32::from_rgb(70, 200, 255);

fn halo(bg: BitmapPreviewBg) -> Color32 {
    if bg == BitmapPreviewBg::White {
        Color32::from_white_alpha(200)
    } else {
        Color32::from_black_alpha(200)
    }
}

fn faint(bg: BitmapPreviewBg, alpha: u8) -> Color32 {
    if bg == BitmapPreviewBg::White {
        Color32::from_black_alpha(alpha)
    } else {
        Color32::from_white_alpha(alpha)
    }
}

fn dashed_rect(painter: &egui::Painter, rect: Rect, color: Color32, bg: BitmapPreviewBg) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.add(egui::Shape::line(corners.to_vec(), Stroke::new(3.0_f32, halo(bg))));
    painter.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.5_f32, color), 5.0, 3.0));
}

fn cross(painter: &egui::Painter, at: Pos2, color: Color32, bg: BitmapPreviewBg) {
    for (width, stroke_color) in [(3.0_f32, halo(bg)), (1.5, color)] {
        let stroke = Stroke::new(width, stroke_color);
        painter.line_segment([at - egui::vec2(7.0, 0.0), at + egui::vec2(7.0, 0.0)], stroke);
        painter.line_segment([at - egui::vec2(0.0, 7.0), at + egui::vec2(0.0, 7.0)], stroke);
    }
    painter.circle_stroke(at, 3.0, Stroke::new(1.5_f32, color));
}

fn diamond(painter: &egui::Painter, at: Pos2, color: Color32, bg: BitmapPreviewBg) {
    let points = vec![
        at + egui::vec2(0.0, -6.0),
        at + egui::vec2(6.0, 0.0),
        at + egui::vec2(0.0, 6.0),
        at + egui::vec2(-6.0, 0.0),
    ];
    painter.add(egui::Shape::closed_line(points.clone(), Stroke::new(3.0_f32, halo(bg))));
    painter.add(egui::Shape::closed_line(points, Stroke::new(1.5_f32, color)));
}

/// Draws the overlay over the image drawn at `image_rect`, clipped to the
/// canvas, and says which sprite the pointer at `hover` is over.
pub(in crate::app) fn paint_sprite_overlay(
    painter: &egui::Painter,
    image_rect: Rect,
    canvas: &SpriteCanvas<'_>,
    preview: &BitmapPreviewState,
    hover: Option<Pos2>,
) -> Option<(usize, usize)> {
    let definition = canvas.definition;
    let view = preview.sprite_view;
    let bg = preview.bg;
    let hovered = hover.and_then(|pointer| {
        let uv = (pointer - image_rect.min) / image_rect.size();
        sprite_at(&definition.sequences, canvas.image, [uv.x, uv.y])
    });
    let selection = preview.sprite_selection;
    let processed_selection = selection.filter(|selection| selection.block == SequenceBlock::Processed);

    for (sequence_index, sequence) in definition.sequences.iter().enumerate() {
        for (sprite_index, sprite) in sequence.sprites.iter().enumerate() {
            if sprite.bitmap_index as usize != canvas.image {
                continue;
            }
            let rect = uv_rect_on_screen(image_rect, sprite.rect());
            let in_selection = processed_selection.is_some_and(|selection| {
                selection.sequence == sequence_index
                    && selection.frame.is_none_or(|frame| frame == sprite_index)
            });
            if in_selection {
                continue;
            }
            if hovered == Some((sequence_index, sprite_index)) {
                painter.rect_stroke(rect, 0.0, Stroke::new(1.5_f32, faint(bg, 220)), egui::StrokeKind::Middle);
            } else if view.show_all {
                painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, faint(bg, 90)), egui::StrokeKind::Middle);
            }
        }
    }

    let Some(selection) = selection else {
        return hovered;
    };
    let Some(sequence) = block_sequences(definition, selection.block).get(selection.sequence) else {
        return hovered;
    };
    let chosen: Vec<&BitmapSprite> = match selection.frame {
        Some(frame) => sequence.sprites.get(frame).into_iter().collect(),
        None => sequence.sprites.iter().collect(),
    };
    for sprite in chosen {
        if sprite.bitmap_index as usize != canvas.image {
            continue;
        }
        match selection.block {
            SequenceBlock::Processed => {
                dashed_rect(painter, uv_rect_on_screen(image_rect, sprite.rect()), SELECTED, bg);
                if selection.frame.is_none() {
                    continue;
                }
                if view.show_content {
                    let image = definition.images.get(canvas.image);
                    if let Some(inset) = image.and_then(|image| {
                        content_rect(sprite, definition.sprite_spacing, image.width, image.height)
                    }) {
                        painter.rect_stroke(
                            uv_rect_on_screen(image_rect, inset),
                            0.0,
                            Stroke::new(1.0_f32, SECONDARY),
                            egui::StrokeKind::Middle,
                        );
                    }
                }
                if view.show_registration {
                    cross(painter, uv_on_screen(image_rect, sprite.registration_uv()), SELECTED, bg);
                }
            }
            SequenceBlock::Manual => {
                for piece in wrapped_window(sprite.rect()) {
                    dashed_rect(painter, uv_rect_on_screen(image_rect, piece), SELECTED, bg);
                }
            }
        }
    }
    hovered
}

/// Marks the shown image's own `registration point`, in pixels, where it
/// means something: not on a cube map, whose generated ones hold garbage,
/// and not at the origin H3 and later write when it was never set.
pub(in crate::app) fn paint_image_registration(
    painter: &egui::Painter,
    image_rect: Rect,
    definition: &BitmapDefinition,
    image: usize,
    bg: BitmapPreviewBg,
) {
    let Some(image_def) = definition.images.get(image) else {
        return;
    };
    let point = image_def.registration_point;
    let is_cube = image_def.bitmap_type.get() == BitmapType::CubeMap;
    if is_cube || (point.x == 0 && point.y == 0) || image_def.width == 0 || image_def.height == 0 {
        return;
    }
    let uv = [
        f32::from(point.x) / f32::from(image_def.width),
        f32::from(point.y) / f32::from(image_def.height),
    ];
    diamond(painter, uv_on_screen(image_rect, uv), SECONDARY, bg);
}

/// The footer's words for the selection on an image `width` by `height`.
pub(in crate::app) fn selection_summary(
    definition: &BitmapDefinition,
    selection: SpriteSelection,
    width: u32,
    height: u32,
) -> Option<String> {
    let sequence = block_sequences(definition, selection.block).get(selection.sequence)?;
    let block = match selection.block {
        SequenceBlock::Processed => "Sequence",
        SequenceBlock::Manual => "Manual sequence",
    };
    let Some(frame) = selection.frame else {
        return Some(format!("{block} {}", selection.sequence));
    };
    let Some(sprite) = sequence.sprites.get(frame) else {
        return Some(format!(
            "{block} {}  ·  frame {frame}  ·  image {}",
            selection.sequence,
            sequence.frame_bitmap(frame)
        ));
    };
    if selection.block == SequenceBlock::Manual {
        let flips = match mirrored(sprite) {
            (true, true) => "  ·  mirrored on both axes",
            (true, false) => "  ·  mirrored across",
            (false, true) => "  ·  mirrored down",
            (false, false) => "",
        };
        return Some(format!(
            "{block} {}  ·  window {frame}  ·  {:.3}–{:.3} × {:.3}–{:.3}{flips}",
            selection.sequence, sprite.left, sprite.right, sprite.top, sprite.bottom
        ));
    }
    let [left, right, top, bottom] = sprite.pixel_rect(width, height);
    let [x, y] = sprite.registration_uv();
    Some(format!(
        "{block} {}  ·  sprite {frame}  ·  {} × {} at ({}, {})  ·  registration ({}, {})  ·  image {}",
        selection.sequence,
        pixels(right - left),
        pixels(bottom - top),
        pixels(left),
        pixels(top),
        pixels(x * width as f32),
        pixels(y * height as f32),
        sprite.bitmap_index
    ))
}

/// A pixel measure: whole where it is whole, else to a tenth.
fn pixels(value: f32) -> String {
    if (value - value.round()).abs() < 0.01 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

// ---------------------------------------------------------------------------
// The sidebar.
// ---------------------------------------------------------------------------

/// Splits the page for the preview and its Sequences sidebar, drawing the
/// handle between them; the canvas card's rect and the sidebar's.
pub(in crate::app) fn split_bitmap_page(ui: &mut Ui, preview: &mut BitmapPreviewState) -> (Rect, Rect) {
    let size = Vec2::new(ui.available_width().max(1.0), ui.available_height().max(1.0));
    let (page, _) = ui.allocate_exact_size(size, Sense::hover());
    let split = split_sidebar(page, preview.sidebar_width, SIDEBAR_LIMITS);
    draw_sidebar_resize_handle(
        ui,
        &split,
        page.width(),
        &mut preview.sidebar_width,
        SIDEBAR_LIMITS,
        "bitmap_sequences_resize",
    );
    (split.main, split.sidebar)
}

/// The Sequences sidebar in `rect`: its cards scroll together. A selection
/// picked in it is left in `picked`.
pub(in crate::app) fn draw_sequence_sidebar(
    ui: &mut Ui,
    rect: Rect,
    preview: &mut BitmapPreviewState,
    definition: &BitmapDefinition,
    picked: &mut Option<Option<SpriteSelection>>,
) {
    in_rect(ui, rect, "bitmap_sequences_sidebar", |ui| {
        let gutter = sidebar_scroll_gutter(ui);
        crate::app::ScrollArea::vertical()
            .id_salt("bitmap_sequences_body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        right: gutter as i8,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        ui.add_space(-(8.0 - ui.spacing().item_spacing.y).max(0.0));
                        if !definition.sequences.is_empty() {
                            draw_sequence_card(ui, preview, definition, SequenceBlock::Processed, picked);
                        }
                        if !definition.manual_sequences.is_empty() {
                            draw_sequence_card(ui, preview, definition, SequenceBlock::Manual, picked);
                        }
                        preview.scroll_to_selection = false;
                    });
            });
    });
}

fn draw_sequence_card(
    ui: &mut Ui,
    preview: &mut BitmapPreviewState,
    definition: &BitmapDefinition,
    block: SequenceBlock,
    picked: &mut Option<Option<SpriteSelection>>,
) {
    let (title, salt) = match block {
        SequenceBlock::Processed => ("Sequences", "bitmap_sequences_card"),
        SequenceBlock::Manual => ("Manual Sequences", "bitmap_manual_sequences_card"),
    };
    let sequences = block_sequences(definition, block);
    let selected_here = preview.sprite_selection.filter(|selection| selection.block == block);
    let reset = draw_resettable_box(ui, title, salt, selected_here.is_some(), |ui, part| match part {
        BoxPart::Controls => {
            if block == SequenceBlock::Processed {
                draw_playback_controls(ui, preview, definition);
            }
        }
        BoxPart::Trailing => {}
        BoxPart::Body => {
            if block == SequenceBlock::Manual {
                ui.label(
                    RichText::new(
                        "Authored input that tool does not read. Decals pick from these, and GPU \
                         particles take their frames from them; the windows repeat across the image.",
                    )
                    .small()
                    .color(subtle_dark()),
                );
                ui.add_space(4.0);
            }
            if block == SequenceBlock::Processed && sequences.len() > FILTER_FROM {
                ui.add(
                    egui::TextEdit::singleline(&mut preview.sequence_filter)
                        .hint_text(placeholder_text("Filter by index or name"))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
            }
            let filter = preview.sequence_filter.trim().to_lowercase();
            for (index, sequence) in sequences.iter().enumerate() {
                if block == SequenceBlock::Processed
                    && !filter.is_empty()
                    && !sequence_matches(index, sequence, &filter)
                {
                    continue;
                }
                draw_sequence_row(
                    ui,
                    preview.sprite_selection,
                    preview.scroll_to_selection,
                    block,
                    index,
                    sequence,
                    definition,
                    picked,
                );
            }
        }
    });
    if reset {
        *picked = Some(None);
    }
}

fn sequence_matches(index: usize, sequence: &BitmapSequence, filter: &str) -> bool {
    index.to_string().starts_with(filter)
        || sequence
            .display_name()
            .is_some_and(|name| name.to_lowercase().contains(filter))
}

/// The pages a sequence's sprites lie on, for a sheet spread over more than
/// one: "image 2", "images 0–7".
fn sequence_pages(sequence: &BitmapSequence) -> Option<String> {
    let low = sequence.sprites.iter().map(|sprite| sprite.bitmap_index).min()?;
    let high = sequence.sprites.iter().map(|sprite| sprite.bitmap_index).max()?;
    Some(if low == high {
        format!("image {low}")
    } else {
        format!("images {low}–{high}")
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_sequence_row(
    ui: &mut Ui,
    selection: Option<SpriteSelection>,
    scroll_to: bool,
    block: SequenceBlock,
    index: usize,
    sequence: &BitmapSequence,
    definition: &BitmapDefinition,
    picked: &mut Option<Option<SpriteSelection>>,
) {
    let is_selected = selection.is_some_and(|selection| selection.block == block && selection.sequence == index);
    let frames = sequence.frame_count();
    let noun = match (block, sequence.sprites.is_empty()) {
        (SequenceBlock::Manual, _) => "window",
        (_, false) => "sprite",
        (_, true) => "frame",
    };
    let mut label = format!("{index}");
    if let Some(name) = sequence.display_name() {
        label.push_str(&format!("  {name}"));
    }
    label.push_str(&format!("  ·  {frames} {noun}{}", if frames == 1 { "" } else { "s" }));
    if block == SequenceBlock::Processed
        && definition.images.len() > 1
        && let Some(pages) = sequence_pages(sequence)
    {
        label.push_str(&format!("  ·  {pages}"));
    }
    let whole = is_selected && selection.is_some_and(|selection| selection.frame.is_none());
    let row = ui.add_enabled(
        frames > 0,
        egui::Button::selectable(whole, label).min_size(Vec2::new(ui.available_width(), 0.0)),
    );
    if whole && scroll_to {
        row.scroll_to_me(Some(egui::Align::Center));
    }
    if row.clicked() {
        *picked = Some(Some(SpriteSelection {
            block,
            sequence: index,
            frame: None,
        }));
    }
    if !is_selected {
        return;
    }
    // The selected sequence lists its frames beneath it.
    ui.indent(("bitmap_sequence_frames", block == SequenceBlock::Manual, index), |ui| {
        for frame in 0..frames {
            let chosen = selection.is_some_and(|selection| selection.frame == Some(frame));
            let text = match sequence.sprites.get(frame) {
                Some(sprite) if block == SequenceBlock::Manual => {
                    let flips = match mirrored(sprite) {
                        (false, false) => "",
                        _ => "  ·  mirrored",
                    };
                    format!("Window {frame}{flips}")
                }
                Some(sprite) => {
                    let image = definition.images.get(sprite.bitmap_index.max(0) as usize);
                    let size = image.map_or(String::new(), |image| {
                        let [left, right, top, bottom] =
                            sprite.pixel_rect(u32::from(image.width), u32::from(image.height));
                        format!("  ·  {} × {}", pixels(right - left), pixels(bottom - top))
                    });
                    let page = if definition.images.len() > 1 {
                        format!("  ·  image {}", sprite.bitmap_index)
                    } else {
                        String::new()
                    };
                    format!("Sprite {frame}{size}{page}")
                }
                None => format!("Frame {frame}  ·  image {}", sequence.frame_bitmap(frame)),
            };
            let row = ui.add(egui::Button::selectable(chosen, text).min_size(Vec2::new(ui.available_width(), 0.0)));
            if chosen && scroll_to {
                row.scroll_to_me(Some(egui::Align::Center));
            }
            if row.clicked() {
                *picked = Some(Some(SpriteSelection {
                    block,
                    sequence: index,
                    frame: Some(frame),
                }));
            }
        }
    });
}

/// Play and Pause for the selected sequence, and the rate it plays at.
fn draw_playback_controls(ui: &mut Ui, preview: &mut BitmapPreviewState, definition: &BitmapDefinition) {
    let selection = preview
        .sprite_selection
        .filter(|selection| selection.block == SequenceBlock::Processed);
    let playable = selection.is_some_and(|selection| {
        definition
            .sequences
            .get(selection.sequence)
            .is_some_and(|sequence| sequence.frame_count() > 1)
    });
    ui.spacing_mut().item_spacing.x = 4.0;
    let playing = preview.sprite_playback.is_some();
    if selectable_icon_button(ui, ButtonIcon::Play, "Play the selected sequence", playing, playable)
        .clicked()
        && let Some(selection) = selection
    {
        // Playing takes over the selection from here; it is not a pick.
        let frame = selection.frame.unwrap_or(0);
        preview.sprite_playback = Some(SpritePlayback {
            started: ui.input(|input| input.time),
            frame,
        });
        preview.sprite_selection = Some(SpriteSelection {
            frame: Some(frame),
            ..selection
        });
    }
    if selectable_icon_button(ui, ButtonIcon::Pause, "Pause", false, playing).clicked() {
        preview.sprite_playback = None;
    }
    let mut fps = preview.sprite_view.playback_fps;
    ui.add_sized(
        Vec2::new(56.0, BUTTON_HEIGHT),
        egui::DragValue::new(&mut fps)
            .range(1..=MAX_SPRITE_PLAYBACK_FPS)
            .suffix(" fps"),
    )
    .on_hover_text("The rate the preview plays at. A bitmap holds no rate: whatever draws it sets one.");
    if fps != preview.sprite_view.playback_fps {
        // Keep the frame playing now, from now on, at the new rate.
        if let (Some(playback), Some(selection)) = (preview.sprite_playback, selection)
            && let Some(sequence) = definition.sequences.get(selection.sequence)
        {
            let now = ui.input(|input| input.time);
            let (frame, _) = playback_frame(playback, preview.sprite_view.playback_fps, sequence.frame_count(), now);
            preview.sprite_playback = Some(SpritePlayback { started: now, frame });
        }
        preview.sprite_view.playback_fps = fps;
    }
}

/// The View menu's sprite options.
pub(in crate::app) fn draw_sprite_view_options(ui: &mut Ui, view: &mut SpriteOverlaySettings) {
    ui.label(RichText::new("Sprites").strong().color(text_dark()));
    ui.baboon_checkbox(&mut view.show_all, "Outline Every Sprite")
        .on_hover_text("A faint outline round every sprite on the shown image.");
    ui.baboon_checkbox(&mut view.show_registration, "Show Registration Point")
        .on_hover_text("A cross where the selected sprite hangs from: its rect's top-left plus the stored offset.");
    ui.baboon_checkbox(&mut view.show_content, "Show Spacing Inset")
        .on_hover_text(
            "The selected sprite less the tag's sprite spacing on each side, as the HUD and antennas \
             read it. Tool includes the spacing in every sprite's rect.",
        );
    ui.baboon_checkbox(&mut view.show_image_registration, "Show Image Registration Point")
        .on_hover_text("A diamond at the shown image's own registration point, in pixels. Not drawn for cube maps.");
    ui.baboon_checkbox(&mut view.pin_registration, "Hold Still While Playing")
        .on_hover_text("Keep the playing sequence's registration point at the middle of the canvas, as the game draws it.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(bitmap_index: i16, rect: [f32; 4], registration: [f32; 2]) -> BitmapSprite {
        let [left, right, top, bottom] = rect;
        BitmapSprite {
            bitmap_index,
            left,
            right,
            top,
            bottom,
            registration_point: registration,
        }
    }

    fn sequence(sprites: Vec<BitmapSprite>) -> BitmapSequence {
        BitmapSequence {
            sprites,
            ..Default::default()
        }
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn a_rect_lands_on_the_image_wherever_it_is_drawn() {
        let image = Rect::from_min_size(egui::pos2(100.0, 50.0), Vec2::new(400.0, 200.0));
        assert_eq!(
            uv_rect_on_screen(image, [0.25, 0.5, 0.5, 1.0]),
            Rect::from_min_max(egui::pos2(200.0, 150.0), egui::pos2(300.0, 250.0))
        );
        // Mirrored, it covers the same ground.
        assert_eq!(
            uv_rect_on_screen(image, [0.5, 0.25, 1.0, 0.5]),
            uv_rect_on_screen(image, [0.25, 0.5, 0.5, 1.0])
        );
        // A rect overhanging the image by its spacing reaches past its edge.
        assert!(uv_rect_on_screen(image, [-0.01, 0.5, 0.0, 1.0]).left() < image.left());
    }

    #[test]
    fn the_spacing_inset_takes_the_stored_spacing_off_each_side() {
        let wide = sprite(0, [0.0, 0.5, 0.25, 0.75], [0.0; 2]);
        assert!(close(
            content_rect(&wide, 4, 256, 128).unwrap(),
            [4.0 / 256.0, 0.5 - 4.0 / 256.0, 0.25 + 4.0 / 128.0, 0.75 - 4.0 / 128.0]
        ));
        assert_eq!(content_rect(&wide, 0, 256, 128), None);
        assert_eq!(content_rect(&wide, -1, 256, 128), None);
        // Nothing left inside.
        let thin = sprite(0, [0.0, 0.02, 0.0, 1.0], [0.0; 2]);
        assert_eq!(content_rect(&thin, 4, 256, 128), None);
    }

    #[test]
    fn a_window_inside_one_repeat_folds_to_one_piece() {
        assert!(close(
            wrapped_window([5.25, 5.5, 3.125, 3.375])[0],
            [0.25, 0.5, 0.125, 0.375]
        ));
        assert_eq!(wrapped_window([5.25, 5.5, 3.125, 3.375]).len(), 1);
    }

    #[test]
    fn a_window_across_a_corner_folds_to_four_pieces() {
        let pieces = wrapped_window([5.75, 6.25, 0.875, 1.125]);
        assert_eq!(pieces.len(), 4);
        assert!(close(pieces[0], [0.75, 1.0, 0.875, 1.0]));
        assert!(close(pieces[3], [0.0, 0.25, 0.0, 0.125]));
        // Mirrored, the same pieces.
        assert_eq!(wrapped_window([6.25, 5.75, 1.125, 0.875]), pieces);
        // A whole repeat or more covers the image.
        assert_eq!(wrapped_window([0.2, 1.2, 0.4, 0.5]), vec![[0.0, 1.0, 0.4, 0.5]]);
    }

    #[test]
    fn a_click_picks_the_smallest_sprite_on_the_shown_image() {
        let sequences = vec![
            sequence(vec![sprite(0, [0.0, 1.0, 0.0, 1.0], [0.5; 2])]),
            sequence(vec![
                sprite(0, [0.0, 0.5, 0.0, 0.5], [0.25; 2]),
                sprite(1, [0.0, 0.25, 0.0, 0.25], [0.1; 2]),
            ]),
        ];
        assert_eq!(sprite_at(&sequences, 0, [0.1, 0.1]), Some((1, 0)));
        assert_eq!(sprite_at(&sequences, 0, [0.75, 0.75]), Some((0, 0)));
        // The smaller one is on another image, so it is not picked here.
        assert_eq!(sprite_at(&sequences, 1, [0.1, 0.1]), Some((1, 1)));
        assert_eq!(sprite_at(&sequences, 1, [0.75, 0.75]), None);
    }

    #[test]
    fn framing_centres_the_rect_and_fits_it() {
        let image = Vec2::new(512.0, 256.0);
        let (zoom, pan) = frame_rect(image, Vec2::new(400.0, 400.0), [0.5, 0.75, 0.5, 1.0]);
        // 128 × 128 pixels with a quarter more room: 400 / 160.
        assert!((zoom - 2.5).abs() < 1e-5);
        // The rect's middle, pixel (320, 192), lands on the canvas centre.
        let image_top_left = Vec2::ZERO + pan - image * zoom * 0.5;
        let centre = image_top_left + Vec2::new(320.0, 192.0) * zoom;
        assert!(centre.length() < 1e-3, "{centre:?}");
        let pinned = pan_to(image, 2.0, [0.25, 0.5]);
        let point = pinned - image * 2.0 * 0.5 + Vec2::new(128.0, 128.0) * 2.0;
        assert!(point.length() < 1e-3, "{point:?}");
    }

    #[test]
    fn playback_steps_at_its_rate_and_wraps() {
        let playback = SpritePlayback {
            started: 10.0,
            frame: 2,
        };
        assert_eq!(playback_frame(playback, 10, 5, 10.0).0, 2);
        assert_eq!(playback_frame(playback, 10, 5, 10.15).0, 3);
        assert_eq!(playback_frame(playback, 10, 5, 10.35).0, 0);
        assert_eq!(playback_frame(playback, 5, 5, 10.35).0, 3);
        let (_, wait) = playback_frame(playback, 10, 5, 10.13);
        assert!((wait - 0.07).abs() < 1e-6, "{wait}");
    }

    #[test]
    fn a_whole_sequence_stays_on_the_shown_image_when_it_has_sprites_there() {
        let definition = BitmapDefinition {
            sequences: vec![sequence(vec![
                sprite(2, [0.0, 0.5, 0.0, 0.5], [0.0; 2]),
                sprite(3, [0.0, 0.5, 0.0, 0.5], [0.0; 2]),
            ])],
            ..Default::default()
        };
        let whole = SpriteSelection {
            block: SequenceBlock::Processed,
            sequence: 0,
            frame: None,
        };
        assert_eq!(selection_image(&definition, whole, 3), Some(3));
        assert_eq!(selection_image(&definition, whole, 0), Some(2));
        let second = SpriteSelection {
            frame: Some(1),
            ..whole
        };
        assert_eq!(selection_image(&definition, second, 0), Some(3));
    }

    #[test]
    fn a_selection_an_edit_took_away_is_dropped() {
        let definition = BitmapDefinition {
            sequences: vec![sequence(vec![sprite(0, [0.0, 1.0, 0.0, 1.0], [0.0; 2])])],
            ..Default::default()
        };
        let in_range = SpriteSelection {
            block: SequenceBlock::Processed,
            sequence: 0,
            frame: Some(0),
        };
        assert!(selection_exists(&definition, in_range));
        assert!(!selection_exists(&definition, SpriteSelection { frame: Some(1), ..in_range }));
        assert!(!selection_exists(&definition, SpriteSelection { sequence: 1, ..in_range }));
        assert!(!selection_exists(
            &definition,
            SpriteSelection {
                block: SequenceBlock::Manual,
                ..in_range
            }
        ));
    }

    /// The bitmap preview drawn headless over a decoded image and its
    /// sequences, with a pointer that moves as a real one does.
    struct Canvas {
        ctx: egui::Context,
        time: f64,
        preview: BitmapPreviewState,
        definition: BitmapDefinition,
        image_count: usize,
        /// The canvas's rect, from the last frame.
        canvas: Rect,
        pointer: Pos2,
    }

    impl Canvas {
        fn new(definition: BitmapDefinition, image_count: usize) -> Self {
            let mut canvas = Self {
                ctx: egui::Context::default(),
                time: 0.0,
                preview: BitmapPreviewState::default(),
                definition,
                image_count,
                canvas: Rect::NOTHING,
                pointer: Pos2::ZERO,
            };
            canvas.decoded();
            canvas.frame(Vec::new());
            canvas.frame(Vec::new());
            canvas
        }

        /// A 256 × 256 decode of the shown image lands.
        fn decoded(&mut self) {
            self.preview.decoded = Some(Ok(BitmapPreviewData {
                width: 256,
                height: 256,
                image_count: self.image_count,
                mip_count: 1,
                format_name: "a8r8g8b8".to_owned(),
                type_name: "2D texture".to_owned(),
                rgba: vec![128; 256 * 256 * 4],
            }));
            self.preview.decoded_for = Some((self.preview.image_index, self.preview.mip_index));
            self.preview.texture_dirty = true;
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 1.0 / 60.0;
            let (ctx, preview, definition) = (self.ctx.clone(), &mut self.preview, &self.definition);
            let output = crate::app::run_ui_test(
                &self.ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        draw_bitmap_preview_data(ui, &ctx, "test", preview, true, "Bitmap", Some(definition));
                    });
                },
            );
            let fill = self.preview.bg.color();
            if let Some(canvas) = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(rect) if rect.fill == fill => Some(rect.rect),
                    _ => None,
                })
                .max_by(|a, b| a.area().total_cmp(&b.area()))
            {
                self.canvas = canvas;
            }
        }

        /// Where `uv` of the shown image is on screen.
        fn at(&self, uv: [f32; 2]) -> Pos2 {
            let size = Vec2::splat(256.0) * self.preview.zoom;
            let image = Rect::from_center_size(self.canvas.center() + self.preview.pan, size);
            uv_on_screen(image, uv)
        }

        /// The pointer slides to `to` over three frames.
        fn slide(&mut self, to: Pos2) {
            let from = self.pointer;
            for step in 1..=3 {
                let at = from + (to - from) * (step as f32 / 3.0);
                self.frame(vec![egui::Event::PointerMoved(at)]);
            }
            self.pointer = to;
        }

        fn button(&mut self, pressed: bool) {
            self.frame(vec![egui::Event::PointerButton {
                pos: self.pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }]);
        }

        fn click(&mut self, at: Pos2) {
            self.slide(at);
            self.button(true);
            self.button(false);
            self.frame(Vec::new());
        }
    }

    fn two_quarters() -> BitmapDefinition {
        BitmapDefinition {
            sequences: vec![
                sequence(vec![sprite(0, [0.0, 0.5, 0.0, 0.5], [0.25, 0.25])]),
                sequence(vec![
                    sprite(0, [0.5, 1.0, 0.5, 1.0], [0.25, 0.25]),
                    sprite(1, [0.0, 0.5, 0.0, 0.5], [0.25, 0.25]),
                ]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn clicking_a_sprite_on_the_canvas_selects_it() {
        let mut canvas = Canvas::new(two_quarters(), 2);
        assert!(canvas.canvas.is_positive(), "no canvas drawn");
        canvas.click(canvas.at([0.75, 0.75]));
        assert_eq!(
            canvas.preview.sprite_selection,
            Some(SpriteSelection {
                block: SequenceBlock::Processed,
                sequence: 1,
                frame: Some(0),
            })
        );
        canvas.click(canvas.at([0.25, 0.25]));
        assert_eq!(
            canvas.preview.sprite_selection.map(|selection| (selection.sequence, selection.frame)),
            Some((0, Some(0)))
        );
    }

    #[test]
    fn dragging_across_a_sprite_pans_and_selects_nothing() {
        let mut canvas = Canvas::new(two_quarters(), 2);
        let start = canvas.at([0.75, 0.75]);
        canvas.slide(start);
        canvas.button(true);
        canvas.slide(start + Vec2::new(60.0, 40.0));
        canvas.button(false);
        canvas.frame(Vec::new());
        assert_eq!(canvas.preview.sprite_selection, None);
    }

    #[test]
    fn a_sprite_on_another_image_moves_there_and_stays_selected_through_the_decode() {
        let mut canvas = Canvas::new(two_quarters(), 2);
        apply_selection(
            &mut canvas.preview,
            &canvas.definition.clone(),
            Some(SpriteSelection {
                block: SequenceBlock::Processed,
                sequence: 1,
                frame: Some(1),
            }),
            2,
        );
        assert_eq!(canvas.preview.image_index, 1);
        // Image 0 stays on screen, and drawn for, until image 1 lands.
        canvas.frame(Vec::new());
        assert_eq!(canvas.preview.decoded_for, Some((0, 0)));
        assert!(canvas.preview.decoded.is_some());
        canvas.decoded();
        canvas.frame(Vec::new());
        assert_eq!(canvas.preview.image_index, 1);
        assert_eq!(
            canvas.preview.sprite_selection.map(|selection| (selection.sequence, selection.frame)),
            Some((1, Some(1)))
        );
        // On image 1 only that sprite is there: a click on the other quarter
        // finds nothing and leaves the selection be.
        canvas.click(canvas.at([0.75, 0.75]));
        assert_eq!(
            canvas.preview.sprite_selection.map(|selection| (selection.sequence, selection.frame)),
            Some((1, Some(1)))
        );
    }

    #[test]
    fn playing_steps_through_the_sequence_and_a_pick_stops_it() {
        let mut canvas = Canvas::new(two_quarters(), 2);
        canvas.preview.sprite_selection = Some(SpriteSelection {
            block: SequenceBlock::Processed,
            sequence: 1,
            frame: Some(0),
        });
        canvas.preview.sprite_view.playback_fps = 10;
        canvas.preview.sprite_playback = Some(SpritePlayback {
            started: canvas.time,
            frame: 0,
        });
        // A tenth of a second on, the second sprite, on the second image.
        for _ in 0..7 {
            canvas.frame(Vec::new());
        }
        assert_eq!(canvas.preview.sprite_selection.and_then(|selection| selection.frame), Some(1));
        assert_eq!(canvas.preview.image_index, 1);
        canvas.decoded();
        canvas.frame(Vec::new());
        canvas.click(canvas.at([0.25, 0.25]));
        assert_eq!(canvas.preview.sprite_playback, None);
        assert_eq!(
            canvas.preview.sprite_selection.map(|selection| (selection.sequence, selection.frame)),
            Some((1, Some(1)))
        );
    }

    /// With `BLAM_TEST_HCEEK`: a real sheet opens with its sequences, read
    /// once per revision; picking a sprite on its second page decodes that
    /// page and the footer measures it; and an edit that removes the selected
    /// sequence drops the selection.
    #[test]
    fn kit_sheet_selects_across_pages_and_forgets_an_edited_away_sequence() {
        let tags = crate::core::test_kits::hceek_tags();
        let path = tags.join("ui/hud/bitmaps/combined/hud_msg_icons.bitmap");
        if !path.is_file() {
            eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
            return;
        }
        let definitions = crate::core::bundled::locate_definitions_root();
        let mut tag = crate::core::source::read_tag_at_path(
            &path,
            Some(crate::core::game::GameId::HaloCe),
            Some(&definitions),
            u32::from_be_bytes(*b"bitm"),
        )
        .unwrap();
        let entry = crate::core::source::TagEntry {
            key: "file:hud_msg_icons".to_owned(),
            display_path: "ui\\hud\\bitmaps\\combined\\hud_msg_icons".to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: None,
            location: crate::core::source::TagEntryLocation::LooseFile(path.clone()),
        };
        let mut preview = BitmapPreviewState::default();
        let first = tag_sequences(&mut preview, &tag, (1, 0)).expect("a sprite sheet");
        assert_eq!(first.sequences.len(), 25);
        let again = tag_sequences(&mut preview, &tag, (1, 0)).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "read again at the same revision");

        let ctx = egui::Context::default();
        let mut texts = Vec::new();
        let frame = |preview: &mut BitmapPreviewState, tag: &TagFile, texts: &mut Vec<String>| {
            let output = crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 800.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        super::super::bitmap::draw_bitmap_preview(ui, &ctx, tag, &entry, preview, (1, 0));
                    });
                },
            );
            *texts = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        };
        let settle = |preview: &mut BitmapPreviewState, tag: &TagFile, texts: &mut Vec<String>| {
            let started = std::time::Instant::now();
            frame(preview, tag, texts);
            while preview.decoded_for != Some((preview.image_index, preview.mip_index)) {
                assert!(started.elapsed().as_secs() < 30, "the decode never landed");
                std::thread::sleep(std::time::Duration::from_millis(5));
                frame(preview, tag, texts);
            }
            frame(preview, tag, texts);
        };
        settle(&mut preview, &tag, &mut texts);
        assert_eq!(preview.image_index, 0);

        // Sequence 7's sprite is on the second page: 378 × 166 at the
        // origin of a 512 × 512 image, hanging from (189, 109).
        let definition = preview.sequences.as_ref().unwrap().1.clone();
        apply_selection(
            &mut preview,
            &definition,
            Some(SpriteSelection {
                block: SequenceBlock::Processed,
                sequence: 7,
                frame: Some(0),
            }),
            4,
        );
        assert_eq!(preview.image_index, 1);
        // The first page stays on screen while the second decodes.
        frame(&mut preview, &tag, &mut texts);
        assert!(preview.decoded.is_some(), "the panel blanked while the next page decoded");
        assert!(!texts.iter().any(|text| text == "Decoding…"), "{texts:?}");
        settle(&mut preview, &tag, &mut texts);
        assert!(
            texts.iter().any(|text| text.contains(
                "Sequence 7  ·  sprite 0  ·  378 × 166 at (0, 0)  ·  registration (189, 109)  ·  image 1"
            )),
            "{texts:?}"
        );

        // Remove the last sequence and select it first: the next revision
        // drops the selection with it.
        preview.sprite_selection = Some(SpriteSelection {
            block: SequenceBlock::Processed,
            sequence: 24,
            frame: Some(0),
        });
        let mut root = tag.root_mut();
        let mut field = root.field_mut("sequences").unwrap();
        field.as_block_mut().unwrap().delete_element(24).unwrap();
        let edited = tag_sequences(&mut preview, &tag, (1, 1)).unwrap();
        assert_eq!(edited.sequences.len(), 24);
        assert_eq!(preview.sprite_selection, None);
    }

    #[test]
    fn the_footer_measures_a_sprite_in_pixels() {
        let definition = BitmapDefinition {
            sequences: vec![sequence(vec![sprite(1, [0.0, 0.25, 0.5, 1.0], [0.125, 0.25])])],
            ..Default::default()
        };
        let selection = SpriteSelection {
            block: SequenceBlock::Processed,
            sequence: 0,
            frame: Some(0),
        };
        assert_eq!(
            selection_summary(&definition, selection, 512, 256).as_deref(),
            Some("Sequence 0  ·  sprite 0  ·  128 × 128 at (0, 128)  ·  registration (64, 192)  ·  image 1")
        );
    }
}
