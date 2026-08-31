//! The textured BSP contract against a real kit: the native H3 decode must
//! keep UVs, tangent frames, and the materials block's shader paths — the
//! three things the ASS text path lost, and the three things diffuse, normal
//! mapping, and alpha-test each depend on.

use super::*;

/// Point `BABOON_MODEL_KIT` at an H3-family kit's `tags` folder to run this
/// against real BSPs; absent, it self-skips like the other fixture tests.
///
/// Several candidates rather than the first found: a working kit accumulates
/// converted and experimental BSPs whose materials are legitimately empty, and
/// the contract only claims that a *shipped* BSP keeps its shading inputs — so
/// one candidate passing the full chain is the assertion.
#[test]
fn a_real_bsps_render_layer_keeps_uvs_tangents_and_shader_paths() {
    let Some(tags_root) = std::env::var_os("BABOON_MODEL_KIT").map(std::path::PathBuf::from)
    else {
        eprintln!("skipping: set BABOON_MODEL_KIT to an editing kit's tags folder");
        return;
    };
    let candidates: Vec<std::path::PathBuf> = walkdir::WalkDir::new(&tags_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|found| {
            found.file_type().is_file()
                && found.path().extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("scenario_structure_bsp")
                })
        })
        .map(|found| found.path().to_path_buf())
        .take(8)
        .collect();
    if candidates.is_empty() {
        eprintln!(
            "skipping: no .scenario_structure_bsp under {}",
            tags_root.display()
        );
        return;
    }

    let source = TagSource::LooseFolder {
        root: tags_root.clone(),
        game: Some("halo3_mcc".to_owned()),
        definitions_root: std::path::PathBuf::new(),
    };
    for path in &candidates {
        let Ok(tag) =
            crate::source::read_tag_at_path(path, None, None, u32::from_be_bytes(*b"sbsp"))
        else {
            continue;
        };
        let Ok(preview) = build_sbsp_preview(&tag, false) else {
            continue;
        };
        let with_shader = preview
            .materials
            .iter()
            .filter(|material| !material.shader_path.is_empty())
            .count();
        eprintln!(
            "{}: {} vertices, {} batches, {with_shader}/{} materials with shaders",
            path.display(),
            preview.vertices.len(),
            preview.batches.len(),
            preview.materials.len(),
        );
        if with_shader == 0 {
            continue;
        }

        // UVs must not all be zero, or every texture would smear one texel.
        let nonzero_uv = preview
            .vertices
            .iter()
            .filter(|vertex| vertex.texcoord[0].abs() > 0.001 || vertex.texcoord[1].abs() > 0.001)
            .count();
        assert!(
            nonzero_uv > preview.vertices.len() / 4,
            "{}: UVs did not survive the decode",
            path.display()
        );
        // Tangent frames must survive for normal mapping to perturb.
        let with_tangent = preview
            .vertices
            .iter()
            .filter(|vertex| {
                let t = vertex.tangent;
                (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt() > 0.5
            })
            .count();
        assert!(
            with_tangent > preview.vertices.len() / 4,
            "{}: tangent frames did not survive the decode",
            path.display()
        );

        let sample: Vec<RenderModelPreviewMaterial> =
            preview.materials.iter().take(12).cloned().collect();
        let resolved = resolve_model_textures(&source, &sample);
        let base = resolved
            .iter()
            .filter(|material| material.get(TextureSlot::Base).is_some())
            .count();
        let bump = resolved
            .iter()
            .filter(|material| material.get(TextureSlot::Bump).is_some())
            .count();
        eprintln!(
            "  of {} sampled materials: {base} diffuse, {bump} normal maps resolved",
            sample.len()
        );
        assert!(base > 0, "{}: no diffuse maps resolved", path.display());
        assert!(bump > 0, "{}: no normal maps resolved", path.display());
        return;
    }
    panic!(
        "none of {} candidate BSPs under {} carried shader materials",
        candidates.len(),
        tags_root.display()
    );
}
