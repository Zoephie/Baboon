//! `core` must not depend on the UI: no egui or eframe, and nothing from
//! `crate::app` or `crate::window_state`, whether named directly or reached
//! by climbing out of `core` with `super::`.

use std::path::Path;

/// Forbidden paths in a file `depth` modules below the crate root
/// (`core/mod.rs` is 1, `core/source/mod.rs` is 2), with comments ignored.
fn layering_problems(label: &str, depth: usize, text: &str) -> Vec<String> {
    let code = without_comments(text);
    let mut problems = Vec::new();
    for word in ["egui", "eframe", "crate::app", "crate::window_state"] {
        let mut rest = code.as_str();
        while let Some(at) = rest.find(word) {
            let before = rest[..at].chars().next_back();
            let after = rest[at + word.len()..].chars().next();
            let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if boundary(before) && boundary(after) {
                problems.push(format!("{label} uses `{word}`"));
                break;
            }
            rest = &rest[at + word.len()..];
        }
    }
    let climb_out = "super::".repeat(depth);
    if code.contains(&climb_out) {
        problems.push(format!("{label} climbs out of `core` with `{climb_out}`"));
    }
    problems
}

fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |end| &after[end..]);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |end| &after[end + 2..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, root, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            // This file names the forbidden paths to test the check itself.
            && !path.ends_with("core/layering_tests.rs")
        {
            let relative = path.strip_prefix(root).unwrap();
            let mut depth = relative.components().count();
            if path.file_name().is_some_and(|name| name == "mod.rs") {
                depth -= 1;
            }
            let label = format!("src/{}", relative.to_string_lossy().replace('\\', "/"));
            let text = std::fs::read_to_string(&path).unwrap();
            out.extend(layering_problems(&label, depth, &text));
        }
    }
}

#[test]
fn core_does_not_depend_on_the_ui() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut problems = Vec::new();
    walk(&src.join("core"), &src, &mut problems);
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_layering_check_notices_a_dependency_on_the_ui() {
    assert_eq!(
        layering_problems("a.rs", 2, "use eframe::egui::Ui;"),
        ["a.rs uses `egui`", "a.rs uses `eframe`"]
    );
    assert_eq!(
        layering_problems("a.rs", 2, "fn f() { crate::app::thing() }"),
        ["a.rs uses `crate::app`"]
    );
    assert_eq!(
        layering_problems("a.rs", 2, "use super::super::app::Baboon;"),
        ["a.rs climbs out of `core` with `super::super::`"]
    );
    // Within `core`, in comments, or as part of a longer name, it is fine.
    assert!(layering_problems("a.rs", 2, "use super::format::x;").is_empty());
    assert!(layering_problems("a.rs", 2, "// see crate::app::Baboon\n/* egui */").is_empty());
    assert!(layering_problems("a.rs", 2, "let not_egui_x = crate::application;").is_empty());
}
