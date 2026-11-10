use super::*;
use crate::app::model_preview::harness::kit::kit_source;
use crate::core::game::GameId;

/// With `BLAM_TEST_HREK`: `color_black`, the change-color map Emile's flair
/// names, is `a8` and samples black, so `two_change_color` leaves it
/// untinted; `gray_50_percent`, an `a8r8g8b8`, keeps its gray.
#[test]
fn kit_an_a8_bitmap_samples_black_with_its_alpha() {
    let Some(source) = kit_source(GameId::HaloReach) else {
        return;
    };
    let first_texel = |path: &str| {
        let (image, _, _) = read_gen3_bitmap(
            &crate::app::model_preview::load::TagReader::new(&source),
            path,
            0,
            SamplerKind::Texture2D,
        )
        .expect(path);
        let Image::Flat(flat) = &*image else {
            panic!("{path} should be flat");
        };
        [flat.rgba[0], flat.rgba[1], flat.rgba[2], flat.rgba[3]]
    };
    assert_eq!(
        first_texel("shaders\\default_bitmaps\\bitmaps\\color_black"),
        [0, 0, 0, 255]
    );
    assert_eq!(
        first_texel("shaders\\default_bitmaps\\bitmaps\\gray_50_percent")[..3],
        [128, 128, 128]
    );
}
