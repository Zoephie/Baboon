use super::*;

/// A material that drew untextured says why, instead of just looking flat.
#[test]
fn untextured_materials_say_why() {
    let material = |path: &str| RenderModelPreviewMaterial {
        shader_path: path.to_owned(),
        ..Default::default()
    };
    let materials = [
        material("shaders/a"),
        material("shaders/b"),
        material("shaders/c"),
    ];
    let textures = [
        MaterialTextures {
            error: Some("shader tag not found".to_owned()),
            ..Default::default()
        },
        MaterialTextures {
            used_shader_parameters_only: true,
            ..Default::default()
        },
        MaterialTextures::default(),
    ];
    let (summary, detail) = texture_resolve_note(&textures, &materials).unwrap();
    assert_eq!(
        summary,
        "1 of 3 materials untextured; 1 read without their definition or template defaults \
             — hover for why"
    );
    assert!(detail.contains("shaders/a: shader tag not found"));
    assert!(detail.contains("shaders/b"));
    assert!(!detail.contains("shaders/c"));
    assert_eq!(texture_resolve_note(&textures[2..], &materials[2..]), None);
}
