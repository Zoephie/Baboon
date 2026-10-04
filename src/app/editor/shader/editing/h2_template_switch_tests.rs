use super::*;
use crate::app::editor::fields::extracted_tests::tests::with_test_edit_context;

/// Switching an H2 shader's template queues the new template's parameter
/// names, so parameters it lacks are pruned. This read the template off a
/// backslash-joined path, which found nothing outside Windows.
#[test]
fn switching_a_template_reads_its_parameters() {
    let root = crate::test_kits::h2ek_tags();
    let reference = "shaders/shader_templates/water/water_static";
    if !root.join(format!("{reference}.shader_template")).is_file() {
        eprintln!("skipping: {reference} not present under {}", root.display());
        return;
    }
    let row_edit = ShaderRowEdit {
        path: "template".to_owned(),
        current: String::new(),
        kind: ShaderRowEditKind::ShaderTemplateRef,
    };
    let root: &'static std::path::Path = std::path::Path::new(crate::test_kits::leak(root));
    with_test_edit_context(|edit| {
        edit.tags_root = Some(root);
        edit.game = Some(GameId::Halo2);
        push_h2_template_reference_edit(edit, &row_edit, reference.to_owned());
        let names = edit.h2_shader_param_ops.iter().find_map(|op| match op {
            H2ShaderParamOp::SwitchTemplate {
                allowed_parameter_names,
                ..
            } => Some(allowed_parameter_names.clone()),
            _ => None,
        });
        let names = names.expect("the template's parameters were read");
        assert!(!names.is_empty());
    });
}
