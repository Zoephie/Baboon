use super::model_preview_supports_textures;
use crate::core::game::GameId;

#[test]
fn textured_shading_is_limited_to_supported_editing_kits() {
    assert!(model_preview_supports_textures(Some(GameId::Halo3)));
    assert!(model_preview_supports_textures(Some(GameId::HaloReach)));
    assert!(model_preview_supports_textures(Some(GameId::Halo2)));
    assert!(model_preview_supports_textures(Some(GameId::HaloCe)));
    for game in [
        None,
        Some(GameId::Halo3Odst),
        Some(GameId::Halo4),
        Some(GameId::Halo2Amp),
        Some(GameId::CampaignEvolved),
    ] {
        assert!(!model_preview_supports_textures(game), "{game:?}");
    }
}
