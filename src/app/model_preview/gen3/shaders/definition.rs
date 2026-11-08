//! A render method definition: its categories, options, vertex types and shared stages, as read off the tag.

use super::*;

/// Indices into [`VERTEX_TYPES`].
#[cfg(test)]
pub(crate) const VERTEX_RIGID: usize = 1;

pub(crate) const VERTEX_SKINNED: usize = 2;

/// tool's tables of entry point, vertex type and deform names, in the order
/// of [`VERTEX_TYPE_OPTIONS`].
pub(super) const VERTEX_TYPES: [(&str, &str); 3] = [
    ("s_world_vertex", "deform_world"),
    ("s_rigid_vertex", "deform_rigid"),
    ("s_skinned_vertex", "deform_skinned"),
];

/// Reach's `shared pixel shader compilation`, and what its tags' layouts
/// label it.
const SHARED_PIXEL: [&str; 2] = ["shared pixel shader compilation", "shared entry point compilation"];

/// The `vertex type` options [`VERTEX_TYPES`] stand for.
const VERTEX_TYPE_OPTIONS: [&str; 3] = ["world", "rigid", "skinned"];

#[derive(Debug, Clone)]
pub(super) struct H3Option {
    pub(super) name: String,
    pub(super) vertex_function: Option<String>,
    pub(super) pixel_function: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct H3Category {
    pub(super) name: String,
    pub(super) vertex_function: Option<String>,
    pub(super) pixel_function: Option<String>,
    pub(super) options: Vec<H3Option>,
}

/// What Reach's tool reads of an entry point's first pass to pick the
/// options each stage compiles with (`sub_140342850` builds the shared
/// shaders; `get_compiled_vertex_shader` 0x14119A2A0 and
/// `get_compiled_pixel_shader` 0x14119A170 choose among them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct H3SharedStages {
    /// Flag 0x1: the pixel shader is shared, not the material's own.
    shared_pixel: bool,
    /// Flag 0x2: the vertex shaders only tell the default option of their
    /// category from any other, so only options 0 and 1 are compiled.
    vertex_two_options: bool,
    vertex_category: Option<usize>,
    pixel_category: Option<usize>,
}

/// A `render_method_definition`, as far as compiling goes.
#[derive(Debug, Clone)]
pub(crate) struct H3Definition {
    /// The family: `shader`, `terrain`..
    pub(crate) name: String,
    dialect: RenderMethodDialect,
    /// The file it compiles, as an `#include` names it: Halo 3's `shader.fx`,
    /// Reach's `templated\shader.fx`.
    pub(super) source_file: String,
    /// `use automatic macros`: categories and options named as defines too.
    pub(super) automatic_macros: bool,
    pub(super) categories: Vec<H3Category>,
    pub(crate) entry_points: Vec<String>,
    /// Reach's shared-shader rules by entry point; Halo 3 has none, and
    /// compiles both stages with the material's options.
    shared: std::collections::BTreeMap<String, H3SharedStages>,
    /// The first `vertex_types` entry, which every pixel shader compiles with.
    pub(super) pixel_vertex_type: usize,
}

pub(crate) fn read_h3_definition(reader: &TagReader, path: &str) -> Option<H3Definition> {
    let dialect = RenderMethodDialect::of(&reader.source);
    let tag = reader.load(path, "render_method_definition", b"rmdf").ok()?;
    let root = tag.root();
    let function = |holder: TagStruct<'_>, name: &str| holder.read_string_id(name).filter(|name| !name.is_empty());
    let categories = elements(root, "categories")
        .into_iter()
        .map(|category| H3Category {
            name: category.read_string_id("category name").unwrap_or_default(),
            vertex_function: function(category, "vertex function"),
            pixel_function: function(category, "pixel function"),
            options: elements(category, "options")
                .into_iter()
                .map(|option| H3Option {
                    name: option.read_string_id("option name").unwrap_or_default(),
                    vertex_function: function(option, "vertex function"),
                    pixel_function: function(option, "pixel function"),
                })
                .collect(),
        })
        .collect();
    // Reach lists entry points with no passes (glass's and mux's `default`),
    // which tool compiles nothing for and the source has no function for.
    let mut entry_points = Vec::new();
    let mut shared = std::collections::BTreeMap::new();
    for entry in elements(root, "entry_points") {
        let passes = elements(entry, "passes");
        if dialect == RenderMethodDialect::Reach && passes.is_empty() {
            continue;
        }
        let Some(name) = entry.read_enum_option("entry point") else {
            continue;
        };
        let name = entry_name(dialect, &name).to_owned();
        if dialect == RenderMethodDialect::Reach {
            let first = passes[0];
            // `sub_140342850` asserts a stage depends on fewer than two.
            let category = |block: &str| {
                elements(first, block).first().and_then(|dependency| dependency.read_int_any("category"))
                    .and_then(|category| usize::try_from(category).ok())
            };
            shared.insert(name.clone(), H3SharedStages {
                shared_pixel: has_flag_labelled(first, "flags", &SHARED_PIXEL),
                vertex_two_options: has_flag(first, "flags", "shared VS only care non-default option of depended category"),
                vertex_category: category("shared VS category dependencies"),
                pixel_category: category("category dependencies"),
            });
        }
        entry_points.push(name);
    }
    let name = path.rsplit('\\').next()?.to_owned();
    // Reach: `c_render_method::postprocess` (tool 0x141165700) compiles
    // `shaders\<location>`; `generate-shared-shaders` (0x140345BD0) falls back on
    // `templated\<name>` when it is empty. Halo 3 has no `location`.
    let source_file = match root.read_string("location") {
        Some(location) if !location.is_empty() => format!("{location}.fx"),
        Some(_) => format!("templated\\{name}.fx"),
        None => format!("{name}.fx"),
    };
    let automatic_macros = has_flag(root, "flags", "use automatic macros");
    let pixel_vertex_type = elements(root, "vertex_types")
        .first()
        .and_then(|first| first.read_enum_option("vertex type"))
        .and_then(|kind| VERTEX_TYPE_OPTIONS.iter().position(|option| *option == kind))
        .unwrap_or(0);
    Some(H3Definition { name, dialect, source_file, automatic_macros, categories, entry_points, shared, pixel_vertex_type })
}

impl H3Definition {
    pub(crate) fn is_decal(&self) -> bool {
        self.name == "decal"
    }

    /// The name of the option chosen in a category.
    pub(crate) fn chosen(&self, category: &str, options: &[usize]) -> Option<&str> {
        let at = self.categories.iter().position(|known| known.name == category)?;
        let option = options.get(at).copied().unwrap_or(0);
        Some(self.categories[at].options.get(option)?.name.as_str())
    }

    /// A decal's `specular` option is `modulate`, rather than `leave`.
    pub(crate) fn decal_modulates_specular(&self, options: &[usize]) -> bool {
        self.chosen("specular", options) == Some("modulate")
    }

    /// A decal's `render_pass`: it goes on before the lights or after them.
    pub(crate) fn after_lighting(&self, options: &[usize]) -> bool {
        self.chosen("render_pass", options) == Some("post_lighting")
    }

    /// The options one stage of `entry` compiles with: the material's own in
    /// Halo 3. In Reach a vertex shader is shared by every material of the
    /// family, compiled with each option at its default (none, under
    /// automatic macros) except the one category it depends on; a shared
    /// pixel shader the same.
    pub(super) fn stage_options(&self, options: &[usize], entry: &str, stage: Stage) -> Vec<Option<usize>> {
        let own = || options.iter().copied().map(Some).collect();
        let Some(rules) = self.shared.get(entry) else {
            return own();
        };
        let (category, most) = match stage {
            Stage::Vertex => (rules.vertex_category, if rules.vertex_two_options { 2 } else { usize::MAX }),
            Stage::Pixel if rules.shared_pixel => (rules.pixel_category, usize::MAX),
            Stage::Pixel => return own(),
        };
        let default = (!self.automatic_macros).then_some(0);
        let mut chosen = vec![default; self.categories.len()];
        if let Some(category) = category.filter(|category| *category < chosen.len()) {
            let count = self.categories[category].options.len().min(most);
            let option = options.get(category).copied().unwrap_or(0);
            chosen[category] = Some(option.min(count.saturating_sub(1)));
        }
        chosen
    }

    /// The `blend_mode` category's choice, for a method held in another tag.
    pub(crate) fn blend(&self, options: &[usize]) -> H3Blend {
        self.blend_of(options, false)
    }

    /// The `blend_mode` category's choice, as each game's bake reads its name:
    /// Halo 3's `blend_mode_from_string` ignoring case, Reach's
    /// `get_alpha_blend_mode_by_name` (0x141167210) minding it. Without the
    /// category Reach blends `shader_glass` and nothing else.
    pub(crate) fn blend_of(&self, options: &[usize], glass: bool) -> H3Blend {
        let Some(name) = self.chosen("blend_mode", options) else {
            return if glass && self.dialect == RenderMethodDialect::Reach { H3Blend::AlphaBlend } else { H3Blend::Opaque };
        };
        let index = match self.dialect {
            RenderMethodDialect::Halo3 => {
                let name = name.to_ascii_lowercase();
                BLEND_MODES.iter().position(|mode| name.starts_with(mode))
            }
            // Reach's entries past `motion_blur_inhibit` are its own; no
            // Reach definition offers them as options, so none reach here.
            RenderMethodDialect::Reach => REACH_BLEND_MODES[..13].iter().position(|mode| name.starts_with(mode)),
        };
        H3Blend::ALL[index.unwrap_or(0)]
    }
}

/// tool's name for an entry point, where the tag's enum spells it for people.
fn entry_name(dialect: RenderMethodDialect, option: &str) -> &str {
    match (dialect, option) {
        (_, "vertex color lighting") => "static_per_vertex_color",
        (_, "water tessellation") => "water_tessellation",
        (_, "water shading") => "water_shading",
        // Reach tool's `entry_point` table, `off_14290E110`.
        (RenderMethodDialect::Reach, "imposter_static_lighting_sh") => "imposter_static_sh",
        (RenderMethodDialect::Reach, "imposter_static_lighting_prt_ambient") => "imposter_static_prt_ambient",
        (_, other) => other,
    }
}

/// The function each stage compiles. Some entry points share one. Reach's
/// are tool's `off_14290DCC0` (vertex) and `off_14290DBD0` (pixel).
pub(super) fn entry_functions(dialect: RenderMethodDialect, entry: &str) -> (String, String) {
    let pair = |vertex: &str, pixel: &str| (vertex.to_owned(), pixel.to_owned());
    match (dialect, entry) {
        (_, "static_prt_ambient" | "static_prt_linear" | "static_prt_quadratic") => {
            (format!("{entry}_vs"), "static_prt_ps".to_owned())
        }
        (RenderMethodDialect::Halo3, "dynamic_light_cinematic") => pair("dynamic_light_cine_vs", "dynamic_light_cine_ps"),
        (RenderMethodDialect::Reach, "dynamic_light") => pair("default_dynamic_light_vs", "dynamic_light_ps"),
        (RenderMethodDialect::Reach, "dynamic_light_cinematic") => pair("default_dynamic_light_vs", "dynamic_light_cine_ps"),
        (RenderMethodDialect::Reach, "dynamic_light_hq_shadows") => pair("default_dynamic_light_vs", "dynamic_light_hq_shadows_ps"),
        (RenderMethodDialect::Reach, "dynamic_light_cinematic_hq_shadows") => {
            pair("default_dynamic_light_vs", "dynamic_light_cine_hq_shadows_ps")
        }
        (RenderMethodDialect::Reach, "imposter_static_prt_ambient") => pair("imposter_static_prt_ambient_vs", "imposter_static_prt_ps"),
        (_, other) => (format!("{other}_vs"), format!("{other}_ps")),
    }
}

#[cfg(test)]
mod stage_option_tests {
    use super::super::*;

    fn category(name: &str, options: &[&str]) -> H3Category {
        let option = |name: &&str| H3Option {
            name: (*name).to_owned(),
            vertex_function: None,
            pixel_function: Some(format!("calc_{name}")),
        };
        H3Category {
            name: name.to_owned(),
            vertex_function: None,
            pixel_function: Some(format!("calc_{name}_ps")),
            options: options.iter().map(option).collect(),
        }
    }

    fn definition(automatic_macros: bool, shared: &[(&str, H3SharedStages)]) -> H3Definition {
        H3Definition {
            name: "shader".to_owned(),
            dialect: RenderMethodDialect::Reach,
            source_file: "templated\\shader.fx".to_owned(),
            automatic_macros,
            categories: vec![
                category("albedo", &["default", "detail", "constant"]),
                category("blend_mode", &["opaque", "additive", "multiply", "alpha_blend"]),
            ],
            entry_points: shared.iter().map(|(entry, _)| (*entry).to_owned()).collect(),
            shared: shared.iter().map(|(entry, rules)| ((*entry).to_owned(), *rules)).collect(),
            pixel_vertex_type: 0,
        }
    }

    /// A decal leaves the albedo's specular mask unless its `specular` is
    /// `modulate`; a definition without the category leaves it too.
    #[test]
    fn only_a_decal_set_to_modulate_writes_the_specular_mask() {
        let mut decal = definition(false, &[]);
        decal.categories.push(category("specular", &["leave", "modulate"]));
        assert!(!decal.decal_modulates_specular(&[0, 3, 0]));
        assert!(decal.decal_modulates_specular(&[0, 3, 1]));
        assert!(!definition(false, &[]).decal_modulates_specular(&[0, 3]));
    }

    #[test]
    fn reach_reads_blend_names_minding_case_and_blends_glass_without_a_category() {
        let mut reach = definition(false, &[]);
        reach.categories[1].options[1].name = "ADDITIVE".to_owned();
        assert_eq!(reach.blend(&[0, 1]), H3Blend::Opaque, "Reach minds case");
        assert_eq!(reach.blend(&[0, 3]), H3Blend::AlphaBlend);
        reach.dialect = RenderMethodDialect::Halo3;
        assert_eq!(reach.blend(&[0, 1]), H3Blend::Additive, "Halo 3 does not");

        let mut glass = definition(false, &[]);
        glass.categories.truncate(1);
        assert_eq!(glass.blend_of(&[0], true), H3Blend::AlphaBlend);
        assert_eq!(glass.blend_of(&[0], false), H3Blend::Opaque);
        glass.dialect = RenderMethodDialect::Halo3;
        assert_eq!(glass.blend_of(&[0], true), H3Blend::Opaque);
    }

    #[test]
    fn halo_3_compiles_both_stages_with_the_materials_options() {
        let halo3 = definition(false, &[]);
        assert_eq!(halo3.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(2), Some(3)]);
        assert_eq!(halo3.stage_options(&[2, 3], "static_sh", Stage::Pixel), [Some(2), Some(3)]);
    }

    #[test]
    fn a_reach_vertex_shader_keeps_only_its_category_and_only_two_of_its_options() {
        let rules = H3SharedStages { vertex_two_options: true, vertex_category: Some(1), ..Default::default() };
        let reach = definition(false, &[("static_sh", rules)]);
        assert_eq!(reach.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(0), Some(1)], "alpha_blend shares additive's");
        assert_eq!(reach.stage_options(&[2, 0], "static_sh", Stage::Vertex), [Some(0), Some(0)]);
        assert_eq!(reach.stage_options(&[2, 3], "static_sh", Stage::Pixel), [Some(2), Some(3)], "the material's own pixel shader");
        let uncapped = definition(false, &[("static_sh", H3SharedStages { vertex_two_options: false, ..rules })]);
        assert_eq!(uncapped.stage_options(&[2, 3], "static_sh", Stage::Vertex), [Some(0), Some(3)]);
    }

    #[test]
    fn a_shared_pixel_shader_reads_only_its_category_and_automatic_macros_default_to_none() {
        let rules = H3SharedStages { shared_pixel: true, pixel_category: Some(0), ..Default::default() };
        let reach = definition(true, &[("shadow_generate", rules)]);
        assert_eq!(reach.stage_options(&[2, 3], "shadow_generate", Stage::Pixel), [Some(2), None]);
        assert_eq!(reach.stage_options(&[2, 3], "shadow_generate", Stage::Vertex), [None, None]);
    }

    #[test]
    fn no_option_names_every_option_but_chooses_none() {
        let reach = definition(true, &[]);
        let defines = option_defines(&reach, &[None, Some(3)]);
        let has = |name: &str| defines.iter().any(|(known, _)| known == name);
        assert!(!has("calc_albedo_ps"), "no function without an option: {defines:?}");
        assert!(!has("category_albedo"), "{defines:?}");
        assert!(has("category_albedo_option_detail"), "{defines:?}");
        assert!(defines.contains(&("calc_blend_mode_ps".to_owned(), "calc_alpha_blend".to_owned())));
        assert!(defines.contains(&("category_blend_mode".to_owned(), "category_blend_mode_option_alpha_blend".to_owned())));
    }
}
