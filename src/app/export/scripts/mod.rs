//! HaloScript source export/import for scenario tags.
//! It owns the `source files` block <-> `.hsc` folder mapping; interactive UI and
//! document lifecycle management belong elsewhere.

use super::*;

/// Extension used for HaloScript source on disk, both directions.
const HSC_EXTENSION: &str = "hsc";

pub(in crate::app) fn is_scenario_group(group_tag: u32) -> bool {
    group_tag == u32::from_be_bytes(*b"scnr")
}

/// Write every element of the scenario's `source files` block into `output` as a
/// `.hsc` file.
pub(in crate::app) fn extract_scenario_scripts(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    if !is_scenario_group(entry.group_tag) {
        anyhow::bail!(
            "Script extraction is only available for scenario tags, got {}",
            format_group_tag(entry.group_tag)
        );
    }
    let tag = read_entry(source, entry)?;
    write_source_files(&tag, output, &entry.display_path)
}

/// The half of the extract that works on a parsed tag — shared with the
/// round-trip test, which has no `TagSource` to read through.
fn write_source_files(tag: &TagFile, output: &Path, label: &str) -> anyhow::Result<String> {
    let files = read_source_files(tag)?;
    if files.is_empty() {
        anyhow::bail!("{label} has no script source files");
    }

    fs::create_dir_all(output)?;
    let mut written = 0usize;
    let mut empty = 0usize;
    for (index, (name, body)) in files.iter().enumerate() {
        // A source blob is stored NUL-terminated (the engine concatenates the
        // terminators too). Editors should not see the terminator, so cut it
        // here and put exactly one back on import.
        let text = source_text(body);
        if text.is_empty() {
            empty += 1;
            continue;
        }
        let path = output.join(hsc_file_name(name, index));
        fs::write(&path, text)?;
        written += 1;
    }
    if written == 0 {
        anyhow::bail!("{label} has no non-empty script source files");
    }

    let mut message = format!("Extracted {written} script file(s) to {}", output.display());
    if empty > 0 {
        message.push_str(&format!("; skipped {empty} empty"));
    }
    Ok(message)
}

/// Replace the scenario's whole `source files` block with the `.hsc` files in
/// `folder`.
///
/// Deliberately touches nothing else: `scripts`, `globals` and `hs syntax
/// datums` still hold the *previously compiled* form of the old source, and the
/// engine runs those, not this text. Clearing them is left to the user.
pub(in crate::app) fn replace_scenario_scripts(
    tag: &mut TagFile,
    folder: &Path,
) -> anyhow::Result<String> {
    let files = read_hsc_folder(folder)?;
    if files.is_empty() {
        anyhow::bail!("No .hsc files found in {}", folder.display());
    }

    let previous = read_source_files(tag).map(|files| files.len()).unwrap_or(0);
    // Each handle borrows the one above it, so they have to stay in scope
    // rather than being chained into a single expression.
    let mut root = tag.root_mut();
    let mut field = root
        .field_mut("source files")
        .ok_or_else(|| anyhow::anyhow!("Tag has no 'source files' block"))?;
    let mut block = field
        .as_block_mut()
        .ok_or_else(|| anyhow::anyhow!("'source files' is not a block"))?;
    block.clear();
    for (name, text) in &files {
        let index = block.add_element();
        let mut element = block
            .element_mut(index)
            .ok_or_else(|| anyhow::anyhow!("Could not address new source file element"))?;
        if let Some(mut field) = element.field_mut("name") {
            field
                .set(TagFieldData::String(name.clone()))
                .map_err(|error| anyhow::anyhow!("Could not set source file name: {error:?}"))?;
        }
        // The blob the engine reads is NUL-terminated, so make sure there is a
        // terminator — but only add one if the file does not already end in it
        // (a folder extracted by some other tool may keep it).
        let mut body = text.clone();
        if body.last() != Some(&0) {
            body.push(0);
        }
        element
            .field_mut("source")
            .ok_or_else(|| anyhow::anyhow!("'source files' element has no 'source' field"))?
            .set(TagFieldData::Data(body))
            .map_err(|error| anyhow::anyhow!("Could not set source file body: {error:?}"))?;
    }

    Ok(format!(
        "Imported {} script file(s) from {} (replacing {previous}); \
         compiled scripts were left as they were",
        files.len(),
        folder.display()
    ))
}

/// `(name, source bytes)` per element of the scenario's `source files` block.
fn read_source_files(tag: &TagFile) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let block = tag
        .root()
        .field_path("source files")
        .and_then(|field| field.as_block())
        .ok_or_else(|| anyhow::anyhow!("Tag has no 'source files' block"))?;
    let mut files = Vec::with_capacity(block.len());
    for index in 0..block.len() {
        let Some(element) = block.element(index) else {
            continue;
        };
        let name = element.read_string("name").unwrap_or_default();
        let source = match element.field_path("source").and_then(|field| field.value()) {
            Some(TagFieldData::Data(data)) => data,
            _ => Vec::new(),
        };
        files.push((name, source));
    }
    Ok(files)
}

/// Read every `.hsc` in `folder` (non-recursive), sorted by file name so a
/// re-import of the same folder produces the same block order every time.
fn read_hsc_folder(folder: &Path) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let is_hsc = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(HSC_EXTENSION));
        if !is_hsc {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        files.push((stem.to_owned(), fs::read(&path)?));
    }
    files.sort_by_key(|(name, _)| name.to_ascii_lowercase());
    Ok(files)
}

/// The text of a source blob: everything up to the first NUL.
///
/// The engine tokenizes each blob as its own C string — a `;` line comment ends
/// at `\0` as well as at `\n` — so the terminator bounds the file and anything
/// after it is padding, not source. Cutting here (rather than trimming a
/// trailing run) is both what the engine reads and the only rule that
/// guarantees no NUL is ever written into a text file.
fn source_text(body: &[u8]) -> &[u8] {
    let end = body
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(body.len());
    &body[..end]
}

/// File name for a source-file element. Falls back to the element index when the
/// element has no name, so an unnamed file still round-trips instead of
/// colliding on an empty name.
fn hsc_file_name(name: &str, index: usize) -> String {
    let mut stem: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    if stem.is_empty() {
        stem = format!("source_{index}");
    }
    format!("{stem}.{HSC_EXTENSION}")
}

#[cfg(test)]
mod tests;
