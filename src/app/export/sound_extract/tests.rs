use super::*;

#[test]
fn wav_header_is_canonical_pcm16() {
    let dir = std::env::temp_dir().join("baboon_wav_test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("t.wav");
    write_wav_pcm16(&path, &[0, 1, -1, 32767], 2, 44_100).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1); // PCM
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2); // channels
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        44_100
    );
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16); // bits
    assert_eq!(&bytes[36..40], b"data");
    // 4 samples * 2 bytes = 8 bytes of data.
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
        8
    );
    assert_eq!(bytes.len(), 44 + 8);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sanitize_strips_path_separators() {
    assert_eq!(sanitize_component("ambient/expl:1"), "ambient_expl_1");
    assert_eq!(sanitize_component("  "), "sound");
    assert_eq!(sanitize_component("plain_name"), "plain_name");
    // Device names Windows reserves, alone or before a dot, in any case.
    assert_eq!(sanitize_component("con"), "_con");
    assert_eq!(sanitize_component("NUL"), "_NUL");
    assert_eq!(sanitize_component("com1"), "_com1");
    assert_eq!(sanitize_component("lpt9.loop"), "_lpt9.loop");
    assert_eq!(sanitize_component("console"), "console");
    assert_eq!(sanitize_component("com10"), "com10");
}

#[test]
fn extract_base_dir_mirrors_the_tag() {
    let layout = KitLayout::from_tags_folder(Path::new("/ek/tags")).unwrap();
    let tag = Path::new("/ek/tags/sound/weapons/rifle.sound");
    assert_eq!(
        reimport_base_dir_lang(&layout, tag, None).unwrap(),
        PathBuf::from("/ek/data/sound/weapons/rifle")
    );
    // A non-default language routes to `data_<lang>\`.
    assert_eq!(
        reimport_base_dir_lang(&layout, tag, Some("french")).unwrap(),
        PathBuf::from("/ek/data_french/sound/weapons/rifle")
    );
}
