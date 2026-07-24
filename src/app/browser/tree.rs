//! Browser tree, list, context-menu, and entry presentation.
//! It owns tag-browser filtering and presentation; source discovery, document loading, and edit application belong elsewhere.

use super::*;

/// A pending "reveal in tree" request threaded through the tree draw: it force-
/// opens the folder nodes along `remaining` (ancestor labels not yet descended)
/// and scrolls the matching leaf (`key`) into view. One-shot — cleared by the
/// caller after the frame.
#[derive(Clone, Copy)]
pub(in crate::app) struct Reveal<'a> {
    pub(in crate::app) key: &'a str,
    pub(in crate::app) remaining: &'a [String],
}

impl<'a> Reveal<'a> {
    /// True when this node's label is the next ancestor to descend into.
    fn matches_node(self, label: &str) -> bool {
        self.remaining.first().map(String::as_str) == Some(label)
    }

    /// The reveal to forward to a matching node's children (one segment shorter).
    fn descend(self) -> Reveal<'a> {
        Reveal {
            key: self.key,
            remaining: self.remaining.get(1..).unwrap_or(&[]),
        }
    }

    /// The leaf key to scroll, but only once all ancestors have been descended
    /// (i.e. this node directly contains the target entry).
    fn leaf_key(self) -> Option<&'a str> {
        self.remaining.is_empty().then_some(self.key)
    }
}

/// Build the reference-input string for a tag entry — `"fourcc:back\\slash"`
/// (group four-CC + extension-less backslash path) — matching the format
/// [`choose_tag_reference_input`] produces, for use as a drag payload.
pub(in crate::app) fn entry_reference_input(entry: &TagEntry) -> String {
    let display = &entry.display_path;
    let without_ext = match display.rfind('.') {
        Some(dot) => &display[..dot],
        None => display.as_str(),
    };
    format_tag_reference_input(entry.group_tag, without_ext)
}

/// Forward-slash, extension-less relative path of an entry — the form shader
/// bitmap rows use for their references.
fn entry_rel_path(entry: &TagEntry) -> String {
    let display = &entry.display_path;
    let without_ext = match display.rfind('.') {
        Some(dot) => &display[..dot],
        None => display.as_str(),
    };
    without_ext.replace('\\', "/")
}

fn context_menu_button(ui: &mut Ui, label: &str) -> egui::Response {
    let text = RichText::new(label).color(text_dark());
    let button = match context_menu_icon(label) {
        Some(icon) => {
            egui::Button::image_and_text(button_icon_image(ui, icon, text_dark(), 16.0), text)
        }
        None => egui::Button::new(text),
    };
    ui.add_sized([ui.available_width().max(280.0), 28.0], button)
}

fn context_menu_primary_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let text = RichText::new(label).color(text_dark());
    let button = match context_menu_icon(label) {
        Some(icon) => {
            egui::Button::image_and_text(button_icon_image(ui, icon, text_dark(), 16.0), text)
        }
        None => egui::Button::new(text),
    };
    ui.add_enabled(enabled, button.min_size(Vec2::new(92.0, 44.0)))
}

fn context_menu_icon(label: &str) -> Option<ButtonIcon> {
    match label {
        "Rename" => Some(ButtonIcon::Rename),
        "Move" => Some(ButtonIcon::Move),
        "Open with File Explorer" => Some(ButtonIcon::FileExplorer),
        "Add to Favorites" | "Remove from Favorites" => Some(ButtonIcon::Favourite),
        "Copy Tag Path" => Some(ButtonIcon::CopyPath),
        "Find Tag References..." => Some(ButtonIcon::Find),
        "Dump Tag to JSON..." => Some(ButtonIcon::Json),
        _ => None,
    }
}

fn context_menu_separator(ui: &mut Ui) {
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(3.0);
}

fn style_tag_context_menu(ui: &mut Ui) {
    ui.set_min_width(300.0);
    ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
    ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
    ui.spacing_mut().interact_size.y = 28.0;
    ui.visuals_mut().override_text_color = Some(text_dark());
    ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
    ui.visuals_mut().widgets.hovered.bg_fill = context_menu_hover();
    ui.visuals_mut().widgets.active.bg_fill = context_menu_hover();
}

fn entry_filename_lower(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path)
        .to_ascii_lowercase()
}

/// Reorder a node's entry indices for display. `Natural` borrows the input with
/// no allocation; `Name`/`Type` clone-and-sort.
fn ordered_indices<'a>(
    indices: &'a [usize],
    entries: &[TagEntry],
    sort: BrowserSort,
) -> std::borrow::Cow<'a, [usize]> {
    use std::borrow::Cow;
    match sort {
        BrowserSort::Natural => Cow::Borrowed(indices),
        BrowserSort::Name => {
            let mut sorted = indices.to_vec();
            sorted.sort_by(|&a, &b| {
                entry_filename_lower(&entries[a]).cmp(&entry_filename_lower(&entries[b]))
            });
            Cow::Owned(sorted)
        }
        BrowserSort::Type => {
            let mut sorted = indices.to_vec();
            sorted.sort_by(|&a, &b| {
                let key_a = (
                    format_group_tag(entries[a].group_tag),
                    entry_filename_lower(&entries[a]),
                );
                let key_b = (
                    format_group_tag(entries[b].group_tag),
                    entry_filename_lower(&entries[b]),
                );
                key_a.cmp(&key_b)
            });
            Cow::Owned(sorted)
        }
    }
}

fn ordered_child_indices(children: &[TagTreeNode], sort: BrowserSort) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..children.len()).collect();
    if !matches!(sort, BrowserSort::Natural) {
        indices.sort_by(|&a, &b| {
            children[a]
                .label
                .to_ascii_lowercase()
                .cmp(&children[b].label.to_ascii_lowercase())
        });
    }
    indices
}

/// Folder-label ancestors of a tag's display path (filename removed).
pub(in crate::app) fn ancestor_labels(display_path: &str) -> Vec<String> {
    let mut segments: Vec<String> = display_path
        .replace('\\', "/")
        .split('/')
        .map(str::to_owned)
        .collect();
    segments.pop(); // drop the filename
    segments
}

pub(in crate::app) fn draw_tree(
    ui: &mut Ui,
    tree: &TagTree,
    entries: &[TagEntry],
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    groups_mode: bool,
    reveal: Option<Reveal>,
    sort: BrowserSort,
    folders_before_tags: bool,
    favorite_keys: Option<&HashSet<String>>,
) -> Option<BrowserAction> {
    let mut clicked = None;
    if !folders_before_tags {
        clicked = clicked.or_else(|| {
            draw_entry_list(
                ui,
                &tree.entries,
                entries,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                reveal.and_then(Reveal::leaf_key),
                sort,
                favorite_keys,
            )
        });
    }
    let child_sort = if groups_mode {
        BrowserSort::Natural
    } else {
        sort
    };
    for index in ordered_child_indices(&tree.children, child_sort) {
        let node = &tree.children[index];
        clicked = clicked.or_else(|| {
            draw_tree_node(
                ui,
                node,
                entries,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                groups_mode,
                reveal,
                sort,
                folders_before_tags,
                favorite_keys,
            )
        });
    }
    if folders_before_tags {
        clicked = clicked.or_else(|| {
            draw_entry_list(
                ui,
                &tree.entries,
                entries,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                reveal.and_then(Reveal::leaf_key),
                sort,
                favorite_keys,
            )
        });
    }
    clicked
}

pub(in crate::app) fn draw_tree_lazy(
    ui: &mut Ui,
    tree: &mut TagTree,
    entries: &mut Vec<TagEntry>,
    group_tree: &mut TagTree,
    root: &Path,
    names: &TagNameIndex,
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    status_update: &mut Option<String>,
    reveal: Option<Reveal>,
    sort: BrowserSort,
    folders_before_tags: bool,
    favorite_keys: Option<&HashSet<String>>,
    expert_mode: bool,
) -> Option<BrowserAction> {
    let mut clicked = None;
    if !folders_before_tags {
        clicked = clicked.or_else(|| {
            draw_entry_list(
                ui,
                &tree.entries,
                entries,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                reveal.and_then(Reveal::leaf_key),
                sort,
                favorite_keys,
            )
        });
    }
    for index in ordered_child_indices(&tree.children, sort) {
        let node = &mut tree.children[index];
        clicked = clicked.or_else(|| {
            draw_tree_node_lazy(
                ui,
                node,
                entries,
                group_tree,
                root,
                names,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                status_update,
                reveal,
                sort,
                folders_before_tags,
                favorite_keys,
                expert_mode,
            )
        });
    }
    if folders_before_tags {
        clicked = clicked.or_else(|| {
            draw_entry_list(
                ui,
                &tree.entries,
                entries,
                selected,
                filter,
                show_prefixes,
                double_click_to_open,
                reveal.and_then(Reveal::leaf_key),
                sort,
                favorite_keys,
            )
        });
    }
    clicked
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn draw_tree_node_lazy(
    ui: &mut Ui,
    node: &mut TagTreeNode,
    entries: &mut Vec<TagEntry>,
    group_tree: &mut TagTree,
    root: &Path,
    names: &TagNameIndex,
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    status_update: &mut Option<String>,
    reveal: Option<Reveal>,
    sort: BrowserSort,
    folders_before_tags: bool,
    favorite_keys: Option<&HashSet<String>>,
    expert_mode: bool,
) -> Option<BrowserAction> {
    if !filter.is_empty() && !lazy_node_matches(node, entries, filter) {
        return None;
    }
    let on_path = reveal.is_some_and(|reveal| reveal.matches_node(&node.label));
    let inner_reveal = on_path.then(|| reveal.expect("on_path implies reveal").descend());
    let mut clicked = None;
    let folder_label = if show_prefixes {
        format!("[folder] {}", node.label)
    } else {
        node.label.clone()
    };
    let response = egui::CollapsingHeader::new(RichText::new(folder_label).color(text_dark()))
        .icon(folder_arrow_icon)
        .default_open(!filter.is_empty())
        .open(on_path.then_some(true))
        .show(ui, |ui| {
            if !node.entries_loaded {
                match load_folder_node_entries(root, node, entries, names) {
                    Ok(()) => {
                        *group_tree = crate::source::build_group_tree(entries);
                        *status_update = Some(format!(
                            "Loaded {} tag(s) from {}",
                            node.entries.len(),
                            node.label
                        ));
                    }
                    Err(error) => {
                        *status_update = Some(format!(
                            "Failed to load folder {}: {error}",
                            node.rel_path.display()
                        ));
                    }
                }
            }
            let leaf_key = inner_reveal.and_then(Reveal::leaf_key);
            if !folders_before_tags {
                if clicked.is_none() {
                    clicked = draw_entry_list(
                        ui,
                        &node.entries,
                        entries,
                        selected,
                        filter,
                        show_prefixes,
                        double_click_to_open,
                        leaf_key,
                        sort,
                        favorite_keys,
                    );
                } else {
                    let _ = draw_entry_list(
                        ui,
                        &node.entries,
                        entries,
                        selected,
                        filter,
                        show_prefixes,
                        double_click_to_open,
                        leaf_key,
                        sort,
                        favorite_keys,
                    );
                }
            }
            for index in ordered_child_indices(&node.children, sort) {
                let child = &mut node.children[index];
                if clicked.is_none() {
                    clicked = draw_tree_node_lazy(
                        ui,
                        child,
                        entries,
                        group_tree,
                        root,
                        names,
                        selected,
                        filter,
                        show_prefixes,
                        double_click_to_open,
                        status_update,
                        inner_reveal,
                        sort,
                        folders_before_tags,
                        favorite_keys,
                        expert_mode,
                    );
                }
            }
            if folders_before_tags {
                if clicked.is_none() {
                    clicked = draw_entry_list(
                        ui,
                        &node.entries,
                        entries,
                        selected,
                        filter,
                        show_prefixes,
                        double_click_to_open,
                        leaf_key,
                        sort,
                        favorite_keys,
                    );
                } else {
                    let _ = draw_entry_list(
                        ui,
                        &node.entries,
                        entries,
                        selected,
                        filter,
                        show_prefixes,
                        double_click_to_open,
                        leaf_key,
                        sort,
                        favorite_keys,
                    );
                }
            }
        });
    response.header_response.context_menu(|ui| {
        if ui.button("Move to...").clicked() {
            clicked = Some(BrowserAction::MoveLooseFolder {
                rel_path: node.rel_path.clone(),
                label: node.label.clone(),
            });
            ui.close_menu();
        }
        if ui.button("Copy to...").clicked() {
            clicked = Some(BrowserAction::CopyLooseFolder {
                rel_path: node.rel_path.clone(),
                label: node.label.clone(),
            });
            ui.close_menu();
        }
        if expert_mode {
            if ui.button("Save folder for another game...").clicked() {
                clicked = Some(BrowserAction::ConvertLooseFolder {
                    rel_path: node.rel_path.clone(),
                    label: node.label.clone(),
                });
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Dump folder to JSON...").clicked() {
            clicked = Some(BrowserAction::DumpLooseFolderJson {
                rel_path: node.rel_path.clone(),
                label: node.label.clone(),
            });
            ui.close_menu();
        }
        let bitmap_keys = collect_bitmap_keys(node, entries);
        if bitmap_keys.is_empty() {
            ui.label(RichText::new("No loaded bitmap tags in this folder").color(subtle_dark()));
        } else if ui
            .button(format!("Extract loaded bitmaps... ({})", bitmap_keys.len()))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractBitmapFolder(bitmap_keys));
            ui.close_menu();
        }
        let material_shader_keys = collect_material_shader_keys(node, entries);
        if material_shader_keys.is_empty() {
            ui.label(
                RichText::new("No loaded material shaders in this folder").color(subtle_dark()),
            );
        } else if ui
            .button(format!(
                "Extract loaded material shader sources... ({})",
                material_shader_keys.len()
            ))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractMaterialShaderSourceFolder(
                material_shader_keys,
            ));
            ui.close_menu();
        }
        let hlsl_include_keys = collect_hlsl_include_keys(node, entries);
        if hlsl_include_keys.is_empty() {
            ui.label(RichText::new("No loaded HLSL includes in this folder").color(subtle_dark()));
        } else if ui
            .button(format!(
                "Extract loaded HLSL includes... ({})",
                hlsl_include_keys.len()
            ))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractHlslIncludeFolder(hlsl_include_keys));
            ui.close_menu();
        }
    });
    clicked
}

pub(in crate::app) fn draw_tree_node(
    ui: &mut Ui,
    node: &TagTreeNode,
    entries: &[TagEntry],
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    groups_mode: bool,
    reveal: Option<Reveal>,
    sort: BrowserSort,
    folders_before_tags: bool,
    favorite_keys: Option<&HashSet<String>>,
) -> Option<BrowserAction> {
    if !filter.is_empty() && !node_matches(node, entries, filter) {
        return None;
    }
    let on_path = reveal.is_some_and(|reveal| reveal.matches_node(&node.label));
    let inner_reveal = on_path.then(|| reveal.expect("on_path implies reveal").descend());
    let mut clicked = None;
    let body = |ui: &mut Ui| {
        let leaf_key = inner_reveal.and_then(Reveal::leaf_key);
        if !folders_before_tags {
            if clicked.is_none() {
                clicked = draw_entry_list(
                    ui,
                    &node.entries,
                    entries,
                    selected,
                    filter,
                    show_prefixes,
                    double_click_to_open,
                    leaf_key,
                    sort,
                    favorite_keys,
                );
            } else {
                let _ = draw_entry_list(
                    ui,
                    &node.entries,
                    entries,
                    selected,
                    filter,
                    show_prefixes,
                    double_click_to_open,
                    leaf_key,
                    sort,
                    favorite_keys,
                );
            }
        }
        for index in ordered_child_indices(&node.children, sort) {
            let child = &node.children[index];
            if clicked.is_none() {
                clicked = draw_tree_node(
                    ui,
                    child,
                    entries,
                    selected,
                    filter,
                    show_prefixes,
                    double_click_to_open,
                    groups_mode,
                    inner_reveal,
                    sort,
                    folders_before_tags,
                    favorite_keys,
                );
            }
        }
        if folders_before_tags {
            if clicked.is_none() {
                clicked = draw_entry_list(
                    ui,
                    &node.entries,
                    entries,
                    selected,
                    filter,
                    show_prefixes,
                    double_click_to_open,
                    leaf_key,
                    sort,
                    favorite_keys,
                );
            } else {
                let _ = draw_entry_list(
                    ui,
                    &node.entries,
                    entries,
                    selected,
                    filter,
                    show_prefixes,
                    double_click_to_open,
                    leaf_key,
                    sort,
                    favorite_keys,
                );
            }
        }
    };
    let header_response = if groups_mode {
        show_group_tree_header(
            ui,
            &node.label,
            show_prefixes,
            !filter.is_empty(),
            on_path,
            body,
        )
    } else {
        let folder_label = if show_prefixes {
            format!("[folder] {}", node.label)
        } else {
            node.label.clone()
        };
        egui::CollapsingHeader::new(RichText::new(folder_label).color(text_dark()))
            .icon(folder_arrow_icon)
            .default_open(!filter.is_empty())
            .open(on_path.then_some(true))
            .show(ui, body)
            .header_response
    };
    header_response.context_menu(|ui| {
        let tag_keys = collect_tag_keys(node, entries);
        if tag_keys.is_empty() {
            ui.label(RichText::new("No tags in this folder").color(subtle_dark()));
        } else if ui
            .button(format!("Dump folder to JSON... ({})", tag_keys.len()))
            .clicked()
        {
            clicked = Some(BrowserAction::DumpLoadedFolderJson(tag_keys));
            ui.close_menu();
        }

        let bitmap_keys = collect_bitmap_keys(node, entries);
        if bitmap_keys.is_empty() {
            ui.label(RichText::new("No bitmap tags in this folder").color(subtle_dark()));
        } else if ui
            .button(format!("Extract all bitmaps... ({})", bitmap_keys.len()))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractBitmapFolder(bitmap_keys));
            ui.close_menu();
        }

        let material_shader_keys = collect_material_shader_keys(node, entries);
        if material_shader_keys.is_empty() {
            ui.label(RichText::new("No material shaders in this folder").color(subtle_dark()));
        } else if ui
            .button(format!(
                "Extract material shader sources... ({})",
                material_shader_keys.len()
            ))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractMaterialShaderSourceFolder(
                material_shader_keys,
            ));
            ui.close_menu();
        }

        let hlsl_include_keys = collect_hlsl_include_keys(node, entries);
        if hlsl_include_keys.is_empty() {
            ui.label(RichText::new("No HLSL includes in this folder").color(subtle_dark()));
        } else if ui
            .button(format!(
                "Extract HLSL includes... ({})",
                hlsl_include_keys.len()
            ))
            .clicked()
        {
            clicked = Some(BrowserAction::ExtractHlslIncludeFolder(hlsl_include_keys));
            ui.close_menu();
        }
    });
    clicked
}

fn group_tree_label_parts(label: &str) -> (&str, &str) {
    label
        .rsplit_once(' ')
        .map_or(("", label), |(name, fourcc)| (name, fourcc))
}

fn show_group_tree_header<R>(
    ui: &mut Ui,
    label: &str,
    show_prefixes: bool,
    default_open: bool,
    force_open: bool,
    add_body: impl FnOnce(&mut Ui) -> R,
) -> egui::Response {
    let id = ui.make_persistent_id(label);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    if force_open {
        state.set_open(true);
    }

    let (name, fourcc) = group_tree_label_parts(label);
    let row = ui.horizontal(|ui| {
        let toggle = state.show_toggle_button(ui, folder_arrow_icon);
        let mut content = toggle.clone();
        let display_name = if show_prefixes && !name.is_empty() {
            format!("[folder] {name}")
        } else if show_prefixes {
            "[folder]".to_owned()
        } else {
            name.to_owned()
        };
        if !display_name.is_empty() {
            content = content.union(ui.label(RichText::new(display_name).color(text_dark())));
        }
        let badge = Frame::none()
            .fill(Color32::from_rgb(48, 58, 66))
            .stroke(Stroke::new(1.0, Color32::from_rgb(76, 89, 98)))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::symmetric(6.0, 1.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(fourcc)
                        .monospace()
                        .color(Color32::from_rgb(226, 235, 240)),
                )
            })
            .response;
        (toggle, content.union(badge))
    });
    let (toggle, content) = row.inner;
    let header = ui.interact(row.response.rect, id.with("header"), Sense::click());
    if header.clicked() && !toggle.clicked() {
        state.toggle(ui);
    }
    let response = header.union(content);
    state.show_body_indented(&response, ui, add_body);
    response
}

#[cfg(test)]
mod group_header_tests {
    use super::*;

    #[test]
    fn group_tree_label_splits_friendly_name_and_fourcc() {
        assert_eq!(group_tree_label_parts("control cntl"), ("control", "cntl"));
        assert_eq!(group_tree_label_parts("bloc"), ("", "bloc"));
    }
}

pub(in crate::app) fn collect_tag_keys(node: &TagTreeNode, entries: &[TagEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    collect_tag_keys_into(node, entries, &mut keys);
    keys
}

pub(in crate::app) fn collect_tag_keys_into(
    node: &TagTreeNode,
    entries: &[TagEntry],
    keys: &mut Vec<String>,
) {
    for &entry_index in &node.entries {
        if let Some(entry) = entries.get(entry_index) {
            keys.push(entry.key.clone());
        }
    }
    for child in &node.children {
        collect_tag_keys_into(child, entries, keys);
    }
}

pub(in crate::app) fn collect_bitmap_keys(node: &TagTreeNode, entries: &[TagEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    collect_bitmap_keys_into(node, entries, &mut keys);
    keys
}

pub(in crate::app) fn collect_bitmap_keys_into(
    node: &TagTreeNode,
    entries: &[TagEntry],
    keys: &mut Vec<String>,
) {
    for &entry_index in &node.entries {
        if let Some(entry) = entries.get(entry_index) {
            if is_bitmap_tag(entry) {
                keys.push(entry.key.clone());
            }
        }
    }
    for child in &node.children {
        collect_bitmap_keys_into(child, entries, keys);
    }
}

pub(in crate::app) fn collect_hlsl_include_keys(
    node: &TagTreeNode,
    entries: &[TagEntry],
) -> Vec<String> {
    let mut keys = Vec::new();
    collect_hlsl_include_keys_into(node, entries, &mut keys);
    keys
}

pub(in crate::app) fn collect_material_shader_keys(
    node: &TagTreeNode,
    entries: &[TagEntry],
) -> Vec<String> {
    let mut keys = Vec::new();
    collect_material_shader_keys_into(node, entries, &mut keys);
    keys
}

pub(in crate::app) fn collect_material_shader_keys_into(
    node: &TagTreeNode,
    entries: &[TagEntry],
    keys: &mut Vec<String>,
) {
    for &entry_index in &node.entries {
        if let Some(entry) = entries.get(entry_index) {
            if is_material_shader_browser_tag(entry) {
                keys.push(entry.key.clone());
            }
        }
    }
    for child in &node.children {
        collect_material_shader_keys_into(child, entries, keys);
    }
}

pub(in crate::app) fn collect_hlsl_include_keys_into(
    node: &TagTreeNode,
    entries: &[TagEntry],
    keys: &mut Vec<String>,
) {
    for &entry_index in &node.entries {
        if let Some(entry) = entries.get(entry_index) {
            if is_hlsl_include_tag(entry) {
                keys.push(entry.key.clone());
            }
        }
    }
    for child in &node.children {
        collect_hlsl_include_keys_into(child, entries, keys);
    }
}

pub(in crate::app) fn draw_entry_list(
    ui: &mut Ui,
    entry_indices: &[usize],
    entries: &[TagEntry],
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    reveal_key: Option<&str>,
    sort: BrowserSort,
    favorite_keys: Option<&HashSet<String>>,
) -> Option<BrowserAction> {
    let ordered = ordered_indices(entry_indices, entries, sort);
    let entry_indices: &[usize] = ordered.as_ref();
    let mut clicked = None;
    for &entry_index in entry_indices {
        let entry = &entries[entry_index];
        if !entry_matches(entry, filter) {
            continue;
        }
        if clicked.is_none() {
            clicked = draw_entry(
                ui,
                entry,
                selected,
                show_prefixes,
                double_click_to_open,
                reveal_key,
                favorite_keys,
            );
        } else {
            let _ = draw_entry(
                ui,
                entry,
                selected,
                show_prefixes,
                double_click_to_open,
                reveal_key,
                favorite_keys,
            );
        }
    }
    clicked
}

pub(in crate::app) fn draw_entry(
    ui: &mut Ui,
    entry: &TagEntry,
    selected: Option<&str>,
    show_prefixes: bool,
    double_click_to_open: bool,
    reveal_key: Option<&str>,
    favorite_keys: Option<&HashSet<String>>,
) -> Option<BrowserAction> {
    let leaf_label = entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path);
    let label = if show_prefixes {
        format!("[tag] {leaf_label}")
    } else {
        leaf_label.to_owned()
    };
    // The row is a drag source: drag it onto a tag-reference cell to set the
    // reference. Payload is our `DraggedTagRef` (what the ref-cell + shader-row
    // drop targets expect); the row paints a tag icon + a cursor drag-preview.
    let payload = DraggedTagRef {
        group_tag: entry.group_tag,
        input: entry_reference_input(entry),
        rel_path: entry_rel_path(entry),
    };
    let selected = selected == Some(entry.key.as_str());
    let row_size = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
    let (row_rect, response) = ui.allocate_exact_size(row_size, Sense::click_and_drag());
    let response = response.on_hover_text(&entry.display_path);
    response.dnd_set_drag_payload(payload);
    if reveal_key == Some(entry.key.as_str()) {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    if ui.is_rect_visible(row_rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() || response.highlighted() || response.has_focus() {
            ui.painter().rect(
                row_rect.expand(visuals.expansion),
                visuals.rounding,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
            );
        }
        let icon_size = 16.0;
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(row_rect.left() + icon_size * 0.5, row_rect.center().y),
            Vec2::splat(icon_size),
        );
        paint_tag_icon_at(ui, entry.group_tag, icon_rect);
        ui.painter().text(
            row_rect.left_center() + Vec2::new(icon_size + 5.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            text_dark(),
        );
    }
    if response.dragged()
        && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
    {
        egui::Area::new(ui.make_persistent_id(("tag_tree_drag_preview", &entry.key)))
            .order(egui::Order::Tooltip)
            .fixed_pos(pointer_pos + Vec2::new(12.0, 12.0))
            .show(ui.ctx(), |ui| {
                ui.label(RichText::new(leaf_label).color(text_dark()));
            });
    }
    let open_requested = if double_click_to_open {
        response.double_clicked()
    } else {
        response.clicked()
    };
    let mut action = open_requested.then(|| BrowserAction::Select(entry.key.clone()));
    response.context_menu(|ui| {
        style_tag_context_menu(ui);

        let rename_enabled = matches!(
            entry.location,
            TagEntryLocation::LooseFile(_) | TagEntryLocation::Container { .. }
        );
        let extract_enabled = supports_tag_extract_menu(entry.group_tag);
        ui.horizontal(|ui| {
            if context_menu_primary_button(ui, "Rename", rename_enabled).clicked() {
                action = Some(BrowserAction::RenameTag(entry.key.clone()));
                ui.close_menu();
            }
            if context_menu_primary_button(ui, "Move", rename_enabled).clicked() {
                action = Some(BrowserAction::MoveTag(entry.key.clone()));
                ui.close_menu();
            }
            ui.add_enabled_ui(extract_enabled, |ui| {
                ui.allocate_ui(Vec2::new(92.0, 44.0), |ui| {
                    ui.set_min_width(92.0);
                    let extract_menu = ui.menu_button("     Extract", |ui| {
                        ui.set_min_width(280.0);
                        if supports_tag_geometry_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract model geometry").clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(entry.key.clone()));
                            ui.close_menu();
                        }
                        if supports_animation_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract animations").clicked()
                        {
                            action = Some(BrowserAction::ExtractAnimation(entry.key.clone()));
                            ui.close_menu();
                        }
                        if supports_tag_import_info_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract import-info").clicked()
                        {
                            action = Some(BrowserAction::ExtractImportInfo(entry.key.clone()));
                            ui.close_menu();
                        }
                    });
                    let icon_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            extract_menu.response.rect.left() + 17.0,
                            extract_menu.response.rect.center().y,
                        ),
                        Vec2::splat(16.0),
                    );
                    paint_button_icon_at(ui, ButtonIcon::Export, icon_rect, text_dark());
                });
            });
        });

        context_menu_separator(ui);
        let has_extra_extract = is_monolithic_entry(entry)
            || is_bitmap_group(entry.group_tag)
            || is_material_shader_group(entry.group_tag)
            || is_hlsl_include_group(entry.group_tag);
        if has_extra_extract {
            ui.menu_button("More extraction tools", |ui| {
                ui.set_min_width(280.0);
                if is_monolithic_entry(entry)
                    && context_menu_button(ui, "Extract raw tag...").clicked()
                {
                    action = Some(BrowserAction::ExtractRaw(entry.key.clone()));
                    ui.close_menu();
                }
                if is_bitmap_group(entry.group_tag)
                    && context_menu_button(ui, "Extract bitmap images...").clicked()
                {
                    action = Some(BrowserAction::ExtractBitmap(entry.key.clone()));
                    ui.close_menu();
                }
                if is_material_shader_group(entry.group_tag)
                    && context_menu_button(ui, "Extract source shaders...").clicked()
                {
                    action = Some(BrowserAction::ExtractMaterialShaderSources(
                        entry.key.clone(),
                    ));
                    ui.close_menu();
                }
                if is_hlsl_include_group(entry.group_tag)
                    && context_menu_button(ui, "Extract HLSL include...").clicked()
                {
                    action = Some(BrowserAction::ExtractHlslIncludeSource(entry.key.clone()));
                    ui.close_menu();
                }
            });
        }
        if context_menu_button(ui, "Open with File Explorer").clicked() {
            action = Some(BrowserAction::OpenInExplorer(entry.key.clone()));
            ui.close_menu();
        }
        if let Some(favorite_keys) = favorite_keys {
            let label = if favorite_keys.contains(&entry.key) {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            };
            if context_menu_button(ui, label).clicked() {
                action = Some(BrowserAction::ToggleFavorite(entry.key.clone()));
                ui.close_menu();
            }
        }
        if context_menu_button(ui, "Copy Tag Path").clicked() {
            action = Some(BrowserAction::CopyTagName(entry.key.clone()));
            ui.close_menu();
        }
        if context_menu_button(ui, "Find Tag References...").clicked() {
            action = Some(BrowserAction::FindReferences(entry.key.clone()));
            ui.close_menu();
        }
        if context_menu_button(ui, "Explore references...").clicked() {
            action = Some(BrowserAction::ExploreReferences(entry.key.clone()));
            ui.close_menu();
        }

        context_menu_separator(ui);
        if context_menu_button(ui, "Dump Tag to JSON...").clicked() {
            action = Some(BrowserAction::DumpJson(entry.key.clone()));
            ui.close_menu();
        }
    });
    action
}

pub(in crate::app) fn draw_favorites(
    ui: &mut Ui,
    entries: &[TagEntry],
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    favorite_keys: &HashSet<String>,
) -> Option<BrowserAction> {
    if entries.is_empty() || !entries.iter().any(|entry| entry_matches(entry, filter)) {
        return None;
    }
    let mut action = None;
    egui::CollapsingHeader::new(
        RichText::new("★ Favorites").color(Color32::from_rgb(242, 196, 48)),
    )
    .default_open(true)
    .show(ui, |ui| {
        for entry in entries {
            if !entry_matches(entry, filter) {
                continue;
            }
            let row_action = draw_entry(
                ui,
                entry,
                selected,
                show_prefixes,
                double_click_to_open,
                None,
                Some(favorite_keys),
            );
            if action.is_none() {
                action = row_action;
            }
        }
    });
    action
}

fn paint_tag_icon_at(ui: &Ui, group_tag: u32, rect: egui::Rect) {
    let group = format_group_tag(group_tag);
    let uri = tag_icon_uri(ui.ctx(), &group);
    egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

pub(in crate::app) fn is_monolithic_entry(entry: &TagEntry) -> bool {
    matches!(entry.location, TagEntryLocation::Monolithic { .. })
}

pub(in crate::app) fn folder_arrow_icon(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let open = openness > 0.5;
    let (icon, color) = if open {
        (ButtonIcon::FolderOpen, disclosure_triangle_green())
    } else {
        (ButtonIcon::FolderClosed, disclosure_triangle_blue())
    };
    let rect = egui::Rect::from_center_size(response.rect.center(), Vec2::splat(16.0));
    paint_button_icon_at(ui, icon, rect, color);
}

pub(in crate::app) fn disclosure_triangle_icon(
    ui: &mut Ui,
    open: bool,
    center: egui::Pos2,
    color: Color32,
) {
    let size = 7.0;
    let points = if open {
        vec![
            egui::pos2(center.x - size, center.y - size * 0.4),
            egui::pos2(center.x + size, center.y - size * 0.4),
            egui::pos2(center.x, center.y + size * 0.7),
        ]
    } else {
        vec![
            egui::pos2(center.x - size * 0.4, center.y - size),
            egui::pos2(center.x - size * 0.4, center.y + size),
            egui::pos2(center.x + size * 0.7, center.y),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

pub(in crate::app) fn tag_tab_label(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path)
        .to_owned()
}

pub(in crate::app) fn tag_file_name(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("tag")
        .to_owned()
}

pub(in crate::app) fn tag_file_stem(entry: &TagEntry) -> String {
    Path::new(&tag_file_name(entry))
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tag")
        .to_owned()
}

pub(in crate::app) fn tag_display_parent(entry: &TagEntry) -> PathBuf {
    Path::new(&entry.display_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

pub(in crate::app) fn tag_json_relative_path(entry: &TagEntry) -> PathBuf {
    let mut path = PathBuf::from(&entry.display_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tag");
    path.set_file_name(format!("{file_name}.json"));
    path
}

pub(in crate::app) fn is_bitmap_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"bitm")
}

pub(in crate::app) fn is_bitmap_tag(entry: &TagEntry) -> bool {
    is_bitmap_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("bitmap")
        || entry.display_path.to_ascii_lowercase().ends_with(".bitmap")
}

pub(in crate::app) fn is_material_shader_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"mats")
}

pub(in crate::app) fn is_material_shader_browser_tag(entry: &TagEntry) -> bool {
    is_material_shader_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("material_shader")
        || entry
            .display_path
            .to_ascii_lowercase()
            .ends_with(".material_shader")
}

pub(in crate::app) fn is_hlsl_include_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"hlsl")
}

pub(in crate::app) fn is_hlsl_include_tag(entry: &TagEntry) -> bool {
    is_hlsl_include_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("hlsl_include")
        || entry
            .display_path
            .to_ascii_lowercase()
            .ends_with(".hlsl_include")
}

pub(in crate::app) fn supports_animation_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"jmad" | b"hlmt" | b"antr" | b"mode"
    )
}

pub(in crate::app) fn supports_tag_extract_menu(group_tag: u32) -> bool {
    supports_tag_geometry_extraction(group_tag)
        || supports_animation_extraction(group_tag)
        || supports_tag_import_info_extraction(group_tag)
}

fn supports_tag_geometry_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"hlmt" | b"mode" | b"phmo" | b"coll" | b"mod2"
    )
}

fn supports_tag_import_info_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"hlmt" | b"mode" | b"phmo" | b"coll" | b"mod2"
    )
}
