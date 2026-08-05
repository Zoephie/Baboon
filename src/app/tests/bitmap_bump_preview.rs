//! The bitmap preview shows a bump map as a normal map, not as noise.
//!
//! `tool.exe` bakes bump/zbump sources into tangent-space normals stored as
//! `dxn` (BC5) with signed channels on PC. Reading them unsigned rotates
//! every texel by half the range, and until the engine reconstructed Z the
//! blue channel was a flat 128 — so the panel drew colour noise on a dead
//! plane and *Extract Bitmap* wrote the same thing to TIFF.
//!
//! The decode itself is the engine's, and `blam-tags` covers it directly.
//! What this asserts is Baboon's half: that `build_bitmap_preview` resolves
//! the format through `BitmapImage::format` (which is what distinguishes the
//! two `dxn` encodings) rather than re-deriving it from the schema name, and
//! that the mip-chain walk hands the decoder the bytes it expects.
//!
//! Skips silently when no H3 editing kit is present.

use std::path::PathBuf;

use blam_tags::TagFile;

use crate::app::editor::bitmap::build_bitmap_preview;

/// A tag whose whole point is that it is a bump map, in a folder every
/// H3EK install ships.
const BUMP_TAG: &str = "levels/dlc/bunkerworld/bitmaps/nature/grassdirt_bump.bitmap";

/// An H3EK install, via `BLAM_TEST_H3EK` or the conventional Steam roots.
fn h3ek_tags() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BLAM_TEST_H3EK") {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    [
        "C:/Program Files (x86)/Steam/steamapps/common/H3EK/tags",
        "D:/SteamLibrary/steamapps/common/H3EK/tags",
        "E:/SteamLibrary/steamapps/common/H3EK/tags",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_dir())
}

#[test]
fn h3_bump_bitmap_previews_as_a_normal_map() {
    let Some(tags) = h3ek_tags() else {
        eprintln!("skipping: no H3 editing kit (set BLAM_TEST_H3EK to its `tags` directory)");
        return;
    };
    let path = tags.join(BUMP_TAG);
    if !path.is_file() {
        eprintln!("skipping: {BUMP_TAG} not in this kit");
        return;
    }

    let tag = TagFile::read(&path).expect("read bump bitmap tag");
    let preview = build_bitmap_preview(&tag, 0, 0).expect("build preview");

    // The panel's format label keeps showing what the tag says, even
    // though the decoder was handed the signed reading of it.
    assert_eq!(preview.format_name, "dxn");

    // Stored X/Y must fit inside the unit circle — the precondition for
    // the third component to mean anything. A wrongly-signed decode puts
    // most texels well outside it.
    let mut inside = 0usize;
    for px in preview.rgba.chunks_exact(4) {
        let x = px[0] as f32 / 127.5 - 1.0;
        let y = px[1] as f32 / 127.5 - 1.0;
        // Slack absorbs the byte round-trip on an exactly-flat texel.
        if x * x + y * y <= 1.02 {
            inside += 1;
        }
    }
    let texels = preview.rgba.len() / 4;
    let ratio = inside as f32 / texels as f32;
    assert!(
        ratio > 0.95,
        "only {:.1}% of texels form a valid normal — the preview is decoding \
         `dxn` with the wrong signedness",
        ratio * 100.0,
    );

    // Z is derived per texel, so a constant blue plane means it was never
    // derived — the symptom that made extracted normal maps unusable.
    let blues: Vec<u8> = preview.rgba.chunks_exact(4).map(|px| px[2]).collect();
    assert!(
        blues.iter().any(|&b| b != blues[0]),
        "blue channel is a constant {} — Z was not reconstructed",
        blues[0],
    );
}
