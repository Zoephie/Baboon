use super::*;

fn dialog(name: &str, folder: &str) -> ModExportDialog {
    ModExportDialog {
        kit: KitId(1),
        review_only: false,
        snapshot: CampaignProjectSnapshot {
            game: "haloce_evolved".to_owned(),
            source_path: PathBuf::new(),
            selected_identity: None,
            tabs: Vec::new(),
            overlays: HashMap::new(),
            history: Default::default(),
            folders: Default::default(),
        },
        rows: Vec::new(),
        name: name.to_owned(),
        folder: PathBuf::from(folder),
        overwrite_acknowledged: false,
        expanded: HashSet::new(),
        diffs: HashMap::new(),
        controls_height: 0.0,
    }
}

#[test]
fn the_mod_name_names_the_files_and_not_a_folder() {
    let dialog = dialog("Cool Mod", "D:/Game/Paks/~mods");
    // `_P` belongs to the files, because it is what gives the container
    // priority rather than part of the name the user typed.
    assert_eq!(dialog.destination(), PathBuf::from("D:/Game/Paks/~mods"));
    assert_eq!(dialog.stem(), "Cool-Mod_P");
    assert_eq!(
        dialog.output_utoc(),
        PathBuf::from("D:/Game/Paks/~mods/Cool-Mod_P.utoc")
    );
}

#[test]
fn a_chosen_folder_is_the_folder_written_to() {
    // Every shape of picked folder, including one that is not `~mods` at
    // all: what was picked is where the files go. Anything else made
    // "Browse..." mean "browse to the parent of where I want this".
    for folder in [
        "D:/Game/Paks",
        "D:/Game/Paks/~mods",
        "D:/Game/Paks/~MODS",
        "D:/Somewhere/Else",
    ] {
        let dialog = dialog("coolmod", folder);
        assert_eq!(dialog.destination(), PathBuf::from(folder));
        assert_eq!(
            dialog.output_utoc(),
            PathBuf::from(folder).join("coolmod_P.utoc")
        );
    }
}

#[test]
fn a_name_that_sanitizes_to_nothing_still_names_a_file() {
    // The export button is disabled for an empty name, but the file names
    // are shown while it is being typed and a bare `_P.utoc` reads like a
    // bug rather than like an unfinished name.
    assert_eq!(dialog("///", "D:/Game/Paks").stem(), "mod_P");
}
