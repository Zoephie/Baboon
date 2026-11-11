//! Preview names first, then apply the reviewed batch through the shared refactor job.
use super::*;
use crate::app::shell::{FolderRefactorUiState, WorkerMessage};
use crate::app::tag_ops::{duplicate::validate_leaf_characters, folder_merge_ops};

#[derive(Clone)]
pub(super) struct RenameItem {
    pub path: PathBuf,
    pub stem: String,
    pub extension: Option<std::ffi::OsString>,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct RenameOptions {
    pub replacing: bool,
    pub pattern: String,
    pub find: String,
    pub replacement: String,
    pub case_sensitive: bool,
    pub start: u32,
    pub digits: u8,
}
impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            replacing: false,
            pattern: "{name}".into(),
            find: String::new(),
            replacement: String::new(),
            case_sensitive: true,
            start: 1,
            digits: 1,
        }
    }
}
struct PreviewRow {
    target: PathBuf,
    error: Option<String>,
    changed: bool,
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Substitutions never parse the inserted name again: a literal token in an
/// existing filename must remain a literal part of that filename.
fn template(
    pattern: &str,
    name: &str,
    ascending: u64,
    descending: u64,
    digits: u8,
) -> Result<String, String> {
    let mut rest = pattern;
    let mut output = String::new();
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("{{") {
            output.push('{');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("}}") {
            output.push('}');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{name}") {
            output.push_str(name);
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{n}") {
            output.push_str(&format!("{ascending:0width$}", width = digits as usize));
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("{N}") {
            output.push_str(&format!("{descending:0width$}", width = digits as usize));
            rest = tail;
        } else if rest.starts_with('{') {
            return Err("Use {name}, {n} or {N}; double braces for literal braces".into());
        } else {
            let character = rest.chars().next().unwrap();
            output.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    Ok(output)
}
fn occupied_names(items: &[RenameItem]) -> Result<HashSet<String>, String> {
    let parents: HashSet<_> = items.iter().filter_map(|item| item.path.parent()).collect();
    let mut occupied = HashSet::new();
    for parent in parents {
        for entry in fs::read_dir(parent).map_err(|e| e.to_string())? {
            occupied.insert(path_key(&entry.map_err(|e| e.to_string())?.path()));
        }
    }
    Ok(occupied)
}
fn replace_literal(input: &str, find: &str, replacement: &str, case_sensitive: bool) -> String {
    if find.is_empty() {
        return input.to_owned();
    }
    if case_sensitive {
        return input.replace(find, replacement);
    }
    let count = find.chars().count();
    let folded = find.to_lowercase();
    let mut rest = input;
    let mut result = String::new();
    while !rest.is_empty() {
        let end = rest
            .char_indices()
            .nth(count)
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        if rest[..end].chars().count() == count && rest[..end].to_lowercase() == folded {
            result.push_str(replacement);
            rest = &rest[end..];
        } else {
            let character = rest.chars().next().unwrap();
            result.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    result
}
fn preview(
    items: &[RenameItem],
    options: &RenameOptions,
    occupied: &HashSet<String>,
) -> Vec<PreviewRow> {
    let mut rows = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let result = (|| {
            let pattern = if options.replacing {
                &options.replacement
            } else {
                &options.pattern
            };
            let expanded = template(
                pattern,
                &item.stem,
                options.start as u64 + index as u64,
                options.start as u64 + (items.len() - 1 - index) as u64,
                options.digits,
            )?;
            let name = if options.replacing {
                if options.find.is_empty() {
                    return Err("Enter text to find".into());
                }
                replace_literal(&item.stem, &options.find, &expanded, options.case_sensitive)
            } else {
                expanded
            };
            // Unmatched rows stay untouched, including unusual names that the
            // application's naming rules would reject for a newly named tag.
            if name == item.stem {
                return Ok(item.path.clone());
            }
            let name = validate_leaf_characters(&name, "Names", "Enter a name")?;
            let mut leaf = std::ffi::OsString::from(name);
            if let Some(extension) = &item.extension {
                leaf.push(".");
                leaf.push(extension);
            }
            let target = item.path.with_file_name(leaf);
            if path_key(&target) == path_key(&item.path) {
                return Err("Only the letter case changed; choose a different name".into());
            }
            if occupied.contains(&path_key(&target)) {
                return Err("That name already exists in this folder".into());
            }
            Ok(target)
        })();
        rows.push(match result {
            Ok(target) => PreviewRow {
                changed: target != item.path,
                target,
                error: None,
            },
            Err(error) => PreviewRow {
                target: item.path.clone(),
                changed: false,
                error: Some(error),
            },
        });
    }
    let mut targets = HashMap::<String, Vec<usize>>::new();
    for (index, row) in rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.error.is_none())
    {
        targets
            .entry(path_key(&row.target))
            .or_default()
            .push(index);
    }
    for group in targets.values().filter(|group| group.len() > 1) {
        for &index in group {
            rows[index].error = Some("Several selected items would have this name".into());
        }
    }
    rows
}

pub(in crate::app) struct BulkRenameDialog {
    request: bulk::BulkRequest,
    items: Vec<RenameItem>,
    occupied: HashSet<String>,
    options: RenameOptions,
    error: Option<String>,
    focus: bool,
}
fn insert_token(ui: &Ui, id: egui::Id, text: &mut String, token: &str) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let range = state.cursor.char_range();
    let (start, end) = range
        .map(|range| {
            (
                usize::from(range.primary.index.min(range.secondary.index)),
                usize::from(range.primary.index.max(range.secondary.index)),
            )
        })
        .unwrap_or((text.chars().count(), text.chars().count()));
    let byte = |index| {
        text.char_indices()
            .nth(index)
            .map(|(index, _)| index)
            .unwrap_or(text.len())
    };
    text.replace_range(byte(start)..byte(end), token);
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(start + token.chars().count()),
        )));
    state.store(ui.ctx(), id);
    ui.memory_mut(|memory| memory.request_focus(id));
}
impl Dialog for BulkRenameDialog {
    fn show(&mut self, cx: &Ctx, _: &AppReads) -> bool {
        let mut open = true;
        let previous_options = self.options.clone();
        let rows = preview(&self.items, &self.options, &self.occupied);
        let changes = rows
            .iter()
            .filter(|row| row.changed && row.error.is_none())
            .count();
        let invalid = rows.iter().filter(|row| row.error.is_some()).count();
        let dirty = cx
            .model
            .resolve_stamp(self.request.stamp)
            .is_some_and(|index| {
                cx.model.kits[index]
                    .parsed_tags
                    .values()
                    .any(|doc| doc.dirty.is_set())
            });
        let modal=egui::Modal::new(egui::Id::unique("bulk_rename_items"))
            .frame(egui::Frame::window(&cx.egui.global_style())).show(cx.egui,|ui| {
                ui.set_width(window_width(cx.egui,760.0));
                crate::app::search::draw_icon_window_header(ui,&format!("Rename {} Items",self.items.len()),ButtonIcon::Rename,&mut open);
                ui.add_space(12.0);
                ui.columns(2,|columns| {
                    columns[0].label(RichText::new("Preview").strong());
                    let height=(window_work_area(cx.egui).height()-270.0).clamp(140.0,340.0);
                    ScrollArea::vertical().id_salt("rename_preview").max_height(height).auto_shrink([false,false]).show_rows(&mut columns[0],64.0,rows.len(),|ui,visible| {
                        for index in visible {
                            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(),64.0),egui::Layout::top_down(egui::Align::Min),|ui| {
                            ui.set_min_height(64.0);
                            let item=&self.items[index];let row=&rows[index];
                            ui.add(egui::Label::new(RichText::new(item.path.file_name().unwrap_or_default().to_string_lossy()).monospace().color(subtle_dark())).truncate()).on_hover_text(item.path.display().to_string());
                            let label=row.error.as_deref().map(str::to_owned).unwrap_or_else(||row.target.file_name().unwrap_or_default().to_string_lossy().into_owned());
                            ui.add(egui::Label::new(RichText::new(format!("→ {label}")).monospace().color(if row.error.is_some(){ui.visuals().error_fg_color}else if row.changed{text_dark()}else{subtle_dark()})).truncate()).on_hover_text(label);
                            ui.add_space(8.0);
                            });
                        }
                    });
                    let ui=&mut columns[1];
                    ui.horizontal(|ui| {
                        if selectable_icon_text_button(ui,ButtonIcon::Rename,"Rename",!self.options.replacing).clicked() {self.options.replacing=false;}
                        if selectable_icon_text_button(ui,ButtonIcon::Replace,"Find and Replace",self.options.replacing).clicked() {self.options.replacing=true;}
                    });
                    ui.add_space(10.0);
                    if self.options.replacing {
                        ui.label("Find (literal text)");
                        ui.add(crate::app::ui_kit::singleline_text_edit(&mut self.options.find).desired_width(f32::INFINITY));
                        ui.baboon_checkbox(&mut self.options.case_sensitive,"Case Sensitive");
                        ui.label("Replace With");
                    } else {ui.label("Name Pattern");}
                    let id=egui::Id::unique(("bulk_rename_pattern",self.request.stamp.kit,self.options.replacing));
                    let text=if self.options.replacing {&mut self.options.replacement}else{&mut self.options.pattern};
                    let response=ui.add(crate::app::ui_kit::singleline_text_edit(text).id(id).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                    if self.focus {response.request_focus();self.focus=false;}
                    ui.horizontal_wrapped(|ui| {
                        for (label,token) in [("Current Name","{name}"),("Number ↑","{n}"),("Number ↓","{N}")] {
                            if ui.button(label).clicked() {insert_token(ui,id,text,token);}
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Start Ascending At");ui.add(egui::DragValue::new(&mut self.options.start));
                    });
                    ui.horizontal(|ui| {ui.label("Digits");ui.add(egui::DragValue::new(&mut self.options.digits).range(1..=12));});
                    ui.add_space(10.0);
                    ui.label(RichText::new("{name} keeps the current name. Add text before or after it for a prefix or suffix. Extensions and parent folders stay unchanged.").small().color(subtle_dark()));
                    ui.label(RichText::new("Numbers follow the displayed selection order. Existing names and duplicate results must be resolved before renaming.").small().color(subtle_dark()));
                    if let Some(error)=&self.error {ui.colored_label(ui.visuals().error_fg_color,error);}
                });
                ui.add_space(12.0);
                popup_footer(ui,|ui| {ui.horizontal(|ui| {
                    ui.label(if invalid>0 {format!("{invalid} invalid names")}else{format!("{changes} to rename · {} unchanged",rows.len()-changes)});
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                        if icon_text_button(ui,ButtonIcon::Clear,"Cancel",true).clicked(){open=false;}
                        if icon_text_button(ui,ButtonIcon::Rename,if dirty{"Save All and Rename"}else{"Rename"},invalid==0&&changes>0).clicked() {
                            cx.send(BrowserCommand::ApplyBulkRename {save:dirty});
                        }
                    });
                });});
            });
        if previous_options != self.options {
            self.error = None;
        }
        open && !modal.should_close()
    }
}

impl Baboon {
    pub(in crate::app) fn open_bulk_rename(&mut self, request: bulk::BulkRequest) {
        let result = (|| {
            let index = self
                .model
                .resolve_stamp(request.stamp)
                .ok_or("The workspace changed; select the items again")?;
            if self.model.editing_kit_is_read_only(index) {
                return Err("This editing kit is read-only".into());
            }
            let source = self.model.kits[index]
                .source
                .as_ref()
                .ok_or("No workspace is loaded")?;
            let TagSource::LooseFolder { root, .. } = &source.source else {
                return Err("Bulk rename requires loose tags and folders".into());
            };
            let mut items = Vec::new();
            for key in &request.keys {
                let folder = key.strip_prefix(FOLDER_PANE_PREFIX);
                let path = if let Some(path) = folder {
                    root.join(tag_path::to_relative(path))
                } else {
                    match source.entry_for_key(key).map(|entry| &entry.location) {
                        Some(TagEntryLocation::LooseFile(path)) => path.clone(),
                        _ => return Err("Every selected item must support Rename".into()),
                    }
                };
                if path == *root || !path.starts_with(root) || !path.exists() {
                    return Err("A selected path is invalid or missing".into());
                }
                let stem = if folder.is_some() {
                    path.file_name()
                } else {
                    path.file_stem()
                }
                .ok_or("A selected item has no name")?
                .to_string_lossy()
                .into_owned();
                let extension = if folder.is_some() {
                    None
                } else {
                    path.extension().map(std::ffi::OsStr::to_os_string)
                };
                items.push(RenameItem {
                    path,
                    stem,
                    extension,
                });
            }
            if items.is_empty() {
                return Err("Select items to rename".into());
            }
            let occupied = occupied_names(&items)?;
            Ok(BulkRenameDialog {
                request,
                items,
                occupied,
                options: RenameOptions::default(),
                error: None,
                focus: true,
            })
        })();
        match result {
            Ok(dialog) => self.dialogs.open(dialog),
            Err(error) => self.model.status = error,
        }
    }
    pub(in crate::app) fn apply_bulk_rename(&mut self, save: bool, _ctx: &egui::Context) {
        let result = (|| {
            let dialog = self
                .dialogs
                .get::<BulkRenameDialog>()
                .ok_or("Rename dialog closed")?;
            let request = dialog.request.clone();
            let items = dialog.items.clone();
            let options = dialog.options.clone();
            let index = self
                .model
                .resolve_stamp(request.stamp)
                .ok_or("The workspace changed; select the items again")?;
            if self.tag_ops.folder_refactor.is_some() {
                return Err("A file operation is already running".into());
            }
            if self.model.editing_kit_is_read_only(index) {
                return Err("This editing kit is read-only".into());
            }
            self.focus_kit(index);
            let rows = preview(&items, &options, &occupied_names(&items)?);
            if let Some(error) = rows.iter().find_map(|row| row.error.clone()) {
                return Err(error);
            }
            let roots: Vec<_> = items
                .iter()
                .zip(&rows)
                .filter(|(_, row)| row.changed)
                .map(|(item, row)| (item.path.clone(), row.target.clone()))
                .collect();
            if roots.is_empty() {
                return Err("No names would change".into());
            }
            let dirty: Vec<_> = self.model.kits[index]
                .parsed_tags
                .iter()
                .filter(|(_, doc)| doc.dirty.is_set())
                .map(|(key, _)| key.clone())
                .collect();
            if !dirty.is_empty() && !save {
                return Err("Save unsaved tags before renaming; use Save All and Rename".into());
            }
            for key in dirty {
                self.save_tag_by_key(&key)?;
            }
            let source = self.model.source().ok_or("Workspace closed")?;
            let TagSource::LooseFolder { root, game, .. } = &source.source else {
                return Err("Bulk rename requires a loose editing kit".into());
            };
            let root = root.clone();
            let game = *game;
            let entries = source.full_entry_set().to_vec();
            let names = self.model.names().clone();
            let dependencies = source.reverse_dependencies.clone();
            let context = self.new_merge_context();
            let tx = self.tx.clone();
            let stamp = request.stamp;
            let pane = request.pane;
            self.tag_ops.folder_refactor = Some(FolderRefactorUiState {
                label: "Renaming Items".into(),
                phase: "Checking references".into(),
                progress: None,
            });
            self.dialogs.close::<BulkRenameDialog>();
            self.spawn_job(
                move || WorkerMessage::BrowserBulkFinished {
                    stamp,
                    result: folder_merge_ops::run_renames(
                        root,
                        roots,
                        names,
                        game,
                        entries,
                        dependencies,
                        context,
                        &tx,
                    )
                    .map(|done| bulk::BulkOutcome {
                        done,
                        pane,
                        completed: Vec::new(),
                        deleted: Vec::new(),
                        deleted_folders: Vec::new(),
                        renamed: true,
                    }),
                },
                move |_| WorkerMessage::BrowserBulkFinished {
                    stamp,
                    result: Err("Bulk rename worker crashed".into()),
                },
            );
            Ok(())
        })();
        if let Err(error) = result {
            if let Some(dialog) = self.dialogs.get_mut::<BulkRenameDialog>() {
                if let Ok(occupied) = occupied_names(&dialog.items) {
                    dialog.occupied = occupied;
                }
                dialog.error = Some(error);
            } else {
                self.model.status = error;
            }
        }
    }
}

#[cfg(test)]
impl BulkRenameDialog {
    pub(in crate::app) fn for_smoke(stamp: KitStamp) -> Self {
        let items = ["rifle.model", "rifle.render_model"]
            .into_iter()
            .map(|leaf| {
                let path = PathBuf::from("tags").join(leaf);
                RenameItem {
                    stem: path.file_stem().unwrap().to_string_lossy().into_owned(),
                    extension: path.extension().map(std::ffi::OsStr::to_os_string),
                    path,
                }
            })
            .collect();
        Self {
            request: bulk::BulkRequest {
                stamp,
                pane: String::new(),
                keys: Vec::new(),
                action: BulkAction::Rename,
            },
            items,
            occupied: HashSet::new(),
            options: RenameOptions::default(),
            error: None,
            focus: true,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::*;
    fn item(path: PathBuf) -> RenameItem {
        RenameItem {
            stem: path.file_stem().unwrap().to_string_lossy().into_owned(),
            extension: path.extension().map(std::ffi::OsStr::to_os_string),
            path,
        }
    }
    #[test]
    fn shared_stems_keep_extensions_and_parent_folders() {
        let items = vec![
            item("ui/banshee.new_hud_definition".into()),
            item("ui/banshee.weapon".into()),
        ];
        let settings = RenameOptions {
            pattern: "test_{name}_rebalance".into(),
            ..Default::default()
        };
        let rows = preview(&items, &settings, &HashSet::new());
        assert!(rows.iter().all(|row| row.changed && row.error.is_none()));
        assert_eq!(
            rows[0].target,
            PathBuf::from("ui/test_banshee_rebalance.new_hud_definition")
        );
        assert_eq!(
            rows[1].target,
            PathBuf::from("ui/test_banshee_rebalance.weapon")
        );
        let rows = preview(
            &items,
            &RenameOptions {
                pattern: "new_common_name".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert!(rows.iter().all(|row| row.error.is_none()));
    }
    #[test]
    fn numbers_follow_selection_order_and_names_are_not_reparsed() {
        let items = vec![item("z.model".into()), item("a.model".into())];
        let rows = preview(
            &items,
            &RenameOptions {
                pattern: "{n}_{name}_{N}".into(),
                start: 7,
                digits: 3,
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert_eq!(rows[0].target, PathBuf::from("007_z_008.model"));
        assert_eq!(rows[1].target, PathBuf::from("008_a_007.model"));
        assert_eq!(
            template("prefix_{name}", "literal{n}", 1, 2, 1).unwrap(),
            "prefix_literal{n}"
        );
        assert_eq!(template("{{name}}", "ignored", 1, 2, 1).unwrap(), "{name}");
        assert!(template("{typo}", "name", 1, 2, 1).is_err());
    }
    #[test]
    fn replacement_is_literal_optional_case_sensitive_and_leaves_unmatched_names_alone() {
        let items = vec![item("old_old.model".into()), item("other.model".into())];
        let rows = preview(
            &items,
            &RenameOptions {
                replacing: true,
                find: "old".into(),
                replacement: "new".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert_eq!(rows[0].target, PathBuf::from("new_new.model"));
        assert!(!rows[1].changed);
        assert_eq!(
            replace_literal("ÉQUIPE_équipe", "équipe", "team", false),
            "team_team"
        );
        assert_eq!(replace_literal("a.b_a_b", "a.b", "new", true), "new_a_b");
        assert_eq!(replace_literal("old", "old", "$1", true), "$1");
    }
    #[test]
    fn collisions_and_invalid_names_disable_apply_without_suffixing() {
        let items = vec![item("a.model".into()), item("b.model".into())];
        let rows = preview(
            &items,
            &RenameOptions {
                pattern: "same".into(),
                ..Default::default()
            },
            &HashSet::new(),
        );
        assert!(rows.iter().all(|row| row.error.is_some()));
        for name in ["../escape", "bad.name", "CON", ""] {
            assert!(
                preview(
                    &items,
                    &RenameOptions {
                        pattern: name.into(),
                        ..Default::default()
                    },
                    &HashSet::new()
                )
                .iter()
                .all(|row| row.error.is_some())
            );
        }
        let occupied = HashSet::from([path_key(Path::new("TAKEN.model"))]);
        assert!(
            preview(
                &items[..1],
                &RenameOptions {
                    pattern: "taken".into(),
                    ..Default::default()
                },
                &occupied
            )[0]
            .error
            .is_some()
        );
        assert!(
            preview(
                &items[..1],
                &RenameOptions {
                    pattern: "A".into(),
                    ..Default::default()
                },
                &HashSet::new()
            )[0]
            .error
            .is_some()
        );
    }
    fn run(
        kit: &LooseKit,
        roots: Vec<(PathBuf, PathBuf)>,
    ) -> Result<crate::app::shell::FolderRefactorFinished, String> {
        let (tx, _rx) = mpsc::channel();
        let context = folder_merge_ops::MergeContext {
            recovery: Err("Rename must not overwrite anything".into()),
            control: Arc::new(crate::app::tag_ops::folder_merge_review::MergeControl::default()),
        };
        folder_merge_ops::run_renames(
            kit.root.clone(),
            roots,
            kit.names(),
            GameId::from_id(kit.game),
            kit.entries(),
            Some(kit.index()),
            context,
            &tx,
        )
    }
    #[test]
    fn renaming_shared_stems_updates_internal_and_external_references_as_one_batch() {
        let kit = LooseKit::new("bulk-rename-shared-stems", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("objects/crate", "render_model", |_| {});
        kit.write_mcc("objects/crate", "model", |tag| {
            set_reference(tag, "render model", group, "objects\\crate")
        });
        kit.write_mcc("other/uses_crate", "model", |tag| {
            set_reference(tag, "render model", group, "objects\\crate")
        });
        let roots = vec![
            (
                kit.path("objects/crate.model"),
                kit.path("objects/new_crate.model"),
            ),
            (
                kit.path("objects/crate.render_model"),
                kit.path("objects/new_crate.render_model"),
            ),
        ];
        let done = run(&kit, roots).unwrap();
        assert_eq!(done.old_to_new_keys.len(), 2);
        assert!(done.status.starts_with("Renamed"));
        assert!(!kit.path("objects/crate.model").exists());
        for name in ["objects/new_crate.model", "other/uses_crate.model"] {
            assert_eq!(
                reference_of(&TagFile::read(kit.path(name)).unwrap(), "render model").as_deref(),
                Some("objects\\new_crate")
            );
        }
    }
    #[test]
    fn a_rename_collision_never_merges_folders_or_overwrites_files() {
        let kit = LooseKit::new("bulk-rename-collision", "halo3_mcc");
        kit.write_mcc("original/a", "model", |_| {});
        kit.write_mcc("existing/b", "model", |_| {});
        let before = std::fs::read(kit.path("original/a.model")).unwrap();
        let error = run(&kit, vec![(kit.path("original"), kit.path("existing"))])
            .err()
            .unwrap();
        assert!(error.contains("already exists"));
        assert_eq!(std::fs::read(kit.path("original/a.model")).unwrap(), before);
        assert!(!kit.path("existing/a.model").exists());
    }
    #[test]
    fn dialog_preview_is_live_and_cancel_changes_nothing() {
        let kit = LooseKit::new("bulk-rename-dialog", "halo3_mcc");
        kit.write_mcc("crate", "model", |_| {});
        kit.write_mcc("crate", "render_model", |_| {});
        let mut app = app();
        kit.install(&mut app);
        let keys = vec![kit.key("crate.model"), kit.key("crate.render_model")];
        app.begin_browser_bulk("pane".into(), keys, BulkAction::Rename, &ctx());
        app.dialogs
            .get_mut::<BulkRenameDialog>()
            .unwrap()
            .options
            .pattern = "prefix_{name}_suffix".into();
        let mut driver = PromptDriver::new();
        driver.frame(&mut app, Vec::new());
        let page = driver.frame(&mut app, Vec::new());
        for label in [
            "Rename 2 Items",
            "Preview",
            "→ prefix_crate_suffix.model",
            "→ prefix_crate_suffix.render_model",
        ] {
            assert!(
                page.iter().any(|(text, _)| text == label),
                "missing {label}: {page:?}"
            );
        }
        driver.click(&mut app, "Cancel");
        assert!(app.dialogs.get::<BulkRenameDialog>().is_none());
        assert!(kit.path("crate.model").exists());
        assert!(!kit.path("prefix_crate_suffix.model").exists());
    }
    #[test]
    fn mixed_folder_and_tag_rename_preserves_contents_and_rewrites_referrers() {
        let kit = LooseKit::new("rename-mixed-roots", "halo3_mcc");
        let group = group_tag(kit.game, "render_model");
        kit.write_mcc("group/part", "render_model", |_| {});
        kit.write_mcc("loose", "render_model", |_| {});
        kit.write_mcc("outside/uses_group", "model", |tag| {
            set_reference(tag, "render model", group, "group\\part")
        });
        fs::create_dir_all(kit.path("group/empty")).unwrap();
        fs::write(kit.path("group/note.txt"), "note").unwrap();
        let done = run(
            &kit,
            vec![
                (kit.path("group"), kit.path("new_group")),
                (
                    kit.path("loose.render_model"),
                    kit.path("new_loose.render_model"),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            done.moved_folders,
            vec![(PathBuf::from("group"), PathBuf::from("new_group"))]
        );
        assert!(kit.path("new_group/empty").is_dir());
        assert_eq!(
            fs::read_to_string(kit.path("new_group/note.txt")).unwrap(),
            "note"
        );
        assert_eq!(
            reference_of(
                &TagFile::read(kit.path("outside/uses_group.model")).unwrap(),
                "render model"
            )
            .as_deref(),
            Some("new_group\\part")
        );
    }
    #[test]
    fn rename_completion_keeps_the_new_names_selected_and_remaps_open_tabs() {
        let kit = LooseKit::new("rename-completion", "halo3_mcc");
        kit.write_mcc("crate", "model", |_| {});
        let mut app = app();
        kit.install(&mut app);
        let ctx = ctx();
        app.handle_browser_action(
            BrowserAction::OpenFolderBrowser {
                rel_path: PathBuf::new(),
                label: "Tags".into(),
                open_in_new_tab: true,
            },
            ctx.clone(),
        );
        let pane = app.model.kits[0].current_pane.clone().unwrap();
        let old = kit.key("crate.model");
        let new = crate::core::source::file_entry_key(&kit.path("new_crate.model"));
        let id = app.model.active_kit_id();
        let browser = app.views[id]
            .browser
            .folder_browsers
            .get_mut(&pane)
            .unwrap();
        browser.selection.keys.insert(old.clone());
        browser.selection.focus = Some(old.clone());
        browser.selected_tag = Some(old.clone());
        app.kit_and_view(0).open_tag_pane(&old);
        let stamp = app.model.kit_stamp();
        let done = run(
            &kit,
            vec![(kit.path("crate.model"), kit.path("new_crate.model"))],
        )
        .unwrap();
        app.handle_browser_bulk_finished(
            stamp,
            Ok(bulk::BulkOutcome {
                done,
                pane: pane.clone(),
                completed: Vec::new(),
                deleted: Vec::new(),
                deleted_folders: Vec::new(),
                renamed: true,
            }),
            &ctx,
        );
        assert!(app.model.kits[0].open_tabs.contains(&new));
        assert!(!app.model.kits[0].open_tabs.contains(&old));
        assert_eq!(
            app.views[id].browser.folder_browsers[&pane].selection.keys,
            std::collections::BTreeSet::from([new])
        );
    }
}
