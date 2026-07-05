use super::*;

/// A pending "reveal in tree" request threaded through the tree draw: it force-
/// opens the folder nodes along `remaining` (ancestor labels not yet descended)
/// and scrolls the matching leaf (`key`) into view. One-shot — cleared by the
/// caller after the frame.
#[derive(Clone, Copy)]
pub(super) struct Reveal<'a> {
    pub(super) key: &'a str,
    pub(super) remaining: &'a [String],
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
fn entry_reference_input(entry: &TagEntry) -> String {
    let display = &entry.display_path;
    let without_ext = match display.rfind('.') {
        Some(dot) => &display[..dot],
        None => display.as_str(),
    };
    format!(
        "{}:{}",
        format_group_tag(entry.group_tag),
        without_ext.replace('/', "\\")
    )
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
pub(super) fn ancestor_labels(display_path: &str) -> Vec<String> {
    let mut segments: Vec<String> = display_path
        .replace('\\', "/")
        .split('/')
        .map(str::to_owned)
        .collect();
    segments.pop(); // drop the filename
    segments
}

pub(super) fn draw_tree(
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
            )
        });
    }
    clicked
}

pub(super) fn draw_tree_lazy(
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
            )
        });
    }
    clicked
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_tree_node_lazy(
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

pub(super) fn draw_tree_node(
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
) -> Option<BrowserAction> {
    if !filter.is_empty() && !node_matches(node, entries, filter) {
        return None;
    }
    let on_path = reveal.is_some_and(|reveal| reveal.matches_node(&node.label));
    let inner_reveal = on_path.then(|| reveal.expect("on_path implies reveal").descend());
    let mut clicked = None;
    let folder_label = if groups_mode {
        group_folder_label(&node.label, show_prefixes)
    } else if show_prefixes {
        format!("[folder] {}", node.label)
    } else {
        node.label.clone()
    };
    let response = egui::CollapsingHeader::new(RichText::new(folder_label).color(text_dark()))
        .icon(folder_arrow_icon)
        .default_open(!filter.is_empty())
        .open(on_path.then_some(true))
        .show(ui, |ui| {
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
                    );
                }
            }
        });
    response.header_response.context_menu(|ui| {
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

pub(super) fn collect_tag_keys(node: &TagTreeNode, entries: &[TagEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    collect_tag_keys_into(node, entries, &mut keys);
    keys
}

pub(super) fn collect_tag_keys_into(
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

pub(super) fn collect_bitmap_keys(node: &TagTreeNode, entries: &[TagEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    collect_bitmap_keys_into(node, entries, &mut keys);
    keys
}

pub(super) fn collect_bitmap_keys_into(
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

pub(super) fn collect_hlsl_include_keys(node: &TagTreeNode, entries: &[TagEntry]) -> Vec<String> {
    let mut keys = Vec::new();
    collect_hlsl_include_keys_into(node, entries, &mut keys);
    keys
}

pub(super) fn collect_material_shader_keys(
    node: &TagTreeNode,
    entries: &[TagEntry],
) -> Vec<String> {
    let mut keys = Vec::new();
    collect_material_shader_keys_into(node, entries, &mut keys);
    keys
}

pub(super) fn collect_material_shader_keys_into(
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

pub(super) fn collect_hlsl_include_keys_into(
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

pub(super) fn draw_entry_list(
    ui: &mut Ui,
    entry_indices: &[usize],
    entries: &[TagEntry],
    selected: Option<&str>,
    filter: &str,
    show_prefixes: bool,
    double_click_to_open: bool,
    reveal_key: Option<&str>,
    sort: BrowserSort,
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
            );
        } else {
            let _ = draw_entry(
                ui,
                entry,
                selected,
                show_prefixes,
                double_click_to_open,
                reveal_key,
            );
        }
    }
    clicked
}

pub(super) fn draw_entry(
    ui: &mut Ui,
    entry: &TagEntry,
    selected: Option<&str>,
    show_prefixes: bool,
    double_click_to_open: bool,
    reveal_key: Option<&str>,
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
        label: label.clone(),
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
        if ui.button("Copy tag name").clicked() {
            action = Some(BrowserAction::CopyTagName(entry.key.clone()));
            ui.close_menu();
        }
        if ui.button("Open with File Explorer").clicked() {
            action = Some(BrowserAction::OpenInExplorer(entry.key.clone()));
            ui.close_menu();
        }
        if ui.button("Find references").clicked() {
            action = Some(BrowserAction::FindReferences(entry.key.clone()));
            ui.close_menu();
        }
        if ui.button("Explore references...").clicked() {
            action = Some(BrowserAction::ExploreReferences(entry.key.clone()));
            ui.close_menu();
        }
        if ui.button("Rename / Move (fix references)...").clicked() {
            action = Some(BrowserAction::RenameTag(entry.key.clone()));
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Dump tag to JSON...").clicked() {
            action = Some(BrowserAction::DumpJson(entry.key.clone()));
            ui.close_menu();
        }
        if is_monolithic_entry(entry) && ui.button("Extract raw tag...").clicked() {
            action = Some(BrowserAction::ExtractRaw(entry.key.clone()));
            ui.close_menu();
        }
        if is_bitmap_group(entry.group_tag) && ui.button("Extract bitmap images...").clicked() {
            action = Some(BrowserAction::ExtractBitmap(entry.key.clone()));
            ui.close_menu();
        }
        if supports_geometry_extraction(entry.group_tag)
            && ui.button(geometry_extract_label(entry.group_tag)).clicked()
        {
            action = Some(BrowserAction::ExtractGeometry(entry.key.clone()));
            ui.close_menu();
        }
        if supports_import_info_extraction(entry.group_tag)
            && ui.button("Extract import info...").clicked()
        {
            action = Some(BrowserAction::ExtractImportInfo(entry.key.clone()));
            ui.close_menu();
        }
        if supports_animation_extraction(entry.group_tag)
            && ui.button("Extract animations...").clicked()
        {
            action = Some(BrowserAction::ExtractAnimation(entry.key.clone()));
            ui.close_menu();
        }
        if is_material_shader_group(entry.group_tag)
            && ui.button("Extract source shaders...").clicked()
        {
            action = Some(BrowserAction::ExtractMaterialShaderSources(
                entry.key.clone(),
            ));
            ui.close_menu();
        }
        if is_hlsl_include_group(entry.group_tag) && ui.button("Extract HLSL include...").clicked()
        {
            action = Some(BrowserAction::ExtractHlslIncludeSource(entry.key.clone()));
            ui.close_menu();
        }
    });
    action
}

fn paint_tag_icon_at(ui: &Ui, group_tag: u32, rect: egui::Rect) {
    let group = format_group_tag(group_tag);
    let uri = format!("bytes://baboon_tag_icons/{group}.svg");
    egui::Image::from_bytes(uri, get_icon_svg(&group).as_bytes())
        .fit_to_exact_size(rect.size())
        .paint_at(ui, rect);
}

pub(super) fn is_monolithic_entry(entry: &TagEntry) -> bool {
    matches!(entry.location, TagEntryLocation::Monolithic { .. })
}

pub(super) fn folder_arrow_icon(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let color = if openness > 0.5 {
        disclosure_triangle_green()
    } else {
        disclosure_triangle_blue()
    };
    disclosure_triangle_icon(ui, openness > 0.5, response.rect.center(), color);
}

pub(super) fn disclosure_triangle_icon(
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

pub(super) fn tag_tab_label(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path)
        .to_owned()
}

pub(super) fn tag_file_name(entry: &TagEntry) -> String {
    entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("tag")
        .to_owned()
}

pub(super) fn tag_file_stem(entry: &TagEntry) -> String {
    Path::new(&tag_file_name(entry))
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tag")
        .to_owned()
}

pub(super) fn tag_display_parent(entry: &TagEntry) -> PathBuf {
    Path::new(&entry.display_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

pub(super) fn tag_json_relative_path(entry: &TagEntry) -> PathBuf {
    let mut path = PathBuf::from(&entry.display_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tag");
    path.set_file_name(format!("{file_name}.json"));
    path
}

pub(super) fn is_bitmap_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"bitm")
}

pub(super) fn is_bitmap_tag(entry: &TagEntry) -> bool {
    is_bitmap_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("bitmap")
        || entry.display_path.to_ascii_lowercase().ends_with(".bitmap")
}

pub(super) fn is_material_shader_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"mats")
}

pub(super) fn is_material_shader_browser_tag(entry: &TagEntry) -> bool {
    is_material_shader_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("material_shader")
        || entry
            .display_path
            .to_ascii_lowercase()
            .ends_with(".material_shader")
}

pub(super) fn is_hlsl_include_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"hlsl")
}

pub(super) fn is_hlsl_include_tag(entry: &TagEntry) -> bool {
    is_hlsl_include_group(entry.group_tag)
        || entry.group_name.as_deref() == Some("hlsl_include")
        || entry
            .display_path
            .to_ascii_lowercase()
            .ends_with(".hlsl_include")
}

pub(super) fn supports_geometry_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"hlmt" | b"scnr" | b"sbsp" | b"mode" | b"mod2" | b"coll" | b"phmo"
    )
}

pub(super) fn supports_import_info_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"mode" | b"coll" | b"phmo" | b"sbsp"
    )
}

pub(super) fn geometry_extract_label(group_tag: u32) -> &'static str {
    match &group_tag.to_be_bytes() {
        b"hlmt" => "Extract model geometry...",
        b"scnr" => "Extract scenario BSP geometry...",
        b"sbsp" => "Extract BSP geometry...",
        b"mode" => "Extract render_model geometry...",
        b"mod2" => "Extract gbxmodel geometry...",
        b"coll" => "Extract collision_model geometry...",
        b"phmo" => "Extract physics_model geometry...",
        _ => "Extract geometry...",
    }
}

pub(super) fn supports_animation_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"jmad" | b"hlmt" | b"antr"
    )
}

pub(super) fn node_matches(node: &TagTreeNode, entries: &[TagEntry], filter: &str) -> bool {
    node.entries
        .iter()
        .any(|&index| entry_matches(&entries[index], filter))
        || node
            .children
            .iter()
            .any(|child| node_matches(child, entries, filter))
}

pub(super) fn lazy_node_matches(node: &TagTreeNode, entries: &[TagEntry], filter: &str) -> bool {
    // Only show a folder node if it contains files whose NAME matches —
    // don't keep a folder open just because its own path contains the term.
    node.entries
        .iter()
        .any(|&index| entry_matches(&entries[index], filter))
        || node
            .children
            .iter()
            .any(|child| lazy_node_matches(child, entries, filter))
}

pub(super) fn entry_matches(entry: &TagEntry, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    entry_matches_lower(entry, &filter.to_ascii_lowercase())
}

/// Like [`entry_matches`] but takes an already-lowercased filter, so callers
/// that test many entries against one query don't re-lowercase it each time.
///
/// Query syntax (all case-insensitive):
/// - whitespace = AND (`elite arm` → both terms must match),
/// - `|` = OR (`elite | rifle`),
/// - `^foo` anchors to the start of the filename, `foo$` to the end,
///   `^foo$` is an exact filename match.
///
/// A plain (un-anchored) term matches the filename, the group four-CC, or the
/// group name; anchored terms match the filename only.
fn entry_matches_lower(entry: &TagEntry, filter_lower: &str) -> bool {
    // Match only the filename (last path segment), not parent folder names.
    // A tag at "floodcombat_elite/garbage/hg_arm/hg_arm.model" should NOT
    // appear when searching "elite" — only "elite.model" etc. should match.
    let filename = entry
        .display_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&entry.display_path)
        .to_ascii_lowercase();
    let fourcc = format_group_tag(entry.group_tag).to_ascii_lowercase();
    let group = entry
        .group_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut had_term = false;
    for or_group in filter_lower.split('|') {
        let mut group_ok = true;
        let mut group_had_term = false;
        for term in or_group.split_whitespace() {
            group_had_term = true;
            had_term = true;
            if !filter_term_matches(term, &filename, &fourcc, &group) {
                group_ok = false;
                break;
            }
        }
        if group_had_term && group_ok {
            return true;
        }
    }
    // A filter with no real terms (e.g. just "|" or whitespace) matches all.
    !had_term
}

fn filter_term_matches(term: &str, filename: &str, fourcc: &str, group: &str) -> bool {
    let anchored_start = term.starts_with('^');
    let anchored_end = term.ends_with('$') && term.len() > 1;
    let inner = term.trim_start_matches('^');
    let inner = if anchored_end {
        &inner[..inner.len().saturating_sub(1)]
    } else {
        inner
    };
    if inner.is_empty() {
        return true; // a lone anchor matches anything
    }
    match (anchored_start, anchored_end) {
        (true, true) => filename == inner,
        (true, false) => filename.starts_with(inner),
        (false, true) => filename.ends_with(inner),
        (false, false) => {
            filename.contains(inner) || fourcc.contains(inner) || group.contains(inner)
        }
    }
}

/// A human-readable warning for a degenerate browser filter, or `None` when it's
/// well-formed. The boolean grammar (space = AND, `|` = OR, `^`/`$` anchors) has
/// no hard syntax errors, so we flag the cases that silently misbehave: an empty
/// operand around `|`, and a term that is only an anchor.
pub(super) fn browser_filter_warning(filter: &str) -> Option<String> {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        return None;
    }
    let operands: Vec<&str> = trimmed.split('|').collect();
    if operands.len() > 1 && operands.iter().any(|operand| operand.trim().is_empty()) {
        return Some("empty term around '|' — that side matches nothing".to_owned());
    }
    for operand in &operands {
        for term in operand.split_whitespace() {
            let inner = term.trim_start_matches('^');
            let inner = inner.strip_suffix('$').unwrap_or(inner);
            if inner.is_empty() {
                return Some(format!("'{term}' is only an anchor — matches everything"));
            }
        }
    }
    None
}

/// Collect the indices of all entries matching `filter`, in display order.
/// Called only when the cached query changes (see [`FilterCache`]), not per
/// frame, so the O(N) lowercase scan happens at most once per keystroke.
pub(super) fn compute_filter_matches(entries: &[TagEntry], filter: &str) -> Vec<usize> {
    let filter_lower = filter.to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry_matches_lower(entry, &filter_lower))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{TagEntry, TagEntryLocation};
    use std::path::PathBuf;

    fn entry(display_path: &str, group: &[u8; 4]) -> TagEntry {
        TagEntry {
            key: display_path.to_owned(),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*group),
            group_name: None,
            location: TagEntryLocation::LooseFile(PathBuf::from(display_path)),
        }
    }

    #[test]
    fn matches_filename_not_parent_folders() {
        let entries = vec![
            entry("floodcombat_elite/garbage/hg_arm/hg_arm.model", b"mode"),
            entry("characters/elite/elite.model", b"mode"),
        ];
        // "elite" should match only the tag whose *filename* contains it.
        let matches = compute_filter_matches(&entries, "elite");
        assert_eq!(matches, vec![1]);
    }

    #[test]
    fn matches_group_tag_and_is_case_insensitive() {
        let entries = vec![
            entry("fx/spark.effect", b"effe"),
            entry("weapons/rifle.weapon", b"weap"),
        ];
        // Group four-CC match, regardless of query case.
        assert_eq!(compute_filter_matches(&entries, "WEAP"), vec![1]);
    }

    #[test]
    fn reference_input_uses_fourcc_and_backslash_path_without_extension() {
        let e = entry("objects/weapons/rifle/rifle.weapon", b"weap");
        // "weap" four-CC, backslash path, extension stripped.
        assert_eq!(
            entry_reference_input(&e),
            "weap:objects\\weapons\\rifle\\rifle"
        );
    }

    #[test]
    fn malformed_filter_warnings() {
        // Well-formed filters: no warning.
        assert!(browser_filter_warning("").is_none());
        assert!(browser_filter_warning("elite").is_none());
        assert!(browser_filter_warning("arm | rifle").is_none());
        assert!(browser_filter_warning("^elite.model$").is_none());
        assert!(browser_filter_warning("weapon$").is_none());
        // Empty OR operand.
        assert!(browser_filter_warning("foo |").is_some());
        assert!(browser_filter_warning("a || b").is_some());
        // Anchor-only term.
        assert!(browser_filter_warning("^").is_some());
        assert!(browser_filter_warning("foo ^").is_some());
    }

    #[test]
    fn boolean_and_or_and_anchors() {
        let entries = vec![
            entry("characters/elite/elite_arm.model", b"mode"),
            entry("characters/elite/elite.model", b"mode"),
            entry("weapons/rifle.weapon", b"weap"),
        ];
        // AND: both terms must match the same entry.
        assert_eq!(compute_filter_matches(&entries, "elite arm"), vec![0]);
        // OR: either side matches.
        assert_eq!(compute_filter_matches(&entries, "arm | rifle"), vec![0, 2]);
        // Prefix anchor on filename.
        assert_eq!(compute_filter_matches(&entries, "^elite_"), vec![0]);
        // Suffix anchor on filename.
        assert_eq!(compute_filter_matches(&entries, "weapon$"), vec![2]);
        // Exact filename anchor.
        assert_eq!(compute_filter_matches(&entries, "^elite.model$"), vec![1]);
    }

    #[test]
    fn folder_hlsl_include_collector_finds_nested_include_entries() {
        let entries = vec![
            entry("rasterizer/hlsl/ssao.hlsl_include", b"hlsl"),
            entry("rasterizer/hlsl/post/tonemap.hlsl_include", b"hlsl"),
            entry("rasterizer/bitmaps/noise.bitmap", b"bitm"),
        ];
        let tree = crate::source::build_tree(&entries);
        let rasterizer = tree
            .children
            .iter()
            .find(|node| node.label == "rasterizer")
            .expect("rasterizer folder");

        assert_eq!(
            collect_hlsl_include_keys(rasterizer, &entries),
            vec![
                "rasterizer/hlsl/ssao.hlsl_include".to_owned(),
                "rasterizer/hlsl/post/tonemap.hlsl_include".to_owned(),
            ]
        );
    }

    #[test]
    fn folder_material_shader_collector_finds_nested_material_shader_entries() {
        let entries = vec![
            entry(
                "shaders/material_shaders/decals/base.material_shader",
                b"mats",
            ),
            entry(
                "shaders/material_shaders/decals/palette/palette.material_shader",
                b"mats",
            ),
            entry("shaders/material_shaders/decals/noise.bitmap", b"bitm"),
        ];
        let tree = crate::source::build_tree(&entries);
        let shaders = tree
            .children
            .iter()
            .find(|node| node.label == "shaders")
            .expect("shaders folder");

        assert_eq!(
            collect_material_shader_keys(shaders, &entries),
            vec![
                "shaders/material_shaders/decals/base.material_shader".to_owned(),
                "shaders/material_shaders/decals/palette/palette.material_shader".to_owned(),
            ]
        );
    }
}
