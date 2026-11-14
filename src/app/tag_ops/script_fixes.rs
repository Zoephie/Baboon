//! Tag paths in mission scripts for Rename, Move and Delete: the check their
//! windows show, and the fix their jobs apply. The logic is
//! [`crate::core::script_paths`]; this is how the windows and jobs use it.
use super::*;
use crate::app::browser::bulk_rename::ScriptRows;
use crate::core::folder_merge::Transaction;
use crate::core::script_paths::{
    ChangeKind, ScriptChanges, ScriptFixSummary, ScriptKit, ScriptReport, check_scripts, fix_scripts,
};
use std::sync::mpsc::Receiver;

fn loose_source(root: &Path, game: Option<GameId>) -> TagSource {
    TagSource::LooseFolder { root: root.to_owned(), game, definitions_root: locate_definitions_root() }
}

/// Renames from tags' paths before and after, paired in order.
pub(in crate::app) fn renames(old: &[TagEntry], new: &[TagEntry]) -> ScriptChanges {
    ScriptChanges::renaming(old.iter().zip(new).map(|(old, new)| (old.display_path.as_str(), Some(new.display_path.as_str()))))
}

/// Which scripts `changes` affects, for a review. A game without checked
/// scripts, or a check that fails, reports none.
pub(in crate::app) fn check(
    root: &Path,
    game: Option<GameId>,
    entries: &[TagEntry],
    changes: &ScriptChanges,
    cancelled: &dyn Fn() -> bool,
) -> ScriptReport {
    let empty = ScriptReport { kind: changes.kind(), scripts: Vec::new() };
    let Some(game) = game else { return empty };
    let source = loose_source(root, Some(game));
    let kit = ScriptKit { tags_root: root, game, source: &source, entries };
    check_scripts(&kit, changes, cancelled).unwrap_or(empty)
}

/// Rewrite the script paths a rename or move changes, recording every file
/// in `transaction`. `relocate` maps a tag's path before the operation to
/// where it is now.
pub(in crate::app) fn fix(
    root: &Path,
    game: Option<GameId>,
    entries: &[TagEntry],
    changes: &ScriptChanges,
    relocate: &dyn Fn(&Path) -> PathBuf,
    transaction: &mut Transaction,
) -> Result<ScriptFixSummary, String> {
    let Some(game) = game else { return Ok(ScriptFixSummary::default()) };
    let source = loose_source(root, Some(game));
    let kit = ScriptKit { tags_root: root, game, source: &source, entries };
    fix_scripts(&kit, changes, relocate, transaction)
}

/// For a job without a transaction of its own: fix the scripts before its
/// files move, in a transaction that undoes them unless committed (dropping
/// it restores every script). Commit once the files have moved.
pub(in crate::app) fn fix_before_move(
    root: &Path,
    game: Option<GameId>,
    entries: &[TagEntry],
    changes: &ScriptChanges,
) -> Result<(Transaction, ScriptFixSummary), String> {
    let mut transaction = Transaction::new()?;
    let summary = fix(root, game, entries, changes, &|path| path.to_path_buf(), &mut transaction)?;
    Ok((transaction, summary))
}

/// Add what fixing scripts did to a job's status and terminal lines.
pub(in crate::app) fn report(summary: &ScriptFixSummary, status: &mut String, lines: &mut Vec<String>) {
    if summary.fixed > 0 {
        status.push_str(&format!(", and {} path(s) in {} script(s)", summary.fixed, summary.scripts));
    }
    if summary.cant_fix > 0 {
        status.push_str(&format!("; {} script path(s) could not be fixed (see terminal)", summary.cant_fix));
    }
    lines.extend(summary.line());
}

/// A script check a window runs in the background while it is open.
pub(in crate::app) struct ScriptCheck {
    game: Option<GameId>,
    kind: ChangeKind,
    pending: Option<Receiver<ScriptReport>>,
    report: Option<ScriptReport>,
}

impl ScriptCheck {
    /// Start checking `changes` against the kit's scripts. Nothing runs for
    /// a game whose scripts Baboon does not check.
    pub fn start(ctx: &egui::Context, root: PathBuf, game: Option<GameId>, entries: Vec<TagEntry>, changes: ScriptChanges) -> Self {
        let kind = changes.kind();
        let checked = crate::core::script_paths::script_support(game) == Some(crate::core::script_paths::ScriptSupport::Checked);
        if !checked || changes.is_empty() {
            return Self { game, kind, pending: None, report: Some(ScriptReport { kind, scripts: Vec::new() }) };
        }
        let (sender, receiver) = mpsc::channel();
        // A check that panics reports no scripts rather than leaving the
        // window checking.
        crate::app::shell::spawn_worker(
            &sender,
            ctx,
            move || check(&root, game, &entries, &changes, &|| false),
            move |_| ScriptReport { kind, scripts: Vec::new() },
        );
        Self { game, kind, pending: Some(receiver), report: None }
    }

    /// A window's script rows: starts its check (`slot`) the first time, on
    /// the tags kit `kit` lists, and polls it after. `changes` picks the tags
    /// the window changes from that list. A kit that is not a loose folder
    /// has no scripts to check.
    pub fn rows_for(
        slot: &mut Option<ScriptCheck>,
        cx: &Ctx,
        kit: KitId,
        changes: impl FnOnce(&[TagEntry]) -> ScriptChanges,
    ) -> ScriptRows {
        if slot.is_none() {
            let Some(source) = cx.model.kit_index(kit).and_then(|index| cx.model.kits[index].source.as_ref()) else {
                return ScriptRows::default();
            };
            let TagSource::LooseFolder { root, game, .. } = &source.source else {
                return ScriptRows::default();
            };
            let entries = if source.all_entries.is_empty() { source.entries.clone() } else { source.all_entries.clone() };
            let changes = changes(&entries);
            *slot = Some(Self::start(cx.egui, root.clone(), *game, entries, changes));
        }
        slot.as_mut().map(Self::rows).unwrap_or_default()
    }

    /// The rows to draw, once the check has finished.
    pub fn rows(&mut self) -> ScriptRows {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(report) => {
                    self.report = Some(report);
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.report = Some(ScriptReport { kind: self.kind, scripts: Vec::new() });
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        ScriptRows::new(self.game, self.kind, self.report.as_ref())
    }

}

/// A throwaway Halo 2 kit whose mission script names a grunt in several
/// ways.
#[cfg(test)]
pub(in crate::app) mod fixture {
    use super::*;
    use crate::app::loose_fixture::LooseKit;
    use blam_tags::TagFieldData;

    pub(in crate::app) const SCRIPT: &str = r#"(global object_definition grunt_type "objects\characters\grunt\grunt")
(script dormant shake
	(custom_animation (ai_get_unit x) "objects\characters\grunt\grunt" "combat:shake" true)
	(predict_model_section objects\characters\grunt\grunt 4)
	(object_type_predict "objects\characters\grunt\grunt")
	(mystery_function "objects\characters\grunt\grunt"))
"#;
    pub(in crate::app) const GRUNT: &str = "objects/characters/grunt/grunt";
    pub(in crate::app) const OLD: &str = r"objects\characters\grunt\grunt";
    pub(in crate::app) const SCENARIO: &str = "scenarios/solo/03a/03a.scenario";

    /// `grunt` as a biped, render model and animation graph; a scenario
    /// holding a copy of the script; the script in `data`.
    pub(in crate::app) fn kit(name: &str) -> LooseKit {
        let kit = LooseKit::new(name, "halo2_mcc");
        for group in ["biped", "render_model", "model_animation_graph"] {
            kit.write_mcc(GRUNT, group, |_| {});
        }
        kit.write_mcc("scenarios/solo/03a/03a", "scenario", |tag| {
            {
                let mut root = tag.root_mut();
                let mut field = root.field_path_mut("source files").unwrap();
                field.as_block_mut().unwrap().add_element();
            }
            let mut root = tag.root_mut();
            root.field_path_mut("source files[0]/name").unwrap().set(TagFieldData::String("03a_mission".into())).unwrap();
            root.field_path_mut("source files[0]/source").unwrap().set(TagFieldData::Data(SCRIPT.as_bytes().to_vec())).unwrap();
        });
        let data = data_script(&kit);
        fs::create_dir_all(data.parent().unwrap()).unwrap();
        fs::write(&data, SCRIPT).unwrap();
        kit
    }

    pub(in crate::app) fn data_script(kit: &LooseKit) -> PathBuf {
        kit.root.parent().unwrap().join("data/scenarios/solo/03a/scripts/03a_mission.hsc")
    }

    pub(in crate::app) fn data_text(kit: &LooseKit) -> String {
        fs::read_to_string(data_script(kit)).unwrap()
    }

    /// The scenario's saved copy, the scenario being at `scenario`.
    pub(in crate::app) fn saved_copy(kit: &LooseKit, scenario: &str) -> String {
        let tag = blam_tags::TagFile::read(kit.path(scenario)).unwrap();
        match tag.root().field_path("source files[0]/source").unwrap().value().unwrap() {
            TagFieldData::Data(bytes) => String::from_utf8(bytes).unwrap(),
            other => panic!("not data: {other:?}"),
        }
    }

    /// The script with `old` replaced on the lines holding any of `markers`.
    pub(in crate::app) fn with_lines_replaced(markers: &[&str], old: &str, new: &str) -> String {
        SCRIPT
            .split_inclusive('\n')
            .map(|line| if markers.iter().any(|marker| line.contains(marker)) { line.replace(old, new) } else { line.to_owned() })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::app::loose_fixture::*;

    const COVENANT: &str = r"objects\characters\covenant\grunt\grunt";
    const RENAMED: &str = r"objects\characters\grunt_v2\grunt";

    fn entry(kit: &LooseKit, path: &str) -> TagEntry {
        kit.entries().into_iter().find(|entry| entry.display_path.replace('\\', "/") == path).unwrap()
    }

    fn rename_folder(kit: &LooseKit) -> Result<crate::app::shell::FolderRefactorFinished, String> {
        let (tx, _rx) = mpsc::channel();
        run_folder_refactor_job(
            kit.root.clone(), PathBuf::from("objects/characters/grunt"), kit.path("objects/characters"),
            Some("grunt_v2".into()), true, "Renaming folder".into(), kit.names(), Some(GameId::Halo2),
            kit.entries(), Some(kit.index()), &tx,
        )
    }

    #[test]
    fn moving_one_tag_fixes_the_paths_that_mean_it_in_data_and_the_scenario() {
        let kit = kit("script-fixes-move-tag");
        let (tx, _rx) = mpsc::channel();
        let done = run_tag_rename_job(
            kit.root.clone(), entry(&kit, "objects/characters/grunt/grunt.biped"),
            COVENANT.into(), "Moving tag".into(), kit.names(),
            Some(GameId::Halo2), kit.entries(), Some(kit.index()), &tx,
        ).unwrap();
        // Only the paths that want an object mean the biped; the animation
        // graph and render model stay. The unknown function is left alone.
        let expected = with_lines_replaced(&["global", "object_type_predict"], OLD, COVENANT);
        assert_eq!(data_text(&kit), expected);
        assert_eq!(saved_copy(&kit, SCENARIO), expected);
        assert!(done.status.contains("2 path(s) in 1 script(s)"), "{}", done.status);
        assert!(done.status.contains("1 script path(s) could not be fixed"), "{}", done.status);
        assert!(done.lines.iter().any(|line| line.contains("Updated 2 path(s) in 1 script(s)")), "{:?}", done.lines);
    }

    #[test]
    fn renaming_a_folder_fixes_every_path_to_its_tags() {
        let kit = kit("script-fixes-rename-folder");
        let done = rename_folder(&kit).unwrap();
        let expected = SCRIPT.replace(OLD, RENAMED);
        assert_eq!(data_text(&kit), expected);
        assert_eq!(saved_copy(&kit, SCENARIO), expected);
        assert!(done.status.contains("5 path(s) in 1 script(s)"), "{}", done.status);
    }

    #[test]
    fn a_script_that_cannot_be_written_stops_a_rename_before_anything_moves() {
        let kit = kit("script-fixes-read-only");
        let data = data_script(&kit);
        let mut permissions = fs::metadata(&data).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&data, permissions.clone()).unwrap();
        let error = rename_folder(&kit).err().expect("a read-only script refuses the rename");
        assert!(error.contains("read-only"), "{error}");
        assert!(kit.path("objects/characters/grunt/grunt.biped").exists(), "nothing moved");
        assert_eq!(saved_copy(&kit, SCENARIO), SCRIPT, "the scenario's copy is untouched");
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&data, permissions).unwrap();
        assert_eq!(data_text(&kit), SCRIPT);
    }

    #[test]
    fn bulk_rename_fixes_scripts_in_its_one_batch() {
        let kit = kit("script-fixes-bulk-rename");
        let (tx, _rx) = mpsc::channel();
        let context = folder_merge_ops::MergeContext {
            recovery: Err("Rename must not overwrite anything".into()),
            control: std::sync::Arc::new(super::super::folder_merge_review::MergeControl::default()),
        };
        let done = folder_merge_ops::run_renames(
            kit.root.clone(),
            vec![(kit.path("objects/characters/grunt"), kit.path("objects/characters/grunt_v2"))],
            kit.names(), Some(GameId::Halo2), kit.entries(), Some(kit.index()), context, &tx,
        ).unwrap();
        let expected = SCRIPT.replace(OLD, RENAMED);
        assert_eq!(data_text(&kit), expected);
        assert_eq!(saved_copy(&kit, SCENARIO), expected);
        assert!(done.status.contains("5 path(s) in 1 script(s)"), "{}", done.status);
    }

    #[test]
    fn the_bulk_rename_window_lists_the_script_once_it_is_checked() {
        let kit = kit("script-fixes-rename-window");
        let mut app = app();
        kit.install_indexed(&mut app);
        let key = kit.key("objects/characters/grunt/grunt.biped");
        app.begin_browser_bulk("pane".into(), vec![key], crate::app::browser::selection::BulkAction::Rename, &ctx());
        let mut driver = PromptDriver::new();
        let mut page = driver.frame(&mut app, Vec::new());
        let started = std::time::Instant::now();
        while !page.iter().any(|(text, _)| text.contains("03a_mission.hsc")) {
            assert!(started.elapsed().as_secs() < 20, "the script row never appeared: {page:?}");
            std::thread::sleep(std::time::Duration::from_millis(20));
            page = driver.frame(&mut app, Vec::new());
        }
        // The biped alone: the global and object_type_predict want an object;
        // the unknown function can't be fixed.
        for label in [
            "1 Reference to Fix",
            "2 paths",
            "· 1 can't fix",
            "Scripts are updated in their .hsc file in data\\ and in the copy saved in their scenario.",
        ] {
            assert!(page.iter().any(|(text, _)| text == label), "missing {label}: {page:?}");
        }
    }

    #[test]
    fn script_rows_say_what_breaks_and_what_cannot_be_fixed() {
        use crate::app::browser::bulk_rename::ScriptRows;
        let kit = kit("script-fixes-rows");
        let entries = kit.entries();
        let biped = r"objects\characters\grunt\grunt.biped";
        let changes = ScriptChanges::renaming([(biped, Some(r"objects\characters\covenant\grunt\grunt.biped"))]);
        let report = check(&kit.root, Some(GameId::Halo2), &entries, &changes, &|| false);
        let rows = ScriptRows::new(Some(GameId::Halo2), changes.kind(), Some(&report));
        assert_eq!(rows.rows.len(), 1);
        let row = &rows.rows[0];
        assert_eq!(row.path, r"scenarios\solo\03a\scripts\03a_mission.hsc");
        assert_eq!((row.count.as_str(), row.problem.as_deref()), ("2 paths", Some("· 1 can't fix")));
        let fixed = format!("Line 1: {OLD} → {COVENANT} (object_definition)");
        assert!(row.tooltip.contains(&fixed), "{}", row.tooltip);
        assert!(row.tooltip.contains("can't fix: it could mean more than one tag"), "{}", row.tooltip);

        let deleting = ScriptChanges::deleting([biped]);
        let report = check(&kit.root, Some(GameId::Halo2), &entries, &deleting, &|| false);
        let rows = ScriptRows::new(Some(GameId::Halo2), deleting.kind(), Some(&report));
        assert_eq!((rows.rows[0].count.as_str(), rows.rows[0].problem.as_deref()), ("2 paths", Some("· 1 may break")));
        assert!(rows.notes[0].starts_with("These scripts won't compile"));

        let checking = ScriptRows::new(Some(GameId::Halo2), ChangeKind::Rename, None);
        assert!(checking.checking && checking.notes.is_empty());
        assert_eq!(checking.note_lines(), 1, "the spinner line takes room in the box");
        assert_eq!(
            ScriptRows::new(Some(GameId::Halo4), ChangeKind::Rename, Some(&report)).notes,
            [crate::core::script_paths::NOT_CHECKED_NOTE]
        );
    }
}
