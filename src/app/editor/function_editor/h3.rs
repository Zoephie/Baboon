//! Foundation H3+ function controls and binary-backed fields.
//! It owns function decoding, visualization, and edit construction; applying edits to documents and unrelated shader layout belong elsewhere.

use super::*;

/// Convenience suggestions for the Input/Range name menus.
pub(in crate::app) const COMMON_FUNCTION_INPUTS: [&str; 7] = [
    "",
    "time",
    "frame",
    "random",
    "shield vitality",
    "change color primary",
    "distance to camera",
];

pub(in crate::app) const OUTPUT_TYPE_OPTIONS: [(i32, &str); 9] = [
    (0, "value"),
    (1, "color"),
    (2, "scale uniform"),
    (3, "scale x"),
    (4, "scale y"),
    (5, "translation x"),
    (6, "translation y"),
    (7, "frame index"),
    (8, "alpha"),
];

pub(in crate::app) const COLOR_GRAPH_OPTIONS: [(ColorGraphType, &str); 5] = [
    (ColorGraphType::Scalar, "scalar"),
    (ColorGraphType::OneColor, "1-color"),
    (ColorGraphType::TwoColor, "2-color"),
    (ColorGraphType::ThreeColor, "3-color"),
    (ColorGraphType::FourColor, "4-color"),
];

fn function_name_input(
    ui: &mut Ui,
    id: &str,
    value: &mut String,
    editable: bool,
    _encoding: FunctionEncoding,
) -> bool {
    const MENU_WIDTH: f32 = 48.0;
    let width = (ui.available_width() - MENU_WIDTH - ui.spacing().item_spacing.x).max(40.0);
    let response = ui
        .add_enabled_ui(editable, |ui| {
            ui.add_sized(
                Vec2::new(width, 24.0),
                egui::TextEdit::singleline(value)
                    .id_salt(id)
                    .desired_width(width)
                    .vertical_align(egui::Align::Center)
                    .hint_text("none"),
            )
        })
        .inner;
    text_edit_cursor_to_start_on_tab_focus(ui, &response);
    let mut changed = response.changed();
    ui.add_enabled_ui(editable, |ui| {
        ui.push_id((id, "suggestions"), |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(MENU_WIDTH, 24.0),
                egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true),
                |ui| {
                    icon_text_dropdown_button(ui, ButtonIcon::Function, "", |ui| {
                        ui.label(
                            RichText::new(if id.ends_with("_range") {
                                "Range Presets"
                            } else {
                                "Input Presets"
                            })
                            .color(subtle_dark())
                            .strong(),
                        );
                        for suggestion in COMMON_FUNCTION_INPUTS {
                            if ui
                                .selectable_label(
                                    value == suggestion,
                                    if suggestion.is_empty() {
                                        "none"
                                    } else {
                                        suggestion
                                    },
                                )
                                .clicked()
                            {
                                *value = suggestion.to_owned();
                                changed = true;
                                close_menu(ui);
                            }
                        }
                    });
                },
            );
        });
    });
    changed
}

fn master_label(master: EngineMasterType) -> &'static str {
    match master {
        EngineMasterType::Basic => "basic",
        EngineMasterType::Curve => "curve",
        EngineMasterType::Periodic => "periodic",
        EngineMasterType::Exponent => "exponent",
        EngineMasterType::Transition => "transition",
    }
}

fn foundation_master_type_combo(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
) -> bool {
    let current = editor.master_type();
    if !editable {
        foundation_input_cell(ui, master_label(current), ui.available_width());
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("foundation_fn_type")
        .selected_text(master_label(current))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for master in [
                EngineMasterType::Basic,
                EngineMasterType::Curve,
                EngineMasterType::Periodic,
                EngineMasterType::Exponent,
                EngineMasterType::Transition,
            ] {
                if ui
                    .selectable_label(master == current, master_label(master))
                    .clicked()
                    && master != current
                    && editor.set_master_type(master).is_ok()
                {
                    changed = true;
                }
            }
        });
    changed
}

/// Which color graph types a field offers. Guerilla restricts them per field:
/// shader scalar animations are scalar-only, shader color animations 2/3/4-
/// color only, everything else all five.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum ColorTypeChoices {
    #[default]
    All,
    ScalarOnly,
    MultiColorOnly,
}

impl ColorTypeChoices {
    fn allows(self, kind: ColorGraphType) -> bool {
        match self {
            Self::All => true,
            Self::ScalarOnly => kind == ColorGraphType::Scalar,
            Self::MultiColorOnly => is_multi_color(kind),
        }
    }
}

fn is_multi_color(kind: ColorGraphType) -> bool {
    matches!(
        kind,
        ColorGraphType::TwoColor | ColorGraphType::ThreeColor | ColorGraphType::FourColor
    )
}

fn color_type_label(editor: &TagFunctionEditor, kind: ColorGraphType) -> &'static str {
    match editor.function().encoding() {
        FunctionEncoding::H2 => blam_tags::tag_function::h2::color_graph_type_name(kind),
        FunctionEncoding::Blob => COLOR_GRAPH_OPTIONS
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or("scalar", |(_, name)| *name),
    }
}

/// The color graph type, offering what the field allows. Moving between
/// color counts resamples the existing colors.
fn color_type_combo(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    choices: ColorTypeChoices,
    editable: bool,
) -> bool {
    let current = editor.color_graph_type();
    if !editable {
        foundation_input_cell(ui, color_type_label(editor, current), ui.available_width());
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("function_color_type")
        .selected_text(color_type_label(editor, current))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (kind, _) in COLOR_GRAPH_OPTIONS {
                if !choices.allows(kind) && kind != current {
                    continue;
                }
                if ui
                    .selectable_label(kind == current, color_type_label(editor, kind))
                    .clicked()
                    && kind != current
                {
                    let resample = is_multi_color(current) && is_multi_color(kind);
                    changed |= if resample {
                        remap_editor_color_count(editor, kind)
                    } else {
                        editor.set_color_graph_type(kind).is_ok()
                    };
                }
            }
        });
    changed
}

/// Halo 2's type picker: the raw type list, as Guerilla shows it.
fn h2_function_type_combo(ui: &mut Ui, editor: &mut TagFunctionEditor, editable: bool) -> bool {
    use blam_tags::tag_function::h2::{FUNCTION_TYPES, function_type_name};
    let current = editor.function_type();
    let label = function_type_name(current);
    if !editable {
        foundation_input_cell(ui, label, ui.available_width());
        return false;
    }
    let mut changed = false;
    egui::ComboBox::from_id_salt("h2_function_type")
        .selected_text(label)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for kind in FUNCTION_TYPES {
                if ui
                    .selectable_label(kind == current, function_type_name(kind))
                    .clicked()
                    && kind != current
                    && editor.set_function_type(kind).is_ok()
                {
                    changed = true;
                }
            }
        });
    changed
}

/// The full function editor shown in the f() window and shared inline previews.
pub(in crate::app) fn draw_function_editor(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    color_popup: &mut Option<MaterialColorPopup>,
) -> bool {
    draw_function_editor_contents(
        ui,
        view,
        editable,
        selected_graph,
        selected_point,
        color_popup,
        true,
    )
}

pub(in crate::app) fn draw_inline_mapping_function_editor(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    color_popup: &mut Option<MaterialColorPopup>,
) -> bool {
    draw_function_editor_contents(
        ui,
        view,
        editable,
        selected_graph,
        selected_point,
        color_popup,
        false,
    )
}

fn draw_function_type_row(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    color_types: ColorTypeChoices,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    graph_view: &mut FunctionGraphView,
) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.vertical(|ui| {
            ui.set_width(FUNCTION_GRAPH_SIZE.x);
            ui.label(RichText::new("Function Type").strong().color(text_dark()));
            ui.horizontal(|ui| {
                let has_options = editor.function().as_blob().is_some()
                    && editor.master_type() == EngineMasterType::Curve;
                let width = FUNCTION_GRAPH_SIZE.x - 32.0 - if has_options { 32.0 } else { 0.0 };
                let retyped = ui
                    .scope(|ui| {
                        ui.set_width(width);
                        match editor.function().encoding() {
                            FunctionEncoding::Blob => {
                                foundation_master_type_combo(ui, editor, editable)
                            }
                            FunctionEncoding::H2 => h2_function_type_combo(ui, editor, editable),
                        }
                    })
                    .inner;
                if retyped {
                    *selected_graph = 0;
                    *selected_point = 0;
                    changed = true;
                }
                icon_menu_button(ui, ButtonIcon::Find, "Zoom and grid", |ui| {
                    ui.add(
                        egui::Slider::new(&mut graph_view.zoom_percent, 50.0..=100.0)
                            .text("Zoom")
                            .suffix("%")
                            .step_by(1.0),
                    );
                    ui.separator();
                    ui.label(RichText::new("Grid").color(text_dark().gamma_multiply(0.5)));
                    for (grid, label) in [
                        (FunctionGrid::Off, "Off"),
                        (FunctionGrid::Ten, "10×10"),
                        (FunctionGrid::Twenty, "20×20"),
                    ] {
                        ui.scope(|ui| {
                            ui.spacing_mut().button_padding = Vec2::new(BUTTON_TEXT_PADDING_X, 2.0);
                            if ui
                                .add(egui::Button::selectable(graph_view.grid == grid, label))
                                .clicked()
                            {
                                graph_view.grid = grid;
                            }
                        });
                    }
                });
                if has_options {
                    icon_menu_button(ui, ButtonIcon::Other, "Curve options", |ui| {
                        changed |=
                            draw_curve_panel(ui, editor, editable, selected_graph, selected_point);
                    });
                }
            });
        });
        ui.vertical(|ui| {
            ui.set_width(150.0);
            ui.label(RichText::new("Output Type").strong().color(text_dark()));
            changed |= color_type_combo(ui, editor, color_types, editable);
        });
    });
    changed
}

fn draw_function_editor_contents(
    ui: &mut Ui,
    view: &mut FunctionView,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    color_popup: &mut Option<MaterialColorPopup>,
    show_wrapper: bool,
) -> bool {
    let mut changed = false;
    let mut editor = TagFunctionEditor::from_function(view.function.clone());
    let graph_view_id = ui.id().with("function_graph_view");
    let mut graph_view = ui
        .data(|data| data.get_temp::<FunctionGraphView>(graph_view_id))
        .unwrap_or_default();
    ui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);
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

    if !show_wrapper {
        changed |= draw_function_type_row(
            ui,
            &mut editor,
            editable,
            view.color_types,
            selected_graph,
            selected_point,
            &mut graph_view,
        );
    }
    if show_wrapper {
        egui::Frame::new()
            .inner_margin(4.0)
            .corner_radius(4.0)
            .stroke(Stroke::new(1.0, foundation_input_edge()))
            .show(ui, |ui| {
                ui.columns(2, |columns| {
                    let mut input_height = 0.0;
                    for slot in 0..2 {
                        let fill = if slot == 0 {
                            Color32::from_rgba_unmultiplied(42, 190, 72, 26)
                        } else {
                            Color32::from_rgba_unmultiplied(220, 55, 55, 26)
                        };
                        let panel = egui::Frame::new()
                            .fill(fill)
                            .corner_radius(4.0)
                            .inner_margin(8.0)
                            .show(&mut columns[slot], |ui| {
                                if slot == 1 {
                                    ui.set_min_height(input_height);
                                }
                                if slot == 0 {
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(24.0);
                                        ui.label(
                                            RichText::new("Input").strong().color(text_dark()),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        changed |= function_name_input(
                                            ui,
                                            "foundation_fn_input",
                                            &mut view.input_name,
                                            input_editable,
                                            editor.function().encoding(),
                                        );
                                    });
                                } else {
                                    let mut ranged = editor.is_ranged();
                                    ui.horizontal(|ui| {
                                        ui.set_min_height(24.0);
                                        if ui
                                            .add_enabled(
                                                editable,
                                                egui::Checkbox::without_text(&mut ranged),
                                            )
                                            .changed()
                                            && set_function_ranged(&mut editor, ranged)
                                        {
                                            *selected_graph = (*selected_graph)
                                                .min(editor.graph_count().saturating_sub(1));
                                            changed = true;
                                        }
                                        ui.label(
                                            RichText::new("Range").strong().color(text_dark()),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        if ranged {
                                            changed |= function_name_input(
                                                ui,
                                                "foundation_fn_range",
                                                &mut view.range_name,
                                                range_editable,
                                                editor.function().encoding(),
                                            );
                                        } else {
                                            changed |= function_name_input(
                                                ui,
                                                "foundation_fn_range",
                                                &mut view.range_name,
                                                false,
                                                editor.function().encoding(),
                                            );
                                        }
                                    });
                                }
                                if slot < editor.graph_count() {
                                    changed |= draw_function_slot(ui, &mut editor, editable, slot);
                                }
                            });
                        if slot == 0 {
                            input_height = (panel.response.rect.height() - 16.0).max(0.0);
                        }
                    }
                });
            });
    } else if matches!(
        editor.master_type(),
        EngineMasterType::Periodic | EngineMasterType::Exponent | EngineMasterType::Transition
    ) {
        ui.add_space(4.0);
        ui.columns(editor.graph_count(), |columns| {
            for (slot, column) in columns.iter_mut().enumerate() {
                changed |= draw_function_slot(column, &mut editor, editable, slot);
            }
        });
    }
    if show_wrapper {
        ui.add_space(4.0);
        changed |= draw_function_type_row(
            ui,
            &mut editor,
            editable,
            view.color_types,
            selected_graph,
            selected_point,
            &mut graph_view,
        );
    }

    ui.data_mut(|data| data.insert_temp(graph_view_id, graph_view));
    let master = editor.master_type();
    if master == EngineMasterType::Basic {
        // A constant's value is its output range (min, and max when ranged).
        ui.add_space(8.0);
        changed |= draw_foundation_right_rail(ui, &mut editor, editable, color_popup, None);
        ui.add_space(6.0);
        if show_wrapper {
            changed |=
                draw_function_time_period(ui, &mut view.time_period_in_seconds, time_editable);
        }
        if changed {
            view.function = editor.into_function();
        }
        return changed;
    }

    ui.add_space(4.0);
    let mut preview_rect = None;
    let mut selection_rect = None;
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let mut left_bottom = 0.0;
        let viewport = egui::Rect::from_min_size(ui.cursor().min, FUNCTION_GRAPH_SIZE)
            .shrink2(FUNCTION_PLOT_INSET);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(FUNCTION_GRAPH_SIZE.x);
            changed |= draw_foundation_graph_with_preview(
                ui,
                &mut editor,
                editable,
                selected_graph,
                selected_point,
                &mut graph_view,
                show_wrapper,
            );
            if !show_wrapper {
                ui.add_space(8.0);
                preview_rect = Some(
                    ui.allocate_exact_size(Vec2::new(FUNCTION_GRAPH_SIZE.x, 52.0), Sense::hover())
                        .0,
                );
            }
            if show_wrapper {
                ui.add_space(8.0);
                changed |=
                    draw_function_time_period(ui, &mut view.time_period_in_seconds, time_editable);
            }
            left_bottom = ui.min_rect().bottom();
        });
        let plot = egui::Rect::from_center_size(
            viewport.center(),
            viewport.size() * (graph_view.zoom_percent / 100.0),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(150.0);
            changed |= draw_foundation_right_rail(
                ui,
                &mut editor,
                editable,
                color_popup,
                Some((plot.top(), plot.bottom())),
            );
            ui.add_space(8.0);
            let selection_top = ui.cursor().top();
            let selection_frame = egui::Frame::new()
                .inner_margin(6.0)
                .corner_radius(4.0)
                .stroke(Stroke::new(1.0, foundation_input_edge()))
                .show(ui, |ui| {
                    ui.set_width(136.0);
                    ui.set_min_height((left_bottom - selection_top - 14.0).max(0.0));
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.spacing_mut().interact_size.y = 20.0;
                    if master == EngineMasterType::Curve {
                        changed |= draw_curve_selection_with_view(
                            ui,
                            &mut editor,
                            editable,
                            selected_graph,
                            selected_point,
                            graph_view,
                        );
                    } else {
                        ui.label(RichText::new("Selected Point").color(text_dark()));
                        ui.add_space(6.0);
                        ui.label(RichText::new("No selectable points").color(subtle_dark()));
                    }
                });
            selection_rect = Some(selection_frame.response.rect);
        });
    });

    if let Some(mut rect) = preview_rect {
        if let Some(selection) = selection_rect {
            rect.min.y = selection.top();
        }
        rect.max.y = row.response.rect.bottom();
        let width =
            (FUNCTION_GRAPH_SIZE.x - FUNCTION_PLOT_INSET.x * 2.0) * graph_view.zoom_percent / 100.0;
        draw_function_preview_container(ui.painter(), rect, width, editor.function());
    }

    ui.add_space(8.0);
    draw_function_shortcuts(ui, &editor, editable);

    ui.data_mut(|data| data.insert_temp(graph_view_id, graph_view));
    if changed {
        view.function = editor.into_function();
    }
    changed
}

fn draw_function_shortcuts(
    ui: &mut Ui,
    editor: &TagFunctionEditor,
    editable: bool,
) {
    let curve = editor.master_type() == EngineMasterType::Curve;
    let can_add = editable
        && curve
        && (editor.function().as_blob().is_some() || editor.curve_points_are_editable_structure());
    let can_remove = can_add;
    let paired_handles = editable && curve && editor.function().as_blob().is_some();
    let can_snap = editable && curve;
    egui::Frame::new()
        .inner_margin(8.0)
        .corner_radius(4.0)
        .fill(foundation_group_bg())
        .stroke(Stroke::new(1.0, foundation_input_edge()))
        .show(ui, |ui| {
            ui.set_width(FUNCTION_GRAPH_SIZE.x + 8.0 + 150.0 - 18.0);
            ui.spacing_mut().interact_size.y = 16.0;
            ui.spacing_mut().item_spacing = Vec2::new(16.0, 2.0);
            if can_snap || can_add || can_remove {
                ui.horizontal_wrapped(|ui| {
                    if can_snap {
                        function_shortcut_hint(ui, "Ctrl + Drag", "Snap to selected grid");
                    }
                    if can_add {
                        function_shortcut_hint(ui, "Shift + Click", "Add point");
                    }
                    if can_remove {
                        function_shortcut_hint(ui, "Alt + Click", "Remove point");
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                if paired_handles {
                    function_shortcut_hint(ui, "Shift + Drag", "Mirror angle & length");
                    function_shortcut_hint(ui, "Alt + Drag", "Decouple angle");
                }
                function_shortcut_hint(ui, "Scroll Wheel", "Zoom");
            });
        });
}

fn function_shortcut_hint(ui: &mut Ui, keys: &str, action: &str) {
    let mut text = egui::text::LayoutJob::default();
    let regular = egui::TextStyle::Small.resolve(ui.style());
    let bold = bold_font(regular.size);
    // Standalone UI contexts may not install the app's named font families.
    let key_font = if ui.fonts(|fonts| fonts.families().contains(&bold.family)) {
        bold
    } else {
        regular
    };
    RichText::new(format!("{keys}: "))
        .font(key_font)
        .color(text_dark())
        .append_to(
            &mut text,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        );
    RichText::new(action).small().color(text_dark()).append_to(
        &mut text,
        ui.style(),
        egui::FontSelection::Default,
        egui::Align::Center,
    );
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
}
fn draw_function_time_period(ui: &mut Ui, value: &mut f32, editable: bool) -> bool {
    // Include the frame's horizontal padding and 1pt border in the graph width.
    let content_width = FUNCTION_GRAPH_SIZE.x - 22.0;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 4))
        .corner_radius(4.0)
        .stroke(Stroke::new(1.0, foundation_input_edge()))
        .show(ui, |ui| {
            ui.set_width(content_width);
            ui.allocate_ui_with_layout(
                Vec2::new(content_width, 24.0),
                egui::Layout::left_to_right(egui::Align::Center)
                    .with_main_align(egui::Align::Min),
                |ui| {
                ui.label(RichText::new("Time Period").color(text_dark()));
                let changed = ui
                    .add_enabled_ui(editable, |ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new((ui.available_width() - 65.0).max(40.0), 24.0),
                            egui::Layout::left_to_right(egui::Align::Center)
                                .with_main_align(egui::Align::Min)
                                .with_main_justify(true),
                            |ui| {
                                ui.add(egui::DragValue::new(value).speed(0.1).range(0.0..=f32::MAX))
                            },
                        )
                        .inner
                        .changed()
                    })
                    .inner;
                ui.label(RichText::new("seconds").color(subtle_dark()));
                changed
            })
            .inner
        })
        .inner
}

fn draw_foundation_right_rail(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    color_popup: &mut Option<MaterialColorPopup>,
    plot_bounds: Option<(f32, f32)>,
) -> bool {
    let mut changed = false;
    let scalar = editor.clamp_range().is_some();
    let mut draw_rail = |ui: &mut Ui| {
        if let Some((mut min, mut max)) = editor.clamp_range() {
            align_function_rail_row(ui, plot_bounds, 0, 2, ui.spacing().interact_size.y);
            if labeled_drag(ui, "Max", &mut max, editable) {
                changed |= editor.set_clamp_range(min, max).is_ok();
            }
            align_function_rail_row(ui, plot_bounds, 1, 2, ui.spacing().interact_size.y);
            if labeled_drag(ui, "Min", &mut min, editable) {
                changed |= editor.set_clamp_range(min, max).is_ok();
            }
        } else {
            let count = editor.color_count();
            for (row, index) in (0..count).rev().enumerate() {
                let Some(argb) = editor.get_color(index) else {
                    continue;
                };
                let alpha = (argb >> 24) as u8;
                let color = color32_from_argb(argb);
                align_function_rail_row(ui, plot_bounds, row, count, 24.0);
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().max(96.0), 24.0),
                    if editable {
                        Sense::click()
                    } else {
                        Sense::hover()
                    },
                );
                paint_shader_color_preview(ui, rect, color);
                if response.clicked() {
                    *color_popup = Some(
                        MaterialColorPopup::new(
                            &format!("Function color {}", index + 1),
                            color.r() as f32 / 255.0,
                            color.g() as f32 / 255.0,
                            color.b() as f32 / 255.0,
                            1.0,
                        )
                        .with_function_draft_color(FunctionDraftColorTarget::Logical(index), alpha),
                    );
                }
            }
        }
    };
    ui.vertical(|ui| {
        if plot_bounds.is_some() {
            egui::Frame::new()
                .inner_margin(4.0)
                .corner_radius(4.0)
                .stroke(Stroke::new(1.0, foundation_input_edge()))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width().max(if scalar { 110.0 } else { 24.0 }));
                    // Frame padding and stroke add 10pt to the content height.
                    ui.set_min_height(FUNCTION_GRAPH_SIZE.y - 10.0);
                    draw_rail(ui);
                });
        } else {
            draw_rail(ui);
        }
    });
    changed
}

fn align_function_rail_row(
    ui: &mut Ui,
    plot_bounds: Option<(f32, f32)>,
    row: usize,
    count: usize,
    height: f32,
) {
    if let Some((top, bottom)) = plot_bounds {
        // Keep the first/last fields inside the plot edges, with intermediate
        // fields centered at equally spaced positions along the gradient.
        let position = if count > 1 { row as f32 / (count - 1) as f32 } else { 0.0 };
        let y = egui::lerp(top..=(bottom - height), position);
        ui.add_space((y - ui.cursor().top()).max(0.0));
    }
}

/// Returns whether the color graph type changed (Halo 2 functions refuse it).
fn remap_editor_color_count(editor: &mut TagFunctionEditor, target: ColorGraphType) -> bool {
    let old = (0..editor.color_count())
        .filter_map(|index| editor.get_color(index))
        .collect::<Vec<_>>();
    if editor.set_color_graph_type(target).is_err() {
        return false;
    }
    let count = editor.color_count();
    for index in 0..count {
        let t = if count <= 1 {
            0.0
        } else {
            index as f32 / (count - 1) as f32
        };
        let argb = sample_argb_stops(&old, t);
        let _ = editor.set_color(index, argb);
    }
    true
}

fn sample_argb_stops(stops: &[u32], t: f32) -> u32 {
    if stops.is_empty() {
        return 0;
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let position = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let index = position.floor() as usize;
    let next = (index + 1).min(stops.len() - 1);
    let local = position - index as f32;
    let lerp = |shift: u32| {
        let a = ((stops[index] >> shift) & 0xff) as f32;
        let b = ((stops[next] >> shift) & 0xff) as f32;
        (a + (b - a) * local).round() as u32
    };
    (lerp(24) << 24) | (lerp(16) << 16) | (lerp(8) << 8) | lerp(0)
}

pub(super) fn set_editor_flag(editor: &mut TagFunctionEditor, flag: u8, value: bool) {
    let mut function = editor.function().clone();
    if let Some(blob) = function.as_blob_mut() {
        blob.set_flag(flag, value);
        *editor = TagFunctionEditor::from_function(function);
    }
}

fn labeled_drag(ui: &mut Ui, label: &str, value: &mut f32, editable: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{label}:"))
                .color(text_dark())
                .small(),
        );
        changed = ui
            .add_enabled_ui(editable, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 24.0),
                    egui::Layout::left_to_right(egui::Align::Center)
                        .with_main_align(egui::Align::Min)
                        .with_main_justify(true),
                    |ui| ui.add(egui::DragValue::new(value).speed(0.01).max_decimals(5)),
                )
                .inner
                .changed()
            })
            .inner;
    });
    changed
}

#[cfg(test)]
fn draw_curve_selection(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
) -> bool {
    draw_curve_selection_with_view(
        ui,
        editor,
        editable,
        selected_graph,
        selected_point,
        FunctionGraphView::default(),
    )
}

fn draw_curve_selection_with_view(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    selected_graph: &mut usize,
    selected_point: &mut usize,
    graph_view: FunctionGraphView,
) -> bool {
    let mut changed = false;
    let selection_bottom = ui.min_rect().bottom();
    let graph = (*selected_graph).min(editor.graph_count().saturating_sub(1));
    let point_count = editor.curve_control_point_count(graph).unwrap_or(0);
    *selected_point = (*selected_point).min(point_count.saturating_sub(1));
    if let Some((mut x, mut y)) = editor.curve_control_point(graph, *selected_point) {
        ui.horizontal(|ui| {
            let (marker, marker_response) =
                ui.allocate_exact_size(Vec2::splat(12.0), Sense::click());
            draw_function_selection_marker(
                ui.painter(),
                marker.center(),
                editor,
                graph,
                *selected_point,
            );
            let point_kind = if function_point_is_on_curve(editor, graph, *selected_point) {
                "Point"
            } else {
                "Curve handle"
            };
            if marker_response
                .on_hover_text(format!(
                    "{point_kind} on the {} curve. Click to switch curves.",
                    if graph == 0 { "green" } else { "red" }
                ))
                .clicked()
            {
                *selected_graph = (graph + 1) % editor.graph_count().max(1);
            }
            ui.label(RichText::new("Selected Point").color(text_dark()));
        });
        let x_movable = editor.curve_point_x_movable(graph, *selected_point);
        if grouped_function_drag(ui, "X", &mut x, editable && x_movable) {
            changed |= move_function_point_in_view(
                editor,
                graph,
                *selected_point,
                (x, y),
                ui.input(|input| input.modifiers),
                graph_view,
            );
            if let Some(value) = editor.curve_control_point(graph, *selected_point) {
                (x, y) = value;
            }
        }
        if grouped_function_drag(ui, "Y", &mut y, editable) {
            changed |= move_function_point_in_view(
                editor,
                graph,
                *selected_point,
                (x, y),
                ui.input(|input| input.modifiers),
                graph_view,
            );
        }
        if let Some(segment) = function_join_segment(editor, graph, *selected_point) {
            let mode = function_join_mode(editor, graph, segment);
            let curved_join = [segment - 1, segment].into_iter().all(|index| {
                editor
                    .curve_segment_type(graph, index)
                    .is_some_and(|kind| kind != CurveSegmentType::Linear)
            });
            ui.add_space(
                (selection_bottom - ui.cursor().top() - ui.spacing().interact_size.y).max(4.0),
            );
            ui.horizontal(|ui| {
                for (target, label) in [
                    (CurvePointMode::Corner, "Corner"),
                    (CurvePointMode::Smooth, "Smooth"),
                ] {
                    if ui
                        .add_enabled(
                            editable && curved_join,
                            egui::Button::selectable(mode == target, label),
                        )
                        .clicked()
                        && mode != target
                        && set_function_join_mode(editor, graph, segment, target, graph_view)
                    {
                        changed = true;
                    }
                }
            });
        }
    }
    changed
}

fn draw_curve_panel(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    _selected_graph: &mut usize,
    _selected_point: &mut usize,
) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        // H3+ header flags follow a separate, vertically stacked section.
        if editor.function().encoding() != FunctionEncoding::H2 {
            ui.label(RichText::new("Curve Options").color(text_dark().gamma_multiply(0.5)));
            let mut clamped = editor.is_clamped();
            if ui
                .add_enabled(editable, egui::Checkbox::new(&mut clamped, "Clamped"))
                .changed()
            {
                set_editor_flag(editor, FunctionFlags::CLAMPED, clamped);
                changed = true;
            }
            let mut cyclic = editor.is_cyclic();
            if ui
                .add_enabled(editable, egui::Checkbox::new(&mut cyclic, "Cyclic"))
                .changed()
            {
                set_function_cyclic(editor, cyclic);
                changed = true;
            }
            let mut exclusion = editor.is_exclusion();
            if ui
                .add_enabled(editable, egui::Checkbox::new(&mut exclusion, "Exclusion"))
                .changed()
            {
                set_editor_flag(editor, FunctionFlags::EXCLUSION, exclusion);
                if exclusion
                    && editor.function().as_blob().is_some_and(|blob| {
                        blob.exclusion_min() == 0.0 && blob.exclusion_max() == 0.0
                    })
                {
                    set_function_exclusion_bounds(editor, 0.25, 0.75);
                }
                changed = true;
            }
            if exclusion {
                ui.label(
                    RichText::new(format!(
                        "range {:.3} – {:.3}",
                        editor
                            .function()
                            .as_blob()
                            .map_or(0.0, BlobFunction::exclusion_min),
                        editor
                            .function()
                            .as_blob()
                            .map_or(0.0, BlobFunction::exclusion_max)
                    ))
                    .color(subtle_dark())
                    .small(),
                );
            }
        }
    });
    changed
}

fn draw_function_slot(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    slot: usize,
) -> bool {
    let mut changed = false;
    ui.add_space(4.0);
    match editor.master_type() {
        EngineMasterType::Periodic => {
            let names = editor.periodic_functions();
            let Some(mut params) = editor.periodic_params(slot) else {
                return false;
            };
            ui.label(RichText::new("Exponent").color(text_dark()).strong());
            let mut slot_changed = periodic_function_combo(ui, slot, &mut params, names, editable);
            ui.columns(2, |columns| {
                slot_changed |= stacked_function_drag(
                    &mut columns[0],
                    "Frequency",
                    &mut params.frequency,
                    editable,
                );
                slot_changed |= stacked_function_drag(&mut columns[1], "Phase", &mut params.phase, editable);
            });
            slot_changed |= function_bounds_row(ui, &mut params.amplitude_min, &mut params.amplitude_max, editable);
            if slot_changed && editor.set_periodic_params(slot, params).is_ok() {
                changed = true;
            }
        }
        EngineMasterType::Exponent => changed |= draw_exponent_slot(ui, editor, editable, slot),
        EngineMasterType::Transition => changed |= draw_transition_slot(ui, editor, editable, slot),
        EngineMasterType::Basic | EngineMasterType::Curve => {}
    }
    changed
}

fn grouped_function_drag(ui: &mut Ui, label: &str, value: &mut f32, editable: bool) -> bool {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(24.0, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center)
                .with_main_align(egui::Align::Min)
                .with_main_justify(true),
            |ui| ui.label(RichText::new(label).color(text_dark())),
        );
        ui.add_enabled_ui(editable, |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center)
                    .with_main_align(egui::Align::Min)
                    .with_main_justify(true),
                |ui| ui.add(egui::DragValue::new(value).speed(0.01).max_decimals(5)),
            )
            .inner
        })
        .inner
        .changed()
    })
    .inner
}

fn stacked_function_drag(ui: &mut Ui, label: &str, value: &mut f32, editable: bool) -> bool {
    ui.label(RichText::new(label).color(text_dark()));
    ui.add_enabled_ui(editable, |ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 24.0),
            egui::Layout::left_to_right(egui::Align::Center)
                .with_main_align(egui::Align::Min)
                .with_main_justify(true),
            |ui| ui.add(egui::DragValue::new(value).speed(0.01).max_decimals(5)),
        )
        .inner
        .changed()
    })
    .inner
}

fn function_bounds_row(ui: &mut Ui, min: &mut f32, max: &mut f32, editable: bool) -> bool {
    let mut changed = false;
    ui.columns(2, |columns| {
        changed |= stacked_function_drag(&mut columns[0], "Min", min, editable);
        changed |= stacked_function_drag(&mut columns[1], "Max", max, editable);
    });
    changed
}

fn periodic_function_combo(
    ui: &mut Ui,
    slot: usize,
    params: &mut PeriodicParams,
    names: &[&str],
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = names.get(current).copied().unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("periodic_function", slot))
        .selected_text(label)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, label) in names.iter().enumerate() {
                if ui
                    .add_enabled(
                        editable,
                        egui::Button::selectable(index == current, *label),
                    )
                    .clicked()
                {
                    params.function_index = index as u8;
                    changed = true;
                }
            }
        });
    changed
}

fn draw_exponent_slot(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    slot: usize,
) -> bool {
    let mut changed = false;
    let Some(mut params) = editor.exponent_params(slot) else {
        return false;
    };
    let mut slot_changed = stacked_function_drag(ui, "Exponent", &mut params.exponent, editable);
    slot_changed |= function_bounds_row(ui, &mut params.amplitude_min, &mut params.amplitude_max, editable);
    if slot_changed && editor.set_exponent_params(slot, params).is_ok() {
        changed = true;
    }
    changed
}

fn draw_transition_slot(
    ui: &mut Ui,
    editor: &mut TagFunctionEditor,
    editable: bool,
    slot: usize,
) -> bool {
    let mut changed = false;
    let names = editor.transition_functions();
    let Some(mut params) = editor.transition_params(slot) else {
        return false;
    };
    ui.label(RichText::new("Exponent").color(text_dark()).strong());
    let mut slot_changed = transition_function_combo(ui, slot, &mut params, names, editable);
    slot_changed |= function_bounds_row(ui, &mut params.amplitude_min, &mut params.amplitude_max, editable);
    if slot_changed && editor.set_transition_params(slot, params).is_ok() {
        changed = true;
    }
    changed
}

fn transition_function_combo(
    ui: &mut Ui,
    slot: usize,
    params: &mut TransitionParams,
    names: &[&str],
    editable: bool,
) -> bool {
    let current = params.function_index as usize;
    let label = names.get(current).copied().unwrap_or("unknown");
    let mut changed = false;
    egui::ComboBox::from_id_salt(("transition_function", slot))
        .selected_text(label)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, label) in names.iter().enumerate() {
                if ui
                    .add_enabled(
                        editable,
                        egui::Button::selectable(index == current, *label),
                    )
                    .clicked()
                {
                    params.function_index = index as u8;
                    changed = true;
                }
            }
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_menu_has_a_heading_and_flags_without_segment_controls() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.insert_curve_point(0, 0.5).unwrap();
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            ui.set_width(220.0);
            draw_curve_panel(ui, &mut editor, true, &mut 0, &mut 0);
        });
        let text_pos = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing selection label: {label}"))
        };
        let labels = ["Curve Options", "Clamped", "Cyclic", "Exclusion"].map(text_pos);
        assert!(labels.windows(2).all(|pair| pair[0].y < pair[1].y));
        assert!((labels[1].x - labels[2].x).abs() < 1.0 && (labels[2].x - labels[3].x).abs() < 1.0);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if matches!(text.galley.text(), "Corner" | "Smooth") || text.galley.text().starts_with("Segment "))));
    }

    #[test]
    fn selection_join_controls_appear_below_coordinates_and_edit_the_selected_join() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();
        let mut selected = insert_function_point(&mut editor, 0, 0.5).unwrap();
        editor
            .set_curve_join_mode(0, 1, CurvePointMode::Corner)
            .unwrap();
        let mut frame = |events| {
            crate::app::run_ui_test(
                &ctx,
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(136.0);
                    draw_curve_selection(ui, &mut editor, true, &mut 0, &mut selected);
                },
            )
        };
        frame(vec![]);
        let output = frame(vec![]);
        let text_rect = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap()
        };
        let smooth = text_rect("Smooth");
        assert!(text_rect("Corner").top() > text_rect("Y").bottom());
        assert!(smooth.top() > text_rect("Y").bottom());
        let pos = smooth.center();
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        assert_eq!(editor.curve_join_mode(0, 1), Some(CurvePointMode::Smooth));
        assert_eq!(
            TagFunctionEditor::parse(&editor.to_bytes())
                .unwrap()
                .curve_join_mode(0, 1),
            Some(CurvePointMode::Smooth)
        );
    }

    fn constant_editor() -> TagFunctionEditor {
        let mut bytes = vec![0u8; 32];
        bytes[0] = FunctionType::Constant as u8;
        bytes[8..12].copy_from_slice(&1.0f32.to_le_bytes());
        TagFunctionEditor::parse(&bytes).expect("constant function")
    }

    #[test]
    fn input_and_range_suggestions_fill_the_textbox() {
        for id in ["foundation_fn_input", "foundation_fn_range"] {
            let ctx = egui::Context::default();
            let origin = std::cell::Cell::new(egui::Pos2::ZERO);
            let mut value = "body vitality".to_owned();
            let mut changed = false;
            let mut frame = |events| crate::app::run_ui_test(&ctx, egui::RawInput { events, ..Default::default() }, |ui| {
                ui.set_width(280.0);
                ui.horizontal(|ui| {
                    origin.set(ui.cursor().min);
                    changed |= function_name_input(ui, id, &mut value, true, FunctionEncoding::H2);
                });
            });
            frame(Vec::new());
            frame(Vec::new());
            let button = origin.get() + Vec2::new(256.0, 12.0);
            frame(vec![egui::Event::PointerMoved(button)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos: button, button: egui::PointerButton::Primary, pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            let output = frame(Vec::new());
            let suggestion = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "time" => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            }).expect("suggestions menu opens");
            frame(vec![egui::Event::PointerMoved(suggestion)]);
            for pressed in [true, false] {
                frame(vec![egui::Event::PointerButton {
                    pos: suggestion, button: egui::PointerButton::Primary, pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            let output = frame(Vec::new());
            assert!(!output.shapes.iter().any(|shape| matches!(
                &shape.shape, egui::Shape::Text(text) if text.galley.text() == "distance to camera"
            )), "the menu closes after selection");
            assert_eq!(value, "time");
            assert!(changed);
        }
    }

    #[test]
    fn shortcut_hints_follow_available_graph_actions() {
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        let hints = |editor: &TagFunctionEditor, editable| {
            let ctx = egui::Context::default();
            ctx.set_fonts(foundation_fonts());
            let output =
                crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                    draw_function_shortcuts(ui, editor, editable)
                });
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let sections = &text.galley.job.sections;
                    assert_eq!(sections[0].format.font_id.family, bold_font(12.0).family);
                    assert_ne!(sections[0].format.font_id.family, sections[1].format.font_id.family);
                }
            }
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let linear = hints(&editor, true);
        assert!(linear.contains("Ctrl + Drag") && linear.contains("Shift + Click"));
        assert!(
            linear.contains("Alt + Click")
                && linear.contains("Shift + Drag")
                && linear.contains("Alt + Drag")
        );
        assert!(
            !linear.contains("Undo") && !linear.contains("Redo") && !linear.contains("Right Click")
        );
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();
        insert_function_point(&mut editor, 0, 0.5).unwrap();
        let curved = hints(&editor, true);
        assert_eq!(linear, curved, "curve hints stay stable as points and handles change");
        assert!(
            curved.contains("Alt + Click")
                && curved.contains("Shift + Drag")
                && curved.contains("Alt + Drag")
        );
        editor.set_master_type(EngineMasterType::Periodic).unwrap();
        let periodic = hints(&editor, true);
        assert!(periodic.contains("Scroll Wheel"));
        assert!(!periodic.contains("Click") && !periodic.contains("Drag"));
        let read_only = hints(&editor, false);
        assert!(read_only.contains("Zoom") && !read_only.contains("Drag"));
    }

    #[test]
    fn inline_preview_fills_the_height_of_the_selection_panel() {
        for color_type in [ColorGraphType::Scalar, ColorGraphType::TwoColor] {
            let mut editor = constant_editor();
            editor.set_master_type(EngineMasterType::Curve).unwrap();
            editor
                .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
                .unwrap();
            let point = insert_function_point(&mut editor, 0, 0.5).unwrap();
            editor.set_color_graph_type(color_type).unwrap();
            let mut view = FunctionView::from_function(editor.into_function());
            let ctx = egui::Context::default();
            let mut frame = || {
                crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
                    draw_inline_mapping_function_editor(
                        ui,
                        &mut view,
                        true,
                        &mut 0,
                        &mut point.clone(),
                        &mut None,
                    );
                })
            };
            frame();
            let output = frame();
            let frames: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.stroke.color == foundation_input_edge() => {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .collect();
            let graph = frames
                .iter()
                .find(|rect| (rect.width() - 465.0).abs() < 0.1 && (rect.height() - 225.0).abs() < 0.1)
                .unwrap();
            let preview = frames
                .iter()
                .find(|rect| (rect.width() - 465.0).abs() < 0.1 && rect.top() > graph.bottom())
                .unwrap();
            let selection = frames
                .iter()
                .find(|rect| (rect.width() - 150.0).abs() < 0.1 && rect.top() > graph.bottom())
                .unwrap();
            assert!(preview.height() > 52.0);
            assert!((preview.top() - selection.top()).abs() < 0.1, "preview {preview:?}, selection {selection:?}");
            assert!((preview.bottom() - selection.bottom()).abs() < 0.1, "preview {preview:?}, selection {selection:?}");
        }
    }

    #[test]
    fn periodic_layout_keeps_parameter_columns_and_selection_placeholder() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Periodic).unwrap();
        editor.set_ranged(true).unwrap();
        editor.set_color_graph_type(ColorGraphType::ThreeColor).unwrap();
        let mut view = FunctionView::from_function(editor.into_function());
        let output = crate::app::run_ui_test(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 900.0))),
            ..Default::default()
        }, |ui| {
            ui.set_width(623.0);
            draw_function_editor(ui, &mut view, true, &mut 0, &mut 0, &mut None);
        });
        let label_pos = |label: &str| output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
            _ => None,
        }).unwrap_or_else(|| panic!("missing label: {label}"));
        let frequency = label_pos("Frequency");
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Output:")));
        assert!(label_pos("Input").y < label_pos("Function Type").y);
        let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if (rect.rect.width() - FUNCTION_GRAPH_SIZE.x).abs() < 1.0 && (rect.rect.height() - FUNCTION_GRAPH_SIZE.y).abs() < 1.0 => Some(rect.rect.shrink2(FUNCTION_PLOT_INSET)), _ => None,
        }).unwrap();
        assert!(label_pos("Function Type").y < plot.top());
        assert!((label_pos("Output Type").x - (plot.left() - 10.0 + 473.0)).abs() < 1.0);
        let phase = label_pos("Phase");
        let max = label_pos("Max");
        let min = label_pos("Min");
        assert!(phase.x > frequency.x);
        assert!((phase.y - frequency.y).abs() < 1.0);
        assert!(min.y > frequency.y);
        assert!(max.x > min.x);
        assert!((min.y - max.y).abs() < 1.0);
        assert!(label_pos("No selectable points").y > min.y);
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Rect(rect)
                if rect.rect.width() > 100.0 && rect.rect.height() == 24.0 && rect.corner_radius.nw == 3
        )), "the color rail uses wide shader color preview buttons");
    }

    #[test]
    fn argb_remap_preserves_endpoints_and_interpolates_interior() {
        let stops = [0x0010_2030, 0x8040_6080];
        assert_eq!(sample_argb_stops(&stops, 0.0), stops[0]);
        assert_eq!(sample_argb_stops(&stops, 1.0), stops[1]);
        assert_eq!(sample_argb_stops(&stops, 0.5), 0x4028_4058);
    }

    #[test]
    fn new_editor_curve_and_ranged_compacts_roundtrip() {
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor.set_ranged(true).unwrap();
        editor.insert_curve_point(0, 0.5).unwrap();
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();

        let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        assert_eq!(reparsed.master_type(), EngineMasterType::Curve);
        assert_eq!(reparsed.graph_count(), 2);
        assert_eq!(reparsed.curve_segment_count(0), Some(2));
        assert_eq!(
            reparsed.curve_segment_type(0, 0),
            Some(CurveSegmentType::Spline)
        );
    }

    #[test]
    fn new_editor_periodic_slots_roundtrip_independently() {
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Periodic).unwrap();
        editor.set_ranged(true).unwrap();
        let first = PeriodicParams {
            function_index: 2,
            frequency: 3.0,
            phase: 0.25,
            amplitude_min: -1.0,
            amplitude_max: 2.0,
        };
        let second = PeriodicParams {
            function_index: 11,
            frequency: 0.5,
            phase: 0.75,
            amplitude_min: 4.0,
            amplitude_max: 8.0,
        };
        editor.set_periodic_params(0, first).unwrap();
        editor.set_periodic_params(1, second).unwrap();

        let reparsed = TagFunctionEditor::parse(&editor.to_bytes()).unwrap();
        assert_eq!(reparsed.periodic_params(0), Some(first));
        assert_eq!(reparsed.periodic_params(1), Some(second));
    }

    /// One frame of the function editor's graph at the origin.
    fn graph_frame(
        ctx: &egui::Context,
        editor: &mut TagFunctionEditor,
        selected: &mut (usize, usize),
        events: Vec<egui::Event>,
    ) {
        graph_frame_with_modifiers(ctx, editor, selected, events, egui::Modifiers::NONE);
    }

    fn graph_frame_with_modifiers(
        ctx: &egui::Context,
        editor: &mut TagFunctionEditor,
        selected: &mut (usize, usize),
        mut events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let _ = crate::app::run_ui_test(
            &ctx,
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 600.0),
                )),
                ..Default::default()
            },
            |_| {
                egui::Area::new(egui::Id::new("function_graph_test"))
                    .fixed_pos(egui::Pos2::ZERO)
                    .show(ctx, |ui| {
                        draw_foundation_graph(ui, editor, true, &mut selected.0, &mut selected.1);
                    });
            },
        );
    }

    fn graph_screen((x, y): (f32, f32)) -> egui::Pos2 {
        let plot = egui::Rect::from_min_size(egui::Pos2::ZERO, FUNCTION_GRAPH_SIZE)
            .shrink2(FUNCTION_PLOT_INSET);
        egui::pos2(
            egui::lerp(plot.left()..=plot.right(), x),
            egui::lerp(plot.bottom()..=plot.top(), y),
        )
    }

    #[test]
    fn repeated_clicks_cycle_overlapping_points_and_handles_without_retargeting_drags() {
        for halo2 in [false, true] {
            for point in [0, 1] {
                let ctx = egui::Context::default();
                let mut editor = if halo2 {
                    let mut editor = TagFunctionEditor::from_function(
                        h2_tag_function(&h2_constant_scalar_function_data(1.0, None)).unwrap(),
                    );
                    editor.set_function_type(FunctionType::Spline).unwrap();
                    editor
                } else {
                    let mut editor = constant_editor();
                    editor.set_master_type(EngineMasterType::Curve).unwrap();
                    editor
                        .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
                        .unwrap();
                    editor
                };
                set_function_ranged(&mut editor, true);
                if !halo2 {
                    editor
                        .set_curve_segment_type(1, 0, CurveSegmentType::Spline)
                        .unwrap();
                }
                let value = if point == 0 {
                    editor.curve_control_point(0, point).unwrap()
                } else {
                    (0.25, 0.5)
                };
                for graph in 0..2 {
                    editor.set_curve_control_point(graph, point, value).unwrap();
                }
                let original = editor.to_bytes();
                let mut selected = (0, point);
                graph_frame(&ctx, &mut editor, &mut selected, vec![]);
                graph_frame(&ctx, &mut editor, &mut selected, vec![]);
                let pos = graph_screen(value);
                for graph in [1, 0, 1] {
                    graph_frame(
                        &ctx,
                        &mut editor,
                        &mut selected,
                        vec![egui::Event::PointerMoved(pos)],
                    );
                    for pressed in [true, false] {
                        graph_frame(
                            &ctx,
                            &mut editor,
                            &mut selected,
                            vec![egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            }],
                        );
                    }
                    assert_eq!(selected, (graph, point));
                    assert_eq!(
                        editor.to_bytes(),
                        original,
                        "cycling only changes selection"
                    );
                }
                graph_frame(
                    &ctx,
                    &mut editor,
                    &mut selected,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let end = pos + Vec2::new(0.0, if value.1 > 0.5 { 30.0 } else { -30.0 });
                graph_frame(
                    &ctx,
                    &mut editor,
                    &mut selected,
                    vec![egui::Event::PointerMoved(end)],
                );
                graph_frame(
                    &ctx,
                    &mut editor,
                    &mut selected,
                    vec![egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert_eq!(
                    selected,
                    (1, point),
                    "dragging keeps the selected overlap target"
                );
                assert_eq!(editor.curve_control_point(0, point), Some(value));
                assert_ne!(editor.curve_control_point(1, point), Some(value));
            }
        }
    }

    #[test]
    fn alt_and_shift_drag_handles_without_adding_or_deleting_points() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();
        let anchor = insert_function_point(&mut editor, 0, 0.5).unwrap();
        let mut selected = (0, 0);
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        for (modifiers, target, mode) in [
            (egui::Modifiers::ALT, (0.75, 0.65), CurvePointMode::Corner),
            (egui::Modifiers::SHIFT, (0.7, 0.6), CurvePointMode::Smooth),
        ] {
            let start = graph_screen(editor.curve_control_point(0, anchor + 1).unwrap());
            let end = graph_screen(target);
            graph_frame_with_modifiers(
                &ctx,
                &mut editor,
                &mut selected,
                vec![egui::Event::PointerMoved(start)],
                modifiers,
            );
            graph_frame_with_modifiers(
                &ctx,
                &mut editor,
                &mut selected,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                }],
                modifiers,
            );
            for _ in 0..2 {
                graph_frame_with_modifiers(
                    &ctx,
                    &mut editor,
                    &mut selected,
                    vec![egui::Event::PointerMoved(end)],
                    modifiers,
                );
            }
            graph_frame_with_modifiers(
                &ctx,
                &mut editor,
                &mut selected,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                }],
                modifiers,
            );
            assert_eq!(selected, (0, anchor + 1));
            assert_eq!(editor.curve_segment_count(0), Some(2));
            assert_eq!(editor.curve_join_mode(0, 1), Some(mode));
        }
        let a = editor.curve_control_point(0, anchor - 1).unwrap();
        let b = editor.curve_control_point(0, anchor + 1).unwrap();
        assert!((a.0 + b.0 - 1.0).abs() < 0.001 && (a.1 + b.1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn adding_and_removing_points_requires_shift_and_alt() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        let mut selected = (0, 0);
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        let pos = graph_screen((0.5, 0.5));
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::SHIFT,
            egui::Modifiers::ALT,
        ] {
            graph_frame_with_modifiers(
                &ctx,
                &mut editor,
                &mut selected,
                vec![egui::Event::PointerMoved(pos)],
                modifiers,
            );
            for pressed in [true, false] {
                graph_frame_with_modifiers(
                    &ctx,
                    &mut editor,
                    &mut selected,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    }],
                    modifiers,
                );
            }
            let expected = if modifiers.shift { 2 } else { 1 };
            assert_eq!(editor.curve_segment_count(0), Some(expected));
        }
    }

    #[test]
    fn control_drag_snaps_to_grid_intersections() {
        let ctx = egui::Context::default();
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        editor
            .set_curve_segment_type(0, 0, CurveSegmentType::Spline)
            .unwrap();
        let mut selected = (0, 0);
        let start = graph_screen(editor.curve_control_point(0, 1).unwrap());
        let end = graph_screen((0.26, 0.44));
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        graph_frame(
            &ctx,
            &mut editor,
            &mut selected,
            vec![egui::Event::PointerMoved(start)],
        );
        graph_frame_with_modifiers(
            &ctx,
            &mut editor,
            &mut selected,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::CTRL,
            }],
            egui::Modifiers::CTRL,
        );
        for _ in 0..2 {
            graph_frame_with_modifiers(
                &ctx,
                &mut editor,
                &mut selected,
                vec![egui::Event::PointerMoved(end)],
                egui::Modifiers::CTRL,
            );
        }
        let value = editor.curve_control_point(0, 1).unwrap();
        assert!(
            (value.0 - 0.3).abs() < 0.001 && (value.1 - 0.4).abs() < 0.001,
            "{value:?}"
        );
    }

    /// Press at `from`, move to `to` in steps, release: a user's drag.
    fn graph_drag(editor: &mut TagFunctionEditor, from: (f32, f32), to: (f32, f32)) -> (usize, usize) {
        let ctx = egui::Context::default();
        let mut selected = (0, 0);
        let (start, end) = (graph_screen(from), graph_screen(to));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        graph_frame(&ctx, editor, &mut selected, vec![]);
        graph_frame(
            &ctx,
            editor,
            &mut selected,
            vec![egui::Event::PointerMoved(start)],
        );
        graph_frame(&ctx, editor, &mut selected, vec![button(start, true)]);
        for step in 1..=10 {
            let pos = start + (end - start) * (step as f32 / 10.0);
            graph_frame(
                &ctx,
                editor,
                &mut selected,
                vec![egui::Event::PointerMoved(pos)],
            );
        }
        graph_frame(&ctx, editor, &mut selected, vec![button(end, false)]);
        graph_frame(&ctx, editor, &mut selected, vec![]);
        selected
    }

    fn h2_editor(kind: FunctionType) -> TagFunctionEditor {
        TagFunctionEditor::from_function(TagFunction::H2(H2Function::new(kind)))
    }

    /// The points after one press in the graph's top margin whose pointer
    /// ends at `to_y` before release.
    fn press_in_the_top_margin(to_y: f32) -> Vec<(f32, f32)> {
        let mut editor = constant_editor();
        editor.set_master_type(EngineMasterType::Curve).unwrap();
        let ctx = egui::Context::default();
        let mut selected = (0, 0);
        let (start, end) = (egui::pos2(230.0, 0.5), egui::pos2(230.0, to_y));
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![egui::Event::PointerMoved(start)]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![button(start, true)]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![egui::Event::PointerMoved(end)]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![button(end, false)]);
        graph_frame(&ctx, &mut editor, &mut selected, vec![]);
        let count = editor.curve_control_point_count(0).unwrap();
        (0..count).filter_map(|point| editor.curve_control_point(0, point)).collect()
    }

    /// A click that slips a few points off the graph adds its point where a
    /// steady click does. egui 0.36 starts a drag as soon as the pointer leaves
    /// the widget, even within click distance, and that drag pulled the new
    /// point to the top edge.
    #[test]
    fn a_click_that_slips_off_the_graph_is_still_a_click() {
        let steady = press_in_the_top_margin(3.0);
        let slipped = press_in_the_top_margin(-5.2);
        assert_eq!(slipped, steady);
    }

    #[test]
    fn dragging_an_h2_linear_key_point_holds_it_between_its_neighbours() {
        let mut editor = h2_editor(FunctionType::LinearKey);
        let before = editor.curve_control_point(0, 1).unwrap();
        let selected = graph_drag(&mut editor, before, (0.95, 0.5));

        assert_eq!(
            selected,
            (0, 1),
            "the drag picked the point under the pointer"
        );
        let (x, y) = editor.curve_control_point(0, 1).unwrap();
        assert_eq!(
            x,
            editor.curve_control_point(0, 2).unwrap().0,
            "stopped at the next point"
        );
        assert!((y - 0.5).abs() < 0.01, "y follows the pointer ({y})");
    }

    #[test]
    fn dragging_an_h2_end_point_moves_it_only_vertically() {
        let mut editor = h2_editor(FunctionType::Spline);
        graph_drag(&mut editor, (0.0, 1.0), (0.5, 0.25));
        let (x, y) = editor.curve_control_point(0, 0).unwrap();
        assert_eq!(x, 0.0);
        assert!((y - 0.25).abs() < 0.01, "y follows the pointer ({y})");
    }

    #[test]
    fn a_drag_from_empty_space_moves_no_point() {
        // Before the fix a press away from every point still dragged whatever
        // point was selected (point 0 here).
        let mut editor = h2_editor(FunctionType::Spline);
        let before = editor.to_bytes();
        graph_drag(&mut editor, (0.5, 0.5), (0.9, 0.1));
        assert_eq!(editor.to_bytes(), before);
    }

    #[test]
    fn clicking_empty_graph_space_adds_no_h2_point() {
        let mut editor = h2_editor(FunctionType::LinearKey);
        let before = editor.to_bytes();
        graph_drag(&mut editor, (0.5, 0.1), (0.5, 0.1));
        assert_eq!(editor.curve_control_point_count(0), Some(4));
        assert_eq!(
            editor.to_bytes(),
            before,
            "a click away from every point changes nothing"
        );
    }
}
