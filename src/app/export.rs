use super::*;
use flate2::read::ZlibDecoder;
use std::io::{BufWriter, Read, Write};

pub(super) fn export_tag_json(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let value = tag_to_json(&tag, entry);
    let text = serde_json::to_string_pretty(&value)?;
    fs::write(output, text)?;
    Ok(format!("Wrote JSON {}", output.display()))
}

pub(super) fn export_loose_folder_json(
    root: &Path,
    rel_path: &Path,
    names: &TagNameIndex,
    output: &Path,
) -> anyhow::Result<String> {
    let entries = scan_folder_subtree_entries(root, rel_path, names)?;
    if entries.is_empty() {
        anyhow::bail!("no tag files found in {}", root.join(rel_path).display());
    }
    let source = TagSource::LooseFolder {
        root: root.to_path_buf(),
        game: None,
        definitions_root: PathBuf::new(),
    };
    export_tag_json_entries(&source, &entries, output)
}

pub(super) fn export_tag_json_entries(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    fs::create_dir_all(output)?;
    let mut written = 0usize;
    let mut failures = Vec::new();

    for entry in entries {
        let path = output.join(tag_json_relative_path(entry));
        if let Some(parent) = path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                failures.push(format!("{}: {error}", entry.display_path));
                continue;
            }
        }

        let result = (|| -> anyhow::Result<()> {
            let tag = read_entry(source, entry)?;
            let value = tag_to_json(&tag, entry);
            let text = serde_json::to_string_pretty(&value)?;
            fs::write(&path, text)?;
            Ok(())
        })();

        match result {
            Ok(()) => written += 1,
            Err(error) => failures.push(format!("{}: {error}", entry.display_path)),
        }
    }

    if written == 0 && !failures.is_empty() {
        anyhow::bail!("failed to dump folder JSON: {}", failures.join("; "));
    }
    if written == 0 {
        anyhow::bail!("no tag files found");
    }

    let mut message = format!("Wrote {written} JSON tag file(s) to {}", output.display());
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

pub(super) fn extract_raw_tag(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    tag.write(output)?;
    Ok(format!("Extracted raw tag {}", output.display()))
}

pub(super) fn extract_bitmap_images(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let count = write_bitmap_images(source, entry, output)?;
    Ok(format!(
        "Extracted {count} bitmap image(s) to {}",
        output.display()
    ))
}

pub(super) fn extract_bitmap_entries(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    fs::create_dir_all(output)?;
    let mut total_images = 0usize;
    let mut total_tags = 0usize;
    let mut failures = Vec::new();
    for entry in entries.iter().filter(|entry| is_bitmap_tag(entry)) {
        let entry_output = output.join(tag_display_parent(entry));
        match write_bitmap_images(source, entry, &entry_output) {
            Ok(count) => {
                total_images += count;
                total_tags += 1;
            }
            Err(error) => failures.push(format!("{}: {error}", entry.display_path)),
        }
    }

    if total_images == 0 && !failures.is_empty() {
        anyhow::bail!("failed to extract bitmap tags: {}", failures.join("; "));
    }
    if total_images == 0 {
        anyhow::bail!("no bitmap tags found");
    }

    let mut message = format!(
        "Extracted {total_images} image(s) from {total_tags} bitmap tag(s) to {}",
        output.display()
    );
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

pub(super) fn write_bitmap_images(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<usize> {
    let tag = read_entry(source, entry)?;
    let bitmap = Bitmap::new(&tag)?;
    if bitmap.is_empty() {
        anyhow::bail!("bitmap tag has no images");
    }
    fs::create_dir_all(output)?;
    let stem = tag_file_stem(entry);
    let mut count = 0usize;
    for (index, image) in bitmap.iter().enumerate() {
        let suffix = if bitmap.len() == 1 {
            String::new()
        } else {
            format!("_{index:02}")
        };
        let path = output.join(format!("{stem}{suffix}.tiff"));
        let mut file = fs::File::create(&path)?;
        image.write_tiff(&mut file)?;
        count += 1;
    }
    Ok(count)
}

pub(super) fn extract_geometry_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    match &entry.group_tag.to_be_bytes() {
        b"hlmt" => extract_model_geometry(source, entry, output),
        b"scnr" => run_shell_extraction(source, entry, "extract-geometry", output),
        b"sbsp" => {
            let tag = read_entry(source, entry)?;
            let ass = AssFile::from_scenario_structure_bsp(&tag)?;
            fs::create_dir_all(output)?;
            let path = output.join(format!("{}.ASS", tag_file_stem(entry)));
            let mut file = fs::File::create(&path)?;
            ass.write(&mut file)?;
            Ok(format!("Extracted BSP geometry {}", path.display()))
        }
        b"mode" | b"mod2" => {
            let tag = read_entry(source, entry)?;
            fs::create_dir_all(output)?;
            let stem = tag_file_stem(entry);
            let jms = render_jms_for_game(&tag)?;
            let path = output.join(format!("{stem}.render.jms"));
            let mut file = fs::File::create(&path)?;
            jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
            Ok(format!(
                "Extracted render_model geometry {}",
                path.display()
            ))
        }
        b"coll" => {
            let tag = read_entry(source, entry)?;
            fs::create_dir_all(output)?;
            let stem = tag_file_stem(entry);
            let jms = JmsFile::from_collision_model(&tag)?;
            let path = output.join(format!("{stem}.collision.jms"));
            let mut file = fs::File::create(&path)?;
            jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
            Ok(format!(
                "Extracted collision_model geometry {}",
                path.display()
            ))
        }
        b"phmo" => {
            let tag = read_entry(source, entry)?;
            fs::create_dir_all(output)?;
            let stem = tag_file_stem(entry);
            let jms = JmsFile::from_physics_model(&tag)?;
            let path = output.join(format!("{stem}.physics.jms"));
            let mut file = fs::File::create(&path)?;
            jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
            Ok(format!(
                "Extracted physics_model geometry {}",
                path.display()
            ))
        }
        _ => anyhow::bail!(
            "geometry extraction is not available for {}",
            format_group_tag(entry.group_tag)
        ),
    }
}

pub(super) fn extract_import_info_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let root = tag.root();
    let import_info = resolve_import_info_struct(&tag, root).ok_or_else(|| {
        anyhow::anyhow!(
            "tag has no import info stream or root `import info` block; there is no baked import source to extract"
        )
    })?;
    let files = import_info
        .field("files")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("`info` stream is missing the `files` block"))?;
    if files.is_empty() {
        anyhow::bail!("import-info `files` block is empty");
    }

    fs::create_dir_all(output)?;
    let mut total_compressed = 0u64;
    let mut total_decompressed = 0u64;
    let mut written = 0usize;
    let mut failures = Vec::new();

    for (index, file) in files.iter().enumerate() {
        let source_path = read_import_info_string(&file, "path")
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| format!("file_{index}"));
        let zipped = file
            .field("zipped data")
            .and_then(|field| field.as_data())
            .unwrap_or(&[]);
        total_compressed += zipped.len() as u64;

        let relative_path = sanitize_import_info_path(&source_path);
        let target = output.join(&relative_path);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                failures.push(format!(
                    "{}: create parent failed: {error}",
                    target.display()
                ));
                continue;
            }
        }

        match decompress_import_info_file(zipped, &target) {
            Ok(size) => {
                total_decompressed += size;
                written += 1;
            }
            Err(error) => failures.push(format!("{}: {error}", target.display())),
        }
    }

    if written == 0 && !failures.is_empty() {
        anyhow::bail!("failed to extract import info: {}", failures.join("; "));
    }
    if written == 0 {
        anyhow::bail!("no import-info files were written");
    }

    let mut message = format!(
        "Extracted {written} import-info file(s) to {} ({} bytes compressed -> {} bytes decompressed)",
        output.display(),
        total_compressed,
        total_decompressed
    );
    if !failures.is_empty() {
        message.push_str(&format!("; {} failed", failures.len()));
    }
    Ok(message)
}

fn resolve_import_info_struct<'a>(tag: &'a TagFile, root: TagStruct<'a>) -> Option<TagStruct<'a>> {
    tag.import_info().or_else(|| {
        root.field("import info")
            .and_then(|field| field.as_block())
            .and_then(|block| block.element(0))
    })
}

fn read_import_info_string(tag_struct: &TagStruct<'_>, name: &str) -> Option<String> {
    tag_struct
        .field(name)
        .and_then(|field| field.value())
        .and_then(|value| match value {
            TagFieldData::String(text) | TagFieldData::LongString(text) => Some(text),
            _ => None,
        })
}

pub(super) fn sanitize_import_info_path(input: &str) -> PathBuf {
    let mut text = input.replace('\\', "/");
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            text = text[2..].to_owned();
        }
    }
    while text.starts_with('/') {
        text = text[1..].to_owned();
    }
    let mut out = PathBuf::new();
    for component in Path::new(&text).components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => {}
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("file")
    } else {
        out
    }
}

fn decompress_import_info_file(zipped: &[u8], target: &Path) -> anyhow::Result<u64> {
    let mut decoder = ZlibDecoder::new(zipped);
    let file = fs::File::create(target)?;
    let mut writer = BufWriter::new(file);
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = decoder.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        total += read as u64;
    }
    writer.flush()?;
    Ok(total)
}

pub(super) fn extract_material_shader_sources(
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

pub(super) fn extract_material_shader_source_entries(
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

pub(super) fn extract_hlsl_include_source(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let output_path = write_hlsl_include_source(source, entry, output)?;
    Ok(format!("Extracted HLSL include {}", output_path.display()))
}

pub(super) fn extract_hlsl_include_entries(
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

pub(super) fn extract_model_geometry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let model = read_entry(source, entry)?;
    let root = model.root();
    let render_ref = tag_ref_path(&root, "render model");
    let collision_ref = tag_ref_path(&root, "collision model");
    let physics_ref =
        tag_ref_path(&root, "physics_model").or_else(|| tag_ref_path(&root, "physics model"));
    let stem = tag_file_stem(entry);
    let mut emitted = Vec::new();
    let mut skipped = Vec::new();

    let render_tag = match render_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "render_model", b"mode") {
                Ok(tag) => Some(tag),
                Err(error) => {
                    skipped.push(format!("render: {error}"));
                    None
                }
            }
        }
        None => {
            skipped.push("render: no render_model reference".to_owned());
            None
        }
    };

    let render_jms_for_skeleton = match render_tag.as_ref() {
        Some(tag) => match render_jms_for_game(tag) {
            Ok(jms) => Some(jms),
            Err(error) => {
                skipped.push(format!("render skeleton: {error}"));
                None
            }
        },
        None => None,
    };
    let render_jms_version = render_tag
        .as_ref()
        .map(|tag| blam_tags::game::Game::of(tag).jms_version())
        .unwrap_or(8213);
    let skeleton = render_jms_for_skeleton
        .as_ref()
        .map(|jms| jms.nodes.as_slice());

    if let Some(tag) = render_tag.as_ref() {
        let render_dir = output.join("render");
        fs::create_dir_all(&render_dir)?;
        let game = blam_tags::game::Game::of(tag);
        if matches!(game, blam_tags::game::Game::Halo3) && render_model_prefers_ass(tag) {
            let ass = AssFile::from_render_model(tag)?;
            let path = render_dir.join(format!("{stem}.render.ASS"));
            let mut file = fs::File::create(&path)?;
            ass.write(&mut file)?;
            emitted.push(format!("render {}", path.display()));
        } else if let Some(jms) = render_jms_for_skeleton.as_ref() {
            let path = render_dir.join(format!("{stem}.render.jms"));
            let mut file = fs::File::create(&path)?;
            jms.write(&mut file, render_jms_version)?;
            emitted.push(format!("render {}", path.display()));
        }
    }

    match collision_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "collision_model", b"coll") {
                Ok(tag) => {
                    let collision_dir = output.join("collision");
                    fs::create_dir_all(&collision_dir)?;
                    let jms = if let Some(skeleton) = skeleton {
                        JmsFile::from_collision_model_with_skeleton(&tag, skeleton)?
                    } else {
                        JmsFile::from_collision_model(&tag)?
                    };
                    let path = collision_dir.join(format!("{stem}.collision.jms"));
                    let mut file = fs::File::create(&path)?;
                    jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
                    emitted.push(format!("collision {}", path.display()));
                }
                Err(error) => skipped.push(format!("collision: {error}")),
            }
        }
        None => skipped.push("collision: no collision_model reference".to_owned()),
    }

    match physics_ref.as_deref() {
        Some(reference) => {
            match load_referenced_tag_from_source(source, reference, "physics_model", b"phmo") {
                Ok(tag) => {
                    let physics_dir = output.join("physics");
                    fs::create_dir_all(&physics_dir)?;
                    let jms = if let Some(skeleton) = skeleton {
                        JmsFile::from_physics_model_with_skeleton(&tag, skeleton)?
                    } else {
                        JmsFile::from_physics_model(&tag)?
                    };
                    let path = physics_dir.join(format!("{stem}.physics.jms"));
                    let mut file = fs::File::create(&path)?;
                    jms.write(&mut file, blam_tags::game::Game::of(&tag).jms_version())?;
                    emitted.push(format!("physics {}", path.display()));
                }
                Err(error) => skipped.push(format!("physics: {error}")),
            }
        }
        None => skipped.push("physics: no physics_model reference".to_owned()),
    }

    if emitted.is_empty() {
        anyhow::bail!(
            "model geometry extraction emitted nothing: {}",
            skipped.join("; ")
        );
    }
    let mut message = format!(
        "Extracted {} model geometry file(s) to {}",
        emitted.len(),
        output.display()
    );
    if !skipped.is_empty() {
        message.push_str(&format!("; skipped {}", skipped.join("; ")));
    }
    Ok(message)
}

pub(super) fn load_referenced_tag_from_source(
    source: &TagSource,
    reference: &str,
    extension: &str,
    group_tag: &[u8; 4],
) -> anyhow::Result<TagFile> {
    let group_tag = u32::from_be_bytes(*group_tag);
    match source {
        TagSource::LooseFolder { root, .. } => {
            let path = resolve_tag_path(root, reference, extension);
            let entry = TagEntry {
                key: format!("file:{}", path.display()),
                display_path: format!("{}.{}", reference.replace('\\', "/"), extension),
                group_tag,
                group_name: Some(extension.to_owned()),
                location: TagEntryLocation::LooseFile(path.clone()),
            };
            read_entry(source, &entry)
                .map_err(|error| anyhow::anyhow!("read {} failed: {error}", path.display()))
        }
        TagSource::SingleFile { path } => {
            let root = derive_tags_root(path)
                .or_else(|| path.parent().map(Path::to_path_buf))
                .ok_or_else(|| {
                    anyhow::anyhow!("could not derive a tag root for {}", path.display())
                })?;
            let resolved = resolve_tag_path(&root, reference, extension);
            TagFile::read(&resolved)
                .map_err(|error| anyhow::anyhow!("read {} failed: {error}", resolved.display()))
        }
        TagSource::MonolithicCache { cache, .. } => cache
            .read_tag_by_name(group_tag, reference)
            .map_err(|error| anyhow::anyhow!("read {reference}.{extension} failed: {error}")),
    }
}

pub(super) fn render_jms_for_game(tag: &TagFile) -> anyhow::Result<JmsFile> {
    Ok(match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => JmsFile::from_gbxmodel(tag)?,
        blam_tags::game::Game::Halo2 => JmsFile::from_h2_render_model(tag)?,
        blam_tags::game::Game::Halo3 => JmsFile::from_render_model(tag)?,
    })
}

pub(super) fn render_model_prefers_ass(tag: &TagFile) -> bool {
    let root = tag.root();
    let instance_mesh_index = root
        .field("instance mesh index")
        .and_then(|field| field.value())
        .and_then(|value| match value {
            TagFieldData::LongBlockIndex(index) => Some(index as i64),
            TagFieldData::CustomLongBlockIndex(index) => Some(index as i64),
            TagFieldData::ShortBlockIndex(index) => Some(index as i64),
            TagFieldData::LongInteger(index) => Some(index as i64),
            _ => None,
        })
        .unwrap_or(-1);
    let placements_len = root
        .field("instance placements")
        .and_then(|field| field.as_block())
        .map(|block| block.len())
        .unwrap_or(0);
    instance_mesh_index >= 0 && placements_len > 0
}

pub(super) fn run_shell_extraction(
    source: &TagSource,
    entry: &TagEntry,
    command_name: &str,
    output: &Path,
) -> anyhow::Result<String> {
    let shell = shell_binary_path()?;
    let mut command = Command::new(&shell);
    if let Some(definitions_parent) = locate_definitions_root().parent() {
        command.current_dir(definitions_parent);
    }
    match source {
        TagSource::MonolithicCache { root, .. } => {
            command.arg("--cache").arg(root);
        }
        TagSource::LooseFolder {
            game: Some(game), ..
        } => {
            command.arg("--game").arg(game);
        }
        _ => {}
    }
    command.arg(command_name);
    command.arg(shell_entry_arg(entry)?);
    if command_name == "extract-geometry" && entry.group_tag == u32::from_be_bytes(*b"hlmt") {
        command.arg("all");
    }
    let output_arg = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(output))
            .unwrap_or_else(|_| output.to_path_buf())
    };
    command.arg("--output").arg(output_arg);
    let output_data = command.output()?;
    if !output_data.status.success() {
        let stderr = String::from_utf8_lossy(&output_data.stderr);
        let stdout = String::from_utf8_lossy(&output_data.stdout);
        anyhow::bail!(
            "{} failed: {}{}",
            command_name,
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" {}", stdout.trim())
            }
        );
    }
    let stdout = String::from_utf8_lossy(&output_data.stdout);
    let message = stdout.lines().last().unwrap_or("").trim();
    if message.is_empty() {
        Ok(format!(
            "{} completed into {}",
            command_name,
            output.display()
        ))
    } else {
        Ok(format!("{command_name}: {message}"))
    }
}

pub(super) fn shell_binary_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let file_name = if cfg!(windows) {
        "blam-tag-shell.exe"
    } else {
        "blam-tag-shell"
    };
    if let Some(parent) = exe.parent() {
        let sibling = parent.join(file_name);
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    let fallback = PathBuf::from("target").join("debug").join(file_name);
    if fallback.exists() {
        return Ok(fallback);
    }
    anyhow::bail!("could not find {file_name}; build the workspace with `cargo build --release`")
}

pub(super) fn shell_entry_arg(entry: &TagEntry) -> anyhow::Result<PathBuf> {
    match &entry.location {
        TagEntryLocation::LooseFile(path) => Ok(path.clone()),
        TagEntryLocation::Monolithic { name, group_tag } => Ok(PathBuf::from(format!(
            "{}.{}",
            name,
            format_group_tag(*group_tag)
        ))),
    }
}

pub(super) fn tag_to_json(tag: &TagFile, entry: &TagEntry) -> Value {
    json!({
        "path": entry.display_path,
        "group": format_group_tag(tag.group().tag),
        "group_name": entry.group_name,
        "version": tag.group().version,
        "endian": match tag.endian {
            Endian::Le => "LE",
            Endian::Be => "BE",
        },
        "fields": struct_to_json(tag.root()),
    })
}

pub(super) fn struct_to_json(tag_struct: TagStruct<'_>) -> Value {
    Value::Array(tag_struct.fields().map(field_to_json).collect())
}

pub(super) fn field_to_json(field: TagField<'_>) -> Value {
    if let Some(value) = field.value() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": field.type_name(),
            "value": field_value_to_json(value),
        });
    }
    if let Some(block) = field.as_block() {
        let elements = block.iter().map(struct_to_json).collect::<Vec<_>>();
        return json!({
            "name": clean_field_name(field.name()),
            "type": "block",
            "count": block.len(),
            "elements": elements,
        });
    }
    if let Some(array) = field.as_array() {
        let elements = array.iter().map(struct_to_json).collect::<Vec<_>>();
        return json!({
            "name": clean_field_name(field.name()),
            "type": "array",
            "count": array.len(),
            "elements": elements,
        });
    }
    if let Some(nested) = field.as_struct() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": "struct",
            "fields": struct_to_json(nested),
        });
    }
    if let Some(resource) = field.as_resource() {
        return json!({
            "name": clean_field_name(field.name()),
            "type": "pageable_resource",
            "kind": format!("{:?}", resource.kind()),
            "inline_bytes": resource.inline_bytes().len(),
            "exploded_payload_bytes": resource.exploded_payload().map(|payload| payload.len()),
            "xsync_payload_bytes": resource.xsync_payload().map(|payload| payload.len()),
            "header": resource.as_struct().map(struct_to_json),
        });
    }
    json!({
        "name": clean_field_name(field.name()),
        "type": field.type_name(),
    })
}

pub(super) fn field_value_to_json(value: TagFieldData) -> Value {
    match value {
        TagFieldData::String(s) | TagFieldData::LongString(s) => json!(s),
        TagFieldData::StringId(s) | TagFieldData::OldStringId(s) => {
            json!({ "string": s.string })
        }
        TagFieldData::TagReference(reference) => match reference.group_tag_and_name {
            Some((group_tag, path)) => json!({
                "group": format_group_tag(group_tag),
                "path": path,
            }),
            None => Value::Null,
        },
        TagFieldData::CharInteger(v) => json!(v),
        TagFieldData::ShortInteger(v) => json!(v),
        TagFieldData::LongInteger(v) => json!(v),
        TagFieldData::Int64Integer(v) => json!(v),
        TagFieldData::ByteInteger(v) => json!(v),
        TagFieldData::WordInteger(v) => json!(v),
        TagFieldData::DwordInteger(v) => json!(v),
        TagFieldData::QwordInteger(v) => json!(v),
        TagFieldData::Tag(v) => json!(format_group_tag(v)),
        TagFieldData::CharEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::ShortEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::LongEnum { value, name } => json!({ "value": value, "name": name }),
        TagFieldData::ByteFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::WordFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::LongFlags { value, names } => json!({ "value": value, "names": names }),
        TagFieldData::Data(bytes) => json!({ "bytes": bytes.len() }),
        TagFieldData::ApiInterop(value) => json!({ "raw_bytes": value.raw.len() }),
        TagFieldData::Custom(bytes) => json!({ "bytes": bytes.len() }),
        other => json!(format!("{other:?}")),
    }
}

#[cfg(test)]
mod import_info_tests {
    use super::*;

    #[test]
    fn import_info_paths_are_sanitized_to_relative_paths() {
        assert_eq!(
            sanitize_import_info_path(r"c:\mcc\source\objects\brute\brute.jms"),
            PathBuf::from("mcc")
                .join("source")
                .join("objects")
                .join("brute")
                .join("brute.jms")
        );
        assert_eq!(
            sanitize_import_info_path(r"..\..\escape.jms"),
            PathBuf::from("escape.jms")
        );
        assert_eq!(sanitize_import_info_path(""), PathBuf::from("file"));
    }

    #[test]
    fn h2_render_model_import_info_resolves_from_root_block() {
        let mut tag = TagFile::new(test_definition_path("halo2_mcc/render_model.json")).unwrap();
        {
            let mut root = tag.root_mut();
            let mut import_info_field = root.field_path_mut("import info").unwrap();
            let mut import_info = import_info_field.as_block_mut().unwrap();
            import_info.add_element();
        }

        let root = tag.root();
        let import_info = resolve_import_info_struct(&tag, root).expect("root import info block");

        assert!(import_info.field("files").is_some());
    }
}
