//! import info export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;
use flate2::read::ZlibDecoder;
use std::io::{BufWriter, Read, Write};

pub(in crate::app) fn extract_import_info_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    extract_import_info_from_tag(&tag, &entry.display_path, output)
}

/// Extract import-info streams from the render, physics, and collision tags
/// referenced by a Halo model (hlmt) tag.
pub(in crate::app) fn extract_import_info_for_model_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let model = read_entry(source, entry)?;
    let root = model.root();
    let references = [
        (
            "render_model",
            tag_ref_path(&root, "render model"),
            "render_model",
            *b"mode",
        ),
        (
            "physics_model",
            tag_ref_path(&root, "physics_model").or_else(|| tag_ref_path(&root, "physics model")),
            "physics_model",
            *b"phmo",
        ),
        (
            "collision_model",
            tag_ref_path(&root, "collision model"),
            "collision_model",
            *b"coll",
        ),
    ];
    let mut extracted = 0usize;
    let mut failures = Vec::new();

    for (folder, reference, extension, group_tag) in references {
        let Some(reference) = reference else {
            failures.push(format!("{folder}: no reference"));
            continue;
        };
        let target_output = output.join(folder);
        match load_referenced_tag_from_source(source, &reference, extension, &group_tag).and_then(
            |tag| {
                extract_import_info_from_tag(
                    &tag,
                    &format!("{reference}.{extension}"),
                    &target_output,
                )
            },
        ) {
            Ok(_) => extracted += 1,
            Err(error) => failures.push(format!("{folder}: {error}")),
        }
    }

    if extracted == 0 {
        anyhow::bail!(
            "no referenced model import-info streams were extracted: {}",
            failures.join("; ")
        );
    }
    let mut message = format!(
        "Extracted import info from {extracted} referenced model tag(s) to {}",
        output.display()
    );
    if !failures.is_empty() {
        message.push_str(&format!("; skipped {}", failures.join("; ")));
    }
    Ok(message)
}

fn extract_import_info_from_tag(
    tag: &TagFile,
    _display_path: &str,
    output: &Path,
) -> anyhow::Result<String> {
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

pub(in crate::app) fn resolve_import_info_struct<'a>(
    tag: &'a TagFile,
    root: TagStruct<'a>,
) -> Option<TagStruct<'a>> {
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

pub(in crate::app) fn sanitize_import_info_path(input: &str) -> PathBuf {
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
