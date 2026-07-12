//! Legacy Halo 2 function layout, controls, and preservation tests.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

pub(super) fn h2_legacy_combo(
    ui: &mut Ui,
    id: &str,
    value: &mut u8,
    options: &[(u8, &str)],
    editable: bool,
    width: f32,
) -> bool {
    let label = options
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, name)| *name)
        .unwrap_or("unknown");
    if !editable {
        foundation_input_cell(ui, label, width);
        return false;
    }
    let mut changed = false;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt(id)
            .selected_text(label)
            .width(width),
        |ui| {
            for (option_value, name) in options {
                if ui
                    .selectable_label(*value == *option_value, *name)
                    .clicked()
                    && *value != *option_value
                {
                    *value = *option_value;
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current_index = options
            .iter()
            .position(|(option_value, _)| *value == *option_value)
            .unwrap_or(0);
        if let Some(next) = combo_scroll_next_index(current_index, options.len(), delta) {
            let option_value = options[next].0;
            *value = option_value;
            changed = true;
        }
    }
    changed
}

pub(super) fn h2_output_type_label(value: u8) -> &'static str {
    match value {
        0 => "scalar (intensity)",
        1 => "scalar (alpha)",
        2 | 0x20 => "2-color",
        3 | 0x40 => "3-color",
        4 | 0x80 => "4-color",
        _ => "unknown",
    }
}

pub(super) fn h2_output_type_combo(ui: &mut Ui, value: &mut u8, editable: bool) -> bool {
    let label = h2_output_type_label(*value);
    if !editable {
        foundation_input_cell(ui, label, 140.0);
        return false;
    }
    let mut changed = false;
    let (_, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt("h2_fn_output")
            .selected_text(label)
            .width(140.0),
        |ui| {
            for (option_value, name) in H2_OUTPUT_TYPE_OPTIONS {
                let selected = h2_output_type_label(*value) == name;
                if ui.selectable_label(selected, name).clicked() && *value != option_value {
                    *value = option_value;
                    changed = true;
                }
            }
        },
    );
    if let Some(delta) = wheel_delta {
        let current_index = H2_OUTPUT_TYPE_OPTIONS
            .iter()
            .position(|(_, name)| *name == label)
            .unwrap_or(0);
        if let Some(next) =
            combo_scroll_next_index(current_index, H2_OUTPUT_TYPE_OPTIONS.len(), delta)
        {
            let option_value = H2_OUTPUT_TYPE_OPTIONS[next].0;
            *value = option_value;
            changed = true;
        }
    }
    changed
}

pub(in crate::app) fn draw_h2_legacy_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
) -> bool {
    let mut changed = false;
    let Some(h2) = view.h2_legacy.as_mut() else {
        return false;
    };
    let input_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.input_name.is_empty());
    let range_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.range_name.is_empty());
    let time_editable = editable
        && view
            .edit
            .as_ref()
            .is_some_and(|paths| !paths.time_period.is_empty());

    ui.horizontal(|ui| {
        ui.label(RichText::new("Function type:").color(text_dark()).small());
        changed |= h2_legacy_combo(
            ui,
            "h2_fn_type",
            &mut h2.function_type,
            &H2_FUNCTION_TYPE_OPTIONS,
            editable,
            130.0,
        );
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Input:").color(text_dark()).small());
        changed |= seeded_name_combo(ui, "h2_fn_input", &mut view.input_name, input_editable);

        let mut ranged = !view.range_name.is_empty();
        if ui
            .add_enabled(range_editable, egui::Checkbox::new(&mut ranged, ""))
            .changed()
        {
            if !ranged {
                view.range_name.clear();
            }
            changed = true;
        }
        ui.label(RichText::new("Range:").color(text_dark()).small());
        if ranged {
            changed |= seeded_name_combo(ui, "h2_fn_range", &mut view.range_name, range_editable);
        } else {
            foundation_input_cell(ui, "", 120.0);
        }

        ui.label(RichText::new("Output Type:").color(text_dark()).small());
        let mut output_type = h2.output_type;
        if h2_output_type_combo(ui, &mut output_type, editable) {
            h2.set_output_type(output_type);
            changed = true;
        }
    });
    ui.add_space(8.0);
    if !h2.is_color_output() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Min:").color(text_dark()).small());
            changed |= h2_number_stepper(ui, "h2_min", &mut h2.min, 1.0, editable);
            ui.label(RichText::new("Max:").color(text_dark()).small());
            changed |= h2_number_stepper(ui, "h2_max", &mut h2.max, 1.0, editable);
        });
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Exponent:").color(text_dark()).small());
        let exponent_options: &[(u8, &str)] = if h2.function_type == 2 {
            &H2_TRANSITION_EXPONENT_OPTIONS
        } else {
            &H2_EXPONENT_OPTIONS
        };
        changed |= h2_legacy_combo(
            ui,
            "h2_fn_exponent",
            &mut h2.exponent,
            exponent_options,
            editable,
            150.0,
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Frequency:").color(text_dark()).small());
        changed |= h2_number_stepper(ui, "h2_frequency", &mut h2.frequency, 0.25, editable);
        ui.label(RichText::new("Phase:").color(text_dark()).small());
        changed |= h2_number_stepper(ui, "h2_phase", &mut h2.phase, 1.0, editable);
    });
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        draw_h2_legacy_graph_preview(ui, h2);
        if h2.is_color_output() {
            ui.add_space(8.0);
            changed |= draw_h2_legacy_color_stop_editors(ui, h2, editable);
        }
    });
    if h2.is_color_output() {
        let (bar, _) = ui.allocate_exact_size(Vec2::new(360.0, 24.0), Sense::hover());
        draw_function_color_gradient_horizontal(ui.painter(), bar, &h2.color_stops());
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("time period").color(text_dark()).small());
        changed |= ui
            .add_enabled(
                time_editable,
                egui::DragValue::new(&mut view.time_period_in_seconds)
                    .speed(0.1)
                    .range(0.0..=f32::MAX),
            )
            .changed();
        ui.label(RichText::new("seconds").color(subtle_dark()).small());
    });
    changed
}

pub(super) fn h2_number_stepper(
    ui: &mut Ui,
    id: &str,
    value: &mut f32,
    step: f32,
    editable: bool,
) -> bool {
    let mut changed = false;
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(editable, egui::Button::new(RichText::new("-").small()))
                .clicked()
            {
                *value -= step;
                changed = true;
            }
            changed |= ui
                .add_enabled(editable, egui::DragValue::new(value).speed(step))
                .changed();
            if ui
                .add_enabled(editable, egui::Button::new(RichText::new("+").small()))
                .clicked()
            {
                *value += step;
                changed = true;
            }
        });
    });
    changed
}

pub(super) fn draw_h2_legacy_color_stop_editors(
    ui: &mut Ui,
    h2: &mut H2LegacyFunctionView,
    editable: bool,
) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        // Top swatch is the high/end color, bottom swatch is the low/start color.
        for index in (0..h2.color_stop_count()).rev() {
            let mut color = h2.color_stop(index);
            ui.horizontal(|ui| {
                let resp = if editable {
                    ui.color_edit_button_srgba(&mut color)
                } else {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color);
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.0, foundation_input_edge()));
                    resp
                };
                ui.label(
                    RichText::new(format!(
                        "#{:02X}{:02X}{:02X}",
                        color.r(),
                        color.g(),
                        color.b()
                    ))
                    .color(subtle_dark())
                    .small()
                    .monospace(),
                );
                if resp.changed() {
                    h2.set_color_stop(index, color);
                    changed = true;
                }
            });
            if index > 0 {
                ui.add_space(if h2.color_stop_count() <= 2 {
                    90.0
                } else {
                    18.0
                });
            }
        }
    });
    changed
}

pub(super) fn draw_h2_legacy_graph_preview(ui: &mut Ui, h2: &H2LegacyFunctionView) {
    let desired = Vec2::new(360.0, 120.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let plot = rect.shrink(12.0);
    if h2.is_color_output() {
        draw_function_color_gradient_vertical(&painter, plot, &h2.color_stops());
    } else {
        painter.rect_filled(plot, 0.0, Color32::from_gray(180));
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, Color32::from_gray(80)));
    for i in 1..10 {
        let x = egui::lerp(plot.left()..=plot.right(), i as f32 / 10.0);
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.0, Color32::from_gray(135)),
        );
        let y = egui::lerp(plot.bottom()..=plot.top(), i as f32 / 10.0);
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            Stroke::new(1.0, Color32::from_gray(135)),
        );
    }
    let low = h2.min.min(h2.max);
    let high = h2.min.max(h2.max);
    let span = (high - low).abs().max(0.0001);
    let mut points = Vec::with_capacity(96);
    for i in 0..96 {
        let t = i as f32 / 95.0;
        let y = ((h2.sample(t) - low) / span).clamp(0.0, 1.0);
        points.push(egui::pos2(
            egui::lerp(plot.left()..=plot.right(), t),
            egui::lerp(plot.bottom()..=plot.top(), y),
        ));
    }
    painter.add(egui::Shape::line(points, Stroke::new(2.0, Color32::GREEN)));
}

#[derive(Clone, PartialEq)]
/// Recognized classic H2 byte layouts whose field offsets differ.
/// Unknown layouts remain representable through their raw bytes rather than
/// being normalized into the default shape.
enum H2LegacyFunctionLayout {
    Default,
    DamageEffectVibration36,
}

#[derive(Clone, PartialEq)]
/// Parsed view over a classic Halo 2 mapping-function byte block.
/// `raw` is the preservation source: serialization patches known offsets into a
/// clone instead of regenerating the blob and discarding unknown bytes.
pub(in crate::app) struct H2LegacyFunctionView {
    raw: Vec<u8>,
    layout: H2LegacyFunctionLayout,
    function_type: u8,
    output_type: u8,
    exponent: u8,
    min: f32,
    max: f32,
    frequency: f32,
    phase: f32,
}

impl H2LegacyFunctionView {
    pub(in crate::app) fn parse(raw: Vec<u8>) -> Option<Self> {
        if raw.len() < 20 {
            return None;
        }
        let function_type = raw[0];
        let output_type = raw[1];
        let min = read_f32_le(&raw, 4).unwrap_or(0.0);
        let max = read_f32_le(&raw, 8).unwrap_or(1.0);
        let (exponent, frequency, phase) = if raw.len() >= 52 && function_type == 3 {
            (
                raw[2],
                read_f32_le(&raw, 20).unwrap_or(0.0),
                read_f32_le(&raw, 24).unwrap_or(0.0),
            )
        } else {
            (
                raw[2],
                read_f32_le(&raw, 12).unwrap_or(0.0),
                read_f32_le(&raw, 16).unwrap_or(0.0),
            )
        };
        Some(Self {
            function_type,
            output_type,
            exponent,
            min,
            max,
            frequency,
            phase,
            raw,
            layout: H2LegacyFunctionLayout::Default,
        })
    }

    pub(in crate::app) fn parse_damage_effect_vibration(raw: Vec<u8>) -> Option<Self> {
        if raw.len() < 20 {
            return None;
        }
        let function_type = raw[0];
        let output_type = raw[1];
        let exponent = raw[2];
        let min = read_f32_le(&raw, 20).unwrap_or(0.0);
        let max = read_f32_le(&raw, 24).unwrap_or(1.0);
        Some(Self {
            function_type,
            output_type,
            exponent,
            min,
            max,
            frequency: 0.0,
            phase: 0.0,
            raw,
            layout: H2LegacyFunctionLayout::DamageEffectVibration36,
        })
    }

    pub(in crate::app) fn to_bytes(&self) -> Vec<u8> {
        let mut raw = self.raw.clone();
        if raw.len() < 20 {
            raw.resize(20, 0);
        }
        raw[0] = self.function_type;
        raw[1] = self.output_type;
        if self.layout == H2LegacyFunctionLayout::DamageEffectVibration36 {
            raw[2] = self.exponent;
            if raw.len() < 28 {
                raw.resize(28, 0);
            }
            raw[20..24].copy_from_slice(&self.min.to_le_bytes());
            raw[24..28].copy_from_slice(&self.max.to_le_bytes());
            return raw;
        }
        if !self.is_color_output() {
            raw[4..8].copy_from_slice(&self.min.to_le_bytes());
            raw[8..12].copy_from_slice(&self.max.to_le_bytes());
        }
        if raw.len() >= 52 && self.function_type == 3 {
            raw[2] = self.exponent;
            raw[20..24].copy_from_slice(&self.frequency.to_le_bytes());
            raw[24..28].copy_from_slice(&self.phase.to_le_bytes());
        } else if !self.is_color_output() || self.color_stop_count() <= 2 {
            raw[2] = self.exponent;
            raw[12..16].copy_from_slice(&self.frequency.to_le_bytes());
            raw[16..20].copy_from_slice(&self.phase.to_le_bytes());
        } else {
            raw[2] = self.exponent;
        }
        raw
    }

    fn is_color_output(&self) -> bool {
        self.color_stop_count() > 0
    }

    fn color_stop_count(&self) -> usize {
        h2_color_stop_count(self.output_type)
    }

    fn set_output_type(&mut self, output_type: u8) {
        if self.output_type == output_type {
            return;
        }

        let old_count = self.color_stop_count();
        let new_count = h2_color_stop_count(output_type);
        if old_count > 0 && new_count > 0 && old_count != new_count {
            let old_slots = (0..old_count)
                .map(|index| self.color_stop_slot_bytes(index))
                .collect::<Vec<_>>();
            let mut new_slots = vec![[0, 0, 0, 0]; new_count];
            new_slots[0] = old_slots[0];
            new_slots[new_count - 1] = old_slots[old_count - 1];

            let interior_count = old_count.saturating_sub(2).min(new_count.saturating_sub(2));
            for index in 0..interior_count {
                new_slots[index + 1] = old_slots[index + 1];
            }

            let required_len = 4 + new_count * 4;
            if self.raw.len() < required_len {
                self.raw.resize(required_len, 0);
            }
            for (index, slot) in new_slots.iter().enumerate() {
                let offset = 4 + index * 4;
                self.raw[offset..offset + 4].copy_from_slice(slot);
            }
        }

        self.output_type = output_type;
    }

    fn color_stop(&self, index: usize) -> Color32 {
        if index >= self.color_stop_count() {
            return Color32::BLACK;
        }
        let offset = 4 + index * 4;
        if self.color_stop_count() == 2 && index == 1 && h2_bgra_color_is_unset(&self.raw, offset) {
            return self.color_stop(0);
        }
        h2_bgra_color(&self.raw, offset).unwrap_or(Color32::BLACK)
    }

    fn set_color_stop(&mut self, index: usize, color: Color32) {
        if index >= self.color_stop_count() {
            return;
        }
        let offset = 4 + index * 4;
        if self.raw.len() < offset + 4 {
            self.raw.resize(offset + 4, 0);
        }
        let alpha = self.raw[offset + 3];
        self.raw[offset] = color.b();
        self.raw[offset + 1] = color.g();
        self.raw[offset + 2] = color.r();
        self.raw[offset + 3] = alpha;
    }

    fn color_stop_slot_bytes(&self, index: usize) -> [u8; 4] {
        let offset = 4 + index * 4;
        if self.color_stop_count() == 2 && index == 1 && h2_bgra_color_is_unset(&self.raw, offset) {
            let color = self.color_stop(0);
            let alpha = self.raw.get(offset + 3).copied().unwrap_or(0);
            return [color.b(), color.g(), color.r(), alpha];
        }
        let mut slot = [0, 0, 0, 0];
        if let Some(bytes) = self.raw.get(offset..offset + 4) {
            slot.copy_from_slice(bytes);
        }
        slot
    }

    fn color_stops(&self) -> Vec<Color32> {
        (0..self.color_stop_count())
            .map(|index| self.color_stop(index))
            .collect()
    }

    fn sample(&self, x: f32) -> f32 {
        let n = match self.function_type {
            0 => x,
            1 => 0.0,
            2 => h2_transition_sample(self.exponent, x),
            3 => h2_periodic_sample(self.exponent, x * self.frequency + self.phase),
            4 => x,
            _ => x,
        }
        .clamp(0.0, 1.0);
        self.min + n * (self.max - self.min)
    }
}

pub(super) fn h2_bgra_color(raw: &[u8], offset: usize) -> Option<Color32> {
    Some(Color32::from_rgb(
        *raw.get(offset + 2)?,
        *raw.get(offset + 1)?,
        *raw.get(offset)?,
    ))
}

pub(super) fn h2_bgra_color_is_unset(raw: &[u8], offset: usize) -> bool {
    raw.get(offset..offset + 4)
        .is_none_or(|bytes| bytes.iter().all(|byte| *byte == 0))
}

pub(super) fn h2_color_stop_count(output_type: u8) -> usize {
    match output_type {
        2 | 0x20 => 2,
        3 | 0x40 => 3,
        4 | 0x80 => 4,
        _ => 0,
    }
}

pub(super) fn read_f32_le(raw: &[u8], offset: usize) -> Option<f32> {
    Some(f32::from_le_bytes(
        raw.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

pub(super) fn h2_periodic_sample(exponent: u8, x: f32) -> f32 {
    let t = x.rem_euclid(1.0);
    match exponent {
        2 | 3 => (1.0 - (t * std::f32::consts::TAU).cos()) * 0.5,
        4 | 5 => {
            if t < 0.5 {
                t * 2.0
            } else {
                (1.0 - t) * 2.0
            }
        }
        _ => t,
    }
}

pub(super) fn h2_transition_sample(exponent: u8, x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    match exponent {
        0 => t,
        1 => t * t,
        2 => t * t * t,
        3 => t.sqrt(),
        4 => t.cbrt(),
        5 => (1.0 - (t * std::f32::consts::PI).cos()) * 0.5,
        6 => 0.0,
        7 => 1.0,
        _ => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_function_type_combo_offers_every_supported_mapping_function_type() {
        let expected = [
            FunctionType::Identity,
            FunctionType::Constant,
            FunctionType::Transition,
            FunctionType::Periodic,
            FunctionType::Linear,
            FunctionType::LinearKey,
            FunctionType::MultiLinearKey,
            FunctionType::Spline,
            FunctionType::MultiSpline,
            FunctionType::Exponent,
            FunctionType::Spline2,
        ];

        assert_eq!(EDITABLE_FUNCTION_TYPES, expected);
        for kind in expected {
            assert!(
                is_editable_function_type(kind),
                "{kind:?} should be editable"
            );
            assert_eq!(FunctionType::from_byte(kind as u8), Some(kind));
        }
    }

    #[test]
    fn foundation_master_types_keep_all_curve_variants_in_curve_mode() {
        assert_eq!(
            FoundationMasterType::from_function_type(FunctionType::Constant),
            FoundationMasterType::Basic
        );
        assert_eq!(
            FoundationMasterType::from_function_type(FunctionType::Periodic),
            FoundationMasterType::Periodic
        );
        assert_eq!(
            FoundationMasterType::from_function_type(FunctionType::Exponent),
            FoundationMasterType::Exponent
        );
        assert_eq!(
            FoundationMasterType::from_function_type(FunctionType::Transition),
            FoundationMasterType::Transition
        );
        for kind in [
            FunctionType::Identity,
            FunctionType::Linear,
            FunctionType::LinearKey,
            FunctionType::MultiLinearKey,
            FunctionType::Spline,
            FunctionType::MultiSpline,
            FunctionType::Spline2,
        ] {
            assert_eq!(
                FoundationMasterType::from_function_type(kind),
                FoundationMasterType::Curve,
                "{kind:?} should retain the curve presentation"
            );
        }
        assert_eq!(FoundationMasterType::Curve.target_function_type(), None);
    }

    #[test]
    fn foundation_color_stop_slots_match_engine_header_layout() {
        assert_eq!(color_graph_slots(ColorGraphType::TwoColor), &[0, 3]);
        assert_eq!(color_graph_slots(ColorGraphType::ThreeColor), &[0, 1, 3]);
        assert_eq!(color_graph_slots(ColorGraphType::FourColor), &[0, 1, 2, 3]);
    }

    #[test]
    fn h2_legacy_function_type_options_match_supported_mapping_function_types() {
        assert_eq!(
            H2_FUNCTION_TYPE_OPTIONS.len(),
            EDITABLE_FUNCTION_TYPES.len()
        );
        for (value, _) in H2_FUNCTION_TYPE_OPTIONS {
            let kind =
                FunctionType::from_byte(value).expect("H2 option should map to function type");
            assert!(
                EDITABLE_FUNCTION_TYPES.contains(&kind),
                "{kind:?} should be available in generic function editor too"
            );
        }
    }

    #[test]
    fn h2_output_type_options_match_guerilla_color_counts() {
        assert_eq!(
            H2_OUTPUT_TYPE_OPTIONS,
            [
                (0, "scalar (intensity)"),
                (1, "scalar (alpha)"),
                (0x20, "2-color"),
                (0x40, "3-color"),
                (0x80, "4-color"),
            ]
        );
        assert_eq!(h2_output_type_label(2), "2-color");
        assert_eq!(h2_output_type_label(3), "3-color");
        assert_eq!(h2_output_type_label(4), "4-color");
    }

    #[test]
    fn h2_legacy_52_byte_periodic_function_reads_frequency_at_offset_20() {
        let mut raw = vec![0; 52];
        raw[0] = 3;
        raw[2] = 6;
        raw[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        raw[20..24].copy_from_slice(&0.25f32.to_le_bytes());
        raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
        raw[36..40].copy_from_slice(&1.0f32.to_le_bytes());

        let view = H2LegacyFunctionView::parse(raw).expect("legacy function should parse");

        assert_eq!(view.exponent, 6);
        assert_eq!(view.min, 0.0);
        assert_eq!(view.max, 1.0);
        assert_eq!(view.frequency, 0.25);
        assert_eq!(view.phase, 0.0);
        assert_eq!(&view.to_bytes()[20..24], &0.25f32.to_le_bytes());
    }

    #[test]
    fn h2_legacy_color_function_preserves_bgra_endpoints() {
        let mut raw = vec![0; 28];
        raw[0] = 3;
        raw[1] = 0x20;
        raw[2] = 2;
        raw[4..8].copy_from_slice(&[0x10, 0x20, 0x30, 0x40]);
        raw[8..12].copy_from_slice(&[0x50, 0x60, 0x70, 0x80]);
        raw[12..16].copy_from_slice(&0.25f32.to_le_bytes());
        raw[16..20].copy_from_slice(&0.5f32.to_le_bytes());

        let view = H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");
        let data = view.to_bytes();

        assert!(view.is_color_output());
        assert_eq!(&data[4..12], &raw[4..12]);
        assert_eq!(&data[12..16], &0.25f32.to_le_bytes());
        assert_eq!(&data[16..20], &0.5f32.to_le_bytes());
    }

    #[test]
    fn h2_legacy_four_color_function_preserves_all_color_slots() {
        let mut raw = vec![0; 28];
        raw[0] = 7;
        raw[1] = 0x80;
        raw[4..8].copy_from_slice(&[0x10, 0x11, 0x12, 0x13]);
        raw[8..12].copy_from_slice(&[0x20, 0x21, 0x22, 0x23]);
        raw[12..16].copy_from_slice(&[0x30, 0x31, 0x32, 0x33]);
        raw[16..20].copy_from_slice(&[0x40, 0x41, 0x42, 0x43]);
        let mut view =
            H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");

        assert_eq!(view.color_stop_count(), 4);
        assert_eq!(view.color_stop(3), Color32::from_rgb(0x42, 0x41, 0x40));
        view.set_color_stop(2, Color32::from_rgb(0xAA, 0xBB, 0xCC));
        let data = view.to_bytes();

        assert_eq!(&data[4..12], &raw[4..12]);
        assert_eq!(&data[12..16], &[0xCC, 0xBB, 0xAA, 0x33]);
        assert_eq!(&data[16..20], &raw[16..20]);
    }

    #[test]
    fn h2_legacy_color_stop_edit_writes_bgr_and_preserves_alpha() {
        let mut raw = vec![0; 28];
        raw[0] = 3;
        raw[1] = 2;
        raw[4..8].copy_from_slice(&[1, 2, 3, 4]);
        raw[8..12].copy_from_slice(&[5, 6, 7, 8]);
        let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

        assert_eq!(view.color_stop(0), Color32::from_rgb(3, 2, 1));
        view.set_color_stop(1, Color32::from_rgb(0xAA, 0xBB, 0xCC));
        let data = view.to_bytes();

        assert_eq!(&data[8..12], &[0xCC, 0xBB, 0xAA, 8]);
    }

    #[test]
    fn h2_legacy_unset_second_color_displays_as_first_until_edited() {
        let mut raw = vec![0; 28];
        raw[0] = 3;
        raw[1] = 2;
        raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
        let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::from_rgb(0xC6, 0x00, 0x00));

        view.set_color_stop(1, Color32::from_rgb(0x80, 0x10, 0x20));
        let data = view.to_bytes();

        assert_eq!(&data[4..8], &[0x00, 0x00, 0xC6, 0x00]);
        assert_eq!(&data[8..12], &[0x20, 0x10, 0x80, 0x00]);
    }

    #[test]
    fn h2_legacy_three_and_four_color_show_real_black_unset_slots() {
        let mut raw = vec![0; 28];
        raw[0] = 7;
        raw[1] = 0x40;
        raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
        let mut view =
            H2LegacyFunctionView::parse(raw.clone()).expect("color function should parse");

        assert_eq!(view.color_stop_count(), 3);
        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::BLACK);
        assert_eq!(view.color_stop(2), Color32::BLACK);

        view.output_type = 0x80;
        assert_eq!(view.color_stop_count(), 4);
        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::BLACK);
        assert_eq!(view.color_stop(2), Color32::BLACK);
        assert_eq!(view.color_stop(3), Color32::BLACK);
    }

    #[test]
    fn h2_legacy_color_output_conversion_preserves_endpoints_and_inserts_black() {
        let mut raw = vec![0; 28];
        raw[0] = 7;
        raw[1] = 0x20;
        raw[4..8].copy_from_slice(&[0x00, 0x00, 0xC6, 0x00]);
        let mut view = H2LegacyFunctionView::parse(raw).expect("color function should parse");

        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::from_rgb(0xC6, 0x00, 0x00));

        view.set_output_type(0x40);
        assert_eq!(view.color_stop_count(), 3);
        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::BLACK);
        assert_eq!(view.color_stop(2), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(
            &view.to_bytes()[4..16],
            &[0x00, 0x00, 0xC6, 0x00, 0, 0, 0, 0, 0x00, 0x00, 0xC6, 0x00]
        );

        view.set_output_type(0x80);
        assert_eq!(view.color_stop_count(), 4);
        assert_eq!(view.color_stop(0), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(view.color_stop(1), Color32::BLACK);
        assert_eq!(view.color_stop(2), Color32::BLACK);
        assert_eq!(view.color_stop(3), Color32::from_rgb(0xC6, 0x00, 0x00));
        assert_eq!(
            &view.to_bytes()[4..20],
            &[
                0x00, 0x00, 0xC6, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00, 0xC6, 0x00,
            ]
        );
    }

    #[test]
    fn damage_effect_vibration_function_reads_transition_values_at_observed_offsets() {
        let mut raw = vec![0; 36];
        raw[0] = 2;
        raw[1] = 0;
        raw[2] = 1;
        raw[20..24].copy_from_slice(&0.8f32.to_le_bytes());
        raw[24..28].copy_from_slice(&0.4f32.to_le_bytes());
        raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());

        let view = H2LegacyFunctionView::parse_damage_effect_vibration(raw)
            .expect("damage effect vibration function should parse");

        assert_eq!(view.function_type, 2);
        assert_eq!(view.output_type, 0);
        assert_eq!(view.exponent, 1);
        assert_eq!(view.min, 0.8);
        assert_eq!(view.max, 0.4);
        assert_eq!(&view.to_bytes()[20..24], &0.8f32.to_le_bytes());
        assert_eq!(&view.to_bytes()[24..28], &0.4f32.to_le_bytes());
    }

    #[test]
    fn damage_effect_vibration_edit_emits_byte_block_op() {
        let mut raw = vec![0; 36];
        raw[0] = 2;
        raw[1] = 0;
        raw[2] = 1;
        raw[20..24].copy_from_slice(&0.8f32.to_le_bytes());
        raw[24..28].copy_from_slice(&0.4f32.to_le_bytes());
        raw[32..36].copy_from_slice(&1.0f32.to_le_bytes());
        let h2_legacy = H2LegacyFunctionView::parse_damage_effect_vibration(raw.clone())
            .expect("damage effect vibration function should parse");
        let function = TagFunction::parse(&decode_hex(&constant_function_hex(0.0)).unwrap())
            .expect("placeholder function should parse");
        let mut view = FunctionView::from_function(function).with_h2_legacy(h2_legacy);
        let previous = FunctionSnapshot::from_view(&view);
        let h2 = view.h2_legacy.as_mut().unwrap();
        h2.exponent = 2;
        h2.min = 1.0;
        h2.max = 0.7;
        let paths = FunctionEditPaths {
            data: FunctionDataStorage::Halo2ByteBlock(
                "player responses[1]/vibration/low frequency vibration/dirty whore/data".to_owned(),
            ),
            parameter_type: String::new(),
            input_name: String::new(),
            range_name: String::new(),
            time_period: String::new(),
            block_path: String::new(),
            block_index: 0,
        };

        let batch = push_function_edit(&paths, &previous, &view);

        assert!(batch.edits.is_empty());
        assert_eq!(batch.data_ops.len(), 1);
        let data = &batch.data_ops[0].data;
        assert_eq!(data.len(), 36);
        assert_eq!(data[2], 2);
        assert_eq!(&data[20..24], &1.0f32.to_le_bytes());
        assert_eq!(&data[24..28], &0.7f32.to_le_bytes());
        assert_eq!(&data[32..36], &raw[32..36]);
    }
}

impl FunctionView {
    pub(in crate::app) fn from_function(function: TagFunction) -> Self {
        Self {
            function,
            h2_legacy: None,
            input_name: String::new(),
            range_name: String::new(),
            output_index: None,
            time_period_in_seconds: 0.0,
            edit: None,
            hide_scalar_color_controls: false,
        }
    }

    pub(in crate::app) fn from_animated(
        animated: &RenderMethodAnimatedParameter,
        function: TagFunction,
    ) -> Self {
        Self {
            function,
            h2_legacy: None,
            input_name: animated.input_name.clone(),
            range_name: animated.range_name.clone(),
            output_index: animated.parameter_type.and_then(|kind| {
                OUTPUT_TYPE_OPTIONS
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(kind.name()))
                    .map(|(value, _)| *value)
            }),
            time_period_in_seconds: animated.time_period_in_seconds,
            edit: None,
            hide_scalar_color_controls: false,
        }
    }

    pub(in crate::app) fn with_edit(mut self, paths: FunctionEditPaths) -> Self {
        self.edit = Some(paths);
        self
    }

    pub(in crate::app) fn with_h2_scalar_ui(mut self) -> Self {
        self.hide_scalar_color_controls = true;
        self
    }

    pub(in crate::app) fn with_h2_legacy(mut self, h2_legacy: H2LegacyFunctionView) -> Self {
        self.h2_legacy = Some(h2_legacy);
        self.hide_scalar_color_controls = true;
        self
    }

    pub(in crate::app) fn data_bytes(&self) -> Vec<u8> {
        self.h2_legacy
            .as_ref()
            .map(H2LegacyFunctionView::to_bytes)
            .unwrap_or_else(|| self.function.to_bytes())
    }
}

#[derive(Clone)]
/// Cross-frame function editor state bound to one tag and one captured view.
/// The original write targets and last-applied snapshot prevent selection or
/// presentation-setting changes from redirecting an in-progress edit.
pub(in crate::app) struct FunctionPopup {
    /// The tag the function belongs to — edits target this tag's doc.
    pub(super) tag_key: String,
    pub(super) title: String,
    pub(super) view: FunctionView,
    /// Whether the owning tag is writable (LE loose file). Read-only
    /// tags still open the dialog but disable the controls.
    pub(super) editable: bool,
    /// Snapshot of the values last pushed as edits; lets us emit a
    /// `PendingFieldEdit` only when something actually changed.
    pub(super) last_applied: FunctionSnapshot,
    /// Currently selected LinearKey control point (drag/x-y target).
    pub(super) selected_point: usize,
    /// Captured on the popup's first frame so toggling the developer setting
    /// cannot change the presentation of an edit already in progress.
    pub(super) h3_presentation: Option<H3FunctionEditorPresentation>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum H3FunctionEditorPresentation {
    Foundation,
    Legacy,
}

impl FunctionPopup {
    pub(in crate::app) fn new(
        tag_key: String,
        title: String,
        view: FunctionView,
        editable: bool,
    ) -> Self {
        let last_applied = FunctionSnapshot::from_view(&view);
        Self {
            tag_key,
            title,
            view,
            editable,
            last_applied,
            selected_point: 0,
            h3_presentation: None,
        }
    }
}

/// Values that map to writable tag fields. Compared frame-to-frame to
/// decide which `PendingFieldEdit`s to emit.
/// Raw function bytes are compared in full so changes never silently discard
/// unrecognized classic H2 data.
#[derive(Clone, PartialEq)]
pub(in crate::app) struct FunctionSnapshot {
    pub(super) data: Vec<u8>,
    pub(super) output_index: Option<i32>,
    pub(super) input_name: String,
    pub(super) range_name: String,
    pub(super) time_period: f32,
}

impl FunctionSnapshot {
    pub(in crate::app) fn from_view(view: &FunctionView) -> Self {
        Self {
            data: view.data_bytes(),
            output_index: view.output_index,
            input_name: view.input_name.clone(),
            range_name: view.range_name.clone(),
            time_period: view.time_period_in_seconds,
        }
    }
}

/// Edits produced by the function dialog this frame, plus the tag they
/// belong to.
pub(in crate::app) struct FunctionEditBatch {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) edits: Vec<PendingFieldEdit>,
    pub(in crate::app) data_ops: Vec<FunctionDataOp>,
}
