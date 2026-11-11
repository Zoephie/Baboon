//! Kit tests of what each preview reader takes from a bitmap's sequences and
//! sprites, so the reading can change underneath them without the preview
//! changing. Every expected value is the tag's own, as the corpus survey in
//! `docs/bitmap-sequence-viewer-research.md` read it field by field; each test
//! has a case where the answer turns on which sequence or sprite is read.

#![allow(clippy::excessive_precision)]

use blam_tags::TagFile;
use blam_tags::bitmap::BitmapDefinition;

use crate::app::export::load_referenced_tag_from_source;
use crate::app::model_preview::antennas::{ANTENNA_TRIM, antenna_sprite};
use crate::app::model_preview::ce::contrails::sequence_ranges;
use crate::app::model_preview::ce::sprite_sheet::{CeSpriteFrame, read_sheet_layout};
use crate::app::model_preview::ce::widgets::glow::glow_sprite;
use crate::app::model_preview::ce::widgets::light_volume::sequence_first_image;
use crate::app::model_preview::h2::particles::read_h2_frames;
use crate::app::model_preview::h3::leaves::leaf_plate_sprites;
use crate::app::model_preview::h4::effects::particles::sprite_placement;
use crate::app::model_preview::harness::kit::kit_source;
use crate::app::model_preview::particles::look::{plate_frames, sprite_corner};
use crate::core::game::GameId;

/// `path`'s bitmap in `game`'s kit, or none where the kit is not set up.
fn bitmap(game: GameId, path: &str) -> Option<TagFile> {
    let source = kit_source(game)?;
    Some(
        load_referenced_tag_from_source(&source, path, "bitmap", b"bitm")
            .unwrap_or_else(|error| panic!("{path}: {error}")),
    )
}

/// A sprite as the frame lists keep it: left, top, width and height.
fn frame([left, right, top, bottom]: [f32; 4]) -> [f32; 4] {
    [left, top, right - left, bottom - top]
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
}

const HUD_MSG_ICONS: &str = "ui\\hud\\bitmaps\\combined\\hud_msg_icons";

/// With `BLAM_TEST_HCEEK`: the sheet keeps every sequence's sprites with the
/// page each is cut from; sequence 7's lies on the second page.
#[test]
fn ce_sprite_sheet_layout_reads_every_sequence_and_page() {
    let Some(bitmap) = bitmap(GameId::HaloCe, HUD_MSG_ICONS) else {
        return;
    };
    let (sizes, sequences) = read_sheet_layout(&bitmap);
    assert_eq!(
        sizes,
        [[512.0, 512.0], [512.0, 512.0], [512.0, 512.0], [512.0, 256.0]]
    );
    assert_eq!(sequences.len(), 25);
    assert_eq!(
        sequences[0],
        [CeSpriteFrame {
            rect: [0.375, 0.556_640_982_627_868_7, 0.773_437_976_837_158_2, 0.955_078_005_790_710_4],
            registration: [0.089_843_802_154_064_18, 0.111_327_998_340_129_85],
            image: 0,
        }]
    );
    assert_eq!(
        sequences[7],
        [CeSpriteFrame {
            rect: [0.0, 0.738_281_25, 0.0, 0.324_218_75],
            registration: [0.369_140_625, 0.212_890_625],
            image: 1,
        }]
    );
}

/// With `BLAM_TEST_HCEEK`: a contrail's sequences are first image and count,
/// an empty sequence's `-1` read as the first image.
#[test]
fn ce_contrail_sequences_are_first_image_and_count() {
    let Some(grafix) = bitmap(GameId::HaloCe, "ui\\shell\\bitmaps\\mp_map_grafix") else {
        return;
    };
    assert_eq!(sequence_ranges(&grafix), [[0, 21]]);
    let grunt = bitmap(
        GameId::HaloCe,
        "characters\\grunt\\bitmaps\\reflection grunt diffuse",
    )
    .unwrap();
    assert_eq!(sequence_ranges(&grunt), [[0, 1], [0, 0]]);
}

/// With `BLAM_TEST_HCEEK`: a light volume's image is its sequence's first,
/// the first image where that sequence is empty or missing.
#[test]
fn ce_light_volume_takes_its_sequences_first_image() {
    let Some(cubemaps) = bitmap(GameId::HaloCe, "characters\\elite\\bitmaps\\cubemaps") else {
        return;
    };
    assert_eq!(sequence_first_image(&cubemaps, 0), 0);
    assert_eq!(sequence_first_image(&cubemaps, 3), 3);
    assert_eq!(sequence_first_image(&cubemaps, 99), 0);
    let dusk = bitmap(GameId::HaloCe, "digsite\\levels\\dusk\\_dig\\d_dusk_too").unwrap();
    assert_eq!(sequence_first_image(&dusk, 3), 0);
}

/// With `BLAM_TEST_HCEEK`: a glow draws its first sequence's first sprite.
#[test]
fn ce_glow_takes_the_first_sprite_and_its_pages_width() {
    let Some(fire) = bitmap(GameId::HaloCe, "weapons\\flamethrower\\bitmaps\\cloud fire") else {
        return;
    };
    assert_eq!(
        glow_sprite(&fire),
        Some((
            [0.703_125, 0.843_75, 0.843_75, 0.984_375],
            [0.070_312_5, 0.070_312_5],
            0,
            256.0
        ))
    );
    let icons = bitmap(GameId::HaloCe, HUD_MSG_ICONS).unwrap();
    assert_eq!(
        glow_sprite(&icons),
        Some((
            [0.375, 0.556_640_982_627_868_7, 0.773_437_976_837_158_2, 0.955_078_005_790_710_4],
            [0.089_843_802_154_064_18, 0.111_327_998_340_129_85],
            0,
            512.0
        ))
    );
}

/// With `BLAM_TEST_HCEEK`: an antenna takes its sequence's first sprite and
/// takes the stored spacing (4) off both sides and one pixel more.
#[test]
fn ce_antenna_takes_its_sequences_first_sprite_less_spacing() {
    let Some(icons) = bitmap(GameId::HaloCe, HUD_MSG_ICONS) else {
        return;
    };
    let second_page = antenna_sprite(&icons, 7, ANTENNA_TRIM).unwrap();
    assert_eq!(second_page.index, 1);
    assert_eq!(second_page.width, 512.0);
    assert_eq!(second_page.rect, [0.0, 0.738_281_25, 0.0, 0.324_218_75]);
    assert_eq!(second_page.registration, [0.369_140_625, 0.212_890_625]);
    assert_eq!(second_page.covered, (378.0 - 8.0 - 1.0) / 512.0);
    let narrow = antenna_sprite(&icons, 3, ANTENNA_TRIM).unwrap();
    assert_eq!(narrow.index, 0);
    assert_eq!(narrow.covered, (93.0 - 8.0 - 1.0) / 512.0);
    assert!(antenna_sprite(&icons, 25, ANTENNA_TRIM).is_none());
}

/// With `BLAM_TEST_H2EK`: a Halo 2 particle's frames come from sequence
/// `first`, wrapping past the last, and its quad from that sequence's first
/// sprite.
#[test]
fn h2_particle_frames_come_from_the_first_sequence_wrapped() {
    let Some(muzzle) = bitmap(
        GameId::Halo2,
        "effects\\bitmaps\\sparks\\flash_h_generic_muzzle",
    ) else {
        return;
    };
    let muzzle = BitmapDefinition::from_tag(&muzzle);
    let first_rect = [0.0, 0.390_625, 0.406_25, 0.804_687_5];
    let (frames, corner) = read_h2_frames(&muzzle, 0);
    assert_eq!(frames, [frame(first_rect)]);
    assert_eq!(
        corner,
        sprite_corner(first_rect, [0.195_312_5, 0.148_437_5], [0.0; 2], 1.0)
    );
    let fifth_rect = [0.0, 0.410_156_25, 0.0, 0.222_656_25];
    let (frames, corner) = read_h2_frames(&muzzle, 5);
    assert_eq!(frames, [frame(fifth_rect)]);
    assert_eq!(
        corner,
        sprite_corner(fifth_rect, [0.093_75, 0.156_25], [0.0; 2], 1.0)
    );
    assert_eq!(read_h2_frames(&muzzle, 17), (frames, corner));
}

/// With `BLAM_TEST_H3EK` and `BLAM_TEST_HREK`: a leaf plate's sprites are its
/// first sequence's, each with its page.
#[test]
fn h3_leaf_plate_sprites_are_the_first_sequences() {
    let Some(medals) = bitmap(GameId::Halo3, "ui\\chud\\bitmaps\\medals\\medals") else {
        return;
    };
    assert_eq!(
        leaf_plate_sprites(&medals),
        [(0, [0.875, 0.996_093_75, 0.375, 0.496_093_75])]
    );
    let Some(fire) = bitmap(GameId::HaloReach, "fx\\bitmaps\\fire\\fire_15fps") else {
        return;
    };
    let sprites = leaf_plate_sprites(&fire);
    assert_eq!(sprites.len(), 15);
    assert_eq!(
        sprites[..2],
        [
            (0, [0.359_375, 0.472_656_25, 0.492_187_5, 0.976_562_5]),
            (0, [0.587_890_625, 0.701_171_875, 0.492_187_5, 0.968_75]),
        ]
    );
}

/// With `BLAM_TEST_HREK`: a particle's quad is the sprite `start` of the way
/// through its first sequence, and its frames the sequence after, here the
/// same one wrapped.
#[test]
fn particle_frames_pick_the_starting_sprite_and_the_next_sequence() {
    let Some(fire) = bitmap(GameId::HaloReach, "fx\\bitmaps\\fire\\fire_15fps") else {
        return;
    };
    let rects = [
        [0.359_375, 0.472_656_25, 0.492_187_5, 0.976_562_5],
        [0.587_890_625, 0.701_171_875, 0.492_187_5, 0.968_75],
        [0.701_171_875, 0.816_406_25, 0.492_187_5, 0.964_843_75],
        [0.230_468_75, 0.345_703_125, 0.0, 0.492_187_5],
    ];
    let (frames, corner) = plate_frames(&fire, 0, [0.0; 2], 0.0, false);
    assert_eq!(frames.len(), 15);
    assert_eq!(frames[..4], rects.map(frame));
    assert_eq!(
        corner,
        sprite_corner(rects[0], [0.058_593_75, 0.238_281_25], [0.0; 2], 2.0)
    );
    let (_, later) = plate_frames(&fire, 0, [0.1, 0.2], 0.25, false);
    assert_eq!(
        later,
        sprite_corner(rects[3], [0.060_546_875, 0.246_093_75], [0.1, 0.2], 2.0)
    );
}

/// With `BLAM_TEST_H3EK`: a plate with only manual sequences keeps the whole
/// image for its quad and lists the manual sprites as its frames.
#[test]
fn particle_frames_list_manual_sequences_first() {
    let Some(smoke) = bitmap(
        GameId::Halo3,
        "fx\\particles\\atmospheric\\_bitmaps\\rolling_smoke_plate",
    ) else {
        return;
    };
    let (frames, corner) = plate_frames(&smoke, 0, [0.0; 2], 0.0, false);
    assert_eq!(frames.len(), 4);
    assert_eq!(
        frames[..2],
        [
            frame([5.476_497_650_146_484, 5.826_497_554_779_053, 5.554_074_287_414_551, 5.904_074_192_047_119]),
            frame([6.145_025_730_133_057, 5.795_025_825_500_488, 6.239_089_965_820_312_5, 5.889_090_061_187_744]),
        ]
    );
    assert_eq!(
        corner,
        sprite_corner([0.0, 1.0, 0.0, 1.0], [0.5, 0.5], [0.0; 2], 1.0)
    );
}

/// With `BLAM_TEST_H4EK`: Halo 4's placement is the starting sprite's quad,
/// moved by the centre offset, with the frames of the sequence after it.
#[test]
fn h4_sprite_placement_reads_the_starting_sprite_and_next_sequence() {
    let Some(tumbleweed) = bitmap(GameId::Halo4, "fx\\bitmaps\\3d\\plant\\tumbleweed_3d") else {
        return;
    };
    let tumbleweed = BitmapDefinition::from_tag(&tumbleweed);
    let root = Some(&tumbleweed);
    let placement = sprite_placement(root, 0, 0.1, false, true, [0.01, 0.02]);
    assert!(placement.animated);
    assert_eq!(placement.sprites.len(), 12);
    assert_eq!(
        placement.sprites[..2],
        [
            frame([0.476_562_5, 0.693_359_375, 0.0, 0.236_328_125]),
            frame([0.695_312_5, 0.931_640_625, 0.0, 0.236_328_125]),
        ]
    );
    // Sprite 1, square: `2 · registration / size - 1` from its middle.
    let size = 0.931_640_625_f32 - 0.695_312_5;
    let cx = 2.0 * (0.125 + 0.01) / size - 1.0;
    let cy = 2.0 * (0.132_812_5 - 0.02) / size - 1.0;
    assert!(
        close(placement.offset_and_scale, [-1.0 - cx, -1.0 - cy, 2.0, 2.0]),
        "{:?}",
        placement.offset_and_scale
    );
    let still = sprite_placement(root, 0, 0.1, false, false, [0.01, 0.02]);
    assert!(!still.animated);

    let skulls = bitmap(GameId::Halo4, "ui\\chud\\bitmaps\\skulls").unwrap();
    let placement = sprite_placement(Some(&BitmapDefinition::from_tag(&skulls)), 1, 0.0, false, true, [0.0; 2]);
    assert!(!placement.animated);
    assert_eq!(placement.sprites, [frame([0.312_5, 0.468_75, 0.0, 0.312_5])]);
    assert!(close(placement.offset_and_scale, [-1.0, -1.0, 2.0, 2.0]));

    let plasma = bitmap(GameId::Halo4, "fx\\bitmaps\\plasma\\plasma_clouds_256_a").unwrap();
    let placement = sprite_placement(Some(&BitmapDefinition::from_tag(&plasma)), 0, 0.0, false, true, [0.0; 2]);
    assert_eq!(placement.sprites.len(), 8);
    assert_eq!(
        placement.sprites[0],
        frame([0.252_979_278_564_453_1, 1.252_979_278_564_453_1, 0.357_060_641_050_338_75, 1.357_061_028_480_529_8])
    );
}
