//! Where the files shipped beside the binary are: the tag definitions and the
//! help docs. Release builds copy both next to `Baboon.exe`; development
//! builds fall back to the checkout.

use std::path::{Path, PathBuf};

/// Locate the runtime definitions root. The primary runtime contract is:
/// `definitions/` sits next to `Baboon.exe`.
pub(crate) fn locate_definitions_root() -> PathBuf {
    let mut expected = None;
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let beside_exe = exe_dir.join("definitions");
            if beside_exe.is_dir() {
                return beside_exe;
            }
            expected = Some(beside_exe);
        }
    }
    // The repo's own submodule first, as build.rs copies it: a sibling
    // checkout beside the repo can be at any other commit, and tests (which run
    // from target/*/deps, with no copy beside them) read whatever they find.
    let dev_at_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("definitions");
    if dev_at_manifest.is_dir() {
        return dev_at_manifest;
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("definitions");
    if dev.is_dir() {
        return dev;
    }
    expected.unwrap_or(dev_at_manifest)
}

pub(crate) fn definitions_missing_message(path: &Path) -> String {
    format!(
        "Could not find definitions folder. Expected it at {} — ensure the definitions submodule is initialised with 'git submodule update --init'.",
        path.display()
    )
}

/// Locate the runtime help docs root. Release builds copy `docs/` next to
/// `Baboon.exe`, matching the editable-on-disk contract used by `definitions/`.
pub(crate) fn locate_help_docs_root() -> PathBuf {
    let mut expected = None;
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let beside_exe = exe_dir.join("docs");
            if beside_exe.is_dir() {
                return beside_exe;
            }
            expected = Some(beside_exe);
        }
    }
    let dev_at_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs");
    if dev_at_manifest.is_dir() {
        return dev_at_manifest;
    }
    expected.unwrap_or(dev_at_manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::format::TagNameIndex;
    use crate::core::game::GameId;
    use blam_tags::TagFile;

    #[test]
    fn copied_classic_definitions_load_halo2_shader_layout() {
        let schema_path = locate_definitions_root()
            .join("halo2_mcc")
            .join("shader.json");
        assert!(schema_path.is_file());
        TagFile::new(&schema_path).expect("copied halo2 shader schema loads");
        let names = TagNameIndex::load_game(&locate_definitions_root(), GameId::Halo2)
            .expect("copied halo2 meta loads");
        assert_eq!(names.name_for(u32::from_be_bytes(*b"shad")), Some("shader"));
    }

    #[test]
    fn copied_definitions_include_all_known_games() {
        let root = locate_definitions_root();
        for game in [
            "haloce_mcc",
            "halo2_mcc",
            "halo2amp_mcc",
            "halo3_mcc",
            "halo3odst_mcc",
            "haloreach_mcc",
            "halo4_mcc",
        ] {
            assert!(
                root.join(game).join("_meta.json").is_file(),
                "missing copied definitions for {game}"
            );
        }
    }
}
