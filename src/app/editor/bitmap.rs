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
    draw_tag_metadata(ui, tag, names);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let can_reimport = bitmap_reimport_data_path(entry, edit.tags_root).is_some();
        if ui
            .add_enabled(can_reimport, egui::Button::new("Reimport"))
            .on_hover_text("Run tool bitmaps for this bitmap source path, then reload the tag")
            .clicked()
        {
            *edit.bitmap_reimport = Some(entry.key.clone());
        }
        ui.separator();
        ui.selectable_value(&mut preview.active_tab, BitmapPanelTab::Fields, "Fields");
        ui.selectable_value(
            &mut preview.active_tab,
            BitmapPanelTab::Texture,
            "Texture preview",
        );
    });
    ui.separator();

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
    if preview.decoded.is_none() {
        preview.decoded = Some(
            build_bitmap_preview(tag, preview.image_index, preview.mip_index)
                .map_err(|error| error.to_string()),
        );
        preview.texture_dirty = true;
    }

    let Some(decoded) = preview.decoded.as_ref() else {
        return;
    };
    let data = match decoded {
        Ok(data) => data,

        Err(error) => {
            ui.colored_label(Color32::from_rgb(130, 32, 24), error);
            return;
        }
    };

    ui.horizontal(|ui| {
        let red_changed = ui.checkbox(&mut preview.show_red, "Red").changed();
        let green_changed = ui.checkbox(&mut preview.show_green, "Green").changed();
        let blue_changed = ui.checkbox(&mut preview.show_blue, "Blue").changed();
        let alpha_changed = ui.checkbox(&mut preview.show_alpha, "Alpha").changed();
        if red_changed || green_changed || blue_changed || alpha_changed {
            preview.texture_dirty = true;
        }
    });
    // Deferred re-decode: index fields are disjoint from `decoded` so we can
    // write them now, but `decoded = None` must wait until `data`'s borrow ends
    // (applied at the end of the function).
    let mut redecode = false;
    ui.horizontal(|ui| {
        // Image (sequence) selector.
        if data.image_count > 1 {
            ui.label(RichText::new("Image").color(subtle_dark()));
            if ui
                .add_enabled(preview.image_index > 0, egui::Button::new("◀"))
                .clicked()
            {
                preview.image_index -= 1;

                preview.mip_index = 0;
                redecode = true;
            }
            ui.monospace(
                RichText::new(format!("{}/{}", preview.image_index, data.image_count - 1))
                    .color(text_dark()),
            );
            if ui
                .add_enabled(
                    preview.image_index + 1 < data.image_count,
                    egui::Button::new("▶"),
                )
                .clicked()
            {
                preview.image_index += 1;
                preview.mip_index = 0;
                redecode = true;
            }
            ui.separator();
        } else {
            ui.label(RichText::new("Image 0").color(subtle_dark()));
        }
        // Mip-level selector.
        if data.mip_count > 1 {
            ui.label(RichText::new("Mip").color(subtle_dark()));
            if ui
                .add_enabled(preview.mip_index > 0, egui::Button::new("◀"))
                .clicked()
            {
                preview.mip_index -= 1;
                redecode = true;
            }
            ui.monospace(
                RichText::new(format!("{}/{}", preview.mip_index, data.mip_count - 1))
                    .color(text_dark()),
            );
            if ui
                .add_enabled(
                    preview.mip_index + 1 < data.mip_count,
                    egui::Button::new("▶"),
                )
                .clicked()
            {
                preview.mip_index += 1;
                redecode = true;
            }
            ui.separator();
        }
        ui.monospace(RichText::new(format!("{} x {}", data.width, data.height)).color(text_dark()));
        ui.label(RichText::new(&data.format_name).color(subtle_dark()));
        ui.label(RichText::new(&data.type_name).color(subtle_dark()));

        ui.separator();
        ui.label(RichText::new(format!("Zoom {:.0}%", preview.zoom * 100.0)).color(subtle_dark()));
        let (_, zoom_wheel_delta) = combo_box_with_scroll(
            ui,
            egui::ComboBox::from_id_salt(("bitmap_zoom_preset", &entry.key))
                .selected_text("Set…")
                .width(58.0),
            |ui| {
                if ui.selectable_label(false, "Fit").clicked() {
                    preview.zoom_initialized = false; // refit next frame
                    preview.pan = Vec2::ZERO;
                }
                for pct in [25u32, 50, 100, 200, 400] {
                    if ui.selectable_label(false, format!("{pct}%")).clicked() {
                        preview.zoom = pct as f32 / 100.0;
                        preview.zoom_initialized = true;
                        preview.pan = Vec2::ZERO;
                    }
                }
            },
        );
        if let Some(delta) = zoom_wheel_delta {
            let presets = [0u32, 25, 50, 100, 200, 400];
            let current_pct = (preview.zoom * 100.0).round() as u32;
            let current = presets
                .iter()
                .position(|pct| *pct == current_pct)
                .unwrap_or_else(|| {
                    presets
                        .iter()
                        .enumerate()
                        .skip(1)
                        .min_by_key(|(_, pct)| pct.abs_diff(current_pct))
                        .map(|(index, _)| index)
                        .unwrap_or(0)
                });
            if let Some(next) = combo_scroll_next_index(current, presets.len(), delta) {
                if presets[next] == 0 {
                    preview.zoom_initialized = false;
                } else {
                    preview.zoom = presets[next] as f32 / 100.0;
                    preview.zoom_initialized = true;
                }
                preview.pan = Vec2::ZERO;
            }
        }
        if ui.button("Reset zoom").clicked() {
            preview.zoom_initialized = false; // triggers fit-to-view on next frame
            preview.pan = Vec2::ZERO;
        }
        ui.separator();
        ui.label(RichText::new("BG").color(subtle_dark()));
        let (_, bg_wheel_delta) = combo_box_with_scroll(
            ui,
            egui::ComboBox::from_id_salt(("bitmap_bg", &entry.key))
                .selected_text(preview.bg.label())
                .width(86.0),
            |ui| {
                for bg in BitmapPreviewBg::ALL {
                    if ui.selectable_label(preview.bg == bg, bg.label()).clicked() {
                        preview.bg = bg;
                    }
                }
            },
        );
        if let Some(delta) = bg_wheel_delta {
            let current = BitmapPreviewBg::ALL
                .iter()
                .position(|bg| *bg == preview.bg)
                .unwrap_or(0);
            if let Some(next) = combo_scroll_next_index(current, BitmapPreviewBg::ALL.len(), delta)
            {
                preview.bg = BitmapPreviewBg::ALL[next];
            }
        }
    });
    ui.add_space(6.0);

    if preview.texture_dirty || preview.texture.is_none() {
        let rgba = filtered_bitmap_rgba(data, preview);

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [data.width as usize, data.height as usize],
            &rgba,
        );
        if let Some(texture) = preview.texture.as_mut() {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            preview.texture = Some(ctx.load_texture(
                format!("bitmap_preview_{}", entry.key),
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        preview.texture_dirty = false;
    }

    let Some(texture) = preview.texture.as_ref() else {
        return;
    };
    let image_size = texture.size_vec2();

    // Allocate the whole remaining area as a fixed canvas and handle pan/zoom
    // manually. Using a ScrollArea here causes the scroll wheel to both zoom
    // (our code) and pan the viewport (egui), which fight and "teleport".
    let canvas_size = ui.available_size();
    let (canvas_rect, canvas_resp) = ui.allocate_exact_size(canvas_size, Sense::click_and_drag());

    // Fit zoom = the scale at which the whole texture fits the canvas (never
    // upscaling past 1:1). This is both the initial zoom and the minimum the
    // user can zoom out to — you can't shrink the texture smaller than fit.
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
            // Floor at fit_zoom so the texture can't be zoomed out smaller
            // than the size where it fully fits the canvas.
            let new_zoom = (old_zoom * factor).clamp(fit_zoom, 32.0);
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

    // Draw: dark background, then the image clipped to the canvas.
    let painter = ui.painter();
    painter.rect_filled(canvas_rect, 0.0, preview.bg.color());
    painter.rect_stroke(canvas_rect, 0.0, Stroke::new(1.0, grid_line()));

    let img_tl = canvas_rect.center() + preview.pan - draw_size * 0.5;
    let img_rect = egui::Rect::from_min_size(img_tl, draw_size);
    painter.with_clip_rect(canvas_rect).image(
        texture.id(),
        img_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );

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

    // Apply a deferred image/mip change now that `data`'s borrow has ended.
    if redecode {
        preview.decoded = None;
        preview.texture_dirty = true;
    }
}

/// Field-aware diff of two same-group tags: walk both root structs in parallel
/// (same layout → same field order) and collect every differing leaf value plus
/// block element-count mismatches. Returns the diffs and whether the cap was hit.

pub(in crate::app) fn build_bitmap_preview(
    tag: &TagFile,
    image_index: usize,
    mip_index: usize,
) -> anyhow::Result<BitmapPreviewData> {
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
    let rgba = decode_to_rgba8(
        format,
        width,
        height,
        &pixel_bytes[offset..offset + mip_len],
        bitmap.p8_palette(),
    )?;
    Ok(BitmapPreviewData {
        width,
        height,
        image_count,
        mip_count,
        format_name: image.format_name().unwrap_or_else(|| format!("{format:?}")),
        type_name: image.type_name().unwrap_or_else(|| "2D texture".to_owned()),
        rgba,
    })
}

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
