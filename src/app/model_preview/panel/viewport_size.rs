//! The viewport's size setting and the viewport with its stats footer.

use super::*;

pub(in crate::app) fn draw_model_viewport_size_input(ui: &mut Ui, model_preview_size: &mut f32) {
    let mut percent = model_preview_size_percent(*model_preview_size);
    let response = ui.add(
        egui::DragValue::new(&mut percent)
            .range(
                model_preview_size_percent(MIN_MODEL_PREVIEW_SIZE)
                    ..=model_preview_size_percent(MAX_MODEL_PREVIEW_SIZE),
            )
            .speed(1.0)
            .max_decimals(0)
            .suffix("%"),
    );
    if response.changed() {
        *model_preview_size = model_preview_size_from_percent(percent);
    }
}

pub(in crate::app::model_preview) fn model_preview_size_percent(model_preview_size: f32) -> f32 {
    model_preview_size * 100.0
}

pub(in crate::app::model_preview) fn model_preview_size_from_percent(percent: f32) -> f32 {
    (percent / 100.0).clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE)
}

pub(in crate::app::model_preview) fn model_viewport_size(available_width: f32, model_preview_size: f32) -> Vec2 {
    let scale = model_preview_size.clamp(MIN_MODEL_PREVIEW_SIZE, MAX_MODEL_PREVIEW_SIZE);
    let desired = Vec2::new(470.0 * scale, 300.0 * scale);
    // Never force a minimum wider than the actual section. Doing so made the
    // GL callback overflow its panel while egui clipped only the width, which
    // visually stretched the model as the window narrowed.
    let width = desired.x.min(available_width.max(1.0));
    Vec2::new(width, desired.y * (width / desired.x))
}

pub(in crate::app::model_preview) fn draw_model_viewport_with_stats(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    // Hold the viewport until the textures land, rather than drawing the model
    // untextured and re-shading it a second later — a model that changes
    // appearance under the cursor reads as a glitch, not as progress.
    let waiting_for_textures = state.render_mode.uses_textures()
        && !data.preview.materials.is_empty()
        && data.textures.is_none();
    if waiting_for_textures {
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        ui.painter()
            .rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0_f32, foundation_input_edge()),
                egui::StrokeKind::Middle,
            );
        crate::app::shell::loading::paint_loading_rings(ui, rect);
    } else {
        draw_model_viewport(ui, data, state, desired_size);
    }
    let footer_width = ui.available_width().max(desired_size.x);
    let (footer_rect, _) = ui.allocate_exact_size(
        Vec2::new(footer_width, MODEL_PREVIEW_STATS_FOOTER_HEIGHT),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(footer_rect, 0.0, foundation_section_bar());
    ui.painter().text(
        footer_rect.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ),
        FontId::proportional(10.0),
        foundation_block_text(),
    );
    // What the load left out, which would otherwise just be missing.
    if !data.notes.is_empty() {
        let summary = match data.notes.len() {
            1 => "1 part of this preview left out".to_owned(),
            count => format!("{count} parts of this preview left out"),
        };
        ui.label(RichText::new(summary).small().color(subtle_dark())).on_hover_text(data.notes.join("\n"));
    }
    // Why a material drew untextured, which the resolve records and nothing
    // showed: the model just looked flat.
    if state.render_mode.uses_textures()
        && let Some(textures) = data.textures.as_deref()
        && let Some((summary, detail)) = texture_resolve_note(
            textures,
            &data.preview.materials,
            state.game_shaders,
        )
    {
        ui.label(RichText::new(summary).small().color(subtle_dark()))
            .on_hover_text(detail);
    }
}

/// A line saying which materials resolved badly, and the reasons, for hover.
/// `None` when every material resolved from its definition.
pub(in crate::app::model_preview) fn texture_resolve_note(
    textures: &[MaterialTextures],
    materials: &[RenderModelPreviewMaterial],
    game_shaders: bool,
) -> Option<(String, String)> {
    let name = |index: usize| {
        materials
            .get(index)
            .map(|material| material.shader_path.as_str())
            .filter(|path| !path.is_empty())
            .unwrap_or("(no shader)")
            .to_owned()
    };
    let failed: Vec<String> = textures
        .iter()
        .enumerate()
        .filter_map(|(index, texture)| {
            let bare = texture.bare_without_passes.as_ref().filter(|_| !game_shaders);
            let error = texture.error.as_ref().or(bare);
            error.map(|error| format!("{}: {error}", name(index)))
        })
        .collect();
    let partial: Vec<String> = textures
        .iter()
        .enumerate()
        .filter(|(_, texture)| texture.error.is_none() && texture.used_shader_parameters_only)
        .map(|(index, _)| name(index))
        .collect();
    if failed.is_empty() && partial.is_empty() {
        return None;
    }
    let mut summary = Vec::new();
    let mut detail = Vec::new();
    if !failed.is_empty() {
        summary.push(format!(
            "{} of {} materials untextured",
            failed.len(),
            textures.len()
        ));
        detail.extend(failed);
    }
    if !partial.is_empty() {
        summary.push(format!(
            "{} read without their definition or template defaults",
            partial.len()
        ));
        detail.push(format!(
            "Read from the shader's own parameters, without the defaults its render \
             method definition or shader template supplies, such as detail-map \
             tiling:\n{}",
            partial.join("\n")
        ));
    }
    Some((
        format!("{} — hover for why", summary.join("; ")),
        detail.join("\n"),
    ))
}
