//! The Material Editor's hover, drawn: a real model of each game through the
//! real renderer, headless, with and without its first material lit.

use super::super::harness::shot::{SHOT_SIZE, Shot, ShotOptions, shoot};
use crate::core::game::GameId;
use crate::core::source::TagSource;

/// A biped of each game whose kit is set.
const MODELS: &[(&str, GameId, &str)] = &[
    ("BLAM_TEST_HCEEK", GameId::HaloCe, "characters\\cyborg\\cyborg"),
    ("BLAM_TEST_H2EK", GameId::Halo2, "objects\\characters\\masterchief\\masterchief"),
    ("BLAM_TEST_H3EK", GameId::Halo3, "objects\\characters\\masterchief\\masterchief"),
    ("BLAM_TEST_HREK", GameId::HaloReach, "objects\\characters\\spartans\\spartans"),
    ("BLAM_TEST_H4EK", GameId::Halo4, "objects\\characters\\storm_masterchief\\storm_masterchief"),
];

/// Pixels that differ between two shots, as a mask.
fn differs(a: &[u8], b: &[u8]) -> Vec<bool> {
    a.chunks_exact(4).zip(b.chunks_exact(4)).map(|(a, b)| a[..3] != b[..3]).collect()
}

/// `mask` grown by two pixels each way: the highlight's edges are lines,
/// which can fall a pixel or two past a part's silhouette.
fn grown(mask: &[bool]) -> Vec<bool> {
    const BY: usize = 2;
    let [width, height] = SHOT_SIZE.map(|edge| edge as usize);
    (0..mask.len())
        .map(|at| {
            let (x, y) = (at % width, at / width);
            (y.saturating_sub(BY)..=(y + BY).min(height - 1))
                .any(|y| (x.saturating_sub(BY)..=(x + BY).min(width - 1)).any(|x| mask[y * width + x]))
        })
        .collect()
}

/// Keep `pixels` as a PNG under `BABOON_HIGHLIGHT_SHOTS`, when it is set.
fn keep(name: &str, pixels: &[u8]) {
    let Some(folder) = std::env::var_os("BABOON_HIGHLIGHT_SHOTS") else { return };
    let row = SHOT_SIZE[0] as usize * 4;
    let upright: Vec<u8> = pixels.chunks_exact(row).rev().flatten().copied().collect();
    let image = image::RgbaImage::from_raw(SHOT_SIZE[0], SHOT_SIZE[1], upright).expect("a whole shot");
    let folder = std::path::PathBuf::from(folder);
    let _ = std::fs::create_dir_all(&folder);
    image.save(folder.join(format!("{name}.png"))).expect("save the shot");
}

/// Lighting a material changes the picture where that material draws, and
/// nowhere else, in amber; under Game Shaders and without, on the dark
/// background and the light one. Where it draws is where hiding it changes
/// the picture.
#[test]
fn the_highlight_lights_its_material_and_nothing_else() {
    let mut ran = 0;
    for (variable, game, model) in MODELS {
        let Ok(root) = std::env::var(variable) else {
            eprintln!("skipping {game:?}: {variable} is not set");
            continue;
        };
        let source = TagSource::LooseFolder {
            root: root.into(),
            game: Some(*game),
            definitions_root: crate::core::bundled::locate_definitions_root(),
        };
        for shaders in [false, true] {
            for light_theme in [false, true] {
                let shot = Shot {
                    model: (*model).to_owned(),
                    shaders,
                    lights: true,
                    yaw: 0.6,
                    pitch: 0.25,
                    kind: Some("biped".to_owned()),
                };
                let options = |highlight: Option<usize>, without: Option<usize>| {
                    let mut options = ShotOptions::new(*game);
                    options.light_theme = light_theme;
                    options.highlight = highlight;
                    options.without_material = without;
                    options
                };
                let shown = shoot(
                    &source,
                    &[
                        (shot.clone(), options(None, None)),
                        (shot.clone(), options(Some(0), None)),
                        (shot, options(None, Some(0))),
                    ],
                );
                let (plain, lit, without) = (&shown[0].pixels, &shown[1].pixels, &shown[2].pixels);
                let lit_changed = differs(plain, lit);
                let footprint = grown(&differs(plain, without));
                let changed = lit_changed.iter().filter(|&&changed| changed).count();
                let outside = lit_changed.iter().zip(&footprint).filter(|(lit, inside)| **lit && !**inside).count();
                let label = format!("{game:?} {model} shaders {shaders} light {light_theme}");
                eprintln!("{label}: {changed} lit, {outside} outside its material");
                let name = format!("{game:?}_{}_{}", if shaders { "shaders" } else { "textured" }, if light_theme { "light" } else { "dark" });
                keep(&format!("{name}_plain"), plain);
                keep(&format!("{name}_lit"), lit);
                assert!(changed > 200, "{label}: the highlight changed {changed} pixels");
                assert!(outside * 50 < changed, "{label}: {outside} of {changed} lit pixels are not its material's");
                let amber = lit
                    .chunks_exact(4)
                    .zip(&lit_changed)
                    .filter(|(pixel, changed)| **changed && (pixel[0] > pixel[2] || pixel[..3].iter().all(|&channel| channel >= 240)))
                    .count();
                // Amber fill, or its white edges, which are most of a small part.
                assert!(amber * 10 > changed * 9, "{label}: {amber} of {changed} lit pixels are amber or its edges");
                ran += 1;
            }
        }
    }
    eprintln!("{ran} looks checked");
}
