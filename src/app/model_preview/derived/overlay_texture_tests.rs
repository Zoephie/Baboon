use super::*;

/// Collision and physics overlays land while the model's textures are
/// still resolving. The merge appends flat-coloured materials after the
/// model's own, so the resolve in flight is still the right answer and
/// has to be kept, not dropped and run again.
#[test]
fn an_overlay_merge_keeps_the_texture_resolve_in_flight() {
    let mut app = Baboon::for_test();
    let stamp = app.model.kit_stamp();
    let key = "file:a.model".to_owned();
    let preview = RenderModelPreview {
        materials: vec![Default::default()],
        ..Default::default()
    };
    let data = super::super::model_preview_data(key.clone(), key.clone(), preview, Vec::new());
    let (geometry_id, textures_id) = (data.geometry_id, data.textures_id);
    let state = app.views[app.model.kits[0].id].caches.model_previews.entry(key.clone()).or_default();
    state.data = Some(Ok(data));
    state.textures_pending = true;

    let overlay = RenderModelPreview {
        materials: vec![Default::default()],
        ..Default::default()
    };
    app.handle_model_overlays_built(stamp, key.clone(), geometry_id, Some(overlay), None);
    let state = &app.views[app.model.kits[0].id].caches.model_previews[&key];
    assert!(
        state.textures_pending,
        "the resolve in flight is still awaited"
    );
    let Some(Ok(data)) = state.data.as_ref() else {
        panic!("preview data");
    };
    assert_ne!(data.geometry_id, geometry_id, "the geometry did change");

    app.handle_model_textures_resolved(
        stamp,
        key.clone(),
        textures_id,
        vec![Default::default()],
    );
    let Some(Ok(data)) = app.views[app.model.kits[0].id].caches.model_previews[&key].data.as_ref() else {
        panic!("preview data");
    };
    assert!(data.textures.is_some(), "and its result is kept");
}
