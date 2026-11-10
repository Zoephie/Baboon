//! String-list import/export options, validation, and document-aware application.

use super::*;
use super::string_lists::*;
use blam_tags::string_list::{self as strings, Storage, StringList};
use crate::app::editor::UndoStep;
use crate::core::document::apply::DeferredOps;
use crate::app::search::{StringFindContext, string_find_context_id};
use super::string_transfer::*;

#[path = "string_transfer_ui.rs"]
mod ui;

pub(in crate::app) struct StringToolReload {
    run_id: u64,
    kit: KitId,
    source_session: u64,
    documents: Vec<(String, u64, u64)>,
}

pub(in crate::app) struct StringTransferWindow {
    stamp: KitStamp,
    source_session: u64,
    key: String,
    import: bool,
    language: Option<usize>,
    stem: String,
    display_path: String,
    directory: Option<PathBuf>,
    path: Option<PathBuf>,
    revision: Option<(u64, u64)>,
    list: Option<std::sync::Arc<StringList>>,
    preview: Option<Result<StringImportPlan, String>>,
    error: Option<String>,
    relative: PathBuf,
    method: StringMethod,
    format: StringFormat,
    destination: StringDestination,
    mode: Option<ImportMode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::LooseKit;
    use crate::core::document::ops::StringListOp;
    use blam_tags::string_list::{StringEntry, StringListEdit};

    fn fixture() -> (Baboon, LooseKit, String) {
        let kit = LooseKit::new("string-transfers", "halo3_mcc");
        kit.write_mcc("ui\\messages", "multilingual_unicode_string_list", |tag| {
            let count = strings::read_string_list(tag).unwrap().languages.len();
            let mut texts = vec![None; count];
            texts[0] = Some("Original".to_owned());
            texts[1] = Some("日本語".to_owned());
            strings::apply_string_list_edits(tag, Some(GameId::Halo3), &[StringListEdit::Insert {
                at: 0, entries: vec![StringEntry { id: Some("s0".to_owned()), texts }],
            }]).unwrap();
        });
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        let key = kit.open(&mut app, "ui\\messages.multilingual_unicode_string_list");
        (app, kit, key)
    }

    #[test]
    fn native_ce_and_h2_string_imports_preflight_serialize_and_undo() {
        use blam_tags::classic::{ClassicEngine, ClassicHeader};
        for (profile, game, engine, group) in [
            ("haloce_mcc", GameId::HaloCe, ClassicEngine::HaloCe, "unicode_string_list"),
            ("haloce_mcc", GameId::HaloCe, ClassicEngine::HaloCe, "hud_message_text"),
            ("halo2_mcc", GameId::Halo2, ClassicEngine::Halo2V4, "multilingual_unicode_string_list"),
        ] {
            for format in [StringFormat::Csv, StringFormat::Text] {
                let kit = LooseKit::new("native-string-transfer", profile);
                let mut tag = TagFile::new_classic(crate::core::bundled::locate_definitions_root().join(profile).join(format!("{group}.json")), engine).unwrap();
                let mut row = strings::read_string_list(&tag).unwrap().new_entry();
                row.id = (group != "unicode_string_list").then(|| "s0".to_owned());
                row.texts[0] = Some("Original".to_owned());
                strings::apply_string_list_edits(&mut tag, Some(game), &[StringListEdit::Insert { at: 0, entries: vec![row] }]).unwrap();
                let path = kit.path(&format!("messages.{group}"));
                let before = tag.write_to_bytes().unwrap();
                assert!(ClassicHeader::parse(&before).is_some());
                assert!(TagFile::read_from_bytes(&before).is_err(), "fixture exercises native decoding");
                fs::write(&path, &before).unwrap();
                let mut app = Baboon::for_test();
                kit.install(&mut app);
                let key = kit.open(&mut app, &format!("messages.{group}"));
                let file = kit.base.join(if format == StringFormat::Csv { "import.csv" } else { "import.txt" });
                let contents = if format == StringFormat::Csv {
                    if group == "hud_message_text" { "Name,Text\ns0,Edited 日本語 %action" } else if game == GameId::HaloCe { "Index,String Text\n0,Edited 日本語" } else { "String ID,English Text\ns0,Edited 日本語" }
                } else if group == "hud_message_text" { "s0=Edited 日本語 %action\n" } else if game == GameId::HaloCe { "Edited 日本語\n###END-STRING###\n" }
                else { "[Strings]\ns0 = \"Edited 日本語\"\n" };
                fs::write(&file, contents).unwrap();
                let result = app.run_string_transfer_options(app.model.kit_stamp(), &key, true, Some(0), &file,
                    StringTransferOptions { format, mode: Some(ImportMode::Replace), destination: StringDestination::Custom }).unwrap();
                assert!(result.contains("updated 1"), "{profile} {format:?}: {result}");
                let doc = &app.model.kits[0].parsed_tags[&key];
                assert_eq!(strings::read_string_list(&doc.tag).unwrap().entries[0].texts[0].as_deref(), Some(if group == "hud_message_text" { "Edited 日本語 %{18}" } else { "Edited 日本語" }));
                assert!(ClassicHeader::parse(&doc.tag.write_to_bytes().unwrap()).is_some());
                assert_eq!(fs::read(&path).unwrap(), before, "import leaves the on-disk tag untouched");
                app.undo_current_tag();
                assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
            }
        }
    }

    #[test]
    fn string_transfer_uses_current_document_imports_unsaved_and_undoes() {
        let (mut app, kit, key) = fixture();
        let ctx = egui::Context::default();
        app.apply_doc_ops(0, &key, "Edit", DeferredOps {
            string_list_ops: vec![StringListOp { game: Some(GameId::Halo3), edit: StringListEdit::SetText {
                index: 0, language: 0, text: "Unsaved text".to_owned(),
            } }], ..Default::default()
        }, UndoStep::Own);
        let stamp = app.model.kit_stamp();
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-string-transfer-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let export = directory.join("export.txt");
        app.run_string_transfer(stamp, &key, false, Some(0), &export).unwrap();
        let list = strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap();
        let syntax = crate::app::editor::string_list::text_syntax(None, Some(GameId::Halo3));
        assert!(plan_string_import(&list, Some(0), "unused", &export, &syntax).unwrap().edits.is_empty());
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        let disk_path = kit.path("ui\\messages.multilingual_unicode_string_list");
        let disk_before = fs::read(&disk_path).unwrap();
        let import = directory.join("import.txt");
        fs::write(&import, "s0 = \"Edited translation\"\nnew_id = \"New text\"").unwrap();
        let message = app.run_string_transfer(stamp, &key, true, Some(1), &import).unwrap();
        assert!(message.contains("updated 1, added 1, removed 0"));
        let doc = &mut app.model.kits[0].parsed_tags.get_mut(&key).unwrap();
        let list = strings::read_string_list(&doc.tag).unwrap();
        assert_eq!(list.entries[0].texts[0].as_deref(), Some("Unsaved text"));
        assert_eq!(list.entries[0].texts[1].as_deref(), Some("Edited translation"));
        assert_eq!(fs::read(&disk_path).unwrap(), disk_before, "import never saves the tag behind the user's back");
        assert_eq!(*doc.journal.undo(&doc.tag).unwrap().0, before);
        fs::remove_dir_all(directory).unwrap();
        // Opening options requires no native picker and preserves the selected language.
        ctx.data_mut(|data| data.insert_temp(string_find_context_id(stamp.kit, &key), StringFindContext {
            selected: Default::default(),
            kit: stamp.kit, tag_key: key.clone(), scope: "test".to_owned(), game: Some(GameId::Halo3),
            definitions_root: None, language: 1, languages: Vec::new(), has_ids: true, editor_active: true,
        }));
        app.open_string_transfer(&key, false, &ctx);
        assert_eq!(app.dialogs.get::<StringTransferWindow>().unwrap().language, Some(1));
    }

    #[test]
    fn string_transfer_rejects_bad_files_and_stale_workspaces_without_changes() {
        let (mut app, _kit, key) = fixture();
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        let path = std::env::temp_dir().join(format!("baboon-invalid-strings-{}.txt", uuid::Uuid::new_v4()));
        fs::write(&path, "s0 = \"Valid first line\"\ninvalid second line").unwrap();
        let stamp = app.model.kit_stamp();
        assert!(app.run_string_transfer(stamp, &key, true, Some(0), &path).is_err());
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
        assert!(!app.model.kits[0].parsed_tags[&key].journal.can_undo());
        let stale = KitStamp { generation: stamp.generation + 1, ..stamp };
        assert!(app.run_string_transfer(stale, &key, true, Some(0), &path).is_err());
        assert_eq!(app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap(), before);
        fs::remove_file(path).unwrap();
    }

    fn draw_dialog(app: &mut Baboon, ctx: &egui::Context) {
        crate::app::run_ui_test(ctx, crate::app::loose_fixture::screen(Vec::new(), 1.0), |_| {
            app.dialogs.draw(&cx!(app, ctx), &app_reads!(app));
        });
    }

    #[test]
    fn string_transfer_settings_width_stays_stable_across_frames() {
        for (import, destination) in [
            (true, StringDestination::Kit), (false, StringDestination::Kit),
            (true, StringDestination::Custom), (false, StringDestination::Custom),
        ] {
            let (mut app, _kit, key) = fixture();
            let ctx = egui::Context::default();
            ctx.set_fonts(foundation_fonts());
            ctx.set_global_style(foundation_style());
            app.open_string_transfer(&key, import, &ctx);
            {
                let window = app.dialogs.get_mut::<StringTransferWindow>().unwrap();
                window.destination = destination;
                if destination == StringDestination::Custom {
                    window.path = Some(PathBuf::from(format!("C:\\{}\\strings.csv", "long-folder-name\\".repeat(30))));
                }
            }
            let mut widths = Vec::new();
            let mut texts = Vec::new();
            for frame in 0..35 {
                let output = crate::app::run_ui_test(&ctx, egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(3000.0, 1600.0))),
                    time: Some(frame as f64 / 60.0), ..Default::default()
                }, |_| app.dialogs.draw(&cx!(app, &ctx), &app_reads!(app)));
                widths.push(ctx.memory(|memory| memory.area_rect(egui::Id::unique("string_list_transfer"))).unwrap().width());
                if frame == 34 {
                    texts = output.shapes.iter().filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some((text.galley.text().to_owned(), egui::Rect::from_min_size(text.pos, text.galley.size()))),
                        _ => None,
                    }).collect::<Vec<_>>();
                }
            }
            assert!((widths[34] - widths[4]).abs() <= 1.0, "import={import}, window widths: {widths:?}");
            let text_rect = |label: &str| texts.iter().find(|(text, _)| text == label).unwrap().1;
            let left = text_rect("Method:").left();
            for label in ["Format:", "Language:", "Destination:"] {
                assert!((text_rect(label).left() - left).abs() < 0.5, "{label} must align with Method");
            }
            if destination == StringDestination::Custom {
                assert!((text_rect("Path:").left() - left).abs() < 0.5);
                assert!(text_rect("Browse…").left() > text_rect("Path:").right());
            }
            if import {
                let refresh = text_rect("Refresh File Preview");
                let action = text_rect("Import");
                assert!(refresh.right() < action.left());
                assert!((refresh.center().y - action.center().y).abs() < 0.5, "refresh belongs in the footer beside Import");
            }
        }
    }

    #[test]
    fn string_transfer_dialog_preserves_choices_through_reindex_but_not_source_replacement() {
        let (mut app, _kit, key) = fixture();
        let ctx = egui::Context::default();
        ctx.set_fonts(foundation_fonts());
        app.open_string_transfer(&key, true, &ctx);
        assert!(app.dialogs.get::<StringTransferWindow>().unwrap().mode.is_none());
        {
            let window = app.dialogs.get_mut::<StringTransferWindow>().unwrap();
            window.mode = Some(ImportMode::Replace);
            window.format = StringFormat::Csv;
            window.destination = StringDestination::Custom;
            window.language = Some(1);
            window.path = Some(PathBuf::from("chosen-file.csv"));
        }
        draw_dialog(&mut app, &ctx);
        let revision = app.dialogs.get::<StringTransferWindow>().unwrap().revision;
        let entries = app.model.kits[0].source.as_ref().unwrap().full_entry_set().to_vec();
        app.install_complete_entry_set(0, entries);
        draw_dialog(&mut app, &ctx);
        let window = app.dialogs.get::<StringTransferWindow>().unwrap();
        assert_eq!(window.mode, Some(ImportMode::Replace));
        assert_eq!(window.format, StringFormat::Csv);
        assert_eq!(window.language, Some(1));
        assert_eq!(window.path.as_deref(), Some(Path::new("chosen-file.csv")));
        assert_eq!(window.revision, revision);
        assert_eq!(window.stamp.generation, app.model.kits[0].generation);
        app.model.kits[0].source_session += 1;
        draw_dialog(&mut app, &ctx);
        assert!(app.dialogs.get::<StringTransferWindow>().is_some(), "source replacement explains unavailability rather than silently retargeting");
        app.dialogs.close::<StringTransferWindow>();
        app.open_string_transfer(&key, true, &ctx);
        assert!(app.dialogs.get::<StringTransferWindow>().unwrap().mode.is_none(), "every import requires a fresh mode choice");
    }

    #[test]
    fn string_csv_replace_is_one_unsaved_undo_and_requires_explicit_mode() {
        let (mut app, kit, key) = fixture();
        let before = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        let disk_before = fs::read(kit.path("ui\\messages.multilingual_unicode_string_list")).unwrap();
        let path = std::env::temp_dir().join(format!("string-replace-{}.csv", uuid::Uuid::new_v4()));
        fs::write(&path, "String ID,English Text\nnew_id,新しい\ns0,Updated\n").unwrap();
        let stamp = app.model.kit_stamp();
        let mut options = StringTransferOptions { format: StringFormat::Csv, mode: None, destination: StringDestination::Custom };
        assert!(app.run_string_transfer_options(stamp, &key, true, None, &path, options).is_err());
        options.mode = Some(ImportMode::Replace);
        app.run_string_transfer_options(stamp, &key, true, None, &path, options).unwrap();
        let doc = app.model.kits[0].parsed_tags.get_mut(&key).unwrap();
        let list = strings::read_string_list(&doc.tag).unwrap();
        assert_eq!(list.entries[0].id.as_deref(), Some("new_id"));
        assert_eq!(list.entries[1].texts[1].as_deref(), Some("日本語"));
        assert_eq!(*doc.journal.undo(&doc.tag).unwrap().0, before);
        assert!(!doc.journal.can_undo());
        assert_eq!(fs::read(kit.path("ui\\messages.multilingual_unicode_string_list")).unwrap(), disk_before);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn string_tool_reload_refreshes_clean_docs_and_preserves_new_edits_after_reindex() {
        let (mut app, kit, key) = fixture();
        let queue_reload = |app: &mut Baboon, run_id| {
            let doc = &app.model.kits[0].parsed_tags[&key];
            app.export.pending_string_tool_reload = Some(StringToolReload {
                run_id, kit: app.model.kits[0].id, source_session: app.model.kits[0].source_session,
                documents: vec![(key.clone(), doc.id, doc.dirty.revision())],
            });
        };
        queue_reload(&mut app, 1);
        let bytes = app.model.kits[0].parsed_tags[&key].tag.write_to_bytes().unwrap();
        let mut tag = TagFile::read_from_bytes(&bytes).unwrap();
        strings::apply_string_list_edits(&mut tag, Some(GameId::Halo3), &[StringListEdit::SetText { index: 0, language: 0, text: "From tool".to_owned() }]).unwrap();
        tag.write_atomic(kit.path("ui\\messages.multilingual_unicode_string_list")).unwrap();
        app.model.kits[0].generation += 1;
        app.finish_tool_string_reload(1);
        assert_eq!(strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap().entries[0].texts[0].as_deref(), Some("From tool"));
        queue_reload(&mut app, 2);
        app.apply_doc_ops(0, &key, "Edit", DeferredOps {
            string_list_ops: vec![StringListOp { game: Some(GameId::Halo3), edit: StringListEdit::SetText { index: 0, language: 0, text: "Newer edit".to_owned() } }], ..Default::default()
        }, UndoStep::Own);
        app.finish_tool_string_reload(2);
        assert_eq!(strings::read_string_list(&app.model.kits[0].parsed_tags[&key].tag).unwrap().entries[0].texts[0].as_deref(), Some("Newer edit"));
    }
}

impl Baboon {
    pub(in crate::app) fn open_string_transfer(&mut self, key: &str, import: bool, ctx: &egui::Context) {
        if import && self.refuse_read_only_edit(self.model.active) { return; }
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "That string list is no longer available".to_owned(); return;
        };
        if !crate::app::editor::string_list::supports_group(entry.group_tag) {
            self.model.status = "String import/export requires a supported string tag".to_owned(); return;
        }
        let kit = self.model.active;
        if !self.model.kits[kit].parsed_tags.contains_key(key) {
            let Some(source) = self.model.source() else { return; };
            match read_entry(&source.source, &entry) {
                Ok(tag) => { self.model.kits[kit].parsed_tags.insert(key.to_owned(), TagDocument::clean(tag)); }
                Err(error) => { self.model.status = format!("Could not read strings: {error:#}"); return; }
            }
        }
        let Some(list) = strings::read_string_list(&self.model.kits[kit].parsed_tags[key].tag) else { return; };
        let stamp = self.model.kit_stamp();
        let language = ctx.data(|data| data.get_temp::<StringFindContext>(string_find_context_id(stamp.kit, key)))
            .map_or(0, |context| context.language).min(list.languages.len().saturating_sub(1));
        let directory = self.model.kit_layout_for(kit).map(|layout| layout.data);
        self.dialogs.open(StringTransferWindow {
            source_session: self.model.kits[kit].source_session,
            relative: PathBuf::from(entry.display_path.replace('\\', "/")).with_extension(""),
            method: StringMethod::Baboon, format: StringFormat::Text, mode: None,
            destination: if directory.is_some() { StringDestination::Kit } else { StringDestination::Custom },
            stamp, key: key.to_owned(), import, language: Some(language), stem: tag_file_stem(&entry),
            display_path: entry.display_path.clone(),
            directory, path: None, revision: None, list: None, preview: None, error: None,
        });
    }

    pub(in crate::app) fn finish_string_transfer_options(&mut self, stamp: KitStamp, key: &str, import: bool, language: Option<usize>, path: &Path, options: StringTransferOptions) {
        let result = self.run_string_transfer_options(stamp, key, import, language, path, options);
        match result {
            Ok(message) => {
                self.model.status = message;
                self.dialogs.close::<StringTransferWindow>();
            }
            Err(error) => {
                self.model.status = format!("Could not {} strings: {error}", if import { "import" } else { "export" });
                if let Some(window) = self.dialogs.get_mut::<StringTransferWindow>() { window.error = Some(error); }
            }
        }
    }

    #[cfg(test)]
    fn run_string_transfer(&mut self, stamp: KitStamp, key: &str, import: bool, language: Option<usize>, path: &Path) -> Result<String, String> {
        self.run_string_transfer_options(stamp, key, import, language, path, StringTransferOptions {
            format: StringFormat::Text, mode: Some(ImportMode::Amend), destination: StringDestination::Custom,
        })
    }

    fn run_string_transfer_options(&mut self, stamp: KitStamp, key: &str, import: bool, language: Option<usize>, path: &Path, options: StringTransferOptions) -> Result<String, String> {
        let index = self.model.resolve_stamp(stamp).ok_or("The source workspace has closed or changed")?;
        if import && self.model.editing_kit_is_read_only(index) { return Err("This editing kit is read-only".to_owned()); }
        // The options dialog lets ordinary focus-loss commits settle. Refused
        // drafts must be resolved before replacing or exporting their document.
        if self.views[stamp.kit].edit_buffers.has_changed_prefix(&format!("{key}|")) {
            return Err("Finish or cancel the current string edit first".to_owned());
        }
        let entry = self.model.kits[index].source.as_ref().and_then(|source| source.full_entry_set().iter().find(|entry| entry.key == key))
            .cloned().ok_or("That string list is no longer in the source")?;
        let document = self.model.kits[index].parsed_tags.get(key).ok_or("The string list is no longer open")?;
        let list = strings::read_string_list(&document.tag).ok_or("The tag is not a supported string tag")?;
        let source = self.model.kits[index].source.as_ref();
        let game = source.and_then(|source| source.game);
        let definitions = source.and_then(|source| match &source.source {
            TagSource::LooseFolder { definitions_root, .. } => Some(definitions_root.as_path()), _ => None,
        });
        let syntax = list.storage.text_syntax(crate::app::editor::string_list::text_syntax(definitions, game));
        let stem = tag_file_stem(&entry);
        let relative = PathBuf::from(entry.display_path.replace('\\', "/")).with_extension("");
        let layout = self.model.kit_layout_for(index);
        let paths = transfer_paths(&list, language, &stem, &relative, path, layout.as_ref(), game, options)?;
        if !import {
            let files = if options.format == StringFormat::Csv {
                write_file(&paths[0].1, &export_csv(&list, language, &syntax)?)?;
                1
            } else {
                let files: Vec<_> = paths.iter().map(|(language, path)| {
                    export_text(&list, *language, &syntax).map(|text| (path, encode_text(&text)))
                }).collect::<Result<_, _>>()?;
                for (path, bytes) in files { write_file(path, &bytes)?; }
                paths.len()
            };
            return Ok(format!("Exported {files} strings file(s) to {}", path.display()));
        }
        let plan = transfer_import_plan(&list, &paths, options, &syntax)?;
        let changed = !plan.edits.is_empty();
        if !plan.edits.is_empty() {
            // Preflight the complete import, including engine limits and Reach's pairs,
            // before any change reaches the actual document or its undo journal.
            let bytes = document.tag.write_to_bytes().map_err(|error| error.to_string())?;
            let mut candidate = crate::core::source::read_tag_from_bytes(
                &bytes, game, definitions, document.tag.group().tag,
            ).map_err(|error| error.to_string())?;
            strings::apply_string_list_edits(&mut candidate, game, &plan.edits)?;
            if list.storage == Storage::HudMessages {
                let after = strings::read_string_list(&candidate).ok_or("Could not read imported HUD messages")?;
                self.check_hud_rows(index, key, &list.entries, &after.entries)?;
            }
            candidate.write_to_bytes().map_err(|error| error.to_string())?;
            self.model.kits[index].parsed_tags.get_mut(key).unwrap().journal.end_edit_window();
            let ops = DeferredOps {
                string_list_ops: plan.edits.into_iter().map(|edit| crate::core::document::ops::StringListOp { game, edit }).collect(),
                ..Default::default()
            };
            self.apply_doc_ops(index, key, "Import strings", ops, UndoStep::Own).ok_or("The import could not be applied")?;
            self.views[stamp.kit].edit_buffers.retain(|draft, _| !draft.starts_with(&format!("{key}|strings|")));
        }
        self.kit_and_view(index).open_tag_pane(key);
        Ok(if changed {
            format!("Imported strings: updated {}, added {}, removed {} (unsaved)", plan.updated, plan.added, plan.removed)
        } else { "Strings already match the imported files".to_owned() })
    }

    pub(in crate::app) fn start_tool_string_transfer(&mut self, stamp: KitStamp, key: &str, import: bool, ctx: &egui::Context) {
        let result = (|| -> Result<(), String> {
            let index = self.model.resolve_stamp(stamp).ok_or("The source workspace has changed")?;
            if self.model.editing_kit_is_read_only(index) { return Err("tool.exe requires a writable editing kit".to_owned()); }
            let kit = &self.model.kits[index];
            if self.views[stamp.kit].edit_buffers.has_changed_prefix(&format!("{key}|")) { return Err("Finish or cancel the current edit before running tool.exe".to_owned()); }
            if import && kit.parsed_tags.values().any(|document| document.dirty.is_set()) || kit.parsed_tags.get(key).is_some_and(|document| document.dirty.is_set()) {
                return Err("Save tag changes before running tool.exe".to_owned());
            }
            if self.kit_tools.terminal.running { return Err("A command is already running".to_owned()); }
            let layout = self.model.kit_layout_for(index).ok_or("No editing kit folder")?;
            if !layout.root.join("tool.exe").is_file() { return Err("tool.exe was not found in this editing kit".to_owned()); }
            let source = kit.source.as_ref().ok_or("Source no longer available")?;
            let entry = source.full_entry_set().iter().find(|entry| entry.key == key).ok_or("Tag no longer available")?;
            if entry.group_tag == strings::HUD_GROUP { return Err("Use Baboon to import or export HUD messages.".to_owned()); }
            let relative = PathBuf::from(entry.display_path.replace('\\', "/")).with_extension("");
            let command = tool_string_command(source.game, import, &relative)?;
            let documents = if import {
                kit.parsed_tags.iter().filter(|(_, document)| strings::read_string_list(&document.tag).is_some())
                    .map(|(key, document)| (key.clone(), document.id, document.dirty.revision())).collect()
            } else { Vec::new() };
            let source_session = kit.source_session;
            self.model.active = index;
            self.submit_terminal_command(command, ctx.clone());
            if !self.kit_tools.terminal.running { return Err(self.model.status.clone()); }
            if import {
                self.export.pending_string_tool_reload = Some(StringToolReload {
                    run_id: self.kit_tools.terminal.running_id.unwrap(), kit: stamp.kit, source_session, documents,
                });
            }
            self.dialogs.close::<StringTransferWindow>();
            Ok(())
        })();
        if let Err(error) = result {
            self.model.status = error.clone();
            if let Some(window) = self.dialogs.get_mut::<StringTransferWindow>() { window.error = Some(error); }
        }
    }

    pub(in crate::app) fn finish_tool_string_reload(&mut self, run_id: u64) {
        if !self.export.pending_string_tool_reload.as_ref().is_some_and(|pending| pending.run_id == run_id) { return; }
        let pending = self.export.pending_string_tool_reload.take().unwrap();
        let Some(index) = self.model.resolve_kit(pending.kit) else { return; };
        if self.model.kits[index].source_session != pending.source_session { return; }
        let mut reloaded = 0;
        let mut skipped = 0;
        for (key, id, revision) in pending.documents {
            let Some(document) = self.model.kits[index].parsed_tags.get(&key) else { continue; };
            if document.id != id || document.dirty.revision() != revision || document.dirty.is_set()
                || self.views[pending.kit].edit_buffers.has_changed_prefix(&format!("{key}|")) {
                skipped += 1;
                continue;
            }
            let Some(source) = self.model.kits[index].source.as_ref() else { continue; };
            let Some(entry) = source.full_entry_set().iter().find(|entry| entry.key == key) else { continue; };
            match read_entry(&source.source, entry) {
                Ok(tag) => {
                    if tag.write_to_bytes().ok() != document.tag.write_to_bytes().ok() {
                        self.model.kits[index].parsed_tags.insert(key.clone(), TagDocument::clean(tag));
                        self.views[pending.kit].edit_buffers.retain(|draft, _| !draft.starts_with(&format!("{key}|")));
                        reloaded += 1;
                    }
                }
                Err(error) => { self.model.status = format!("Tool finished; could not reload {key}: {error}"); return; }
            }
        }
        self.model.status = format!("Tool finished; reloaded {reloaded} changed string lists; preserved {skipped} documents with newer edits");
    }
}
