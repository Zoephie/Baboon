//! Where tests find real editing kits and game installs.
//!
//! Always an environment variable, never a path on one developer's machine.
//! When a variable is unset the path is a placeholder that does not exist, so
//! a test's own "skip unless present" check skips it, and its skip message
//! names the variable to set.

use std::path::PathBuf;

fn root(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("<set {var}>")))
}

/// A Halo: Combat Evolved (MCC) editing kit's `tags` folder.
pub(crate) fn hceek_tags() -> PathBuf {
    root("BLAM_TEST_HCEEK")
}

/// A Halo 2 (MCC) editing kit's `tags` folder.
pub(crate) fn h2ek_tags() -> PathBuf {
    root("BLAM_TEST_H2EK")
}

/// A Halo 3 (MCC) editing kit's `tags` folder.
pub(crate) fn h3ek_tags() -> PathBuf {
    root("BLAM_TEST_H3EK")
}

/// A Halo Reach (MCC) editing kit's `tags` folder.
pub(crate) fn hrek_tags() -> PathBuf {
    root("BLAM_TEST_HREK")
}

/// A Halo: Campaign Evolved install (the folder holding `Meteorite`).
pub(crate) fn ce_install() -> PathBuf {
    root("BLAM_TEST_CE")
}

/// A Campaign Evolved install's `Paks` folder.
pub(crate) fn ce_paks() -> PathBuf {
    ce_install().join("Meteorite/Content/Paks")
}

/// The environment variable naming a game's editing-kit `tags` folder.
fn kit_var(game: &str) -> String {
    match game {
        "halo2_mcc" => "BLAM_TEST_H2EK".to_owned(),
        "halo3_mcc" => "BLAM_TEST_H3EK".to_owned(),
        "halo3odst_mcc" => "BLAM_TEST_ODSTEK".to_owned(),
        "haloreach_mcc" => "BLAM_TEST_HREK".to_owned(),
        "halo4_mcc" => "BLAM_TEST_H4EK".to_owned(),
        "halo2amp_mcc" => "BLAM_TEST_H2AEK".to_owned(),
        "haloce_mcc" => "BLAM_TEST_HCEEK".to_owned(),
        other => format!("BLAM_TEST_{}", other.to_ascii_uppercase()),
    }
}

/// A path into a game's editing-kit `tags` folder, as a `&str` so a test that
/// used to hold a literal keeps its shape. Leaked; this is test code.
pub(crate) fn tag_path(game: &str, rel: &str) -> &'static str {
    let root = root(&kit_var(game));
    let path = if rel.is_empty() { root } else { root.join(rel) };
    leak(path)
}

/// This repository's own definitions, which tests read tags against.
pub(crate) fn definitions() -> &'static std::path::Path {
    std::path::Path::new(leak(crate::core::bundled::locate_definitions_root()))
}

pub(crate) fn leak(path: PathBuf) -> &'static str {
    Box::leak(path.display().to_string().into_boxed_str())
}

/// A path under the system temp directory that no other test, in this run or
/// a parallel one, will be handed. Not created.
pub(crate) fn unique_temp_path(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "baboon-{name}-{}-{nanos}-{serial}",
        std::process::id()
    ))
}

/// [`unique_temp_path`], created as an empty directory.
pub(crate) fn unique_temp_dir(name: &str) -> PathBuf {
    let dir = unique_temp_path(name);
    std::fs::create_dir_all(&dir).expect("create a temporary test directory");
    dir
}

/// `text` without its top-level `#[cfg(test)] mod … { … }` blocks.
fn product_code(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("#[cfg(test)]\n") {
        let module = &rest[start..];
        let header_end = module.find('\n').unwrap() + 1;
        let line_end = header_end + module[header_end..].find('\n').unwrap_or(0);
        let declaration = &module[header_end..line_end];
        let after_visibility = match declaration.strip_prefix("pub") {
            Some(scoped) if scoped.starts_with('(') => {
                scoped.split_once(") ").map_or("", |(_, rest)| rest)
            }
            Some(public) => public.trim_start(),
            None => declaration,
        };
        if !after_visibility.starts_with("mod ") {
            out.push_str(&rest[..start + header_end]);
            rest = &module[header_end..];
            continue;
        }
        out.push_str(&rest[..start]);
        rest = if declaration.trim_end().ends_with('{') {
            // Through the module's closing brace, at the start of a line.
            match module.find("\n}\n") {
                Some(end) => &module[end + 3..],
                None => "",
            }
        } else {
            &module[line_end..]
        };
    }
    out.push_str(rest);
    out
}

/// Every shipped source file under `src/app`, as `(path relative to src/app,
/// text)`: test files (`tests.rs`, `*_tests.rs`, anything under a `tests`
/// folder) are left out, and so are the inline `#[cfg(test)] mod … { … }`
/// blocks of the rest. For tests that check a rule across the code, so
/// that moving code between files cannot take it out of their sight.
pub(crate) fn app_product_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "tests" {
                    walk(&path, root, out);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, product_code(&std::fs::read_to_string(&path).unwrap())));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inline test modules are cut, whatever their visibility; product code
    /// under a `#[cfg(test)]` that is not a module is kept.
    #[test]
    fn product_code_cuts_inline_test_modules_only() {
        let text = "fn a() {}\n\
                    #[cfg(test)]\nmod tests {\n    fn t() {}\n}\n\
                    #[cfg(test)]\npub(in crate::app) mod probe_tests {\n    fn p() {}\n}\n\
                    #[cfg(test)]\nmod file_tests;\n\
                    #[cfg(test)]\nfn helper() {}\n\
                    fn b() {}\n";
        assert_eq!(
            product_code(text),
            "fn a() {}\n\n#[cfg(test)]\nfn helper() {}\nfn b() {}\n"
        );
    }
}
