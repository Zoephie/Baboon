//! Reading the shipped H3 shader option tags.
//!
//! `source extern` resolves by the option name embedded in the tag, and the
//! engine panics on a name it has no variant for — deliberately, so a decode gap
//! surfaces. That made three of the H3 kit's own option tags unreadable, and in a
//! GUI an unreadable option tag is a dead process rather than a message.

const H3_SHADERS: &str = "/Users/camden/Halo/halo3_mcc/tags/shaders";

/// Every `render_method_option` in the kit must decode through the typed reader
/// the shader grid uses. Named tags are called out because they are the ones that
/// used to panic, and because a regression here is silent until someone opens a
/// shader that happens to use them.
#[test]
fn every_shipped_h3_shader_option_decodes() {
    let root = std::path::Path::new(H3_SHADERS);
    if !root.exists() {
        eprintln!("skipping: no H3 editing kit");
        return;
    }
    const PREVIOUSLY_PANICKING: [&str; 3] = [
        "albedo_two_change_color_anim",
        "albedo_two_change_color_chameleon",
        "illum_detail_world_space_four_cc",
    ];

    let mut decoded = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut covered: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("render_method_option") {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(tag) = blam_tags::TagFile::read_from_bytes(&bytes) else {
            continue;
        };
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        // Caught rather than propagated: a panic here is exactly the defect, and
        // one failing tag should report itself instead of ending the test.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            blam_tags::render_method::RenderMethodOption::from_tag(&tag).is_ok()
        }));
        match outcome {
            Ok(true) => decoded += 1,
            Ok(false) => failed.push(format!("{stem} (error)")),
            Err(_) => failed.push(format!("{stem} (panic)")),
        }
        if PREVIOUSLY_PANICKING.contains(&stem.as_str()) {
            covered.push(stem);
        }
    }

    assert!(decoded > 100, "only {decoded} option tag(s) decoded — kit incomplete?");
    assert!(failed.is_empty(), "{} option tag(s) failed: {failed:?}", failed.len());
    assert_eq!(
        covered.len(),
        PREVIOUSLY_PANICKING.len(),
        "the tags this test exists for are missing from the kit: found {covered:?}"
    );
}
