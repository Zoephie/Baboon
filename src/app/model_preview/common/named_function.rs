//! A function a tag drives by name: what it reads as its input and range,
//! and its `Mapping`. How each game samples one differs, and stays with it.

use std::sync::Arc;

use blam_tags::{TagFunction, TagStruct};

use crate::app::model_preview::dx9::passes::tag_function_at;

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
