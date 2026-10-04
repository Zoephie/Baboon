//! Scans of Baboon's own source under `src/app`, for tests that check a rule
//! across the code, so that moving code between files cannot take it out of
//! their sight.

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
/// blocks of the rest.
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
