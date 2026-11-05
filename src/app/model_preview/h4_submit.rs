//! What Halo 4 sets per draw on top of a material's bake: its dynamic
//! functions re-run on the object's function values (blam-tags
//! `material::update_constants`), and a hologram's light rig.


use blam_tags::material::{
    BakedMaterial, FunctionContext, PIXEL_REGISTERS, TEXTURE_TRANSFORM_REGISTERS, VERTEX_REGISTERS, postprocess_flags,
    update_constants,
};

use std::sync::Arc;

use super::h3_shaders::{H3Pass, H3Texture, H3Value, H3Values};

/// `e_material_extern`s 35 to 41: a hologram's key, fill and rim light
/// directions (odd) and the scalars beside them (even).
const HOLOGRAM_EXTERNS: std::ops::RangeInclusive<u8> = 35..=41;

/// A transform the bake left no constant for: no scale, no offset.
const IDENTITY_TRANSFORM: [f32; 4] = [1.0, 1.0, 0.0, 0.0];

/// Which of a material's draws this is: it decides the phases its functions
/// run for, a hologram's clip, and the blend state the game sets, which
/// `ps_material_blend_constant` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum H4Draw {
    Albedo,
    /// The material's own pass, under its own blend mode.
    #[default]
    Lit,
    /// `opaque_alpha_blend`'s transparent half: `single_pass_single_probe`
    /// under `alpha_blend` (`material_submit_volatile_per_shader`, X360
    /// 0x83B08008).
    Fringe,
    /// `handle_object_part`'s pass 0, blending off.
    HologramOpaque,
    /// Its pass 2, `alpha_blend_max`.
    HologramBlend,
}

/// `alpha_blend` and `alpha_blend_additive_transparent`, the modes under
/// which the blend constant's w is −1.
const FLIPPED_CLIP_MODES: [u8; 2] = [3, 24];
/// `multiply`, `double_multiply` and `multiply_add`, under which its z is 1.
const MULTIPLY_MODES: [u8; 3] = [2, 4, 7];

/// `SetAlphaBlendShaderConstants` (X360 0x82BC890C) with no atmosphere:
/// `(0, 1, z, w)`. Alpha-clipped shaders (`srf_char_hair`) clip below
/// near-white under w = 1 and above it under w = −1, so one material's
/// opaque and blended halves meet.
pub(crate) fn blend_constant(state_mode: u8) -> [f32; 4] {
    let z = if MULTIPLY_MODES.contains(&state_mode) { 1.0 } else { 0.0 };
    let w = if FLIPPED_CLIP_MODES.contains(&state_mode) { -1.0 } else { 1.0 };
    [0.0, 1.0, z, w]
}

/// `ps_material_object_parameters` for an object's change colours
/// (`MaterialSubmitData`, X360 0x82A886D0, MCC 0x1408CA2F0): the first
/// `count` colours as packed to 8 bits (`real_rgb_color_to_pixel32` truncates,
/// `sub_14041AE10` divides by 255) with w = 1, then white with w = 0 (MCC
/// leaves that w unwritten; the X360 build writes 0). Hologram bipeds take a
/// node matrix here instead (`setup_object_shader_parameters`, X360
/// 0x839D6580), which none of their shaders read; that is not built.
pub(crate) fn object_parameters(colors: [[f32; 3]; 4], count: u8) -> Vec<[f32; 4]> {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0) as u8 as f32 / 255.0;
    (0..4)
        .map(|index| match colors.get(index).filter(|_| index < usize::from(count)) {
            Some(color) => [byte(color[0]), byte(color[1]), byte(color[2]), 1.0],
            None => [1.0, 1.0, 1.0, 0.0],
        })
        .collect()
}

/// A Halo 4 material's per-draw state.
#[derive(Debug)]
pub(crate) struct H4Submit {
    baked: BakedMaterial,
    /// `GetEntrypointRenderPhaseMask` of the albedo pass's entry and the lit
    /// pass's.
    albedo_phases: u8,
    lit_phases: u8,
    /// `alpha_blend_mode` as baked, the state its own pass draws under.
    alpha_blend_mode: u8,
    /// For a hologram, the alpha its pass 0 clips below: 1 for a blended one.
    hologram: Option<f32>,
    /// For a hologram, the screen materials its picture goes through.
    post: Option<Arc<H4HologramPost>>,
    /// Samplers whose image a frame-index function picks, by uniform: the
    /// function's queue index and the bitmap's image count.
    frames: Vec<(String, usize, usize)>,
}

/// One of `rasterizer_globals`' explicit shaders that is a `material`: its
/// screen pass, its per-draw state, and its bitmaps.
#[derive(Debug)]
pub(crate) struct H4ScreenMaterial {
    pub(crate) pass: Arc<H3Pass>,
    pub(crate) submit: H4Submit,
    pub(crate) values: H3Values,
    pub(crate) textures: Vec<H3Texture>,
}

/// What a hologram's picture goes through (`RenderHolograms`, X360
/// 0x829B63C8; `docs/h4-preview-research/holograms.md`): explicit shader 22,
/// `globals\materials\final_combine_hologram`, onto its own surface, then 24,
/// `globals\materials\hologram_composite`, into the scene's light.
#[derive(Debug)]
pub(crate) struct H4HologramPost {
    pub(crate) combine: H4ScreenMaterial,
    pub(crate) composite: H4ScreenMaterial,
}

/// The drawn object, as a material's functions read it.
pub(crate) struct DrawnObject<'a> {
    pub(crate) values: Option<&'a super::h2_functions::H2FunctionValues>,
    pub(crate) time: f32,
    pub(crate) owner: u8,
}

impl FunctionContext for DrawnObject<'_> {
    /// What the panel and the object's functions give the name; a name they
    /// do not know reads as an idle object's.
    fn object_function(&self, name: &str) -> f32 {
        let known = self.values.and_then(|values| values.get(name)).copied();
        known.unwrap_or_else(|| super::h2_passes::object_input(name))
    }

    fn time(&self) -> f32 {
        self.time
    }

    fn object_index(&self) -> u32 {
        u32::from(self.owner)
    }
}

impl H4Submit {
    /// `albedo_entry` and `lit_entry` are the entry points the passes were
    /// taken from; `hologram` the clip alpha of a hologram's pass 0.
    pub(crate) fn new(baked: BakedMaterial, albedo_entry: usize, lit_entry: usize, hologram: Option<f32>) -> Self {
        Self {
            alpha_blend_mode: baked.alpha_blend_mode,
            baked,
            albedo_phases: blam_tags::material::entry_point_phase_mask(albedo_entry),
            lit_phases: blam_tags::material::entry_point_phase_mask(lit_entry),
            hologram,
            post: None,
            frames: Vec::new(),
        }
    }

    /// The sampler `uniform` shows the image a queued function picks, of
    /// `image_count`.
    pub(crate) fn add_frames(&mut self, uniform: String, function: usize, image_count: usize) {
        self.frames.push((uniform, function, image_count));
    }

    /// Which image `uniform` shows this draw (`MaterialSubmitGetBitmapIndex`),
    /// held to the bitmap's images; none for a sampler no function animates.
    pub(crate) fn frame(&self, uniform: &str, object: &DrawnObject<'_>) -> Option<usize> {
        let (_, function, count) = self.frames.iter().find(|(known, ..)| known == uniform)?;
        let function = &self.baked.functions.get(*function)?.function;
        let index = blam_tags::material::bitmap_index(function, object, *count);
        Some(usize::try_from(index).unwrap_or(0).min(count.saturating_sub(1)))
    }

    /// A hologram's, with the screen materials its picture goes through.
    pub(crate) fn with_post(mut self, post: Option<Arc<H4HologramPost>>) -> Self {
        self.post = post;
        self
    }

    /// Whether this draws through `RenderHolograms` rather than with the scene.
    pub(crate) fn is_hologram(&self) -> bool {
        self.hologram.is_some()
    }

    /// A blended hologram (`is_blended_hologram`): drawn again in the blend
    /// pass, and clipped below full alpha in the first.
    pub(crate) fn is_blended_hologram(&self) -> bool {
        self.hologram == Some(1.0)
    }

    pub(crate) fn post(&self) -> Option<&Arc<H4HologramPost>> {
        self.post.as_ref()
    }

    /// Whether a function runs on the clock: one whose input has no name
    /// (or is `unique_id`, which also runs on it when periodic), or a ranged
    /// one whose range has none, over a curve that is not constant.
    pub(crate) fn moves(&self) -> bool {
        let clocked = |name: &str| name.is_empty() || name == "unique_id";
        self.baked.functions.iter().map(|function| &function.function).any(|function| {
            let Some(curve) = function.function.as_ref() else { return false };
            curve.as_constant().is_none()
                && (clocked(&function.input_name) || (curve.is_ranged() && clocked(&function.range_name)))
        })
    }

    /// Every name the functions read, for the object to answer.
    pub(crate) fn inputs(&self) -> impl Iterator<Item = &str> {
        let functions = self.baked.functions.iter().map(|function| &function.function);
        let names = functions.flat_map(|function| [&function.input_name, &function.range_name, &function.output_modifier_input]);
        names.map(String::as_str).filter(|name| !name.is_empty() && *name != "unique_id")
    }

    /// The bake's constants under the names the translated source reads.
    pub(crate) fn baked_values(&self) -> H3Values {
        let baked = &self.baked;
        let mut values = H3Values::new();
        insert_registers(&mut values, "user_parameter", PIXEL_REGISTERS.0, &baked.real_vectors);
        insert_registers(&mut values, "user_vertex_parameter", VERTEX_REGISTERS.0, &baked.real_vertex_vectors);
        values.insert("ps_material_shader_transforms".to_owned(), H3Value::FloatArray(transforms(baked)));
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

    /// What this draw sets over the bake: `SetMaterialVertexConstants`,
    /// `SetMaterialPixelConstants` and `SetMaterialTextureTransformConstants`
    /// (MCC 0x1408CA980), each only when the bake queued functions of its
    /// kind, and a hologram's rig. `node` is the hologram's node matrix (see
    /// [`hologram_rows`]).
    pub(crate) fn draw_values(&self, draw: H4Draw, object: &DrawnObject<'_>, node: [[f32; 4]; 3]) -> H3Values {
        /// `single_pass_single_probe`, the fringe's entry.
        const ENTRY_SINGLE_PASS_SINGLE_PROBE: usize = 23;
        let baked = &self.baked;
        let (phases, state) = match draw {
            H4Draw::Albedo => (self.albedo_phases, self.alpha_blend_mode),
            H4Draw::Lit => (self.lit_phases, self.alpha_blend_mode),
            H4Draw::Fringe => (blam_tags::material::entry_point_phase_mask(ENTRY_SINGLE_PASS_SINGLE_PROBE), 3),
            H4Draw::HologramOpaque => (self.lit_phases, 0),
            H4Draw::HologramBlend => (self.lit_phases, 22),
        };
        let mut values = H3Values::new();
        values.insert("ps_material_blend_constant".to_owned(), H3Value::Floats(blend_constant(state)));
        let queued = |flag: u16| baked.flags & flag != 0;
        if queued(postprocess_flags::VERTEX_CONSTANT_FUNCTIONS) {
            let mut vertex = registers(&baked.real_vertex_vectors, VERTEX_REGISTERS.1);
            update_constants(baked, phases, VERTEX_REGISTERS, &mut vertex, object);
            insert_registers(&mut values, "user_vertex_parameter", VERTEX_REGISTERS.0, &vertex);
        }
        if queued(postprocess_flags::PIXEL_CONSTANT_FUNCTIONS) {
            let mut pixel = registers(&baked.real_vectors, PIXEL_REGISTERS.1);
            update_constants(baked, phases, PIXEL_REGISTERS, &mut pixel, object);
            insert_registers(&mut values, "user_parameter", PIXEL_REGISTERS.0, &pixel);
        }
        if queued(postprocess_flags::TEXTURE_TRANSFORM_FUNCTIONS) {
            let mut xforms = transforms(baked);
            update_constants(baked, phases, TEXTURE_TRANSFORM_REGISTERS, &mut xforms, object);
            values.insert("ps_material_shader_transforms".to_owned(), H3Value::FloatArray(xforms));
        }
        if let Some(clip) = self.hologram {
            // `handle_object_part` sets the threshold in pass 0 alone.
            let clip = if draw == H4Draw::HologramBlend { 0.0 } else { clip };
            // `handle_object_part` runs every phase's functions for its rig.
            let mut pixel = registers(&baked.real_vectors, PIXEL_REGISTERS.1);
            if queued(postprocess_flags::PIXEL_CONSTANT_FUNCTIONS) {
                update_constants(baked, 0, PIXEL_REGISTERS, &mut pixel, object);
            }
            let rows = hologram_rows(&baked.externs, &pixel, node, clip);
            values.insert("ps_material_generic_parameters".to_owned(), H3Value::FloatArray(rows.to_vec()));
        }
        values
    }
}

/// The bake's registers, padded with zeros to the range's `count`.
fn registers(baked: &[[f32; 4]], count: u8) -> Vec<[f32; 4]> {
    let mut registers = baked.to_vec();
    registers.resize(usize::from(count).max(baked.len()), [0.0; 4]);
    registers
}

/// The texture transforms by sampler, those the bake left none for as
/// identity.
fn transforms(baked: &BakedMaterial) -> Vec<[f32; 4]> {
    let mut transforms = baked.texture_constants.clone();
    transforms.resize(usize::from(TEXTURE_TRANSFORM_REGISTERS.1), IDENTITY_TRANSFORM);
    transforms
}

fn insert_registers(values: &mut H3Values, prefix: &str, start: u8, registers: &[[f32; 4]]) {
    for (index, vector) in registers.iter().enumerate() {
        values.insert(format!("{prefix}_{}", usize::from(start) + index), H3Value::Floats(*vector));
    }
}

/// `ps_material_generic_parameters` for a hologram's pass 0
/// (`s_render_hologram_parts::handle_object_part`, X360 0x83B2EB80): rows 0
/// to 2 the key, fill and rim lights. An odd extern names a direction, the
/// three lanes from its register's lane, turned by `node` (rows of the
/// skinning matrix of the object's node 1, `min(1, count - 1)`) and
/// normalized; an even one a scalar for the row's w. Row 3's x is the alpha
/// the pass clips below (constant 203, set apart from the rig).
pub(crate) fn hologram_rows(externs: &[[u8; 4]], pixel: &[[f32; 4]], node: [[f32; 4]; 3], clip: f32) -> [[f32; 4]; 4] {
    let mut rows = [[0.0; 4]; 4];
    let lanes = pixel.as_flattened();
    for &[index, register, offset, _] in externs {
        if !HOLOGRAM_EXTERNS.contains(&index) {
            continue;
        }
        let row = usize::from((index - HOLOGRAM_EXTERNS.start()) / 2);
        // Row 3 is past the three the game uploads.
        let Some(row) = rows.get_mut(row).filter(|_| row < 3) else { continue };
        let Some(start) = usize::from(register).checked_sub(usize::from(PIXEL_REGISTERS.0)) else { continue };
        let start = start * 4 + usize::from(offset);
        if (index - HOLOGRAM_EXTERNS.start()) % 2 == 0 {
            let Some(&[x, y, z]) = lanes.get(start..start + 3) else { continue };
            let turned: [f32; 3] = std::array::from_fn(|axis| node[axis][0] * x + node[axis][1] * y + node[axis][2] * z);
            let length = turned.iter().map(|lane| lane * lane).sum::<f32>().sqrt();
            if length > 0.0 {
                row[..3].copy_from_slice(&turned.map(|lane| lane / length));
            }
        } else if let Some(&scalar) = lanes.get(start) {
            row[3] = scalar;
        }
    }
    rows[3][0] = clip;
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

    /// The light vectors run across register boundaries, the even externs
    /// fill w, and the blended hologram (Cortana's hair) clips below full
    /// alpha.
    #[test]
    fn a_holograms_lights_come_from_its_lgt_vectors() {
        let pixel = [[0.0, 0.0, 3.0, 4.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.0, 2.0]];
        // Key at c160.z (spills into c161.x), its scalar at c161.y; rim at
        // c162.w: out of lanes.
        let externs = [[35, 160, 2, 0xFF], [36, 161, 1, 0xFF], [39, 162, 3, 0xFF]];
        let rows = hologram_rows(&externs, &pixel, IDENTITY, 0.0);
        assert_eq!(rows[0], [0.6, 0.8, 0.0, 0.5]);
        assert_eq!(rows[2], [0.0; 4], "a vector past the last lane is left alone");
        assert_eq!(rows[3][0], 0.0);
        assert_eq!(hologram_rows(&externs, &pixel, IDENTITY, 1.0)[3][0], 1.0);
    }

    /// The node turns the rig: a quarter turn about up takes forward to left.
    #[test]
    fn the_node_matrix_turns_the_rig() {
        let pixel = [[1.0, 0.0, 0.0, 0.0]];
        let quarter = [[0.0, -1.0, 0.0, 5.0], [1.0, 0.0, 0.0, 5.0], [0.0, 0.0, 1.0, 5.0]];
        let rows = hologram_rows(&[[35, 160, 0, 0xFF]], &pixel, quarter, 0.0);
        assert_eq!(rows[0], [0.0, 1.0, 0.0, 0.0], "a direction ignores the node's position");
    }

    /// With `BLAM_TEST_H4EK`: Cortana's body runs its lights' functions on
    /// her biped's functions, so the key, fill and rim point three ways
    /// where the bake, at input 0, left them all one way.
    #[test]
    fn halo_4_cortanas_lights_follow_her_functions() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let root = std::path::Path::new(&root);
        let read = |path: &str| blam_tags::TagFile::read(root.join(path)).unwrap();
        let biped = read("objects/characters/storm_cortana/storm_cortana.biped");
        let functions = super::super::h2_functions::read_h2_object_functions(&biped, None).unwrap();
        let material = blam_tags::material::Material::from_tag(&read(
            "objects/characters/storm_cortana/materials/storm_cortana_default/storm_cortana_default_body.material",
        ));
        let shader = blam_tags::material::MaterialShader::from_tag(&read(&format!(
            "{}.material_shader",
            material.shader.replace('\\', "/")
        )));
        let baked = blam_tags::material::bake(&material, &shader, &|_: &str| Some(false));
        let submit = H4Submit::new(baked, 1, 23, Some(0.0));
        let extra: Vec<String> = submit.inputs().map(str::to_owned).collect();
        let values = functions.values(&Default::default(), 0, &extra, 0.0, 0);
        for name in &extra {
            eprintln!("NOTE input {name} = {:?}", values.get(name));
        }
        let object = DrawnObject { values: Some(&values), time: 0.0, owner: 0 };
        let drawn = submit.draw_values(H4Draw::Lit, &object, IDENTITY);
        for function in &submit.baked.functions {
            let register = function.register_index;
            let name = if function.vertex { format!("user_vertex_parameter_{register}") } else { format!("user_parameter_{register}") };
            eprintln!(
                "NOTE fn {} {:?} in {:?} range {:?} -> {name}.{} = {:?}",
                function.parameter, function.function.kind, function.function.input_name, function.function.range_name,
                function.register_offset, drawn.get(&name)
            );
        }
        let Some(H3Value::FloatArray(rows)) = drawn.get("ps_material_generic_parameters") else { panic!("no rig") };
        eprintln!("NOTE rig {rows:?}");
        let same = |a: [f32; 4], b: [f32; 4]| (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-3);
        assert!(!same(rows[0], rows[1]) && !same(rows[0], rows[2]), "{rows:?}");
    }

    /// With `BLAM_TEST_H4EK`: the hologram composite's maps scroll on the
    /// clock, so the preview keeps drawing; the Chief's helmet has nothing
    /// that does.
    #[test]
    fn halo_4_functions_with_no_input_keep_the_preview_drawing() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let root = std::path::Path::new(&root);
        let submit = |path: &str| {
            let read = |path: &str| blam_tags::TagFile::read(root.join(path)).unwrap();
            let material = blam_tags::material::Material::from_tag(&read(path));
            let shader = read(&format!("{}.material_shader", material.shader.replace('\\', "/")));
            let shader = blam_tags::material::MaterialShader::from_tag(&shader);
            H4Submit::new(blam_tags::material::bake(&material, &shader, &|_: &str| Some(false)), 1, 8, None)
        };
        assert!(submit("globals/materials/final_combine_hologram.material").moves());
        let helmet = "objects/characters/storm_masterchief/materials/storm_masterchief_default/storm_masterchief_default_armor_helm.material";
        assert!(!submit(helmet).moves());
    }

    /// The hair's halves: its own `opaque_alpha_blend` state keeps w = 1
    /// (clip below near-white), the fringe's `alpha_blend` flips it, and the
    /// multiplies set z.
    #[test]
    fn the_blend_constant_follows_the_state_drawn() {
        assert_eq!(blend_constant(23), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(blend_constant(3), [0.0, 1.0, 0.0, -1.0]);
        assert_eq!(blend_constant(24), [0.0, 1.0, 0.0, -1.0]);
        assert_eq!(blend_constant(2), [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(blend_constant(0), [0.0, 1.0, 0.0, 1.0]);
    }

    /// The colours the object has come through packed to bytes; the rest
    /// read white with no weight.
    #[test]
    fn change_colors_pack_to_bytes_and_the_rest_are_white() {
        let colors = [[1.0, 0.5, 0.0], [0.2, 0.4, 0.6], [0.3; 3], [0.9; 3]];
        let packed = object_parameters(colors, 2);
        assert_eq!(packed[0], [1.0, 127.0 / 255.0, 0.0, 1.0], "0.5 truncates to 127");
        assert_eq!(packed[1][3], 1.0);
        assert_eq!(packed[2], [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(packed[3], [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(object_parameters(colors, 0)[0], [1.0, 1.0, 1.0, 0.0]);
    }

    /// With `BLAM_TEST_H4EK`: an animated screen's frame-index function runs
    /// on the clock and walks its bitmap's images.
    #[test]
    fn halo_4_frame_index_walks_a_bitmaps_images() {
        let Ok(root) = std::env::var("BLAM_TEST_H4EK") else {
            return;
        };
        let root = std::path::Path::new(&root);
        let read = |path: &str| blam_tags::TagFile::read(root.join(path)).unwrap();
        let material = blam_tags::material::Material::from_tag(&read(
            "environments/shared/materials/cinematics/screens/cin_unsc_screen_crest_anim_b.material",
        ));
        let shader = read(&format!("{}.material_shader", material.shader.replace('\\', "/")));
        let shader = blam_tags::material::MaterialShader::from_tag(&shader);
        let baked = blam_tags::material::bake(&material, &shader, &|_: &str| Some(false));
        let texture = baked.textures.iter().find(|texture| texture.frame_index_function.is_some()).expect("a framed texture");
        let function = texture.frame_index_function.unwrap();
        let bitmap = read(&format!("{}.bitmap", texture.bitmap.replace('\\', "/")));
        let count = super::super::ce_attachments::elements(bitmap.root(), "bitmaps*").len();
        assert!(count > 1, "{count}");
        let mut submit = H4Submit::new(baked, 1, 8, None);
        submit.add_frames("selfillum_mapTexture".to_owned(), function, count);
        let at = |time: f32| submit.frame("selfillum_mapTexture", &DrawnObject { values: None, time, owner: 0 }).unwrap();
        let frames: std::collections::HashSet<usize> = (0..40).map(|step| at(step as f32 * 0.05)).collect();
        assert!(frames.len() > 1, "{frames:?}");
        assert!(frames.iter().all(|frame| *frame < count));
        assert!(submit.frame("other", &DrawnObject { values: None, time: 0.0, owner: 0 }).is_none());
    }
}
