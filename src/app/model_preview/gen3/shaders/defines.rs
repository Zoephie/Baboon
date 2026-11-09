//! The preprocessor defines a pass compiles with: the base set and each chosen option's.

use super::*;

/// `sub_140C587A0`: what every shader compiles with. Reach's `sub_141171B10`
/// adds `_standard_vs`, without which every vertex shader body is compiled
/// out, and `disable_register_reorder`.
pub(in crate::app::model_preview) fn base_defines(
    dialect: RenderMethodDialect,
    vertex_type: usize,
    stage: Stage,
) -> Vec<(String, String)> {
    let (vertex, deform) = VERTEX_TYPES[vertex_type];
    let stage = match stage {
        Stage::Vertex => "VERTEX_SHADER",
        Stage::Pixel => "PIXEL_SHADER",
    };
    let mut defines = vec![
        ("vertex_type", vertex),
        ("deform", deform),
        ("SHADER_30", "1"),
        ("pc", "1"),
        ("DX_VERSION", "11"),
        ("PARAM_ALLOC_FIRST_PASS", "1"),
        (stage, "1"),
    ];
    if dialect == RenderMethodDialect::Reach {
        defines.extend([("_standard_vs", "1"), ("disable_register_reorder", "1")]);
    }
    defines.into_iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect()
}

/// `sub_140C589E0`: each category's function, as its option's. Reach's
/// `build_options_macro` (0x141171D10) also takes no option at all, which
/// writes no function and no choice but still names every option.
pub(super) fn option_defines(definition: &Gen3Definition, options: &[Option<usize>]) -> Vec<(String, String)> {
    let mut defines = Vec::new();
    for (index, category) in definition.categories.iter().enumerate() {
        let chosen = options.get(index).copied().unwrap_or(Some(0));
        let chosen = chosen.map(|chosen| chosen.min(category.options.len().saturating_sub(1)));
        let option = chosen.and_then(|chosen| category.options.get(chosen));
        if option.is_none() && !(chosen.is_none() && definition.automatic_macros) {
            continue;
        }
        for (name, value) in [
            (&category.vertex_function, option.and_then(|option| option.vertex_function.as_ref())),
            (&category.pixel_function, option.and_then(|option| option.pixel_function.as_ref())),
        ] {
            if let (Some(name), Some(value)) = (name, value) {
                defines.push((name.clone(), value.clone()));
            }
        }
        if definition.automatic_macros {
            let prefix = format!("category_{}", category.name);
            if let Some(option) = option {
                defines.push((prefix.clone(), format!("{prefix}_option_{}", option.name)));
            }
            for (index, option) in category.options.iter().enumerate() {
                defines.push((format!("{prefix}_option_{}", option.name), index.to_string()));
            }
        }
    }
    defines
}
