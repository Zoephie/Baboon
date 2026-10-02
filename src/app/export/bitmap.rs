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

/// `<stem>.tif`: the name a bitmap tag's source image is written under.
fn bitmap_source_file_name(entry: &TagEntry) -> String {
    format!("{}.tif", tag_file_stem(entry))
}

/// Write one bitmap tag's source image into `output`, as `<stem>.tif`.
pub(in crate::app) fn extract_bitmap_source(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let path = output.join(bitmap_source_file_name(entry));
    write_bitmap_source(source, entry, &path)?;
    Ok(format!("Extracted bitmap source to {}", path.display()))
}

/// Write each bitmap tag's source image under `output` at the tag's own
/// folder, as the folder extract of the bitmap images does.
pub(in crate::app) fn extract_bitmap_sources(
    source: &TagSource,
    entries: &[TagEntry],
    output: &Path,
) -> anyhow::Result<String> {
    export_each(
        entries.iter().filter(|entry| is_bitmap_tag(entry)),
        |entry| {
            let path = output
                .join(tag_display_parent(entry))
                .join(bitmap_source_file_name(entry));
            write_bitmap_source(source, entry, &path)
        },
    )
    .finish(
        "no bitmap tags found",
        "failed to extract bitmap sources",
        |written, _| {
            format!(
                "Extracted {written} bitmap source(s) to {}",
                output.display()
            )
        },
    )
}

/// Write the color plate a bitmap tag was compiled from (the source image
/// Tool keeps zlib-compressed in the tag) to `path` as a TIFF that
/// `tool bitmaps` imports again. An existing file is never replaced: it may
/// be the artist's real source.
pub(in crate::app) fn write_bitmap_source(
    source: &TagSource,
    entry: &TagEntry,
    path: &Path,
) -> anyhow::Result<usize> {
    let tag = read_entry(source, entry)?;
    let Some(plate) = blam_tags::bitmap::color_plate(&tag)? else {
        anyhow::bail!("the tag carries no source image (it was saved without its color plate)");
    };
    // Encoded before the file exists, so a failure can't leave an empty
    // `.tif` behind that the next attempt would refuse to replace.
    let mut tiff = Vec::new();
    plate.write_tiff(&mut tiff)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::bail!("{} already exists", path.display())
        }
        Err(error) => return Err(error.into()),
    };
    std::io::Write::write_all(&mut file, &tiff)?;
    Ok(1)
}
