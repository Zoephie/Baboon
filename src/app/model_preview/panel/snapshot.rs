//! The preview header's snapshot: the camera, which copies the frame on
//! screen to the clipboard without its background, and its scale menu
//! (`model_preview::snapshot`).

use super::super::super::ui_kit::button_icons::{ButtonIcon, split_icon_dropdown_button};
use super::super::super::ui_kit::style::{subtle_dark, text_dark};
use super::super::snapshot::{LARGE_SNAPSHOT_BYTES, SnapshotPhase, snapshot_bytes, snapshot_size};
use super::super::ModelPreviewState;
use eframe::egui::{self, RichText, Ui};

/// The slider's reach; the number box takes any scale above 0.
pub(in crate::app::model_preview) const SLIDER_SCALES: std::ops::RangeInclusive<f32> = 0.1..=10.0;

/// The snapshot's split button: the camera with the scale on its main half,
/// which copies the frame on screen, and a chevron that opens the scale menu.
pub(in crate::app::model_preview) fn draw_snapshot_controls(ui: &mut Ui, state: &mut ModelPreviewState) {
    let size = state
        .snapshot_source
        .as_ref()
        .map(|source| snapshot_size(source.points, source.pixels_per_point, state.snapshot_scale));
    let busy = state.snapshot.busy();
    let ready = size.is_some() && !busy;
    let label = scale_label(state.snapshot_scale);
    let (response, chevron, _) = split_icon_dropdown_button(ui, ButtonIcon::Camera, &label, ready, |ui| {
        draw_scale_menu(ui, state);
    });
    let chevron = chevron.on_hover_text("The snapshot's size, as a scale of the viewport's");
    // Icons paint no text for a test to find them by.
    #[cfg(test)]
    ui.ctx().data_mut(|data| data.insert_temp(snapshot_chevron_id(), chevron.rect));
    #[cfg(not(test))]
    let _ = chevron;
    let tooltip = match size {
        Some([width, height]) => format!("Copy the frame to the clipboard, without its background ({width} × {height})"),
        None => "Copy the frame to the clipboard, without its background".to_owned(),
    };
    let response = if busy {
        response.on_disabled_hover_text("Copying the last snapshot…")
    } else if size.is_none() {
        response.on_disabled_hover_text("The preview has drawn nothing to copy yet")
    } else {
        response.on_hover_text(tooltip)
    };
    if response.clicked()
        && let Some(size) = size
    {
        // A very large image asks first: it takes gigabytes on its way.
        let confirmed = matches!(state.snapshot, SnapshotPhase::Confirming(asked) if asked == size);
        if snapshot_bytes(size) > LARGE_SNAPSHOT_BYTES && !confirmed {
            state.snapshot = SnapshotPhase::Confirming(size);
            state.snapshot_message = Some(format!(
                "A {} × {} snapshot is {}: press the camera again to copy it",
                size[0],
                size[1],
                gigabytes(snapshot_bytes(size))
            ));
        } else {
            state.snapshot = SnapshotPhase::Asked;
        }
    }
}

/// `bytes` as gigabytes, to one place.
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Where the snapshot's chevron was last drawn, for a test to press it.
#[cfg(test)]
pub(in crate::app) fn snapshot_chevron_id() -> egui::Id {
    egui::Id::unique("snapshot_chevron")
}

/// "1×", "2.5×", "0.25×": the scale as the menu's button shows it.
pub(in crate::app::model_preview) fn scale_label(scale: f32) -> String {
    let rounded = (scale * 100.0).round() / 100.0;
    format!("{rounded}×")
}

/// A scale typed into the number box: a finite number above 0, with or
/// without a trailing "×" or "x"; anything else is refused.
pub(in crate::app::model_preview) fn parse_scale(text: &str) -> Option<f32> {
    let text = text.trim().trim_end_matches(['×', 'x', 'X']).trim();
    text.parse::<f32>().ok().filter(|scale| scale.is_finite() && *scale > 0.0)
}

/// A whole number of pixels typed into a size box: above 0, else refused.
pub(in crate::app::model_preview) fn parse_pixels(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|pixels| *pixels > 0)
}

/// A text box showing `shown`, editable: what was typed, once Enter or a
/// click elsewhere commits it, if it was changed (a box left as it was
/// commits nothing, so a rounded width doesn't nudge the scale). Shows what
/// it was given again after.
fn committed_text(ui: &mut Ui, id: egui::Id, shown: String, width: f32) -> Option<String> {
    let mut text = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width).align(egui::Align2::LEFT_CENTER));
    if response.lost_focus() {
        ui.data_mut(|data| data.remove::<String>(id));
        (text.trim() != shown.trim()).then_some(text)
    } else {
        if response.has_focus() {
            ui.data_mut(|data| data.insert_temp(id, text));
        } else {
            ui.data_mut(|data| data.remove::<String>(id));
        }
        None
    }
}

/// The scale menu: a slider from 0.1× to 10×, a number box for any scale,
/// and the image's width and height, each editable, the other following at
/// the viewport's ratio (both set the scale).
fn draw_scale_menu(ui: &mut Ui, state: &mut ModelPreviewState) {
    ui.set_width(240.0);
    ui.label(RichText::new("Snapshot scale").color(subtle_dark()).small());
    // Past its reach, the slider rests at its end; moving it sets the scale.
    let mut slid = state.snapshot_scale.clamp(*SLIDER_SCALES.start(), *SLIDER_SCALES.end());
    ui.spacing_mut().slider_width = ui.available_width();
    let slider = ui.add(egui::Slider::new(&mut slid, SLIDER_SCALES).logarithmic(true).show_value(false));
    if slider.changed() {
        state.snapshot_scale = slid;
    }
    let id = ui.scope_id();
    // The labels in one column, so the boxes line up.
    let label = |ui: &mut Ui, text: &str| {
        ui.add_sized([36.0, ui.spacing().interact_size.y], egui::Label::new(text));
    };
    ui.horizontal(|ui| {
        label(ui, "Scale");
        let shown = scale_label(state.snapshot_scale);
        if let Some(scale) = committed_text(ui, id.with("snapshot_scale_box"), shown, 80.0).as_deref().and_then(parse_scale) {
            state.snapshot_scale = scale;
        }
    });
    let Some(source) = &state.snapshot_source else {
        ui.label(RichText::new("the viewport's size, once it has drawn").color(text_dark()));
        return;
    };
    let viewport = [source.points.x, source.points.y].map(|edge| edge * source.pixels_per_point);
    let [width, height] = snapshot_size(source.points, source.pixels_per_point, state.snapshot_scale);
    ui.horizontal(|ui| {
        label(ui, "Size");
        let typed_width = committed_text(ui, id.with("snapshot_width_box"), width.to_string(), 60.0);
        ui.label("×");
        let typed_height = committed_text(ui, id.with("snapshot_height_box"), height.to_string(), 60.0);
        // Either side sets the scale; the other follows at the viewport's ratio.
        if let Some(pixels) = typed_width.as_deref().and_then(parse_pixels) {
            state.snapshot_scale = pixels as f32 / viewport[0].max(1.0);
        }
        if let Some(pixels) = typed_height.as_deref().and_then(parse_pixels) {
            state.snapshot_scale = pixels as f32 / viewport[1].max(1.0);
        }
    });
    let mut notes = Vec::new();
    if super::super::renderer::snapshot_size_limit().is_some_and(|limit| width.max(height) > limit) {
        notes.push("drawn in tiles".to_owned());
    }
    let bytes = snapshot_bytes([width, height]);
    if bytes > LARGE_SNAPSHOT_BYTES {
        notes.push(format!("{}: asks first", gigabytes(bytes)));
    }
    if !notes.is_empty() {
        ui.label(RichText::new(notes.join(", ")).color(subtle_dark()).small());
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_pixels, parse_scale, scale_label};

    #[test]
    fn a_scale_is_a_number_above_zero() {
        assert_eq!(parse_scale("2"), Some(2.0));
        assert_eq!(parse_scale(" 2.5× "), Some(2.5));
        assert_eq!(parse_scale("40x"), Some(40.0));
        for refused in ["0", "-1", "abc", "", "inf", "NaN"] {
            assert_eq!(parse_scale(refused), None, "{refused:?}");
        }
        assert_eq!(scale_label(1.0), "1×");
        assert_eq!(scale_label(2.5), "2.5×");
        assert_eq!(scale_label(0.333), "0.33×");
        assert_eq!(parse_pixels(" 2160 "), Some(2160));
        for refused in ["0", "-5", "12.5", "abc", ""] {
            assert_eq!(parse_pixels(refused), None, "{refused:?}");
        }
    }
}
