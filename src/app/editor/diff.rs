//! Structural tag comparison.
//! It owns tag-editor presentation and deferred edit construction; source loading and application lifecycle coordination belong elsewhere.

use super::*;

pub(in crate::app) fn diff_tags(
    a: &TagFile,
    b: &TagFile,
    names: &TagNameIndex,
    limit: usize,
) -> (Vec<TagFieldDiff>, bool) {
    let mut out = Vec::new();
    diff_structs(&a.root(), &b.root(), "", names, &mut out, limit);
    let truncated = out.len() > limit;
    out.truncate(limit);
    (out, truncated)
}

fn diff_structs(
    a: &TagStruct<'_>,
    b: &TagStruct<'_>,
    path: &str,
    names: &TagNameIndex,
    out: &mut Vec<TagFieldDiff>,
    limit: usize,
) {
    for (fa, fb) in a.fields_all().zip(b.fields_all()) {
        if out.len() > limit {
            return;
        }
        let field_path = append_field_path(path, fa.name());
        if let (Some(ba), Some(bb)) = (fa.as_block(), fb.as_block()) {
            if ba.len() != bb.len() {
                out.push(TagFieldDiff {
                    path: field_path.clone(),
                    a: format!("{} element(s)", ba.len()),
                    b: format!("{} element(s)", bb.len()),
                });
            }
            for i in 0..ba.len().min(bb.len()) {
                let (Some(ea), Some(eb)) = (ba.element(i), bb.element(i)) else {
                    continue;
                };
                diff_structs(&ea, &eb, &format!("{field_path}[{i}]"), names, out, limit);
            }
        } else if let (Some(aa), Some(ab)) = (fa.as_array(), fb.as_array()) {
            for i in 0..aa.len().min(ab.len()) {
                let (Some(ea), Some(eb)) = (aa.element(i), ab.element(i)) else {
                    continue;
                };
                diff_structs(&ea, &eb, &format!("{field_path}[{i}]"), names, out, limit);
            }
        } else if let (Some(sa), Some(sb)) = (fa.as_struct(), fb.as_struct()) {
            diff_structs(&sa, &sb, &field_path, names, out, limit);
        } else if let (Some(va), Some(vb)) = (fa.value(), fb.value()) {
            let ta = foundation::format_foundation_scalar_value(names, &va);
            let tb = foundation::format_foundation_scalar_value(names, &vb);
            if ta != tb {
                out.push(TagFieldDiff {
                    path: field_path,
                    a: ta,
                    b: tb,
                });
            }
        }
    }
}
