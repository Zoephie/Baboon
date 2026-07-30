//! Modal dialogs for rename, paste, keyword selection, and new-tag workflows.
//! It owns immediate-mode presentation and request collection; tag mutation, persistence, and source I/O belong to their owning subsystems.

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
}

/// A container in the tag, holding whatever changed inside it.
#[derive(Default)]
struct DiffNode {
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
            self.title = format!("{} \u{203a} {}", self.title, child.title);
            self.children = child.children;
            self.sections = child.sections;
        }
    }
}

impl Baboon {
    pub(super) fn draw_tag_conversion_window(&mut self, ctx: &egui::Context) {
        if !self.expert_mode {
            self.tag_conversion_dialog = None;
            return;
        }
        if self.tag_conversion_dialog.is_none() {
            return;
        }

        let mut open = true;
        let mut analyze = false;
        let mut choose_and_save = false;
        let mut confirm_inside_source = false;
        let mut cancel_inside_source = false;
        {
            let dialog = self.tag_conversion_dialog.as_mut().expect("checked above");
            egui::Window::new("Save Tag for Another Game")
                .id(egui::Id::new("tag_conversion"))
                .open(&mut open)
                .default_width(680.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Source tag").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&dialog.source_label)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Source profile").color(subtle_dark()));
                        ui.label(RichText::new(&dialog.source_game).color(text_dark()));
                    });

                    let previous_target = dialog.target_game.clone();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Target profile").color(subtle_dark()));
                        egui::ComboBox::from_id_salt("tag_conversion_target")
                            .selected_text(&dialog.target_game)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                for game in CONVERSION_GAMES {
                                    if *game != dialog.source_game {
                                        ui.selectable_value(
                                            &mut dialog.target_game,
                                            (*game).to_owned(),
                                            *game,
                                        );
                                    }
                                }
                            });
                    });
                    if dialog.target_game != previous_target {
                        dialog.draft = None;
                        dialog.error = None;
                        dialog.pending_source_destination = None;
                    }

                    ui.add_space(8.0);
                    if ui.button("Analyze Conversion").clicked() {
                        analyze = true;
                    }

                    if let Some(draft) = dialog.draft.as_ref() {
                        let report = &draft.report;
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Target: {} (.{}), group {}",
                                dialog.target_game,
                                draft.target_extension,
                                draft.target_group_name
                            ))
                            .color(text_dark())
                            .strong(),
                        );
                        egui::Grid::new("tag_conversion_summary")
                            .num_columns(2)
                            .spacing([20.0, 3.0])
                            .show(ui, |ui| {
                                ui.label("Copied exactly");
                                ui.label(report.copied_exact.to_string());
                                ui.end_row();
                                ui.label("Converted semantically");
                                ui.label(report.converted_semantic.to_string());
                                ui.end_row();
                                ui.label("Mapped through schema/catalog aliases");
                                ui.label(report.mapped_aliases.to_string());
                                ui.end_row();
                                ui.label("Target fields left at defaults");
                                ui.label(report.defaulted_target.to_string());
                                ui.end_row();
                                ui.label("Unsupported source values");
                                ui.label(report.unsupported_source.to_string());
                                ui.end_row();
                                ui.label("Truncated elements");
                                ui.label(report.truncated.to_string());
                                ui.end_row();
                            });

                        if !report.issues.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Conversion details")
                                    .color(subtle_dark())
                                    .small(),
                            );
                            egui::ScrollArea::vertical()
                                .id_salt("tag_conversion_issues")
                                .max_height(230.0)
                                .show(ui, |ui| {
                                    for issue in &report.issues {
                                        let kind = match issue.kind {
                                            ConversionIssueKind::Unsupported => "Unsupported",
                                            ConversionIssueKind::Truncated => "Truncated",
                                            ConversionIssueKind::Warning => "Warning",
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{kind}: {} — {}",
                                                issue.path, issue.message
                                            ))
                                            .color(subtle_dark())
                                            .small(),
                                        );
                                    }
                                });
                        }
                    }

                    if let Some(error) = dialog.error.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(error).color(material_delete_text()));
                    }

                    if let Some(output) = dialog.pending_source_destination.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(
                                "This destination is inside the currently loaded source tags folder. The converted tag uses a different profile and will not be added to the current browser.",
                            )
                            .color(material_delete_text()),
                        );
                        ui.label(
                            RichText::new(output.display().to_string())
                                .monospace()
                                .small()
                                .color(subtle_dark()),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Save There Anyway").clicked() {
                                confirm_inside_source = true;
                            }
                            if ui.button("Choose Another Location").clicked() {
                                cancel_inside_source = true;
                                choose_and_save = true;
                            }
                        });
                    } else {
                        ui.add_space(10.0);
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add_enabled(
                                        dialog.draft.is_some(),
                                        egui::Button::new("Choose Location and Save..."),
                                    )
                                    .clicked()
                                {
                                    choose_and_save = true;
                                }
                                ui.label(
                                    RichText::new(
                                        "Saving creates a new copy; the source tag is not modified.",
                                    )
                                    .color(subtle_dark())
                                    .small(),
                                );
                            },
                        );
                    }
                });
        }

        if analyze {
            self.analyze_tag_conversion();
        }
        if cancel_inside_source {
            if let Some(dialog) = self.tag_conversion_dialog.as_mut() {
                dialog.pending_source_destination = None;
            }
        }
        if confirm_inside_source {
            self.confirm_tag_conversion_inside_source();
        } else if choose_and_save {
            self.choose_tag_conversion_destination();
        }
        if !open {
            self.tag_conversion_dialog = None;
        }
    }

    pub(super) fn draw_folder_conversion_window(&mut self, ctx: &egui::Context) {
        if !self.expert_mode
            && self
                .folder_conversion_dialog
                .as_ref()
                .is_none_or(|dialog| !dialog.running)
        {
            self.folder_conversion_dialog = None;
            return;
        }
        if self.folder_conversion_dialog.is_none() {
            return;
        }
        let mut open = true;
        let mut choose_destination = false;
        let mut start = false;
        let running;
        {
            let dialog = self
                .folder_conversion_dialog
                .as_mut()
                .expect("checked above");
            running = dialog.running;
            egui::Window::new("Save Folder for Another Game")
                .id(egui::Id::new("folder_conversion"))
                .open(&mut open)
                .default_width(760.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Source folder").color(subtle_dark()).small());
                    ui.label(RichText::new(&dialog.source_label).monospace().color(text_dark()));
                    ui.horizontal(|ui| {
                        ui.label(format!("Source profile: {}", dialog.source_game));
                        ui.label("Target profile:");
                        let previous = dialog.target_game.clone();
                        ui.add_enabled_ui(!dialog.running, |ui| {
                            egui::ComboBox::from_id_salt("folder_conversion_target")
                                .selected_text(&dialog.target_game)
                                .show_ui(ui, |ui| {
                                    for game in CONVERSION_GAMES {
                                        if *game != dialog.source_game {
                                            ui.selectable_value(
                                                &mut dialog.target_game,
                                                (*game).to_owned(),
                                                *game,
                                            );
                                        }
                                    }
                                });
                        });
                        if dialog.target_game != previous {
                            dialog.destination_parent = None;
                            dialog.report = None;
                            dialog.error = None;
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!dialog.running, egui::Button::new("Choose Destination..."))
                            .clicked()
                        {
                            choose_destination = true;
                        }
                        if let Some(destination) = dialog.destination_parent.as_ref() {
                            ui.label(
                                RichText::new(format!(
                                    "{}\\{}",
                                    destination.display(),
                                    dialog.source_label
                                ))
                                .monospace()
                                .small()
                                .color(subtle_dark()),
                            );
                        }
                    });
                    ui.label(
                        RichText::new(
                            "Existing destination tags are replaced atomically. Reference paths are not relocated.",
                        )
                        .small()
                        .color(subtle_dark()),
                    );

                    if let Some(progress) = dialog.progress.as_ref() {
                        ui.add_space(8.0);
                        let fraction = if progress.total == 0 {
                            0.0
                        } else {
                            progress.processed as f32 / progress.total as f32
                        };
                        ui.label(RichText::new(&progress.phase).strong());
                        ui.add(
                            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                                .animate(progress.total == 0)
                                .text(format!(
                                    "{} / {} — {} converted, {} failed",
                                    progress.processed,
                                    progress.total,
                                    progress.converted,
                                    progress.failed
                                )),
                        );
                        if !progress.current.is_empty() {
                            ui.label(
                                RichText::new(&progress.current)
                                    .monospace()
                                    .small()
                                    .color(subtle_dark()),
                            );
                        }
                        ctx.request_repaint();
                    }

                    if let Some(report) = dialog.report.as_ref() {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Completed: {} native-layout, {} generated-layout, {} failed, {} ignored",
                                report.native_count(),
                                report.generated_count(),
                                report.failed_count(),
                                report.ignored_files.len()
                            ))
                            .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} -> {} | Destination: {}",
                                report.source_label,
                                report.target_game,
                                report.destination_root.display()
                            ))
                            .monospace()
                            .small(),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("folder_conversion_results")
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for wanted in [
                                    FolderConversionFileStatus::NativeLayout,
                                    FolderConversionFileStatus::GeneratedLayout,
                                    FolderConversionFileStatus::Failed,
                                ] {
                                    let (label, color) = match wanted {
                                        FolderConversionFileStatus::NativeLayout => {
                                            ("Native-layout verified", text_dark())
                                        }
                                        FolderConversionFileStatus::GeneratedLayout => (
                                            "Generated layout — native compatibility unverified",
                                            material_delete_text(),
                                        ),
                                        FolderConversionFileStatus::Failed => {
                                            ("Failed / skipped", material_delete_text())
                                        }
                                    };
                                    let matching = report
                                        .files
                                        .iter()
                                        .filter(|file| file.status == wanted)
                                        .collect::<Vec<_>>();
                                    if matching.is_empty() {
                                        continue;
                                    }
                                    ui.collapsing(
                                        RichText::new(format!("{label} ({})", matching.len()))
                                            .color(color),
                                        |ui| {
                                            for file in matching {
                                                let replaced = if file.overwritten {
                                                    " [replaced]"
                                                } else {
                                                    ""
                                                };
                                                let output = file
                                                    .output
                                                    .as_ref()
                                                    .map(|path| format!(" -> {}", path.display()))
                                                    .unwrap_or_default();
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{}{}{} — {}",
                                                        file.source, output, replaced, file.detail
                                                    ))
                                                    .small()
                                                    .color(color),
                                                );
                                            }
                                        },
                                    );
                                }
                                if !report.ignored_files.is_empty() {
                                    ui.collapsing(
                                        format!("Ignored non-tag files ({})", report.ignored_files.len()),
                                        |ui| {
                                            for path in &report.ignored_files {
                                                ui.label(RichText::new(path).monospace().small());
                                            }
                                        },
                                    );
                                }
                            });
                    }
                    if let Some(error) = dialog.error.as_ref() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(error).color(material_delete_text()));
                    }
                    ui.add_space(8.0);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add_enabled(
                                    !dialog.running && dialog.destination_parent.is_some(),
                                    egui::Button::new("Convert Folder"),
                                )
                                .clicked()
                            {
                                start = true;
                            }
                        },
                    );
                });
        }
        if choose_destination {
            self.choose_folder_conversion_destination();
        }
        if start {
            self.begin_folder_conversion();
        }
        if !open && !running {
            self.folder_conversion_dialog = None;
        }
    }

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
                if row.b.is_empty() { row.a.clone() } else { row.b.clone() }
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
                && matches!(last.kind, ModExportChange::New | ModExportChange::Unresolved)
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
                }),
            }
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
            for path in [Some(&row.path), row.base_path.as_ref()].into_iter().flatten() {
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
        filter: &FieldFilter,
        names: &TagNameIndex,
        group_tag: u32,
        game: Option<&str>,
        definitions_root: Option<&Path>,
        expert_mode: bool,
        scope: &str,
    ) {
        // The editor collects deferred edits as it draws. Nothing here is
        // editable, so they are collected into locals and dropped.
        let mut buffers = EditDrafts::default();
        let mut pending = Vec::new();
        let mut block_ops = Vec::new();
        let mut block_confirm = None;
        let mut open_request = None;
        let mut sound_play_request = None;
        let mut sound_extract_request = None;
        let mut tool_import = None;
        let mut bitmap_reimport = None;
        let mut shader_ops = Vec::new();
        let mut shader_param_ops = Vec::new();
        let mut h2_shader_param_ops = Vec::new();
        let mut function_data_ops = Vec::new();
        let mut model_variant_ops = Vec::new();
        let mut color_request = None;
        let mut function_request = None;
        let mut block_clip_request = None;
        let mut tsv_paste_request = None;
        let mut tag_reference_picker = None;
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
        let filter_action = FieldFilterAction::Apply((*filter).clone());
        let mut edit = FieldEditContext {
            expand_all: Some(true),
            nested_default: NestedDefault::Expanded,
            view_scope: scope,
            tag_key: scope,
            group_tag,
            root: Some(root),
            game,
            definitions_root,
            names: Some(names),
            tags_root: None,
            tag_reference_catalog: None,
            tag_reference_picker: &mut tag_reference_picker,
            status: None,
            editable: false,
            show_block_sizes: false,
            buffers: &mut buffers,
            pending: &mut pending,
            block_ops: &mut block_ops,
            block_confirm: &mut block_confirm,
            open_request: &mut open_request,
            sound_play_request: &mut sound_play_request,
            sound_status: None,
            sound_volume: 1.0,
            sound_extract_request: &mut sound_extract_request,
            sound_language: None,
            ce_sound: None,
            ce_sound_ref_request: &mut None,
            ce_paks_root: None,
            tool_import: &mut tool_import,
            bitmap_reimport: &mut bitmap_reimport,
            shader_ops: &mut shader_ops,
            shader_param_ops: &mut shader_param_ops,
            h2_shader_param_ops: &mut h2_shader_param_ops,
            function_data_ops: &mut function_data_ops,
            model_variant_ops: &mut model_variant_ops,
            color_request: &mut color_request,
            function_request: &mut function_request,
            block_clipboard: None,
            docs: None,
            tsv_paste_request: &mut tsv_paste_request,
            block_clip_request: &mut block_clip_request,
            field_filter: Some(&filter_action),
            field_nav: None,
        };
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
        let tree = Self::build_diff_tree(Self::build_diff_sections(&diff.rows));
        Self::draw_diff_node(
            ui,
            &tree,
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
            rows: section_rows,
        } = section;
        let (kind, element, base_element, label) = (*kind, element.clone(), base_element.clone(), label.clone());
        let filter = Self::diff_field_filter(section_rows);
        ui.add_space(6.0);
        if !element.is_empty() {
            // `Unresolved` stands in for "gone", the only way an element leaves.
            let (marker, heading) = match kind {
                ModExportChange::New => ("+", added_text()),
                ModExportChange::Unresolved => ("-", removed_text()),
                ModExportChange::Modified => ("~", modified_text()),
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
                        .split_once(" \u{2014} ")
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
                            .stroke(Stroke::new(1.0, accent.gamma_multiply(0.5)))
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
                                                &filter,
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
                ModExportChange::Modified => {
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
    pub(super) fn draw_mod_export_window(&mut self, ctx: &egui::Context) {
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
        let stem = dialog.stem();
        let existing = dialog.existing_files();
        let in_game_folder = self
            .kits
            .iter()
            .find(|k| k.id == kit)
            .and_then(|k| k.source.as_ref())
            .map(|source| source.source.root_path() == dialog.folder)
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
        let expert_mode = self.expert_mode;
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
                        RichText::new(format!("{stem}.utoc / .ucas / .pak"))
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Folder").color(text_dark()));
                    ui.label(
                        RichText::new(dialog.folder.display().to_string())
                            .color(subtle_dark())
                            .monospace()
                            .small(),
                    );
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });
                if in_game_folder {
                    ui.label(
                        RichText::new("This is the game's own Paks folder — nothing to copy afterwards.")
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
                    format!("Wrote a diagnostic for {count} tag(s) to {}", folder.display())
                }
                Err(error) => error,
            };
        }
        if browse
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Export mod into folder")
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
            let included: HashSet<String> = dialog
                .included()
                .map(|row| row.identity.clone())
                .collect();
            let output = dialog.folder.join(format!("{}.utoc", dialog.stem()));
            let snapshot = dialog.snapshot.clone();
            // The workspace may have been closed while this was open.
            if self.focus_navigation_kit(kit) {
                self.write_reviewed_mod(&snapshot, &included, output);
            }
            self.mod_export = None;
        }
    }

    /// What to do with a mod that was just exported.
    ///
    /// A mod is three files and only the `.pak` looks like one, so copying that
    /// alone -- the obvious thing to do -- produces a mod the game finds and
    /// then has nothing to load. The instruction used to live in the status
    /// line, was lost when exports moved onto projects, and the status line now
    /// clears itself after a few seconds besides.
    pub(super) fn draw_exported_mod_window(&mut self, ctx: &egui::Context) {
        let Some(exported) = self.exported_mod.as_ref() else {
            return;
        };
        let stem = exported.stem.clone();
        let directory = exported.directory.clone();
        let count = exported.count;
        let skipped = exported.skipped;
        let mut open = true;
        let mut close = false;
        let mut reveal = false;
        egui::Window::new("Mod exported")
            .id(egui::Id::new("exported_mod"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("{count} tag(s) exported. The base game is unchanged."))
                        .color(text_dark()),
                );
                if skipped > 0 {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{skipped} tag(s) in this workspace's project could not be resolved \
                             and are NOT in this mod."
                        ))
                        .color(egui::Color32::from_rgb(210, 120, 90)),
                    );
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Copy all three files into the game:").color(text_dark()));
                ui.add_space(4.0);
                for extension in ["utoc", "ucas", "pak"] {
                    ui.label(
                        RichText::new(format!("    {stem}.{extension}"))
                            .color(text_dark())
                            .monospace(),
                    );
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("    Meteorite/Content/Paks/")
                        .color(text_dark())
                        .monospace(),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "All three are needed. The .pak is the only one the game scans for, but \
                         the tag data is in the .ucas -- copying it alone loads nothing.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "If you rename them, keep the _P suffix. It is what gives the mod \
                         priority over the game's own files; without it the mod is ignored.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if !directory.as_os_str().is_empty()
                        && ui.button("Show in file browser").clicked()
                    {
                        reveal = true;
                    }
                    if ui.button("Done").clicked() {
                        close = true;
                    }
                });
            });
        if reveal {
            self.open_folder_in_explorer(directory, "mod");
        }
        if !open || close {
            self.exported_mod = None;
        }
    }

    /// Confirmation for the Campaign Evolved "clear modifications" action.
    ///
    /// It is irreversible and can drop work stashed in earlier sessions, so it
    /// lists exactly what is about to go rather than asking in the abstract.
    pub(super) fn draw_clear_stash_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.clear_stash_confirm.as_ref() else {
            return;
        };
        let kit = confirm.kit;
        let stashed = confirm.stashed.clone();
        let unsaved = confirm.unsaved;
        let mut open = true;
        let mut do_clear = false;
        let mut cancel = false;
        egui::Window::new("Clear unsaved modifications?")
            .id(egui::Id::new("clear_stash_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Every tag in this workspace goes back to the way the game ships it.",
                    )
                    .color(text_dark()),
                );
                ui.add_space(6.0);
                if unsaved > 0 {
                    let noun = if unsaved == 1 { "tag" } else { "tags" };
                    ui.label(
                        RichText::new(format!("{unsaved} open {noun} with unsaved edits"))
                            .color(text_dark()),
                    );
                }
                if !stashed.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{} tag(s) stashed in this workspace's project, including any kept \
                             from earlier sessions:",
                            stashed.len()
                        ))
                        .color(text_dark()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for path in &stashed {
                                ui.label(RichText::new(path).color(text_dark()).monospace().small(),);
                            }
                        });
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This cannot be undone. Tags already saved into the game's pak files, \
                         and mods you have already exported, are not affected.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Clear Modifications").clicked() {
                        do_clear = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.clear_stash_confirm = None;
        } else if do_clear {
            self.clear_stash_confirm = None;
            // Resolved rather than assumed: the workspace may have been closed
            // while the confirmation was up.
            if let Some(index) = self.resolve_kit(kit) {
                self.clear_campaign_stash(index, ctx);
            }
        }
    }

    pub(super) fn draw_overwrite_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((kit, key)) = self
            .overwrite_confirm
            .as_ref()
            .map(|confirm| (confirm.kit, confirm.key.clone()))
        else {
            return;
        };
        let mut open = true;
        let mut do_overwrite = false;
        let mut do_export = false;
        let mut cancel = false;
        let mut dont_ask = !self.confirm_container_overwrite;
        // Which container this would actually be written into. With a mod mounted
        // over the tag, that is the mod — not the game's shipped pak, which is
        // what this dialog used to promise in every case.
        let target = self
            .resolve_kit(kit)
            .and_then(|index| self.container_label_for_tag(index, &key));
        egui::Window::new("Overwrite game files?")
            .id(egui::Id::new("overwrite_confirm"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(match target.as_ref() {
                        Some((label, true)) => format!(
                            "Save will overwrite this tag inside the mounted mod {label}, in place:"
                        ),
                        Some((label, false)) => format!(
                            "Save will overwrite this tag inside the game's shipped pak {label}, \
                             in place:"
                        ),
                        None => "Save will overwrite this tag inside the game's shipped pak files, \
                                 in place:"
                            .to_owned(),
                    })
                    .color(text_dark()),
                );
                ui.add_space(5.0);
                ui.label(RichText::new(&key).color(text_dark()).monospace());
                ui.add_space(9.0);
                ui.label(
                    RichText::new(
                        "This modifies the original game content and cannot be undone without a backup of the pak files.",
                    )
                    .color(egui::Color32::from_rgb(210, 120, 90)),
                );
                ui.add_space(5.0);
                ui.label(
                    RichText::new(
                        "To keep the base game untouched, cancel and use File \u{2192} Export Mod\u{2026} instead — it bundles your changes into a separate mod overlay.",
                    )
                    .color(subtle_dark())
                    .small(),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut dont_ask,
                    "Don't ask again (changeable in Settings)",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite Game Files").clicked() {
                        do_overwrite = true;
                    }
                    if ui.button("Export Mod Instead\u{2026}").clicked() {
                        do_export = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.overwrite_confirm = None;
        } else if do_overwrite {
            self.overwrite_confirm = None;
            // Apply the opt-out only when the user commits to the overwrite.
            if dont_ask && self.confirm_container_overwrite {
                self.confirm_container_overwrite = false;
                self.persist_prefs_if_changed();
            }
            // Both actions write through the active kit's source. Return to the
            // workspace this was raised from, and drop it if that workspace has
            // been closed — overwriting the game's paks in place is the last
            // thing that should land on whichever game is focused by now.
            if self.focus_navigation_kit(kit) {
                self.overwrite_current_tag_in_place(&key);
            }
        } else if do_export {
            self.overwrite_confirm = None;
            if self.focus_navigation_kit(kit) {
                self.export_mod();
            }
        }
    }

    pub(super) fn draw_rename_tag_window(&mut self, ctx: &egui::Context) {
        if self.rename_tag.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        let mut cancel = false;
        {
            let state = self.rename_tag.as_mut().expect("checked above");
            let title = match (state.is_new_container, state.redirect) {
                (true, true) => "Rename / Move New Tag",
                (true, false) => "Copy New Tag",
                (false, false) if state.is_container => "Save Tag As (New Copy)",
                _ => "Rename Tag",
            };
            egui::Window::new(title)
                .id(egui::Id::new("rename_tag"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(RichText::new("Current path").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(&state.old_display)
                            .color(text_dark())
                            .monospace(),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if state.is_new_container {
                            "New path (folders allowed; extension is fixed)"
                        } else {
                            "New name (extension is fixed)"
                        })
                        .color(subtle_dark())
                        .small(),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.new_path_input)
                                .desired_width(430.0)
                                .font(egui::TextStyle::Monospace),
                        );
                        ui.label(
                            RichText::new(format!(".{}", state.extension)).color(subtle_dark()),
                        );
                    });
                    // A new tag edits its whole path, so it keeps no parent from
                    // the old one — what is typed IS the destination.
                    let preview_parent = if state.is_new_container {
                        ""
                    } else {
                        state
                            .old_display
                            .rsplit_once('/')
                            .map(|(parent, _)| parent)
                            .unwrap_or("")
                    };
                    let preview_name = state.new_path_input.trim();
                    let preview = if preview_name.is_empty() {
                        "(enter a new name)".to_owned()
                    } else if preview_parent.is_empty() {
                        format!("{preview_name}.{}", state.extension)
                    } else {
                        format!("{preview_parent}/{preview_name}.{}", state.extension)
                    };
                    ui.add_space(3.0);
                    ui.label(RichText::new("Preview").color(subtle_dark()).small());
                    ui.label(
                        RichText::new(preview)
                            .color(text_dark())
                            .monospace()
                            .small(),
                    );
                    ui.add_space(8.0);
                    if state.is_new_container {
                        ui.label(
                            RichText::new(if state.redirect {
                                "This tag has not been saved yet, so it simply moves to the new \
                                 path — nothing is written."
                            } else {
                                "Creates a second unsaved tag with a copy of this one's contents."
                            })
                            .color(text_dark()),
                        );
                        ui.label(
                            RichText::new(if state.redirect {
                                "It is written when you Save it or Export Mod."
                            } else {
                                "Both tags are written only when you Save them or Export Mod."
                            })
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.is_container {
                        if state.redirect {
                            ui.label(
                                RichText::new(
                                    "Existing references will be redirected to the new tag via the \
                                     overlay container.",
                                )
                                .color(text_dark()),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "Writes an independent new tag; existing references are \
                                     unchanged.",
                                )
                                .color(text_dark()),
                            );
                        }
                        ui.label(
                            RichText::new(
                                "A higher-priority overlay container is written; base game files \
                                 are never modified.",
                            )
                            .color(subtle_dark())
                            .small(),
                        );
                    } else if state.referrers_unavailable {
                        ui.label(
                            RichText::new(
                                "Reference index unavailable — references are still rewritten on \
                                 apply, but can't be previewed here.",
                            )
                            .color(subtle_dark()),
                        );
                    } else if state.referrers.is_empty() {
                        ui.label(
                            RichText::new("No other tags reference this tag.").color(subtle_dark()),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} referring tag(s) will be updated:",
                                state.referrers.len()
                            ))
                            .color(text_dark()),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("rename_referrers")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for referrer in &state.referrers {
                                    ui.label(RichText::new(referrer).color(subtle_dark()).small());
                                }
                            });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !state.new_path_input.trim().is_empty(),
                                egui::Button::new("Apply"),
                            )
                            .on_hover_text(if state.is_new_container && state.redirect {
                                "Move this unsaved tag to the new path (nothing is written yet)"
                            } else if state.is_new_container {
                                "Copy this unsaved tag to the new path (nothing is written yet)"
                            } else if state.is_container {
                                "Write a higher-priority overlay container (base game unchanged)"
                            } else {
                                "Move the file on disk and rewrite all references"
                            })
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
        }
        if do_apply {
            // begin_rename_tag clears `rename_tag` on success; on a validation
            // error it leaves the dialog open with a status message.
            self.begin_rename_tag();
        }
        if cancel || !open {
            self.rename_tag = None;
        }
    }

    /// TSV import window: the user pastes tab-separated rows (header = field
    /// names) and applies them onto the target block's existing elements.
    pub(super) fn draw_tsv_paste_window(&mut self, ctx: &egui::Context) {
        if self.tsv_paste.is_none() {
            return;
        }
        let mut open = true;
        let mut do_apply = false;
        {
            let paste = self.tsv_paste.as_mut().expect("checked above");
            egui::Window::new(format!("Paste TSV → {}", paste.block_label))
                .id(egui::Id::new("tsv_paste"))
                .open(&mut open)
                .default_width(560.0)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Paste tab-separated rows (first row = field names) to overwrite \
                             this block's {} element(s), cell by cell. Extra rows are ignored — \
                             add elements first if you need more.",
                            paste.element_count
                        ))
                        .color(subtle_dark()),
                    );
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut paste.text)
                                    .desired_rows(12)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("paste TSV here (Ctrl+V)"),
                            );
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!paste.text.trim().is_empty(), egui::Button::new("Apply"))
                            .clicked()
                        {
                            do_apply = true;
                        }
                        if let Some(status) = &paste.status {
                            ui.label(RichText::new(status).color(subtle_dark()));
                        }
                    });
                });
        }
        if do_apply {
            self.apply_tsv_paste();
        }
        if !open {
            self.tsv_paste = None;
        }
    }

    pub(super) fn draw_keyword_chooser_window(&mut self, ctx: &egui::Context) {
        if !self.keyword_chooser_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        let all = self.kits[self.active].keywords.all_keywords();
        egui::Window::new("Keywords")
            .id(egui::Id::new("keyword_chooser"))
            .open(&mut open)
            .default_width(280.0)
            .show(ctx, |ui| {
                if all.is_empty() {
                    ui.label(
                        RichText::new("No keywords yet — add them on a tag's Keywords bar.")
                            .color(subtle_dark()),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (keyword, count) in &all {
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{keyword}  ({count})"))
                                            .color(text_dark()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text("Show tags with this keyword")
                                .clicked()
                            {
                                chosen = Some(keyword.clone());
                            }
                        }
                    });
            });
        if let Some(keyword) = chosen {
            self.show_tags_with_keyword(&keyword);
        }
        self.keyword_chooser_open = open;
    }

    /// Reference-graph navigator: parents (referenced by) on the left, children
    /// (references) on the right, with the focused tag and back/forward history.
    pub(super) fn draw_new_tag_window(&mut self, ctx: &egui::Context) {
        if !self.new_tag_open {
            return;
        }

        let mut open = self.new_tag_open;
        let mut refresh_groups = false;
        let mut create = false;
        let mut close_requested = false;
        // Campaign Evolved container sources create the tag in memory (no loose
        // tags folder, no filesystem picker) at a container-relative path.
        let is_container = self.current_source_is_container();
        egui::Window::new("New Tag")
            .id(egui::Id::new("new_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                if !is_container && self.loaded_tags_root().is_none() {
                    ui.label(
                        RichText::new(
                            "Load a loose editing-kit tags folder before creating a tag.",
                        )
                        .color(subtle_dark()),
                    );
                    ui.add_space(8.0);
                }

                if is_container {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Game").color(subtle_dark()));
                        ui.label("Halo: Campaign Evolved");
                    });
                } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Game").color(subtle_dark()));
                    let before = self.new_tag_dialog.game.clone();
                    let games = crate::app::controller::available_definition_games();
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_game")
                            .selected_text(&self.new_tag_dialog.game)
                            .width(220.0),
                        |ui| {
                            for game in &games {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.game,
                                    game.clone(),
                                    game,
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = games
                            .iter()
                            .position(|game| game == &self.new_tag_dialog.game)
                            .unwrap_or(0);
                        if let Some(next) = combo_scroll_next_index(current, games.len(), delta) {
                            self.new_tag_dialog.game = games[next].clone();
                        }
                    }
                    if self.new_tag_dialog.game != before {
                        refresh_groups = true;
                    }
                });
                }

                let selected_group_before = self.new_tag_dialog.selected_group;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    let selected = self
                        .new_tag_dialog
                        .groups
                        .get(self.new_tag_dialog.selected_group)
                        .map(|group| {
                            format!("{} ({})", group.name, format_group_tag(group.group_tag))
                        })
                        .unwrap_or_else(|| "No schemas".to_owned());
                    let (_, wheel_delta) = combo_box_with_scroll(
                        ui,
                        egui::ComboBox::from_id_salt("new_tag_group")
                            .selected_text(selected)
                            .width(320.0),
                        |ui| {
                            for (index, group) in self.new_tag_dialog.groups.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.new_tag_dialog.selected_group,
                                    index,
                                    format!(
                                        "{} ({})",
                                        group.name,
                                        format_group_tag(group.group_tag)
                                    ),
                                );
                            }
                        },
                    );
                    if let Some(delta) = wheel_delta {
                        let current = self.new_tag_dialog.selected_group;
                        if let Some(next) = combo_scroll_next_index(
                            current,
                            self.new_tag_dialog.groups.len(),
                            delta,
                        ) {
                            self.new_tag_dialog.selected_group = next;
                        }
                    }
                });
                if self.new_tag_dialog.selected_group != selected_group_before {
                    // The container path is group-independent (the user types it);
                    // only the loose filesystem output path is tied to the group.
                    if !is_container {
                        self.new_tag_dialog.rel_path.clear();
                        self.new_tag_dialog.output_path = None;
                    }
                    self.new_tag_dialog.error = None;
                }

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Path").color(subtle_dark()));
                    if is_container {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_tag_dialog.rel_path)
                                // Prefixed, because a bare path here reads as a
                                // filled field: the placeholder looked exactly
                                // like a value someone had already typed, and
                                // Create sat disabled with no explanation.
                                .hint_text("e.g. objects/characters/foo/foo")
                                .desired_width(440.0),
                        );
                    } else {
                        let location = if self.new_tag_dialog.rel_path.is_empty() {
                            "No tag selected".to_owned()
                        } else {
                            self.new_tag_dialog.rel_path.clone()
                        };
                        let mut location_text = location;
                        ui.add_enabled(
                            false,
                            egui::TextEdit::singleline(&mut location_text).desired_width(360.0),
                        );
                        if ui
                            .add_enabled(
                                self.loaded_tags_root().is_some()
                                    && !self.new_tag_dialog.groups.is_empty(),
                                egui::Button::new("Choose..."),
                            )
                            .clicked()
                        {
                            self.choose_new_tag_output_path();
                        }
                    }
                });

                if let Some(group) = self
                    .new_tag_dialog
                    .groups
                    .get(self.new_tag_dialog.selected_group)
                {
                    let hint = if is_container {
                        format!(
                            "Creates a .{} tag in memory. Save writes a new override \
                             container; Export Mod bundles it. The base game is untouched.",
                            group.extension
                        )
                    } else {
                        format!(
                            "Creates a .{} tag relative to the loaded tags folder.",
                            group.extension
                        )
                    };
                    ui.label(RichText::new(hint).color(subtle_dark()).small());
                }

                if let Some(error) = &self.new_tag_dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                    let can_create = !self.new_tag_dialog.groups.is_empty()
                        && if is_container {
                            !self.new_tag_dialog.rel_path.trim().is_empty()
                        } else {
                            self.loaded_tags_root().is_some()
                                && self.new_tag_dialog.output_path.is_some()
                        };
                    if ui
                        .add_enabled(can_create, egui::Button::new("Create"))
                        .on_disabled_hover_text(if self.new_tag_dialog.groups.is_empty() {
                            "No tag groups are available for this game"
                        } else if is_container {
                            "Enter a path for the new tag"
                        } else if self.loaded_tags_root().is_none() {
                            "Load a loose editing-kit tags folder first"
                        } else {
                            "Choose where to save the new tag"
                        })
                        .clicked()
                    {
                        create = true;
                    }
                });
            });

        if refresh_groups {
            self.refresh_new_tag_groups();
        }
        if close_requested {
            open = false;
        }
        self.new_tag_open = open;
        if create {
            self.create_new_tag();
        }
    }

    pub(super) fn draw_import_tag_window(&mut self, ctx: &egui::Context) {
        if self.import_tag_dialog.is_none() {
            return;
        }
        // Snapshot fields for the immutable overwrite lookup before borrowing the
        // dialog mutably for rendering (the banner lags edits by one frame).
        let (folder_snapshot, name_snapshot, group_tag) = {
            let dialog = self.import_tag_dialog.as_ref().unwrap();
            (dialog.folder_rel.clone(), dialog.name.clone(), dialog.group_tag,)
        };
        let overwrite_logical =
            self.import_overwrite_target(&folder_snapshot, &name_snapshot, group_tag);

        let mut open = true;
        let mut do_import = false;
        let mut do_cancel = false;
        egui::Window::new("Import Tag")
            .id(egui::Id::new("import_tag_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let dialog = self.import_tag_dialog.as_mut().unwrap();

                ui.horizontal(|ui| {
                    ui.label(RichText::new("File").color(subtle_dark()));
                    ui.label(
                        dialog
                            .source_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("(unknown)"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Group").color(subtle_dark()));
                    ui.label(format!(
                        "{} ({})",
                        dialog.group_name,
                        format_group_tag(dialog.group_tag)
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Folder").color(subtle_dark()));
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.folder_rel)
                            .hint_text("objects/characters/foo (blank = root)")
                            .desired_width(440.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(subtle_dark()));
                    ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(440.0));
                });

                match &overwrite_logical {
                    Some(logical) => {
                        ui.label(
                            RichText::new(format!(
                                "⟳ Overwrites existing tag  {logical}.{}",
                                dialog.extension
                            ))
                            .color(Color32::from_rgb(242, 196, 48))
                            .small(),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("✦ New tag (no base-game counterpart)")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                }

                match &dialog.comparison {
                    Some(cmp) => match cmp.severity {
                        blam_tags::LayoutSeverity::Match => {
                            ui.label(
                                RichText::new(format!("✔ Schema matches {}", dialog.group_name))
                                    .color(disclosure_triangle_green())
                                    .small(),
                            );
                        }
                        blam_tags::LayoutSeverity::Drift => {
                            ui.label(
                                RichText::new(
                                    "⚠ Schema differs in field metadata only (wire layout \
                                     matches). Safe to import.",
                                )
                                .color(Color32::from_rgb(242, 196, 48))
                                .small(),
                            );
                            ui.checkbox(&mut dialog.import_anyway, "Import anyway");
                        }
                        blam_tags::LayoutSeverity::Incompatible => {
                            ui.label(
                                RichText::new(
                                    "✖ Schema is incompatible (group, version, or size differs) — \
                                     this tag does not match the base game.",
                                )
                                .color(material_delete_text())
                                .small(),
                            );
                        }
                    },
                    None => {
                        ui.label(
                            RichText::new("No shipped definition for this group — not validated.")
                                .color(subtle_dark())
                                .small(),
                        );
                    }
                }

                if let Some(error) = &dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).color(material_delete_text()));
                }

                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                    let blocked = matches!(
                        dialog.comparison.as_ref().map(|cmp| cmp.severity),
                        Some(blam_tags::LayoutSeverity::Incompatible)
                    ) || dialog.name.trim().is_empty();
                    if ui
                        .add_enabled(!blocked, egui::Button::new("Import"))
                        .clicked()
                    {
                        do_import = true;
                    }
                });
            });

        if !open {
            do_cancel = true;
        }
        if do_cancel {
            self.import_tag_dialog = None;
        } else if do_import {
            self.confirm_import_tag();
        }
    }

    pub(super) fn draw_import_discard_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.import_discard_confirm.as_ref() else {
            return;
        };
        let label = self.tag_path_label(&pending.target_key);
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new("Discard unsaved changes?")
            .id(egui::Id::new("import_discard_confirm"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{label} has unsaved edits. Replace it with the imported tag?"
                ));
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Discard & Replace").clicked() {
                        discard = true;
                    }
                });
            });
        if discard {
            self.apply_import_discard();
        } else if cancel {
            self.import_discard_confirm = None;
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
                "removed \u{2014} element 3",
                "",
            ),
        ];
        // The removed element's own fields follow it, and must fold into it.
        for field in ["structure bsp mask", "version"] {
            let path = format!("zone set pvs[3]/{field}");
            rows.push(row(&path, Some(&path), "11", ""));
        }
        rows.push(row("zone sets[5]", None, "", "added \u{2014} element 5"));
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
        };
        let tree = Baboon::build_diff_tree(vec![section]);
        assert_eq!(tree.children.len(), 1);
        let chain = &tree.children[0];
        // Intermediate containers keep their element index -- it is the only
        // place that says *which* cluster the change is in. The innermost one
        // drops it because the section row states it.
        assert_eq!(
            chain.title,
            "structure bsp pvs[0] \u{203a} cluster pvs[0] \u{203a} cluster pvs bit vectors",
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
        assert_eq!(
            Baboon::split_element_path("flags"),
            ("", "flags")
        );
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
