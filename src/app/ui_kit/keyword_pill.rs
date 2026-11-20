//! Keyword pills: a keyword's label, in the colour picked for it in Manage
//! Keywords, wherever keywords show.

use super::*;
use crate::core::keyword_catalog::KeywordColour;

const PADDING: f32 = 8.0;
const DOT: f32 = 8.0;
const DOT_LEFT: f32 = 6.0;
const DOT_GAP: f32 = 5.0;

/// Where a pill's text starts, from its left edge: after its dot. Every
/// pill has one, the uncoloured (grey) one too.
fn text_left(_: Option<KeywordColour>) -> f32 {
    DOT_LEFT + DOT + DOT_GAP
}

/// The width a pill needs for `label` in `font`. `trailing` leaves room for
/// a button inside its right end; 0 gives the usual padding.
pub(in crate::app) fn keyword_pill_width(
    painter: &egui::Painter,
    label: &str,
    font: &FontId,
    colour: Option<KeywordColour>,
    trailing: f32,
) -> f32 {
    let text = painter.layout_no_wrap(label.to_owned(), font.clone(), text_dark()).size().x;
    text_left(colour) + text + if trailing > 0.0 { trailing } else { PADDING }
}

/// Paint a keyword pill filling `rect`: fill, border and dot from its
/// colour, then its label.
pub(in crate::app) fn paint_keyword_pill(
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    font: &FontId,
    colour: Option<KeywordColour>,
) {
    let colours = keyword_pill_colours(colour);
    let radius = egui::CornerRadius::same((rect.height() / 2.0).round() as u8);
    painter.rect_filled(rect, radius, colours.fill);
    if let Some(border) = colours.border {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
    if let Some(dot) = colours.dot {
        let center = egui::pos2(rect.left() + DOT_LEFT + DOT / 2.0, rect.center().y);
        painter.circle(center, DOT / 2.0, dot, Stroke::new(1.0, colours.border.unwrap_or(dot)));
    }
    let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), colours.text);
    let at = egui::pos2(
        rect.left() + text_left(colour),
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(at, galley, colours.text);
}
