use super::*;
use crate::app::mods::review::classify_overlay;
use crate::app::mods::export::default_mod_export_folder;
use crate::app::mods::in_place::ContainerSaveRoute;
use crate::app::mods::in_place::container_save_route;
use crate::app::mods::export::mod_output_path;

#[test]
fn a_mod_is_written_into_a_folder_of_its_own_under_mods() {
    // The name the user chose becomes the folder; `_P` is a property of the
    // container, not part of what they called it.
    assert_eq!(
        mod_output_path(PathBuf::from("D:/Game/Paks/coolmod_P.utoc")),
        PathBuf::from("D:/Game/Paks/~mods/coolmod/coolmod_P.utoc")
    );
    assert_eq!(
        mod_output_path(PathBuf::from("D:/Game/Paks/coolmod_p.utoc")),
        PathBuf::from("D:/Game/Paks/~mods/coolmod/coolmod_p.utoc")
    );
    // A name with no suffix keeps its whole stem as the folder.
    assert_eq!(
        mod_output_path(PathBuf::from("D:/Game/Paks/plain.utoc")),
        PathBuf::from("D:/Game/Paks/~mods/plain/plain.utoc")
    );
}

#[test]
fn a_path_already_under_mods_is_left_alone() {
    // Browsing into the mods folder, or into a mod's own folder, must not
    // bury the output another level down each time.
    for path in [
        "D:/Game/Paks/~mods/coolmod_P.utoc",
        "D:/Game/Paks/~mods/coolmod/coolmod_P.utoc",
        "D:/Game/Paks/~MODS/coolmod/coolmod_P.utoc",
    ] {
        assert_eq!(mod_output_path(PathBuf::from(path)), PathBuf::from(path));
    }
}

#[test]
fn saving_a_container_tag_never_touches_the_game_without_expert_mode() {
    // The confirmation preference is irrelevant outside expert mode: there
    // is nothing destructive left for it to guard. A user who once ticked
    // "don't ask again" must not silently get the in-place write back.
    for confirm in [true, false] {
        assert_eq!(
            container_save_route(false, confirm),
            ContainerSaveRoute::ExportReview,
            "confirm = {confirm}"
        );
    }
}

#[test]
fn expert_mode_keeps_both_in_place_routes() {
    assert_eq!(
        container_save_route(true, true),
        ContainerSaveRoute::ConfirmOverwriteInPlace
    );
    assert_eq!(
        container_save_route(true, false),
        ContainerSaveRoute::OverwriteInPlace
    );
}

#[test]
fn export_mod_defaults_into_the_games_own_mods_folder() {
    assert_eq!(
        default_mod_export_folder(Path::new("D:/Game/Meteorite/Content/Paks")),
        PathBuf::from("D:/Game/Meteorite/Content/Paks/~mods")
    );
}

#[test]
fn the_default_export_creates_mods_when_it_is_missing() {
    // The one behaviour that must survive the destination change: a first
    // export into a `Paks` folder that has never had a mod in it makes
    // `~mods` rather than failing.
    let paks = std::env::temp_dir().join(format!(
        "baboon-export-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&paks).expect("a Paks folder to export into");
    let mods = default_mod_export_folder(&paks);
    assert!(!mods.exists(), "the fixture starts without a ~mods folder");

    ensure_export_directory(&mods.join("mymod_P.utoc")).expect("created");

    assert!(mods.is_dir(), "~mods was created for the export");
    // And the files land directly in it — no folder named after the mod.
    assert_eq!(mods.join("mymod_P.utoc").parent(), Some(mods.as_path()));
    let _ = fs::remove_dir_all(&paks);
}

#[test]
fn a_copy_baboon_authored_exports_as_a_new_package() {
    // A duplicate mounts as an ordinary container tag, so without the
    // ledger's word for it the export would build a field override against
    // a package that exists only inside the mod it was copied into.
    let entry = TagEntry {
        key: "ublock:mymod_P:Tags/objects/copy-biped.ubulk".to_owned(),
        display_path: "objects/copy.biped".to_owned(),
        group_tag: parse_group_tag("bipd").unwrap(),
        group_name: Some("biped".to_owned()),
        location: TagEntryLocation::Container {
            container: 0,
            rel_path: "Tags/objects/copy-biped.ubulk".to_owned(),
        },
    };
    let package = "/Game/Tags/objects/copy-biped".to_owned();

    let (_, _, authored_kind, authored_package) =
        crate::app::mods::campaign_entry_project_parts_with(&entry, Some(package.clone()))
            .expect("a container entry has project parts");
    assert_eq!(authored_kind, CampaignProjectTagKind::New);
    assert_eq!(authored_package.as_deref(), Some(package.as_str()));
    // `New` never reaches the "identical to the game's copy" branch: there
    // is no shipped copy for it to be identical to.
    assert_eq!(
        classify_overlay(true, authored_kind, true),
        ModExportChange::New
    );

    // The same entry with nothing in the ledger is still what it looks
    // like: an edit to a tag the game ships.
    let (_, _, shipped_kind, shipped_package) =
        crate::app::mods::campaign_entry_project_parts_with(&entry, None).expect("project parts");
    assert_eq!(shipped_kind, CampaignProjectTagKind::Existing);
    assert_eq!(shipped_package, None);
    assert_eq!(
        classify_overlay(true, shipped_kind, false),
        ModExportChange::Modified
    );
}
