//! The preview header's snapshot: the camera, which copies the frame on
//! screen to the clipboard without its background, and its scale menu
//! (`model_preview::snapshot`).

use super::super::super::ui_kit::button_icons::{ButtonIcon, icon_button, text_dropdown_button};
use super::super::super::ui_kit::style::{subtle_dark, text_dark};
use super::super::snapshot::{LARGE_SNAPSHOT_BYTES, SnapshotPhase, snapshot_bytes, snapshot_size};
use super::super::ModelPreviewState;
use eframe::egui::{self, RichText, Ui};

/// The slider's reach; the number box takes any scale above 0.
pub(in crate::app::model_preview) const SLIDER_SCALES: std::ops::RangeInclusive<f32> = 0.1..=10.0;

/// The camera and, right of it, the scale menu. Drawn right to left, so the
/// menu first.
pub(in crate::app::model_preview) fn draw_snapshot_controls(ui: &mut Ui, state: &mut ModelPreviewState) {
    let size = state
        .snapshot_source
        .as_ref()
        .map(|source| snapshot_size(source.points, source.pixels_per_point, state.snapshot_scale));
    text_dropdown_button(ui, &scale_label(state.snapshot_scale), None, |ui| {
        draw_scale_menu(ui, state);
    })
    .response
    .on_hover_text("The snapshot's size, as a scale of the viewport's");
    let busy = state.snapshot.busy();
    let ready = size.is_some() && !busy;
    let tooltip = match size {
        Some([width, height]) => format!("Copy the frame to the clipboard, without its background ({width} × {height})"),
        None => "Copy the frame to the clipboard, without its background".to_owned(),
    };
    let response = icon_button(ui, ButtonIcon::Camera, &tooltip, ready, text_dark());
    let response = if busy {
        response.on_disabled_hover_text("Copying the last snapshot…")
    } else if size.is_none() {
        response.on_disabled_hover_text("The preview has drawn nothing to copy yet")
    } else {
        response
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

/// The scale menu: a slider from 0.1× to 10×, a number box for any scale,
/// and the size the scale gives the viewport as it is.
fn draw_scale_menu(ui: &mut Ui, state: &mut ModelPreviewState) {
    ui.set_width(240.0);
    ui.label(RichText::new("Snapshot scale").color(subtle_dark()).small());
    // Past its reach, the slider rests at its end; moving it sets the scale.
    let mut slid = state.snapshot_scale.clamp(*SLIDER_SCALES.start(), *SLIDER_SCALES.end());
    let slider = ui.add(egui::Slider::new(&mut slid, SLIDER_SCALES).logarithmic(true).show_value(false));
    if slider.changed() {
        state.snapshot_scale = slid;
    }
    let id = ui.scope_id().with("snapshot_scale_box");
    ui.horizontal(|ui| {
        ui.label("Scale");
        let mut text = ui.data(|data| data.get_temp::<String>(id)).unwrap_or_else(|| scale_label(state.snapshot_scale));
        let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0));
        let commit = response.lost_focus();
        if commit {
            if let Some(scale) = parse_scale(&text) {
                state.snapshot_scale = scale;
            }
            ui.data_mut(|data| data.remove::<String>(id));
        } else if response.has_focus() {
            ui.data_mut(|data| data.insert_temp(id, text));
        } else {
            ui.data_mut(|data| data.remove::<String>(id));
        }
    });
    let resolution = match &state.snapshot_source {
        Some(source) => {
            let [width, height] = snapshot_size(source.points, source.pixels_per_point, state.snapshot_scale);
            let mut line = format!("{width} × {height}");
            if super::super::renderer::snapshot_size_limit().is_some_and(|limit| width.max(height) > limit) {
                line.push_str(", drawn in tiles");
            }
            let bytes = snapshot_bytes([width, height]);
            if bytes > LARGE_SNAPSHOT_BYTES {
                line.push_str(&format!(", {}: asks first", gigabytes(bytes)));
            }
            line
        }
        None => "the viewport's size, once it has drawn".to_owned(),
    };
    ui.label(RichText::new(resolution).color(text_dark()));
}

#[cfg(test)]
mod tests {
    use super::{parse_scale, scale_label};

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
    }
}
