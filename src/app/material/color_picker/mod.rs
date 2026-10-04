//! Shared color picker, swatches, and Baboon palette persistence.
//! It owns material-specific presentation and color workflows; generic field editing and document persistence belong elsewhere.

use super::*;

#[derive(Clone)]
pub(in crate::app) struct MaterialColorPopup {
    title: String,
    original_color: [f32; 4],
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    hue: u8,
    saturation: u8,
    brightness: u8,
    alpha_available: bool,
    pub(in crate::app) sc_hex: String,
    hex_input: String,
    hex_error: Option<String>,
    palette_status: Option<String>,
    confirm_clear_palette: bool,
    show_save_palette_dialog: bool,
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
    /// A logical color index (0..color count) of the function being edited.
    Logical(usize),
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
        let (hue, saturation, brightness) = rgb_to_hsb_255(red, green, blue);
        Self {
            title: clean_field_name(title),
            original_color: [red, green, blue, alpha],
            red,
            green,
            blue,
            alpha,
            hue,
            saturation,
            brightness,
            alpha_available: true,
            sc_hex: format!(
                "sc#{}, {}, {}, {}",
                format_pc_float(alpha),
                format_pc_float(red),
                format_pc_float(green),
                format_pc_float(blue)
            ),
            hex_input: format_rgb_hex(red, green, blue),
            hex_error: None,
            palette_status: None,
            confirm_clear_palette: false,
            show_save_palette_dialog: false,
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
        self = self.with_alpha_available(argb);
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
        self.alpha_available = !matches!(&op, H2ShaderParamOp::EditTemplateBackedValue { .. });
        if !self.alpha_available {
            self.alpha = 1.0;
            self.original_color[3] = 1.0;
        }
        self.tag_key = tag_key.into();
        self.create_h2_shader_param_op = Some(op);
        self
    }

    pub(in crate::app) fn with_function_draft_color(
        mut self,
        target: FunctionDraftColorTarget,
        original_alpha: u8,
    ) -> Self {
        self = self.with_alpha_available(false);
        self.function_draft_color = Some(FunctionDraftColorWrite {
            target,
            original_alpha,
        });
        self
    }

    pub(in crate::app) fn with_alpha_available(mut self, available: bool) -> Self {
        self.alpha_available = available;
        if !available {
            self.alpha = 1.0;
            self.original_color[3] = 1.0;
        }
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

    fn original_color32(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(
            float_channel_to_u8(self.original_color[0]),
            float_channel_to_u8(self.original_color[1]),
            float_channel_to_u8(self.original_color[2]),
            float_channel_to_u8(self.original_color[3]),
        )
    }

    fn set_rgb_bytes(&mut self, red: u8, green: u8, blue: u8) {
        self.set_rgb_components(
            byte_to_float(red),
            byte_to_float(green),
            byte_to_float(blue),
        );
        self.hex_input = format!("#{red:02X}{green:02X}{blue:02X}");
        self.hex_error = None;
    }

    fn set_rgb_components(&mut self, red: f32, green: f32, blue: f32) {
        self.red = red.clamp(0.0, 1.0);
        self.green = green.clamp(0.0, 1.0);
        self.blue = blue.clamp(0.0, 1.0);
        let (hue, saturation, brightness) = rgb_to_hsb_255(self.red, self.green, self.blue);
        self.brightness = brightness;
        if brightness != 0 {
            self.saturation = saturation;
            if saturation != 0 {
                self.hue = hue;
            }
        }
    }

    fn update_rgb_from_hsb(&mut self) {
        let (red, green, blue) = hsb_to_rgb(
            self.hue as f32 / 255.0,
            self.saturation as f32 / 255.0,
            self.brightness as f32 / 255.0,
        );
        self.red = red;
        self.green = green;
        self.blue = blue;
        self.hex_input = format_rgb_hex(red, green, blue);
        self.hex_error = None;
    }

    fn set_rgba_bytes(&mut self, red: u8, green: u8, blue: u8, alpha: u8) {
        self.set_rgb_bytes(red, green, blue);
        if self.alpha_available {
            self.alpha = byte_to_float(alpha);
        }
    }
}

pub(in crate::app) fn color_popup_for_value(
    title: &str,
    value: &TagFieldData,
    formatted: &str,
) -> Option<MaterialColorPopup> {
    match value {
        TagFieldData::RealRgbColor(color) => Some(
            MaterialColorPopup::new(title, color.red, color.green, color.blue, 1.0)
                .with_alpha_available(false),
        ),
        TagFieldData::RealArgbColor(color) => Some(MaterialColorPopup::new(
            title,
            color.red,
            color.green,
            color.blue,
            color.alpha,
        )),
        TagFieldData::RgbColor(color) => {
            let raw = color.0;
            Some(
                MaterialColorPopup::new(
                    title,
                    byte_to_float(((raw >> 16) & 0xFF) as u8),
                    byte_to_float(((raw >> 8) & 0xFF) as u8),
                    byte_to_float((raw & 0xFF) as u8),
                    1.0,
                )
                .with_alpha_available(false),
            )
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
    custom_swatches: &mut Vec<Option<ColorPaletteSwatch>>,
    palette_last_dir: &mut Option<PathBuf>,
) -> Option<ColorPopupResult> {
    let color = color_popup.as_mut()?;
    let mut close = false;
    let editable = color.write_path.is_some()
        || color.write_color_field.is_some()
        || color.create_shader_op.is_some()
        || color.create_shader_param_op.is_some()
        || color.create_h2_shader_param_op.is_some()
        || color.function_draft_color.is_some();
    let mut result: Option<ColorPopupResult> = None;
    let window_title = format!("Color Picker - {}", color.title);
    egui::Window::new("Color Picker")
        .constrain_to(window_work_area(ctx))
        .id(egui::Id::new("material_color_picker"))
        .title_bar(false)
        .collapsible(false)
        .movable(true)
        .resizable(false)
        .default_size(window_size(ctx, Vec2::new(560.0, 480.0), false))
        .show(ctx, |ui| {
            super::super::ui::draw_icon_window_header_without_close(
                ui,
                &window_title,
                ButtonIcon::ColorPicker,
            );
            ui.separator();
            if editable {
                draw_color_picker_editor(ui, color, custom_swatches);
            } else {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(80.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, color.color32());
                    ui.painter()
                        .rect_stroke(
                            rect,
                            0.0,
                            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
                            egui::StrokeKind::Middle,
                        );
                    ui.add_space(14.0);
                    draw_color_channel_table(ui, color);
                });
            }
            if !editable {
                ui.add_space(10.0);
                let sc_hex = current_pc_hex(color);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("PC Hex:").color(text_dark()));
                    let response = draw_copy_text(ui, &sc_hex, 225.0);
                    if response.clicked() {
                        ui.copy_text(sc_hex.clone());
                    }
                });
                ui.small(RichText::new("Click PC Hex to copy").color(subtle_dark()));
            } else {
                draw_palette_feedback(ui, color, custom_swatches);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if editable {
                    draw_palette_file_buttons(ui, color, custom_swatches, palette_last_dir);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if editable && icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked()
                    {
                        close = true;
                    }
                    if icon_text_button(ui, ButtonIcon::Confirm, "OK", true).clicked() {
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
                                    *input =
                                        format!("{}, {}, {}", color.red, color.green, color.blue);
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
                });
            });
        });
    if editable && !close && color.show_save_palette_dialog {
        draw_save_palette_format_dialog(ctx, color, custom_swatches, palette_last_dir);
    }
    if close {
        *color_popup = None;
    }
    result
}

pub(in crate::app) fn draw_color_picker_editor(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<ColorPaletteSwatch>>,
) {
    ui.add_space(10.0);
    let alpha_width = if color.alpha_available {
        COLOR_SLIDER_GAP + COLOR_SLIDER_WIDTH
    } else {
        0.0
    };
    let color_controls_width = 248.0 + COLOR_SLIDER_GAP + COLOR_SLIDER_WIDTH + alpha_width;
    let top_row_width = color_controls_width + 18.0 + COLOR_NUMERIC_PANEL_WIDTH;
    let left_padding = ((ui.available_width() - top_row_width) * 0.5).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(left_padding);
        ui.allocate_ui_with_layout(
            Vec2::new(color_controls_width, COLOR_TOP_ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                draw_color_sv_square(ui, color);
                ui.add_space(COLOR_SLIDER_GAP);
                draw_color_hue_strip(ui, color);
                if color.alpha_available {
                    ui.add_space(COLOR_SLIDER_GAP);
                    draw_color_alpha_strip(ui, color);
                }
            },
        );
        ui.add_space(18.0);
        ui.allocate_ui_with_layout(
            Vec2::new(COLOR_NUMERIC_PANEL_WIDTH, COLOR_TOP_ROW_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| draw_color_numeric_editor(ui, color),
        );
    });
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        draw_custom_color_swatches(ui, color, custom_swatches);
        ui.add_space(6.0);
        let (divider, _) =
            ui.allocate_exact_size(Vec2::new(1.0, color_palette_grid_size().y), Sense::hover());
        ui.painter().vline(
            divider.center().x,
            divider.y_range(),
            Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
        ui.add_space(6.0);
        draw_color_comparison(ui, color);
    });
    ui.add_space(6.0);
    ui.separator();
}

fn draw_color_comparison(ui: &mut Ui, color: &MaterialColorPopup) {
    let (container, response) =
        ui.allocate_exact_size(Vec2::new(84.0, color_palette_grid_size().y), Sense::hover());
    let font = TextStyle::Small.resolve(ui.style());
    let current_label =
        ui.painter()
            .layout_no_wrap("current".to_owned(), font.clone(), subtle_dark());
    let new_label = ui
        .painter()
        .layout_no_wrap("new".to_owned(), font, subtle_dark());
    const LABEL_GAP: f32 = 3.0;
    let content_height = current_label.size().y + LABEL_GAP + 56.0 + LABEL_GAP + new_label.size().y;
    let content_top = container.center().y - content_height * 0.5;
    let current_pos = egui::pos2(
        container.center().x - current_label.size().x * 0.5,
        content_top,
    );
    ui.painter()
        .galley(current_pos, current_label.clone(), subtle_dark());
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            container.left(),
            current_pos.y + current_label.size().y + LABEL_GAP,
        ),
        Vec2::new(84.0, 56.0),
    );
    let current_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.center().y));
    let new_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.center().y), rect.max);
    if color.alpha_available {
        paint_alpha_checkerboard(ui.painter(), rect);
    }
    ui.painter()
        .rect_filled(current_rect, 0.0, color.original_color32());
    ui.painter().rect_filled(new_rect, 0.0, color.color32());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.center().y),
            egui::pos2(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
    );
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
            egui::StrokeKind::Middle,
        );
    ui.painter().galley(
        egui::pos2(
            container.center().x - new_label.size().x * 0.5,
            rect.bottom() + LABEL_GAP,
        ),
        new_label,
        subtle_dark(),
    );
    response.on_hover_cursor(egui::CursorIcon::Default);
}

fn paint_sv_gradient(painter: &egui::Painter, rect: egui::Rect, hue: f32) {
    const STEPS: usize = 64;
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((STEPS + 1) * (STEPS + 1));
    mesh.reserve_triangles(STEPS * STEPS * 2);
    for y in 0..=STEPS {
        let brightness = 1.0 - y as f32 / STEPS as f32;
        for x in 0..=STEPS {
            let saturation = x as f32 / STEPS as f32;
            let (red, green, blue) = hsb_to_rgb(hue, saturation, brightness);
            mesh.colored_vertex(
                egui::pos2(
                    egui::lerp(rect.left()..=rect.right(), saturation),
                    egui::lerp(rect.top()..=rect.bottom(), y as f32 / STEPS as f32),
                ),
                Color32::from_rgb(
                    float_channel_to_u8(red),
                    float_channel_to_u8(green),
                    float_channel_to_u8(blue),
                ),
            );
        }
    }
    let stride = (STEPS + 1) as u32;
    for y in 0..STEPS as u32 {
        for x in 0..STEPS as u32 {
            let top_left = y * stride + x;
            let top_right = top_left + 1;
            let bottom_left = top_left + stride;
            let bottom_right = bottom_left + 1;
            mesh.add_triangle(top_left, top_right, bottom_right);
            mesh.add_triangle(top_left, bottom_right, bottom_left);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_hue_gradient(painter: &egui::Painter, rect: egui::Rect) {
    const STEPS: usize = 128;
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((STEPS + 1) * 2);
    mesh.reserve_triangles(STEPS * 2);
    for step in 0..=STEPS {
        let position = step as f32 / STEPS as f32;
        let (red, green, blue) = hsb_to_rgb(1.0 - position, 1.0, 1.0);
        let color = Color32::from_rgb(
            float_channel_to_u8(red),
            float_channel_to_u8(green),
            float_channel_to_u8(blue),
        );
        let y = egui::lerp(rect.top()..=rect.bottom(), position);
        mesh.colored_vertex(egui::pos2(rect.left(), y), color);
        mesh.colored_vertex(egui::pos2(rect.right(), y), color);
    }
    for step in 0..STEPS as u32 {
        let top_left = step * 2;
        let top_right = top_left + 1;
        let bottom_left = top_left + 2;
        let bottom_right = top_left + 3;
        mesh.add_triangle(top_left, top_right, bottom_right);
        mesh.add_triangle(top_left, bottom_right, bottom_left);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_alpha_gradient(painter: &egui::Painter, rect: egui::Rect) {
    const STEPS: usize = 64;
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((STEPS + 1) * 2);
    mesh.reserve_triangles(STEPS * 2);
    for step in 0..=STEPS {
        let position = step as f32 / STEPS as f32;
        let opacity = float_channel_to_u8(1.0 - position);
        let color = Color32::from_white_alpha(opacity);
        let y = egui::lerp(rect.top()..=rect.bottom(), position);
        mesh.colored_vertex(egui::pos2(rect.left(), y), color);
        mesh.colored_vertex(egui::pos2(rect.right(), y), color);
    }
    for step in 0..STEPS as u32 {
        let top_left = step * 2;
        let top_right = top_left + 1;
        let bottom_left = top_left + 2;
        let bottom_right = top_left + 3;
        mesh.add_triangle(top_left, top_right, bottom_right);
        mesh.add_triangle(top_left, bottom_right, bottom_left);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_color_slider_marker(painter: &egui::Painter, strip: egui::Rect, y: f32, fill: Color32) {
    let outer = egui::Rect::from_center_size(
        egui::pos2(strip.center().x, y),
        Vec2::new(strip.width() + 8.0, 6.0),
    );
    painter.rect_filled(outer, 0.0, Color32::BLACK);
    painter.rect_filled(outer.shrink(1.0), 0.0, Color32::WHITE);
    painter.rect_filled(outer.shrink(2.0), 0.0, fill);
}

pub(in crate::app) fn draw_color_sv_square(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let size = Vec2::new(248.0, 268.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    paint_sv_gradient(ui.painter(), rect, color.hue as f32 / 255.0);
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
            egui::StrokeKind::Middle,
        );
    let cursor = egui::pos2(
        egui::lerp(rect.left()..=rect.right(), color.saturation as f32 / 255.0),
        egui::lerp(rect.bottom()..=rect.top(), color.brightness as f32 / 255.0),
    );
    let selected = Color32::from_rgb(
        float_channel_to_u8(color.red),
        float_channel_to_u8(color.green),
        float_channel_to_u8(color.blue),
    );
    ui.painter().circle_filled(cursor, 4.0, selected);
    ui.painter()
        .circle_stroke(cursor, 6.0, Stroke::new(1.0_f32, Color32::BLACK));
    ui.painter()
        .circle_stroke(cursor, 5.0, Stroke::new(1.0_f32, Color32::WHITE));
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let sat = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let bri = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            color.saturation = float_channel_to_u8(sat);
            color.brightness = float_channel_to_u8(bri);
            color.update_rgb_from_hsb();
        }
    }
}

pub(in crate::app) fn draw_color_hue_strip(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(COLOR_SLIDER_WIDTH, 268.0),
        Sense::click_and_drag(),
    );
    paint_hue_gradient(ui.painter(), rect);
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
            egui::StrokeKind::Middle,
        );
    let marker_y = egui::lerp(rect.bottom()..=rect.top(), color.hue as f32 / 255.0);
    let (red, green, blue) = hsb_to_rgb(color.hue as f32 / 255.0, 1.0, 1.0);
    paint_color_slider_marker(
        ui.painter(),
        rect,
        marker_y,
        Color32::from_rgb(
            float_channel_to_u8(red),
            float_channel_to_u8(green),
            float_channel_to_u8(blue),
        ),
    );
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hue = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            color.hue = float_channel_to_u8(hue);
            color.update_rgb_from_hsb();
        }
    }
}

pub(in crate::app) fn draw_color_alpha_strip(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(COLOR_SLIDER_WIDTH, 268.0),
        Sense::click_and_drag(),
    );
    paint_alpha_checkerboard(ui.painter(), rect);
    paint_alpha_gradient(ui.painter(), rect);
    ui.painter()
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
            egui::StrokeKind::Middle,
        );
    let marker_y = egui::lerp(rect.bottom()..=rect.top(), color.alpha);
    let alpha = float_channel_to_u8(color.alpha);
    paint_color_slider_marker(ui.painter(), rect, marker_y, Color32::from_gray(alpha));
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            color.alpha = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        }
    }
}

pub(in crate::app) fn draw_color_numeric_editor(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let (mut h, mut s, mut b) = (color.hue, color.saturation, color.brightness);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                Vec2::new(COLOR_ROW_LABEL_WIDTH, COLOR_ROW_HEIGHT),
                egui::Label::new(""),
            );
            ui.add_sized(
                Vec2::new(48.0, 20.0),
                egui::Label::new(RichText::new("Xenon").color(text_dark()).small())
                    .halign(egui::Align::Center),
            )
            .on_hover_cursor(egui::CursorIcon::Default);
            ui.add_sized(
                Vec2::new(54.0, 20.0),
                egui::Label::new(RichText::new("PC").color(text_dark()).small())
                    .halign(egui::Align::Center),
            )
            .on_hover_cursor(egui::CursorIcon::Default);
        });
        let h_pc = h as f32 / 255.0;
        let s_pc = s as f32 / 255.0;
        let b_pc = b as f32 / 255.0;
        let h_changed = draw_color_byte_row(ui, "H:", &mut h, h_pc);
        let s_changed = draw_color_byte_row(ui, "S:", &mut s, s_pc);
        let b_changed = draw_color_byte_row(ui, "B:", &mut b, b_pc);
        if h_changed || s_changed || b_changed {
            color.hue = h;
            color.saturation = s;
            color.brightness = b;
            color.update_rgb_from_hsb();
        }

        ui.separator();
        let mut r = float_channel_to_u8(color.red);
        let mut g = float_channel_to_u8(color.green);
        let mut blue = float_channel_to_u8(color.blue);
        let mut a = float_channel_to_u8(color.alpha);
        if draw_color_byte_row(ui, "R:", &mut r, color.red) {
            color.set_rgb_components(byte_to_float(r), color.green, color.blue);
        }
        if draw_color_byte_row(ui, "G:", &mut g, color.green) {
            color.set_rgb_components(color.red, byte_to_float(g), color.blue);
        }
        if draw_color_byte_row(ui, "B:", &mut blue, color.blue) {
            color.set_rgb_components(color.red, color.green, byte_to_float(blue));
        }
        if color.alpha_available && draw_color_byte_row(ui, "A:", &mut a, color.alpha) {
            color.alpha = byte_to_float(a);
        }

        ui.separator();
        draw_color_hex_rows(ui, color);
    });
}

pub(in crate::app) fn draw_color_byte_row(
    ui: &mut Ui,
    label: &str,
    value: &mut u8,
    pc: f32,
) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        draw_color_row_label(ui, label);
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
        changed || pc_changed
    })
    .inner
}

fn draw_color_row_label(ui: &mut Ui, label: &str) {
    ui.add_sized(
        Vec2::new(COLOR_ROW_LABEL_WIDTH, COLOR_ROW_HEIGHT),
        egui::Label::new(RichText::new(label).color(text_dark()).strong()).halign(egui::Align::Max),
    )
    .on_hover_cursor(egui::CursorIcon::Default);
}

const COLOR_ROW_LABEL_WIDTH: f32 = 48.0;
const COLOR_ROW_HEIGHT: f32 = BUTTON_HEIGHT;
const COLOR_COMBINED_FIELD_WIDTH: f32 = 108.0;
const COLOR_NUMERIC_PANEL_WIDTH: f32 = COLOR_ROW_LABEL_WIDTH + 6.0 + 48.0 + 6.0 + 54.0;
const COLOR_TOP_ROW_HEIGHT: f32 = 300.0;
const COLOR_SLIDER_WIDTH: f32 = 22.0;
const COLOR_SLIDER_GAP: f32 = 12.0;
const COLOR_PALETTE_COLUMNS: usize = 16;
const COLOR_SWATCH_SIZE: f32 = 24.0;
const COLOR_SWATCH_GAP: f32 = 4.0;

fn color_palette_grid_size() -> Vec2 {
    let rows = CUSTOM_COLOR_SWATCH_COUNT.div_ceil(COLOR_PALETTE_COLUMNS);
    Vec2::new(
        COLOR_PALETTE_COLUMNS as f32 * COLOR_SWATCH_SIZE
            + (COLOR_PALETTE_COLUMNS - 1) as f32 * COLOR_SWATCH_GAP,
        rows as f32 * COLOR_SWATCH_SIZE + (rows - 1) as f32 * COLOR_SWATCH_GAP,
    )
}

pub(in crate::app) fn draw_custom_color_swatches(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<ColorPaletteSwatch>>,
) {
    if custom_swatches.len() < CUSTOM_COLOR_SWATCH_COUNT {
        custom_swatches.resize(CUSTOM_COLOR_SWATCH_COUNT, None);
    }
    let size = color_palette_grid_size();
    let (palette_rect, _) = ui.allocate_exact_size(size, Sense::hover());
    for index in 0..CUSTOM_COLOR_SWATCH_COUNT {
        let column = index % COLOR_PALETTE_COLUMNS;
        let row = index / COLOR_PALETTE_COLUMNS;
        let min = palette_rect.min
            + Vec2::new(
                column as f32 * (COLOR_SWATCH_SIZE + COLOR_SWATCH_GAP),
                row as f32 * (COLOR_SWATCH_SIZE + COLOR_SWATCH_GAP),
            );
        let rect = egui::Rect::from_min_size(min, Vec2::splat(COLOR_SWATCH_SIZE));
        let response = ui.interact(
            rect,
            ui.id().with(("material_color_palette_swatch", index)),
            Sense::click(),
        );
        match custom_swatches[index].as_ref() {
            Some(swatch) => {
                let [r, g, b, a] = swatch.rgba;
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
                if response.clicked() {
                    color.set_rgba_bytes(r, g, b, a);
                }
            }
            None => draw_empty_custom_swatch(ui, rect),
        }
        ui.painter()
            .rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
                egui::StrokeKind::Middle,
            );
        if response.secondary_clicked() {
            custom_swatches[index] = Some(ColorPaletteSwatch::unnamed([
                float_channel_to_u8(color.red),
                float_channel_to_u8(color.green),
                float_channel_to_u8(color.blue),
                float_channel_to_u8(color.alpha),
            ]));
        }
        let instructions = "Left-click to apply. Right-click to save the current colour here.";
        if let Some(name) = custom_swatches[index]
            .as_ref()
            .and_then(|swatch| swatch.name.as_deref())
        {
            response.on_hover_text(format!("{name}\n{instructions}"));
        } else {
            response.on_hover_text(instructions);
        }
    }
}

pub(in crate::app) fn draw_palette_file_buttons(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<ColorPaletteSwatch>>,
    palette_last_dir: &mut Option<PathBuf>,
) {
    if ui
        .add(egui::Button::new("Load Palette...").min_size(Vec2::new(0.0, BUTTON_HEIGHT)))
        .clicked()
    {
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
    if icon_text_button(ui, ButtonIcon::Save, "Save Palette...", true).clicked() {
        color.show_save_palette_dialog = true;
    }
    if ui
        .add(egui::Button::new("Clear Palette").min_size(Vec2::new(0.0, BUTTON_HEIGHT)))
        .clicked()
    {
        color.confirm_clear_palette = true;
    }
}

fn draw_save_palette_format_dialog(
    ctx: &egui::Context,
    color: &mut MaterialColorPopup,
    custom_swatches: &[Option<ColorPaletteSwatch>],
    palette_last_dir: &mut Option<PathBuf>,
) {
    let mut open = true;
    egui::Window::new("Save Palette Format")
        .id(egui::Id::new("save_palette_format_dialog"))
        .title_bar(false)
        .collapsible(false)
        .movable(true)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .fixed_size(window_size(ctx, Vec2::new(420.0, 225.0), false))
        .show(ctx, |ui| {
            super::super::ui::draw_icon_window_header(
                ui,
                "Save Palette",
                ButtonIcon::Save,
                &mut open,
            );
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new("Choose a palette format:").color(text_dark()));
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new("Baboon Palette...")
                            .min_size(Vec2::new(132.0, BUTTON_HEIGHT)),
                    )
                    .clicked()
                {
                    match save_custom_palette(custom_swatches, palette_last_dir) {
                        Ok(Some(path)) => {
                            color.palette_status =
                                Some(format!("Saved palette: {}", path.display()));
                            open = false;
                        }
                        Ok(None) => {}
                        Err(error) => color.palette_status = Some(error),
                    }
                }
                ui.add_sized(
                    Vec2::new(245.0, 36.0),
                    egui::Label::new(
                        RichText::new(
                            "Native format; preserves RGBA transparency, names, and all 64 slots.",
                        )
                        .color(subtle_dark())
                        .small(),
                    )
                    .wrap(),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new("Halo 3 .txt...")
                            .min_size(Vec2::new(132.0, BUTTON_HEIGHT)),
                    )
                    .clicked()
                {
                    match save_halo3_palette(custom_swatches, palette_last_dir) {
                        Ok(Some(path)) => {
                            color.palette_status = Some(format!(
                                "Saved Halo 3 RGB palette: {}",
                                path.display()
                            ));
                            open = false;
                        }
                        Ok(None) => {}
                        Err(error) => color.palette_status = Some(error),
                    }
                }
                ui.add_sized(
                    Vec2::new(245.0, 36.0),
                    egui::Label::new(
                        RichText::new(
                            "Halo 3-compatible RGB format; preserves names but does not store alpha.",
                        )
                        .color(subtle_dark())
                        .small(),
                    )
                    .wrap(),
                );
            });

            ui.add_space(8.0);
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_text_button(ui, ButtonIcon::Clear, "Cancel", true).clicked() {
                    open = false;
                }
            });
        });
    if !open {
        color.show_save_palette_dialog = false;
    }
}

fn draw_palette_feedback(
    ui: &mut Ui,
    color: &mut MaterialColorPopup,
    custom_swatches: &mut Vec<Option<ColorPaletteSwatch>>,
) {
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
    custom_swatches: &[Option<ColorPaletteSwatch>],
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

pub(in crate::app) fn save_halo3_palette(
    custom_swatches: &[Option<ColorPaletteSwatch>],
    palette_last_dir: &mut Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let start_dir = palette_last_dir
        .clone()
        .or_else(documents_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let Some(mut path) = rfd::FileDialog::new()
        .set_title("Save Halo 3 Color Preferences")
        .add_filter("Halo 3 Color Preferences", &["txt"])
        .set_directory(start_dir)
        .set_file_name("color_preferences.txt")
        .save_file()
    else {
        return Ok(None);
    };
    if path.extension().and_then(|ext| ext.to_str()) != Some("txt") {
        path.set_extension("txt");
    }
    std::fs::write(&path, encode_halo3_color_preferences(custom_swatches))
        .map_err(|error| format!("Could not save Halo 3 palette: {error}"))?;
    if let Some(parent) = path.parent() {
        *palette_last_dir = Some(parent.to_path_buf());
    }
    Ok(Some(path))
}

pub(in crate::app) fn load_custom_palette(
    palette_last_dir: &mut Option<PathBuf>,
) -> Result<Option<Vec<Option<ColorPaletteSwatch>>>, String> {
    let start_dir = palette_last_dir
        .clone()
        .or_else(documents_dir)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let Some(path) = rfd::FileDialog::new()
        .set_title("Load Color Palette")
        .add_filter("Color Palettes", &["baboon_palette", "txt"])
        .add_filter("Baboon Palette", &["baboon_palette"])
        .add_filter("Halo 3 Color Preferences", &["txt"])
        .set_directory(start_dir)
        .pick_file()
    else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not load palette: {error}"))?;
    let swatches = decode_color_palette(&text)?;
    if let Some(parent) = path.parent() {
        *palette_last_dir = Some(parent.to_path_buf());
    }
    Ok(Some(swatches))
}

pub(in crate::app) fn encode_baboon_palette(
    name: &str,
    swatches: &[Option<ColorPaletteSwatch>],
) -> String {
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
        match swatches.get(index).and_then(Option::as_ref) {
            Some(swatch) => {
                let [r, g, b, a] = swatch.rgba;
                out.push_str(&format!("#{r:02X}{g:02X}{b:02X}{a:02X}"));
                if let Some(name) = swatch.name.as_deref() {
                    out.push('\t');
                    out.push_str(&name.replace(['\r', '\n', '\t'], " "));
                }
                out.push('\n');
            }
            None => out.push_str("#empty\n"),
        }
    }
    out
}

pub(in crate::app) fn encode_halo3_color_preferences(
    swatches: &[Option<ColorPaletteSwatch>],
) -> String {
    let mut out = String::new();
    for (slot, swatch) in swatches.iter().take(CUSTOM_COLOR_SWATCH_COUNT).enumerate() {
        let Some(swatch) = swatch else {
            continue;
        };
        let [red, green, blue, _alpha] = swatch.rgba;
        let name = swatch
            .name
            .as_deref()
            .unwrap_or("")
            .replace(['\r', '\n'], " ");
        out.push_str(&format!("{slot},{red},{green},{blue},{name}\r\n"));
    }
    out
}

pub(in crate::app) fn default_color_swatches() -> Vec<Option<ColorPaletteSwatch>> {
    decode_baboon_palette(include_root_str!("assets/default.baboon_palette"))
        .expect("the bundled Baboon palette must be valid")
}

pub(in crate::app) fn decode_color_palette(
    text: &str,
) -> Result<Vec<Option<ColorPaletteSwatch>>, String> {
    let first = text.lines().map(str::trim).find(|line| !line.is_empty());
    if first.is_some_and(|line| line.starts_with('#')) {
        decode_baboon_palette(text)
    } else {
        decode_halo3_color_preferences(text)
    }
}

pub(in crate::app) fn decode_baboon_palette(
    text: &str,
) -> Result<Vec<Option<ColorPaletteSwatch>>, String> {
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
        } else if let Some((color, name)) = parse_baboon_palette_entry(trimmed) {
            swatches.push(Some(ColorPaletteSwatch::named(color, name)));
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

fn parse_baboon_palette_entry(text: &str) -> Option<([u8; 4], &str)> {
    let (color, name) = text.split_once('\t').unwrap_or((text, ""));
    Some((parse_palette_rgba(color)?, name))
}

pub(in crate::app) fn decode_halo3_color_preferences(
    text: &str,
) -> Result<Vec<Option<ColorPaletteSwatch>>, String> {
    let mut swatches = vec![None; CUSTOM_COLOR_SWATCH_COUNT];
    let mut found = false;
    for (line_number, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut fields = trimmed.splitn(5, ',');
        let invalid = || {
            format!(
                "Invalid Halo 3 palette entry on line {}: {trimmed}",
                line_number + 1
            )
        };
        let slot = fields
            .next()
            .and_then(|value| {
                value
                    .trim()
                    .trim_start_matches('\u{feff}')
                    .parse::<usize>()
                    .ok()
            })
            .ok_or_else(invalid)?;
        let red = fields
            .next()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .ok_or_else(invalid)?;
        let green = fields
            .next()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .ok_or_else(invalid)?;
        let blue = fields
            .next()
            .and_then(|value| value.trim().parse::<u8>().ok())
            .ok_or_else(invalid)?;
        let name = fields.next().ok_or_else(invalid)?;
        if slot >= CUSTOM_COLOR_SWATCH_COUNT {
            return Err(format!(
                "Halo 3 palette slot {slot} on line {} is outside 0-{}",
                line_number + 1,
                CUSTOM_COLOR_SWATCH_COUNT - 1
            ));
        }
        swatches[slot] = Some(ColorPaletteSwatch::named([red, green, blue, 255], name));
        found = true;
    }
    found
        .then_some(swatches)
        .ok_or_else(|| "The Halo 3 palette is empty".to_owned())
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
    let stroke = Stroke::new(1.0_f32, subtle_dark());
    ui.painter()
        .line_segment([rect.left_top(), rect.right_bottom()], stroke);
    ui.painter()
        .line_segment([rect.right_top(), rect.left_bottom()], stroke);
}

fn current_pc_hex(color: &MaterialColorPopup) -> String {
    format!(
        "sc#{}, {}, {}, {}",
        format_pc_float(color.alpha),
        format_pc_float(color.red),
        format_pc_float(color.green),
        format_pc_float(color.blue)
    )
}

pub(in crate::app) fn draw_color_hex_rows(ui: &mut Ui, color: &mut MaterialColorPopup) {
    let current_hex = format_rgb_hex(color.red, color.green, color.blue);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        draw_color_row_label(ui, "Hex:");
        let response = ui.add_sized(
            Vec2::new(COLOR_COMBINED_FIELD_WIDTH, COLOR_ROW_HEIGHT),
            egui::TextEdit::singleline(&mut color.hex_input)
                .hint_text(placeholder_text("#RRGGBB"))
                .vertical_align(egui::Align::Center)
                .desired_width(COLOR_COMBINED_FIELD_WIDTH),
        );
        if response.changed()
            && let Ok([r, g, b]) = parse_rgb_hex(&color.hex_input)
        {
            color.set_rgb_components(byte_to_float(r), byte_to_float(g), byte_to_float(b));
            color.hex_error = None;
        }
        let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if lost_focus_once(&response) || (response.has_focus() && enter_pressed) {
            match parse_rgb_hex(&color.hex_input) {
                Ok([r, g, b]) => color.set_rgb_bytes(r, g, b),
                Err(error) => color.hex_error = Some(error),
            }
        }
        if !response.has_focus() && color.hex_error.is_none() && color.hex_input != current_hex {
            color.hex_input = current_hex;
        }
    });

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        draw_color_row_label(ui, "PC Hex:");
        let pc_hex = current_pc_hex(color);
        let response = ui.add_sized(
            Vec2::new(COLOR_COMBINED_FIELD_WIDTH, COLOR_ROW_HEIGHT),
            egui::Button::new(
                RichText::new(truncate_for_cell(
                    &pc_hex,
                    COLOR_COMBINED_FIELD_WIDTH - 12.0,
                ))
                .monospace()
                .small(),
            ),
        );
        if response.clicked() {
            ui.copy_text(pc_hex.clone());
        }
        response.on_hover_text(format!("{pc_hex}\nClick to copy PC Hex"));
    });
    if let Some(error) = color.hex_error.as_deref() {
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
                if color.alpha_available {
                    draw_color_channel_row(ui, "A:", float_channel_to_u8(color.alpha), color.alpha);
                }
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
        .rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0_f32, MATERIAL_INPUT_EDGE),
            egui::StrokeKind::Middle,
        );
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

#[cfg(test)]
mod tests;
