//! A function a tag drives by name: what it reads as its input and range,
//! and its `Mapping`. How each game samples one differs, and stays with it.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

/// `Input Variable`, `Range Variable` and `Mapping` under one field.
#[derive(Debug, Clone)]
pub(in crate::app::model_preview) struct NamedFunction {
    pub(in crate::app::model_preview) input: String,
    pub(in crate::app::model_preview) range: String,
    pub(in crate::app::model_preview) function: Arc<TagFunction>,
}

impl NamedFunction {
    /// The function under `root`'s `name`, or none when it has no mapping.
    pub(in crate::app::model_preview) fn read(root: TagStruct<'_>, name: &str) -> Option<Self> {
        let holder = root.descend(name)?;
        Some(Self {
            input: holder.read_string_id("Input Variable").unwrap_or_default(),
            range: holder.read_string_id("Range Variable").unwrap_or_default(),
            function: Arc::new(tag_function_at(&holder, "Mapping")?),
        })
    }
}

/// The `mapping_function` `name` under `holder`, as the engine's function:
/// a later engine's blob, a Halo 2 byte block, or Halo 2's laid-out fields.
pub(in crate::app::model_preview) fn tag_function_at(holder: &TagStruct<'_>, name: &str) -> Option<TagFunction> {
    let Some(field) = holder.field_path(&format!("{name}/data")) else {
        return h2_laid_out_function(holder.descend(name)?);
    };
    // Later engines keep the bytes as one blob, in their own encoding.
    if let Some(data) = field.as_data() {
        return TagFunction::parse(data).ok();
    }
    let data = field.as_block()?;
    let bytes: Vec<u8> = (0..data.len())
        .filter_map(|index| data.element(index))
        .filter_map(|byte| byte.read_int_any("Value").map(|value| value as i8 as u8))
        .collect();
    crate::app::editor::function_editor::h2_tag_function(&bytes)
}

/// Some of Halo 2's functions lay the engine's block out as fields: the header,
/// its four colors and the graphs' reals. Put back together it is what `data` is.
fn h2_laid_out_function(mapping: TagStruct<'_>) -> Option<TagFunction> {
    // Raw, for what a ranged function keeps in its colors is its clamps, as
    // reals, and read as colors they would lose their top bytes. Older tags
    // stop short of the last two slots.
    let mut bytes = mapping.raw().to_vec();
    bytes.resize(blam_tags::tag_function::h2::HEADER_SIZE, 0);
    for value in crate::app::model_preview::common::fields::elements(mapping, "Values") {
        bytes.extend(value.read_real("Value")?.to_le_bytes());
    }
    crate::app::editor::function_editor::h2_tag_function(&bytes)
}
