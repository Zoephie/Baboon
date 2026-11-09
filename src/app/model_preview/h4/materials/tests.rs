use super::*;

/// The Didact's whole suit glowed and the Chief turned to chrome while
/// `control_map_spglrf` missed `control_map_SpGlRfTexture` and the white
/// placeholder stood in for it.
#[test]
fn a_parameter_finds_its_sampler_whatever_the_source_spells_it() {
    let wanted = vec![
        ("color_mapTexture".to_owned(), TextureKind::Flat),
        ("control_map_SpGlRfTexture".to_owned(), TextureKind::Flat),
        ("reflection_mapTexture".to_owned(), TextureKind::Cube),
    ];
    assert_eq!(
        texture_uniform(&wanted, "control_map_spglrf").map(|(name, _)| name),
        Some("control_map_SpGlRfTexture")
    );
    assert_eq!(
        texture_uniform(&wanted, "reflection_map").map(|(_, kind)| *kind),
        Some(TextureKind::Cube)
    );
    assert_eq!(
        texture_uniform(&wanted, "control_map"),
        None,
        "a prefix is not a match"
    );
}

#[test]
fn blend_modes_take_the_preview_states_their_names_describe() {
    assert_eq!(h4_blend(0), Gen3Blend::Opaque);
    assert_eq!(
        h4_blend(23),
        Gen3Blend::Opaque,
        "opaque_alpha_blend counts as opaque"
    );
    assert_eq!(h4_blend(3), Gen3Blend::AlphaBlend);
    assert_eq!(
        h4_blend(24),
        Gen3Blend::Additive,
        "alpha_blend_additive_transparent"
    );
    assert_eq!(h4_blend(9), Gen3Blend::AddSrcTimesSrcAlpha);
    assert_eq!(h4_blend(10), Gen3Blend::InvAlphaBlend);
}
