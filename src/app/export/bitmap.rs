//! bitmap export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(in crate::app) fn extract_bitmap_images(
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

pub(in crate::app) fn extract_bitmap_entries(
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

pub(in crate::app) fn write_bitmap_images(
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
