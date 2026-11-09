//! Resolving a render_model's materials to textures, against real editing kits.
//!
//! Every link in this chain is a lookup that can fail quietly — a shader group
//! that resolves to the wrong extension, a parameter name spelled differently
//! per game, an rmop default that never gets consulted. A unit test with a
//! synthetic tag would pass through all of it and prove nothing, because the
//! thing being tested *is* whether shipped tags match the assumptions. So these
//! run against installed kits and self-skip without them.

use super::*;
use crate::app::model_preview::harness::kit::{kit_root, kit_source};
use crate::core::game::GameId;

/// The model's hologram and damage passes go to its own materials, never
/// to a held weapon's.
#[test]
fn a_models_overlay_passes_stay_on_its_own_object() {
    let material = |object, overlay, shield| {
        RenderModelPreviewMaterial { object, overlay, shield, ..Default::default() }
    };
    let materials = [
        material(0, false, None),
        material(0, true, None),
        material(0, false, Some(H2Layer::ShieldDamaged)),
        material(1, false, None),
    ];
    let mut textures = vec![MaterialTextures::default(); materials.len()];
    let [(_, pass), ..] = h2_shadow_passes();
    textures[1].h2_layers[H2Layer::Transparent as usize] = vec![pass.clone()];
    textures[2].h2_layers[H2Layer::Transparent as usize] = vec![pass];
    share_object_layers(&materials, &mut textures);
    let has = |index: usize, layer: H2Layer| !textures[index].h2_layers[layer as usize].is_empty();
    for layer in [H2Layer::Hologram, H2Layer::ShieldDamaged] {
        assert!(has(0, layer) && has(1, layer) && has(2, layer), "{layer:?} on the model");
        assert!(!has(3, layer), "{layer:?} on the weapon");
    }
}

/// The kit a case resolves against, through the shared harness helper.
///
/// Halo 3 and Reach both, deliberately — they are the two games this targets,
/// and a slot table that silently only worked for one is exactly the failure
/// worth catching.
fn case_kit_root(case: &Case) -> Option<PathBuf> {
    crate::app::model_preview::harness::kit::kit_root(case.kit)
}

fn loose_source(tags_root: PathBuf, game: &str) -> TagSource {
    TagSource::LooseFolder {
        root: tags_root,
        game: GameId::from_id(game),
        definitions_root: crate::core::bundled::locate_definitions_root(),
    }
}

/// Load a render_model and turn it into the preview's material list, the same
/// way the preview loader does.
fn model_materials(source: &TagSource, rel: &str) -> Option<Vec<RenderModelPreviewMaterial>> {
    let tag = load_referenced_tag_from_source(source, rel, "render_model", b"mode").ok()?;
    let model = blam_tags::render_model::RenderModel::from_tag(&tag).ok()?;
    Some(
        model
            .materials
            .iter()
            .map(|material| RenderModelPreviewMaterial {
                shader_path: material.render_method.clone(),
                shader_group: material.render_method_group,
                overlay: false, shield: None,
                permutation: 0,
                object: 0,
            })
            .collect(),
    )
}

struct Case {
    game: &'static str,
    kit: GameId,
    model: &'static str,
}

const CASES: [Case; 3] = [
    Case {
        game: "halo3_mcc",
        kit: GameId::Halo3,
        model: r"objects\characters\masterchief\masterchief",
    },
    Case {
        game: "halo3_mcc",
        kit: GameId::Halo3,
        model: r"objects\characters\dervish\dervish",
    },
    Case {
        game: "haloreach_mcc",
        kit: GameId::HaloReach,
        model: r"objects\characters\brute\brute",
    },
];

/// The whole chain, on a shipped character: render_model → per-part material →
/// shader tag → resolved parameters → decoded bitmaps.
#[test]
fn a_shipped_character_resolves_to_real_textures_in_both_games() {
    let mut ran = 0;
    for case in CASES {
        let Some(root) = case_kit_root(&case) else {
            continue;
        };
        let source = loose_source(root, case.game);
        let Some(materials) = model_materials(&source, case.model) else {
            eprintln!("skipping: could not read {}", case.model);
            continue;
        };
        assert!(
            !materials.is_empty(),
            "{} has no materials at all",
            case.model
        );

        let resolved = resolve_model_textures(&source, &materials);
        assert_eq!(resolved.len(), materials.len(), "one result per material");

        // Every material must resolve to *something*. A character whose parts
        // all came back bare would mean the chain broke, not that the artist
        // shipped it untextured.
        let with_base = resolved
            .iter()
            .filter(|textures| textures.get(TextureSlot::Base).is_some())
            .count();
        assert!(
            with_base > 0,
            "{}: no material resolved a base_map. errors: {:?}",
            case.model,
            resolved
                .iter()
                .filter_map(|t| t.error.clone())
                .collect::<Vec<_>>()
        );

        // The decoded image has to be usable as a texture, not just non-empty.
        for textures in &resolved {
            for (slot, name) in SLOT_PARAMETERS {
                let Some(image) = textures.get(slot) else {
                    continue;
                };
                assert!(
                    image.width > 0 && image.height > 0,
                    "{}: {name} decoded to an empty image",
                    case.model
                );
                assert_eq!(
                    image.rgba.len(),
                    image.width * image.height * 4,
                    "{}: {name} is not tightly packed RGBA8",
                    case.model
                );
                assert!(
                    image.width as u32 <= MAX_TEXTURE_EDGE
                        && image.height as u32 <= MAX_TEXTURE_EDGE,
                    "{}: {name} came back at {}x{}, above the {MAX_TEXTURE_EDGE} cap",
                    case.model,
                    image.width,
                    image.height
                );
            }
        }

        let slots_found: Vec<&str> = SLOT_PARAMETERS
            .iter()
            .filter(|(slot, _)| resolved.iter().any(|t| t.get(*slot).is_some()))
            .map(|(_, name)| *name)
            .collect();
        eprintln!(
            "{}: {} materials, {with_base} with a base map, slots seen: {}",
            case.game,
            materials.len(),
            slots_found.join(", ")
        );
        ran += 1;
    }
    if ran == 0 {
        eprintln!("skipping: no editing kit was available for either game");
    }
}

/// Masterchief specifically, because his shader is the one whose slots were
/// read off disk while designing the table — diffuse, detail and normal all
/// present on one material is what the feature was asked for.
#[test]
fn masterchiefs_body_shader_carries_diffuse_detail_and_normal() {
    let Some(root) = case_kit_root(&CASES[0]) else {
        eprintln!("skipping: H3EK not present");
        return;
    };
    let source = loose_source(root, "halo3_mcc");
    let Some(materials) = model_materials(&source, CASES[0].model) else {
        eprintln!("skipping: could not read masterchief.render_model");
        return;
    };
    let resolved = resolve_model_textures(&source, &materials);

    // The body material is the one that has all three; the visor and light
    // shaders legitimately do not.
    let has_all_three = resolved.iter().any(|textures| {
        textures.get(TextureSlot::Base).is_some()
            && textures.get(TextureSlot::Detail).is_some()
            && textures.get(TextureSlot::Bump).is_some()
    });
    assert!(
        has_all_three,
        "no masterchief material carried base + detail + bump together; errors: {:?}",
        resolved
            .iter()
            .filter_map(|t| t.error.clone())
            .collect::<Vec<_>>()
    );
}

/// A material with no shader must come back explained rather than silently
/// bare, so the panel can say why a part is untextured.
#[test]
fn a_material_with_no_shader_reports_why() {
    let source = loose_source(PathBuf::from("C:/nonexistent/tags"), "halo3_mcc");
    let resolved = resolve_model_textures(
        &source,
        &[RenderModelPreviewMaterial {
            shader_path: String::new(),
            shader_group: 0,
            overlay: false, shield: None,
            permutation: 0,
            object: 0,
        }],
    );
    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].error.is_some());
    assert!(resolved[0].slots.iter().all(Option::is_none));
}

/// Address modes decide whether a detail map tiles. Halo authors them to repeat
/// many times over a surface, so a clamp here would smear one edge texel.
#[test]
fn wrap_modes_map_to_repeating_and_clamping() {
    use blam_tags::render_method::BitmapAddressMode;
    assert!(address_mode_repeats(BitmapAddressMode::Wrap));
    assert!(address_mode_repeats(BitmapAddressMode::Mirror));
    assert!(!address_mode_repeats(BitmapAddressMode::Clamp));
    assert!(!address_mode_repeats(BitmapAddressMode::BlackBorder));
}

/// Dervish carries a `bump_detail_map` on top of his `bump_map`, which is what
/// prompted adding that slot — and he is also the model the base-map alpha
/// discard used to punch holes through, so he is worth keeping as a fixture.
#[test]
fn dervish_resolves_a_detail_normal_on_top_of_his_bump_map() {
    let Some(root) = case_kit_root(&CASES[1]) else {
        eprintln!("skipping: H3EK not present");
        return;
    };
    let source = loose_source(root, "halo3_mcc");
    let Some(materials) = model_materials(&source, CASES[1].model) else {
        eprintln!("skipping: could not read dervish.render_model");
        return;
    };
    let resolved = resolve_model_textures(&source, &materials);

    assert!(
        resolved
            .iter()
            .any(|textures| textures.get(TextureSlot::Bump).is_some()
                && textures.get(TextureSlot::BumpDetail).is_some()),
        "no dervish material carried both a bump map and a detail normal; errors: {:?}",
        resolved
            .iter()
            .filter_map(|t| t.error.clone())
            .collect::<Vec<_>>()
    );
}

/// The detail maps tile by the shader's own scale animator, as tool's
/// `render_method_update_real_parameter` bakes them: dervish's armor
/// authors 16.
#[test]
fn detail_tiling_comes_from_the_shaders_animator() {
    let Some(root) = case_kit_root(&CASES[0]) else {
        eprintln!("skipping: H3EK not present");
        return;
    };
    let source = loose_source(root, "halo3_mcc");
    let resolved = resolve_model_textures(
        &source,
        &[RenderModelPreviewMaterial {
            shader_path: r"objects\characters\dervish\shaders\dervish_armor".to_owned(),
            shader_group: u32::from_be_bytes(*b"rmsh"),
            overlay: false, shield: None,
            permutation: 0,
            object: 0,
        }],
    );
    let material = &resolved[0];
    assert!(
        !material.used_shader_parameters_only,
        "the render-method definition should load"
    );

    for slot in [TextureSlot::Detail, TextureSlot::BumpDetail] {
        let image = material
            .get(slot)
            .expect("dervish carries both detail maps");
        assert!(
            image.scale.iter().all(|scale| (scale - 16.0).abs() < 0.01),
            "detail tiling should come from the shader's animator, got {:?}",
            image.scale
        );
    }
    // The base map is not a detail map and must not inherit its tiling.
    let base = material.get(TextureSlot::Base).expect("a base map");
    assert!(base.scale.iter().all(|scale| (scale - 1.0).abs() < 0.01), "base scale {:?}", base.scale);
}


/// One classic shader, resolved the way the preview resolves a material.
fn resolve_classic(source: &TagSource, path: &str, group: &[u8; 4]) -> MaterialTextures {
    resolve_model_textures(
        source,
        &[RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            shader_group: u32::from_be_bytes(*group),
            overlay: false, shield: None,
            permutation: 0,
            object: 0,
        }],
    )
    .remove(0)
}

/// Halo CE model shading, against `BLAM_TEST_HCEEK` (the kit's `tags`).
/// Each case pairs with one that sets the opposite flag, so a resolver that
/// ignored the flag fails one of them.
#[test]
fn halo_ce_shaders_resolve_as_the_engine_composes_them() {
    let Some(tags) = kit_root(GameId::HaloCe) else { return };
    let source = loose_source(tags, "haloce_mcc");

    // The cyborg's armor: every map, its detail masked by the reflection
    // channel in Xbox order, tiled ten times, alpha tested on the base.
    let armor = resolve_classic(&source, r"characters\cyborg\shaders\armor", b"soso");
    assert_eq!(armor.error, None);
    for slot in [TextureSlot::Base, TextureSlot::Detail, TextureSlot::Multipurpose] {
        assert!(armor.get(slot).is_some(), "armor lacks {slot:?}");
    }
    assert_eq!(
        armor.detail,
        DetailComposition {
            function: DetailFunction::BiasedMultiply,
            mask: 2,
            xbox_channel_order: true,
        }
    );
    // `detail map v scale` ships 0, which can only mean 1.
    assert_eq!(armor.get(TextureSlot::Detail).unwrap().scale, [10.0, 10.0]);
    assert!(armor.get(TextureSlot::AlphaTest).is_some(), "armor is alpha tested");
    assert!(armor.get(TextureSlot::Bump).is_none());

    // `not alpha tested`: the same resolver must leave the alpha slot empty.
    let metal = resolve_classic(&source, r"scenery\c_metalwide\shaders\c_metal", b"soso");
    assert!(metal.get(TextureSlot::Base).is_some());
    assert!(metal.get(TextureSlot::AlphaTest).is_none(), "c_metal is not alpha tested");

    // An alpha-tested environment shader tests on its bump map's alpha, and
    // never shades the bump: on a model, CE bump is lightmap-only.
    let teleporter = resolve_classic(
        &source,
        r"scenery\teleporter_base\shaders\teleporter_base",
        b"senv",
    );
    assert!(teleporter.get(TextureSlot::Base).is_some());
    assert!(teleporter.get(TextureSlot::AlphaTest).is_some());
    assert!(teleporter.get(TextureSlot::Bump).is_none());
    assert_eq!(teleporter.detail.mask, 0);

    // A chicago shader draws through its pass, not through the slots.
    let shield = resolve_classic(
        &source,
        r"characters\cyborg\shaders\light shield",
        b"schi",
    );
    assert!(shield.error.is_none());
    assert!(shield.slots.iter().all(Option::is_none));
    assert_eq!(shield.h2_layers[H2Layer::Transparent as usize].len(), 1);
    // `smet` is Halo 4's `structure_meta` in the cross-game extension table;
    // the kit's own names make it `shader_transparent_meter`, so the shader
    // loads rather than being reported as a missing file.
    let meter = resolve_classic(&source, r"vehicles\warthog\shaders\meter engine", b"smet");
    assert!(meter.error.is_none(), "{:?}", meter.error);
    assert_eq!(meter.h2_layers[H2Layer::Transparent as usize].len(), 1);
}

/// With `BLAM_TEST_HCEEK`: a permutation draws its own image of a bitmap,
/// in both drawing paths, and past the last one wraps.
#[test]
fn a_halo_ce_permutation_picks_the_image_and_wraps() {
    let Some(source) = kit_source(GameId::HaloCe) else { return };
    let path = r"characters\grunt\shaders\grunt torso";
    let resolve = |permutation| {
        let material = RenderModelPreviewMaterial {
            shader_path: path.to_owned(),
            shader_group: u32::from_be_bytes(*b"soso"),
            overlay: false, shield: None,
            permutation,
            object: 0,
        };
        resolve_model_textures(&source, &[material]).remove(0)
    };
    let shader = load_referenced_tag_from_source(&source, path, "shader_model", b"soso").unwrap();
    let base = shader.root().read_tag_ref_path("base map").unwrap();
    let tag = load_referenced_tag_from_source(&source, &base, "bitmap", b"bitm").unwrap();
    let count = blam_tags::bitmap::Bitmap::new(&tag).unwrap().len();
    assert!(count > 1, "{base} holds {count}");
    let slot = |textures: &MaterialTextures| {
        textures.get(TextureSlot::Base).map(|image| image.rgba.clone()).unwrap()
    };
    let pass = |textures: &MaterialTextures| match textures.h2_layers
        [H2Layer::Lightmap as usize][0]
        .textures[0]
        .image
        .as_deref()
    {
        Some(H2Image::Flat(image)) => image.rgba.clone(),
        _ => panic!("no base map"),
    };
    let (first, second, wrapped) = (resolve(0), resolve(1), resolve(count));
    assert_ne!(slot(&first), slot(&second));
    assert_ne!(pass(&first), pass(&second));
    assert_eq!(slot(&first), slot(&wrapped));
    assert_eq!(pass(&first), pass(&wrapped));
}

/// Halo 2, against `BLAM_TEST_H2EK` (the kit's `tags`).
#[test]
fn halo_2_shaders_resolve_parameters_over_template_defaults() {
    let Some(tags) = kit_root(GameId::Halo2) else { return };
    let source = loose_source(tags, "halo2_mcc");

    // The chief authors his maps, and his detail tiling as a `bitmap scale
    // x`/`y` pair of 9 with no uniform; reading only the uniform fell back to
    // the template's 16.
    let chief = resolve_classic(&source, r"objects\characters\masterchief\shaders\masterchief", b"shad");
    assert_eq!(chief.error, None);
    assert!(!chief.used_shader_parameters_only, "the template should load");
    for slot in [TextureSlot::Base, TextureSlot::Detail, TextureSlot::Bump] {
        assert!(chief.get(slot).is_some(), "masterchief lacks {slot:?}");
    }
    assert_eq!(chief.get(TextureSlot::Detail).unwrap().scale, [9.0, 9.0]);
    assert_eq!(chief.get(TextureSlot::Base).unwrap().scale, [1.0, 1.0]);
    assert_eq!(chief.detail.function, DetailFunction::BiasedMultiply);
    assert!(chief.get(TextureSlot::AlphaTest).is_none(), "tex_bump does not alpha test");

    // brute_shoulder_armor names no detail_map at all: its template supplies
    // both the bitmap (`default_detail`) and the tiling (16).
    let shoulder = resolve_classic(
        &source,
        r"objects\characters\brute\shaders\brute_shoulder_armor",
        b"shad",
    );
    assert_eq!(shoulder.get(TextureSlot::Detail).expect("template default detail").scale, [16.0, 16.0]);

    // A base-alpha alpha-test template with no alpha_test_map cuts out on the
    // base map.
    let shotgun = resolve_classic(&source, r"objects\weapons\rifle\shotgun\shaders\shotgun_primary", b"shad");
    assert!(shotgun.get(TextureSlot::AlphaTest).is_some(), "shotgun_primary alpha tests");
}

/// Passes draw a material with no slots only while Game Shaders are on.
#[test]
fn a_material_of_passes_alone_is_bare_only_without_game_shaders() {
    let materials = [RenderModelPreviewMaterial {
        shader_path: "shaders/emblem".to_owned(),
        ..Default::default()
    }];
    let textures = [MaterialTextures {
        bare_without_passes: Some("no maps".to_owned()),
        ..Default::default()
    }];
    assert_eq!(texture_resolve_note(&textures, &materials, true), None);
    let (summary, detail) = texture_resolve_note(&textures, &materials, false).unwrap();
    assert!(summary.starts_with("1 of 1 materials untextured"), "{summary}");
    assert!(detail.contains("shaders/emblem: no maps"), "{detail}");
}

/// A material that drew untextured says why, instead of just looking flat.
#[test]
fn untextured_materials_say_why() {
    let material = |path: &str| RenderModelPreviewMaterial {
        shader_path: path.to_owned(),
        ..Default::default()
    };
    let materials = [
        material("shaders/a"),
        material("shaders/b"),
        material("shaders/c"),
    ];
    let textures = [
        MaterialTextures {
            error: Some("shader tag not found".to_owned()),
            ..Default::default()
        },
        MaterialTextures {
            used_shader_parameters_only: true,
            ..Default::default()
        },
        MaterialTextures::default(),
    ];
    let (summary, detail) = texture_resolve_note(&textures, &materials, true).unwrap();
    assert_eq!(
        summary,
        "1 of 3 materials untextured; 1 read without their definition or template defaults \
             — hover for why"
    );
    assert!(detail.contains("shaders/a: shader tag not found"));
    assert!(detail.contains("shaders/b"));
    assert!(!detail.contains("shaders/c"));
    assert_eq!(texture_resolve_note(&textures[2..], &materials[2..], true), None);
}

/// With `BLAM_TEST_H2EK`: `+X`'s top row is `+Y`'s right column reversed,
/// so a cube in the right order joins there. In D3D order this pair is
/// `+X` and `-X`, which share no edge at all.
#[test]
fn h2_cube_faces_join_at_their_edges() {
    let Some(source) = kit_source(GameId::Halo2) else { return };
    let path = "objects\\bitmaps\\reflection_maps\\vehicle_glass";
    let Some(H2Image::Cube { size, faces }) = decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), path, SamplerKind::Cube, 0)
    else {
        panic!("{path} should decode as a cube");
    };
    let texel = |face: usize, x: usize, y: usize| -> [u8; 3] {
        let at = (y * size + x) * 4;
        [faces[face][at], faces[face][at + 1], faces[face][at + 2]]
    };
    let seam = |other: usize| -> f32 {
        let total: u32 = (0..size)
            .map(|x| {
                let (a, b) = (texel(0, x, 0), texel(other, size - 1, size - 1 - x));
                (0..3).map(|c| a[c].abs_diff(b[c]) as u32).sum::<u32>()
            })
            .sum();
        total as f32 / size as f32
    };
    assert!(seam(2) < 30.0, "+X and +Y should join: {}", seam(2));
    assert!(seam(1) > seam(2) * 2.0, "{} against {}", seam(1), seam(2));
    let mean = |face: usize| faces[face].iter().map(|byte| *byte as u64).sum::<u64>();
    assert!(mean(4) > mean(5), "the sky is on +Z");
}

/// With `BLAM_TEST_H2EK`: a brute names no camo shader, and still takes
/// the global one, with its own shader's bump.
#[test]
fn every_h2_shader_takes_the_global_active_camo() {
    let Some(source) = kit_source(GameId::Halo2) else { return };
    let material = |path: &str| RenderModelPreviewMaterial {
        shader_path: path.to_owned(),
        shader_group: u32::from_be_bytes(*b"shad"),
        overlay: false, shield: None,
        permutation: 0,
        object: 0,
    };
    let camo = |path: &str| {
        let textures = resolve_model_textures(&source, &[material(path)]);
        textures[0].h2_layers[H2Layer::ActiveCamo as usize].clone()
    };
    let chief = camo("objects\\characters\\masterchief\\shaders\\masterchief");
    let brute = camo("objects\\characters\\brute\\shaders\\brute");
    assert_eq!(chief.len(), 1);
    assert_eq!(brute.len(), 1);
    let bump = |passes: &[H2DrawPass]| match passes[0].textures[0].image.as_deref() {
        Some(H2Image::Flat(image)) => image.rgba.clone(),
        other => panic!("the bump should be a flat map: {other:?}"),
    };
    assert_ne!(bump(&chief), bump(&brute), "each shader brings its own bump");
}

/// With `BLAM_TEST_H2EK`: a volume's slices all decode.
#[test]
fn h2_volumes_decode_every_slice() {
    let Some(source) = kit_source(GameId::Halo2) else { return };
    let Some(H2Image::Volume { width, height, depth, rgba }) =
        decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), "rasterizer\\plasma_shield_noise", SamplerKind::Volume, 0)
    else {
        panic!("the plasma noise should decode as a volume");
    };
    assert!(depth > 1);
    assert_eq!(rgba.len(), width * height * depth * 4);
    let slice = width * height * 4;
    assert_ne!(rgba[..slice], rgba[slice * (depth - 1)..], "slices differ");
}

/// With `BLAM_TEST_HREK`: an array's layers each keep their whole chain,
/// so `globals\global_render_texture`'s second layer starts 21844 bytes
/// in, not a level's 16384 in as a volume's would.
#[test]
fn kit_an_arrays_layers_are_whole_chains_apart() {
    let Some(source) = kit_source(GameId::HaloReach) else { return };
    let path = "globals\\global_render_texture";
    let Some(H2Image::Volume { width, height, depth, rgba }) = decode_h2_image(&crate::app::model_preview::load::TagReader::new(&source), path, SamplerKind::Volume, 0)
    else {
        panic!("the render texture should decode as layers");
    };
    assert_eq!((width, height, depth), (64, 64, 14));
    let tag = load_referenced_tag_from_source(&source, path, "bitmap", b"bitm").unwrap();
    let bitmap = blam_tags::bitmap::Bitmap::new(&tag).unwrap();
    let image = bitmap.image(0).unwrap();
    let pixels = image.pixel_bytes().unwrap();
    assert_eq!(pixels.len(), 14 * 21844);
    let decode = |bytes: &[u8]| {
        let format = image.format().unwrap();
        blam_tags::bitmap::decode::decode_to_rgba8(format, 64, 64, bytes, bitmap.p8_palette()).unwrap()
    };
    let second = &rgba[64 * 64 * 4..2 * 64 * 64 * 4];
    assert_eq!(second, decode(&pixels[21844..21844 + 16384]).as_slice());
    assert_ne!(second, decode(&pixels[16384..2 * 16384]).as_slice(), "a volume's layout reads the first layer's mips");
    assert_eq!(array_layer_bytes(&image, 1, 1).map(<[u8]>::len), Some(32 * 32 * 4));
    assert_eq!(array_layer_bytes(&image, 1, 1).map(<[u8]>::as_ptr), Some(pixels[21844 + 16384..].as_ptr()));
}
