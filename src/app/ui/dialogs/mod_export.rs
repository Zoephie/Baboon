//! The Export Mod review window and its field-by-field diff view.
//! It owns the review's presentation and the diff tree it draws; building the mod and writing the pak belong to the controller.

use super::*;

/// One changed element and the rows that belong to it.
struct DiffSection {
    /// Path in the edited tag, e.g. `zone set pvs[3]`. Empty for the root.
    element: String,
    /// The same element in the shipped tag, where the index differs.
    base_element: Option<String>,
    label: String,
    kind: ModExportChange,
    rows: Vec<TagFieldDiff>,
    /// The fields `rows` touch, as the editor's filter; see `diff_field_filter`.
    filter: std::sync::Arc<FieldFilter>,
}

/// A container in the tag, holding whatever changed inside it.
///
/// Built once per reviewed tag and kept with its diff (`ModRowDiff::view`):
/// building it clones every changed row and derives a filter per section,
/// which for a large diff is too much to repeat every frame.
#[derive(Default)]
pub(in crate::app) struct DiffNode {
    title: String,
    children: Vec<DiffNode>,
    sections: Vec<DiffSection>,
}

impl DiffNode {
    /// Merge a container that only leads somewhere else into its child, so a
    /// deep change reads as one breadcrumb rather than a stack of boxes.
    fn collapse_chains(&mut self) {
        for child in self.children.iter_mut() {
            child.collapse_chains();
        }
        while self.sections.is_empty() && self.children.len() == 1 && !self.title.is_empty() {
            let child = self.children.remove(0);
            // A chevron the shipped fonts carry -- see the glyph fallback work.
            self.title = format!("{} › {}", self.title, child.title);
            self.children = child.children;
            self.sections = child.sections;
        }
    }
}

impl Baboon {
    /// Destructive-save confirmation for Campaign Evolved container tags. Save
    /// overwrites the shipped pak files in place, so we always confirm and point
    /// the user at Export Mod as the non-destructive alternative.
    /// Split a diff path into the element it belongs to and the field within
    /// it, so changes can be grouped under one heading per element.
    ///
    /// Changes nest -- `weapons[2]/triggers[0]/barrels[1]/damage` -- and the
    /// innermost element is the one worth heading, with the whole chain shown
    /// so it is unambiguous which one it is.
    fn split_element_path(path: &str) -> (&str, &str) {
        match path.rfind(']') {
            Some(end) => {
                let field = &path[end + 1..];
                (&path[..=end], field.strip_prefix('/').unwrap_or(field))
            }
            None => ("", path),
        }
    }

    /// One section per changed element, in the order the tag has them, each
    /// carrying the rows that belong to it so its panes can be filtered to just
    /// those. A single filter over every row made each section render the
    /// ancestors of every *other* section too -- a block that merely contained a
    /// change appeared as though it were one.
    fn build_diff_sections(rows: &[TagFieldDiff]) -> Vec<DiffSection> {
        let mut sections: Vec<DiffSection> = Vec::new();
        for row in rows {
            let (element, _) = Self::split_element_path(&row.path);
            let base_element = row
                .base_path
                .as_deref()
                .map(|path| Self::split_element_path(path).0.to_owned());
            let label = if Self::split_element_path(&row.path).1.is_empty() {
                if row.b.is_empty() {
                    row.a.clone()
                } else {
                    row.b.clone()
                }
            } else {
                String::new()
            };
            // What happened to the element as a whole, from the row that is
            // about the element rather than about a field inside it.
            let kind = if Self::split_element_path(&row.path).1.is_empty() {
                if row.a.is_empty() {
                    ModExportChange::New
                } else if row.b.is_empty() {
                    ModExportChange::Unresolved
                } else {
                    ModExportChange::Modified
                }
            } else {
                ModExportChange::Modified
            };
            // An added or removed element is reported with all of its
            // contents, and those rows sit underneath it. They are already
            // shown by that element's own pane, so they must not each open a
            // section of their own -- doing so rendered a removed element's
            // fields as a before/after split against whatever index had
            // shifted into their place, inventing changes the diff never
            // reported.
            if let Some(last) = sections.last_mut()
                && matches!(
                    last.kind,
                    ModExportChange::New | ModExportChange::Unresolved
                )
                && row.path.starts_with(last.element.as_str())
            {
                last.rows.push(row.clone());
                continue;
            }
            match sections.last_mut() {
                Some(last) if last.element == element => {
                    if last.label.is_empty() && !label.is_empty() {
                        last.label = label;
                        last.kind = kind;
                    }
                    last.rows.push(row.clone());
                }
                _ => sections.push(DiffSection {
                    element: element.to_owned(),
                    base_element,
                    label,
                    kind,
                    rows: vec![row.clone()],
                    filter: Default::default(),
                }),
            }
        }
        for section in &mut sections {
            section.filter = std::sync::Arc::new(Self::diff_field_filter(&section.rows));
        }
        sections
    }

    /// Arrange the sections into the shape of the tag, so a change is shown
    /// inside the containers that hold it rather than under a path.
    ///
    /// Keyed on each section's container path -- its element path without the
    /// final `[n]` -- split at `/`. An unbranching chain of containers is
    /// merged into one title: four nested boxes around a single changed dword
    /// is depth without information.
    fn build_diff_tree(sections: Vec<DiffSection>) -> DiffNode {
        #[cfg(test)]
        diff_view_tests::TREES_BUILT.with(|built| built.set(built.get() + 1));
        let mut root = DiffNode::default();
        for section in sections {
            let (container, _) = Self::split_element_index(&section.element);
            let mut node = &mut root;
            for segment in container.split('/').filter(|s| !s.is_empty()) {
                let existing = node
                    .children
                    .iter()
                    .position(|child| child.title == segment);
                let index = match existing {
                    Some(index) => index,
                    None => {
                        node.children.push(DiffNode {
                            title: segment.to_owned(),
                            ..DiffNode::default()
                        });
                        node.children.len() - 1
                    }
                };
                node = &mut node.children[index];
            }
            node.sections.push(section);
        }
        root.collapse_chains();
        root
    }

    /// Split `zone set pvs[3]` into the block it names and the element index.
    ///
    /// A reader wants to know which block changed and which element of it, not
    /// to parse an indexed path.
    fn split_element_index(element: &str) -> (&str, Option<usize>) {
        let Some(open) = element.rfind('[') else {
            return (element, None);
        };
        let index = element[open + 1..]
            .trim_end_matches(']')
            .parse::<usize>()
            .ok();
        match index {
            Some(index) => (&element[..open], Some(index)),
            None => (element, None),
        }
    }

    /// Which fields a diff touched, as the editor's own field filter.
    ///
    /// Canonical (index-free) paths, so one filter serves both sides: deleting
    /// an element shifts indices, but `zone set pvs/structure bsp mask` names
    /// the same field whether it sits at element 3 or 4.
    fn diff_field_filter(rows: &[TagFieldDiff]) -> FieldFilter {
        let mut visible_paths = HashSet::new();
        for row in rows {
            for path in [Some(&row.path), row.base_path.as_ref()]
                .into_iter()
                .flatten()
            {
                let canonical = strip_node_indices(path);
                // Ancestors too: a container has to render for what is inside
                // it to be reachable.
                let mut prefix = canonical.as_str();
                loop {
                    visible_paths.insert(prefix.to_owned());
                    match prefix.rfind('/') {
                        Some(cut) => prefix = &prefix[..cut],
                        None => break,
                    }
                }
            }
        }
        FieldFilter { visible_paths }
    }

    /// Render one side of a diff through the real field editor, read-only.
    ///
    /// This is the editor's own renderer, not an imitation of it: values are
    /// formatted, enums named and references resolved exactly as they are when
    /// editing, which is what makes the change inspectable rather than merely
    /// visible.
    #[allow(clippy::too_many_arguments)]
    fn draw_diff_side(
        ui: &mut Ui,
        tag: &blam_tags::TagFile,
        path: &str,
        filter: &std::sync::Arc<FieldFilter>,
        names: &TagNameIndex,
        group_tag: u32,
        game: Option<&str>,
        definitions_root: Option<&Path>,
        expert_mode: bool,
        scope: &str,
    ) {
        // The editor collects deferred edits as it draws. Nothing here is
        // editable, so they are collected into locals and dropped.
        let mut sinks = EditSinks::default();
        let root = tag.root();
        let Some(target) = (if path.is_empty() {
            Some(root)
        } else {
            root.descend(path)
        }) else {
            ui.label(
                RichText::new("not present on this side")
                    .color(subtle_dark())
                    .small(),
            );
            return;
        };
        let filter_action = FieldFilterAction::Apply(filter.clone());
        let mut edit = FieldEditContext::read_only(&mut sinks, scope, scope);
        edit.expand_all = Some(true);
        edit.nested_default = NestedDefault::Expanded;
        edit.group_tag = group_tag;
        edit.root = Some(root);
        edit.game = game;
        edit.definitions_root = definitions_root;
        edit.names = Some(names);
        edit.field_filter = Some(&filter_action);
        draw_struct_fields_inline(ui, target, names, 0, expert_mode, path, &mut edit);
    }

    /// One tag's differences, each shown through the real field editor with
    /// the shipped value on the left and the edited one on the right.
    ///
    /// Rendered per changed element rather than as one whole-tag view: the
    /// editor shows a block one element at a time behind its instance
    /// selector, so changes spanning elements 3 and 7 could never both be on
    /// screen. Each changed element gets its own section, which is what makes
    /// the whole change visible at once.
    #[allow(clippy::too_many_arguments)]
    fn draw_mod_export_diff(
        ui: &mut Ui,
        diff: &ModRowDiff,
        names: &TagNameIndex,
        group_tag: u32,
        game: Option<&str>,
        definitions_root: Option<&Path>,
        expert_mode: bool,
        scope: &str,
    ) {
        if let Some(error) = diff.error.as_deref() {
            ui.label(RichText::new(error).color(removed_text()).small());
            return;
        }
        if diff.rows.is_empty() {
            ui.label(
                RichText::new("No differences from the shipped tag.")
                    .color(subtle_dark())
                    .small(),
            );
            return;
        }
        let tree = diff
            .view
            .get_or_init(|| Self::build_diff_tree(Self::build_diff_sections(&diff.rows)));
        Self::draw_diff_node(
            ui,
            tree,
            0,
            diff,
            names,
            group_tag,
            game,
            definitions_root,
            expert_mode,
            scope,
        );
        if diff.truncated {
            ui.add_space(4.0);
            ui.label(
                RichText::new("More differences than can be listed here.")
                    .color(subtle_dark())
                    .small(),
            );
        }
    }

    /// How many elements a block has on one side, for the block a change sits
    /// in.
    ///
    /// It is the one fact the element panes cannot convey -- a pane shows the
    /// element that went, not that the block went from six to five -- and it is
    /// the first thing a reader checks.
    fn block_len(tag: &blam_tags::TagFile, container: &str) -> Option<usize> {
        let (parent, name) = match container.rsplit_once('/') {
            Some((parent, name)) => (parent, name),
            None => ("", container),
        };
        let root = tag.root();
        let owner = if parent.is_empty() {
            root
        } else {
            root.descend(parent)?
        };
        owner
            .fields_all()
            .find(|field| field.name() == name)?
            .as_block()
            .map(|block| block.len())
    }

    /// `6 \u{2192} 5`, when a container's element count changed.
    fn block_count_change(diff: &ModRowDiff, node: &DiffNode) -> Option<String> {
        let section = node.sections.first()?;
        let (container, _) = Self::split_element_index(&section.element);
        let base_container = section
            .base_element
            .as_deref()
            .map(|element| Self::split_element_index(element).0)
            .unwrap_or(container);
        let before = Self::block_len(diff.base.as_ref()?, base_container)?;
        let after = Self::block_len(diff.edited.as_ref()?, container)?;
        (before != after).then(|| format!("{before} \u{2192} {after}"))
    }

    /// One container and everything that changed inside it.
    #[allow(clippy::too_many_arguments)]
    fn draw_diff_node(
        ui: &mut Ui,
        node: &DiffNode,
        depth: usize,
        diff: &ModRowDiff,
        names: &TagNameIndex,
        group_tag: u32,
        game: Option<&str>,
        definitions_root: Option<&Path>,
        expert_mode: bool,
        scope: &str,
    ) {
        for section in &node.sections {
            Self::draw_diff_section(
                ui,
                section,
                diff,
                names,
                group_tag,
                game,
                definitions_root,
                expert_mode,
                scope,
            );
        }
        for child in &node.children {
            // The editor's own container chrome, so a block in the review looks
            // like the block it is.
            let title = match Self::block_count_change(diff, child) {
                Some(counts) => format!("{}  {counts}", child.title),
                None => child.title.clone(),
            };
            draw_foundation_group(
                ui,
                title,
                ("diff_node", scope, child.title.as_str()),
                depth,
                true,
                None,
                |ui| {
                    Self::draw_diff_node(
                        ui,
                        child,
                        depth + 1,
                        diff,
                        names,
                        group_tag,
                        game,
                        definitions_root,
                        expert_mode,
                        scope,
                    );
                },
            );
        }
    }

    /// One changed element, inside whatever container is already drawn around it.
    #[allow(clippy::too_many_arguments)]
    fn draw_diff_section(
        ui: &mut Ui,
        section: &DiffSection,
        diff: &ModRowDiff,
        names: &TagNameIndex,
        group_tag: u32,
        game: Option<&str>,
        definitions_root: Option<&Path>,
        expert_mode: bool,
        scope: &str,
    ) {
        let DiffSection {
            element,
            base_element,
            label,
            kind,
            filter,
            ..
        } = section;
        let (kind, element, base_element, label) =
            (*kind, element.clone(), base_element.clone(), label.clone());
        ui.add_space(6.0);
        if !element.is_empty() {
            // `Unresolved` stands in for "gone", the only way an element leaves.
            // `Unchanged` is a whole-tag verdict and never labels an element,
            // so it reads as a plain change here rather than inventing a marker.
            let (marker, heading) = match kind {
                ModExportChange::New => ("+", added_text()),
                ModExportChange::Unresolved => ("-", removed_text()),
                ModExportChange::Modified | ModExportChange::Unchanged => ("~", modified_text()),
            };
            // The container above already names the block, so this only has to
            // say which element of it, and what happened to it.
            let (_, index) = Self::split_element_index(&element);
            ui.horizontal(|ui| {
                ui.label(RichText::new(marker).color(heading).monospace().strong());
                if let Some(index) = index {
                    ui.label(RichText::new(format!("element {index}")).color(heading));
                }
                if !label.is_empty() {
                    let detail = label
                        .split_once(" — ")
                        .map(|(_, rest)| rest)
                        .unwrap_or(label.as_str());
                    if detail != format!("element {}", index.unwrap_or_default()) {
                        ui.label(RichText::new(detail).color(heading).small());
                    }
                }
            });
        }
        // Only a modified element has two sides worth comparing. An
        // element that was added or removed exists on one side only, and a
        // half-width pane beside an empty twin says less than one full
        // pane in the colour of what happened.
        let available = ui.available_width();
        let half = ((available - 16.0) / 2.0).max(160.0);
        let pane = |ui: &mut Ui, width: f32, before: bool, title: &str| {
            let (wash, accent) = if before {
                (removed_wash(), removed_text())
            } else {
                (added_wash(), added_text())
            };
            ui.allocate_ui_with_layout(
                Vec2::new(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    Frame::none()
                        .fill(wash)
                        .stroke(Stroke::new(1.0_f32, accent.gamma_multiply(0.5)))
                        .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(title).color(accent).small());
                            // Scrolled within its own pane: an editor row is
                            // wider than half a dialog, and without this the
                            // window grows to fit it every frame.
                            egui::ScrollArea::horizontal()
                                .id_salt((title, &element))
                                // Fill the pane's width, but only as tall as
                                // what is in it.
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    let (tag, path, side) = if before {
                                        (
                                            diff.base.as_ref(),
                                            base_element.as_deref().unwrap_or(&element),
                                            "before",
                                        )
                                    } else {
                                        (diff.edited.as_ref(), element.as_str(), "after")
                                    };
                                    match tag {
                                        Some(tag) => Self::draw_diff_side(
                                            ui,
                                            tag,
                                            path,
                                            filter,
                                            names,
                                            group_tag,
                                            game,
                                            definitions_root,
                                            expert_mode,
                                            &format!("{scope}|{side}"),
                                        ),
                                        None => {
                                            ui.label(
                                                RichText::new("not present")
                                                    .color(subtle_dark())
                                                    .small(),
                                            );
                                        }
                                    }
                                });
                        });
                },
            );
        };
        match kind {
            ModExportChange::New => pane(ui, available, false, "added"),
            ModExportChange::Unresolved => pane(ui, available, true, "removed"),
            ModExportChange::Modified | ModExportChange::Unchanged => {
                ui.horizontal_top(|ui| {
                    pane(ui, half, true, "before");
                    // Not a separator: in a horizontal layout it stretches
                    // to the panel's whole remaining height, which left a
                    // screen of empty space under two short panes. The two
                    // washes already read as two panes.
                    ui.add_space(4.0);
                    pane(ui, half, false, "after");
                });
            }
        }
    }

    /// Review what Export Mod is about to write, and where.
    ///
    /// The save dialog this replaces asked for one file name when the output is
    /// three, which invited renaming -- and renaming is how a mod loses the
    /// `_P` that gives it priority over the game's own containers. It also
    /// guarded only the container, silently overwriting the `.ucas` and `.pak`
    /// beside it.
    pub(in crate::app::ui) fn draw_mod_export_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.mod_export.as_ref() else {
            return;
        };
        let kit = dialog.kit;
        let new_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::New)
            .count();
        let modified_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::Modified)
            .count();
        let unresolved_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::Unresolved)
            .count();
        let unchanged_count = dialog
            .rows
            .iter()
            .filter(|row| row.kind == ModExportChange::Unchanged)
            .count();
        let stem = dialog.stem();
        let destination = dialog.destination();
        let existing = dialog.existing_files();
        let in_game_folder = self
            .kits
            .iter()
            .find(|k| k.id == kit)
            .and_then(|k| k.source.as_ref())
            .map(|source| destination.starts_with(source.source.root_path()))
            .unwrap_or(false);
        let included = dialog.included().count();
        let name_ok = !dialog.name.trim().is_empty();
        // The editor needs its source's naming and definitions to render values
        // the way the editor does.
        let kit_index = self.kits.iter().position(|k| k.id == kit);
        let names = kit_index
            .map(|index| self.kits[index].names.clone())
            .unwrap_or_default();
        let game = kit_index
            .and_then(|index| self.kits[index].source.as_ref())
            .and_then(|source| source.game.clone());
        let definitions_root = kit_index
            .and_then(|index| self.kits[index].source.as_ref())
            .and_then(|source| match &source.source {
                TagSource::LooseFolder {
                    definitions_root, ..
                } => Some(definitions_root.clone()),
                _ => None,
            });
        let expert_mode = self.prefs.expert_mode;
        // Mods installed under `Paks` are mounted like any other container, so
        // they serve their tags in place of the game's. Both facts below follow
        // from that and neither was visible: comparisons here are against the
        // game's own packs, and the file this would write may be one of those
        // mounts — which cannot be replaced while it is mapped.
        let export_target = self
            .mod_export
            .as_ref()
            .filter(|dialog| !dialog.review_only)
            .map(ModExportDialog::output_utoc);
        let mounted_mods = kit_index
            .map(|index| self.mounted_mod_labels(index))
            .unwrap_or_default();
        let replaces_mounted = export_target
            .as_deref()
            .zip(kit_index)
            .map(|(target, index)| self.export_replaces_mounted(index, target))
            .unwrap_or_default();

        let mut open = true;
        let mut cancel = false;
        let mut export = false;
        let mut browse = false;
        let mut save_diagnostic = false;
        let mut acknowledge: Option<bool> = None;
        let mut set_all: Option<bool> = None;
        let mut toggled: Option<usize> = None;
        let mut expand_toggled: Option<String> = None;
        let mut measured_controls: Option<f32> = None;
        let mut name_edit = dialog.name.clone();

        let review_only = dialog.review_only;
        egui::Window::new(if review_only {
            "Unexported changes"
        } else {
            "Export Mod"
        })
            .id(egui::Id::new("mod_export"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(1100.0)
            .default_height(640.0)
            // Centred on first open, and draggable after that. `anchor` looks
            // like the way to centre a window and is not: it calls
            // `movable(false)` internally and re-pins the window every frame, so
            // the review -- the one dialog a reader wants to slide aside to look
            // at the tag underneath -- could be resized but never moved.
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                let Some(dialog) = self.mod_export.as_ref() else {
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{new_count} new · {modified_count} modified"
                        ))
                        .color(text_dark()),
                    );
                    if unchanged_count > 0 {
                        // Named rather than silently dropped: these are tags the
                        // workspace still has stashed, and a user who remembers
                        // touching one deserves to see that it came to nothing.
                        ui.label(
                            RichText::new(format!("· {unchanged_count} unchanged"))
                                .color(subtle_dark()),
                        )
                        .on_hover_text(
                            "Byte-identical to the game's own copy, so there is nothing \
                             to export. They stay stashed.",
                        );
                    }
                    if unresolved_count > 0 {
                        ui.label(
                            RichText::new(format!("· {unresolved_count} excluded"))
                                .color(egui::Color32::from_rgb(210, 120, 90)),
                        );
                    }
                    if !review_only {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Include none").clicked() {
                                set_all = Some(false);
                            }
                            if ui.button("Include all").clicked() {
                                set_all = Some(true);
                            }
                        });
                    }
                });
                if !mounted_mods.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "Mounted mod(s) in this install: {}. Changes are compared against the \
                             game's own packs, so a tag one of these already provides still shows \
                             what it changes.",
                            mounted_mods.join(", ")
                        ))
                        .small()
                        .color(subtle_dark()),
                    );
                }
                ui.add_space(6.0);
                // Grows with the window: the naming and buttons below keep the
                // slice they measured last frame, and the list takes whatever is
                // left, so making the dialog taller shows more of the diff
                // rather than more empty space.
                //
                // The slice is measured rather than assumed. It was 120px, and
                // the block below is 141px once the overwrite warning and the
                // in-game-folder note are both showing -- so the contents came
                // out 21px taller than the window, every frame, and a resizable
                // egui window expands to fit its contents and never shrinks
                // back. The dialog grew until it was larger than the screen,
                // showing the extra height as empty list.
                let reserve = if dialog.controls_height > 0.0 {
                    dialog.controls_height
                } else {
                    // First frame, nothing measured yet. Over-reserving costs
                    // one frame of a shorter list; under-reserving is the bug.
                    160.0
                };
                let list_height = (ui.available_height() - reserve).max(120.0);
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, row) in dialog.rows.iter().enumerate() {
                            // A new tag opens too: it has no counterpart to
                            // compare against, so it shows what is in it.
                            let expandable = row.kind != ModExportChange::Unresolved;
                            let expanded = dialog.expanded.contains(&row.identity);
                            ui.horizontal(|ui| {
                                if expandable {
                                    if ui
                                        .small_button(if expanded { "v" } else { ">" })
                                        .on_hover_text(if row.kind == ModExportChange::New {
                                            "Show what this tag contains"
                                        } else {
                                            "Show what changed"
                                        })
                                        .clicked()
                                    {
                                        expand_toggled = Some(row.identity.clone());
                                    }
                                } else {
                                    ui.add_space(18.0);
                                }
                                if !review_only {
                                    let mut include = row.include;
                                    let enabled = row.kind != ModExportChange::Unresolved;
                                    if ui
                                        .add_enabled(enabled, egui::Checkbox::new(&mut include, ""))
                                        .changed()
                                    {
                                        toggled = Some(index);
                                    }
                                }
                                let (marker, color) = match row.kind {
                                    ModExportChange::New => ("+", added_text()),
                                    ModExportChange::Modified => ("~", modified_text()),
                                    ModExportChange::Unresolved => {
                                        ("!", egui::Color32::from_rgb(210, 120, 90))
                                    }
                                    // Nothing to write, so nothing to mark.
                                    ModExportChange::Unchanged => {
                                        ("=", egui::Color32::from_gray(130))
                                    }
                                };
                                // A marker as well as a colour: this is a
                                // confirmation before writing files, and colour
                                // alone excludes a good number of readers.
                                ui.label(RichText::new(marker).color(color).monospace());
                                ui.label(RichText::new(&row.display_path).color(color));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("{} KB", row.bytes / 1024))
                                                .color(subtle_dark())
                                                .small(),
                                        );
                                        if let Some(reason) = row.reason.as_deref() {
                                            ui.label(
                                                RichText::new(reason).color(subtle_dark()).small(),
                                            );
                                        }
                                        // The editor is showing this mod's values
                                        // for this tag, which is why an edit can
                                        // look like it was already there.
                                        if let Some(mod_label) = row.overridden_by.as_deref() {
                                            ui.label(
                                                RichText::new(format!("in {mod_label}"))
                                                    .color(modified_text())
                                                    .small(),
                                            )
                                            .on_hover_text(format!(
                                                "This install's {mod_label} already provides this \
                                                 tag, so the editor reads its values. The \
                                                 comparison below is against the game's own pack.",
                                            ));
                                        }
                                    },
                                );
                            });
                            if expandable && expanded {
                                ui.indent(("mod_export_diff", index), |ui| {
                                    match dialog.diffs.get(&row.identity) {
                                        Some(diff) => Self::draw_mod_export_diff(
                                            ui,
                                            diff,
                                            &names,
                                            row.group_tag,
                                            game.as_deref(),
                                            definitions_root.as_deref(),
                                            expert_mode,
                                            &row.identity,
                                        ),
                                        None => {
                                            ui.label(
                                                RichText::new("Comparing...")
                                                    .color(subtle_dark())
                                                    .small(),
                                            );
                                        }
                                    }
                                });
                            }
                        }
                    });
                // Everything from here down is what `reserve` covers. Taken from
                // the list's own bottom rather than from the cursor, so the
                // spacing between them is inside the figure -- a few pixels
                // short is the same runaway, only slower.
                let controls_top = ui.min_rect().bottom();
                if review_only {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close").clicked() {
                            cancel = true;
                        }
                        if ui
                            .button("Save diagnostic...")
                            .on_hover_text("Write both sides of every tag, and the computed differences, to a folder")
                            .clicked()
                        {
                            save_diagnostic = true;
                        }
                    });
                    measured_controls = Some(ui.min_rect().bottom() - controls_top);
                    return;
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Mod name").color(text_dark()));
                    ui.add(egui::TextEdit::singleline(&mut name_edit).desired_width(220.0));
                    ui.label(
                        RichText::new("names the files, not a folder")
                            .color(subtle_dark())
                            .small(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Export folder").color(text_dark()));
                    ui.label(
                        RichText::new(destination.display().to_string())
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });
                // Named in full rather than summarised: a mod is four files, the
                // review is the last chance to notice one of them is about to
                // land somewhere unintended, and "{stem}.utoc / .ucas / .pak"
                // left the `.baboon` sidecar out of a list it was already
                // writing.
                ui.label(RichText::new("Writes").color(text_dark()));
                for extension in MOD_FILE_EXTENSIONS {
                    ui.label(
                        RichText::new(format!("    {stem}.{extension}"))
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                }
                if in_game_folder {
                    ui.label(
                        RichText::new(
                            "This is under the game's own Paks folder — nothing to copy \
                             afterwards.",
                        )
                        .color(subtle_dark())
                        .small(),
                    );
                }
                if !existing.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Overwrites: {}", existing.join(", ")))
                            .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                    // Named and then confirmed. A mod is three files plus its
                    // sidecar, and replacing someone's existing mod should take
                    // more than not noticing a line of text.
                    let mut acknowledged = dialog.overwrite_acknowledged;
                    if ui
                        .checkbox(&mut acknowledged, "Replace these files")
                        .changed()
                    {
                        acknowledge = Some(acknowledged);
                    }
                }
                // Said as the name is typed, because it is the difference between
                // writing a new mod and replacing one this workspace is reading
                // from. The export releases the mapping to do it, and the browser
                // then shows what was just written.
                if !replaces_mounted.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "Replaces {}, which is mounted here — the browser will show what this \
                             writes. Reload the source afterwards if the tag list changed.",
                            replaces_mounted.join(", ")
                        ))
                        .small()
                        .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let overwrite_ok = existing.is_empty() || dialog.overwrite_acknowledged;
                    let ready = name_ok && included > 0 && overwrite_ok;
                    if ui
                        .add_enabled(ready, egui::Button::new("Export"))
                        .on_disabled_hover_text(if !name_ok {
                            "Enter a name for the mod"
                        } else if included == 0 {
                            "Nothing is selected to export"
                        } else {
                            "Confirm that the existing files may be replaced"
                        })
                        .clicked()
                    {
                        export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .button("Save diagnostic...")
                        .on_hover_text("Write both sides of every tag, and the computed differences, to a folder")
                        .clicked()
                    {
                        save_diagnostic = true;
                    }
                });
                measured_controls = Some(ui.min_rect().bottom() - controls_top);
            });

        // Applied after the window closes its borrow of `self`.
        if let Some(dialog) = self.mod_export.as_mut() {
            if dialog.name != name_edit {
                // Kept verbatim. Folding the buffer on every keystroke ate the
                // space in "My Mod" before the second word could be typed; the
                // fold belongs to the file name, which `stem` produces and the
                // dialog shows live beside the field.
                dialog.name = name_edit;
                dialog.overwrite_acknowledged = false;
            }
            if let Some(value) = acknowledge {
                dialog.overwrite_acknowledged = value;
            }
            if let Some(value) = set_all {
                for row in dialog.rows.iter_mut() {
                    if row.kind != ModExportChange::Unresolved {
                        row.include = value;
                    }
                }
            }
            if let Some(index) = toggled
                && let Some(row) = dialog.rows.get_mut(index)
            {
                row.include = !row.include;
            }
            if let Some(identity) = expand_toggled.as_ref() {
                if !dialog.expanded.remove(identity) {
                    dialog.expanded.insert(identity.clone());
                }
            }
            if let Some(height) = measured_controls {
                // The tallest seen, not the latest. The overwrite warning comes
                // and goes as the name is typed, and a reserve that tracked it
                // downwards would under-reserve the frame it comes back --
                // which, since the window cannot shrink, is a bump it keeps.
                // Over-reserving only costs a few pixels of list.
                dialog.controls_height = dialog.controls_height.max(height.max(0.0));
            }
        }
        // Computed outside the window, and only for rows that are open and have
        // no result yet: each one costs a container read and two parses.
        let pending: Vec<String> = self
            .mod_export
            .as_ref()
            .map(|dialog| {
                dialog
                    .expanded
                    .iter()
                    .filter(|identity| !dialog.diffs.contains_key(*identity))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !pending.is_empty()
            && let Some(index) = self.resolve_kit(kit)
        {
            for identity in pending {
                let diff = self.diff_reviewed_tag(index, &identity);
                if let Some(dialog) = self.mod_export.as_mut() {
                    dialog.diffs.insert(identity, diff);
                }
            }
        }
        if save_diagnostic
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Save review diagnostic into folder")
                .pick_folder()
        {
            self.status = match self.save_review_diagnostic(folder.clone()) {
                Ok(count) => {
                    format!(
                        "Wrote a diagnostic for {count} tag(s) to {}",
                        folder.display()
                    )
                }
                Err(error) => error,
            };
        }
        // Opens where the mod is currently going, rather than at whatever the
        // OS last remembered — the common edit is "somewhere near here", and
        // the default is already the game's own `~mods`.
        if browse
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Export mod into folder")
                .set_directory(&destination)
                .pick_folder()
            && let Some(dialog) = self.mod_export.as_mut()
        {
            dialog.folder = folder;
            dialog.overwrite_acknowledged = false;
        }
        if !open || cancel {
            self.mod_export = None;
            return;
        }
        if export {
            let Some(dialog) = self.mod_export.as_ref() else {
                return;
            };
            let included: HashSet<String> =
                dialog.included().map(|row| row.identity.clone()).collect();
            let output = dialog.destination().join(format!("{}.utoc", dialog.stem()));
            // Kept for the next export in this session, so replacing a mod's
            // files does not mean typing its name again.
            let remembered = dialog.name.clone();
            self.last_mod_export_name = Some(remembered);
            let snapshot = dialog.snapshot.clone();
            // The workspace may have been closed while this was open.
            if self.focus_navigation_kit(kit) {
                self.write_reviewed_mod(&snapshot, &included, output, ctx);
            }
            self.mod_export = None;
        }
    }
}

#[cfg(test)]
mod mod_export_tests {
    use super::*;
    use std::path::PathBuf;

    /// The reported case, from `review-diagnostic.json`: two top-level fields
    /// changed, `zone set pvs[3]` deleted, a `zone sets` element added. What the
    /// reporter asked to see is exactly three things, in the containers that
    /// hold them -- not 131 rows of shifted indices.
    #[test]
    fn tree_matches_the_reported_case() {
        fn row(path: &str, base: Option<&str>, before: &str, after: &str) -> TagFieldDiff {
            TagFieldDiff {
                path: path.to_owned(),
                base_path: base.map(str::to_owned),
                a: before.to_owned(),
                b: after.to_owned(),
            }
        }
        let mut rows = vec![
            row("flags", Some("flags"), "0x0000 (none set)", "0x000E [...]"),
            row(
                "sandbox origin point",
                Some("sandbox origin point"),
                "x=0, y=-0, z=0",
                "x=1, y=2, z=3",
            ),
            row(
                "zone set pvs[3]",
                Some("zone set pvs[3]"),
                "removed — element 3",
                "",
            ),
        ];
        // The removed element's own fields follow it, and must fold into it.
        for field in ["structure bsp mask", "version"] {
            let path = format!("zone set pvs[3]/{field}");
            rows.push(row(&path, Some(&path), "11", ""));
        }
        rows.push(row("zone sets[5]", None, "", "added — element 5"));
        for field in ["cinematic zones", "hint previous zone set"] {
            rows.push(row(&format!("zone sets[5]/{field}"), None, "", "0"));
        }

        let sections = Baboon::build_diff_sections(&rows);
        let kinds: Vec<_> = sections
            .iter()
            .map(|s| (s.element.as_str(), s.kind, s.rows.len()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("", ModExportChange::Modified, 2),
                ("zone set pvs[3]", ModExportChange::Unresolved, 3),
                ("zone sets[5]", ModExportChange::New, 3),
            ],
        );

        let tree = Baboon::build_diff_tree(sections);
        // The two top-level field changes stay at the root; each block holds
        // only its own changed element.
        assert_eq!(tree.sections.len(), 1);
        assert_eq!(tree.sections[0].element, "");
        let containers: Vec<_> = tree
            .children
            .iter()
            .map(|c| (c.title.as_str(), c.sections.len(), c.children.len()))
            .collect();
        assert_eq!(
            containers,
            vec![("zone set pvs", 1, 0), ("zone sets", 1, 0)],
        );
    }

    /// A change buried several blocks deep reads as one breadcrumb, not a stack
    /// of boxes each containing only the next.
    #[test]
    fn single_child_container_chains_collapse() {
        let section = DiffSection {
            element: "structure bsp pvs[0]/cluster pvs[0]/cluster pvs bit vectors[0]".to_owned(),
            base_element: None,
            label: String::new(),
            kind: ModExportChange::Modified,
            rows: Vec::new(),
            filter: Default::default(),
        };
        let tree = Baboon::build_diff_tree(vec![section]);
        assert_eq!(tree.children.len(), 1);
        let chain = &tree.children[0];
        // Intermediate containers keep their element index -- it is the only
        // place that says *which* cluster the change is in. The innermost one
        // drops it because the section row states it.
        assert_eq!(
            chain.title,
            "structure bsp pvs[0] › cluster pvs[0] › cluster pvs bit vectors",
        );
        assert_eq!(chain.sections.len(), 1);
        assert!(chain.children.is_empty());
    }

    fn dialog(name: &str) -> ModExportDialog {
        ModExportDialog {
            kit: KitId(0),
            review_only: false,
            snapshot: CampaignProjectSnapshot {
                game: "haloce_evolved".to_owned(),
                source_path: PathBuf::new(),
                selected_identity: None,
                tabs: Vec::new(),
                overlays: Default::default(),
                history: Default::default(),
                folders: Default::default(),
            },
            rows: Vec::new(),
            name: name.to_owned(),
            folder: PathBuf::from("/tmp"),
            overwrite_acknowledged: false,
            expanded: Default::default(),
            diffs: Default::default(),
            controls_height: 0.0,
        }
    }

    /// `_P` is what gives a mod priority over the game's own containers, so it
    /// is part of the name rather than something a rename can drop -- which is
    /// exactly how a reported mod came to build correctly and do nothing.
    #[test]
    fn the_stem_always_carries_the_priority_suffix() {
        assert_eq!(dialog("h2a_magnum").stem(), "h2a_magnum_P");
        assert_eq!(dialog("h2a_magnum_P").stem(), "h2a_magnum_P");
        assert_eq!(dialog("  spaced  ").stem(), "spaced_P");
    }

    /// A heading names the block and the element within it, rather than an
    /// indexed path the reader has to parse.
    #[test]
    fn an_element_path_splits_into_its_block_and_index() {
        assert_eq!(
            Baboon::split_element_index("zone set pvs[3]"),
            ("zone set pvs", Some(3))
        );
        // Nested: the chain stays, so it is clear which block is meant.
        assert_eq!(
            Baboon::split_element_index("weapons[2]/triggers[0]"),
            ("weapons[2]/triggers", Some(0))
        );
        // Not an element at all.
        assert_eq!(Baboon::split_element_index("flags"), ("flags", None));
    }

    /// Changes nest, and the innermost element is the one worth heading. The
    /// whole chain is kept so it is unambiguous which element that is.
    #[test]
    fn a_diff_path_splits_into_its_element_and_field() {
        assert_eq!(
            Baboon::split_element_path("weapons[2]/triggers[0]/barrels[1]/damage"),
            ("weapons[2]/triggers[0]/barrels[1]", "damage")
        );
        // A row about the element itself -- added, removed or moved -- has no
        // field part, which is how the renderer tells the two apart.
        assert_eq!(
            Baboon::split_element_path("vehicle palette[3]"),
            ("vehicle palette[3]", "")
        );
        // A field at the top level of the tag belongs to no element.
        assert_eq!(Baboon::split_element_path("flags"), ("", "flags"));
    }

    /// The name becomes three file names in a folder the user never types, so
    /// spaces and punctuation are separators to normalise, not characters to
    /// carry through -- while the user's own capitalisation is theirs to keep.
    #[test]
    fn a_mod_name_becomes_a_file_safe_stem() {
        assert_eq!(sanitize_mod_name("My Cool Mod"), "My-Cool-Mod");
        assert_eq!(sanitize_mod_name("h2a magnum!"), "h2a-magnum");
        assert_eq!(sanitize_mod_name("  trimmed  "), "trimmed");
        // Path syntax cannot survive: these become three files somewhere the
        // user did not choose.
        assert_eq!(sanitize_mod_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_mod_name("my:mod?"), "my-mod");
        // Underscores stay, so `_P` keeps meaning what it means.
        assert_eq!(sanitize_mod_name("my_mod_P"), "my_mod_P");
    }

    /// The buffer holds what the user typed; only the file name is folded.
    /// Folding as they type ate the space in "My Mod" before the second word
    /// could be reached.
    #[test]
    fn a_name_is_folded_only_when_it_becomes_a_file_name() {
        assert_eq!(dialog("My Mod").stem(), "My-Mod_P");
        assert_eq!(dialog("My ").stem(), "My_P");
        // Already suffixed, in either case the game accepts.
        assert_eq!(dialog("thing_P").stem(), "thing_P");
        assert_eq!(dialog("thing_p").stem(), "thing_p");
    }
}

#[cfg(test)]
mod diff_view_tests {
    use super::*;

    thread_local! {
        pub(super) static TREES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    fn row(path: &str, a: &str, b: &str) -> TagFieldDiff {
        TagFieldDiff {
            path: path.to_owned(),
            base_path: None,
            a: a.to_owned(),
            b: b.to_owned(),
        }
    }

    fn rows() -> Vec<TagFieldDiff> {
        vec![
            row("flags", "0", "1"),
            row("zone sets[2]", "old", "new"),
            row("zone sets[2]/name", "a", "b"),
            row("zone sets[2]/bsp flags", "0", "4"),
            row("zone set pvs[3]", "pvs", ""),
            row("zone set pvs[3]/bsp mask", "1", ""),
        ]
    }

    /// Each section's filter is built once its rows are complete, and is the
    /// filter those rows describe.
    #[test]
    fn each_section_keeps_the_filter_its_rows_describe() {
        let sections = Baboon::build_diff_sections(&rows());
        assert!(sections.len() >= 3);
        for section in &sections {
            assert_eq!(
                section.filter.visible_paths,
                Baboon::diff_field_filter(&section.rows).visible_paths,
                "{}",
                section.element
            );
        }
    }

    /// The review draws a tag's diff every frame; its tree is built on the
    /// first and reused after.
    #[test]
    fn a_reviewed_diff_is_arranged_once() {
        let diff = ModRowDiff {
            rows: rows(),
            base: None,
            edited: None,
            truncated: false,
            error: None,
            view: Default::default(),
        };
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::foundation_fonts());
        TREES_BUILT.with(|built| built.set(0));
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    Baboon::draw_mod_export_diff(
                        ui,
                        &diff,
                        &TagNameIndex::default(),
                        u32::from_be_bytes(*b"scnr"),
                        None,
                        None,
                        false,
                        "review",
                    );
                });
            });
        }
        assert_eq!(TREES_BUILT.with(std::cell::Cell::get), 1);
    }
}
