//! geometry export operations.
//! It owns export transformation and file-output preparation; interactive UI and document lifecycle management belong elsewhere.

use super::*;

pub(in crate::app) fn extract_geometry_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    match &entry.group_tag.to_be_bytes() {
        b"hlmt" => extract_model_geometry(source, entry, output),
        b"scnr" => extract_scenario_geometry(source, entry, output),
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

pub(in crate::app) fn extract_model_geometry(
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

pub(in crate::app) fn load_referenced_tag_from_source(
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

pub(in crate::app) fn render_jms_for_game(tag: &TagFile) -> anyhow::Result<JmsFile> {
    Ok(match blam_tags::game::Game::of(tag) {
        blam_tags::game::Game::Halo1 => JmsFile::from_gbxmodel(tag)?,
        blam_tags::game::Game::Halo2 => JmsFile::from_h2_render_model(tag)?,
        blam_tags::game::Game::Halo3 => JmsFile::from_render_model(tag)?,
    })
}

pub(in crate::app) fn render_model_prefers_ass(tag: &TagFile) -> bool {
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

/// Adapts a [`TagSource`] into a [`blam_tags::extract::TagResolver`] so the
/// shared extraction orchestration can resolve child tag references
/// (jmad → render_model, scenario → structure_bsp/stli) through Baboon's
/// cache- and classic-aware loader.
struct SourceResolver<'a> {
    source: &'a TagSource,
}

impl blam_tags::extract::TagResolver for SourceResolver<'_> {
    fn resolve(
        &self,
        reference: &str,
        group_ext: &str,
        group_tag: u32,
    ) -> Result<TagFile, blam_tags::extract::ExtractError> {
        load_referenced_tag_from_source(self.source, reference, group_ext, &group_tag.to_be_bytes())
            .map_err(|error| blam_tags::extract::ExtractError::resolve(error.to_string()))
    }
}

/// Extract every animation in `entry` (a jmad, `.model`, object tag, or
/// Halo CE `model_animations`) to JMA-family files under
/// `<output>/<stem>/animations/`, in-process via `blam_tags::extract`.
pub(in crate::app) fn extract_animations_for_entry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let resolver = SourceResolver { source };
    let stem = tag_file_stem(entry);
    let summary = blam_tags::extract::animation::animations_to_dir(&tag, &resolver, output, &stem)?;
    let mut message = format!(
        "Extracted {} animation(s) from {} into {}",
        summary.written,
        entry.display_path,
        output.display(),
    );
    if summary.skipped > 0 {
        message.push_str(&format!(" ({} skipped)", summary.skipped));
    }
    Ok(message)
}

/// Extract per-BSP scenario geometry — one ASS (Halo 2 / Halo 3) or render +
/// collision JMS (Halo CE) per structure BSP — under
/// `<output>/<stem>/structure/`, in-process via `blam_tags::extract`.
pub(in crate::app) fn extract_scenario_geometry(
    source: &TagSource,
    entry: &TagEntry,
    output: &Path,
) -> anyhow::Result<String> {
    let tag = read_entry(source, entry)?;
    let resolver = SourceResolver { source };
    let stem = tag_file_stem(entry);
    let summary =
        blam_tags::extract::geometry::scenario_geometry_to_dir(&tag, &resolver, output, &stem)?;
    let mut message = format!(
        "Extracted {} geometry file(s) from {} into {}",
        summary.emitted.len(),
        entry.display_path,
        output.display(),
    );
    if !summary.warnings.is_empty() {
        message.push_str(&format!(" ({} warning(s))", summary.warnings.len()));
    }
    Ok(message)
}
