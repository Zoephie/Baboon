//! shader source export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(in crate::app) fn extract_material_shader_sources(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let written = write_material_shader_sources(source, entry, output)?;
    Ok(format!(
        "Extracted {written} source shader file(s) to {}",
        output.display()
    ))
}

pub(in crate::app) fn extract_material_shader_source_entries(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    fs::create_dir_all(output)?;
    let mut total_written = 0usize;
    let mut total_tags = 0usize;
    let mut failures = Vec::new();

    for entry in entries
        .iter()
        .filter(|entry| is_material_shader_group(entry.group_tag))
    {
        match write_material_shader_sources(source, entry, output) {
            Ok(count) => {
                total_written += count;
                total_tags += 1;
            }
            Err(error) => failures.push(format!("{}: {error}", entry.display_path)),
        }
    }

    if total_written == 0 && !failures.is_empty() {
        anyhow::bail!(
            "failed to extract material shader sources: {}",
            failures.join("; ")
        );
    }
    if total_written == 0 {
        anyhow::bail!("no material shader sources found");
    }

    let mut message = format!(
        "Extracted {total_written} source shader file(s) from {total_tags} material shader tag(s) to {}",
        output.display()
    );
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

fn write_material_shader_sources(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<usize> {
    if !is_material_shader_group(entry.group_tag) {
        anyhow::bail!(
            "source shader extraction is only available for material_shader tags, got {}",
            format_group_tag(entry.group_tag)
        );
    }

    let tag = read_entry(source, entry)?;
    let source_files = field_by_clean_key(tag.root(), "source shader files")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("material_shader has no source shader files block"))?;
    if source_files.is_empty() {
        anyhow::bail!("material_shader has no source shader files");
    }

    fs::create_dir_all(output)?;
    let mut written = 0usize;
    let mut skipped = Vec::new();
    for (index, element) in source_files.iter().enumerate() {
        let shader_path = match long_string_by_clean_key(element, "shader path") {
            Some(path) if !path.trim().is_empty() => path,
            _ => {
                skipped.push(format!("source shader {index}: missing shader path"));
                continue;
            }
        };
        let Some(shader_data) = data_by_clean_key(element, "shader data") else {
            skipped.push(format!("{shader_path}: missing shader data"));
            continue;
        };
        if shader_data.is_empty() {
            skipped.push(format!("{shader_path}: empty shader data"));
            continue;
        }

        let relative_path = material_shader_source_relative_path(&shader_path, index);
        let output_path = output.join(relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output_path, shader_data)?;
        written += 1;
    }

    if written == 0 && !skipped.is_empty() {
        anyhow::bail!(
            "source shader extraction emitted nothing: {}",
            skipped.join("; ")
        );
    }
    if written == 0 {
        anyhow::bail!("source shader extraction emitted nothing");
    }

    Ok(written)
}

pub(in crate::app) fn extract_hlsl_include_source(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let output_path = write_hlsl_include_source(source, entry, output)?;
    Ok(format!("Extracted HLSL include {}", output_path.display()))
}

pub(in crate::app) fn extract_hlsl_include_entries(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    fs::create_dir_all(output)?;
    let mut written = 0usize;
    let mut failures = Vec::new();

    for entry in entries.iter().filter(|entry| is_hlsl_include_tag(entry)) {
        match write_hlsl_include_source(source, entry, output) {
            Ok(_) => written += 1,
            Err(error) => failures.push(format!("{}: {error}", entry.display_path)),
        }
    }

    if written == 0 && !failures.is_empty() {
        anyhow::bail!("failed to extract HLSL includes: {}", failures.join("; "));
    }
    if written == 0 {
        anyhow::bail!("no HLSL includes found");
    }

    let mut message = format!(
        "Extracted {written} HLSL include file(s) to {}",
        output.display()
    );
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

fn write_hlsl_include_source(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<PathBuf> {
    if !is_hlsl_include_group(entry.group_tag) {
        anyhow::bail!(
            "HLSL include extraction is only available for hlsl_include tags, got {}",
            format_group_tag(entry.group_tag)
        );
    }

    let tag = read_entry(source, entry)?;
    let include_file = data_by_clean_key(tag.root(), "include file")
        .ok_or_else(|| anyhow::anyhow!("hlsl_include has no include file data"))?;
    if include_file.is_empty() {
        anyhow::bail!("hlsl_include include file data is empty");
    }

    let relative_path = hlsl_include_source_relative_path(&entry.display_path);
    let output_path = output.join(relative_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, include_file)?;
    Ok(output_path)
}

fn field_by_clean_key<'a>(tag_struct: TagStruct<'a>, key: &str) -> Option<TagField<'a>> {
    tag_struct
        .fields()
        .find(|field| clean_field_key(field.name()) == key)
}

fn long_string_by_clean_key(tag_struct: TagStruct<'_>, key: &str) -> Option<String> {
    match field_by_clean_key(tag_struct, key)?.value()? {
        TagFieldData::LongString(value) | TagFieldData::String(value) => Some(value),
        _ => None,
    }
}

fn data_by_clean_key<'a>(tag_struct: TagStruct<'a>, key: &str) -> Option<&'a [u8]> {
    field_by_clean_key(tag_struct, key)?.as_data()
}

fn material_shader_source_relative_path(shader_path: &str, index: usize) -> PathBuf {
    shader_source_relative_path(shader_path, index, "fx")
}

fn hlsl_include_source_relative_path(display_path: &str) -> PathBuf {
    let mut relative = shader_source_relative_path(display_path, 0, "hlsl");
    if relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("hlsl_include"))
    {
        relative.set_extension("hlsl");
    }
    relative
}

fn shader_source_relative_path(
    source_path: &str,
    index: usize,
    default_extension: &str,
) -> PathBuf {
    let cleaned = source_path.replace('\0', "");
    let mut components = cleaned
        .trim()
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(sanitize_shader_path_segment)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if components
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case("data"))
    {
        components.remove(0);
    }

    if components.is_empty() {
        components.push(format!("shader_{index:03}"));
    }

    let mut relative = PathBuf::new();
    for component in components {
        relative.push(component);
    }
    if relative.extension().is_none() {
        relative.set_extension(default_extension);
    }
    relative
}

fn sanitize_shader_path_segment(segment: &str) -> String {
    let segment = segment.trim();
    if segment.len() == 2
        && segment.ends_with(':')
        && segment
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
    {
        return String::new();
    }
    segment
        .chars()
        .filter_map(|ch| match ch {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => Some('_'),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect()
}

#[cfg(test)]
mod material_shader_source_tests {
    use super::*;

    /// Build an expected relative path from components, so the comparison uses
    /// the native separator on any platform (the function joins via `PathBuf`).
    fn rel(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn material_shader_source_path_strips_data_and_adds_fx() {
        assert_eq!(
            material_shader_source_relative_path(r"data\shaders\material_shaders\decals\base", 0),
            rel(&["shaders", "material_shaders", "decals", "base.fx"])
        );
    }

    #[test]
    fn material_shader_source_path_preserves_existing_extension() {
        assert_eq!(
            material_shader_source_relative_path(
                r"data\shaders\material_shaders\include\core\lighting.hlsli",
                1
            ),
            rel(&[
                "shaders",
                "material_shaders",
                "include",
                "core",
                "lighting.hlsli"
            ])
        );
    }

    #[test]
    fn material_shader_source_path_cannot_escape_output_folder() {
        assert_eq!(
            material_shader_source_relative_path(r"C:\data\..\shaders\bad:name\base", 2),
            rel(&["shaders", "bad_name", "base.fx"])
        );
    }

    #[test]
    fn hlsl_include_source_path_preserves_hlsl_extension() {
        assert_eq!(
            hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao.hlsl"),
            rel(&["rasterizer", "hlsl", "ssao.hlsl"])
        );
    }

    #[test]
    fn hlsl_include_source_path_replaces_friendly_tag_extension() {
        assert_eq!(
            hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao.hlsl_include"),
            rel(&["rasterizer", "hlsl", "ssao.hlsl"])
        );
    }

    #[test]
    fn hlsl_include_source_path_adds_hlsl_extension() {
        assert_eq!(
            hlsl_include_source_relative_path(r"rasterizer\hlsl\ssao"),
            rel(&["rasterizer", "hlsl", "ssao.hlsl"])
        );
    }
}
