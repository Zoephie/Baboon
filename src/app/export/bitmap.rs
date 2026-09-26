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
    export_each(
        entries.iter().filter(|entry| is_bitmap_tag(entry)),
        |entry| write_bitmap_images(source, entry, &output.join(tag_display_parent(entry))),
    )
    .finish(
        "no bitmap tags found",
        "failed to extract bitmap tags",
        |images, tags| {
            format!(
                "Extracted {images} image(s) from {tags} bitmap tag(s) to {}",
                output.display()
            )
        },
    )
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
