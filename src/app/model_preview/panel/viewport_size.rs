//! The viewport with its stats footer.

use super::super::super::ui_kit::style::{
    foundation_block_text, foundation_input_edge, foundation_section_bar, subtle_dark,
};
use super::super::{
    MaterialTextures, ModelPreviewData, ModelPreviewState, RenderModelPreviewMaterial,
    draw_model_viewport, model_gl_notes,
};
use super::section::MODEL_PREVIEW_STATS_FOOTER_HEIGHT;
use eframe::egui;
use eframe::egui::{FontId, RichText, Sense, Stroke, Ui, Vec2};

pub(in crate::app::model_preview) fn draw_model_viewport_with_stats(
    ui: &mut Ui,
    data: &ModelPreviewData,
    state: &mut ModelPreviewState,
    desired_size: Vec2,
) {
    // Hold the viewport until the textures land, rather than drawing the model
    // untextured and re-shading it a second later — a model that changes
    // appearance under the cursor reads as a glitch, not as progress. Game
    // Shaders turned on waits the same way for the textures it resolves.
    let waiting_for_textures = state.render_mode.uses_textures()
        && !data.preview.materials.is_empty()
        && data.textures_for(state.game_shaders).is_none();
    if waiting_for_textures {
        let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
        ui.painter().rect_stroke(
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
    // The stats, then what the load left out and why materials drew
    // untextured, all in the footer's one row so the card keeps its height.
    let mut footer = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(footer_rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    footer.spacing_mut().item_spacing.x = 12.0;
    footer.label(
        RichText::new(format!(
            "{} vertices, {} triangles",
            data.preview.vertices.len(),
            data.preview.indices.len() / 3
        ))
        .font(FontId::proportional(10.0))
        .color(foundation_block_text()),
    );
    // What the load left out, which would otherwise just be missing.
    if !data.notes.is_empty() {
        let summary = match data.notes.len() {
            1 => "1 part of this preview left out".to_owned(),
            count => format!("{count} parts of this preview left out"),
        };
        footer
            .label(RichText::new(summary).small().color(subtle_dark()))
            .on_hover_text(data.notes.join("\n"));
    }
    // Why a material drew untextured, which the resolve records and nothing
    // showed: the model just looked flat.
    if state.render_mode.uses_textures()
        && let Some(textures) = data.textures.as_deref()
        && let Some((summary, detail)) =
            texture_resolve_note(textures, &data.preview.materials, state.game_shaders)
    {
        footer
            .label(RichText::new(summary).small().color(subtle_dark()))
            .on_hover_text(detail);
    }
    // What Game Shaders left out, which otherwise only reaches a console a
    // Windows release build does not have: the model just drew as with them
    // off.
    if state.game_shaders
        && let Some((summary, detail)) = game_shaders_note(
            model_gl_notes(data.geometry_id),
            data.textures.as_deref().map_or(&[], Vec::as_slice),
            &data.preview.materials,
        )
    {
        footer
            .label(
                RichText::new(summary)
                    .small()
                    .color(footer.visuals().warn_fg_color),
            )
            .on_hover_text(detail);
    }
}

/// A line saying Game Shaders left something out, and why, for hover: what
/// the renderer noted while drawing (`renderer`, with the context it drew
/// in) and the materials whose own shader was not translated. `None` when
/// nothing was left out.
pub(in crate::app::model_preview) fn game_shaders_note(
    renderer: Option<(Vec<String>, String)>,
    textures: &[MaterialTextures],
    materials: &[RenderModelPreviewMaterial],
) -> Option<(String, String)> {
    let untranslated: Vec<String> = textures
        .iter()
        .enumerate()
        .filter_map(|(index, texture)| {
            let error = texture.gen3_error.as_ref()?;
            let name = materials
                .get(index)
                .map(|material| material.shader_path.as_str())
                .filter(|path| !path.is_empty())
                .unwrap_or("(no shader)");
            Some(format!("{name}: {error}"))
        })
        .collect();
    let (notes, context) = renderer.unwrap_or_default();
    if notes.is_empty() && untranslated.is_empty() {
        return None;
    }
    let mut detail = notes;
    if !untranslated.is_empty() {
        detail.push(format!(
            "{} of {} materials not translated, drawn as with Game Shaders off:",
            untranslated.len(),
            textures.len()
        ));
        detail.extend(untranslated);
    }
    if !context.is_empty() {
        detail.push(format!("Context: {context}"));
    }
    Some((
        "Game Shaders incomplete — hover for why".to_owned(),
        detail.join("\n"),
    ))
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
            let bare = texture
                .bare_without_passes
                .as_ref()
                .filter(|_| !game_shaders);
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
