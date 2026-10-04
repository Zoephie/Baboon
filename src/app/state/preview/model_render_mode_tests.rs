use super::*;

#[test]
fn model_render_modes_select_expected_passes() {
    assert!(ModelRenderMode::Solid.draws_shading());
    assert!(!ModelRenderMode::Solid.draws_wireframe());
    assert!(!ModelRenderMode::Solid.uses_textures());

    assert!(!ModelRenderMode::Wireframe.draws_shading());
    assert!(ModelRenderMode::Wireframe.draws_wireframe());

    assert!(ModelRenderMode::SolidWireframe.draws_shading());
    assert!(ModelRenderMode::SolidWireframe.draws_wireframe());
    assert!(ModelRenderMode::Textured.draws_shading());
    assert!(!ModelRenderMode::Textured.draws_wireframe());
    assert!(ModelRenderMode::Textured.uses_textures());
    assert!(ModelRenderMode::TexturedWireframe.draws_shading());
    assert!(ModelRenderMode::TexturedWireframe.draws_wireframe());
    assert!(ModelRenderMode::TexturedWireframe.uses_textures());
    assert_eq!(
        ModelPreviewState::default().render_mode,
        ModelRenderMode::Textured
    );
}

#[test]
fn unavailable_textures_keep_the_wireframe_choice() {
    assert_eq!(
        ModelRenderMode::Textured.without_textures(),
        ModelRenderMode::Solid
    );
    assert_eq!(
        ModelRenderMode::TexturedWireframe.without_textures(),
        ModelRenderMode::SolidWireframe
    );
    assert_eq!(
        ModelRenderMode::Wireframe.without_textures(),
        ModelRenderMode::Wireframe
    );
}
