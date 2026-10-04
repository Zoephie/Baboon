//! Resolving a render_model's materials to textures, against real editing kits.
//!
//! Every link in this chain is a lookup that can fail quietly — a shader group
//! that resolves to the wrong extension, a parameter name spelled differently
//! per game, an rmop default that never gets consulted. A unit test with a
//! synthetic tag would pass through all of it and prove nothing, because the
//! thing being tested *is* whether shipped tags match the assumptions. So these
//! run against installed kits and self-skip without them.

use super::*;

/// Kits to resolve against: env var first, then the usual Steam location.
///
/// Halo 3 and Reach both, deliberately — they are the two games this targets,
/// and a slot table that silently only worked for one is exactly the failure
/// worth catching.
fn kit_root(env: &str, default: &str) -> Option<PathBuf> {
    let root = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));
    root.is_dir().then_some(root)
}

fn loose_source(tags_root: PathBuf, game: &str) -> TagSource {
    TagSource::LooseFolder {
        root: tags_root,
        game: Some(game.to_owned()),
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
            })
            .collect(),
    )
}

struct Case {
    game: &'static str,
    env: &'static str,
    default_root: &'static str,
    model: &'static str,
}

const CASES: [Case; 3] = [
    Case {
        game: "halo3_mcc",
        env: "BABOON_H3EK_TAGS",
        default_root: r"D:\SteamLibrary\steamapps\common\H3EK\tags",
        model: r"objects\characters\masterchief\masterchief",
    },
    Case {
        game: "halo3_mcc",
        env: "BABOON_H3EK_TAGS",
        default_root: r"D:\SteamLibrary\steamapps\common\H3EK\tags",
        model: r"objects\characters\dervish\dervish",
    },
    Case {
        game: "haloreach_mcc",
        env: "BABOON_HREK_TAGS",
        default_root: r"D:\SteamLibrary\steamapps\common\HREK\tags",
        model: r"objects\characters\brute\brute",
    },
];

/// The whole chain, on a shipped character: render_model → per-part material →
/// shader tag → resolved parameters → decoded bitmaps.
#[test]
fn a_shipped_character_resolves_to_real_textures_in_both_games() {
    let mut ran = 0;
    for case in CASES {
        let Some(root) = kit_root(case.env, case.default_root) else {
            eprintln!(
                "skipping: {} not present (set {})",
                case.default_root, case.env
            );
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
    let Some(root) = kit_root(CASES[0].env, CASES[0].default_root) else {
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
    let Some(root) = kit_root(CASES[1].env, CASES[1].default_root) else {
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

/// The detail maps' tiling comes from the resolver's option defaults.
///
/// `detail_map_scale_uniform` is **16** and dervish's shader never mentions it,
/// so reading only the shader's own block found nothing and every detail map
/// tiled once across the whole model — which is most of what "detail" is for.
#[test]
fn detail_tiling_comes_from_the_option_defaults() {
    let Some(root) = kit_root(CASES[0].env, CASES[0].default_root) else {
        eprintln!("skipping: H3EK not present");
        return;
    };
    let source = loose_source(root, "halo3_mcc");
    let resolved = resolve_model_textures(
        &source,
        &[RenderModelPreviewMaterial {
            shader_path: r"objects\characters\dervish\shaders\dervish_armor".to_owned(),
            shader_group: u32::from_be_bytes(*b"rmsh"),
        }],
    );
    let material = &resolved[0];
    assert!(
        !material.used_shader_parameters_only,
        "the render-method definition should load; without it the option          default below is unreachable"
    );

    for slot in [TextureSlot::Detail, TextureSlot::BumpDetail] {
        let image = material
            .get(slot)
            .expect("dervish carries both detail maps");
        assert!(
            image.scale.iter().all(|scale| (scale - 16.0).abs() < 0.01),
            "detail tiling should come from the option default, got {:?}",
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
        }],
    )
    .remove(0)
}

/// Halo CE model shading, against `BLAM_TEST_HCEEK` (the kit's `tags`).
/// Each case pairs with one that sets the opposite flag, so a resolver that
/// ignored the flag fails one of them.
#[test]
fn halo_ce_shaders_resolve_as_the_engine_composes_them() {
    let tags = std::path::PathBuf::from(crate::test_kits::tag_path("haloce_mcc", ""));
    if !tags.join("characters/cyborg/shaders/armor.shader_model").is_file() {
        eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
        return;
    }
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

    // Transparent effects stay untextured, and say why.
    let shield = resolve_classic(
        &source,
        r"characters\cyborg\shaders\light shield",
        b"schi",
    );
    assert!(shield.error.as_deref().is_some_and(|error| error.contains("transparent")));
    assert!(shield.slots.iter().all(Option::is_none));
    // `smet` is Halo 4's `structure_meta` in the cross-game extension table;
    // the kit's own names make it `shader_transparent_meter`, so the shader
    // loads and is reported as what it is rather than as a missing file.
    let meter = resolve_classic(&source, r"vehicles\warthog\shaders\meter engine", b"smet");
    assert!(
        meter.error.as_deref().is_some_and(|error| error.contains("transparent")),
        "{:?}",
        meter.error
    );
}

/// Halo 2, against `BLAM_TEST_H2EK` (the kit's `tags`).
#[test]
fn halo_2_shaders_resolve_parameters_over_template_defaults() {
    let tags = crate::test_kits::h2ek_tags();
    if !tags.join("objects/characters/masterchief/shaders/masterchief.shader").is_file() {
        eprintln!("skipping: set BLAM_TEST_H2EK to a Halo 2 kit's tags folder");
        return;
    }
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
