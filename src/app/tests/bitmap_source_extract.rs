//! Recovering a bitmap tag's source image to a folder the user picks.
//! `BLAM_TEST_HCEEK` names a Halo CE kit's `tags` folder.

use super::*;

fn ce_bitmap(tags: &Path, rel: &str) -> TagEntry {
    TagEntry {
        key: format!("file:{}", tags.join(rel).display()),
        display_path: rel.to_owned(),
        group_tag: u32::from_be_bytes(*b"bitm"),
        group_name: Some("bitmap".to_owned()),
        location: TagEntryLocation::LooseFile(tags.join(rel)),
    }
}

const WHITE: &str = "ui/shell/bitmaps/white.bitmap";
/// The one stock CE bitmap saved without its color plate.
const NO_PLATE: &str = "digsite/placeholder/000-000-000-000-invisible.bitmap";

fn ce_kit() -> Option<(PathBuf, TagSource)> {
    let tags = PathBuf::from(crate::test_kits::tag_path("haloce_mcc", ""));
    if !tags.join(WHITE).is_file() || !tags.join(NO_PLATE).is_file() {
        eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
        return None;
    }
    let source = TagSource::LooseFolder {
        root: tags.clone(),
        game: Some("haloce_mcc".to_owned()),
        definitions_root: crate::test_kits::definitions().to_path_buf(),
    };
    Some((tags, source))
}

#[test]
fn a_bitmaps_source_lands_in_the_picked_folder() {
    let Some((tags, source)) = ce_kit() else {
        return;
    };
    let entry = ce_bitmap(&tags, WHITE);
    let out = crate::test_kits::unique_temp_dir("bitmap-source");

    let plate = blam_tags::bitmap::color_plate(&read_entry(&source, &entry).unwrap())
        .unwrap()
        .expect("white.bitmap carries its color plate");
    assert_eq!((plate.width, plate.height), (32, 32));
    // The white image sits on a color plate: Tool's key colors around it
    // (blue background, magenta sequence divider, cyan registration). With
    // red and blue swapped they'd read as red and yellow.
    let mut colors: Vec<[u8; 4]> = plate
        .rgba
        .chunks_exact(4)
        .map(|px| [px[0], px[1], px[2], px[3]])
        .collect();
    colors.sort();
    colors.dedup();
    assert_eq!(
        colors,
        [
            [0, 0, 255, 255],
            [0, 255, 255, 255],
            [255, 0, 255, 255],
            [255, 255, 255, 255]
        ]
    );

    // One tag lands in the picked folder itself.
    extract_bitmap_source(&source, &entry, &out).unwrap();
    let path = out.join("white.tif");
    let written = fs::read(&path).unwrap();
    let mut expected = Vec::new();
    plate.write_tiff(&mut expected).unwrap();
    assert_eq!(written, expected);

    // A second extraction must not replace what is there: it may be the
    // artist's own source by then.
    fs::write(&path, b"artist's edit").unwrap();
    let error = extract_bitmap_source(&source, &entry, &out).unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"artist's edit");

    let _ = fs::remove_dir_all(&out);
}

#[test]
fn a_bitmap_without_a_source_says_so_and_writes_nothing() {
    let Some((tags, source)) = ce_kit() else {
        return;
    };
    let out = crate::test_kits::unique_temp_dir("bitmap-source-none");
    let entry = ce_bitmap(&tags, NO_PLATE);

    let error = extract_bitmap_source(&source, &entry, &out).unwrap_err();
    assert!(error.to_string().contains("no source image"), "{error}");
    assert!(!out.join("000-000-000-000-invisible.tif").exists());

    // In a folder extract it is reported, not fatal to the rest, and each
    // tag keeps its folder.
    let entries = [ce_bitmap(&tags, WHITE), entry];
    let status = extract_bitmap_sources(&source, &entries, &out).unwrap();
    assert!(
        status.starts_with("Extracted 1 bitmap source(s)"),
        "{status}"
    );
    assert!(
        status.contains("1 failed") && status.contains(NO_PLATE),
        "{status}"
    );
    assert!(out.join("ui/shell/bitmaps/white.tif").is_file());

    let _ = fs::remove_dir_all(&out);
}

/// The menu items read the game the browser panel publishes, and are offered
/// only where bitmaps keep their source.
#[test]
fn only_ce_and_halo_2_offer_bitmap_source_extraction() {
    let ctx = egui::Context::default();
    let mut offered = Vec::new();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for game in [
                "haloce_mcc",
                "halo2_mcc",
                "halo3_mcc",
                "haloreach_mcc",
                "haloce_evolved",
            ] {
                crate::app::browser::set_browser_game(ui, Some(game.to_owned()));
                if crate::app::browser::browser_game_keeps_bitmap_sources(ui) {
                    offered.push(game);
                }
            }
            crate::app::browser::set_browser_game(ui, None);
            assert!(!crate::app::browser::browser_game_keeps_bitmap_sources(ui));
        });
    });
    assert_eq!(offered, ["haloce_mcc", "halo2_mcc"]);
}
