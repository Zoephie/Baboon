//! GLSL pieces spliced into more than one program's source.

/// `view_depth`: a window depth back to the distance along the view, by
/// `u_depth`'s projection terms (see [`scene_depth_terms`](crate::app::model_preview::common::camera::scene_depth_terms)), for the sprites
/// that fade against the scene's depth.
pub(in crate::app::model_preview) const VIEW_DEPTH: &str = include_str!("shaders/view_depth.glsl");

/// `template` with each `${name}` in it replaced by `name`'s value in
/// `values`, so a `.glsl` file can take what Rust computes (a version line,
/// a count, a typed constant) without `format!`'s doubled braces.
///
/// # Panics
///
/// On a `${` with no closing `}`, a placeholder `values` has no value for,
/// or a value no placeholder uses. Every template is a constant, so the
/// first test that builds one finds the mistake.
pub(in crate::app::model_preview) fn fill(template: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(template.len());
    let mut used = vec![false; values.len()];
    let mut rest = template;
    while let Some(open) = rest.find("${") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after.find('}').unwrap_or_else(|| panic!("a `${{` with no `}}` in a GLSL template"));
        let name = &after[..close];
        let index = values
            .iter()
            .position(|(known, _)| *known == name)
            .unwrap_or_else(|| panic!("no value for `${{{name}}}` in a GLSL template"));
        used[index] = true;
        let _ = write!(out, "{}", values[index].1);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    if let Some(index) = used.iter().position(|used| !used) {
        panic!("`{}` is given to a GLSL template that has no `${{{}}}`", values[index].0, values[index].0);
    }
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_takes_each_value_by_name_and_keeps_its_braces() {
        let rows = 768;
        let filled = fill("uniform vec4 u_bones[${rows}];\nvoid main() { ${out} = vec4(1.0); }\n", &[("rows", &rows), ("out", &"out_color")]);
        assert_eq!(filled, "uniform vec4 u_bones[768];\nvoid main() { out_color = vec4(1.0); }\n");
        assert_eq!(fill("${a}${a}", &[("a", &1)]), "11", "a placeholder may repeat");
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    #[should_panic(expected = "no value for `${missing}`")]
    fn a_placeholder_without_a_value_panics() {
        fill("${missing}", &[]);
    }

    #[test]
    #[should_panic(expected = "`spare` is given to a GLSL template that has no `${spare}`")]
    fn a_value_no_placeholder_uses_panics() {
        fill("plain", &[("spare", &1)]);
    }

    #[test]
    #[should_panic(expected = "with no `}`")]
    fn an_unclosed_placeholder_panics() {
        fill("${open", &[("open", &1)]);
    }
}
