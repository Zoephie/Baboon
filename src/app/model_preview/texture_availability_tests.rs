use super::model_preview_supports_textures;

#[test]
fn textured_shading_is_limited_to_supported_editing_kits() {
    assert!(model_preview_supports_textures(Some("halo3_mcc")));
    assert!(model_preview_supports_textures(Some("haloreach_mcc")));
    assert!(model_preview_supports_textures(Some("halo2_mcc")));
    assert!(model_preview_supports_textures(Some("haloce_mcc")));
    for game in [
        None,
        Some("halo3odst_mcc"),
        Some("halo4_mcc"),
        Some("halo2amp_mcc"),
        Some("haloce_evolved"),
    ] {
        assert!(!model_preview_supports_textures(game), "{game:?}");
    }
}
