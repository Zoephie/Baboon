//! GLSL pieces spliced into more than one program's source.

/// `view_depth`: a window depth back to the distance along the view, by
/// `u_depth`'s projection terms (see [`scene_depth_terms`](crate::app::model_preview::common::camera::scene_depth_terms)), for the sprites
/// that fade against the scene's depth.
macro_rules! glsl_view_depth {
    () => {
        "float view_depth(float window) {
    float ndc = window * 2.0 - 1.0;
    return (ndc - u_depth.y) / (u_depth.x - ndc * u_depth.z);
}
"
    };
}

pub(in crate::app::model_preview) use glsl_view_depth;

/// With `BABOON_SHADER_DUMP` set to a folder, writes `source` there once,
/// named by its hash: every shader source the tests compile, so a change
/// that should leave the GLSL as it was can be diffed against a dump taken
/// before it. The pixel gates see what a program draws, not its text.
#[cfg(test)]
pub(in crate::app::model_preview) fn record_source(source: &str) {
    let Some(dir) = std::env::var_os("BABOON_SHADER_DUMP") else { return };
    let hash = source.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let dir = std::path::PathBuf::from(dir);
    let path = dir.join(format!("{hash:016x}.glsl"));
    if path.exists() {
        return;
    }
    // Written aside and renamed, since tests on other threads may write the same one.
    let aside = dir.join(format!("{hash:016x}.{:?}.part", std::thread::current().id()));
    if std::fs::write(&aside, source).is_ok() {
        let _ = std::fs::rename(&aside, &path);
    }
}

