//! Import-tool error report extraction for model preview overlays.

use super::*;

/// Attach every drawable error-report primitive in `tag` to `preview`.
///
/// The three model tag families share this schema (with small naming/version
/// differences). Missing blocks and fields are intentionally tolerated: older
/// tags and stripped shipping tags commonly retain the schema but no reports.
pub(super) fn append_model_errors(
    tag: &TagFile,
    preview: &mut RenderModelPreview,
    layer: ModelPreviewLayer,
) {
    let root = tag.root();
    let Some(categories) = error_field(&root, "errors").and_then(|field| field.as_block()) else {
        return;
    };

    for category in categories.iter() {
        let category_name = read_string(&category, "name")
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Model error".into());
        let category_severity = read_report_severity(&category, "report type");
        let Some(reports) = error_field(&category, "reports").and_then(|field| field.as_block())
        else {
            continue;
        };
        for report in reports.iter() {
            // Hover shows the concise report category ("open edge",
            // "duplicate triangle", …), not the often long per-instance log
            // sentence. Comment primitives are the one exception below.
            let label = category_name.clone();
            let severity = read_report_severity(&report, "type")
                .or(category_severity)
                .unwrap_or_default();
            if !matches!(severity, ReportSeverity::Warning | ReportSeverity::Error) {
                continue;
            }
            let non_critical = read_non_critical_flag(&report).unwrap_or(false)
                || read_non_critical_flag(&category).unwrap_or(false);

            append_point_block(&report, "vertices", &label, non_critical, layer, preview);
            append_comment_block(&report, &label, non_critical, layer, preview);
            append_vector_block(&report, &label, non_critical, layer, preview);
            append_point_array_block(
                &report,
                "lines",
                false,
                &label,
                non_critical,
                layer,
                preview,
            );
            append_point_array_block(
                &report,
                "triangles",
                true,
                &label,
                non_critical,
                layer,
                preview,
            );
            append_point_array_block(&report, "quads", true, &label, non_critical, layer, preview);
        }
    }
}

fn append_point_block(
    report: &TagStruct<'_>,
    field_name: &str,
    label: &str,
    non_critical: bool,
    layer: ModelPreviewLayer,
    preview: &mut RenderModelPreview,
) {
    let Some(block) = error_field(report, field_name).and_then(|field| field.as_block()) else {
        return;
    };
    preview.errors.extend(block.iter().filter_map(|entry| {
        read_error_point(&entry).map(|point| ModelErrorPrimitive {
            label: label.to_owned(),
            non_critical,
            color: read_error_color(&entry),
            layer,
            shape: ModelErrorShape::Point(point),
        })
    }));
}

fn append_comment_block(
    report: &TagStruct<'_>,
    fallback_label: &str,
    non_critical: bool,
    layer: ModelPreviewLayer,
    preview: &mut RenderModelPreview,
) {
    let Some(block) = error_field(report, "comments").and_then(|field| field.as_block()) else {
        return;
    };
    preview.errors.extend(block.iter().filter_map(|entry| {
        let point = read_error_point(&entry)?;
        let label = read_data_text(&entry, "text")
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| fallback_label.to_owned());
        Some(ModelErrorPrimitive {
            label,
            non_critical,
            color: read_error_color(&entry),
            layer,
            shape: ModelErrorShape::Point(point),
        })
    }));
}

fn append_vector_block(
    report: &TagStruct<'_>,
    label: &str,
    non_critical: bool,
    layer: ModelPreviewLayer,
    preview: &mut RenderModelPreview,
) {
    let Some(block) = error_field(report, "vectors").and_then(|field| field.as_block()) else {
        return;
    };
    preview.errors.extend(block.iter().filter_map(|entry| {
        let point = read_error_point(&entry)?;
        let normal = read_vector(&entry, "normal")?;
        let length = match error_field(&entry, "screen length").and_then(|field| field.value()) {
            Some(TagFieldData::Real(value)) if value.is_finite() && value > 0.0 => value,
            _ => 0.1,
        };
        Some(ModelErrorPrimitive {
            label: label.to_owned(),
            non_critical,
            color: read_error_color(&entry),
            layer,
            shape: ModelErrorShape::Vector {
                point,
                normal,
                length,
            },
        })
    }));
}

fn append_point_array_block(
    report: &TagStruct<'_>,
    field_name: &str,
    face: bool,
    label: &str,
    non_critical: bool,
    layer: ModelPreviewLayer,
    preview: &mut RenderModelPreview,
) {
    let Some(block) = error_field(report, field_name).and_then(|field| field.as_block()) else {
        return;
    };
    preview.errors.extend(block.iter().filter_map(|entry| {
        let points = error_field(&entry, "points")?
            .as_array()?
            .iter()
            .filter_map(|element| read_error_point(&element))
            .collect::<Vec<_>>();
        if points.len() < 2 {
            return None;
        }
        Some(ModelErrorPrimitive {
            label: label.to_owned(),
            non_critical,
            color: read_error_color(&entry),
            layer,
            shape: if face {
                ModelErrorShape::Face(points)
            } else {
                ModelErrorShape::Polyline(points)
            },
        })
    }));
}

fn read_error_point(value: &TagStruct<'_>) -> Option<ModelErrorPoint> {
    // Vertex/vector/comment elements contain the point definition directly;
    // line/triangle/quad arrays wrap it in a field also named `point`.
    let point = error_field(value, "point")
        .and_then(|field| field.as_struct())
        .unwrap_or(*value);
    let position = match error_field(&point, "position")?.value()? {
        TagFieldData::RealPoint3d(point) => [point.x, point.y, point.z],
        _ => return None,
    };
    let mut node_indices = [-1; 4];
    if let Some(array) = error_field(&point, "node indices").and_then(|field| field.as_array()) {
        for (slot, element) in array.iter().take(4).enumerate() {
            node_indices[slot] =
                match error_field(&element, "node index").and_then(|field| field.value()) {
                    Some(TagFieldData::CharInteger(index)) => index as i16,
                    Some(TagFieldData::ByteInteger(index)) => index as i16,
                    _ => -1,
                };
        }
    }
    let mut node_weights = [0.0; 4];
    if let Some(array) = error_field(&point, "node weights").and_then(|field| field.as_array()) {
        for (slot, element) in array.iter().take(4).enumerate() {
            if let Some(TagFieldData::Real(weight)) =
                error_field(&element, "node weight").and_then(|field| field.value())
            {
                node_weights[slot] = weight;
            }
        }
    }
    Some(ModelErrorPoint {
        position,
        node_indices,
        node_weights,
    })
}

fn read_string(value: &TagStruct<'_>, field_name: &str) -> Option<String> {
    match error_field(value, field_name)?.value()? {
        TagFieldData::String(text) | TagFieldData::LongString(text) => Some(text),
        TagFieldData::StringId(id) | TagFieldData::OldStringId(id) => Some(id.string),
        _ => None,
    }
}

fn read_data_text(value: &TagStruct<'_>, field_name: &str) -> Option<String> {
    let bytes = error_field(value, field_name)?.as_data()?;
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).trim().to_owned())
}

fn read_vector(value: &TagStruct<'_>, field_name: &str) -> Option<[f32; 3]> {
    match error_field(value, field_name)?.value()? {
        TagFieldData::RealVector3d(vector) => Some([vector.i, vector.j, vector.k]),
        _ => None,
    }
}

fn read_error_color(value: &TagStruct<'_>) -> [u8; 4] {
    let channel = |value: f32| {
        if value.is_finite() {
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            0
        }
    };
    match error_field(value, "color").and_then(|field| field.value()) {
        Some(TagFieldData::RealArgbColor(color)) => [
            channel(color.red),
            channel(color.green),
            channel(color.blue),
            channel(color.alpha),
        ],
        Some(TagFieldData::RealRgbColor(color)) => [
            channel(color.red),
            channel(color.green),
            channel(color.blue),
            255,
        ],
        Some(TagFieldData::ArgbColor(color)) => {
            let raw = color.0;
            [
                ((raw >> 16) & 0xFF) as u8,
                ((raw >> 8) & 0xFF) as u8,
                (raw & 0xFF) as u8,
                ((raw >> 24) & 0xFF) as u8,
            ]
        }
        Some(TagFieldData::RgbColor(color)) => {
            let raw = color.0;
            [
                ((raw >> 16) & 0xFF) as u8,
                ((raw >> 8) & 0xFF) as u8,
                (raw & 0xFF) as u8,
                255,
            ]
        }
        _ => MODEL_ERROR_FALLBACK_COLOR,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReportSeverity {
    Silent,
    Comment,
    Warning,
    #[default]
    Error,
}

fn read_report_severity(value: &TagStruct<'_>, field_name: &str) -> Option<ReportSeverity> {
    let (numeric, name) = match error_field(value, field_name)?.value()? {
        TagFieldData::CharEnum { value, name } => (value as i64, name),
        TagFieldData::ShortEnum { value, name } => (value as i64, name),
        TagFieldData::LongEnum { value, name } => (value as i64, name),
        _ => return None,
    };
    match name.as_deref().map(str::trim).map(str::to_ascii_lowercase) {
        Some(name) if name == "silent" => Some(ReportSeverity::Silent),
        Some(name) if name == "comment" => Some(ReportSeverity::Comment),
        Some(name) if name == "warning" => Some(ReportSeverity::Warning),
        Some(name) if name == "error" => Some(ReportSeverity::Error),
        _ => match numeric {
            0 => Some(ReportSeverity::Silent),
            1 => Some(ReportSeverity::Comment),
            2 => Some(ReportSeverity::Warning),
            3 => Some(ReportSeverity::Error),
            _ => None,
        },
    }
}

fn read_non_critical_flag(value: &TagStruct<'_>) -> Option<bool> {
    match error_field(value, "flags")?.value()? {
        TagFieldData::ByteFlags { value, .. } => Some(value & (1 << 2) != 0),
        TagFieldData::WordFlags { value, .. } => Some(value & (1 << 2) != 0),
        TagFieldData::LongFlags { value, .. } => Some(value & (1 << 2) != 0),
        _ => None,
    }
}

/// H2A definitions retain editor annotations in field names (`errors*`,
/// `name^*`, and similar), while classic H2 definitions use plain names.
/// Match through the same cleaned-name rules used by the field editor so one
/// extractor can walk both schemas.
fn error_field<'a>(value: &TagStruct<'a>, expected: &str) -> Option<TagField<'a>> {
    value
        .fields()
        .find(|field| field_name_matches(field.name(), expected))
}

#[cfg(test)]
mod tests {
    //! Real-tag coverage for model error extraction and wrapper/overlay plumbing.

    use super::*;
    use std::path::{Path, PathBuf};

    const H2_GAME: &str = "halo2_mcc";

    fn definitions_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("definitions")
    }

    fn installed_tags(env_var: &str, default: &str) -> PathBuf {
        std::env::var_os(env_var)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default))
    }

    fn h3_tags_root() -> PathBuf {
        installed_tags(
            "BLAM_TEST_H3EK",
            r"C:\Program Files (x86)\Steam\steamapps\common\H3EK\tags",
        )
    }

    fn h2_tags_root() -> PathBuf {
        installed_tags(
            "BLAM_TEST_H2EK",
            r"C:\Program Files (x86)\Steam\steamapps\common\H2EK\tags",
        )
    }

    fn h2r_tags_root() -> PathBuf {
        installed_tags(
            "BLAM_TEST_H2R",
            r"C:\Program Files (x86)\Steam\steamapps\common\H2R\tags",
        )
    }

    fn read_h2_tag(path: &Path, definition: &str) -> TagFile {
        let bytes = std::fs::read(path).expect("read shipped Halo 2 tag");
        let layout =
            blam_tags::layout::TagLayout::from_json(&definitions_root().join(H2_GAME).join(definition))
                .expect("load Halo 2 classic layout");
        blam_tags::classic::read_classic_tag_file(&bytes, layout).expect("decode shipped Halo 2 tag")
    }

    fn read_h2r_tag(path: &Path, group_tag: &[u8; 4]) -> TagFile {
        crate::core::source::read_tag_at_path(
            path,
            GameId::from_id(H2_GAME),
            Some(&definitions_root()),
            u32::from_be_bytes(*group_tag),
        )
        .expect("read shipped H2R tag")
    }

    fn h2_source(root: PathBuf) -> TagSource {
        TagSource::LooseFolder {
            root,
            game: GameId::from_id(H2_GAME),
            definitions_root: definitions_root(),
        }
    }

    fn model_entry(path: PathBuf, display_path: &str) -> TagEntry {
        TagEntry {
            key: file_entry_key(&path),
            display_path: display_path.to_owned(),
            group_tag: u32::from_be_bytes(*b"hlmt"),
            group_name: Some("model".to_owned()),
            location: TagEntryLocation::LooseFile(path),
        }
    }

    #[test]
    fn shipped_h3_render_errors_have_drawable_primitives() {
        let path =
            h3_tags_root().join(r"levels\ui\mainmenu\objects\spartan_cheap\spartan_cheap.render_model");
        if !path.is_file() {
            eprintln!("skipping: no shipped H3 render model at {}", path.display());
            return;
        }
        let tag = TagFile::read(&path).expect("read shipped render model");
        let mut preview = RenderModelPreview::default();
        append_model_errors(&tag, &mut preview, ModelPreviewLayer::Render);
        assert!(!preview.errors.is_empty());
        assert!(
            preview
                .errors
                .iter()
                .any(|error| error.label.to_ascii_lowercase().contains("triangle"))
        );
    }

    #[test]
    fn shipped_h3_collision_open_edge_is_a_hoverable_line() {
        let path = h3_tags_root()
            .join(r"objects\levels\solo\120_halo\plates\half_girder\half_girder.collision_model");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H3 collision model at {}",
                path.display()
            );
            return;
        }
        let tag = TagFile::read(&path).expect("read shipped collision model");
        let mut preview = RenderModelPreview::default();
        append_model_errors(&tag, &mut preview, ModelPreviewLayer::Collision);
        assert!(preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("open edge")
                && matches!(error.shape, ModelErrorShape::Polyline(_))
        }));
    }

    #[test]
    fn shipped_h2_render_errors_have_drawable_faces() {
        let path = h2_tags_root().join(r"objects\cinematics\human\cairo\cairo.render_model");
        if !path.is_file() {
            eprintln!("skipping: no shipped H2 render model at {}", path.display());
            return;
        }
        let preview = build_render_preview(&read_h2_tag(&path, "render_model.json"))
            .expect("build Halo 2 render preview");
        assert!(preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("duplicate triangle")
                && matches!(error.shape, ModelErrorShape::Face(_))
        }));
    }

    #[test]
    fn shipped_h2_model_wrapper_keeps_referenced_render_errors() {
        let root = h2_tags_root();
        let path = root.join(r"objects\cinematics\human\cairo\cairo.model");
        if !path.is_file() {
            eprintln!("skipping: no shipped H2 model at {}", path.display());
            return;
        }
        let tag = read_h2_tag(&path, "model.json");
        let names =
            TagNameIndex::load_game(&definitions_root(), GameId::from_id(H2_GAME).unwrap()).expect("load Halo 2 tag names");
        let source = h2_source(root);
        let entry = model_entry(path, "objects/cinematics/human/cairo/cairo.model");
        let data = load_model_preview(
            &tag,
            &entry,
            &names,
            Some(&source),
            &PreviewLoadSettings::default(),
        )
        .expect("build preview through the Halo 2 model wrapper");
        assert!(data.preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("duplicate triangle")
                && matches!(error.shape, ModelErrorShape::Face(_))
        }));
    }

    #[test]
    fn shipped_h2_collision_open_edge_is_a_hoverable_line() {
        let path = h2_tags_root()
            .join(r"scenarios\objects\multi\triplicate\kiosk_monitor\kiosk_monitor.collision_model");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H2 collision model at {}",
                path.display()
            );
            return;
        }
        let preview = build_collision_preview(&read_h2_tag(&path, "collision_model.json"), None)
            .expect("build Halo 2 collision preview");
        assert!(preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("open edge")
                && matches!(error.shape, ModelErrorShape::Polyline(_))
        }));
    }

    #[test]
    fn shipped_h2_model_wrapper_resolves_collision_errors() {
        let root = h2_tags_root();
        let path = root.join(r"scenarios\objects\multi\triplicate\kiosk_monitor\kiosk_monitor.model");
        if !path.is_file() {
            eprintln!("skipping: no shipped H2 model at {}", path.display());
            return;
        }
        let preview = hlmt_collision_overlay(&read_h2_tag(&path, "model.json"), &h2_source(root))
            .expect("resolve collision preview through the Halo 2 model wrapper");
        assert!(preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("open edge")
                && matches!(error.shape, ModelErrorShape::Polyline(_))
        }));
    }

    #[test]
    fn shipped_h2_physics_model_builds_with_the_shared_error_schema() {
        let path = h2_tags_root().join(r"objects\characters\brute\brute.physics_model");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H2 physics model at {}",
                path.display()
            );
            return;
        }
        let preview = build_physics_preview(&read_h2_tag(&path, "physics_model.json"), None)
            .expect("build Halo 2 physics preview");
        assert!(!preview.batches.is_empty());
        assert!(preview.errors.is_empty());
    }

    #[test]
    fn shipped_h2r_non_critical_flag_is_preserved() {
        let path = h2r_tags_root().join(r"!extracted\branding_iron\branding_iron.render_model");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H2R non-critical example at {}",
                path.display()
            );
            return;
        }
        let preview =
            build_render_preview(&read_h2r_tag(&path, b"mode")).expect("build H2R render preview");
        assert!(!preview.errors.is_empty());
        assert!(preview.errors.iter().all(|error| error.non_critical));
    }

    #[test]
    fn shipped_h2r_structure_bsp_reports_reach_the_preview() {
        let path = h2r_tags_root()
            .join(r"digsite\scenarios\multi\03_ascension\03_ascension.scenario_structure_bsp");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H2R structure BSP example at {}",
                path.display()
            );
            return;
        }
        let preview = build_sbsp_preview(&read_h2r_tag(&path, b"sbsp"), false)
            .expect("build H2R structure BSP preview");
        assert!(!preview.errors.is_empty());
        assert!(preview.errors.iter().all(|error| {
            error.layer == ModelPreviewLayer::Render
                && error.non_critical
                && error.color == [255, 128, 0, 255]
        }));
    }

    #[test]
    fn shipped_h2r_cov_barrier_duplicate_triangle_is_a_drawable_face() {
        let root = h2r_tags_root();
        let path = root.join(r"objects\gear\covenant\military\cov_barrier\cov_barrier.render_model");
        if !path.is_file() {
            eprintln!(
                "skipping: no shipped H2R render model at {}",
                path.display()
            );
            return;
        }
        let preview =
            build_render_preview(&read_h2r_tag(&path, b"mode")).expect("build H2R render preview");
        assert!(preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("duplicate triangle")
                && error.color == [255, 0, 0, 255]
                && matches!(error.shape, ModelErrorShape::Face(_))
        }));

        let model_path = root.join(r"objects\gear\covenant\military\cov_barrier\cov_barrier.model");
        let model = read_h2r_tag(&model_path, b"hlmt");
        let names =
            TagNameIndex::load_game(&definitions_root(), GameId::from_id(H2_GAME).unwrap()).expect("load Halo 2 tag names");
        let source = h2_source(root);
        let entry = model_entry(
            model_path,
            "objects/gear/covenant/military/cov_barrier/cov_barrier.model",
        );
        let data = load_model_preview(
            &model,
            &entry,
            &names,
            Some(&source),
            &PreviewLoadSettings::default(),
        )
        .expect("build preview through the H2R model wrapper");
        assert!(data.preview.errors.iter().any(|error| {
            error.label.eq_ignore_ascii_case("duplicate triangle")
                && matches!(error.shape, ModelErrorShape::Face(_))
        }));
    }
}
