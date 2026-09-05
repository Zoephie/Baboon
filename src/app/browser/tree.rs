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
pub(in crate::app) fn entry_rel_path(entry: &TagEntry) -> String {
    let display = &entry.display_path;
    let without_ext = match display.rfind('.') {
        Some(dot) => &display[..dot],
        None => display.as_str(),
    };
    without_ext.replace('\\', "/")
}

/// The file an entry is on disk, for a drag that may leave Baboon and land
/// on Sapien. Cache and container tags have none.
pub(in crate::app) fn entry_loose_file(entry: &TagEntry) -> Option<PathBuf> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => Some(path.clone()),
        _ => None,
    }
}

/// Hover text for a drag source, which egui's own tooltips cannot be.
///
/// An egui 0.29 tooltip is an `Area`, and once one has shown, the next
/// pointer press is hit-tested against the frame that contained it — the
/// press lands on the tooltip instead of the widget, so a drag source under
/// it never starts its drag (`interactable(false)` does not save it: the
/// area still enters the hit-test). "Aim at the row, then drag" is exactly
/// how the browser rows and library cards are used, so they show this
/// instead: pure painting on a Tooltip-order layer. A painter registers no
/// widgets and no area, so nothing exists for a press to land on.
pub(in crate::app) fn hover_tooltip_beside_pointer(
    ui: &Ui,
    response: &egui::Response,
    text: &str,
) {
    if !response.hovered() || response.dragged() {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        response.id.with("hover_tooltip"),
    ));
    let galley = painter.layout(
        text.to_owned(),
        FontId::proportional(12.5),
        text_dark(),
        360.0,
    );
    let padding = Vec2::new(7.0, 5.0);
    let mut rect = egui::Rect::from_min_size(
        pointer + Vec2::new(14.0, 18.0),
        galley.size() + padding * 2.0,
    );
    // Keep it on screen; never under the pointer, so even egui's
    // closest-widget search cannot be confused by it.
    let screen = ui.ctx().screen_rect();
    if rect.right() > screen.right() {
        rect = rect.translate(Vec2::new(screen.right() - rect.right(), 0.0));
    }
    if rect.bottom() > screen.bottom() {
        rect = rect.translate(Vec2::new(0.0, -rect.height() - 24.0));
    }
    let visuals = ui.visuals();
    painter.rect(
        rect,
        4.0,
        visuals.window_fill,
        visuals.window_stroke,
    );
    painter.galley(rect.min + padding, galley, text_dark());
}

pub(in crate::app) fn context_menu_button(ui: &mut Ui, label: &str) -> egui::Response {
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
        "Duplicate" => Some(ButtonIcon::Duplicate),
        "Delete" => Some(ButtonIcon::Garbage),
        "Move" => Some(ButtonIcon::Move),
        "Open with File Explorer" => Some(ButtonIcon::FileExplorer),
        "Add to Favorites" | "Remove from Favorites" => Some(ButtonIcon::Favourite),
        "Copy Tag Path" => Some(ButtonIcon::CopyPath),
        "Find Tag References..." => Some(ButtonIcon::Find),
        "Dump Tag to JSON..." => Some(ButtonIcon::Json),
        "Dump Tag References..." => Some(ButtonIcon::Doc),
        "Open in Sapien" | "Open in tag_test" => Some(ButtonIcon::Open),
        _ => None,
    }
}

pub(in crate::app) fn context_menu_separator(ui: &mut Ui) {
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(3.0);
}

pub(in crate::app) fn style_tag_context_menu(ui: &mut Ui) {
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
        // `sort_by_cached_key`, not `sort_by`: the key is a freshly lowercased
        // String, and computing it inside the comparator meant two allocations
        // per comparison -- ~9,000 of them per frame for a 600-entry folder,
        // repeated for every expanded folder. Cached, it is one per entry.
        BrowserSort::Name => {
            let mut sorted = indices.to_vec();
            sorted.sort_by_cached_key(|&index| entry_filename_lower(&entries[index]));
            Cow::Owned(sorted)
        }
        BrowserSort::Type => {
            let mut sorted = indices.to_vec();
            sorted.sort_by_cached_key(|&index| {
                (
                    format_group_tag(entries[index].group_tag),
                    entry_filename_lower(&entries[index]),
                )
            });
            Cow::Owned(sorted)
        }
    }
}

fn ordered_child_indices(children: &[TagTreeNode], sort: BrowserSort) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..children.len()).collect();
    if !matches!(sort, BrowserSort::Natural) {
        indices.sort_by_cached_key(|&index| children[index].label.to_ascii_lowercase());
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
    is_container: bool,
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
                is_container,
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
    // Last, so it covers whatever the tree left over. Groups mode is excluded
    // for the same reason the folder menu is: a group node's path is a label,
    // not a folder, and there is no root to author into.
    if is_container && !groups_mode {
        clicked = clicked.or_else(|| draw_container_root_target(ui));
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
    let response = egui::CollapsingHeader::new(
        RichText::new(folder_label).color(folder_label_color(ui, node, entries)),
    )
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
        // Not Expert-gated, unlike the "save this folder for another game"
        // action it replaces: bringing another game's tags in is the headline
        // feature now, and the write is confirmed by the dialog either way.
        if ui
            .button("Import tags here...")
            .on_hover_text("Convert a tag, or a whole folder of them, from another game into this folder")
            .clicked()
        {
            clicked = Some(BrowserAction::ImportTagsIntoLooseFolder {
                rel_path: node.rel_path.clone(),
            });
            ui.close_menu();
        }
        context_menu_separator(ui);
        // The same action the tag menu offers, aimed at the folder rather than a
        // file in it. A browser folder maps straight onto a directory under the
        // tags root, so there is nothing to resolve beyond joining the two.
        if context_menu_button(ui, "Open with File Explorer").clicked() {
            clicked = Some(BrowserAction::OpenLooseFolderInExplorer {
                rel_path: node.rel_path.clone(),
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
    is_container: bool,
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
                    is_container,
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
            folder_label_color(ui, node, entries),
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
        egui::CollapsingHeader::new(
            RichText::new(folder_label).color(folder_label_color(ui, node, entries)),
        )
            .icon(folder_arrow_icon)
            .default_open(!filter.is_empty())
            .open(on_path.then_some(true))
            .show(ui, body)
            .header_response
    };
    header_response.context_menu(|ui| {
        // Campaign Evolved folder authoring (Folders mode only — in Groups mode
        // the node path is a group label, not a folder).
        if is_container && !groups_mode {
            let folder_rel = node.rel_path.to_string_lossy().replace('\\', "/");
            let folder_rel = (!folder_rel.is_empty()).then_some(folder_rel);
            if let Some(action) = container_authoring_menu_items(ui, folder_rel.clone()) {
                clicked = Some(action);
            }
            // Rename and Delete are offered only for a folder the user made and
            // nothing has landed in. Once a tag is inside, the folder is
            // expressed by the container's own directory index, and moving it
            // means rewriting every tag beneath it — a container write, not a
            // workspace edit, and not what this menu does.
            if let Some(rel) = folder_rel.filter(|_| folder_is_pending_and_empty(node)) {
                if ui.button("Rename folder...").clicked() {
                    clicked = Some(BrowserAction::RenameContainerFolder { rel: rel.clone() });
                    ui.close_menu();
                }
                if ui.button("Delete folder").clicked() {
                    clicked = Some(BrowserAction::DeleteContainerFolder { rel });
                    ui.close_menu();
                }
            }
            context_menu_separator(ui);
        }

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

        // Folders mode only. In Groups mode this node is a group label rather
        // than a folder, and the extraction lays tags out by their own paths —
        // so a group node would write a tree that has nothing to do with the
        // node that was clicked.
        if is_container && !groups_mode {
            let container_keys = collect_container_tag_keys(node, entries);
            if container_keys.is_empty() {
                ui.label(RichText::new("No shipped tags in this folder").color(subtle_dark()));
            } else if ui
                .button(format!("Extract tags to folder... ({})", container_keys.len()))
                .on_hover_text(
                    "Write every tag this folder ships to a folder on disk, laid out like an \
                     editing kit",
                )
                .clicked()
            {
                clicked = Some(BrowserAction::ExtractContainerFolderTags {
                    label: folder_display_path(node),
                    keys: container_keys,
                });
                ui.close_menu();
            }
        }

        // Monolithic caches only. The tags in one are big-endian and read-only,
        // so the way out of a cache is a conversion into a kit rather than a copy
        // — and the destination is the kit's tags root, because a cache tag has
        // to land at its own path for every reference to it to resolve.
        if !groups_mode {
            let cache_tags = count_cache_tags(node, entries);
            if cache_tags > 0
                && ui
                    .button(format!("Import into editing kit... ({cache_tags})"))
                    .on_hover_text(
                        "Convert this folder to little-endian tags in an open editing kit, \
                         at the same paths, pulling in whatever they reference",
                    )
                    .clicked()
            {
                clicked = Some(BrowserAction::ImportCacheFolderIntoKit {
                    prefix: folder_display_path(node),
                });
                ui.close_menu();
            }
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

/// Colour for a folder row: marked when anything beneath it carries edits that
/// are not written into the game, so the workspace can be scanned top-down for
/// where the modifications actually are.
fn folder_label_color(ui: &Ui, node: &TagTreeNode, entries: &[TagEntry]) -> Color32 {
    match browser_modified_tags(ui) {
        Some(modified) if modified.subtree_has_modified(node, entries) => modified_text(),
        // A folder the user made that nothing has landed in yet is not in any
        // pak, so it is drawn as the intention it is rather than as content.
        _ if folder_is_pending_and_empty(node) => subtle_dark(),
        _ => text_dark(),
    }
}

/// The Campaign Evolved authoring items, shared by a folder's context menu and
/// by the browser's empty space.
///
/// `folder_rel` is `None` at the container root. Shared so the two cannot offer
/// different sets: the root gained these only after the fact, and a second copy
/// is how it would quietly fall behind again.
fn container_authoring_menu_items(
    ui: &mut Ui,
    folder_rel: Option<String>,
) -> Option<BrowserAction> {
    let mut clicked = None;
    if ui.button("New tag here...").clicked() {
        clicked = Some(BrowserAction::NewTagInFolder {
            folder_rel: folder_rel.clone(),
        });
        ui.close_menu();
    }
    if ui.button("Import tag here...").clicked() {
        clicked = Some(BrowserAction::ImportTagInFolder {
            folder_rel: folder_rel.clone(),
        });
        ui.close_menu();
    }
    if ui.button("New folder here...").clicked() {
        clicked = Some(BrowserAction::NewContainerFolder {
            parent_rel: folder_rel,
        });
        ui.close_menu();
    }
    clicked
}

/// Claim the browser's empty space below the tree so the container *root* can be
/// right-clicked.
///
/// Without this the root is unreachable. Every authoring menu hangs off a tree
/// node, and a real node's `rel_path` is never empty — so `folder_rel: None`,
/// which is the whole representation of "the container root", had no way to be
/// produced by any gesture.
fn draw_container_root_target(ui: &mut Ui) -> Option<BrowserAction> {
    // Fills the empty area when the tree is short — the usual case, since
    // folders start collapsed — and stays a right-clickable strip under the last
    // row when the tree is long enough to have scrolled past the viewport.
    let size = Vec2::new(
        ui.available_width(),
        ui.available_size_before_wrap().y.max(24.0),
    );
    let (_, response) = ui.allocate_exact_size(size, Sense::click());
    let mut clicked = None;
    response.context_menu(|ui| {
        // Right-clicking blank space is ambiguous about what it acts on, so the
        // menu says.
        ui.label(RichText::new("Container root").color(subtle_dark()).small());
        context_menu_separator(ui);
        clicked = container_authoring_menu_items(ui, None);
    });
    clicked
}

/// Whether this node exists only because the user asked for it and still holds
/// nothing — the one kind of folder the browser offers to rename or delete.
///
/// Emptiness is read from the tree rather than stored, so a folder that gains a
/// tag stops being editable this way on the very next rebuild without anything
/// having to remember to clear a flag.
pub(in crate::app) fn folder_is_pending_and_empty(node: &TagTreeNode) -> bool {
    node.pending && node.entries.is_empty() && node.children.is_empty()
}

fn group_tree_label_parts(label: &str) -> (&str, &str) {
    label
        .rsplit_once(' ')
        .map_or(("", label), |(name, fourcc)| (name, fourcc))
}

fn show_group_tree_header<R>(
    ui: &mut Ui,
    label: &str,
    label_color: Color32,
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
            content = content.union(ui.label(RichText::new(display_name).color(label_color)));
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

/// A folder node's path for display, in the forward-slash form the rest of the
/// container UI uses. Falls back to the node's own label for a node with no
/// relative path, so the root still names itself in a confirmation.
fn folder_display_path(node: &TagTreeNode) -> String {
    let rel = node.rel_path.to_string_lossy().replace('\\', "/");
    if rel.is_empty() { node.label.clone() } else { rel }
}

/// Keys beneath `node` for tags the container actually ships.
///
/// Deliberately narrower than [`collect_tag_keys`]: the extraction reads each
/// tag's shipped payload back out of the mounted `.ucas`, so an entry authored
/// in this session has nothing to read and is silently skipped by the worker.
/// Counting those would put a number in the menu that the run then fails to
/// deliver, so they are excluded here and the label reports what will be written.
pub(in crate::app) fn collect_container_tag_keys(
    node: &TagTreeNode,
    entries: &[TagEntry],
) -> Vec<String> {
    let mut keys = Vec::new();
    collect_container_tag_keys_into(node, entries, &mut keys);
    keys
}

fn collect_container_tag_keys_into(
    node: &TagTreeNode,
    entries: &[TagEntry],
    keys: &mut Vec<String>,
) {
    for &entry_index in &node.entries {
        if let Some(entry) = entries.get(entry_index) {
            if matches!(entry.location, TagEntryLocation::Container { .. }) {
                keys.push(entry.key.clone());
            }
        }
    }
    for child in &node.children {
        collect_container_tag_keys_into(child, entries, keys);
    }
}

/// How many tags beneath `node` come out of a monolithic cache.
///
/// Only the count is wanted, not the keys: the import filters the cache by name
/// prefix on the worker thread, so carrying a list of thousands of keys through
/// a context menu would be work done twice. The number is what the menu says.
pub(in crate::app) fn count_cache_tags(node: &TagTreeNode, entries: &[TagEntry]) -> usize {
    let mut count = node
        .entries
        .iter()
        .filter_map(|&index| entries.get(index))
        .filter(|entry| matches!(entry.location, TagEntryLocation::Monolithic { .. }))
        .count();
    for child in &node.children {
        count += count_cache_tags(child, entries);
    }
    count
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
        file_path: entry_loose_file(entry),
    };
    let selected = selected == Some(entry.key.as_str());
    let row_size = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
    let (row_rect, response) = ui.allocate_exact_size(row_size, Sense::click_and_drag());
    // Not `on_hover_text`: an egui tooltip would block the very drag this row
    // exists to start. See `hover_tooltip_beside_pointer`.
    hover_tooltip_beside_pointer(ui, &response, &entry.display_path);
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
            // Marked when the tag has unsaved edits, or bytes stashed in the
            // workspace's project from an earlier session.
            match browser_modified_tags(ui) {
                Some(modified) if modified.contains_key(&entry.key) => modified_text(),
                _ => text_dark(),
            },
        );
    }
    if response.dragged()
        && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
    {
        egui::Area::new(ui.make_persistent_id(("tag_tree_drag_preview", &entry.key)))
            .order(egui::Order::Tooltip)
            // A fast drag can outrun the preview's stale position and put the
            // pointer inside it; interactable, it would then block the drop
            // target's hit-test at release.
            .interactable(false)
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
        if let Some(menu_action) = draw_tag_context_menu_contents(ui, entry, favorite_keys) {
            action = Some(menu_action);
        }
    });
    action
}

pub(in crate::app) fn draw_tag_context_menu_contents(
    ui: &mut Ui,
    entry: &TagEntry,
    favorite_keys: Option<&HashSet<String>>,
) -> Option<BrowserAction> {
    let mut action = None;
        style_tag_context_menu(ui);

        // A cache tag's way out is a conversion, and one tag is the case where
        // the user may want it somewhere other than the path the build gave it.
        if matches!(entry.location, TagEntryLocation::Monolithic { .. })
            && context_menu_button(ui, "Import into editing kit...")
                .on_hover_text(
                    "Convert this tag to a little-endian tag in an open editing kit, at a                      folder you choose",
                )
                .clicked()
        {
            action = Some(BrowserAction::ImportCacheTagIntoKit {
                key: entry.key.clone(),
            });
            ui.close_menu();
        }

        let rename_enabled = supports_rename_menu(entry);
        let duplicate_enabled = supports_duplicate_menu(entry);
        let deletable = browser_deletable_keys(ui);
        let delete_enabled = supports_delete_menu(entry, deletable.as_deref());
        let extract_enabled = supports_tag_extract_menu(entry.group_tag);
        ui.horizontal(|ui| {
            if context_menu_primary_button(ui, "Rename", rename_enabled).clicked() {
                action = Some(BrowserAction::RenameTag(entry.key.clone()));
                ui.close_menu();
            }
            if context_menu_primary_button(ui, "Duplicate", duplicate_enabled).clicked() {
                action = Some(BrowserAction::DuplicateTag(entry.key.clone()));
                ui.close_menu();
            }
            if context_menu_primary_button(ui, "Delete", delete_enabled)
                .on_disabled_hover_text(
                    "Only loose tags and Campaign Evolved tags duplicated by Baboon can be deleted",
                )
                .clicked()
            {
                action = Some(BrowserAction::DeleteTag(entry.key.clone()));
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
                        if supports_bsp_geometry_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract BSP geometry").clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(entry.key.clone()));
                            ui.close_menu();
                        }
                        if supports_scenario_geometry_extraction(entry.group_tag)
                            && context_menu_button(ui, "Extract level geometry (one file per BSP)")
                                .clicked()
                        {
                            action = Some(BrowserAction::ExtractGeometry(entry.key.clone()));
                            ui.close_menu();
                        }
                        if supports_particle_geometry_extraction(entry.group_tag)
                            && context_menu_button(
                                ui,
                                "Extract particle geometry (JMI + one JMS per object)",
                            )
                            .clicked()
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
                            action =
                                Some(BrowserAction::ExtractHlslIncludeSource(entry.key.clone()));
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

        // Whole-tag operations, inline: the raw payload, and the scenario's
        // script source. Per-asset extraction lives in the Extract submenu
        // above instead.
        // Campaign Evolved only for the script pair: the scenario keeps its
        // original `.hsc` source, and replacing it is only meaningful where that
        // round-trip is known to hold.
        let scenario_scripts =
            is_scenario_group(entry.group_tag) && browser_game_is_campaign_evolved(ui);
        if is_embedded_tag_entry(entry) || scenario_scripts {
            context_menu_separator(ui);
            if is_embedded_tag_entry(entry)
                && context_menu_button(ui, "Extract raw tag...").clicked()
            {
                action = Some(BrowserAction::ExtractRaw(entry.key.clone()));
                ui.close_menu();
            }
            if scenario_scripts {
                if context_menu_button(ui, "Extract scripts...").clicked() {
                    action = Some(BrowserAction::ExtractScenarioScripts(entry.key.clone()));
                    ui.close_menu();
                }
                if context_menu_button(ui, "Import scripts...").clicked() {
                    action = Some(BrowserAction::ImportScenarioScripts(entry.key.clone()));
                    ui.close_menu();
                }
            }
        }

        // The same two launches the tag pane's header offers, so a scenario can
        // be opened in the kit's tools without opening the tag first. Shown only
        // where the kit can launch at all; Sapien is *hidden* rather than
        // disabled where it takes no scenario argument, matching the toolbar —
        // a control that can never work is not offered greyed out.
        let launch = browser_scenario_launch(ui);
        if launch.supported && is_scenario_group(entry.group_tag) {
            context_menu_separator(ui);
            if launch.offers_sapien {
                let response = ui
                    .add_enabled_ui(launch.sapien_present, |ui| {
                        context_menu_button(ui, "Open in Sapien")
                    })
                    .inner;
                if response.clicked() {
                    action = Some(BrowserAction::LaunchScenarioInSapien(entry.key.clone()));
                    ui.close_menu();
                }
                if !launch.sapien_present {
                    response.on_disabled_hover_text("sapien.exe was not found in this editing kit");
                }
            }
            let response = ui
                .add_enabled_ui(launch.tag_test_present, |ui| {
                    context_menu_button(ui, "Open in tag_test")
                })
                .inner;
            if response.clicked() {
                action = Some(BrowserAction::LaunchScenarioInTagTest(entry.key.clone()));
                ui.close_menu();
            }
            if !launch.tag_test_present {
                response.on_disabled_hover_text("This kit's tag_test was not found in it");
            }
        }

        context_menu_separator(ui);
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
        if context_menu_button(ui, "Dump Tag References...").clicked() {
            action = Some(BrowserAction::DumpReferences(entry.key.clone()));
            ui.close_menu();
        }
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

/// True when the tag lives *inside* a container source (a monolithic cache or a
/// mounted UE5 IoStore pak set) rather than as a standalone loose file. These are
/// the sources for which "Extract raw tag..." is meaningful: it pulls the embedded
/// tag out to a self-describing standalone file. Loose-file tags are already on
/// disk, so they have nothing to extract.
pub(in crate::app) fn is_embedded_tag_entry(entry: &TagEntry) -> bool {
    matches!(
        entry.location,
        TagEntryLocation::Monolithic { .. } | TagEntryLocation::Container { .. }
    )
}

/// Whether the tag can be renamed or moved. A loose tag moves on disk, a
/// container tag writes a renamed override container, and a brand-new container
/// tag rewrites its in-memory entry — the only way to correct a path typed into
/// the New Tag dialog. A monolithic cache tag has no writable path at all.
pub(in crate::app) fn supports_rename_menu(entry: &TagEntry) -> bool {
    matches!(
        entry.location,
        TagEntryLocation::LooseFile(_)
            | TagEntryLocation::Container { .. }
            | TagEntryLocation::NewContainer { .. }
    )
}

/// Whether the context-menu duplicate has a writable on-disk provider. A
/// monolithic cache and an unsaved NewContainer have no destination bytes that
/// this action is allowed to create.
pub(in crate::app) fn supports_duplicate_menu(entry: &TagEntry) -> bool {
    matches!(
        entry.location,
        TagEntryLocation::LooseFile(_) | TagEntryLocation::Container { .. }
    )
}

/// Whether this tag may be deleted.
///
/// A loose tag is a file, so it is always offered — the delete moves it into a
/// recoverable trash folder. A container tag rewrites the game's own pak, so it
/// is offered only for copies recorded in the duplicate ledger; `deletable` is
/// that ledger, narrowed to the containers this workspace has mounted.
pub(in crate::app) fn supports_delete_menu(
    entry: &TagEntry,
    deletable: Option<&HashSet<String>>,
) -> bool {
    match &entry.location {
        TagEntryLocation::LooseFile(_) => true,
        TagEntryLocation::Container { .. } => {
            deletable.is_some_and(|keys| keys.contains(&entry.key))
        }
        TagEntryLocation::NewContainer { .. } | TagEntryLocation::Monolithic { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(location: TagEntryLocation) -> TagEntry {
        TagEntry {
            key: "test".to_owned(),
            display_path: "objects/example.model".to_owned(),
            group_tag: 0,
            group_name: None,
            location,
        }
    }

    /// Draw one browser tree and report how much vertical space it left unused.
    fn unused_height_after_tree(is_container: bool, groups_mode: bool) -> f32 {
        let entries = vec![entry(TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/objects/example-hlmt.ubulk".to_owned(),
        })];
        let tree = crate::source::build_tree(&entries);
        let ctx = egui::Context::default();
        let mut left = 0.0;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(300.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    draw_tree(
                        ui,
                        &tree,
                        &entries,
                        None,
                        "",
                        false,
                        false,
                        groups_mode,
                        None,
                        BrowserSort::Natural,
                        false,
                        None,
                        is_container,
                    );
                    left = ui.available_size_before_wrap().y;
                });
            },
        );
        left
    }

    /// The container root has no tree node — a real node's `rel_path` is never
    /// empty — so the only way to author into it is a gesture on the browser's
    /// empty space. If that space stops being claimed, `folder_rel: None`
    /// becomes unreachable again and root-level New Tag / New Folder silently
    /// disappear, with nothing failing to say so.
    #[test]
    fn a_container_browser_claims_its_empty_space_so_the_root_is_reachable() {
        assert!(
            unused_height_after_tree(true, false) < 1.0,
            "a container tree must leave no unclaimed space below it"
        );
        // A loose folder has no container root to author into, and Groups mode
        // node paths are group labels rather than folders.
        assert!(unused_height_after_tree(false, false) > 100.0);
        assert!(unused_height_after_tree(true, true) > 100.0);
    }

    /// Control for the regression test below: the same pointer script against
    /// a bare egui drag source, no Baboon code. Proves the synthetic events
    /// can start a drag at all, so the test below indicts `draw_entry` rather
    /// than the script.
    #[test]
    fn control_a_bare_drag_source_sets_its_payload() {
        let ctx = egui::Context::default();
        let mut source_rect = egui::Rect::NOTHING;
        let mut frame = |events: Vec<egui::Event>, source_rect: &mut egui::Rect| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (rect, response) = ui
                            .allocate_exact_size(Vec2::new(240.0, 20.0), Sense::click_and_drag());
                        *source_rect = rect;
                        // The non-blocking tooltip the real rows use; the raw
                        // `on_hover_text` here is what broke them (an egui
                        // 0.29 tooltip is interactable and owns the pointer's
                        // hit-test once shown, so the press never reaches the
                        // row and no drag starts).
                        hover_tooltip_beside_pointer(ui, &response, "objects/example.bitmap");
                        response.dnd_set_drag_payload(DraggedTagRef {
                            group_tag: 0,
                            input: String::new(),
                            rel_path: "control".to_owned(),
                            file_path: None,
                        });                    });
                },
            );
        };
        frame(Vec::new(), &mut source_rect);
        let start = source_rect.center();
        frame(vec![egui::Event::PointerMoved(start)], &mut source_rect);
        frame(
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut source_rect,
        );
        frame(
            vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 40.0))],
            &mut source_rect,
        );
        frame(
            vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 80.0))],
            &mut source_rect,
        );
        assert!(
            egui::DragAndDrop::has_any_payload(&ctx),
            "even a bare egui drag source set no payload — the event script is wrong",
        );
    }

    /// Regression (user report: dragging textures into shader image fields
    /// stopped working): a browser row dragged onto a shader-style drop cell
    /// must deliver its `DraggedTagRef`. Drives the real drag source —
    /// `draw_entry` — with real pointer events, into the same
    /// TextEdit-then-hover-interact structure the shader bitmap cell uses.
    #[test]
    fn a_dragged_row_delivers_its_payload_to_a_reference_cell() {
        let bitm = TagEntry {
            key: "objects/example.bitmap".to_owned(),
            display_path: "objects/example.bitmap".to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from("C:/kit/tags/objects/example.bitmap")),
        };
        let ctx = egui::Context::default();
        let mut row_rect = egui::Rect::NOTHING;
        let mut target_rect = egui::Rect::NOTHING;
        let mut hover_seen = false;
        let mut dropped: Option<String> = None;

        let mut frame = |events: Vec<egui::Event>,
                         row_rect: &mut egui::Rect,
                         target_rect: &mut egui::Rect,
                         hover_seen: &mut bool,
                         dropped: &mut Option<String>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let row_top = ui.cursor().min;
                        draw_entry(ui, &bitm, None, false, false, None, None);
                        *row_rect = egui::Rect::from_min_size(
                            row_top,
                            Vec2::new(240.0, ui.spacing().interact_size.y),
                        );
                        ui.add_space(120.0);
                        // The shader bitmap cell's structure: a TextEdit in the
                        // cell rect, then a hover interact over the same rect
                        // asking for the payload.
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(220.0, 22.0), Sense::hover());
                        *target_rect = rect;
                        let mut text = String::new();
                        ui.put(
                            rect,
                            egui::TextEdit::singleline(&mut text)
                                .hint_text(placeholder_text("(no reference)")),
                        );
                        let drop =
                            ui.interact(rect, ui.make_persistent_id("test_drop"), Sense::hover());
                        if drop.dnd_hover_payload::<DraggedTagRef>().is_some() {
                            *hover_seen = true;
                        }
                        if let Some(payload) = drop.dnd_release_payload::<DraggedTagRef>() {
                            *dropped = Some(payload.rel_path.clone());
                        }
                    });
                },
            );
        };

        // Lay out once to learn the rects, then press on the row, drag past
        // egui's drag threshold, cross onto the cell, and release there.
        frame(Vec::new(), &mut row_rect, &mut target_rect, &mut hover_seen, &mut dropped);
        let start = row_rect.center();
        let end = target_rect.center();
        frame(
            vec![egui::Event::PointerMoved(start)],
            &mut row_rect,
            &mut target_rect,
            &mut hover_seen,
            &mut dropped,
        );
        frame(
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut row_rect,
            &mut target_rect,
            &mut hover_seen,
            &mut dropped,
        );
        frame(
            vec![egui::Event::PointerMoved(start + Vec2::new(0.0, 40.0))],
            &mut row_rect,
            &mut target_rect,
            &mut hover_seen,
            &mut dropped,
        );
        frame(
            vec![egui::Event::PointerMoved(end)],
            &mut row_rect,
            &mut target_rect,
            &mut hover_seen,
            &mut dropped,
        );
        assert!(
            egui::DragAndDrop::has_any_payload(&ctx),
            "the row never set a drag payload — the drag itself is not starting",
        );
        frame(
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut row_rect,
            &mut target_rect,
            &mut hover_seen,
            &mut dropped,
        );

        assert!(
            hover_seen,
            "the drop cell never saw the drag payload while hovered",
        );
        assert_eq!(
            dropped.as_deref(),
            Some(entry_rel_path(&bitm).as_str()),
            "releasing over the cell must deliver the dragged bitmap's path",
        );
    }

    /// The value a drop writes must be one the edit applier accepts. The
    /// shader bitmap cell appends `.bitmap` to the payload's rel_path, and
    /// structural cells use the payload's `GROUP:path` input form — the bare
    /// rel_path is refused ("expected <path>.<group> or GROUP:<path>"), which
    /// is exactly the error users hit dropping a bitmap onto base_map.
    #[test]
    fn drop_payload_forms_survive_the_reference_parser() {
        use crate::app::editor::parse_tag_reference;
        let bitm = TagEntry {
            key: "objects/example.bitmap".to_owned(),
            display_path: "objects/example.bitmap".to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::LooseFile(PathBuf::from(
                "C:/kit/tags/objects/example.bitmap",
            )),
        };
        let expected = Some((u32::from_be_bytes(*b"bitm"), "objects\\example".to_owned()));

        let bare = entry_rel_path(&bitm);
        assert!(
            parse_tag_reference(&bare).is_err(),
            "a bare rel_path is not a valid reference, so no drop site may push it",
        );
        let suffixed = format!("{bare}.bitmap");
        assert_eq!(
            parse_tag_reference(&suffixed).expect("suffixed form parses").group_tag_and_name,
            expected,
            "the shader bitmap cell's dropped value",
        );
        assert_eq!(
            parse_tag_reference(&entry_reference_input(&bitm))
                .expect("GROUP:path form parses")
                .group_tag_and_name,
            expected,
            "the structural cell's dropped value",
        );
    }

    #[test]
    fn duplicate_menu_is_limited_to_writable_on_disk_entries() {
        assert!(supports_duplicate_menu(&entry(TagEntryLocation::LooseFile(
            PathBuf::from("objects/example.model")
        ))));
        assert!(supports_duplicate_menu(&entry(TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/objects/example-hlmt.ubulk".to_owned(),
        })));
        assert!(!supports_duplicate_menu(&entry(TagEntryLocation::Monolithic {
            name: "objects/example".to_owned(),
            group_tag: 0,
        })));
        assert!(!supports_duplicate_menu(&entry(TagEntryLocation::NewContainer {
            template: NewContainerTemplate::Donor {
                container: 0,
                rel_path: "Tags/template-hlmt.uasset".to_owned(),
            },
            package: "/Game/Tags/example-hlmt".to_owned(),
            group_tag: 0,
        })));
    }

    #[test]
    fn delete_menu_is_offered_for_loose_tags_and_recorded_container_copies_only() {
        let recorded = HashSet::from(["ublock:pakchunk240-WinGDK:Tags/copy-biped.ubulk".to_owned()]);
        let copy = TagEntry {
            key: "ublock:pakchunk240-WinGDK:Tags/copy-biped.ubulk".to_owned(),
            ..entry(TagEntryLocation::Container {
                container: 0,
                rel_path: "Tags/copy-biped.ubulk".to_owned(),
            })
        };
        let shipped = entry(TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/shipped-biped.ubulk".to_owned(),
        });

        assert!(supports_delete_menu(
            &entry(TagEntryLocation::LooseFile(PathBuf::from(
                "objects/example.model"
            ))),
            Some(&recorded)
        ));
        assert!(supports_delete_menu(&copy, Some(&recorded)));
        // A tag the game shipped is byte-for-byte as legitimate as a copy, so
        // the ledger is the only thing that may enable this.
        assert!(!supports_delete_menu(&shipped, Some(&recorded)));
        assert!(!supports_delete_menu(&copy, None));
        assert!(!supports_delete_menu(
            &entry(TagEntryLocation::Monolithic {
                name: "objects/example".to_owned(),
                group_tag: 0,
            }),
            Some(&recorded)
        ));
        assert!(!supports_delete_menu(
            &entry(TagEntryLocation::NewContainer {
                template: NewContainerTemplate::Donor {
                    container: 0,
                    rel_path: "Tags/template-hlmt.uasset".to_owned(),
                },
                package: "/Game/Tags/example-hlmt".to_owned(),
                group_tag: 0,
            }),
            Some(&recorded)
        ));
    }

    #[test]
    fn delete_context_button_uses_the_garbage_icon() {
        assert_eq!(context_menu_icon("Delete"), Some(ButtonIcon::Garbage));
    }

    #[test]
    fn duplicate_context_button_uses_duplicate_asset_icon() {
        assert_eq!(
            context_menu_icon("Duplicate"),
            Some(ButtonIcon::Duplicate)
        );
    }

    /// The count that decides whether a folder offers the cache import at all.
    ///
    /// Recursive, and monolithic-only: a workspace can hold a mix after a
    /// browser rebuild, and offering "import 40 tags" on a folder holding 3
    /// cache tags and 37 loose ones would promise a run that converts 3.
    #[test]
    fn a_folder_counts_only_the_cache_tags_beneath_it() {
        let entries = vec![
            entry(TagEntryLocation::Monolithic {
                name: r"objects\weapons\rifle\assault_rifle".to_owned(),
                group_tag: u32::from_be_bytes(*b"weap"),
            }),
            entry(TagEntryLocation::Monolithic {
                name: r"objects\weapons\rifle\scope\scope".to_owned(),
                group_tag: u32::from_be_bytes(*b"weap"),
            }),
            entry(TagEntryLocation::LooseFile(PathBuf::from(
                "D:/HREK/tags/objects/weapons/rifle/assault_rifle.weapon",
            ))),
        ];
        let node = |rel: &str, entries: Vec<usize>| crate::source::TagTreeNode {
            label: rel.rsplit('/').next().unwrap_or(rel).to_owned(),
            rel_path: PathBuf::from(rel),
            children: Vec::new(),
            children_loaded: true,
            entries,
            entries_loaded: true,
            pending: false,
        };
        let mut tree = node("objects/weapons/rifle", vec![0, 2]);
        assert_eq!(count_cache_tags(&tree, &entries), 1, "the loose tag was counted");
        tree.children.push(node("objects/weapons/rifle/scope", vec![1]));
        assert_eq!(
            count_cache_tags(&tree, &entries),
            2,
            "a subfolder's tags were not counted"
        );
    }
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

/// The groups that *are* render geometry: `mode` (H2+ render_model, and H1's
/// legacy `model`, which previews through the same dispatch) and `mod2`
/// (H1 gbxmodel). Deliberately not `hlmt` — a model tag has no geometry of its
/// own, only a reference to one of these.
pub(in crate::app) fn is_render_model_group(group_tag: u32) -> bool {
    matches!(&group_tag.to_be_bytes(), b"mode" | b"mod2")
}

pub(in crate::app) fn is_render_model_tag(entry: &TagEntry) -> bool {
    is_render_model_group(entry.group_tag)
        || matches!(
            entry.group_name.as_deref(),
            Some("render_model") | Some("gbxmodel")
        )
        || {
            let path = entry.display_path.to_ascii_lowercase();
            path.ends_with(".render_model") || path.ends_with(".gbxmodel")
        }
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
        || supports_bsp_geometry_extraction(group_tag)
        || supports_scenario_geometry_extraction(group_tag)
        || supports_particle_geometry_extraction(group_tag)
        || supports_animation_extraction(group_tag)
        || supports_tag_import_info_extraction(group_tag)
        || is_bitmap_group(group_tag)
        || is_material_shader_group(group_tag)
        || is_hlsl_include_group(group_tag)
}

pub(in crate::app) fn supports_tag_geometry_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"hlmt" | b"mode" | b"phmo" | b"coll" | b"mod2"
    )
}

/// A single structure BSP exports to one ASS. Kept apart from
/// [`supports_tag_geometry_extraction`] because the menu wording differs
/// — a BSP is level geometry, not a model — and because the two land in
/// different arms of `extract_geometry_for_entry`.
pub(in crate::app) fn supports_bsp_geometry_extraction(group_tag: u32) -> bool {
    group_tag.to_be_bytes().as_slice() == b"sbsp"
}

/// A scenario exports every BSP it references, one file each.
pub(in crate::app) fn supports_scenario_geometry_extraction(group_tag: u32) -> bool {
    group_tag.to_be_bytes().as_slice() == b"scnr"
}

/// A particle_model exports to a `.jmi` manifest plus one JMS per object
/// it was imported from — not a single file — so it gets its own wording
/// rather than joining [`supports_tag_geometry_extraction`].
pub(in crate::app) fn supports_particle_geometry_extraction(group_tag: u32) -> bool {
    blam_tags::is_particle_model_group(group_tag)
}

pub(in crate::app) fn supports_tag_import_info_extraction(group_tag: u32) -> bool {
    matches!(
        group_tag.to_be_bytes().as_slice(),
        b"hlmt" | b"mode" | b"phmo" | b"coll" | b"mod2"
    )
}
