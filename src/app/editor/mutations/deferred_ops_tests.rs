use super::*;

fn document() -> TagDocument {
    let schema = locate_definitions_root().join("halo3_mcc/render_model.json");
    TagDocument::clean(TagFile::new(schema).unwrap())
}

/// Every kind of deferred op must open an undo window, not just the ones
/// someone remembered to list. The H2 shader grid's value edits and the
/// function editor's byte-block writes go through the two kinds the old
/// hand-written condition left out, so they changed the tag with nothing
/// to undo to. Whether the op then applies is beside the point here: the
/// snapshot is taken before it runs.
#[test]
fn every_deferred_op_kind_opens_an_undo_window() {
    let mut h2_param = document();
    apply_deferred_ops(
        &mut h2_param,
        DeferredOps {
            h2_shader_param_ops: vec![H2ShaderParamOp::EditFunctionData {
                block_path: "missing".to_owned(),
                data: Vec::new(),
            }],
            ..DeferredOps::default()
        },
        "Edit",
    );
    assert!(h2_param.journal.can_undo(), "H2 shader parameter op");

    let mut function_data = document();
    let applied = apply_deferred_ops(
        &mut function_data,
        DeferredOps {
            function_data_ops: vec![FunctionDataOp {
                block_path: "missing".to_owned(),
                data: Vec::new(),
            }],
            ..DeferredOps::default()
        },
        "Edit",
    );
    assert!(function_data.journal.can_undo(), "function data op");
    assert!(
        applied
            .status
            .is_some_and(|status| status.starts_with("Function edit failed for missing")),
        "the function data op never ran"
    );

    let mut untouched = document();
    apply_deferred_ops(&mut untouched, DeferredOps::default(), "Edit");
    assert!(!untouched.journal.can_undo(), "a frame with no ops");
}
