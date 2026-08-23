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
        definitions_root: crate::app::locate_definitions_root(),
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
            (image.scale - 16.0).abs() < 0.01,
            "detail tiling should come from the option default, got {}",
            image.scale
        );
    }
    // The base map is not a detail map and must not inherit its tiling.
    let base = material.get(TextureSlot::Base).expect("a base map");
    assert!((base.scale - 1.0).abs() < 0.01, "base scale {}", base.scale);
}
