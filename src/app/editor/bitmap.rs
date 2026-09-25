//! Bitmap panel, reimport paths, and preview image processing.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn draw_bitmap_tag(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    names: &TagNameIndex,
    _color_popup: &mut Option<MaterialColorPopup>,
    preview: &mut BitmapPreviewState,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) {
    if expert_mode {
        draw_tag_metadata(ui, tag, entry, names);
    }
    draw_preview_panel_toggle(
        ui,
        &mut preview.active_tab,
        BitmapPanelTab::Fields,
        BitmapPanelTab::Texture,
        "Bitmap Preview",
        ButtonIcon::Bitmap,
    );
    ui.add_space(6.0);

    match preview.active_tab {
        BitmapPanelTab::Fields => {
            ScrollArea::both()
                .id_salt(("bitmap_fields_scroll", edit.view_scope, edit.tag_key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(TAG_FIELD_SCROLL_MIN_WIDTH);
                    draw_struct_fields(ui, tag.root(), names, 0, expert_mode, "", edit);
                });
        }
        BitmapPanelTab::Texture => draw_bitmap_preview(ui, ctx, tag, entry, preview),
    }
}

pub(in crate::app) fn bitmap_reimport_data_path(
    entry: &TagEntry,
    tags_root: Option<&Path>,
) -> Option<String> {
    let TagEntryLocation::LooseFile(path) = &entry.location else {
        return None;
    };
    let tags_root = tags_root?;
    let rel = path.strip_prefix(tags_root).ok()?;
    let mut source = rel.to_path_buf();
    source.set_extension("");
    Some(source.to_string_lossy().replace('/', "\\"))
}

pub(in crate::app) fn draw_bitmap_preview(
    ui: &mut Ui,
    ctx: &egui::Context,
    tag: &TagFile,
    entry: &TagEntry,
    preview: &mut BitmapPreviewState,
) {
    // Decoded on a worker: a full-size block-compressed image can take a
    // noticeable time to expand to RGBA, and it used to hold the frame. The
    // level's compressed bytes are read out here, since the tag cannot leave
    // this thread; turning them into pixels happens off it.
    if preview.decoded.is_none() {
        if let Some(receiver) = &preview.decoding {
            match receiver.try_recv() {
                Ok(result) => {
                    preview.decoded = Some(result);
                    preview.decoding = None;
                    preview.texture_dirty = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    preview.decoded =
                        Some(Err("The bitmap decode stopped unexpectedly".to_owned()));
                    preview.decoding = None;
                }
            }
        } else {
            match extract_bitmap_level(tag, preview.image_index, preview.mip_index) {
                Ok(level) => {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let repaint = ctx.clone();
                    std::thread::spawn(move || {
                        let result = std::panic::catch_unwind(|| decode_bitmap_level(level))
                            .unwrap_or_else(|_| {
                                Err(anyhow::anyhow!("decoding the bitmap panicked"))
                            })
                            .map_err(|error| error.to_string());
                        let _ = sender.send(result);
                        repaint.request_repaint();
                    });
                    preview.decoding = Some(receiver);
                }
                Err(error) => preview.decoded = Some(Err(error.to_string())),
            }
        }
    }
    if preview.decoded.is_none() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("Decoding…").color(subtle_dark()));
        });
        return;
    }

    draw_bitmap_preview_data(ui, ctx, &entry.key, preview, true, "Bitmap");
}

/// Render an already-decoded RGBA preview with the same controls and canvas as
/// Baboon's bitmap editor. Other asset systems (such as Chimp's Texture2D
/// packages) use this entry point so channel toggles, zooming, backgrounds and
/// pixel inspection stay identical rather than drifting into parallel viewers.
/// `image_label` names the outer selection axis: bitmap tags step through images
/// in a sequence, while Chimp's virtual textures step through layers.
pub(in crate::app) fn draw_bitmap_preview_data(
    ui: &mut Ui,
    ctx: &egui::Context,
    texture_key: &str,
    preview: &mut BitmapPreviewState,
    supports_image_selection: bool,
    image_label: &str,
) {
    // Move the decoded payload out while the panel is drawn. This lets the
    // shared header/body callbacks mutate the rest of the preview state
    // without cloning a potentially very large RGBA buffer.
    let Some(decoded) = preview.decoded.take() else {
        return;
    };
    let data = match decoded {
        Ok(data) => data,
        Err(error) => {
            ui.colored_label(Color32::from_rgb(130, 32, 24), &error);
            preview.decoded = Some(Err(error));
            return;
        }
    };

    // Deferred re-decode: index fields are disjoint from `decoded` so we can
    // write them now, then decide whether to restore the payload after drawing.
    let mut redecode = false;
    if preview.texture_dirty || preview.texture.is_none() {
        let rgba = filtered_bitmap_rgba(&data, preview);

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [data.width as usize, data.height as usize],
            &rgba,
        );
        if let Some(texture) = preview.texture.as_mut() {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            preview.texture = Some(ctx.load_texture(
                format!("bitmap_preview_{texture_key}"),
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        preview.texture_dirty = false;
    }

    let texture = preview
        .texture
        .as_ref()
        .expect("a valid bitmap preview always uploads a texture")
        .clone();
    if preview.show_checkerboard && preview.checker_texture.is_none() {
        let mut rgba = Vec::with_capacity(8 * 8 * 4);
        for y in 0..8 {
            for x in 0..8 {
                let white = (x / 4 + y / 4) % 2 == 0;
                rgba.extend_from_slice(if white {
                    &[255, 255, 255, 13]
                } else {
                    &[0, 0, 0, 13]
                });
            }
        }
        preview.checker_texture = Some(ctx.load_texture(
            format!("bitmap_checkerboard_{texture_key}"),
            egui::ColorImage::from_rgba_unmultiplied([8, 8], &rgba),
            egui::TextureOptions::NEAREST_REPEAT,
        ));
    }
    let checker_texture = preview.checker_texture.clone();
    let image_size = texture.size_vec2();
    let header_wrap_width =
        bitmap_header_wrap_width(ui, &data, preview, supports_image_selection, image_label);

    draw_model_preview_section_with_header_wrap(
        ui,
        "Bitmap Preview",
        None,
        header_wrap_width,
        |ui, part| match part {
            ModelPreviewSectionPart::Header => {
                draw_bitmap_selection_controls(
                    ui,
                    &data,
                    preview,
                    supports_image_selection,
                    image_label,
                    &mut redecode,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_bitmap_view_menu(ui, preview);
                    draw_bitmap_camera_menu(ui, preview);
                    draw_bitmap_header_separator(ui);
                    let previous_spacing = ui.spacing().item_spacing.x;
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let before = (
                        preview.show_red,
                        preview.show_green,
                        preview.show_blue,
                        preview.show_alpha,
                    );
                    let alpha_action = draw_bitmap_channel_toggle(
                        ui,
                        preview.show_alpha,
                        ButtonIcon::ChannelAlpha,
                        "Alpha channel\nAlt-click: show only this channel\nShift-click: hide only this channel",
                    );
                    let blue_action = draw_bitmap_channel_toggle(
                        ui,
                        preview.show_blue,
                        ButtonIcon::ChannelBlue,
                        "Blue channel\nAlt-click: show only this channel\nShift-click: hide only this channel",
                    );
                    let green_action = draw_bitmap_channel_toggle(
                        ui,
                        preview.show_green,
                        ButtonIcon::ChannelGreen,
                        "Green channel\nAlt-click: show only this channel\nShift-click: hide only this channel",
                    );
                    let red_action = draw_bitmap_channel_toggle(
                        ui,
                        preview.show_red,
                        ButtonIcon::ChannelRed,
                        "Red channel\nAlt-click: show only this channel\nShift-click: hide only this channel",
                    );
                    ui.spacing_mut().item_spacing.x = previous_spacing;

                    apply_bitmap_channel_action(preview, ButtonIcon::ChannelAlpha, alpha_action);
                    apply_bitmap_channel_action(preview, ButtonIcon::ChannelBlue, blue_action);
                    apply_bitmap_channel_action(preview, ButtonIcon::ChannelGreen, green_action);
                    apply_bitmap_channel_action(preview, ButtonIcon::ChannelRed, red_action);
                    preview.texture_dirty |= before
                        != (
                            preview.show_red,
                            preview.show_green,
                            preview.show_blue,
                            preview.show_alpha,
                        );
                });
            }
            ModelPreviewSectionPart::Body => {
                draw_bitmap_canvas_and_footer(
                    ui,
                    &texture,
                    checker_texture.as_ref(),
                    image_size,
                    &data,
                    preview,
                );
            }
        },
    );

    if redecode && supports_image_selection {
        preview.decoded = None;
        preview.texture_dirty = true;
    } else {
        preview.decoded = Some(Ok(data));
    }
}

fn bitmap_header_wrap_width(
    ui: &Ui,
    data: &BitmapPreviewData,
    preview: &BitmapPreviewState,
    supports_image_selection: bool,
    image_label: &str,
) -> Option<f32> {
    if !supports_image_selection {
        return None;
    }

    let image_selector = data.image_count > 1;
    let mip_selector = data.mip_count > 1;
    if !image_selector && !mip_selector {
        return None;
    }

    let label_width = |label: &str| {
        ui.painter()
            .layout_no_wrap(
                label.to_owned(),
                TextStyle::Body.resolve(ui.style()),
                foundation_block_text(),
            )
            .size()
            .x
    };
    // Label + two 24 px steppers + 64 px index menu + the three 4 px gaps.
    let selector_width = |label: &str| label_width(label) + 124.0;
    let mut selection_width = 0.0;
    if image_selector {
        selection_width += selector_width(image_label);
    }
    if image_selector && mip_selector {
        selection_width += 33.0; // 16 px + divider + 16 px.
    }
    if mip_selector {
        selection_width += selector_width("Mipmap");
    }

    let menu_width = |label: &str| {
        let text = format!("     {label}      ");
        ui.painter()
            .layout_no_wrap(
                text,
                TextStyle::Button.resolve(ui.style()),
                foundation_block_text(),
            )
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
    };
    let zoom_label = format!("Zoom: {:.0}%", preview.zoom * 100.0);
    let action_width =
        menu_width("View") + menu_width(&zoom_label) + 4.0 * BUTTON_HEIGHT + 3.0 * 4.0 + 1.0 + 24.0;

    // Include the section's 8 px inset on both sides and retain an 8 px gap
    // between the selector and action groups at the point where they wrap.
    Some(selection_width + action_width + 24.0)
}

fn draw_bitmap_selection_controls(
    ui: &mut Ui,
    data: &BitmapPreviewData,
    preview: &mut BitmapPreviewState,
    supports_image_selection: bool,
    image_label: &str,
    redecode: &mut bool,
) {
    if !supports_image_selection {
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if data.image_count > 1 {
            let next = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    draw_bitmap_index_control(
                        ui,
                        image_label,
                        "bitmap_image_selector",
                        preview.image_index,
                        data.image_count,
                    )
                })
                .inner;
            if next != preview.image_index {
                preview.image_index = next;
                preview.mip_index = 0;
                *redecode = true;
            }
        }
        if data.image_count > 1 && data.mip_count > 1 {
            ui.add_space(16.0);
            draw_bitmap_header_separator(ui);
            ui.add_space(16.0);
        }
        if data.mip_count > 1 {
            let next = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    draw_bitmap_index_control(
                        ui,
                        "Mipmap",
                        "bitmap_mipmap_selector",
                        preview.mip_index,
                        data.mip_count,
                    )
                })
                .inner;
            if next != preview.mip_index {
                preview.mip_index = next;
                *redecode = true;
            }
        }
    });
}

fn draw_bitmap_header_separator(ui: &mut Ui) {
    let (separator_rect, _) = ui.allocate_exact_size(Vec2::new(1.0, BUTTON_HEIGHT), Sense::hover());
    ui.painter().vline(
        separator_rect.center().x,
        separator_rect.y_range(),
        Stroke::new(1.0, foundation_group_edge()),
    );
}

fn draw_bitmap_index_control(
    ui: &mut Ui,
    label: &str,
    id_salt: &'static str,
    selected: usize,
    count: usize,
) -> usize {
    let mut next = selected.min(count.saturating_sub(1));
    ui.label(RichText::new(label).color(foundation_block_text()));
    if foundation_header_stepper_clicked(ui, "<", next > 0) {
        next -= 1;
    }
    let mut selected_text = egui::text::LayoutJob::default();
    let font_id = TextStyle::Button.resolve(ui.style());
    selected_text.append(
        &next.to_string(),
        0.0,
        egui::TextFormat {
            font_id: font_id.clone(),
            color: foundation_block_text(),
            ..Default::default()
        },
    );
    selected_text.append(
        &format!("/ {}", count.saturating_sub(1)),
        8.0,
        egui::TextFormat {
            font_id,
            color: foundation_block_text().gamma_multiply(0.5),
            ..Default::default()
        },
    );
    let (combo, wheel_delta) = combo_box_with_scroll(
        ui,
        egui::ComboBox::from_id_salt((id_salt, count))
            .selected_text(selected_text)
            .width(64.0),
        |ui| {
            let just_opened = combo_popup_just_opened(ui);
            for index in 0..count {
                let row = ui.selectable_value(&mut next, index, format!("{index}"));
                if just_opened && index == selected {
                    row.scroll_to_me(Some(egui::Align::Center));
                }
            }
        },
    );
    if let Some(delta) = wheel_delta
        && let Some(index) = combo_scroll_next_index(next, count, delta)
    {
        next = index;
    }
    let _ = combo;
    if foundation_header_stepper_clicked(ui, ">", next + 1 < count) {
        next += 1;
    }
    next
}

fn draw_bitmap_view_menu(ui: &mut Ui, preview: &mut BitmapPreviewState) {
    const VIEW_SETTINGS_WIDTH: f32 = 240.0;
    right_aligned_icon_text_dropdown_button(
        ui,
        ButtonIcon::View,
        "View",
        VIEW_SETTINGS_WIDTH,
        |ui| {
            ui.set_width(VIEW_SETTINGS_WIDTH);
            ui.scope(|ui| {
                // The flat-list treatment makes the selected background read
                // like other menu selections. Keep it local: its transparent
                // inactive fill would otherwise erase the checkbox boxes in
                // the Options section below.
                style_list_menu(ui);
                ui.label(
                    RichText::new("Background Color")
                        .strong()
                        .color(text_dark()),
                );
                for bg in BitmapPreviewBg::ALL {
                    ui.selectable_value(&mut preview.bg, bg, bg.label());
                }
            });
            context_menu_separator(ui);
            ui.label(RichText::new("Options").strong().color(text_dark()));
            ui.checkbox(&mut preview.show_checkerboard, "Show Checkerboard")
                .on_hover_text(
                    "Draw a fixed 4 px checker behind the bitmap to make transparency visible.",
                );
            ui.checkbox(&mut preview.show_border, "Show Bitmap Border")
                .on_hover_text("Draw a two-pixel contrasting outline outside the bitmap edge.");
        },
    );
}

fn draw_bitmap_channel_toggle(
    ui: &mut Ui,
    enabled: bool,
    icon: ButtonIcon,
    tooltip: &str,
) -> BitmapChannelAction {
    let response = ui
        .scope(|ui| {
            if enabled {
                let (active_fill, active_border) = bitmap_channel_active_colors(icon);
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = active_fill;
                widgets.hovered.weak_bg_fill = active_fill;
                widgets.active.weak_bg_fill = active_fill;
                widgets.inactive.bg_stroke = Stroke::new(1.0, active_border);
                widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::WHITE);
                widgets.active.bg_stroke = Stroke::new(1.0, Color32::WHITE);
            }
            icon_button(ui, icon, tooltip, true, text_dark())
        })
        .inner;
    if response.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        if modifiers.alt {
            BitmapChannelAction::Solo
        } else if modifiers.shift {
            BitmapChannelAction::Exclude
        } else {
            BitmapChannelAction::Toggle
        }
    } else {
        BitmapChannelAction::None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BitmapChannelAction {
    None,
    Toggle,
    Solo,
    Exclude,
}

fn apply_bitmap_channel_action(
    preview: &mut BitmapPreviewState,
    icon: ButtonIcon,
    action: BitmapChannelAction,
) {
    if action == BitmapChannelAction::None {
        return;
    }
    match action {
        BitmapChannelAction::Solo => {
            preview.show_red = false;
            preview.show_green = false;
            preview.show_blue = false;
            preview.show_alpha = false;
        }
        BitmapChannelAction::Exclude => {
            preview.show_red = true;
            preview.show_green = true;
            preview.show_blue = true;
            preview.show_alpha = true;
        }
        BitmapChannelAction::None | BitmapChannelAction::Toggle => {}
    }
    let channel = match icon {
        ButtonIcon::ChannelRed => &mut preview.show_red,
        ButtonIcon::ChannelGreen => &mut preview.show_green,
        ButtonIcon::ChannelBlue => &mut preview.show_blue,
        ButtonIcon::ChannelAlpha => &mut preview.show_alpha,
        _ => return,
    };
    *channel = match action {
        BitmapChannelAction::Solo => true,
        BitmapChannelAction::Exclude => false,
        BitmapChannelAction::Toggle => !*channel,
        BitmapChannelAction::None => *channel,
    };
}

fn bitmap_channel_active_colors(icon: ButtonIcon) -> (Color32, Color32) {
    // Use explicit premultiplied values here. `from_rgba_unmultiplied` converts
    // saturated colors through linear space, which looks much brighter than a
    // restrained UI fill. These are explicit 40%-opacity channel colors.
    match icon {
        ButtonIcon::ChannelRed => (
            Color32::from_rgba_premultiplied(102, 0, 0, 102),
            Color32::from_rgb(255, 0, 0),
        ),
        ButtonIcon::ChannelGreen => (
            Color32::from_rgba_premultiplied(0, 102, 0, 102),
            Color32::from_rgb(0, 255, 0),
        ),
        ButtonIcon::ChannelBlue => (
            Color32::from_rgba_premultiplied(0, 0, 102, 102),
            Color32::from_rgb(0, 0, 255),
        ),
        ButtonIcon::ChannelAlpha if is_dark_mode() => (
            Color32::from_rgba_premultiplied(102, 102, 102, 102),
            Color32::WHITE,
        ),
        ButtonIcon::ChannelAlpha => (
            Color32::from_rgba_premultiplied(0, 0, 0, 102),
            Color32::BLACK,
        ),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    }
}

#[cfg(test)]
mod bitmap_channel_control_tests {
    use super::*;

    #[test]
    fn soloing_a_channel_enables_it_and_disables_the_rest() {
        let mut preview = BitmapPreviewState::default();
        apply_bitmap_channel_action(
            &mut preview,
            ButtonIcon::ChannelBlue,
            BitmapChannelAction::Solo,
        );

        assert!(!preview.show_red);
        assert!(!preview.show_green);
        assert!(preview.show_blue);
        assert!(!preview.show_alpha);
    }

    #[test]
    fn excluding_a_channel_disables_it_and_enables_the_rest() {
        let mut preview = BitmapPreviewState::default();
        preview.show_red = false;
        preview.show_green = false;
        preview.show_blue = false;
        preview.show_alpha = false;
        apply_bitmap_channel_action(
            &mut preview,
            ButtonIcon::ChannelGreen,
            BitmapChannelAction::Exclude,
        );

        assert!(preview.show_red);
        assert!(!preview.show_green);
        assert!(preview.show_blue);
        assert!(preview.show_alpha);
    }

    #[test]
    fn active_channel_fills_are_explicitly_forty_percent() {
        assert_eq!(
            bitmap_channel_active_colors(ButtonIcon::ChannelRed)
                .0
                .to_array(),
            [102, 0, 0, 102]
        );
        assert_eq!(
            bitmap_channel_active_colors(ButtonIcon::ChannelGreen)
                .0
                .to_array(),
            [0, 102, 0, 102]
        );
        assert_eq!(
            bitmap_channel_active_colors(ButtonIcon::ChannelBlue)
                .0
                .to_array(),
            [0, 0, 102, 102]
        );
    }
}

fn draw_bitmap_camera_menu(ui: &mut Ui, preview: &mut BitmapPreviewState) {
    let label = format!("Zoom: {:.0}%", preview.zoom * 100.0);
    const ZOOM_SETTINGS_WIDTH: f32 = 240.0;
    right_aligned_icon_text_dropdown_button(
        ui,
        ButtonIcon::Find,
        &label,
        ZOOM_SETTINGS_WIDTH,
        |ui| {
            ui.set_width(ZOOM_SETTINGS_WIDTH);
            if ui.selectable_label(false, "Fit to View").clicked() {
                preview.zoom_initialized = false;
                preview.pan = Vec2::ZERO;
            }
            ui.separator();
            for pct in [25u32, 50, 100, 200, 400] {
                if ui
                    .selectable_label(
                        preview.zoom_initialized && (preview.zoom * 100.0).round() as u32 == pct,
                        format!("{pct}%"),
                    )
                    .clicked()
                {
                    preview.zoom = pct as f32 / 100.0;
                    preview.zoom_initialized = true;
                    preview.pan = Vec2::ZERO;
                }
            }
            ui.separator();
            if ui.button("Reset View").clicked() {
                preview.zoom = 1.0;
                preview.zoom_initialized = true;
                preview.pan = Vec2::ZERO;
            }
        },
    );
}

fn draw_bitmap_canvas_and_footer(
    ui: &mut Ui,
    texture: &egui::TextureHandle,
    checker_texture: Option<&egui::TextureHandle>,
    image_size: Vec2,
    data: &BitmapPreviewData,
    preview: &mut BitmapPreviewState,
) {
    const FOOTER_HEIGHT: f32 = 32.0;

    // Allocate the whole remaining area as a fixed canvas and handle pan/zoom
    // manually. Using a ScrollArea here causes the scroll wheel to both zoom
    // (our code) and pan the viewport (egui), which fight and "teleport".
    let available = ui.available_size();
    let canvas_size = Vec2::new(available.x, (available.y - FOOTER_HEIGHT).max(1.0));
    let (canvas_rect, canvas_resp) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());

    // Fit zoom = the scale at which the whole texture fits the canvas (never
    // upscaling past 1:1). It is the initial zoom, but manual zooming may go
    // down to 25% even when a small bitmap already fits at 1:1.
    let fit_zoom = if canvas_rect.width() > 1.0
        && canvas_rect.height() > 1.0
        && image_size.x > 0.0
        && image_size.y > 0.0
    {
        let fit_w = canvas_rect.width() / image_size.x;
        let fit_h = canvas_rect.height() / image_size.y;
        fit_w.min(fit_h).min(1.0).max(0.001)
    } else {
        0.001
    };

    // On first load, set zoom to fit and center.
    if !preview.zoom_initialized && fit_zoom > 0.001 {
        preview.zoom = fit_zoom;
        preview.pan = Vec2::ZERO;
        preview.zoom_initialized = true;
    }

    // Scroll-to-zoom, anchored at the cursor (the image pixel under the
    // pointer stays fixed). All math is self-contained in this frame, so
    // there's no one-frame feedback lag.
    if canvas_resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            let old_zoom = preview.zoom;
            let factor = (scroll / 240.0).exp();
            let min_zoom = fit_zoom.min(0.25);
            let new_zoom = (old_zoom * factor).clamp(min_zoom, 32.0);
            if (new_zoom - old_zoom).abs() > f32::EPSILON {
                if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                    // Image top-left in screen space at the current zoom.
                    let center = canvas_rect.center();
                    let img_tl = center + preview.pan - image_size * old_zoom * 0.5;
                    // Pixel coordinate under the cursor.
                    let img_px = (ptr - img_tl) / old_zoom;
                    // Solve for the pan that keeps img_px under the cursor.
                    let new_img_tl = ptr - img_px * new_zoom;
                    preview.pan = new_img_tl - center + image_size * new_zoom * 0.5;
                }
                preview.zoom = new_zoom;
            }
        }
    }

    // Drag to pan.
    if canvas_resp.dragged() {
        preview.pan += canvas_resp.drag_delta();
    }

    // Clamp the pan so the image always covers the canvas — you can't drag
    // into empty background past the image edge. When the image is smaller
    // than the canvas on an axis (e.g. at fit zoom), it stays centered there.
    let draw_size = image_size * preview.zoom;
    let half_extra_x = ((draw_size.x - canvas_rect.width()) * 0.5).max(0.0);
    let half_extra_y = ((draw_size.y - canvas_rect.height()) * 0.5).max(0.0);
    preview.pan.x = preview.pan.x.clamp(-half_extra_x, half_extra_x);
    preview.pan.y = preview.pan.y.clamp(-half_extra_y, half_extra_y);

    // Draw the canvas, then the screen-space alpha checker, the image, and an
    // optional outline. The checker UVs are based on screen coordinates so
    // its 4 px tiles never zoom with the bitmap.
    let painter = ui.painter();
    painter.rect_filled(canvas_rect, 0.0, preview.bg.color());
    painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, grid_line()));

    let img_tl = canvas_rect.center() + preview.pan - draw_size * 0.5;
    let img_rect = egui::Rect::from_min_size(img_tl, draw_size);
    let clipped_img_rect = img_rect.intersect(canvas_rect);
    if preview.show_checkerboard
        && clipped_img_rect.is_positive()
        && let Some(checker_texture) = checker_texture
    {
        let uv = egui::Rect::from_min_max(
            egui::pos2(clipped_img_rect.left() / 8.0, clipped_img_rect.top() / 8.0),
            egui::pos2(
                clipped_img_rect.right() / 8.0,
                clipped_img_rect.bottom() / 8.0,
            ),
        );
        painter.image(checker_texture.id(), clipped_img_rect, uv, Color32::WHITE);
    }
    painter.with_clip_rect(canvas_rect).image(
        texture.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if preview.show_border {
        let color = if preview.bg == BitmapPreviewBg::White {
            Color32::from_black_alpha(153)
        } else {
            Color32::from_white_alpha(153)
        };
        painter.with_clip_rect(canvas_rect).rect_stroke(
            img_rect.expand(1.0),
            0.0,
            Stroke::new(2.0, color),
        );
    }

    // Under-cursor pixel coordinate + RGBA readout (samples the original
    // decoded pixels, independent of the channel-view toggles).
    if let Some(ptr) = canvas_resp.hover_pos() {
        let img_px = (ptr - img_tl) / preview.zoom;
        let (px, py) = (img_px.x.floor() as i64, img_px.y.floor() as i64);

        if px >= 0 && py >= 0 && (px as u32) < data.width && (py as u32) < data.height {
            let idx = (py as usize * data.width as usize + px as usize) * 4;
            if let Some(rgba) = data.rgba.get(idx..idx + 4) {
                let (r, g, b, a) = (rgba[0], rgba[1], rgba[2], rgba[3]);
                let text = format!("({px}, {py})  R{r} G{g} B{b} A{a}");
                let font = egui::FontId::monospace(12.0);
                let galley = painter.layout_no_wrap(text.clone(), font.clone(), text_dark());
                let pad = 5.0;
                let swatch = 12.0;
                let box_w = pad + swatch + 6.0 + galley.size().x + pad;
                let box_h = galley.size().y.max(swatch) + pad * 2.0;
                let box_min =
                    egui::pos2(canvas_rect.left() + 6.0, canvas_rect.bottom() - box_h - 6.0);
                let box_rect = egui::Rect::from_min_size(box_min, egui::vec2(box_w, box_h));
                painter.rect_filled(box_rect, 3.0, Color32::from_black_alpha(190));
                let swatch_rect = egui::Rect::from_min_size(
                    box_min + egui::vec2(pad, (box_h - swatch) * 0.5),
                    egui::vec2(swatch, swatch),
                );
                painter.rect_filled(swatch_rect, 2.0, Color32::from_rgb(r, g, b));
                painter.rect_stroke(swatch_rect, 2.0, Stroke::new(1.0, grid_line()));
                painter.text(
                    swatch_rect.right_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    text_dark(),
                );
            }
        }
    }

    draw_bitmap_stats_footer(ui, data);
}

fn draw_bitmap_stats_footer(ui: &mut Ui, data: &BitmapPreviewData) {
    const FOOTER_HEIGHT: f32 = 32.0;
    let (footer_rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(1.0), FOOTER_HEIGHT),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(footer_rect, 0.0, foundation_section_bar());
    ui.painter().text(
        footer_rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        format!(
            "{} × {}  ·  {}  ·  {}",
            data.width, data.height, data.format_name, data.type_name
        ),
        FontId::proportional(10.0),
        foundation_block_text(),
    );
}

/// Field-aware diff of two same-group tags: walk both root structs in parallel
/// (same layout → same field order) and collect every differing leaf value plus
/// block element-count mismatches. Returns the diffs and whether the cap was hit.

pub(in crate::app) fn build_bitmap_preview(
    tag: &TagFile,
    image_index: usize,
    mip_index: usize,
) -> anyhow::Result<BitmapPreviewData> {
    decode_bitmap_level(extract_bitmap_level(tag, image_index, mip_index)?)
}

/// One level of one image, as compressed bytes plus what decoding them needs.
/// Owns its bytes, so it can go to another thread; the tag cannot.
pub(in crate::app) struct BitmapLevel {
    format: blam_tags::bitmap::BitmapFormat,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
    palette: blam_tags::bitmap::P8Palette,
    image_count: usize,
    mip_count: usize,
    format_name: String,
    type_name: String,
}

/// Read one mip of one image out of a bitmap tag.
pub(in crate::app) fn extract_bitmap_level(
    tag: &TagFile,
    image_index: usize,
    mip_index: usize,
) -> anyhow::Result<BitmapLevel> {
    let bitmap = Bitmap::new(tag)?;
    if bitmap.is_empty() {
        anyhow::bail!("bitmap tag has no images");
    }
    let image_count = bitmap.len();
    let image_index = image_index.min(image_count - 1);
    let image = bitmap
        .image(image_index)
        .ok_or_else(|| anyhow::anyhow!("bitmap tag has no image {image_index}"))?;
    let format = image.format()?;
    let base_width = image.width();
    let base_height = image.height();

    if base_width == 0 || base_height == 0 {
        anyhow::bail!("bitmap image has empty dimensions");
    }
    let mip_count = (image.mipmap_levels() as usize).max(1);
    let mip = mip_index.min(mip_count - 1);

    // Walk the face-0 mip chain to this level: offset = Σ smaller-level bytes,
    // dims halve each step (floored at 1). Layout is `[face0_mips … faceN_mips]`,
    // so face 0's chain starts at offset 0.
    let mut offset = 0usize;
    let (mut width, mut height) = (base_width, base_height);
    for _ in 0..mip {
        offset += format.level_bytes(width, height) as usize;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    let mip_len = format.level_bytes(width, height) as usize;

    let pixel_bytes = image.pixel_bytes()?;
    if pixel_bytes.len() < offset + mip_len {
        anyhow::bail!(
            "bitmap image mip {mip} needs {} bytes at offset {offset} but only {} were available",
            mip_len,
            pixel_bytes.len()
        );
    }
    Ok(BitmapLevel {
        format,
        width,
        height,
        bytes: pixel_bytes[offset..offset + mip_len].to_vec(),
        palette: bitmap.p8_palette(),
        image_count,
        mip_count,
        format_name: image.format_name().unwrap_or_else(|| format!("{format:?}")),
        type_name: image.type_name().unwrap_or_else(|| "2D texture".to_owned()),
    })
}

/// Expand a level to RGBA. Needs no tag, so it can run on a worker.
pub(in crate::app) fn decode_bitmap_level(level: BitmapLevel) -> anyhow::Result<BitmapPreviewData> {
    let rgba = decode_to_rgba8(
        level.format,
        level.width,
        level.height,
        &level.bytes,
        level.palette,
    )?;
    Ok(BitmapPreviewData {
        width: level.width,
        height: level.height,
        image_count: level.image_count,
        mip_count: level.mip_count,
        format_name: level.format_name,
        type_name: level.type_name,
        rgba,
    })
}

#[cfg(test)]
#[path = "../tests/bitmap_bump_preview.rs"]
mod bump_preview_tests;

pub(in crate::app) fn filtered_bitmap_rgba(
    data: &BitmapPreviewData,
    preview: &BitmapPreviewState,
) -> Vec<u8> {
    let alpha_only =
        !preview.show_red && !preview.show_green && !preview.show_blue && preview.show_alpha;
    let mut out = data.rgba.clone();
    for pixel in out.chunks_exact_mut(4) {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if alpha_only {
            pixel[0] = a;
            pixel[1] = a;
            pixel[2] = a;
            pixel[3] = 255;
        } else {
            pixel[0] = if preview.show_red { r } else { 0 };
            pixel[1] = if preview.show_green { g } else { 0 };
            pixel[2] = if preview.show_blue { b } else { 0 };

            pixel[3] = if preview.show_alpha { a } else { 255 };
        }
    }
    out
}
