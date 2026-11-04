//! Halo 4's `material`s: baked as the game bakes them at load (blam-tags
//! `material::bake`), then drawn through their own material shader's
//! translated passes. Objects take the path every loaded level gives them,
//! `albedo` then `static_probe` (`select_mesh_entry_point_for_pass`, MCC
//! 0x1408CE100: lightmaps available, no per-object policy flags); with no
//! level at all the game picks `static_per_vertex_color`, which no material
//! shader compiles. What blends draws `single_pass_single_probe`.

use super::*;
use crate::app::model_preview::h4_shaders::{H4_DEFINES, H4ShaderSource, translate_pass};
use crate::core::hlsl::emit::Stage;
use crate::core::hlsl::parse::Program;
use blam_tags::material::{BakedMaterial, Material, MaterialShader, bake, shader_flags};

/// A material shader read once: its parameters and its parsed effect.
pub(super) struct H4Shader {
    shader: MaterialShader,
    program: Program,
    /// Translated passes by technique, kept for every material of the shader.
    passes: HashMap<&'static str, Option<Arc<H3Pass>>>,
}

/// `alpha_blend_mode_enum` as the preview's blend states. Kit objects use
/// opaque, alpha_blend, additive, opaque_alpha_blend, pre_multiplied_alpha,
/// add_src_times_srcalpha, multiply, inv_alpha_blend, alpha_blend_max and
/// maximum (`materials.md` §2.3).
pub(super) fn h4_blend(mode: u8) -> H3Blend {
    match mode {
        1 | 24 => H3Blend::Additive,
        2 => H3Blend::Multiply,
        // `alpha_blend_max` differs only in its alpha's MAX, which the preview
        // reads as coverage. One kit material uses it.
        3 | 14 | 22 => H3Blend::AlphaBlend,
        4 => H3Blend::DoubleMultiply,
        5 => H3Blend::PreMultiplied,
        6 => H3Blend::Maximum,
        7 => H3Blend::MultiplyAdd,
        8 => H3Blend::AddSrcTimesDstAlpha,
        9 => H3Blend::AddSrcTimesSrcAlpha,
        10 => H3Blend::InvAlphaBlend,
        11 => H3Blend::MotionBlurStatic,
        12 => H3Blend::MotionBlurInhibit,
        // `opaque_alpha_blend` counts as opaque (`GetIsOpaque`, X360
        // 0x82A9EA18); the rest are screen and shadow-mask modes no object uses.
        _ => H3Blend::Opaque,
    }
}

/// `bitmap_filter_mode` as stored, onto the shared enum's meanings.
fn h4_filter(filter: u8) -> BitmapFilterMode {
    match filter {
        1 => BitmapFilterMode::Point,
        2 => BitmapFilterMode::Bilinear,
        3 | 4 => BitmapFilterMode::Anisotropic2Expensive,
        5 => BitmapFilterMode::Anisotropic3Expensive,
        6 => BitmapFilterMode::Anisotropic4Expensive,
        _ => BitmapFilterMode::Trilinear,
    }
}

fn h4_address(address: u8) -> BitmapAddressMode {
    BitmapAddressMode::from_index(i128::from(address)).unwrap_or_default()
}

/// Whether a bitmap tag exists, and whether its first image is a cube.
fn bitmap_kind(source: &TagSource, path: &str, caches: &mut ResolveCaches) -> Option<bool> {
    if let Some(known) = caches.h4_bitmap_kinds.get(path) {
        return *known;
    }
    let found = load_referenced_tag_from_source(source, path, "bitmap", b"bitm").ok().map(|tag| {
        elements(tag.root(), "bitmaps*").first().and_then(|image| image.read_enum_name("type")).as_deref() == Some("cube map")
    });
    caches.h4_bitmap_kinds.insert(path.to_owned(), found);
    found
}

fn read_shader(source: &TagSource, path: &str) -> Result<H4Shader, String> {
    let tag = load_referenced_tag_from_source(source, path, "material_shader", b"mats").map_err(|error| error.to_string())?;
    let program = H4ShaderSource::read(source, path)?.program(&H4_DEFINES)?;
    Ok(H4Shader { shader: MaterialShader::from_tag(&tag), program, passes: HashMap::new() })
}

impl H4Shader {
    /// A technique drawn on the preview's skinned vertices.
    fn pass(&mut self, technique: &'static str) -> Option<Arc<H3Pass>> {
        let program = &self.program;
        self.passes
            .entry(technique)
            .or_insert_with(|| {
                if !program.techniques.iter().any(|candidate| &*candidate.name == technique) {
                    return None;
                }
                let vertex = translate_pass(program, technique, "skinned", Stage::Vertex).ok()?;
                let pixel = translate_pass(program, technique, "_default", Stage::Pixel).ok()?;
                Some(Arc::new(super::super::h3_shaders::pass_of(vertex, pixel)))
            })
            .clone()
    }
}

/// The pass's sampler for a parameter. Parameter names are string ids, which
/// hold lowercase; the source spells the sampler as it likes
/// (`control_map_SpGlRf`) and GLSL names are exact, so the pass's spelling
/// is the one to bind.
fn texture_uniform<'a>(wanted: &'a [(String, TextureKind)], parameter: &str) -> Option<(&'a str, &'a TextureKind)> {
    let lowercase = format!("{parameter}Texture");
    wanted.iter().find(|(name, _)| name.eq_ignore_ascii_case(&lowercase)).map(|(name, kind)| (name.as_str(), kind))
}

/// `ps_material_generic_parameters` for a hologram's pass 0
/// (`handle_object_part`): rows 0-2 the key, fill and rim light directions,
/// each the material's `lgt_*_forward`, `_right` and `_up` values (externs
/// 35, 37 and 39, which point at the first of three neighbouring lanes)
/// taken as a vector and normalized; row 3's x the alpha the shader clips
/// below, 1 for a blended hologram. The game first turns each vector by a
/// node's matrix (node 1's, `model_skinning_get_node_matrix`); the preview
/// leaves them in model space.
fn hologram_parameters(baked: &BakedMaterial, blended: bool) -> [[f32; 4]; 4] {
    let mut rows = [[0.0; 4]; 4];
    // The pixel constants as one run of lanes, c160.x first.
    let lanes = baked.real_vectors.as_flattened();
    for extern_ in &baked.externs {
        let [index @ (35 | 37 | 39), register, offset, _] = *extern_ else { continue };
        let Some(start) = usize::from(register).checked_sub(160).map(|slot| slot * 4 + usize::from(offset)) else {
            continue;
        };
        let Some(&[x, y, z]) = lanes.get(start..start + 3) else { continue };
        let length = (x * x + y * y + z * z).sqrt();
        if length > 0.0 {
            rows[usize::from((index - 35) / 2)] = [x / length, y / length, z / length, 0.0];
        }
    }
    rows[3][0] = if blended { 1.0 } else { 0.0 };
    rows
}

/// The bake's constants under the names the translated source reads.
fn baked_values(baked: &BakedMaterial) -> HashMap<String, H3Value> {
    let mut values = HashMap::new();
    for (index, vector) in baked.real_vectors.iter().enumerate() {
        values.insert(format!("user_parameter_{}", 160 + index), H3Value::Floats(*vector));
    }
    for (index, vector) in baked.real_vertex_vectors.iter().enumerate() {
        values.insert(format!("user_vertex_parameter_{}", 240 + index), H3Value::Floats(*vector));
    }
    let mut transforms = baked.texture_constants.clone();
    transforms.resize(12, [1.0, 1.0, 0.0, 0.0]);
    values.insert("ps_material_shader_transforms".to_owned(), H3Value::FloatArray(transforms));
    for bit in 0..32 {
        if baked.bools.used & (1 << bit) != 0 {
            values.insert(format!("user_parameter_bool_{bit}"), H3Value::Integer(i32::from(baked.bools.values & (1 << bit) != 0)));
        }
    }
    for bit in 0..8 {
        if baked.vertex_bools.used & (1 << bit) != 0 {
            let on = i32::from(baked.vertex_bools.values & (1 << bit) != 0);
            values.insert(format!("user_parameter_vertex_bool_{}", 40 + bit), H3Value::Integer(on));
        }
    }
    values
}

pub(super) fn resolve_h4_material(source: &TagSource, tag: &TagFile, caches: &mut ResolveCaches) -> MaterialTextures {
    let material = Material::from_tag(tag);
    if material.shader.is_empty() {
        return MaterialTextures::failed("the material names no material shader");
    }
    let shader = match caches.h4_shaders.remove(&material.shader) {
        Some(known) => known,
        None => read_shader(source, &material.shader).map_err(Arc::<str>::from),
    };
    let mut shader = match shader {
        Ok(shader) => shader,
        Err(error) => {
            caches.h4_shaders.insert(material.shader.clone(), Err(Arc::clone(&error)));
            return MaterialTextures::failed(format!("Halo 4 material shader not read: {error}"));
        }
    };
    let baked = {
        let lookup = |path: &str| bitmap_kind(source, path, caches);
        // The lookup needs the caches mutably; the bake only reads.
        let lookup = std::cell::RefCell::new(lookup);
        bake(&material, &shader.shader, &|path: &str| (lookup.borrow_mut())(path))
    };
    let blend = h4_blend(baked.alpha_blend_mode);
    // A decal shader's one technique (`techniques_decals.fxh`) writes the
    // albedo target, ahead of the lights unless the material puts it after
    // (`materialFlags` bit 1, `decal post lighting`).
    let decal = shader.shader.flags & blam_tags::material::shader_flags::IS_DECAL != 0;
    let hologram = shader.shader.flags & shader_flags::IS_HOLOGRAM != 0;
    let blended_hologram = shader.shader.flags & shader_flags::IS_BLENDED_HOLOGRAM != 0;
    // `c_render_hologram::render_object_callback` (X360 0x83B2CCD8): outside a
    // cinematic the transparent mode is 0, and `handle_object_part`
    // (0x83B2EB80) draws every hologram, blended ones too, in pass 0 alone:
    // opaque, depth written, back faces culled, through pass 16's entry,
    // `single_pass_single_probe`.
    let blend = if hologram || blended_hologram { H3Blend::Opaque } else { blend };
    let opaque = blend == H3Blend::Opaque;
    let (albedo, lit) = if hologram || blended_hologram {
        (None, shader.pass("single_pass_single_probe"))
    } else if decal {
        (None, shader.pass("_default"))
    } else if opaque && let (Some(albedo), Some(lit)) = (shader.pass("albedo"), shader.pass("static_probe")) {
        (Some(albedo), Some(lit))
    } else if opaque {
        // `opaque_alpha_blend` counts as transparent too (`GetIsTransparent`,
        // X360 0x82A9EAB0), and its blend state is off: a shader with no
        // deferred techniques (Cortana's hair compiles only `single_pass_*`)
        // shows through that pass, drawn as opaque.
        (None, shader.pass("single_pass_single_probe"))
    } else {
        (None, shader.pass("single_pass_single_probe").or_else(|| shader.pass("albedo")))
    };
    let mut textures = MaterialTextures::default();
    let result = match lit {
        Some(lit) => {
            let mut h3 = H3Material {
                dialect: H3Dialect::Halo4,
                albedo: albedo.filter(|_| opaque),
                lit: Arc::clone(&lit),
                screen_maps: Vec::new(),
                decal,
                decal_modulates_specular: false,
                after_lighting: decal && material.flags & 0x2 != 0,
                sort_layer: i32::from(baked.sort_layer),
                prt: [None, None, None],
                camo: None,
                shadow_generate: None,
                dynamic_light: None,
                blend,
                values: {
                    let mut values = baked_values(&baked);
                    if hologram || blended_hologram {
                        values.insert(
                            "ps_material_generic_parameters".to_owned(),
                            H3Value::FloatArray(hologram_parameters(&baked, blended_hologram).to_vec()),
                        );
                    }
                    values
                },
                textures: Vec::new(),
                change_colors: Vec::new(),
                animations: Vec::new(),
                shield: None,
            };
            let passes = [h3.albedo.as_ref(), Some(&lit)];
            let wanted: Vec<(String, TextureKind)> =
                passes.into_iter().flatten().flat_map(|pass| pass.textures.iter().cloned()).collect();
            for texture in &baked.textures {
                let Some((uniform, kind)) = texture_uniform(&wanted, &texture.parameter) else { continue };
                let uniform = uniform.to_owned();
                if h3.textures.iter().any(|known| known.uniform == uniform) {
                    continue;
                }
                let kind = match kind {
                    TextureKind::Cube => SamplerKind::Cube,
                    TextureKind::Volume => SamplerKind::Volume,
                    _ => SamplerKind::Texture2D,
                };
                let Some((image, texels, mips)) = h3_image(source, &texture.bitmap, 0, kind, caches) else { continue };
                h3.textures.push(H3Texture {
                    uniform,
                    image,
                    texels,
                    mips,
                    sampler: Some(H3Sampler {
                        address: [h4_address(texture.address_u), h4_address(texture.address_v)],
                        filter: h4_filter(texture.filter),
                    }),
                    frames: Vec::new(),
                });
            }
            Ok(h3)
        }
        None => Err("no object pass compiled".to_owned()),
    };
    // The plain shaded preview still shows the colour map.
    if let Some(color) = baked.textures.iter().find(|texture| texture.parameter == "color_map") {
        let binding = SlotBitmap {
            path: color.bitmap.clone(),
            image_index: 0,
            repeat_x: true,
            repeat_y: true,
            scale: baked.texture_constants.get(usize::from(color.sampler)).map_or([1.0, 1.0], |xform| [xform[0], xform[1]]),
        };
        textures.slots[TextureSlot::Base as usize] = cached_bitmap(source, &binding, caches);
    }
    match result {
        Ok(h3) => textures.h3 = Some(Arc::new(h3)),
        Err(error) => textures.error = Some(format!("Halo 4 material not drawn: {error}")),
    }
    caches.h4_shaders.insert(material.shader, Ok(shader));
    textures
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Didact's whole suit glowed and the Chief turned to chrome while
    /// `control_map_spglrf` missed `control_map_SpGlRfTexture` and the white
    /// placeholder stood in for it.
    #[test]
    fn a_parameter_finds_its_sampler_whatever_the_source_spells_it() {
        let wanted = vec![
            ("color_mapTexture".to_owned(), TextureKind::Flat),
            ("control_map_SpGlRfTexture".to_owned(), TextureKind::Flat),
            ("reflection_mapTexture".to_owned(), TextureKind::Cube),
        ];
        assert_eq!(texture_uniform(&wanted, "control_map_spglrf").map(|(name, _)| name), Some("control_map_SpGlRfTexture"));
        assert_eq!(texture_uniform(&wanted, "reflection_map").map(|(_, kind)| *kind), Some(TextureKind::Cube));
        assert_eq!(texture_uniform(&wanted, "control_map"), None, "a prefix is not a match");
    }

    /// The light vectors run across register boundaries, and the blended
    /// hologram (Cortana's hair) clips below full alpha.
    #[test]
    fn a_holograms_lights_come_from_its_lgt_vectors() {
        let mut baked = BakedMaterial::default();
        baked.real_vectors = vec![[0.0, 0.0, 3.0, 4.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 2.0]];
        // Key at c160.z (spills into c161.x), rim at c162.w: out of lanes.
        baked.externs = vec![[35, 160, 2, 0xFF], [36, 160, 1, 0xFF], [39, 162, 3, 0xFF]];
        let rows = hologram_parameters(&baked, false);
        assert_eq!(rows[0], [0.6, 0.8, 0.0, 0.0]);
        assert_eq!(rows[2], [0.0; 4], "a vector past the last lane is left alone");
        assert_eq!(rows[3][0], 0.0);
        assert_eq!(hologram_parameters(&baked, true)[3][0], 1.0);
    }

    #[test]
    fn blend_modes_take_the_preview_states_their_names_describe() {
        assert_eq!(h4_blend(0), H3Blend::Opaque);
        assert_eq!(h4_blend(23), H3Blend::Opaque, "opaque_alpha_blend counts as opaque");
        assert_eq!(h4_blend(3), H3Blend::AlphaBlend);
        assert_eq!(h4_blend(24), H3Blend::Additive, "alpha_blend_additive_transparent");
        assert_eq!(h4_blend(9), H3Blend::AddSrcTimesSrcAlpha);
        assert_eq!(h4_blend(10), H3Blend::InvAlphaBlend);
    }
}
