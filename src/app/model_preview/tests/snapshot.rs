//! The snapshot, drawn: real models of each game through the real renderer,
//! headless. First its premise, that every game blends the background in
//! linearly, so two renders give each pixel's colour and cover.

use super::super::harness::gl::headless_gl;
use super::super::harness::shot::{SHOT_SIZE, Shot, ShotFrame, ShotOptions, shot_frame};
use super::super::renderer::ModelGlRenderer;
use crate::core::game::GameId;
use crate::core::source::TagSource;
use std::sync::Arc;

/// An object of each game whose kit is set, and its kind.
const MODELS: &[(&str, GameId, &str, &str)] = &[
    ("BLAM_TEST_HCEEK", GameId::HaloCe, "vehicles\\scorpion\\scorpion", "vehicle"),
    ("BLAM_TEST_HCEEK", GameId::HaloCe, "characters\\cyborg\\cyborg", "biped"),
    ("BLAM_TEST_H2EK", GameId::Halo2, "objects\\characters\\masterchief\\masterchief", "biped"),
    ("BLAM_TEST_H3EK", GameId::Halo3, "objects\\characters\\masterchief\\masterchief", "biped"),
    ("BLAM_TEST_HREK", GameId::HaloReach, "objects\\characters\\spartans\\spartans", "biped"),
    ("BLAM_TEST_HREK", GameId::HaloReach, "objects\\vehicles\\human\\warthog\\warthog", "vehicle"),
    ("BLAM_TEST_H4EK", GameId::Halo4, "objects\\characters\\storm_masterchief\\storm_masterchief", "biped"),
];

/// The kit source for `variable`'s game, when it is set.
fn source(variable: &str, game: GameId) -> Option<TagSource> {
    let Ok(root) = std::env::var(variable) else {
        eprintln!("skipping {game:?}: {variable} is not set");
        return None;
    };
    Some(TagSource::LooseFolder {
        root: root.into(),
        game: Some(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    })
}

fn shot(model: &str, kind: &str, shaders: bool) -> Shot {
    Shot { model: model.to_owned(), shaders, lights: true, yaw: 0.6, pitch: 0.25, kind: Some(kind.to_owned()) }
}

/// How far the render on mid grey is from halfway between the renders on
/// black and on white, at worst: 0 where each pixel is `C + T · background`.
fn worst_nonlinearity(black: &[f32], grey: &[f32], white: &[f32]) -> (f32, usize) {
    let mut worst = (0.0f32, 0);
    for (at, ((black, grey), white)) in black.iter().zip(grey).zip(white).enumerate() {
        // Alpha is the targets' own bookkeeping, not the picture.
        if at % 4 == 3 {
            continue;
        }
        let off = (grey - (black + white) / 2.0).abs();
        if off > worst.0 {
            worst = (off, at / 4);
        }
    }
    worst
}

/// A pass that read the background other than linearly would fail the
/// premise check: here, a frame squared after its background went in.
#[test]
fn the_premise_check_catches_a_frame_not_linear_in_its_background() {
    let frame = |background: f32| -> Vec<f32> {
        // One pixel: half a red surface over the background, then squared.
        let pixel = [0.5 + 0.5 * background, 0.5 * background, 0.5 * background, 1.0];
        pixel.iter().map(|channel| channel * channel).collect()
    };
    let (worst, _) = worst_nonlinearity(&frame(0.0), &frame(0.5), &frame(1.0));
    assert!(worst > 0.05, "squared after the background: {worst}");
    let linear = |background: f32| vec![0.5 + 0.5 * background, 0.5 * background, 0.5 * background, 1.0];
    assert!(worst_nonlinearity(&linear(0.0), &linear(0.5), &linear(1.0)).0 < 1e-6);
}

/// The matte's premise, on each game's models, with Game Shaders on (bloom
/// too) and off: each pixel of the frame is `C + T · background`, so the
/// render on mid grey is halfway between those on black and on white.
#[test]
fn every_games_frame_is_linear_in_its_background() {
    let size = SHOT_SIZE.map(|edge| edge as i32);
    let mut ran = 0;
    let mut failures = Vec::new();
    for (variable, game, model, kind) in MODELS {
        let Some(source) = source(variable, *game) else { continue };
        for shaders in [false, true] {
            let mut options = ShotOptions::new(*game);
            options.bloom = true;
            options.time = 1.5;
            let ShotFrame { frame, .. } = shot_frame(&source, &shot(model, kind, shaders), &options);
            let gl = Arc::new(headless_gl());
            let mut renderer = ModelGlRenderer::new(&gl).unwrap();
            // As the snapshot draws them: black first, building the bloom the
            // others add as it is.
            let [black, grey, white] = [(0.0, false), (0.5, true), (1.0, true)].map(|(background, held)| unsafe {
                super::super::renderer::draw_floats(&gl, &mut renderer, &frame, size, background, held)
            });
            let label = format!("{game:?} {model} shaders {shaders}");
            // Covered: some of the background hidden, white showing less than
            // black plus the whole background.
            let covered = black
                .chunks_exact(4)
                .zip(white.chunks_exact(4))
                .filter(|(b, w)| (0..3).any(|c| w[c] - b[c] < 0.99))
                .count();
            let (worst, at) = worst_nonlinearity(&black, &grey, &white);
            let (x, y) = (at % size[0] as usize, at / size[0] as usize);
            let px = |floats: &[f32]| floats[at * 4..at * 4 + 4].to_vec();
            eprintln!(
                "{label}: {covered} pixels cover; worst {worst} at {x},{y}: black {:?} grey {:?} white {:?}",
                px(&black),
                px(&grey),
                px(&white)
            );
            if covered < 1000 {
                failures.push(format!("{label}: the model drew only {covered} pixels"));
            }
            // Half floats: about three decimal places near 1.
            if worst >= 0.004 {
                failures.push(format!("{label}: {worst} off linear at {x},{y}"));
            }
            ran += 1;
        }
    }
    eprintln!("ran {ran}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}


/// Halo 3's Master Chief, whose bloom spreads past his outline: the snapshot
/// of him, with `bloom` on or off, and his renders on black and white.
fn halo_3_chief(bloom: bool) -> Option<(crate::app::model_preview::snapshot::SnapshotImage, Vec<f32>, Vec<f32>)> {
    let source = source("BLAM_TEST_H3EK", GameId::Halo3)?;
    let size = SHOT_SIZE.map(|edge| edge as i32);
    let mut options = ShotOptions::new(GameId::Halo3);
    options.bloom = bloom;
    options.time = 1.5;
    let ShotFrame { frame, .. } = shot_frame(&source, &shot("objects\\characters\\masterchief\\masterchief", "biped", true), &options);
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    let request = crate::app::model_preview::snapshot::SnapshotRequest::new(frame.clone(), size, Vec::new(), size.map(|edge| edge as f32));
    let image = unsafe { super::super::renderer::draw_snapshot(&gl, &|_| None, &mut renderer, &request) }.expect("a snapshot");
    let [black, white] = [(0.0, false), (1.0, true)].map(|(background, held)| unsafe {
        super::super::renderer::draw_floats(&gl, &mut renderer, &frame, size, background, held)
    });
    Some((image, black, white))
}

/// A pixel that shows the whole background: nothing covers it.
fn all_background(black: &[f32], white: &[f32]) -> bool {
    (0..3).all(|c| white[c] - black[c] > 0.999)
}

/// The snapshot's image is the matte of its two renders, as
/// [`matte_pixel`](crate::app::model_preview::snapshot::matte_pixel) makes
/// it; where nothing covers the background and no light is added, it is
/// clear, with no colour; and the bloom past the model's outline is kept,
/// as cover, where with bloom off there is none.
#[test]
fn the_snapshot_has_no_background_and_keeps_the_glow() {
    let Some((image, black, white)) = halo_3_chief(true) else { return };
    let [width, height] = SHOT_SIZE.map(|edge| edge as usize);
    // `BABOON_SNAPSHOT_SHOTS`: keep the image, to look at.
    if let Some(folder) = std::env::var_os("BABOON_SNAPSHOT_SHOTS") {
        let folder = std::path::PathBuf::from(folder);
        let _ = std::fs::create_dir_all(&folder);
        image::RgbaImage::from_raw(width as u32, height as u32, image.rgba.clone())
            .expect("a whole image")
            .save(folder.join("halo3_chief_bloom.png"))
            .expect("save the snapshot");
    }
    assert_eq!((image.width, image.height), (width, height));
    let mut worst = 0u8;
    let (mut clear, mut glowing) = (0, 0);
    for y in 0..height {
        for x in 0..width {
            // The renders are bottom row first; the image top row first.
            let render = ((height - 1 - y) * width + x) * 4;
            let (b, w) = (&black[render..render + 4], &white[render..render + 4]);
            let at = (y * width + x) * 4;
            let pixel = &image.rgba[at..at + 4];
            let expected = crate::app::model_preview::snapshot::matte_pixel([b[0], b[1], b[2]], [w[0], w[1], w[2]]);
            for (channel, value) in pixel.iter().zip(expected) {
                worst = worst.max(channel.abs_diff((value * 255.0).round() as u8));
            }
            if all_background(b, w) {
                if b[..3].iter().all(|channel| *channel <= 0.5 / 255.0) {
                    assert_eq!(pixel, [0, 0, 0, 0], "background at {x},{y} must be clear");
                    clear += 1;
                } else if pixel[3] > 0 {
                    glowing += 1;
                }
            }
        }
    }
    eprintln!("worst {worst}; {clear} clear, {glowing} glowing past the outline");
    // A half float's rounding, and the shader's against the CPU's.
    assert!(worst <= 2, "the image is off the matte by {worst}");
    assert!(clear > 100_000, "most of the frame is clear: {clear}");
    assert!(glowing > 10_000, "the bloom past his outline is kept: {glowing}");

    let Some((image, black, white)) = halo_3_chief(false) else { return };
    let glowing = (0..width * height)
        .filter(|&at| {
            let (x, y) = (at % width, at / width);
            let render = ((height - 1 - y) * width + x) * 4;
            all_background(&black[render..render + 4], &white[render..render + 4]) && image.rgba[at * 4 + 3] > 0
        })
        .count();
    assert_eq!(glowing, 0, "with bloom off, nothing glows past his outline");
}


/// The simulations and the frame after 1.5 s then `next`, with a snapshot of
/// the first frame in between when `snapshot`.
fn after(source: &TagSource, game: GameId, model: &str, kind: &str, next: f32, snapshot: bool) -> (String, Vec<f32>) {
    let size = SHOT_SIZE.map(|edge| edge as i32);
    let at = |time: f32| {
        let mut options = ShotOptions::new(game);
        options.time = time;
        options.bloom = true;
        shot_frame(source, &shot(model, kind, true), &options)
    };
    let (first, second) = (at(1.5), at(next));
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    unsafe { super::super::renderer::draw_floats(&gl, &mut renderer, &first.frame, size, 0.2, false) };
    if snapshot {
        let request =
            crate::app::model_preview::snapshot::SnapshotRequest::new(first.frame.clone(), size, Vec::new(), size.map(|edge| edge as f32));
        unsafe { super::super::renderer::draw_snapshot(&gl, &|_| None, &mut renderer, &request) }.expect("a snapshot");
    }
    let pixels = unsafe { super::super::renderer::draw_floats(&gl, &mut renderer, &second.frame, size, 0.2, false) };
    let mut simulations = String::new();
    renderer.for_each_simulation(second.data.geometry_id, |name, state| {
        super::super::gates::field_lines(&mut simulations, name, &state);
    });
    (simulations, pixels)
}

/// A snapshot draws the frame on screen again without moving anything on:
/// the frame after it, and every simulation, are as they would have been
/// without it. The comparison isn't blind: the next frame drawn a tenth of
/// a second later differs, on the models whose effects and shaders move.
#[test]
fn a_snapshot_leaves_the_simulations_where_the_frame_on_screen_left_them() {
    let mut ran = 0;
    let mut moving = 0;
    for (variable, game, model, kind) in MODELS {
        let Some(source) = source(variable, *game) else { continue };
        let plain = after(&source, *game, model, kind, 1.6, false);
        let snapped = after(&source, *game, model, kind, 1.6, true);
        let label = format!("{game:?} {model}");
        assert_eq!(snapped.0, plain.0, "{label}: the simulations moved");
        assert!(snapped.1 == plain.1, "{label}: the next frame changed");
        let later = after(&source, *game, model, kind, 1.7, false);
        if later.0 != plain.0 || later.1 != plain.1 {
            moving += 1;
        }
        ran += 1;
    }
    eprintln!("ran {ran}; {moving} move between 1.6 s and 1.7 s");
    if ran > 0 {
        assert!(moving > 0, "the comparison sees a frame that moved on");
    }
}

/// The snapshot's render on black is the viewport's own frame drawn on black:
/// the same passes, only in half floats.
#[test]
fn the_snapshots_render_is_the_viewports_frame() {
    let size = SHOT_SIZE.map(|edge| edge as i32);
    for (variable, game, model, kind) in MODELS {
        let Some(source) = source(variable, *game) else { continue };
        for shaders in [false, true] {
            let mut options = ShotOptions::new(*game);
            options.time = 1.5;
            let ShotFrame { mut frame, .. } = shot_frame(&source, &shot(model, kind, shaders), &options);
            frame.background = [0.0; 3];
            frame.show_grid = false;
            let gl = Arc::new(headless_gl());
            let mut renderer = ModelGlRenderer::new(&gl).unwrap();
            let viewport = unsafe { super::super::harness::shot::paint_frame(&gl, &mut renderer, &frame, size) };
            let gl = Arc::new(headless_gl());
            let mut renderer = ModelGlRenderer::new(&gl).unwrap();
            let floats = unsafe { super::super::renderer::draw_floats(&gl, &mut renderer, &frame, size, 0.0, false) };
            let worst = viewport
                .iter()
                .zip(&floats)
                .enumerate()
                .filter(|(at, _)| at % 4 != 3)
                .map(|(_, (byte, float))| byte.abs_diff((float.clamp(0.0, 1.0) * 255.0).round() as u8))
                .max()
                .unwrap_or(0);
            assert!(worst <= 2, "{game:?} {model} shaders {shaders}: off the viewport's frame by {worst}");
        }
    }
}

/// The viewport's egui helpers (markers, the armature, the bounding sphere)
/// are in the snapshot as they are on screen, over the frame: a shape painted
/// over an empty corner of the viewport is there in its own colour, whole.
#[test]
fn the_snapshot_has_the_helpers_shown_over_the_frame() {
    let Some(source) = source("BLAM_TEST_H3EK", GameId::Halo3) else { return };
    let size = SHOT_SIZE.map(|edge| edge as i32);
    let mut options = ShotOptions::new(GameId::Halo3);
    options.time = 1.5;
    let ShotFrame { frame, .. } = shot_frame(&source, &shot("objects\\characters\\masterchief\\masterchief", "biped", true), &options);
    let ctx = eframe::egui::Context::default();
    let _ = ctx.run_ui(Default::default(), |_| {});
    let corner = eframe::egui::Rect::from_min_size(eframe::egui::pos2(10.0, 10.0), eframe::egui::vec2(40.0, 30.0));
    let shape = eframe::egui::Shape::rect_filled(corner, 0.0, eframe::egui::Color32::from_rgb(255, 128, 0));
    let helpers = ctx.tessellate(
        vec![eframe::egui::epaint::ClippedShape::new(eframe::egui::Rect::EVERYTHING, shape)],
        1.0,
    );
    let gl = Arc::new(headless_gl());
    let mut renderer = ModelGlRenderer::new(&gl).unwrap();
    // egui's texture, white, as its font atlas is where shapes sample.
    let white = unsafe {
        use eframe::glow::HasContext as _;
        let texture = gl.create_texture().unwrap();
        gl.bind_texture(eframe::glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(eframe::glow::TEXTURE_2D, eframe::glow::TEXTURE_MIN_FILTER, eframe::glow::NEAREST as i32);
        gl.tex_parameter_i32(eframe::glow::TEXTURE_2D, eframe::glow::TEXTURE_MAG_FILTER, eframe::glow::NEAREST as i32);
        gl.tex_image_2d(
            eframe::glow::TEXTURE_2D,
            0,
            eframe::glow::RGBA8 as i32,
            1,
            1,
            0,
            eframe::glow::RGBA,
            eframe::glow::UNSIGNED_BYTE,
            eframe::glow::PixelUnpackData::Slice(Some(&[255, 255, 255, 255])),
        );
        texture
    };
    let request = crate::app::model_preview::snapshot::SnapshotRequest::new(frame, size, helpers, size.map(|edge| edge as f32));
    let image = unsafe { super::super::renderer::draw_snapshot(&gl, &|_| Some(white), &mut renderer, &request) }.expect("a snapshot");
    let at = |x: usize, y: usize| image.rgba[(y * image.width + x) * 4..(y * image.width + x) * 4 + 4].to_vec();
    assert_eq!(at(30, 25), [255, 128, 0, 255], "the helper, whole");
    assert_eq!(at(70, 25), [0, 0, 0, 0], "beside it, clear");
}
