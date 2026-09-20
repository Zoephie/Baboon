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
    crate::source::read_tag_at_path(
        path,
        Some(H2_GAME),
        Some(&definitions_root()),
        u32::from_be_bytes(*group_tag),
    )
    .expect("read shipped H2R tag")
}

fn h2_source(root: PathBuf) -> TagSource {
    TagSource::LooseFolder {
        root,
        game: Some(H2_GAME.to_owned()),
        definitions_root: definitions_root(),
    }
}

fn model_entry(path: PathBuf, display_path: &str) -> TagEntry {
    TagEntry {
        key: format!("file:{}", path.display()),
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
        TagNameIndex::load_game(&definitions_root(), H2_GAME).expect("load Halo 2 tag names");
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
        TagNameIndex::load_game(&definitions_root(), H2_GAME).expect("load Halo 2 tag names");
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
