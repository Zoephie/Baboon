//! Shared color picker, swatches, and Baboon palette persistence.
//! It owns material-specific presentation and color workflows; generic field editing and document persistence belong elsewhere.

use super::*;

#[derive(Clone)]
pub(in crate::app) struct MaterialColorPopup {
    title: String,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    pub(in crate::app) sc_hex: String,
    pc_hex_input: String,
    pc_hex_error: Option<String>,
    palette_status: Option<String>,
    confirm_clear_palette: bool,
    /// When Some, clicking OK writes a constant-color function blob to this path.
    write_path: Option<String>,
    /// When Some, clicking OK writes a plain RGB/ARGB color value to this path
    /// (e.g. a permutation `color lower bound` field), not a function blob.
    write_color_field: Option<ColorFieldWrite>,
    /// When Some, clicking OK creates a constant-color animated parameter.
    create_shader_op: Option<ShaderOp>,
    /// When Some, clicking OK creates a shader parameter with a constant-color
    /// animated child.
    create_shader_param_op: Option<ShaderParamOp>,
    /// When Some, clicking OK creates/edits a classic H2 shader parameter.
    create_h2_shader_param_op: Option<H2ShaderParamOp>,
    /// When Some, OK returns a color to the still-open function-editor draft
    /// instead of writing directly to the tag document.
    function_draft_color: Option<FunctionDraftColorWrite>,
    /// Tag key that owns the write_path. Used by draw_color_popup to route the edit.
    tag_key: String,
}

#[derive(Clone, Copy)]
pub(in crate::app) enum FunctionDraftColorTarget {
    H3Logical(usize),
    H2Logical(usize),
}

#[derive(Clone, Copy)]
struct FunctionDraftColorWrite {
    target: FunctionDraftColorTarget,
    original_alpha: u8,
}

/// Target for writing a picked color back into a plain color-valued field.
#[derive(Clone)]
pub(in crate::app) struct ColorFieldWrite {
    path: String,
    /// True for `real_argb_color` (4 channels); false for `real_rgb_color`.
    argb: bool,
}

impl MaterialColorPopup {
    pub(in crate::app) fn new(title: &str, red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        let red = red.clamp(0.0, 1.0);
        let green = green.clamp(0.0, 1.0);
        let blue = blue.clamp(0.0, 1.0);
        let alpha = alpha.clamp(0.0, 1.0);
        Self {
            title: clean_field_name(title),
            red,
            green,
            blue,
            alpha,
            sc_hex: format!(
                "sc#{}, {}, {}, {}",
                format_pc_float(alpha),
                format_pc_float(red),
                format_pc_float(green),
                format_pc_float(blue)
            ),
            pc_hex_input: format_rgb_hex(red, green, blue),
            pc_hex_error: None,
            palette_status: None,
            confirm_clear_palette: false,
            write_path: None,
            write_color_field: None,
            create_shader_op: None,
            create_shader_param_op: None,
            create_h2_shader_param_op: None,
            function_draft_color: None,
            tag_key: String::new(),
        }
    }

    pub(in crate::app) fn with_write(
        mut self,
        tag_key: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.write_path = Some(path.into());
        self
    }

    /// Configure the popup to write a plain color value (RGB or ARGB) back into
    /// the given field path when the user clicks OK.
    pub(in crate::app) fn with_color_field(
        mut self,
        tag_key: impl Into<String>,
        path: impl Into<String>,
        argb: bool,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.write_color_field = Some(ColorFieldWrite {
            path: path.into(),
            argb,
        });
        self
    }

    pub(in crate::app) fn with_shader_op(
        mut self,
        tag_key: impl Into<String>,
        op: ShaderOp,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.create_shader_op = Some(op);
        self
    }

    pub(in crate::app) fn with_shader_param_op(
        mut self,
        tag_key: impl Into<String>,
        op: ShaderParamOp,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.create_shader_param_op = Some(op);
        self
    }

    pub(in crate::app) fn with_h2_shader_param_op(
        mut self,
        tag_key: impl Into<String>,
        op: H2ShaderParamOp,
    ) -> Self {
        self.tag_key = tag_key.into();
        self.create_h2_shader_param_op = Some(op);
        self
    }

    pub(in crate::app) fn with_function_draft_color(
        mut self,
        target: FunctionDraftColorTarget,
        original_alpha: u8,
    ) -> Self {
        self.function_draft_color = Some(FunctionDraftColorWrite {
            target,
            original_alpha,
        });
        self
    }

    pub(in crate::app) fn color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            float_channel_to_u8(self.red),
            float_channel_to_u8(self.green),
            float_channel_to_u8(self.blue),
            float_channel_to_u8(self.alpha),
        )
    }

    fn set_rgb_bytes(&mut self, red: u8, green: u8, blue: u8) {
        self.red = byte_to_float(red);
        self.green = byte_to_float(green);
        self.blue = byte_to_float(blue);
        self.pc_hex_input = format!("#{red:02X}{green:02X}{blue:02X}");
        self.pc_hex_error = None;
    }

    fn set_rgba_bytes(&mut self, red: u8, green: u8, blue: u8, alpha: u8) {
        self.set_rgb_bytes(red, green, blue);
        self.alpha = byte_to_float(alpha);
    }
}

pub(in crate::app) fn color_popup_for_value(
    title: &str,
    value: &TagFieldData,
    formatted: &str,
) -> Option<MaterialColorPopup> {
    match value {
        TagFieldData::RealRgbColor(color) => Some(MaterialColorPopup::new(
            title,
            color.red,
            color.green,
            color.blue,
            1.0,
        )),
        TagFieldData::RealArgbColor(color) => Some(MaterialColorPopup::new(
            title,
            color.red,
            color.green,
            color.blue,
            color.alpha,
        )),
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some(MaterialColorPopup::new(
                title,
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                1.0,
            ))
        }
        TagFieldData::ArgbColor(color) => {
            let raw = color.0;
            Some(MaterialColorPopup::new(
                title,
                byte_to_float(((raw >> 16) & 0xFF) as u8),
                byte_to_float(((raw >> 8) & 0xFF) as u8),
                byte_to_float((raw & 0xFF) as u8),
                byte_to_float(((raw >> 24) & 0xFF) as u8),
            ))
        }
        _ if formatted.starts_with("sc#") => parse_sc_color(title, formatted),
        _ => None,
    }
}

pub(in crate::app) fn material_parameter_color_title(
    element: TagStruct<'_>,
    names: &TagNameIndex,
    fallback: &str,
) -> String {
    material_parameter_name(element, names).unwrap_or_else(|| clean_field_name(fallback))
}

pub(in crate::app) fn parse_sc_color(title: &str, formatted: &str) -> Option<MaterialColorPopup> {
    let values = formatted.strip_prefix("sc#")?;
    let parts = values
        .split(',')
        .map(str::trim)
        .filter_map(|part| part.parse::<f32>().ok())
        .collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    Some(MaterialColorPopup::new(
        title, parts[1], parts[2], parts[3], parts[0],
    ))
}

pub(in crate::app) enum ColorPopupResult {
    FieldEdit {
        tag_key: String,
        edit: PendingFieldEdit,
    },
    ShaderOp {
        tag_key: String,
        op: ShaderOp,
    },
    ShaderParamOp {
        tag_key: String,
        op: ShaderParamOp,
    },
    H2ShaderParamOp {
        tag_key: String,
        op: H2ShaderParamOp,
    },
    FunctionDraftColor {
        target: FunctionDraftColorTarget,
        argb: u32,
    },
}

/// Draw the color inspector / editor popup.
///
/// Returns a write result when the user clicks OK on an editable popup.
pub(in crate::app) fn draw_color_popup(
    ctx: &egui::Context,
    color_popup: &mut Option<MaterialColorPopup>,
    custom_swatches: &mut Vec<Option<[u8; 4]>>,
    palette_last_dir: &mut Option<PathBuf>,
) -> Option<ColorPopupResult> {
    let color = color_popup.as_mut()?;
    let mut open = true;
    let mut close = false;
    let editable = color.write_path.is_some()
        || color.write_color_field.is_some()
        || color.create_shader_op.is_some()
        || color.create_shader_param_op.is_some()
        || color.create_h2_shader_param_op.is_some()
        || color.function_draft_color.is_some();
    let mut result: Option<ColorPopupResult> = None;
    egui::Window::new(color.title.clone())
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .default_size(Vec2::new(448.0, 480.0))
        .show(ctx, |ui| {
            if editable {
                draw_color_picker_editor(ui, color, custom_swatches, palette_last_dir);
            } else {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(80.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color.color32());
                    ui.painter()
                        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                    ui.add_space(14.0);
                    draw_color_channel_table(ui, color);
                });
            }
            ui.add_space(10.0);
            let sc_hex = format!(
                "sc#{}, {}, {}, {}",
                format_pc_float(color.alpha),
                format_pc_float(color.red),
                format_pc_float(color.green),
                format_pc_float(color.blue)
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("PC Hex:").color(text_dark()));
                let response = draw_copy_text(ui, &sc_hex, 225.0);
                if response.clicked() {
                    ui.output_mut(|output| output.copied_text = sc_hex.clone());
                }
            });
            if !editable {
                ui.small(RichText::new("Click PC Hex to copy").color(subtle_dark()));
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("OK").clicked() {
                    if let Some(target) = color.function_draft_color {
                        let argb = ((target.original_alpha as u32) << 24)
                            | ((float_channel_to_u8(color.red) as u32) << 16)
                            | ((float_channel_to_u8(color.green) as u32) << 8)
                            | float_channel_to_u8(color.blue) as u32;
                        result = Some(ColorPopupResult::FunctionDraftColor {
                            target: target.target,
                            argb,
                        });
                    } else if let Some(field) = color.write_color_field.clone() {
                        // Plain color value: emit the channel string the field
                        // parser expects (RGB = "r, g, b", ARGB = "a, r, g, b").
                        let input = if field.argb {
                            format!(
                                "{}, {}, {}, {}",
                                color.alpha, color.red, color.green, color.blue
                            )
                        } else {
                            format!("{}, {}, {}", color.red, color.green, color.blue)
                        };
                        result = Some(ColorPopupResult::FieldEdit {
                            tag_key: color.tag_key.clone(),
                            edit: PendingFieldEdit {
                                path: field.path,
                                input,
                            },
                        });
                    } else if let Some(path) = color.write_path.clone() {
                        let hex = constant_color_function_hex(
                            color.red,
                            color.green,
                            color.blue,
                            color.alpha,
                        );
                        result = Some(ColorPopupResult::FieldEdit {
                            tag_key: color.tag_key.clone(),
                            edit: PendingFieldEdit { path, input: hex },
                        });
                    } else if let Some(mut op) = color.create_shader_op.clone() {
                        op.initial_function_hex = constant_color_function_hex(
                            color.red,
                            color.green,
                            color.blue,
                            color.alpha,
                        );
                        result = Some(ColorPopupResult::ShaderOp {
                            tag_key: color.tag_key.clone(),
                            op,
                        });
                    } else if let Some(mut op) = color.create_shader_param_op.clone() {
                        if let Some(animated) = op.animated_parameters.first_mut() {
                            animated.initial_function_hex = constant_color_function_hex(
                                color.red,
                                color.green,
                                color.blue,
                                color.alpha,
                            );
                        }
                        result = Some(ColorPopupResult::ShaderParamOp {
                            tag_key: color.tag_key.clone(),
                            op,
                        });
                    } else if let Some(mut op) = color.create_h2_shader_param_op.clone() {
                        match &mut op {
                            H2ShaderParamOp::EditTemplateBackedValue { input, .. } => {
                                *input = format!("{}, {}, {}", color.red, color.green, color.blue);
                            }
                            H2ShaderParamOp::EnsureAnimationProperty {
                                initial_function_data,
                                ..
                            }
                            | H2ShaderParamOp::EditFunctionData {
                                data: initial_function_data,
                                ..
                            } => {
                                *initial_function_data = h2_constant_color_function_data(
                                    color.red,
                                    color.green,
                                    color.blue,
                                    color.alpha,
                                    Some(initial_function_data.as_slice()),
                                );
                            }
                            H2ShaderParamOp::SwitchTemplate { .. } => {}
                        }
                        result = Some(ColorPopupResult::H2ShaderParamOp {
                            tag_key: color.tag_key.clone(),
                            op,
                        });
                    }
                    close = true;
                }
                if editable && ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    if close || !open {
        *color_popup = None;
    }
    result
}

pub(in crate::app) fn draw_color_picker_editor(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<[u8; 4]>>,
    palette_last_dir: &mut Option<PathBuf>,
) {
    ui.horizontal(|ui| {
        draw_color_sv_square(ui, color);
        ui.add_space(8.0);
        draw_color_hue_strip(ui, color);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            draw_color_numeric_editor(ui, color);
            ui.add_space(8.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(84.0, 56.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            ui.painter()
                .rect_filled(rect.shrink(5.0), 0.0, color.color32());
        });
    });
    ui.add_space(8.0);
    draw_palette_grid(ui, color);
    ui.add_space(8.0);
    draw_custom_color_swatches(ui, color, custom_swatches);
    draw_palette_file_controls(ui, color, custom_swatches, palette_last_dir);
    ui.add_space(8.0);
    draw_editable_pc_hex(ui, color);
}

pub(in crate::app) fn draw_color_sv_square(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let size = Vec2::new(248.0, 268.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (h, s, b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    for y in 0..64 {
        let y0 = rect.top() + rect.height() * y as f32 / 64.0;
        let y1 = rect.top() + rect.height() * (y + 1) as f32 / 64.0;
        let bri = 1.0 - (y as f32 + 0.5) / 64.0;
        for x in 0..64 {
            let x0 = rect.left() + rect.width() * x as f32 / 64.0;
            let x1 = rect.left() + rect.width() * (x + 1) as f32 / 64.0;
            let sat = (x as f32 + 0.5) / 64.0;
            let (r, g, blue) = hsb_to_rgb(h as f32 / 255.0, sat, bri);
            ui.painter().rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
                0.0,
                Color32::from_rgb(
                    float_channel_to_u8(r),
                    float_channel_to_u8(g),
                    float_channel_to_u8(blue),
                ),
            );
        }
    }
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    let cursor = egui::pos2(
        egui::lerp(rect.left()..=rect.right(), s as f32 / 255.0),
        egui::lerp(rect.bottom()..=rect.top(), b as f32 / 255.0),
    );
    ui.painter()
        .circle_stroke(cursor, 5.0, Stroke::new(1.0, Color32::BLACK));
    ui.painter()
        .circle_stroke(cursor, 4.0, Stroke::new(1.0, Color32::WHITE));
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let sat = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let bri = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let (r, g, b) = hsb_to_rgb(h as f32 / 255.0, sat, bri);
            color.red = r;
            color.green = g;
            color.blue = b;
        }
    }
}

pub(in crate::app) fn draw_color_hue_strip(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (h, s, b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 268.0), Sense::click_and_drag());
    for i in 0..128 {
        let t0 = i as f32 / 128.0;
        let t1 = (i + 1) as f32 / 128.0;
        let hue = 1.0 - (i as f32 + 0.5) / 128.0;
        let (r, g, blue) = hsb_to_rgb(hue, 1.0, 1.0);
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), egui::lerp(rect.top()..=rect.bottom(), t0)),
                egui::pos2(rect.right(), egui::lerp(rect.top()..=rect.bottom(), t1)),
            ),
            0.0,
            Color32::from_rgb(
                float_channel_to_u8(r),
                float_channel_to_u8(g),
                float_channel_to_u8(blue),
            ),
        );
    }
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    let marker_y = egui::lerp(rect.bottom()..=rect.top(), h as f32 / 255.0);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() - 4.0, marker_y),
            egui::pos2(rect.right() + 4.0, marker_y),
        ],
        Stroke::new(1.0, Color32::BLACK),
    );
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hue = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let (r, g, blue) = hsb_to_rgb(hue, s as f32 / 255.0, b as f32 / 255.0);
            color.red = r;
            color.green = g;
            color.blue = blue;
        }
    }
}

pub(in crate::app) fn draw_color_numeric_editor(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (mut h, mut s, mut b) = rgb_to_hsb_255(color.red, color.green, color.blue);
    egui::Grid::new("material_color_picker_values")
        .spacing(Vec2::new(6.0, 4.0))
        .show(ui, |ui| {
            ui.label("");
            ui.label(RichText::new("Xenon").color(text_dark()).small());
            ui.label(RichText::new("PC").color(text_dark()).small());
            ui.end_row();
            let h_pc = h as f32 / 255.0;
            let s_pc = s as f32 / 255.0;
            let b_pc = b as f32 / 255.0;
            let h_changed = draw_color_byte_row(ui, "H:", &mut h, h_pc);
            let s_changed = draw_color_byte_row(ui, "S:", &mut s, s_pc);
            let b_changed = draw_color_byte_row(ui, "B:", &mut b, b_pc);
            if h_changed || s_changed || b_changed {
                let (r, g, blue) = hsb_to_rgb(h as f32 / 255.0, s as f32 / 255.0, b as f32 / 255.0);
                color.red = r;
                color.green = g;
                color.blue = blue;
            }
            let mut r = float_channel_to_u8(color.red);
            let mut g = float_channel_to_u8(color.green);
            let mut blue = float_channel_to_u8(color.blue);
            let mut a = float_channel_to_u8(color.alpha);
            if draw_color_byte_row(ui, "R:", &mut r, color.red) {
                color.red = byte_to_float(r);
            }
            if draw_color_byte_row(ui, "G:", &mut g, color.green) {
                color.green = byte_to_float(g);
            }
            if draw_color_byte_row(ui, "B:", &mut blue, color.blue) {
                color.blue = byte_to_float(blue);
            }
            if draw_color_byte_row(ui, "A:", &mut a, color.alpha) {
                color.alpha = byte_to_float(a);
            }
        });
}

pub(in crate::app) fn draw_color_byte_row(
    ui: &mut Ui,
    label: &str,
    value: &mut u8,
    pc: f32,
) -> bool {
    ui.label(RichText::new(label).color(text_dark()).strong());
    let mut v = *value as i32;
    let changed = ui
        .add_sized(
            Vec2::new(48.0, 20.0),
            egui::DragValue::new(&mut v).range(0..=255).speed(1.0),
        )
        .changed();
    if changed {
        *value = v.clamp(0, 255) as u8;
    }
    let mut pc_value = pc;
    let pc_changed = ui
        .add_sized(
            Vec2::new(54.0, 20.0),
            egui::DragValue::new(&mut pc_value)
                .range(0.0..=1.0)
                .speed(0.01),
        )
        .changed();
    if pc_changed {
        *value = float_channel_to_u8(pc_value);
    }
    ui.end_row();
    changed || pc_changed
}

pub(in crate::app) fn draw_palette_grid(ui: &mut Ui, color: &mut MaterialColorPopup) {
    const PALETTE: &[(u8, u8, u8)] = &[
        (255, 0, 0),
        (0, 255, 0),
        (0, 0, 255),
        (255, 255, 0),
        (0, 255, 255),
        (255, 0, 255),
        (255, 255, 255),
        (224, 224, 224),
        (192, 192, 192),
        (160, 160, 160),
        (128, 128, 128),
        (96, 96, 96),
        (64, 64, 64),
        (32, 32, 32),
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (0, 0, 128),
        (128, 128, 0),
        (0, 128, 128),
        (128, 0, 128),
        (255, 128, 128),
        (128, 255, 128),
        (128, 128, 255),
        (255, 192, 128),
        (255, 128, 0),
        (128, 64, 0),
        (64, 32, 0),
        (255, 220, 180),
        (180, 120, 80),
        (90, 50, 35),
        (60, 32, 24),
        (255, 180, 220),
        (220, 90, 160),
        (140, 50, 120),
        (70, 30, 80),
        (180, 220, 255),
        (90, 160, 220),
        (40, 100, 180),
        (20, 60, 110),
        (210, 255, 180),
        (140, 220, 80),
        (80, 160, 40),
        (40, 90, 24),
        (240, 240, 220),
        (210, 200, 150),
        (160, 145, 90),
        (95, 85, 55),
    ];
    egui::Grid::new("material_color_palette")
        .spacing(Vec2::new(5.0, 5.0))
        .show(ui, |ui| {
            for (i, &(r, g, b)) in PALETTE.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_rgb(r, g, b));
                ui.painter()
                    .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
                if response.clicked() {
                    color.set_rgb_bytes(r, g, b);
                }
                if (i + 1) % 16 == 0 {
                    ui.end_row();
                }
            }
        });
}

pub(in crate::app) fn draw_custom_color_swatches(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<[u8; 4]>>,
) {
    if custom_swatches.len() < CUSTOM_COLOR_SWATCH_COUNT {
        custom_swatches.resize(CUSTOM_COLOR_SWATCH_COUNT, None);
    }
    ui.label(
        RichText::new("Custom swatches")
            .color(subtle_dark())
            .small(),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
        for index in 0..CUSTOM_COLOR_SWATCH_COUNT {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
            match custom_swatches[index] {
                Some([r, g, b, a]) => {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(r, g, b, a),
                    );
                    if response.clicked() {
                        color.set_rgba_bytes(r, g, b, a);
                    }
                }
                None => {
                    draw_empty_custom_swatch(ui, rect);
                }
            }
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
            if response.secondary_clicked() {
                custom_swatches[index] = Some([
                    float_channel_to_u8(color.red),
                    float_channel_to_u8(color.green),
                    float_channel_to_u8(color.blue),
                    float_channel_to_u8(color.alpha),
                ]);
            }
            response
                .on_hover_text("Left-click to apply. Right-click to save the current colour here.");
            if (index + 1) % 16 == 0 {
                ui.end_row();
            }
        }
    });
    if ui
        .small_button("Save current colour to first empty slot")
        .clicked()
    {
        let slot = custom_swatches
            .iter()
            .position(Option::is_none)
            .unwrap_or(0);
        custom_swatches[slot] = Some([
            float_channel_to_u8(color.red),
            float_channel_to_u8(color.green),
            float_channel_to_u8(color.blue),
            float_channel_to_u8(color.alpha),
        ]);
    }
}

pub(in crate::app) fn draw_palette_file_controls(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<[u8; 4]>>,
    palette_last_dir: &mut Option<PathBuf>,
) {
    ui.horizontal(|ui| {
        if ui.small_button("Save Palette...").clicked() {
            match save_custom_palette(custom_swatches, palette_last_dir) {
                Ok(Some(path)) => {
                    color.palette_status = Some(format!("Saved palette: {}", path.display()))
                }
                Ok(None) => {}
                Err(error) => color.palette_status = Some(error),
            }
        }
        if ui.small_button("Load Palette...").clicked() {
            match load_custom_palette(palette_last_dir) {
                Ok(Some(swatches)) => {
                    *custom_swatches = swatches;
                    color.palette_status = Some("Loaded palette".to_owned());
                    color.confirm_clear_palette = false;
                }
                Ok(None) => {}
                Err(error) => color.palette_status = Some(error),
            }
        }
        if ui.small_button("Clear Palette").clicked() {
            color.confirm_clear_palette = true;
        }
    });
    if color.confirm_clear_palette {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Clear all custom swatches?").color(text_dark()));
            if ui.small_button("Clear").clicked() {
                *custom_swatches = vec![None; CUSTOM_COLOR_SWATCH_COUNT];
                color.palette_status = Some("Cleared palette".to_owned());
                color.confirm_clear_palette = false;
            }
            if ui.small_button("Cancel").clicked() {
                color.confirm_clear_palette = false;
            }
        });
    }
    if let Some(status) = color.palette_status.as_deref() {
        ui.small(RichText::new(status).color(subtle_dark()));
    }
}

pub(in crate::app) fn save_custom_palette(
    custom_swatches: &[Option<[u8; 4]>],
    palette_last_dir: &mut Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let start_dir = palette_last_dir
        .clone()
        .or_else(documents_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let Some(mut path) = rfd::FileDialog::new()
        .set_title("Save Baboon Palette")
        .add_filter("Baboon Palette", &["baboon_palette"])
        .set_directory(start_dir)
        .set_file_name("palette.baboon_palette")
        .save_file()
    else {
        return Ok(None);
    };
    if path.extension().and_then(|ext| ext.to_str()) != Some("baboon_palette") {
        path.set_extension("baboon_palette");
    }
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("Untitled");
    let text = encode_baboon_palette(name, custom_swatches);
    std::fs::write(&path, text).map_err(|error| format!("Could not save palette: {error}"))?;
    if let Some(parent) = path.parent() {
        *palette_last_dir = Some(parent.to_path_buf());
    }
    Ok(Some(path))
}

pub(in crate::app) fn load_custom_palette(
    palette_last_dir: &mut Option<PathBuf>,
) -> Result<Option<Vec<Option<[u8; 4]>>>, String> {
    let start_dir = palette_last_dir
        .clone()
        .or_else(documents_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let Some(path) = rfd::FileDialog::new()
        .set_title("Load Baboon Palette")
        .add_filter("Baboon Palette", &["baboon_palette"])
        .set_directory(start_dir)
        .pick_file()
    else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not load palette: {error}"))?;
    let swatches = decode_baboon_palette(&text)?;
    if let Some(parent) = path.parent() {
        *palette_last_dir = Some(parent.to_path_buf());
    }
    Ok(Some(swatches))
}

pub(in crate::app) fn encode_baboon_palette(name: &str, swatches: &[Option<[u8; 4]>]) -> String {
    let mut out = String::new();
    out.push_str("# Baboon Colour Palette\n");
    out.push_str("# Name: ");
    out.push_str(if name.trim().is_empty() {
        "Untitled"
    } else {
        name.trim()
    });
    out.push('\n');
    for index in 0..CUSTOM_COLOR_SWATCH_COUNT {
        match swatches.get(index).copied().flatten() {
            Some([r, g, b, a]) => out.push_str(&format!("#{r:02X}{g:02X}{b:02X}{a:02X}\n")),
            None => out.push_str("#empty\n"),
        }
    }
    out
}

pub(in crate::app) fn decode_baboon_palette(text: &str) -> Result<Vec<Option<[u8; 4]>>, String> {
    let mut swatches = Vec::with_capacity(CUSTOM_COLOR_SWATCH_COUNT);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("# Baboon Colour Palette")
            || trimmed.starts_with("# Name:")
        {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("#empty") {
            swatches.push(None);
        } else if let Some(color) = parse_palette_rgba(trimmed) {
            swatches.push(Some(color));
        } else if trimmed.starts_with('#') {
            continue;
        } else {
            return Err(format!("Invalid palette entry: {trimmed}"));
        }
        if swatches.len() >= CUSTOM_COLOR_SWATCH_COUNT {
            break;
        }
    }
    swatches.resize(CUSTOM_COLOR_SWATCH_COUNT, None);
    Ok(swatches)
}

fn parse_palette_rgba(text: &str) -> Option<[u8; 4]> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
        u8::from_str_radix(&hex[6..8], 16).ok()?,
    ])
}

fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .map(|home| home.join("Documents"))
        .filter(|path| path.is_dir())
}

fn draw_empty_custom_swatch(ui: &mut Ui, rect: egui::Rect) {
    ui.painter()
        .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
    let stroke = Stroke::new(1.0, subtle_dark());
    ui.painter()
        .line_segment([rect.left_top(), rect.right_bottom()], stroke);
    ui.painter()
        .line_segment([rect.right_top(), rect.left_bottom()], stroke);
}

pub(in crate::app) fn draw_editable_pc_hex(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let current_hex = format_rgb_hex(color.red, color.green, color.blue);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Hex:").color(text_dark()));
        let response = ui.add_sized(
            Vec2::new(118.0, 20.0),
            egui::TextEdit::singleline(&mut color.pc_hex_input)
                .hint_text("#RRGGBB")
                .desired_width(118.0),
        );
        if !response.has_focus() && color.pc_hex_input != current_hex {
            color.pc_hex_input = current_hex;
        }
        let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if response.lost_focus() && enter_pressed {
            match parse_rgb_hex(&color.pc_hex_input) {
                Ok([r, g, b]) => color.set_rgb_bytes(r, g, b),
                Err(error) => color.pc_hex_error = Some(error),
            }
        }
    });
    if let Some(error) = color.pc_hex_error.as_deref() {
        ui.small(RichText::new(error).color(Color32::from_rgb(220, 80, 80)));
    }
}

pub(in crate::app) fn hsb_to_rgb(h: f32, s: f32, b: f32) -> (f32, f32, f32) {
    let h = (h.fract() * 6.0).clamp(0.0, 5.999);
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = b * (1.0 - s);
    let q = b * (1.0 - s * f);
    let t = b * (1.0 - s * (1.0 - f));
    match i {
        0 => (b, t, p),
        1 => (q, b, p),
        2 => (p, b, t),
        3 => (p, q, b),
        4 => (t, p, b),
        _ => (b, p, q),
    }
}

pub(in crate::app) fn draw_color_channel_table(ui: &mut Ui, color: &MaterialColorPopup) {
    let (hue, saturation, brightness) = rgb_to_hsb_255(color.red, color.green, color.blue);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add_space(34.0);
            ui.label(RichText::new("0-255").color(subtle_dark()));
            ui.add_space(23.0);
            ui.label(RichText::new("PC (float)").color(subtle_dark()));
        });
        egui::Grid::new("material_color_channels")
            .spacing(Vec2::new(6.0, 4.0))
            .show(ui, |ui| {
                draw_color_channel_row(ui, "R:", float_channel_to_u8(color.red), color.red);
                draw_color_channel_row(ui, "G:", float_channel_to_u8(color.green), color.green);
                draw_color_channel_row(ui, "B:", float_channel_to_u8(color.blue), color.blue);
                draw_color_channel_row(ui, "A:", float_channel_to_u8(color.alpha), color.alpha);
                draw_hsb_row(ui, "H:", hue);
                draw_hsb_row(ui, "S:", saturation);
                draw_hsb_row(ui, "B:", brightness);
            });
    });
}

pub(in crate::app) fn draw_color_channel_row(
    ui: &mut Ui,
    label: &str,
    channel_255: u8,
    channel_float: f32,
) {
    ui.label(RichText::new(label).color(text_dark()).strong());
    draw_copy_text(ui, &channel_255.to_string(), 56.0);
    draw_copy_text(ui, &format_pc_float(channel_float), 72.0);
    ui.end_row();
}

pub(in crate::app) fn draw_hsb_row(ui: &mut Ui, label: &str, value: u8) {
    ui.label(RichText::new(label).color(text_dark()).strong());
    draw_copy_text(ui, &value.to_string(), 56.0);
    ui.label("");
    ui.end_row();
}

pub(in crate::app) fn draw_copy_text(ui: &mut Ui, value: &str, width: f32) -> egui::Response {
    let height = 22.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let fill = if response.hovered() {
        Color32::from_rgb(246, 246, 244)
    } else {
        Color32::from_rgb(238, 238, 235)
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, MATERIAL_INPUT_EDGE));
    ui.painter().text(
        rect.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        truncate_for_cell(value, width - 10.0),
        FontId::monospace(12.5),
        MATERIAL_TEXT,
    );
    response
}

pub(in crate::app) fn truncate_for_cell(text: &str, width: f32) -> String {
    let max_chars = (width / 7.0).floor().max(8.0) as usize;
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}
