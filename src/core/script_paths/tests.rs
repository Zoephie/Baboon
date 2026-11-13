use super::*;
use crate::core::bundled::locate_definitions_root;

/// A throwaway Halo 2 kit: `tags\` with the given tags (empty files, except
/// a scenario holding a copy of the mission script), and the script itself
/// in `data\`.
struct Kit {
    base: PathBuf,
    tags: PathBuf,
    entries: Vec<TagEntry>,
    source: TagSource,
}

const SCENARIO: &str = "scenarios\\solo\\03a\\03a.scenario";
const DATA_SCRIPT: &str = "scenarios\\solo\\03a\\scripts\\03a_mission.hsc";

const SCRIPT: &str = "; the grunts: objects\\characters\\grunt\\grunt.\n\
(global object_definition grunt_type \"objects\\characters\\grunt\\grunt\")\n\
(script dormant shake\n\
\t(custom_animation (ai_get_unit x) \"objects\\characters\\grunt\\grunt\" \"combat:shake\" true)\n\
\t(predict_model_section objects\\characters\\grunt\\grunt 4)\n\
\t(object_type_predict \"objects\\weapons\\rifle\\sniper_rifle\\sniper_rifle.weapon\")\n\
\t(mystery_function \"objects\\characters\\grunt\\grunt\"))\n";

const GRUNT: [&str; 3] = [
    "objects\\characters\\grunt\\grunt.biped",
    "objects\\characters\\grunt\\grunt.render_model",
    "objects\\characters\\grunt\\grunt.model_animation_graph",
];
const SNIPER: &str = "objects\\weapons\\rifle\\sniper_rifle\\sniper_rifle.weapon";

impl Kit {
    fn new(name: &str) -> Self {
        let base = crate::core::test_kits::unique_temp_dir(name);
        let tags = base.join("H2EK").join("tags");
        fs::create_dir_all(&tags).unwrap();
        let definitions = locate_definitions_root();
        let mut entries = Vec::new();
        let mut add = |display: &str, bytes: Vec<u8>| {
            let path = tags.join(tag_path::to_relative(display));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            let group = tag_path::split_extension(display).unwrap().1;
            let tag = fs::read_to_string(definitions.join("halo2_mcc").join(format!("{group}.json")))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|value| value.get("tag").and_then(|tag| tag.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "????".to_owned());
            let mut four = [b' '; 4];
            four[..tag.len()].copy_from_slice(tag.as_bytes());
            entries.push(TagEntry {
                key: display.to_ascii_lowercase(),
                display_path: display.to_owned(),
                group_tag: u32::from_be_bytes(four),
                group_name: Some(group.to_owned()),
                location: TagEntryLocation::LooseFile(path),
            });
        };
        for path in GRUNT.iter().chain([&SNIPER]) {
            add(path, Vec::new());
        }
        let mut scenario = TagFile::new(definitions.join("halo2_mcc").join("scenario.json")).unwrap();
        {
            let mut root = scenario.root_mut();
            let mut field = root.field_path_mut("source files").unwrap();
            let mut block = field.as_block_mut().unwrap();
            block.add_element();
        }
        let mut root = scenario.root_mut();
        root.field_path_mut("source files[0]/name").unwrap().set(TagFieldData::String("03a_mission".into())).unwrap();
        // The saved copy is lowercase, as Halo 2's are.
        root.field_path_mut("source files[0]/source")
            .unwrap()
            .set(TagFieldData::Data(SCRIPT.to_ascii_lowercase().into_bytes()))
            .unwrap();
        add(SCENARIO, scenario.write_to_bytes().unwrap());

        let data = base.join("H2EK").join("data").join(tag_path::to_relative(DATA_SCRIPT));
        fs::create_dir_all(data.parent().unwrap()).unwrap();
        fs::write(&data, SCRIPT).unwrap();
        let source = TagSource::LooseFolder { root: tags.clone(), game: Some(GameId::Halo2), definitions_root: definitions };
        Self { base, tags, entries, source }
    }

    fn kit(&self) -> ScriptKit<'_> {
        ScriptKit { tags_root: &self.tags, game: GameId::Halo2, source: &self.source, entries: &self.entries }
    }

    fn data_script(&self) -> PathBuf {
        self.base.join("H2EK").join("data").join(tag_path::to_relative(DATA_SCRIPT))
    }

    fn saved_copy(&self) -> String {
        let entry = self.entries.iter().find(|entry| entry.display_path == SCENARIO).unwrap();
        let tag = read_entry(&self.source, entry).unwrap();
        String::from_utf8(scenario_sources(&tag).remove(0).2).unwrap()
    }

    fn check(&self, changes: &ScriptChanges) -> ScriptReport {
        check_scripts(&self.kit(), changes, &|| false).unwrap()
    }
}

impl Drop for Kit {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn moved(paths: &[&str], from: &str, to: &str) -> ScriptChanges {
    let news: Vec<String> = paths.iter().map(|path| path.replacen(from, to, 1)).collect();
    ScriptChanges::renaming(paths.iter().copied().zip(news.iter().map(|new| Some(new.as_str()))))
}

fn outcomes(report: &ScriptReport) -> Vec<(usize, Outcome, Option<String>)> {
    report.scripts[0].hits.iter().map(|hit| (hit.line, hit.outcome, hit.new.clone())).collect()
}

const COVENANT: &str = "objects\\characters\\covenant\\grunt\\grunt";

#[test]
fn moving_a_whole_folder_fixes_every_path_in_both_copies() {
    let kit = Kit::new("script-paths-move-folder");
    let changes = moved(&GRUNT, "objects\\characters\\", "objects\\characters\\covenant\\");
    let report = kit.check(&changes);
    assert_eq!(report.scripts.len(), 1, "one row for the data file and the saved copy");
    let script = &report.scripts[0];
    assert_eq!(script.path(), DATA_SCRIPT);
    assert!(script.data_file);
    assert_eq!(script.tag_copies, 1);
    let fixed = Some(COVENANT.to_owned());
    // The comment on line 1 is not counted.
    assert_eq!(
        outcomes(&report),
        [
            (2, Outcome::Fixed, fixed.clone()),
            (4, Outcome::Fixed, fixed.clone()),
            (5, Outcome::Fixed, fixed.clone()),
            (7, Outcome::Fixed, fixed),
        ]
    );
    assert_eq!(script.hits[0].expected.as_deref(), Some("object_definition"));
    assert_eq!(script.hits[1].expected.as_deref(), Some("animation_graph"));

    let mut transaction = Transaction::new().unwrap();
    let summary = fix_scripts(&kit.kit(), &changes, &|path| path.to_path_buf(), &mut transaction).unwrap();
    transaction.commit();
    assert_eq!(summary, ScriptFixSummary { scripts: 1, fixed: 4, cant_fix: 0, files_written: 2 });
    let expected = SCRIPT.replace("objects\\characters\\grunt", "objects\\characters\\covenant\\grunt");
    assert_eq!(fs::read_to_string(kit.data_script()).unwrap(), expected, "comments are updated too");
    assert_eq!(kit.saved_copy(), expected.to_ascii_lowercase(), "the saved copy keeps its lowercase");
}

#[test]
fn moving_one_of_several_same_named_tags_uses_the_expected_type() {
    let kit = Kit::new("script-paths-move-biped");
    let changes = moved(&GRUNT[..1], "objects\\characters\\", "objects\\characters\\covenant\\");
    let report = kit.check(&changes);
    // The global and object_type_predict want an object: only the biped.
    // custom_animation wants the animation graph and predict_model_section the
    // render model, which stay. The unknown function could mean any of them.
    assert_eq!(
        outcomes(&report),
        [(2, Outcome::Fixed, Some(COVENANT.to_owned())), (7, Outcome::CantFix, None)]
    );

    let mut transaction = Transaction::new().unwrap();
    let summary = fix_scripts(&kit.kit(), &changes, &|path| path.to_path_buf(), &mut transaction).unwrap();
    transaction.commit();
    assert_eq!((summary.fixed, summary.cant_fix), (1, 1));
    let text = fs::read_to_string(kit.data_script()).unwrap();
    assert!(text.contains("(global object_definition grunt_type \"objects\\characters\\covenant\\grunt\\grunt\")"));
    assert!(text.contains("(mystery_function \"objects\\characters\\grunt\\grunt\")"), "left alone");
    assert!(text.contains("; the grunts: objects\\characters\\grunt\\grunt."), "an ambiguous comment is left alone");
}

#[test]
fn a_path_with_an_extension_means_that_tag() {
    let kit = Kit::new("script-paths-extension");
    let changes = moved(&[SNIPER], "objects\\weapons\\rifle\\", "objects\\weapons\\");
    let report = kit.check(&changes);
    assert_eq!(
        outcomes(&report),
        [(6, Outcome::Fixed, Some("objects\\weapons\\sniper_rifle\\sniper_rifle.weapon".to_owned()))]
    );
}

#[test]
fn deleting_counts_only_the_paths_that_will_break() {
    let kit = Kit::new("script-paths-delete");
    let report = kit.check(&ScriptChanges::deleting([GRUNT[0]]));
    assert_eq!(report.kind, ChangeKind::Delete);
    assert_eq!(outcomes(&report), [(2, Outcome::Missing, None), (7, Outcome::MayBreak, None)]);

    let everything = kit.check(&ScriptChanges::deleting(GRUNT));
    assert_eq!(everything.scripts[0].count(Outcome::Missing), 4);

    // Deleting never rewrites.
    let mut transaction = Transaction::new().unwrap();
    let summary = fix_scripts(&kit.kit(), &ScriptChanges::deleting(GRUNT), &|path| path.to_path_buf(), &mut transaction).unwrap();
    transaction.commit();
    assert_eq!(summary, ScriptFixSummary::default());
    assert_eq!(fs::read_to_string(kit.data_script()).unwrap(), SCRIPT);
}

#[test]
fn a_rename_with_no_new_name_yet_is_listed_but_not_written() {
    let kit = Kit::new("script-paths-pending");
    let changes = ScriptChanges::renaming(GRUNT.iter().map(|path| (*path, None)));
    let report = kit.check(&changes);
    assert_eq!(report.scripts[0].count(Outcome::Fixed), 4);
    assert!(report.scripts[0].hits.iter().all(|hit| hit.new.is_none()));
}

#[test]
fn a_rollback_restores_both_copies() {
    let kit = Kit::new("script-paths-rollback");
    let saved = kit.saved_copy();
    let changes = moved(&GRUNT, "objects\\characters\\", "objects\\characters\\covenant\\");
    let mut transaction = Transaction::new().unwrap();
    fix_scripts(&kit.kit(), &changes, &|path| path.to_path_buf(), &mut transaction).unwrap();
    assert_ne!(fs::read_to_string(kit.data_script()).unwrap(), SCRIPT);
    let _ = transaction.rollback("a later step failed".into());
    assert_eq!(fs::read_to_string(kit.data_script()).unwrap(), SCRIPT);
    assert_eq!(kit.saved_copy(), saved);
}

#[test]
fn a_read_only_script_stops_the_fix_before_anything_is_written() {
    let kit = Kit::new("script-paths-read-only");
    let mut permissions = fs::metadata(kit.data_script()).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(kit.data_script(), permissions.clone()).unwrap();
    let changes = moved(&GRUNT, "objects\\characters\\", "objects\\characters\\covenant\\");
    let mut transaction = Transaction::new().unwrap();
    let error = fix_scripts(&kit.kit(), &changes, &|path| path.to_path_buf(), &mut transaction).unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    let _ = transaction.rollback(error);
    assert_eq!(fs::read_to_string(kit.data_script()).unwrap(), SCRIPT);
    // Only so the temporary kit can be removed.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(kit.data_script(), permissions).unwrap();
}

#[test]
fn games_without_checked_scripts_report_nothing() {
    let kit = Kit::new("script-paths-unsupported");
    let changes = moved(&GRUNT, "objects\\characters\\", "objects\\characters\\covenant\\");
    let other = ScriptKit { game: GameId::Halo4, ..kit.kit() };
    assert!(check_scripts(&other, &changes, &|| false).unwrap().scripts.is_empty());
    assert_eq!(script_support(Some(GameId::Halo2Amp)), Some(ScriptSupport::NotChecked));
    assert_eq!(script_support(Some(GameId::HaloReach)), Some(ScriptSupport::Checked));
}
