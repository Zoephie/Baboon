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

#[cfg(test)]
mod tests {
    use super::*;

    // Recovering a bitmap tag's source image to a folder the user picks.
    // `BLAM_TEST_HCEEK` names a Halo CE kit's `tags` folder.

    fn ce_bitmap(tags: &Path, rel: &str) -> TagEntry {
        TagEntry {
            key: file_entry_key(&tags.join(rel)),
            display_path: rel.to_owned(),
            group_tag: u32::from_be_bytes(*b"bitm"),
            group_name: Some("bitmap".to_owned()),
            location: TagEntryLocation::LooseFile(tags.join(rel)),
        }
    }

    const WHITE: &str = "ui/shell/bitmaps/white.bitmap";

    /// The one stock CE bitmap saved without its color plate.
    const NO_PLATE: &str = "digsite/placeholder/000-000-000-000-invisible.bitmap";

    fn ce_kit() -> Option<(PathBuf, TagSource)> {
        let tags = PathBuf::from(crate::core::test_kits::tag_path("haloce_mcc", ""));
        if !tags.join(WHITE).is_file() || !tags.join(NO_PLATE).is_file() {
            eprintln!("skipping: set BLAM_TEST_HCEEK to a Halo CE kit's tags folder");
            return None;
        }
        let source = TagSource::LooseFolder {
            root: tags.clone(),
            game: Some(GameId::HaloCe),
            definitions_root: crate::core::test_kits::definitions().to_path_buf(),
        };
        Some((tags, source))
    }

    #[test]
    fn a_bitmaps_source_lands_in_the_picked_folder() {
        let Some((tags, source)) = ce_kit() else {
            return;
        };
        let entry = ce_bitmap(&tags, WHITE);
        let out = crate::core::test_kits::unique_temp_dir("bitmap-source");

        let plate = blam_tags::bitmap::color_plate(&read_entry(&source, &entry).unwrap())
            .unwrap()
            .expect("white.bitmap carries its color plate");
        assert_eq!((plate.width, plate.height), (32, 32));
        // The white image sits on a color plate: Tool's key colors around it
        // (blue background, magenta sequence divider, cyan registration). With
        // red and blue swapped they'd read as red and yellow.
        let mut colors: Vec<[u8; 4]> = plate
            .rgba
            .chunks_exact(4)
            .map(|px| [px[0], px[1], px[2], px[3]])
            .collect();
        colors.sort();
        colors.dedup();
        assert_eq!(
            colors,
            [
                [0, 0, 255, 255],
                [0, 255, 255, 255],
                [255, 0, 255, 255],
                [255, 255, 255, 255]
            ]
        );

        // One tag lands in the picked folder itself.
        extract_bitmap_source(&source, &entry, &out).unwrap();
        let path = out.join("white.tif");
        let written = fs::read(&path).unwrap();
        let mut expected = Vec::new();
        plate.write_tiff(&mut expected).unwrap();
        assert_eq!(written, expected);

        // A second extraction must not replace what is there: it may be the
        // artist's own source by then.
        fs::write(&path, b"artist's edit").unwrap();
        let error = extract_bitmap_source(&source, &entry, &out).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"artist's edit");

        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn a_bitmap_without_a_source_says_so_and_writes_nothing() {
        let Some((tags, source)) = ce_kit() else {
            return;
        };
        let out = crate::core::test_kits::unique_temp_dir("bitmap-source-none");
        let entry = ce_bitmap(&tags, NO_PLATE);

        let error = extract_bitmap_source(&source, &entry, &out).unwrap_err();
        assert!(error.to_string().contains("no source image"), "{error}");
        assert!(!out.join("000-000-000-000-invisible.tif").exists());

        // In a folder extract it is reported, not fatal to the rest, and each
        // tag keeps its folder.
        let entries = [ce_bitmap(&tags, WHITE), entry];
        let status = extract_bitmap_sources(&source, &entries, &out).unwrap();
        assert!(
            status.starts_with("Extracted 1 bitmap source(s)"),
            "{status}"
        );
        assert!(
            status.contains("1 failed") && status.contains(NO_PLATE),
            "{status}"
        );
        assert!(out.join("ui/shell/bitmaps/white.tif").is_file());

        let _ = fs::remove_dir_all(&out);
    }
}
