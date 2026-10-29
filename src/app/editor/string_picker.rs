//! Shared presentation for string lookup menus; providers own resolution and validation.
use super::*;
use blam_tags::string_editor::MenuItem;

pub(super) const POPUP_MIN_WIDTH: f32 = 250.0;
const MAX_WIDTH: f32 = 400.0;

pub(super) fn caption(editor: &str, original: &str) -> String {
    let known = match editor {
        "material" => "Materials",
        "object_function" | "object_function_no_own" | "object_function_panel" => {
            "Object Functions"
        }
        "object_marker" => "Cinematic Object Markers",
        "marker_names" => "Marker Names",
        "hud_message" => "HUD Messages",
        "ai_dialogue_vocalization" => "AI Dialogue",
        "ai_activities" => "AI Activities",
        "ai_formations" => "AI Formations",
        "ai_cue_template" => "AI Cue Templates",
        "ai_performance_template" => "AI Performance Templates",
        "ai_squad_template" => "AI Squad Templates",
        "app_selection" => "Apps",
        "challenge" => "Challenges",
        "cheap_particle_type" => "Particle Types",
        "cinematic_object_variant" | "object_variant" => "Object Variants",
        "commendation" | "commendation_selection" => "Commendations",
        "game_achievement" => "Achievements",
        "game_avatar_award" => "Avatar Awards",
        "game_engine_event" => "Game Events",
        "game_medal" => "Medals",
        "generic_chud" => "HUD Strings",
        "generic_ui_sounds" => "UI Sounds",
        "incident" => "Incidents",
        "item_permutation_selection" => "Item Permutations",
        "location_name" => "Location Names",
        "model_permutation_selection" => "Model Permutations",
        "model_region_selection" => "Model Regions",
        "ordnance_selection" => "Ordnance",
        "player_item_selection" => "Player Items",
        "portrait_pose_selection" => "Portrait Poses",
        "style_dropshadow_selection" => "Drop Shadows",
        "style_gradient_selection" => "Gradients",
        "style_outer_glow_selection" => "Outer Glows",
        "supply_depot_selection" => "Supply Depots",
        "visor_color_selection" => "Visor Colors",
        "voice_selection" => "Voices",
        _ => return original.to_owned(),
    };
    known.to_owned()
}

pub(super) fn image(ui: &Ui, editor: &str) -> Option<egui::Image<'static>> {
    let icon = match editor {
        "object_function" | "object_function_no_own" | "object_function_panel" => {
            ButtonIcon::Function
        }
        "object_marker" | "marker_names" => ButtonIcon::Markers,
        "default_model_variant" => ButtonIcon::Model,
        _ => return None,
    };
    Some(button_icon_image(ui, icon, text_dark(), 16.0))
}

pub(super) fn search(
    ui: &mut Ui,
    filter: &mut String,
    id: egui::Id,
    focus: bool,
    width: f32,
    hint: &str,
) {
    ui.set_width(width.clamp(POPUP_MIN_WIDTH, MAX_WIDTH));
    // An Area starts each frame with its previous size. Restore room to grow
    // before laying out results, so clearing a short search can expand again.
    ui.set_max_height(ui.ctx().content_rect().height());
    let response = ui.add_sized(
        Vec2::new(ui.available_width(), BUTTON_HEIGHT),
        singleline_text_edit(filter)
            .min_size(Vec2::new(0.0, BUTTON_HEIGHT))
            .id(id)
            .hint_text(hint),
    );
    if focus {
        response.request_focus();
    }
    if response.changed() {
        ui.ctx().request_repaint();
    }
}

/// Follow visible content up to the screen-dependent scrolling limit.
pub(super) fn scroll(ui: &Ui, id: egui::Id) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .min_scrolled_height(BUTTON_HEIGHT)
        .max_height((ui.ctx().content_rect().height() * 0.6).clamp(120.0, 360.0))
        .auto_shrink([false, true])
}

fn label_width(ui: &Ui, label: &str, submenu: bool) -> f32 {
    let text = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    (text.size().x + ui.spacing().button_padding.x * 2.0
        // right_text adds two gaps around its flexible spacer.
        + ui.spacing().icon_spacing * 2.0
        + if submenu { BUTTON_HEIGHT } else { 0.0 })
    .ceil()
}

/// Size one submenu to its own labels, allowing padding and disclosure arrows.
pub(super) fn menu_width(ui: &Ui, labels: impl IntoIterator<Item = (String, bool)>) -> f32 {
    labels
        .into_iter()
        .map(|(label, submenu)| label_width(ui, &label, submenu))
        .fold(64.0, f32::max)
        .min(MAX_WIDTH)
}

/// Main popups also reserve room for headings/entries indented by search.
pub(super) fn popup_width(ui: &Ui, labels: impl IntoIterator<Item = (String, bool, usize)>) -> f32 {
    labels
        .into_iter()
        .map(|(label, submenu, depth)| {
            label_width(ui, &label, submenu) + ui.spacing().indent * depth as f32
        })
        .fold(POPUP_MIN_WIDTH, f32::max)
        .min(MAX_WIDTH)
}

fn with_tooltip(response: egui::Response, text: &str, details: Option<&str>) -> egui::Response {
    let truncated = response
        .intrinsic_size()
        .is_some_and(|size| size.x > response.rect.width() + 0.5);
    let tooltip = match (truncated, details.filter(|details| !details.is_empty())) {
        (true, Some(details)) => Some(format!("{text}\n\n{details}")),
        (true, None) => Some(text.to_owned()),
        (false, Some(details)) => Some(details.to_owned()),
        (false, None) => None,
    };
    if let Some(tooltip) = tooltip {
        response
            .on_hover_text(tooltip.clone())
            .on_disabled_hover_text(tooltip)
    } else {
        response
    }
}

pub(super) fn entry(
    ui: &mut Ui,
    text: &str,
    enabled: bool,
    details: Option<&str>,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text)
            .right_text("")
            .min_size(Vec2::new(ui.available_width(), BUTTON_HEIGHT))
            .truncate(),
    );
    with_tooltip(response, text, details)
}

pub(super) fn group_heading(ui: &mut Ui, caption: egui::text::LayoutJob) {
    let text = caption.text.clone();
    let response = ui.add(egui::Label::new(caption).truncate());
    with_tooltip(response, &text, None);
}

pub(super) fn group_caption(ui: &Ui, label: &str, count: usize) -> egui::text::LayoutJob {
    let mut caption = egui::text::LayoutJob::default();
    let format = egui::TextFormat {
        font_id: egui::TextStyle::Button.resolve(ui.style()),
        color: ui.visuals().text_color(),
        ..Default::default()
    };
    caption.append(label, 0.0, format.clone());
    caption.append(
        &format!(" ({count})"),
        0.0,
        egui::TextFormat {
            color: format.color.gamma_multiply(0.5),
            ..format
        },
    );
    caption
}

pub(super) fn submenu(
    ui: &mut Ui,
    id: egui::Id,
    caption: egui::text::LayoutJob,
    contents: impl FnOnce(&mut Ui),
) {
    ui.scope_builder(egui::UiBuilder::new().id(id), |ui| {
        let text = caption.text.clone();
        let button = egui::Button::new(caption)
            .right_text("    ")
            .min_size(Vec2::new(ui.available_width(), BUTTON_HEIGHT))
            .truncate();
        let (response, _) =
            egui::containers::menu::SubMenuButton::from_button(button).ui(ui, |ui| {
                ui.set_max_height(ui.ctx().content_rect().height());
                contents(ui);
            });
        crate::app::ui_kit::button_icons::paint_disclosure_chevron(
            ui,
            egui::pos2(response.rect.right() - 12.0, response.rect.center().y),
            0.0,
        );
        with_tooltip(response, &text, None);
    });
}

/// Use the same first-underscore grouping as the string list pickers. Only
/// presentation changes: leaves keep their complete IDs for search and edits.
pub(super) fn prefix_groups(items: &[MenuItem]) -> Vec<MenuItem> {
    use super::string_groups::{StringIdGroup, group_string_ids};
    group_string_ids(items.iter().map(|item| item.text.as_str()))
        .into_iter()
        .map(|group| match group {
            StringIdGroup::Entry(index) => items[index].clone(),
            StringIdGroup::Prefix { prefix, indices } => MenuItem::submenu(
                prefix,
                indices.into_iter().map(|index| items[index].clone()).collect(),
            ),
        })
        .collect()
}

fn match_count(item: &MenuItem, query: &str) -> usize {
    if item.is_submenu() {
        item.children
            .iter()
            .map(|child| match_count(child, query))
            .sum()
    } else {
        usize::from(item.text.to_lowercase().contains(query))
    }
}

/// Browse through submenus; search leaves at any depth and retain their headings inline.
pub(super) fn menu_items(
    ui: &mut Ui,
    items: &[MenuItem],
    filter: &str,
    id: egui::Id,
    picked: &mut Option<String>,
) {
    let query = filter.to_lowercase();
    for (index, item) in items.iter().enumerate() {
        let count = match_count(item, &query);
        if count == 0 {
            continue;
        }
        let child_id = id.with(index);
        if item.is_submenu() {
            let caption = group_caption(ui, &item.text, count);
            if filter.is_empty() {
                submenu(ui, child_id, caption, |ui| {
                    let width = menu_width(
                        ui,
                        item.children.iter().map(|child| {
                            let label = if child.is_submenu() {
                                format!("{} ({})", child.text, match_count(child, ""))
                            } else {
                                child.text.clone()
                            };
                            (label, child.is_submenu())
                        }),
                    );
                    ui.set_width(width);
                    scroll(ui, child_id.with("scroll")).show(ui, |ui| {
                        menu_items(ui, &item.children, filter, child_id, picked);
                    });
                });
            } else {
                group_heading(ui, caption);
                ui.indent(child_id, |ui| {
                    menu_items(ui, &item.children, filter, child_id, picked)
                });
            }
        } else if entry(ui, &item.text, true, None).clicked() {
            *picked = Some(item.text.clone());
            crate::app::ui_kit::button_icons::close_menu(ui);
        }
    }
}

pub(super) fn menu_has_matches(items: &[MenuItem], filter: &str) -> bool {
    let query = filter.to_lowercase();
    items.iter().any(|item| match_count(item, &query) > 0)
}

pub(super) fn menu_popup_width(ui: &Ui, items: &[MenuItem], filter: &str) -> f32 {
    fn collect(
        items: &[MenuItem],
        query: &str,
        depth: usize,
        rows: &mut Vec<(String, bool, usize)>,
    ) {
        for item in items {
            let count = match_count(item, query);
            if count == 0 {
                continue;
            }
            if item.is_submenu() {
                rows.push((format!("{} ({count})", item.text), query.is_empty(), depth));
                if !query.is_empty() {
                    collect(&item.children, query, depth + 1, rows);
                }
            } else {
                rows.push((item.text.clone(), false, depth));
            }
        }
    }
    let mut rows = Vec::new();
    collect(items, &filter.to_lowercase(), 0, &mut rows);
    popup_width(ui, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_groups_keep_full_variant_ids_and_singletons() {
        let items: Vec<_> = ["minor_scl", "plain", "minor_dog", "major_scl", "major_dog", "odd_one"]
            .into_iter().map(MenuItem::item).collect();
        let grouped = prefix_groups(&items);
        assert_eq!(grouped, vec![
            MenuItem::submenu("major", vec![MenuItem::item("major_dog"), MenuItem::item("major_scl")]),
            MenuItem::submenu("minor", vec![MenuItem::item("minor_dog"), MenuItem::item("minor_scl")]),
            MenuItem::item("odd_one"), MenuItem::item("plain"),
        ]);
        assert!(menu_has_matches(&grouped, "DOG"));
        assert!(!menu_has_matches(&grouped, "unmatched"));
        let menu = blam_tags::string_editor::Menu { items: grouped, ..Default::default() };
        assert!(menu.accepts("MINOR_DOG"));
        assert!(!menu.accepts("minor"), "group headings are not variant IDs");
        assert_eq!(items[0].text, "minor_scl");
        assert!(items.iter().all(|item| !item.is_submenu()));
    }
}