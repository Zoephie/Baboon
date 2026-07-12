//! Tag, bitmap, geometry, animation, and source-code export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(super) mod bitmap;
pub(super) mod geometry;
pub(super) mod import_info;
pub(super) mod json;
pub(super) mod shader_source;

pub(super) use bitmap::*;
pub(super) use geometry::*;
pub(super) use import_info::*;
pub(super) use json::*;
pub(super) use shader_source::*;

pub(super) fn extract_raw_tag(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    tag.write(output)?;
    Ok(format!("Extracted raw tag {}", output.display()))
}
